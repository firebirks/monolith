//! The message store file of `docs/adr/0008-message-store.md`.
//!
//! One file, `messages`, beside the vault and under its lock, rewritten
//! whole and replaced atomically like the vault. Its payload is encrypted
//! with XChaCha20-Poly1305 under a key that HKDF-SHA256 derives from the
//! vault key ([`crate::vault::Vault::message_store_key`]), with the whole
//! header as associated data. It has no passphrase and no KDF of its own.
//!
//! ```text
//! offset  size  field
//! 0       8     magic "MNLTMSG" 0x00
//! 8       2     format version (1)
//! 10      2     reserved (0)
//! 12      8     generation
//! 20      24    nonce
//! 44      4     payload length, tag included
//! 48      n     payload
//! ```
//!
//! This module knows bytes. What the payload means is
//! [`crate::message_records`]'s.

use monolith_protocol::limits::{MAX_MESSAGE_STORE_FILE_LEN, MESSAGE_STORE_PADDING_BLOCK_LEN};
use zeroize::Zeroizing;

use crate::StorageError;
use crate::dir::{MESSAGES, MESSAGES_NEW, VaultDir};
use crate::vault::{open_with, seal_with};

/// The first eight bytes of a message store.
const MAGIC: [u8; 8] = *b"MNLTMSG\0";

/// The format version this build writes and reads.
pub const FORMAT_VERSION: u16 = 1;

const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;
const TAG_LEN: usize = 16;

/// The whole header, up to the payload.
pub const HEADER_LEN: usize = 48;

/// The key of a message store. Erased when dropped; prints nothing.
pub struct StoreKey(Zeroizing<[u8; KEY_LEN]>);

impl StoreKey {
    pub(crate) const fn new(key: Zeroizing<[u8; KEY_LEN]>) -> Self {
        Self(key)
    }

    /// A key made from fixed bytes, for tests and the fuzz targets of
    /// other crates. A store sealed under it protects nothing.
    #[doc(hidden)]
    pub fn from_bytes_for_testing(bytes: [u8; KEY_LEN]) -> Self {
        Self(Zeroizing::new(bytes))
    }
}

impl core::fmt::Debug for StoreKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("StoreKey([redacted])")
    }
}

/// What opening a message store found besides the store itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Recovery {
    /// Only the store was there.
    Clean,
    /// A write that had not completed was found next to the store and
    /// removed. The store is as before that write.
    DroppedUnfinishedWrite,
    /// Only a complete new store was found: a creation that was interrupted
    /// after the file was complete. It was put in place.
    CompletedCreation,
}

/// An open message store: its directory, its key, its generation.
pub struct MessageFile<D: VaultDir> {
    dir: D,
    key: StoreKey,
    generation: u64,
}

impl<D: VaultDir> core::fmt::Debug for MessageFile<D> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MessageFile")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

fn random<const N: usize>() -> Result<[u8; N], StorageError> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(|_| StorageError::Randomness)?;
    Ok(bytes)
}

/// Reads `N` bytes at `offset`.
fn field<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N], StorageError> {
    bytes
        .get(offset..offset.checked_add(N).ok_or(StorageError::BadFormat)?)
        .and_then(|slice| slice.try_into().ok())
        .ok_or(StorageError::BadFormat)
}

/// Authenticates and decrypts a whole store file under `key`. Returns its
/// generation and its padded plaintext.
fn decrypt(key: &StoreKey, file: &[u8]) -> Result<(u64, Zeroizing<Vec<u8>>), StorageError> {
    if field::<8>(file, 0)? != MAGIC {
        return Err(StorageError::BadFormat);
    }
    let version = u16::from_be_bytes(field(file, 8)?);
    if version > FORMAT_VERSION {
        return Err(StorageError::UnsupportedVersion);
    }
    if version != FORMAT_VERSION || field::<2>(file, 10)? != [0, 0] {
        return Err(StorageError::BadFormat);
    }
    let generation = u64::from_be_bytes(field(file, 12)?);
    let nonce: [u8; NONCE_LEN] = field(file, 20)?;
    let payload_len = u32::from_be_bytes(field(file, 44)?);
    let payload_len = usize::try_from(payload_len).map_err(|_| StorageError::BadFormat)?;
    // The file is exactly the header and the payload.
    if HEADER_LEN.checked_add(payload_len) != Some(file.len()) {
        return Err(StorageError::BadFormat);
    }
    let header = file.get(..HEADER_LEN).ok_or(StorageError::BadFormat)?;
    let mut payload = Zeroizing::new(
        file.get(HEADER_LEN..)
            .ok_or(StorageError::BadFormat)?
            .to_vec(),
    );
    open_with(&key.0, &nonce, header, &mut payload)?;
    Ok((generation, payload))
}

/// The bytes of a store file with `plaintext` as payload, padded, at
/// `generation`.
fn seal_file(key: &StoreKey, generation: u64, plaintext: &[u8]) -> Result<Vec<u8>, StorageError> {
    let padded_len = plaintext
        .len()
        .div_ceil(MESSAGE_STORE_PADDING_BLOCK_LEN)
        .max(1)
        .checked_mul(MESSAGE_STORE_PADDING_BLOCK_LEN)
        .ok_or(StorageError::TooLarge)?;
    let payload_len = padded_len
        .checked_add(TAG_LEN)
        .ok_or(StorageError::TooLarge)?;
    let file_len = HEADER_LEN
        .checked_add(payload_len)
        .ok_or(StorageError::TooLarge)?;
    if u64::try_from(file_len).map_err(|_| StorageError::TooLarge)? > MAX_MESSAGE_STORE_FILE_LEN {
        return Err(StorageError::TooLarge);
    }
    let nonce = random::<NONCE_LEN>()?;
    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(&MAGIC);
    header.extend_from_slice(&FORMAT_VERSION.to_be_bytes());
    header.extend_from_slice(&[0, 0]);
    header.extend_from_slice(&generation.to_be_bytes());
    header.extend_from_slice(&nonce);
    header.extend_from_slice(
        &u32::try_from(payload_len)
            .map_err(|_| StorageError::TooLarge)?
            .to_be_bytes(),
    );
    let mut payload = Zeroizing::new(Vec::with_capacity(payload_len));
    payload.extend_from_slice(plaintext);
    payload.resize(padded_len, 0);
    seal_with(&key.0, &nonce, &header, &mut payload)?;
    let mut file = header;
    file.extend_from_slice(&payload);
    Ok(file)
}

/// The largest plaintext a store file can hold: what pads to the largest
/// whole number of blocks that still fits the file with its header and tag.
pub const MAX_PLAINTEXT_LEN: usize = {
    let room = MAX_FILE_LEN - HEADER_LEN - TAG_LEN;
    room - room % MESSAGE_STORE_PADDING_BLOCK_LEN
};

/// [`MAX_MESSAGE_STORE_FILE_LEN`] in the type of lengths in memory.
const MAX_FILE_LEN: usize = 16 * 1024 * 1024;

const _: () = assert!(MAX_FILE_LEN as u64 == MAX_MESSAGE_STORE_FILE_LEN);

impl<D: VaultDir> MessageFile<D> {
    /// Creates a store in `dir` with `plaintext` as its payload: written as
    /// `messages.new`, flushed, and given the name `messages` by a link,
    /// which fails if a store exists. Fails with [`StorageError::Exists`]
    /// if `messages` or `messages.new` exists.
    pub fn create(dir: D, key: StoreKey, plaintext: &[u8]) -> Result<Self, StorageError> {
        if dir.read(MESSAGES, MAX_MESSAGE_STORE_FILE_LEN)?.is_some()
            || dir
                .read(MESSAGES_NEW, MAX_MESSAGE_STORE_FILE_LEN)?
                .is_some()
        {
            return Err(StorageError::Exists);
        }
        let file = seal_file(&key, 1, plaintext)?;
        dir.write_new(MESSAGES_NEW, &file)?;
        dir.link(MESSAGES_NEW, MESSAGES)?;
        dir.remove(MESSAGES_NEW)?;
        dir.sync()?;
        Ok(Self {
            dir,
            key,
            generation: 1,
        })
    }

    /// Opens the store in `dir` and returns it with its padded payload.
    ///
    /// The cases of `docs/STORAGE.md` section 3.5, with the names of the
    /// store: only `messages` is opened; with `messages.new` beside a
    /// `messages` that authenticates, the unfinished write is removed; with
    /// `messages.new` beside one that does not, and a `messages.new` that
    /// does, nothing is changed and the result is
    /// [`StorageError::RecoveryNeeded`]; a `messages.new` alone that
    /// authenticates is an interrupted creation and is linked into place.
    /// Fails with [`StorageError::NotFound`] if neither exists. A file that
    /// does not authenticate is never removed or overwritten.
    pub fn open(
        dir: D,
        key: StoreKey,
    ) -> Result<(Self, Zeroizing<Vec<u8>>, Recovery), StorageError> {
        let current = dir.read(MESSAGES, MAX_MESSAGE_STORE_FILE_LEN)?;
        let new = dir.read(MESSAGES_NEW, MAX_MESSAGE_STORE_FILE_LEN)?;
        let (generation, plaintext, recovery) = match (current, new) {
            (None, None) => return Err(StorageError::NotFound),
            (Some(current), None) => {
                let (generation, plaintext) = decrypt(&key, &current)?;
                (generation, plaintext, Recovery::Clean)
            }
            (Some(current), Some(new)) => match decrypt(&key, &current) {
                Ok((generation, plaintext)) => {
                    dir.remove(MESSAGES_NEW)?;
                    dir.sync()?;
                    (generation, plaintext, Recovery::DroppedUnfinishedWrite)
                }
                Err(error) => {
                    return Err(if decrypt(&key, &new).is_ok() {
                        StorageError::RecoveryNeeded
                    } else {
                        error
                    });
                }
            },
            (None, Some(new)) => {
                let (generation, plaintext) = decrypt(&key, &new)?;
                dir.link(MESSAGES_NEW, MESSAGES)?;
                dir.remove(MESSAGES_NEW)?;
                dir.sync()?;
                (generation, plaintext, Recovery::CompletedCreation)
            }
        };
        Ok((
            Self {
                dir,
                key,
                generation,
            },
            plaintext,
            recovery,
        ))
    }

    /// Replaces the payload with `plaintext`: a complete new file with the
    /// next generation is written as `messages.new`, flushed, renamed to
    /// `messages`, and the directory flushed. A `messages.new` left by a
    /// write that failed is removed first. If the write fails, the store on
    /// disk is the previous state, or the new one if the rename completed.
    pub fn write(&mut self, plaintext: &[u8]) -> Result<(), StorageError> {
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(StorageError::TooLarge)?;
        let file = seal_file(&self.key, generation, plaintext)?;
        if self
            .dir
            .read(MESSAGES_NEW, MAX_MESSAGE_STORE_FILE_LEN)?
            .is_some()
        {
            self.dir.remove(MESSAGES_NEW)?;
        }
        self.dir.write_new(MESSAGES_NEW, &file)?;
        self.dir.rename(MESSAGES_NEW, MESSAGES)?;
        self.dir.sync()?;
        self.generation = generation;
        Ok(())
    }

    /// The generation of the store on disk as of the last write.
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;
    use crate::dir::{CrashOutcome, MemoryDir};

    fn key() -> StoreKey {
        StoreKey::from_bytes_for_testing([7; KEY_LEN])
    }

    fn padded(text: &[u8]) -> Vec<u8> {
        let mut bytes = text.to_vec();
        bytes.resize(MESSAGE_STORE_PADDING_BLOCK_LEN, 0);
        bytes
    }

    #[test]
    fn a_store_opens_with_its_key_only() {
        let dir = MemoryDir::new();
        let store = MessageFile::create(dir.clone(), key(), b"hello").unwrap();
        assert_eq!(store.generation(), 1);
        let file = dir
            .read(MESSAGES, MAX_MESSAGE_STORE_FILE_LEN)
            .unwrap()
            .unwrap();
        assert_eq!(
            file.len(),
            HEADER_LEN + MESSAGE_STORE_PADDING_BLOCK_LEN + TAG_LEN
        );
        assert_eq!(&file[..8], b"MNLTMSG\0");
        let (mut store, plaintext, recovery) = MessageFile::open(dir.clone(), key()).unwrap();
        assert_eq!(*plaintext, padded(b"hello"));
        assert_eq!(recovery, Recovery::Clean);
        store.write(b"again").unwrap();
        assert_eq!(store.generation(), 2);
        let (_, plaintext, _) = MessageFile::open(dir.clone(), key()).unwrap();
        assert_eq!(*plaintext, padded(b"again"));
        assert_eq!(
            MessageFile::open(dir, StoreKey::from_bytes_for_testing([8; KEY_LEN])).err(),
            Some(StorageError::AuthenticationFailed)
        );
    }

    #[test]
    fn a_changed_byte_anywhere_fails_and_nothing_is_created_twice() {
        let dir = MemoryDir::new();
        MessageFile::create(dir.clone(), key(), b"hello").unwrap();
        assert_eq!(
            MessageFile::create(dir.clone(), key(), b"other").err(),
            Some(StorageError::Exists)
        );
        let file = dir
            .read(MESSAGES, MAX_MESSAGE_STORE_FILE_LEN)
            .unwrap()
            .unwrap();
        for index in [0, 9, 11, 12, 20, 44, 48, file.len() - 1] {
            let mut changed = file.clone();
            changed[index] ^= 1;
            dir.overwrite(MESSAGES, &changed).unwrap();
            assert!(MessageFile::open(dir.clone(), key()).is_err(), "{index}");
        }
        // A truncated file and a longer one.
        dir.overwrite(MESSAGES, &file[..file.len() - 1]).unwrap();
        assert!(MessageFile::open(dir.clone(), key()).is_err());
        dir.overwrite(MESSAGES, &[file.as_slice(), &[0]].concat())
            .unwrap();
        assert!(MessageFile::open(dir.clone(), key()).is_err());
        // A newer format version is refused as such.
        let mut newer = file.clone();
        newer[9] = 2;
        dir.overwrite(MESSAGES, &newer).unwrap();
        assert_eq!(
            MessageFile::open(dir, key()).err(),
            Some(StorageError::UnsupportedVersion)
        );
    }

    #[test]
    fn the_largest_plaintext_fits_and_one_more_byte_does_not() {
        let dir = MemoryDir::new();
        let mut store = MessageFile::create(dir.clone(), key(), b"").unwrap();
        store.write(&vec![1; MAX_PLAINTEXT_LEN]).unwrap();
        assert_eq!(
            store.write(&vec![1; MAX_PLAINTEXT_LEN + 1]).err(),
            Some(StorageError::TooLarge)
        );
        let (_, plaintext, _) = MessageFile::open(dir, key()).unwrap();
        assert_eq!(plaintext.len(), MAX_PLAINTEXT_LEN);
    }

    #[test]
    fn a_crash_at_any_step_of_a_write_leaves_the_old_or_the_new_state() {
        for outcome in CrashOutcome::ALL {
            let mut step = 0;
            loop {
                let dir = MemoryDir::new();
                let mut store = MessageFile::create(dir.clone(), key(), b"old").unwrap();
                dir.crash_at(dir.steps() + step);
                let written = store.write(b"new");
                let crashed = dir.crashed();
                let after = dir.restart(outcome);
                let (_, plaintext, _) = MessageFile::open(after, key()).unwrap();
                assert!(
                    *plaintext == padded(b"old") || *plaintext == padded(b"new"),
                    "{outcome:?} {step}"
                );
                if written.is_ok() && !crashed {
                    assert_eq!(*plaintext, padded(b"new"), "{outcome:?} {step}");
                    break;
                }
                step += 1;
            }
        }
    }

    #[test]
    fn a_crash_during_creation_leaves_no_store_or_a_whole_one() {
        for outcome in CrashOutcome::ALL {
            let mut step = 0;
            loop {
                let dir = MemoryDir::new();
                dir.crash_at(step);
                let created = MessageFile::create(dir.clone(), key(), b"first");
                let crashed = dir.crashed();
                let after = dir.restart(outcome);
                match MessageFile::open(after, key()) {
                    Ok((_, plaintext, _)) => assert_eq!(*plaintext, padded(b"first")),
                    // Nothing, or an unfinished creation that is left alone.
                    Err(error) => assert!(
                        matches!(
                            error,
                            StorageError::NotFound | StorageError::AuthenticationFailed
                        ) || matches!(error, StorageError::BadFormat),
                        "{outcome:?} {step} {error:?}"
                    ),
                }
                if created.is_ok() && !crashed {
                    break;
                }
                step += 1;
            }
        }
    }

    #[test]
    fn debug_shows_no_key() {
        let text = format!("{:?}", key());
        assert!(!text.contains('7'), "{text}");
    }
}
