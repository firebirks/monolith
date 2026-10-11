//! The vault file: version 1 of `docs/STORAGE.md` section 3.
//!
//! A vault is one file, rewritten whole and replaced atomically. Its
//! payload is encrypted with XChaCha20-Poly1305 under a key derived by
//! HKDF-SHA256 from a random vault key; the vault key is wrapped under a
//! key that Argon2id derives from the passphrase. Every byte of the header
//! is associated data of the payload, and the first 64 bytes of the
//! wrapped key, so a changed byte anywhere makes opening fail.
//!
//! This module knows bytes. What the payload means is `record`'s.

use chacha20poly1305::aead::AeadInOut;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use monolith_protocol::limits::{
    DEFAULT_KDF_ITERATIONS, DEFAULT_KDF_MEMORY_KIB, DEFAULT_KDF_PARALLELISM, MAX_KDF_ITERATIONS,
    MAX_KDF_MEMORY_KIB, MAX_KDF_PARALLELISM, MAX_VAULT_FILE_LEN, MIN_KDF_ITERATIONS,
    MIN_KDF_MEMORY_KIB, MIN_KDF_PARALLELISM,
};
use sha2::Sha256;
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

use crate::StorageError;
use crate::dir::{VAULT, VAULT_NEW, VaultDir};

/// The first eight bytes of a vault.
const MAGIC: [u8; 8] = *b"MNLTVLT\0";

/// The format version this build writes and reads.
pub const FORMAT_VERSION: u16 = 1;

/// The KDF identifier for Argon2id, version 0x13.
const KDF_ARGON2ID: u8 = 1;

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;
const TAG_LEN: usize = 16;
const WRAPPED_LEN: usize = KEY_LEN + TAG_LEN;

/// The part of the header that is fixed when the vault is created or its
/// passphrase changes: magic to wrapped key.
const KEY_HEADER_LEN: usize = 64 + WRAPPED_LEN;

/// The whole header, up to the payload.
pub const HEADER_LEN: usize = 148;

/// The plaintext of a payload is padded to a multiple of this.
pub const PADDING_BLOCK: usize = 64 * 1024;

/// Longest passphrase, in bytes of UTF-8 after normalization to NFC.
pub const MAX_PASSPHRASE_LEN: usize = 1024;

/// The HKDF label of the payload key.
const PAYLOAD_INFO: &[u8] = b"MONOLITH-VAULT-PAYLOAD-V1";

/// The cost of the key derivation of a vault.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KdfParams {
    /// Memory, in KiB.
    pub memory_kib: u32,
    /// Iterations.
    pub iterations: u32,
    /// Lanes.
    pub parallelism: u32,
}

impl KdfParams {
    /// The default of a new vault: provisional until the benchmark of
    /// `docs/STORAGE.md` section 3.3 is complete.
    pub const DEFAULT: Self = Self {
        memory_kib: DEFAULT_KDF_MEMORY_KIB,
        iterations: DEFAULT_KDF_ITERATIONS,
        parallelism: DEFAULT_KDF_PARALLELISM,
    };

    /// The weakest parameters a vault may have: RFC 9106's second
    /// recommended setting.
    pub const FLOOR: Self = Self {
        memory_kib: MIN_KDF_MEMORY_KIB,
        iterations: MIN_KDF_ITERATIONS,
        parallelism: 4,
    };

    /// Checks the parameters against the bounds a header may have. Values
    /// outside are refused before Argon2 runs, so that a changed header can
    /// neither make an unlock allocate without bound nor weaken a vault.
    pub fn check(self) -> Result<Self, StorageError> {
        let within = (MIN_KDF_MEMORY_KIB..=MAX_KDF_MEMORY_KIB).contains(&self.memory_kib)
            && (MIN_KDF_ITERATIONS..=MAX_KDF_ITERATIONS).contains(&self.iterations)
            && (MIN_KDF_PARALLELISM..=MAX_KDF_PARALLELISM).contains(&self.parallelism);
        if within {
            Ok(self)
        } else {
            Err(StorageError::BadFormat)
        }
    }
}

/// The passphrase of a vault, normalized to NFC.
///
/// Erased when dropped. Prints nothing.
pub struct Passphrase(Zeroizing<String>);

impl Passphrase {
    /// Normalizes `text` to NFC. Fails with [`StorageError::BadPassphrase`]
    /// for an empty passphrase or one longer than [`MAX_PASSPHRASE_LEN`]
    /// bytes after normalization.
    pub fn new(text: &str) -> Result<Self, StorageError> {
        // Room for the longest form NFC can give, three times the input,
        // so that the string never grows and leaves a copy behind.
        let mut normalized = Zeroizing::new(String::with_capacity(text.len().saturating_mul(3)));
        normalized.extend(text.nfc());
        if normalized.is_empty() || normalized.len() > MAX_PASSPHRASE_LEN {
            return Err(StorageError::BadPassphrase);
        }
        Ok(Self(normalized))
    }

    fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl core::fmt::Debug for Passphrase {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Passphrase([redacted])")
    }
}

fn random<const N: usize>() -> Result<Zeroizing<[u8; N]>, StorageError> {
    let mut bytes = Zeroizing::new([0_u8; N]);
    getrandom::fill(bytes.as_mut_slice()).map_err(|_| StorageError::Randomness)?;
    Ok(bytes)
}

/// Argon2id of the passphrase.
fn derive_kek(
    passphrase: &Passphrase,
    salt: &[u8; SALT_LEN],
    params: KdfParams,
) -> Result<Zeroizing<[u8; KEY_LEN]>, StorageError> {
    let params = params.check()?;
    let argon_params = argon2::Params::new(
        params.memory_kib,
        params.iterations,
        params.parallelism,
        Some(KEY_LEN),
    )
    .map_err(|_| StorageError::BadFormat)?;
    let argon = argon2::Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        argon_params,
    );
    let mut kek = Zeroizing::new([0_u8; KEY_LEN]);
    argon
        .hash_password_into(passphrase.as_bytes(), salt, kek.as_mut_slice())
        .map_err(|_| StorageError::Kdf)?;
    Ok(kek)
}

fn payload_key(vault_key: &[u8; KEY_LEN]) -> Result<Zeroizing<[u8; KEY_LEN]>, StorageError> {
    let mut key = Zeroizing::new([0_u8; KEY_LEN]);
    hkdf::Hkdf::<Sha256>::new(None, vault_key)
        .expand(PAYLOAD_INFO, key.as_mut_slice())
        .map_err(|_| StorageError::Internal)?;
    Ok(key)
}

fn cipher(key: &[u8; KEY_LEN]) -> Result<XChaCha20Poly1305, StorageError> {
    XChaCha20Poly1305::new_from_slice(key).map_err(|_| StorageError::Internal)
}

/// Encrypts `buffer` in place and appends the tag.
fn seal(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    associated: &[u8],
    buffer: &mut Vec<u8>,
) -> Result<(), StorageError> {
    let tag = cipher(key)?
        .encrypt_inout_detached(
            &XNonce::from(*nonce),
            associated,
            buffer.as_mut_slice().into(),
        )
        .map_err(|_| StorageError::Internal)?;
    buffer.extend_from_slice(&tag);
    Ok(())
}

/// Checks the tag at the end of `buffer` and decrypts the rest in place.
/// Fails with [`StorageError::AuthenticationFailed`] for anything that
/// does not authenticate.
fn open_sealed(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    associated: &[u8],
    buffer: &mut Vec<u8>,
) -> Result<(), StorageError> {
    let split = buffer
        .len()
        .checked_sub(TAG_LEN)
        .ok_or(StorageError::AuthenticationFailed)?;
    let tag_bytes: [u8; TAG_LEN] = buffer
        .get(split..)
        .and_then(|tag| tag.try_into().ok())
        .ok_or(StorageError::AuthenticationFailed)?;
    buffer.truncate(split);
    cipher(key)?
        .decrypt_inout_detached(
            &XNonce::from(*nonce),
            associated,
            buffer.as_mut_slice().into(),
            &tag_bytes.into(),
        )
        .map_err(|_| StorageError::AuthenticationFailed)
}

/// Reads `N` bytes at `offset`.
fn field<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N], StorageError> {
    bytes
        .get(offset..offset.checked_add(N).ok_or(StorageError::BadFormat)?)
        .and_then(|slice| slice.try_into().ok())
        .ok_or(StorageError::BadFormat)
}

/// The fixed part of a header: magic, version, KDF and wrapped key.
#[derive(Clone, PartialEq, Eq)]
struct KeyHeader([u8; KEY_HEADER_LEN]);

impl KeyHeader {
    /// Wraps `vault_key` under a key derived from `passphrase` with fresh
    /// salt and nonce.
    fn wrap(
        passphrase: &Passphrase,
        params: KdfParams,
        vault_key: &[u8; KEY_LEN],
    ) -> Result<Self, StorageError> {
        let salt = random::<SALT_LEN>()?;
        let nonce = random::<NONCE_LEN>()?;
        let mut header = [0_u8; KEY_HEADER_LEN];
        let mut prefix = Vec::with_capacity(64);
        prefix.extend_from_slice(&MAGIC);
        prefix.extend_from_slice(&FORMAT_VERSION.to_be_bytes());
        prefix.push(KDF_ARGON2ID);
        prefix.push(0);
        prefix.extend_from_slice(&params.memory_kib.to_be_bytes());
        prefix.extend_from_slice(&params.iterations.to_be_bytes());
        prefix.extend_from_slice(&params.parallelism.to_be_bytes());
        prefix.extend_from_slice(salt.as_slice());
        prefix.extend_from_slice(nonce.as_slice());
        let kek = derive_kek(passphrase, &salt, params)?;
        let mut wrapped = vault_key.to_vec();
        seal(&kek, &nonce, &prefix, &mut wrapped)?;
        for (slot, byte) in header.iter_mut().zip(prefix.iter().chain(wrapped.iter())) {
            *slot = *byte;
        }
        Ok(Self(header))
    }

    /// Checks the fields of a header read from a file, without the key.
    fn parse(bytes: &[u8]) -> Result<(Self, KdfParams), StorageError> {
        let header: [u8; KEY_HEADER_LEN] = field(bytes, 0)?;
        if field::<8>(&header, 0)? != MAGIC {
            return Err(StorageError::BadFormat);
        }
        let version = u16::from_be_bytes(field(&header, 8)?);
        if version > FORMAT_VERSION {
            return Err(StorageError::UnsupportedVersion);
        }
        if version != FORMAT_VERSION
            || field::<1>(&header, 10)? != [KDF_ARGON2ID]
            || field::<1>(&header, 11)? != [0]
        {
            return Err(StorageError::BadFormat);
        }
        let params = KdfParams {
            memory_kib: u32::from_be_bytes(field(&header, 12)?),
            iterations: u32::from_be_bytes(field(&header, 16)?),
            parallelism: u32::from_be_bytes(field(&header, 20)?),
        }
        .check()?;
        Ok((Self(header), params))
    }

    /// Derives the key-encryption key from `passphrase` and unwraps the
    /// vault key.
    fn unwrap_key(
        &self,
        passphrase: &Passphrase,
        params: KdfParams,
    ) -> Result<Zeroizing<[u8; KEY_LEN]>, StorageError> {
        let salt: [u8; SALT_LEN] = field(&self.0, 24)?;
        let nonce: [u8; NONCE_LEN] = field(&self.0, 40)?;
        let kek = derive_kek(passphrase, &salt, params)?;
        let mut wrapped = self
            .0
            .get(64..KEY_HEADER_LEN)
            .ok_or(StorageError::BadFormat)?
            .to_vec();
        let prefix = self.0.get(..64).ok_or(StorageError::BadFormat)?;
        open_sealed(&kek, &nonce, prefix, &mut wrapped)?;
        let wrapped = Zeroizing::new(wrapped);
        let key: [u8; KEY_LEN] = wrapped
            .as_slice()
            .try_into()
            .map_err(|_| StorageError::AuthenticationFailed)?;
        Ok(Zeroizing::new(key))
    }
}

/// What opening a vault found besides the vault itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Recovery {
    /// Only the vault was there.
    Clean,
    /// A write that had not completed was found next to the vault and
    /// removed. The vault is as before that write.
    DroppedUnfinishedWrite,
    /// Only a complete new vault was found: a creation that was
    /// interrupted after the file was complete. It was put in place. The
    /// user is told.
    CompletedCreation,
}

/// An open vault: the directory, the vault key and the header, so that
/// later writes need no key derivation.
pub struct Vault<D: VaultDir> {
    dir: D,
    key_header: KeyHeader,
    vault_key: Zeroizing<[u8; KEY_LEN]>,
    generation: u64,
}

impl<D: VaultDir> core::fmt::Debug for Vault<D> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Vault")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl<D: VaultDir> Vault<D> {
    /// Creates a vault in `dir` with `plaintext` as its payload.
    ///
    /// The new file is written as `vault.new`, flushed, and given the name
    /// `vault` by a link, which fails if a vault exists: a vault is never
    /// created over another one, and `vault` never exists partly written.
    /// Fails with [`StorageError::Exists`] if a vault or a `vault.new`
    /// exists.
    pub fn create(
        dir: D,
        passphrase: &Passphrase,
        params: KdfParams,
        plaintext: &[u8],
    ) -> Result<Self, StorageError> {
        if dir.read(VAULT, MAX_VAULT_FILE_LEN)?.is_some()
            || dir.read(VAULT_NEW, MAX_VAULT_FILE_LEN)?.is_some()
        {
            return Err(StorageError::Exists);
        }
        let vault_key = random::<KEY_LEN>()?;
        let key_header = KeyHeader::wrap(passphrase, params, &vault_key)?;
        let vault = Self {
            dir,
            key_header,
            vault_key,
            generation: 0,
        };
        let file = vault.seal_file(1, plaintext)?;
        vault.dir.write_new(VAULT_NEW, &file)?;
        vault.dir.link(VAULT_NEW, VAULT)?;
        vault.dir.remove(VAULT_NEW)?;
        vault.dir.sync()?;
        Ok(Self {
            generation: 1,
            ..vault
        })
    }

    /// Opens the vault in `dir` and returns it with its payload.
    ///
    /// The cases of `docs/STORAGE.md` section 3.5 at startup:
    ///
    /// - only `vault`: it is opened;
    /// - `vault` and `vault.new`: if `vault` authenticates it is used and
    ///   `vault.new`, a write that did not commit, is removed; if it does
    ///   not and `vault.new` does, nothing is changed and the result is
    ///   [`StorageError::RecoveryNeeded`], for the user to decide;
    /// - only `vault.new`: if it authenticates, it is a creation that was
    ///   interrupted; it is linked to `vault` and removed.
    ///
    /// A file that does not authenticate is never deleted or overwritten.
    /// A wrong passphrase and a changed file are the same error,
    /// [`StorageError::AuthenticationFailed`].
    pub fn open(
        dir: D,
        passphrase: &Passphrase,
    ) -> Result<(Self, Zeroizing<Vec<u8>>, Recovery), StorageError> {
        let current = dir.read(VAULT, MAX_VAULT_FILE_LEN)?;
        let new = dir.read(VAULT_NEW, MAX_VAULT_FILE_LEN)?;
        match (current, new) {
            (None, None) => Err(StorageError::NotFound),
            (Some(current), None) => {
                let (vault, plaintext) = Self::open_file(dir, passphrase, &current)?;
                Ok((vault, plaintext, Recovery::Clean))
            }
            (Some(current), Some(new)) => match Self::decrypt(passphrase, &current) {
                Ok((key_header, vault_key, generation, plaintext)) => {
                    dir.remove(VAULT_NEW)?;
                    dir.sync()?;
                    let vault = Self {
                        dir,
                        key_header,
                        vault_key,
                        generation,
                    };
                    Ok((vault, plaintext, Recovery::DroppedUnfinishedWrite))
                }
                Err(error) => {
                    if Self::decrypt(passphrase, &new).is_ok() {
                        Err(StorageError::RecoveryNeeded)
                    } else {
                        Err(error)
                    }
                }
            },
            (None, Some(new)) => {
                let (key_header, vault_key, generation, plaintext) =
                    Self::decrypt(passphrase, &new)?;
                dir.link(VAULT_NEW, VAULT)?;
                dir.remove(VAULT_NEW)?;
                dir.sync()?;
                let vault = Self {
                    dir,
                    key_header,
                    vault_key,
                    generation,
                };
                Ok((vault, plaintext, Recovery::CompletedCreation))
            }
        }
    }

    fn open_file(
        dir: D,
        passphrase: &Passphrase,
        file: &[u8],
    ) -> Result<(Self, Zeroizing<Vec<u8>>), StorageError> {
        let (key_header, vault_key, generation, plaintext) = Self::decrypt(passphrase, file)?;
        Ok((
            Self {
                dir,
                key_header,
                vault_key,
                generation,
            },
            plaintext,
        ))
    }

    /// Authenticates and decrypts a whole vault file.
    #[allow(clippy::type_complexity)]
    fn decrypt(
        passphrase: &Passphrase,
        file: &[u8],
    ) -> Result<(KeyHeader, Zeroizing<[u8; KEY_LEN]>, u64, Zeroizing<Vec<u8>>), StorageError> {
        let (key_header, params) = KeyHeader::parse(file)?;
        let generation = u64::from_be_bytes(field(file, 112)?);
        let nonce: [u8; NONCE_LEN] = field(file, 120)?;
        let payload_len = u32::from_be_bytes(field(file, 144)?);
        let payload_len = usize::try_from(payload_len).map_err(|_| StorageError::BadFormat)?;
        // The file is exactly the header and the payload.
        if HEADER_LEN.checked_add(payload_len) != Some(file.len()) {
            return Err(StorageError::BadFormat);
        }
        let vault_key = key_header.unwrap_key(passphrase, params)?;
        let key = payload_key(&vault_key)?;
        let header = file.get(..HEADER_LEN).ok_or(StorageError::BadFormat)?;
        let mut payload = Zeroizing::new(
            file.get(HEADER_LEN..)
                .ok_or(StorageError::BadFormat)?
                .to_vec(),
        );
        open_sealed(&key, &nonce, header, &mut payload)?;
        Ok((key_header, vault_key, generation, payload))
    }

    /// The bytes of a vault file with `plaintext` as payload, padded, at
    /// `generation`.
    fn seal_file(&self, generation: u64, plaintext: &[u8]) -> Result<Vec<u8>, StorageError> {
        let padded_len = plaintext
            .len()
            .div_ceil(PADDING_BLOCK)
            .max(1)
            .checked_mul(PADDING_BLOCK)
            .ok_or(StorageError::TooLarge)?;
        let payload_len = padded_len
            .checked_add(TAG_LEN)
            .ok_or(StorageError::TooLarge)?;
        let file_len = HEADER_LEN
            .checked_add(payload_len)
            .ok_or(StorageError::TooLarge)?;
        if u64::try_from(file_len).map_err(|_| StorageError::TooLarge)? > MAX_VAULT_FILE_LEN {
            return Err(StorageError::TooLarge);
        }
        let nonce = random::<NONCE_LEN>()?;
        let mut header = Vec::with_capacity(HEADER_LEN);
        header.extend_from_slice(&self.key_header.0);
        header.extend_from_slice(&generation.to_be_bytes());
        header.extend_from_slice(nonce.as_slice());
        header.extend_from_slice(
            &u32::try_from(payload_len)
                .map_err(|_| StorageError::TooLarge)?
                .to_be_bytes(),
        );
        let mut payload = Zeroizing::new(Vec::with_capacity(payload_len));
        payload.extend_from_slice(plaintext);
        payload.resize(padded_len, 0);
        let key = payload_key(&self.vault_key)?;
        seal(&key, &nonce, &header, &mut payload)?;
        let mut file = header;
        file.extend_from_slice(&payload);
        Ok(file)
    }

    /// Replaces the payload with `plaintext`: a complete new file with the
    /// next generation is written as `vault.new`, flushed, renamed to
    /// `vault`, and the directory flushed (`docs/STORAGE.md` section 3.5).
    /// Where rename is atomic, `vault` is always the old or the new state.
    ///
    /// A `vault.new` left by a write of this process that failed is
    /// removed first. If the write fails, the vault on disk is the previous
    /// state, or the new one if the rename completed; the caller treats the
    /// outcome as unknown until a write succeeds.
    pub fn write(&mut self, plaintext: &[u8]) -> Result<(), StorageError> {
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(StorageError::TooLarge)?;
        let file = self.seal_file(generation, plaintext)?;
        if self.dir.read(VAULT_NEW, MAX_VAULT_FILE_LEN)?.is_some() {
            self.dir.remove(VAULT_NEW)?;
        }
        self.dir.write_new(VAULT_NEW, &file)?;
        self.dir.rename(VAULT_NEW, VAULT)?;
        self.dir.sync()?;
        self.generation = generation;
        Ok(())
    }

    /// The generation of the vault on disk as of the last write.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Changes the passphrase: the same vault key is wrapped again with a
    /// new salt and nonce under `passphrase`, with `params`, and the
    /// payload is written again.
    pub fn change_passphrase(
        &mut self,
        passphrase: &Passphrase,
        params: KdfParams,
        plaintext: &[u8],
    ) -> Result<(), StorageError> {
        let previous = self.key_header.clone();
        self.key_header = KeyHeader::wrap(passphrase, params, &self.vault_key)?;
        let written = self.write(plaintext);
        if written.is_err() {
            self.key_header = previous;
        }
        written
    }

    /// The directory of the vault.
    pub const fn dir(&self) -> &D {
        &self.dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dir::{CrashOutcome, MemoryDir};

    fn passphrase() -> Passphrase {
        Passphrase::new("correct horse battery staple").unwrap()
    }

    #[test]
    fn a_vault_opens_with_its_passphrase_only() {
        let dir = MemoryDir::new();
        let vault = Vault::create(dir.clone(), &passphrase(), KdfParams::FLOOR, b"hello").unwrap();
        assert_eq!(vault.generation(), 1);
        let file = dir.read(VAULT, MAX_VAULT_FILE_LEN).unwrap().unwrap();
        // A whole padding block and the tag.
        assert_eq!(file.len(), HEADER_LEN + PADDING_BLOCK + TAG_LEN);
        let (mut vault, plaintext, recovery) = Vault::open(dir.clone(), &passphrase()).unwrap();
        assert_eq!(recovery, Recovery::Clean);
        assert_eq!(&plaintext[..5], b"hello");
        assert!(plaintext[5..].iter().all(|byte| *byte == 0));
        assert_eq!(
            Vault::open(dir.clone(), &Passphrase::new("wrong").unwrap()).err(),
            Some(StorageError::AuthenticationFailed)
        );
        vault.write(b"again").unwrap();
        assert_eq!(vault.generation(), 2);
        let (vault, plaintext, _) = Vault::open(dir.clone(), &passphrase()).unwrap();
        assert_eq!(&plaintext[..5], b"again");
        assert_eq!(vault.generation(), 2);
        // A vault is never created over another.
        assert_eq!(
            Vault::create(dir, &passphrase(), KdfParams::FLOOR, b"x").err(),
            Some(StorageError::Exists)
        );
    }

    #[test]
    fn every_byte_of_a_vault_is_authenticated() {
        let dir = MemoryDir::new();
        Vault::create(dir.clone(), &passphrase(), KdfParams::FLOOR, b"payload").unwrap();
        let file = dir.read(VAULT, MAX_VAULT_FILE_LEN).unwrap().unwrap();
        // A sample of positions in each field and in the payload. KDF
        // parameters outside the bounds are refused before Argon2 runs.
        let positions = [
            0,
            7,
            8,
            9,
            10,
            11,
            12,
            15,
            16,
            19,
            20,
            23,
            24,
            39,
            40,
            63,
            64,
            111,
            112,
            119,
            120,
            143,
            144,
            147,
            148,
            1000,
            file.len() - 1,
        ];
        for position in positions {
            let mut changed = file.clone();
            changed[position] ^= 0x01;
            dir.overwrite(VAULT, &changed).unwrap();
            let error = Vault::open(dir.clone(), &passphrase()).err();
            assert!(
                matches!(
                    error,
                    Some(
                        StorageError::AuthenticationFailed
                            | StorageError::BadFormat
                            | StorageError::UnsupportedVersion
                    )
                ),
                "{position}: {error:?}"
            );
        }
        // A file of any other length is refused.
        for changed in [
            &file[..file.len() - 1],
            &[file.as_slice(), &[0]].concat()[..],
        ] {
            dir.overwrite(VAULT, changed).unwrap();
            assert_eq!(
                Vault::open(dir.clone(), &passphrase()).err(),
                Some(StorageError::BadFormat)
            );
        }
    }

    #[test]
    fn kdf_parameters_outside_the_bounds_are_refused() {
        for params in [
            KdfParams {
                memory_kib: MIN_KDF_MEMORY_KIB - 1,
                ..KdfParams::FLOOR
            },
            KdfParams {
                memory_kib: MAX_KDF_MEMORY_KIB + 1,
                ..KdfParams::FLOOR
            },
            KdfParams {
                iterations: MIN_KDF_ITERATIONS - 1,
                ..KdfParams::FLOOR
            },
            KdfParams {
                iterations: MAX_KDF_ITERATIONS + 1,
                ..KdfParams::FLOOR
            },
            KdfParams {
                parallelism: 0,
                ..KdfParams::FLOOR
            },
            KdfParams {
                parallelism: MAX_KDF_PARALLELISM + 1,
                ..KdfParams::FLOOR
            },
        ] {
            assert_eq!(params.check(), Err(StorageError::BadFormat));
            assert_eq!(
                Vault::create(MemoryDir::new(), &passphrase(), params, b"").err(),
                Some(StorageError::BadFormat)
            );
        }
        assert!(KdfParams::DEFAULT.check().is_ok());
        assert!(KdfParams::FLOOR.check().is_ok());
    }

    #[test]
    fn a_passphrase_is_normalized_and_bounded() {
        // U+00E9 and U+0065 U+0301 are the same passphrase.
        let dir = MemoryDir::new();
        Vault::create(
            dir.clone(),
            &Passphrase::new("caf\u{e9}").unwrap(),
            KdfParams::FLOOR,
            b"x",
        )
        .unwrap();
        assert!(Vault::open(dir, &Passphrase::new("cafe\u{301}").unwrap()).is_ok());
        assert_eq!(Passphrase::new("").err().map(|_| ()), Some(()));
        assert!(Passphrase::new(&"a".repeat(MAX_PASSPHRASE_LEN)).is_ok());
        assert!(Passphrase::new(&"a".repeat(MAX_PASSPHRASE_LEN + 1)).is_err());
        assert_eq!(format!("{:?}", passphrase()), "Passphrase([redacted])");
    }

    #[test]
    fn a_changed_passphrase_keeps_the_vault_key() {
        let dir = MemoryDir::new();
        let mut vault =
            Vault::create(dir.clone(), &passphrase(), KdfParams::FLOOR, b"one").unwrap();
        let key = *vault.vault_key;
        let new = Passphrase::new("another one").unwrap();
        vault
            .change_passphrase(&new, KdfParams::FLOOR, b"two")
            .unwrap();
        assert_eq!(
            Vault::open(dir.clone(), &passphrase()).err(),
            Some(StorageError::AuthenticationFailed)
        );
        let (vault, plaintext, _) = Vault::open(dir, &new).unwrap();
        assert_eq!(&plaintext[..3], b"two");
        assert_eq!(*vault.vault_key, key);
    }

    /// Runs `operation` on a fresh copy of a vault that holds `before`, and
    /// stops it at every step; after each stop, a new process must find
    /// `before` or `after` and nothing else.
    fn interrupted<F>(before: &[u8], after: &[u8], operation: F)
    where
        F: Fn(&mut Vault<MemoryDir>) -> Result<(), StorageError>,
    {
        let base = MemoryDir::new();
        Vault::create(base.clone(), &passphrase(), KdfParams::FLOOR, before).unwrap();
        let (mut probe, _, _) =
            Vault::open(base.restart(CrashOutcome::ALL[0]), &passphrase()).unwrap();
        let start = probe.dir().steps();
        operation(&mut probe).unwrap();
        let total = probe.dir().steps() - start;
        assert!(total >= 6, "{total}");
        for step in 1..=total + 1 {
            for outcome in CrashOutcome::ALL {
                let dir = base.restart(CrashOutcome::ALL[0]);
                let (mut vault, _, _) = Vault::open(dir.clone(), &passphrase()).unwrap();
                dir.crash_at(dir.steps() + step);
                let result = operation(&mut vault);
                assert_eq!(result.is_ok(), !dir.crashed(), "{step}");
                let (reopened, plaintext, _) =
                    Vault::open(dir.restart(outcome), &passphrase()).unwrap();
                let found = &plaintext[..before.len().max(after.len())];
                let expected_before =
                    [before, &vec![0; after.len().saturating_sub(before.len())]].concat();
                let expected_after =
                    [after, &vec![0; before.len().saturating_sub(after.len())]].concat();
                assert!(
                    found == expected_before.as_slice() || found == expected_after.as_slice(),
                    "step {step} {outcome:?}"
                );
                if result.is_ok() {
                    assert_eq!(found, expected_after.as_slice(), "step {step} {outcome:?}");
                }
                // Nothing is left that a later write would trip over.
                let mut reopened = reopened;
                reopened.write(b"later").unwrap();
            }
        }
    }

    #[test]
    fn a_write_stopped_at_any_step_leaves_the_old_or_the_new_vault() {
        interrupted(b"old state", b"new state!", |vault| {
            vault.write(b"new state!")
        });
    }

    #[test]
    fn a_creation_stopped_at_any_step_leaves_no_vault_or_a_complete_one() {
        let probe = MemoryDir::new();
        Vault::create(probe.clone(), &passphrase(), KdfParams::FLOOR, b"first").unwrap();
        let total = probe.steps();
        for step in 1..=total + 1 {
            for outcome in CrashOutcome::ALL {
                let dir = MemoryDir::new();
                dir.crash_at(step);
                let created = Vault::create(dir.clone(), &passphrase(), KdfParams::FLOOR, b"first");
                assert_eq!(created.is_ok(), !dir.crashed());
                let after = dir.restart(outcome);
                match Vault::open(after.clone(), &passphrase()) {
                    Ok((_, plaintext, _)) => assert_eq!(&plaintext[..5], b"first"),
                    Err(StorageError::NotFound) => assert!(created.is_err()),
                    Err(StorageError::AuthenticationFailed | StorageError::BadFormat) => {
                        // A torn vault.new alone: it is left as it is.
                        assert!(created.is_err());
                        assert_eq!(after.names(), vec![VAULT_NEW.to_owned()]);
                    }
                    Err(error) => panic!("step {step} {outcome:?}: {error:?}"),
                }
            }
        }
    }

    #[test]
    fn a_damaged_vault_next_to_a_complete_new_one_is_left_for_the_user() {
        let dir = MemoryDir::new();
        let mut vault = Vault::create(dir.clone(), &passphrase(), KdfParams::FLOOR, b"a").unwrap();
        vault.write(b"b").unwrap();
        // A complete vault.new beside a vault that no longer authenticates.
        let good = dir.read(VAULT, MAX_VAULT_FILE_LEN).unwrap().unwrap();
        dir.write_new(VAULT_NEW, &good).unwrap();
        let mut bad = good.clone();
        bad[HEADER_LEN + 10] ^= 1;
        dir.overwrite(VAULT, &bad).unwrap();
        assert_eq!(
            Vault::open(dir.clone(), &passphrase()).err(),
            Some(StorageError::RecoveryNeeded)
        );
        // Nothing was removed or replaced.
        assert_eq!(dir.read(VAULT, MAX_VAULT_FILE_LEN).unwrap(), Some(bad));
        assert_eq!(dir.read(VAULT_NEW, MAX_VAULT_FILE_LEN).unwrap(), Some(good));
    }

    #[test]
    fn a_vault_on_disk_survives_a_new_process() {
        let path = std::env::temp_dir().join(format!("monolith-vault-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let dir = crate::dir::DiskDir::open(&path, true).unwrap();
        let mut vault = Vault::create(dir, &passphrase(), KdfParams::FLOOR, b"disk").unwrap();
        vault.write(b"disk two").unwrap();
        drop(vault);
        let dir = crate::dir::DiskDir::open(&path, false).unwrap();
        let (_, plaintext, recovery) = Vault::open(dir, &passphrase()).unwrap();
        assert_eq!(&plaintext[..8], b"disk two");
        assert_eq!(recovery, Recovery::Clean);
        std::fs::remove_dir_all(&path).unwrap();
    }
}
