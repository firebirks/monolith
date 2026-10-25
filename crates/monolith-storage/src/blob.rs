//! The staged data of a transfer: the blob format of
//! `docs/adr/0014-transfer-staging.md`.
//!
//! A blob is a 32-byte header and segments. Each segment holds
//! [`SEGMENT_LEN`] bytes of plaintext (the last one padded with zero bytes)
//! sealed with XChaCha20-Poly1305 under a key of its own blob, a random
//! nonce for every write, and the header, the size of the file, the index
//! of the segment and whether it is the last as associated data. A blob is
//! reserved at its full length when it is created, so its length says the
//! size of its file only to within a segment. Nothing here holds more than
//! one segment of plaintext at a time.

use std::io::Read;

use chacha20poly1305::aead::AeadInOut;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use monolith_protocol::limits::{MAX_FILE_CHUNK_LEN, MAX_FILE_SIZE};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::StorageError;
use crate::dir::{BlobFile, VaultDir};

/// The name of a blob: this prefix and 32 lower-case hexadecimal digits.
pub const BLOB_PREFIX: &str = "transfer-";

/// Plaintext bytes of a segment: the full chunk size of protocol version 1,
/// so that a segment is a chunk. A constant of blob format version 1.
pub const SEGMENT_LEN: usize = MAX_FILE_CHUNK_LEN;

/// The length of the header.
pub const HEADER_LEN: usize = 32;

const MAGIC: [u8; 8] = *b"MNLTBLB\0";
const FORMAT_VERSION: u16 = 1;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
const KEY_LEN: usize = 32;
const ID_LEN: usize = 16;
const BLOB_INFO: &[u8] = b"MONOLITH-TRANSFER-BLOB-V1";

/// Bytes a segment takes in the blob: nonce, sealed plaintext and tag.
pub const SEGMENT_SPAN: usize = NONCE_LEN + SEGMENT_LEN + TAG_LEN;

const SEGMENT_LEN_U64: u64 = SEGMENT_LEN as u64;
const SEGMENT_SPAN_U64: u64 = SEGMENT_SPAN as u64;
const HEADER_LEN_U64: u64 = HEADER_LEN as u64;

/// How many draws of a blob identifier may meet a name that exists before
/// creating a blob gives up.
const ID_DRAWS: usize = 8;

/// The length of a blob name.
const NAME_LEN: usize = BLOB_PREFIX.len() + 2 * ID_LEN;

/// The key from which the key of every blob is derived: from the vault key
/// in a persistent installation, random in an ephemeral one.
pub struct StagingKey(Zeroizing<[u8; KEY_LEN]>);

impl StagingKey {
    pub(crate) fn new(key: Zeroizing<[u8; KEY_LEN]>) -> Self {
        Self(key)
    }

    /// A random key, never written anywhere: for an ephemeral installation.
    pub fn random() -> Result<Self, StorageError> {
        let mut key = Zeroizing::new([0_u8; KEY_LEN]);
        getrandom::fill(key.as_mut_slice()).map_err(|_| StorageError::Randomness)?;
        Ok(Self(key))
    }

    /// A fixed key, for tests and test vectors only.
    pub fn from_bytes_for_testing(key: [u8; KEY_LEN]) -> Self {
        Self(Zeroizing::new(key))
    }

    /// `HKDF-SHA256(staging_key, salt = none, info = label || blob_id)`.
    fn blob_key(&self, id: &BlobId) -> Result<Zeroizing<[u8; KEY_LEN]>, StorageError> {
        let mut info = [0_u8; BLOB_INFO.len() + ID_LEN];
        let (label, rest) = info.split_at_mut(BLOB_INFO.len());
        label.copy_from_slice(BLOB_INFO);
        rest.copy_from_slice(&id.0);
        let mut key = Zeroizing::new([0_u8; KEY_LEN]);
        hkdf::Hkdf::<Sha256>::new(None, self.0.as_slice())
            .expand(&info, key.as_mut_slice())
            .map_err(|_| StorageError::Internal)?;
        Ok(key)
    }
}

impl core::fmt::Debug for StagingKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("StagingKey([redacted])")
    }
}

/// The identifier of a blob: 16 random bytes, never zero, which name its
/// file and enter its key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlobId([u8; ID_LEN]);

impl BlobId {
    /// The identifier from its bytes. `None` for all zeros, which a record
    /// uses for no blob.
    pub fn from_bytes(bytes: [u8; ID_LEN]) -> Option<Self> {
        (bytes != [0; ID_LEN]).then_some(Self(bytes))
    }

    /// The bytes of the identifier.
    pub fn as_bytes(&self) -> &[u8; ID_LEN] {
        &self.0
    }

    fn random() -> Result<Self, StorageError> {
        loop {
            let mut bytes = [0_u8; ID_LEN];
            getrandom::fill(&mut bytes).map_err(|_| StorageError::Randomness)?;
            if let Some(id) = Self::from_bytes(bytes) {
                return Ok(id);
            }
        }
    }

    /// The file name of the blob.
    pub fn name(&self) -> String {
        let mut name = String::with_capacity(NAME_LEN);
        name.push_str(BLOB_PREFIX);
        for byte in self.0 {
            for nibble in [byte >> 4, byte & 0x0F] {
                name.push(char::from_digit(u32::from(nibble), 16).unwrap_or('0'));
            }
        }
        name
    }

    /// The identifier named by `name`, if it is a blob name exactly: the
    /// prefix and 32 lower-case hexadecimal digits.
    pub fn from_name(name: &str) -> Option<Self> {
        let hex = name.strip_prefix(BLOB_PREFIX)?.as_bytes();
        if name.len() != NAME_LEN {
            return None;
        }
        let mut bytes = [0_u8; ID_LEN];
        for (byte, pair) in bytes.iter_mut().zip(hex.chunks_exact(2)) {
            let digit = |c: &u8| match c {
                b'0'..=b'9' => Some(c.wrapping_sub(b'0')),
                b'a'..=b'f' => Some(c.wrapping_sub(b'a').wrapping_add(10)),
                _ => None,
            };
            let (high, low) = (digit(pair.first()?)?, digit(pair.get(1)?)?);
            *byte = (high << 4) | low;
        }
        Self::from_bytes(bytes)
    }
}

/// The number of segments of a file of `size` bytes.
pub fn segment_count(size: u64) -> u64 {
    size.div_ceil(SEGMENT_LEN_U64)
}

/// The length a blob of a file of `size` bytes is reserved at. `None` for a
/// size above [`MAX_FILE_SIZE`].
pub fn reserved_len(size: u64) -> Option<u64> {
    if size > MAX_FILE_SIZE {
        return None;
    }
    segment_count(size)
        .checked_mul(SEGMENT_SPAN_U64)?
        .checked_add(HEADER_LEN_U64)
}

/// The bytes of the file that `segments` whole segments from the start
/// hold: the offset a FileAccept states.
pub fn held_bytes(segments: u64, size: u64) -> u64 {
    segments.saturating_mul(SEGMENT_LEN_U64).min(size)
}

/// The bytes of the file in segment `index`, without padding. `None` for
/// an index past the last segment.
pub fn chunk_len(index: u64, size: u64) -> Option<usize> {
    let start = index.checked_mul(SEGMENT_LEN_U64)?;
    let rest = size.checked_sub(start).filter(|rest| *rest > 0)?;
    usize::try_from(rest.min(SEGMENT_LEN_U64)).ok()
}

fn header(id: &BlobId) -> [u8; HEADER_LEN] {
    let mut header = [0_u8; HEADER_LEN];
    let (magic, rest) = header.split_at_mut(MAGIC.len());
    magic.copy_from_slice(&MAGIC);
    let (version, rest) = rest.split_at_mut(2);
    version.copy_from_slice(&FORMAT_VERSION.to_be_bytes());
    let (_reserved, rest) = rest.split_at_mut(2);
    let (blob_id, rest) = rest.split_at_mut(ID_LEN);
    blob_id.copy_from_slice(&id.0);
    rest.copy_from_slice(&u32::try_from(SEGMENT_LEN).unwrap_or(0).to_be_bytes());
    header
}

/// `header || u64 size || u64 index || u8 final`.
fn associated(id: &BlobId, size: u64, index: u64, last: bool) -> [u8; HEADER_LEN + 17] {
    let mut ad = [0_u8; HEADER_LEN + 17];
    let (head, rest) = ad.split_at_mut(HEADER_LEN);
    head.copy_from_slice(&header(id));
    let (size_bytes, rest) = rest.split_at_mut(8);
    size_bytes.copy_from_slice(&size.to_be_bytes());
    let (index_bytes, flag) = rest.split_at_mut(8);
    index_bytes.copy_from_slice(&index.to_be_bytes());
    flag.copy_from_slice(&[u8::from(last)]);
    ad
}

/// An open blob of a known record: its file, its key and the size of its
/// file, which the record holds and the store authenticates.
pub struct Blob {
    file: Box<dyn BlobFile>,
    id: BlobId,
    size: u64,
    cipher: XChaCha20Poly1305,
    /// One segment as it is on disk, reused for every read and write.
    buffer: Zeroizing<Vec<u8>>,
}

impl core::fmt::Debug for Blob {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Blob")
            .field("id", &self.id)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl Blob {
    fn with_file(
        file: Box<dyn BlobFile>,
        key: &StagingKey,
        id: BlobId,
        size: u64,
    ) -> Result<Self, StorageError> {
        let blob_key = key.blob_key(&id)?;
        let cipher = XChaCha20Poly1305::new_from_slice(blob_key.as_slice())
            .map_err(|_| StorageError::Internal)?;
        let mut buffer = Zeroizing::new(Vec::new());
        buffer
            .try_reserve_exact(SEGMENT_SPAN)
            .map_err(|_| StorageError::TooLarge)?;
        buffer.resize(SEGMENT_SPAN, 0);
        Ok(Self {
            file,
            id,
            size,
            cipher,
            buffer,
        })
    }

    /// Creates a blob for a file of `size` bytes under a fresh random
    /// identifier: reserved at its full length with `margin` bytes left
    /// free, its header written, the file and the directory synced. The
    /// caller writes the record that names it afterwards.
    pub fn create(
        dir: &dyn VaultDir,
        key: &StagingKey,
        size: u64,
        margin: u64,
    ) -> Result<Self, StorageError> {
        let len = reserved_len(size).ok_or(StorageError::TooLarge)?;
        let mut draws = 0;
        let (id, file) = loop {
            let id = BlobId::random()?;
            match dir.create_reserved(&id.name(), len, margin) {
                Ok(file) => break (id, file),
                Err(StorageError::Exists) if draws < ID_DRAWS => {
                    draws = draws.saturating_add(1);
                }
                Err(error) => return Err(error),
            }
        };
        let mut blob = Self::with_file(file, key, id, size)?;
        let written = blob
            .file
            .write_at(0, &header(&id))
            .and_then(|()| blob.file.sync())
            .and_then(|()| dir.sync());
        if let Err(error) = written {
            drop(blob);
            let _ = dir.remove(&id.name());
            return Err(error);
        }
        Ok(blob)
    }

    /// Opens the blob `id` of a record for a file of `size` bytes. `None`
    /// if there is no such file. Fails with [`StorageError::BadFormat`] if
    /// its header or its length is not what the record says.
    pub fn open(
        dir: &dyn VaultDir,
        key: &StagingKey,
        id: BlobId,
        size: u64,
    ) -> Result<Option<Self>, StorageError> {
        let len = reserved_len(size).ok_or(StorageError::BadFormat)?;
        let Some(mut file) = dir.open_blob(&id.name())? else {
            return Ok(None);
        };
        if file.length()? != len {
            return Err(StorageError::BadFormat);
        }
        let mut found = [0_u8; HEADER_LEN];
        file.read_at(0, &mut found)?;
        if found != header(&id) {
            return Err(StorageError::BadFormat);
        }
        Self::with_file(file, key, id, size).map(Some)
    }

    /// The identifier of the blob.
    pub fn id(&self) -> BlobId {
        self.id
    }

    /// The size of its file.
    pub fn size(&self) -> u64 {
        self.size
    }

    fn position(&self, index: u64) -> Result<u64, StorageError> {
        index
            .checked_mul(SEGMENT_SPAN_U64)
            .and_then(|offset| offset.checked_add(HEADER_LEN_U64))
            .ok_or(StorageError::TooLarge)
    }

    fn last(&self, index: u64) -> bool {
        index.saturating_add(1) == segment_count(self.size)
    }

    /// Seals `data`, the bytes of the file in segment `index`, under a new
    /// random nonce and writes the segment at its position. `data` must be
    /// exactly what [`chunk_len`] says.
    pub fn write_segment(&mut self, index: u64, data: &[u8]) -> Result<(), StorageError> {
        if chunk_len(index, self.size) != Some(data.len()) {
            return Err(StorageError::Internal);
        }
        let position = self.position(index)?;
        let ad = associated(&self.id, self.size, index, self.last(index));
        let (nonce_space, rest) = self.buffer.split_at_mut(NONCE_LEN);
        let mut nonce = [0_u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|_| StorageError::Randomness)?;
        nonce_space.copy_from_slice(&nonce);
        let nonce = XNonce::from(nonce);
        let (plaintext, tag_space) = rest.split_at_mut(SEGMENT_LEN);
        let (file_bytes, padding) = plaintext.split_at_mut(data.len());
        file_bytes.copy_from_slice(data);
        padding.fill(0);
        let tag = self
            .cipher
            .encrypt_inout_detached(&nonce, &ad, plaintext.into())
            .map_err(|_| StorageError::Internal)?;
        tag_space.copy_from_slice(&tag);
        self.file.write_at(position, &self.buffer)
    }

    /// Reads segment `index`, authenticates it and copies the bytes of the
    /// file it holds into `out`, returning how many there are. Fails with
    /// [`StorageError::AuthenticationFailed`] for a segment that does not
    /// authenticate under its index, or whose padding is not zero.
    pub fn read_segment(
        &mut self,
        index: u64,
        out: &mut [u8; SEGMENT_LEN],
    ) -> Result<usize, StorageError> {
        let len = chunk_len(index, self.size).ok_or(StorageError::Internal)?;
        let position = self.position(index)?;
        self.file.read_at(position, &mut self.buffer)?;
        let ad = associated(&self.id, self.size, index, self.last(index));
        let (nonce, rest) = self.buffer.split_at_mut(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = (&*nonce).try_into().map_err(|_| StorageError::Internal)?;
        let nonce = XNonce::from(nonce);
        let (plaintext, tag) = rest.split_at_mut(SEGMENT_LEN);
        let tag: [u8; TAG_LEN] = (&*tag).try_into().map_err(|_| StorageError::Internal)?;
        self.cipher
            .decrypt_inout_detached(&nonce, &ad, plaintext.into(), &tag.into())
            .map_err(|_| StorageError::AuthenticationFailed)?;
        let (file_bytes, padding) = plaintext.split_at(len);
        if padding.iter().any(|byte| *byte != 0) {
            return Err(StorageError::AuthenticationFailed);
        }
        out.get_mut(..len)
            .ok_or(StorageError::Internal)?
            .copy_from_slice(file_bytes);
        Ok(len)
    }

    /// Flushes what was written to stable storage.
    pub fn sync(&mut self) -> Result<(), StorageError> {
        self.file.sync()
    }

    /// The number of segments from the start that authenticate, up to the
    /// first that does not: how much of an incoming accepted transfer is
    /// staged (`docs/adr/0014`, "how much is staged").
    pub fn scan(&mut self) -> Result<u64, StorageError> {
        let mut segment = Zeroizing::new([0_u8; SEGMENT_LEN]);
        let mut good = 0;
        while good < segment_count(self.size) {
            match self.read_segment(good, &mut segment) {
                Ok(_) => good = good.saturating_add(1),
                Err(StorageError::AuthenticationFailed) => break,
                Err(error) => return Err(error),
            }
        }
        Ok(good)
    }

    /// The SHA-256 of the file: every segment read, authenticated and
    /// hashed in order. Fails at the first segment that does not
    /// authenticate.
    pub fn digest(&mut self) -> Result<[u8; 32], StorageError> {
        let mut hash = Sha256::new();
        let mut segment = Zeroizing::new([0_u8; SEGMENT_LEN]);
        for index in 0..segment_count(self.size) {
            let len = self.read_segment(index, &mut segment)?;
            hash.update(segment.get(..len).ok_or(StorageError::Internal)?);
        }
        Ok(hash.finalize().into())
    }
}

/// Stages a whole file of `size` bytes read from `source` in a new blob
/// (`docs/adr/0014`): the blob is created and reserved, every segment
/// written as it is read and the plaintext hashed, and the file and the
/// directory synced. Fails, and removes the blob, if the source gives
/// fewer or more than `size` bytes. Returns the blob and the SHA-256 of
/// the file.
pub fn stage(
    dir: &dyn VaultDir,
    key: &StagingKey,
    size: u64,
    margin: u64,
    source: &mut dyn Read,
) -> Result<(BlobId, [u8; 32]), StorageError> {
    let blob = Blob::create(dir, key, size, margin)?;
    let id = blob.id();
    fill(dir, blob, source).map(|(_, digest)| (id, digest))
}

/// Writes the whole file of `size` bytes read from `source` into `blob`,
/// just created for it, as [`stage`] does once the blob exists: every
/// segment written as it is read and the plaintext hashed, and the file
/// synced. Fails, and removes the blob from `dir`, if the source gives
/// fewer or more than `size` bytes or a write fails. Returns the blob,
/// still open, with the SHA-256 of the file.
pub fn fill(
    dir: &dyn VaultDir,
    mut blob: Blob,
    source: &mut dyn Read,
) -> Result<(Blob, [u8; 32]), StorageError> {
    match write_all(&mut blob, source) {
        Ok(digest) => Ok((blob, digest)),
        Err(error) => {
            let id = blob.id();
            drop(blob);
            let _ = dir.remove(&id.name());
            Err(error)
        }
    }
}

fn write_all(blob: &mut Blob, source: &mut dyn Read) -> Result<[u8; 32], StorageError> {
    let size = blob.size();
    let mut hash = Sha256::new();
    let mut segment = Zeroizing::new([0_u8; SEGMENT_LEN]);
    for index in 0..segment_count(size) {
        let len = chunk_len(index, size).ok_or(StorageError::Internal)?;
        let part = segment.get_mut(..len).ok_or(StorageError::Internal)?;
        source.read_exact(part).map_err(|_| StorageError::Io)?;
        hash.update(&*part);
        blob.write_segment(index, part)?;
    }
    // The source must end where the size said.
    let mut extra = [0_u8; 1];
    loop {
        match source.read(&mut extra) {
            Ok(0) => break,
            Ok(_) => return Err(StorageError::TooLarge),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Err(StorageError::Io),
        }
    }
    blob.sync()?;
    Ok(hash.finalize().into())
}

/// Reads the file of a complete blob for saving: segment by segment,
/// authenticated, decrypted and hashed. After the last byte it returns an
/// error, never the end of the file, unless the hash is `digest`
/// (`docs/adr/0014`, "Saving a received file").
pub struct BlobReader {
    blob: Blob,
    digest: [u8; 32],
    hash: Sha256,
    /// One segment of plaintext, on the heap: a reader is moved around
    /// by value.
    segment: Zeroizing<Vec<u8>>,
    next: u64,
    held: usize,
    taken: usize,
    checked: bool,
}

impl core::fmt::Debug for BlobReader {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BlobReader")
            .field("blob", &self.blob)
            .field("next", &self.next)
            .finish_non_exhaustive()
    }
}

impl BlobReader {
    /// A reader of `blob` that checks its file against `digest`.
    pub fn new(blob: Blob, digest: [u8; 32]) -> Self {
        Self {
            blob,
            digest,
            hash: Sha256::new(),
            segment: Zeroizing::new(vec![0_u8; SEGMENT_LEN]),
            next: 0,
            held: 0,
            taken: 0,
            checked: false,
        }
    }
}

impl Read for BlobReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.taken == self.held {
            if self.next == segment_count(self.blob.size()) {
                if !self.checked {
                    let hash = core::mem::take(&mut self.hash).finalize();
                    if hash.as_slice() != self.digest {
                        return Err(std::io::Error::other("the file does not match its digest"));
                    }
                    self.checked = true;
                }
                return Ok(0);
            }
            let segment: &mut [u8; SEGMENT_LEN] = self
                .segment
                .as_mut_slice()
                .try_into()
                .map_err(|_| std::io::Error::other("a segment buffer of the wrong length"))?;
            let len = self
                .blob
                .read_segment(self.next, segment)
                .map_err(|_| std::io::Error::other("a segment does not authenticate"))?;
            self.hash
                .update(self.segment.get(..len).unwrap_or_default());
            self.next = self.next.saturating_add(1);
            self.held = len;
            self.taken = 0;
        }
        let available = self.segment.get(self.taken..self.held).unwrap_or_default();
        let count = available.len().min(buf.len());
        buf.get_mut(..count)
            .unwrap_or_default()
            .copy_from_slice(available.get(..count).unwrap_or_default());
        self.taken = self.taken.saturating_add(count);
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dir::{CrashOutcome, MemoryDir};

    const L: usize = SEGMENT_LEN;

    fn key() -> StagingKey {
        StagingKey::from_bytes_for_testing([7; 32])
    }

    fn data(len: usize) -> Vec<u8> {
        (0..len).map(|i| u8::try_from(i % 251).unwrap()).collect()
    }

    fn sha(bytes: &[u8]) -> [u8; 32] {
        Sha256::digest(bytes).into()
    }

    fn segment_of(source: &[u8], index: usize) -> &[u8] {
        &source[index * L..((index + 1) * L).min(source.len())]
    }

    #[test]
    fn a_staged_file_reads_back_exactly_at_every_size_boundary() {
        for len in [0, 1, L - 1, L, L + 1, 3 * L + 5] {
            let dir = MemoryDir::new();
            let source = data(len);
            let size = len as u64;
            let (id, digest) = stage(&dir, &key(), size, 0, &mut source.as_slice()).unwrap();
            assert_eq!(digest, sha(&source), "{len}");
            assert_eq!(
                dir.content(&id.name()).unwrap().len() as u64,
                reserved_len(size).unwrap()
            );
            let mut blob = Blob::open(&dir, &key(), id, size).unwrap().unwrap();
            assert_eq!(blob.scan().unwrap(), segment_count(size));
            assert_eq!(blob.digest().unwrap(), digest);
            let mut read = Vec::new();
            BlobReader::new(blob, digest)
                .read_to_end(&mut read)
                .unwrap();
            assert_eq!(read, source, "{len}");
        }
    }

    #[test]
    fn the_length_of_a_blob_shows_the_size_only_to_within_a_segment() {
        assert_eq!(reserved_len(1), reserved_len(L as u64));
        assert_eq!(reserved_len(0), Some(HEADER_LEN as u64));
        assert_eq!(reserved_len(MAX_FILE_SIZE + 1), None);
        assert_eq!(chunk_len(0, 0), None);
        assert_eq!(chunk_len(1, L as u64 + 1), Some(1));
        assert_eq!(chunk_len(2, L as u64 + 1), None);
    }

    #[test]
    fn a_source_that_gives_fewer_or_more_bytes_than_its_size_stages_nothing() {
        for (declared, given) in [(10, 9), (10, 11), (L as u64 + 1, L)] {
            let dir = MemoryDir::new();
            let source = data(given);
            assert!(stage(&dir, &key(), declared, 0, &mut source.as_slice()).is_err());
            assert!(dir.list().unwrap().is_empty(), "{declared} {given}");
        }
    }

    #[test]
    fn a_segment_changed_moved_or_taken_from_another_blob_does_not_authenticate() {
        let dir = MemoryDir::new();
        let len = 3 * L;
        let source = data(len);
        let size = len as u64;
        let (id, digest) = stage(&dir, &key(), size, 0, &mut source.as_slice()).unwrap();
        let (other, _) = stage(&dir, &key(), size, 0, &mut source.as_slice()).unwrap();
        let original = dir.content(&id.name()).unwrap();
        let span = |index: usize| {
            HEADER_LEN + index * SEGMENT_SPAN..HEADER_LEN + (index + 1) * SEGMENT_SPAN
        };

        // A byte changed in segment 1.
        let mut changed = original.clone();
        changed[span(1).start + 100] ^= 1;
        // Segment 0 copied over segment 1.
        let mut moved = original.clone();
        let first = original[span(0)].to_vec();
        moved[span(1)].copy_from_slice(&first);
        // Segment 1 of the other blob, whose key differs.
        let mut foreign = original.clone();
        foreign[span(1)].copy_from_slice(&dir.content(&other.name()).unwrap()[span(1)]);

        for bytes in [changed, moved, foreign] {
            dir.overwrite(&id.name(), &bytes).unwrap();
            let mut blob = Blob::open(&dir, &key(), id, size).unwrap().unwrap();
            let mut out = [0; L];
            assert!(blob.read_segment(0, &mut out).is_ok());
            assert_eq!(
                blob.read_segment(1, &mut out),
                Err(StorageError::AuthenticationFailed)
            );
            assert_eq!(blob.scan().unwrap(), 1);
            assert!(blob.digest().is_err());
            let mut read = Vec::new();
            assert!(
                BlobReader::new(blob, digest)
                    .read_to_end(&mut read)
                    .is_err()
            );
        }
    }

    #[test]
    fn a_blob_is_read_only_under_the_size_identifier_and_key_of_its_record() {
        let dir = MemoryDir::new();
        let source = data(L + 3);
        let size = source.len() as u64;
        let (id, _) = stage(&dir, &key(), size, 0, &mut source.as_slice()).unwrap();
        // Another size within the same segments: every segment fails.
        let mut blob = Blob::open(&dir, &key(), id, size + 1).unwrap().unwrap();
        assert_eq!(blob.scan().unwrap(), 0);
        // Another number of segments: refused at open.
        assert_eq!(
            Blob::open(&dir, &key(), id, 3).map(|_| ()),
            Err(StorageError::BadFormat)
        );
        // The bytes of this blob under the name of another: the header
        // names another identifier.
        let other = BlobId::from_bytes([9; 16]).unwrap();
        let mut copy = dir.create_reserved(&other.name(), 0, 0).unwrap();
        copy.write_at(0, &dir.content(&id.name()).unwrap()).unwrap();
        assert_eq!(
            Blob::open(&dir, &key(), other, size).map(|_| ()),
            Err(StorageError::BadFormat)
        );
        // No such file.
        let unknown = BlobId::from_bytes([10; 16]).unwrap();
        assert!(Blob::open(&dir, &key(), unknown, size).unwrap().is_none());
        // Another key: nothing authenticates.
        let mut wrong = Blob::open(&dir, &StagingKey::from_bytes_for_testing([8; 32]), id, size)
            .unwrap()
            .unwrap();
        assert_eq!(wrong.scan().unwrap(), 0);
    }

    #[test]
    fn padding_that_is_not_zero_does_not_authenticate() {
        let dir = MemoryDir::new();
        let mut blob = Blob::create(&dir, &key(), 5, 0).unwrap();
        let id = blob.id();
        // Seal the last segment with a byte of padding that is not zero.
        let mut plaintext = vec![0_u8; L];
        plaintext[..5].copy_from_slice(b"hello");
        plaintext[5] = 1;
        let mut nonce = [0_u8; NONCE_LEN];
        nonce[0] = 3;
        let ad = associated(&id, 5, 0, true);
        let blob_key = key().blob_key(&id).unwrap();
        let cipher = XChaCha20Poly1305::new_from_slice(blob_key.as_slice()).unwrap();
        let tag = cipher
            .encrypt_inout_detached(&XNonce::from(nonce), &ad, plaintext.as_mut_slice().into())
            .unwrap();
        let mut segment = nonce.to_vec();
        segment.extend_from_slice(&plaintext);
        segment.extend_from_slice(&tag);
        blob.file.write_at(HEADER_LEN as u64, &segment).unwrap();
        let mut out = [0; L];
        assert_eq!(
            blob.read_segment(0, &mut out),
            Err(StorageError::AuthenticationFailed)
        );
    }

    #[test]
    fn after_a_crash_the_scan_stops_at_the_first_segment_that_does_not_authenticate() {
        let dir = MemoryDir::new();
        let len = 4 * L;
        let source = data(len);
        let size = len as u64;
        let mut blob = Blob::create(&dir, &key(), size, 0).unwrap();
        let id = blob.id();
        for index in 0..2 {
            blob.write_segment(index as u64, segment_of(&source, index))
                .unwrap();
        }
        blob.sync().unwrap();
        // Segments 2 and 3 not synced: 2 survives torn, 3 whole.
        for index in 2..4 {
            blob.write_segment(index as u64, segment_of(&source, index))
                .unwrap();
        }
        let torn = dir.restart(CrashOutcome {
            keep_unsynced_names: false,
            keep_torn_contents: true,
        });
        let mut after = Blob::open(&torn, &key(), id, size).unwrap().unwrap();
        assert_eq!(after.scan().unwrap(), 2);
        // Written again, segment 2 lets the scan count the old segment 3
        // too: the own ciphertext of this blob for that position.
        after.write_segment(2, segment_of(&source, 2)).unwrap();
        assert_eq!(after.scan().unwrap(), 4);
        assert_eq!(after.digest().unwrap(), sha(&source));
        let lost = dir.restart(CrashOutcome::ALL[0]);
        let mut after = Blob::open(&lost, &key(), id, size).unwrap().unwrap();
        assert_eq!(after.scan().unwrap(), 2);
        assert_eq!(held_bytes(2, size), 2 * L as u64);
        assert_eq!(held_bytes(5, size), size);
    }

    #[test]
    fn the_reader_fails_at_the_end_when_the_digest_differs() {
        let dir = MemoryDir::new();
        let source = data(100);
        let (id, digest) = stage(&dir, &key(), 100, 0, &mut source.as_slice()).unwrap();
        let mut wrong = digest;
        wrong[0] ^= 1;
        let blob = Blob::open(&dir, &key(), id, 100).unwrap().unwrap();
        let mut reader = BlobReader::new(blob, wrong);
        let mut buf = vec![0; 100];
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(buf, source);
        assert!(reader.read(&mut [0; 1]).is_err());
        // An empty file has a digest too.
        let (empty, empty_digest) = stage(&dir, &key(), 0, 0, &mut [].as_slice()).unwrap();
        assert_eq!(empty_digest, sha(&[]));
        let blob = Blob::open(&dir, &key(), empty, 0).unwrap().unwrap();
        assert!(BlobReader::new(blob, wrong).read(&mut [0; 1]).is_err());
        let blob = Blob::open(&dir, &key(), empty, 0).unwrap().unwrap();
        assert_eq!(
            BlobReader::new(blob, empty_digest)
                .read(&mut [0; 1])
                .unwrap(),
            0
        );
    }

    #[test]
    fn a_blob_name_is_the_prefix_and_32_lower_case_hexadecimal_digits() {
        let id = BlobId::from_bytes([0xAB; 16]).unwrap();
        assert_eq!(id.name(), format!("transfer-{}", "ab".repeat(16)));
        assert_eq!(BlobId::from_name(&id.name()), Some(id));
        for name in [
            "transfers".to_owned(),
            "transfers.new".to_owned(),
            "transfer-".to_owned(),
            format!("transfer-{}", "AB".repeat(16)),
            format!("transfer-{}", "ab".repeat(15)),
            format!("transfer-{}0", "ab".repeat(16)),
            format!("transfer-{}", "00".repeat(16)),
        ] {
            assert_eq!(BlobId::from_name(&name), None, "{name}");
        }
        assert_eq!(BlobId::from_bytes([0; 16]), None);
    }

    #[test]
    fn a_reservation_without_room_creates_no_blob() {
        let dir = MemoryDir::new();
        dir.set_free(Some(1000));
        assert!(matches!(
            Blob::create(&dir, &key(), L as u64, 0),
            Err(StorageError::Full)
        ));
        assert!(dir.list().unwrap().is_empty());
    }
}
