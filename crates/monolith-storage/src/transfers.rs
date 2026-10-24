//! The transfer store of `docs/adr/0014-transfer-staging.md`: the records
//! of every transfer of the installation, in one file, `transfers`, beside
//! the vault and under its lock.
//!
//! It is built as the message store is (`docs/adr/0008-message-store.md`):
//! rewritten whole and replaced atomically, its payload encrypted with
//! XChaCha20-Poly1305 under a key that HKDF-SHA256 derives from the vault
//! key ([`crate::vault::Vault::transfer_store_key`]), the whole header as
//! associated data.
//!
//! ```text
//! offset  size  field
//! 0       8     magic "MNLTXFR" 0x00
//! 8       2     format version (1)
//! 10      2     reserved (0)
//! 12      8     generation
//! 20      24    nonce
//! 44      4     payload length, tag included
//! 48      n     payload
//! ```
//!
//! The payload is a version, the length of the records, the records and
//! zero padding to a multiple of 64 KiB. Each record is checked when it is
//! read and before it is written, so what is written always reads back.

use monolith_protocol::limits::{
    MAX_FILE_SIZE, MAX_TRANSFER_RECORDS, MAX_TRANSFER_STORE_FILE_LEN,
    TRANSFER_STORE_PADDING_BLOCK_LEN,
};
use monolith_protocol::text::Filename;
use zeroize::Zeroizing;

use crate::StorageError;
use crate::blob::BlobId;
use crate::dir::{TRANSFERS, TRANSFERS_NEW, VaultDir};
use crate::record::{ContactId, IdentityId, RECORD_ID_LEN};
use crate::vault::{open_with, seal_with};

/// The first eight bytes of a transfer store.
const MAGIC: [u8; 8] = *b"MNLTXFR\0";

/// The format version this build writes and reads.
pub const FORMAT_VERSION: u16 = 1;

/// The payload version this build writes and reads.
pub const PAYLOAD_VERSION: u16 = 1;

const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;
const TAG_LEN: usize = 16;
const TRANSFER_ID_LEN: usize = 16;
const DIGEST_LEN: usize = 32;

/// The whole header, up to the payload.
pub const HEADER_LEN: usize = 48;

/// The key of a transfer store. Erased when dropped; prints nothing.
pub struct TransferStoreKey(Zeroizing<[u8; KEY_LEN]>);

impl TransferStoreKey {
    pub(crate) const fn new(key: Zeroizing<[u8; KEY_LEN]>) -> Self {
        Self(key)
    }

    /// A random key, never written anywhere: for an ephemeral installation.
    pub fn random() -> Result<Self, StorageError> {
        Ok(Self(Zeroizing::new(random::<KEY_LEN>()?)))
    }

    /// A key made from fixed bytes, for tests and test vectors. A store
    /// sealed under it protects nothing.
    #[doc(hidden)]
    pub fn from_bytes_for_testing(bytes: [u8; KEY_LEN]) -> Self {
        Self(Zeroizing::new(bytes))
    }
}

impl core::fmt::Debug for TransferStoreKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("TransferStoreKey([redacted])")
    }
}

/// Which way a transfer goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    /// This side offers the file.
    Outgoing,
    /// The peer offers the file.
    Incoming,
}

/// The state of a transfer in the store (`docs/adr/0014`, codes 1 to 22).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum State {
    /// Outgoing: staged, not yet accepted.
    Waiting,
    /// Outgoing: accepted, being sent.
    Sending,
    /// Outgoing: the receiver acknowledged the whole file.
    Delivered,
    /// Incoming: offered, the user has not decided.
    Offered,
    /// Incoming: accepted, not complete.
    Accepted,
    /// Incoming: the whole file is staged and verified.
    Complete,
    /// Incoming: the offer was not decided in time.
    Expired,
    /// Rejected by the receiver.
    Rejected,
    /// Cancelled by either side.
    Cancelled,
    /// Failed; the reason says why.
    Failed,
}

impl State {
    const fn code(self) -> u8 {
        match self {
            Self::Waiting => 1,
            Self::Sending => 2,
            Self::Delivered => 3,
            Self::Offered => 11,
            Self::Accepted => 12,
            Self::Complete => 13,
            Self::Expired => 14,
            Self::Rejected => 20,
            Self::Cancelled => 21,
            Self::Failed => 22,
        }
    }

    const fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            1 => Self::Waiting,
            2 => Self::Sending,
            3 => Self::Delivered,
            11 => Self::Offered,
            12 => Self::Accepted,
            13 => Self::Complete,
            14 => Self::Expired,
            20 => Self::Rejected,
            21 => Self::Cancelled,
            22 => Self::Failed,
            _ => return None,
        })
    }

    /// Whether the state belongs to `direction`.
    pub const fn fits(self, direction: Direction) -> bool {
        match self {
            Self::Waiting | Self::Sending | Self::Delivered => {
                matches!(direction, Direction::Outgoing)
            }
            Self::Offered | Self::Accepted | Self::Complete | Self::Expired => {
                matches!(direction, Direction::Incoming)
            }
            Self::Rejected | Self::Cancelled | Self::Failed => true,
        }
    }
}

/// Why a transfer failed or who cancelled it (`docs/adr/0014`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reason {
    /// Failed: the staged data could not be written or read.
    Storage,
    /// Failed: the digest did not match.
    Digest,
    /// Failed: no progress for the stall limit.
    Stalled,
    /// Failed, incoming only: the offerer no longer has the transfer.
    OffererGone,
    /// Cancelled by this side.
    ByUs,
    /// Cancelled by the peer.
    ByPeer,
    /// Cancelled because the contact was blocked or deleted.
    ContactEnded,
}

impl Reason {
    const fn code(self) -> u8 {
        match self {
            Self::Storage => 1,
            Self::Digest => 2,
            Self::Stalled => 3,
            Self::OffererGone => 4,
            Self::ByUs => 16,
            Self::ByPeer => 17,
            Self::ContactEnded => 18,
        }
    }

    const fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            1 => Self::Storage,
            2 => Self::Digest,
            3 => Self::Stalled,
            4 => Self::OffererGone,
            16 => Self::ByUs,
            17 => Self::ByPeer,
            18 => Self::ContactEnded,
            _ => return None,
        })
    }

    /// Whether the reason belongs to `state` in `direction`.
    pub const fn fits(self, state: State, direction: Direction) -> bool {
        match self {
            Self::Storage | Self::Digest | Self::Stalled => matches!(state, State::Failed),
            Self::OffererGone => {
                matches!(state, State::Failed) && matches!(direction, Direction::Incoming)
            }
            Self::ByUs | Self::ByPeer | Self::ContactEnded => matches!(state, State::Cancelled),
        }
    }
}

/// The flags of a record (`docs/adr/0014`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Flags(u8);

impl Flags {
    /// The outgoing offer is known to have been read by the peer.
    pub const OFFER_READ: u8 = 1 << 0;
    /// The peer is not known to have read a decision, a cancellation or a
    /// final answer.
    pub const UNREAD: u8 = 1 << 1;
    /// The incoming file was saved.
    pub const SAVED: u8 = 1 << 2;
    /// A save of the incoming file began and did not end.
    pub const SAVING: u8 = 1 << 3;
    /// The user removed the ended incoming transfer from the interface.
    pub const REMOVED: u8 = 1 << 4;
    const ALL: u8 = Self::OFFER_READ | Self::UNREAD | Self::SAVED | Self::SAVING | Self::REMOVED;

    /// The flags from their bits. `None` if a reserved bit is set.
    pub const fn from_bits(bits: u8) -> Option<Self> {
        if bits & !Self::ALL != 0 {
            None
        } else {
            Some(Self(bits))
        }
    }

    /// The bits.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether `flag` is set.
    pub const fn has(self, flag: u8) -> bool {
        self.0 & flag != 0
    }

    /// The flags with `flag` set or cleared.
    #[must_use]
    pub const fn with(self, flag: u8, on: bool) -> Self {
        if on {
            Self(self.0 | flag)
        } else {
            Self(self.0 & !flag)
        }
    }
}

/// The record of one transfer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferRecord {
    /// The 16 bytes the offerer drew.
    pub transfer: [u8; TRANSFER_ID_LEN],
    /// The local identity.
    pub identity: IdentityId,
    /// The contact record.
    pub contact: ContactId,
    /// Which way the file goes.
    pub direction: Direction,
    /// Where the transfer is.
    pub state: State,
    /// What else is known of it.
    pub flags: Flags,
    /// Why it failed, or who cancelled it.
    pub reason: Option<Reason>,
    /// The declared size of the file.
    pub size: u64,
    /// The filename of the offer.
    pub name: Filename,
    /// Outgoing: of the staged file; incoming: as verified, zero before.
    pub digest: [u8; DIGEST_LEN],
    /// The blob that holds the staged data, if there is one.
    pub blob: Option<BlobId>,
    /// Unix time when the record was made.
    pub created: u64,
    /// The time the state says (`docs/adr/0014`), or 0.
    pub deadline: u64,
}

impl TransferRecord {
    /// Whether the state has a blob.
    pub fn has_blob(&self) -> bool {
        match self.state {
            State::Waiting | State::Sending | State::Accepted => true,
            State::Complete => !self.flags.has(Flags::SAVED) && !self.flags.has(Flags::REMOVED),
            _ => false,
        }
    }

    /// Whether the record can stall: its deadline is a stall deadline.
    pub fn can_stall(&self) -> bool {
        match self.state {
            State::Waiting | State::Sending | State::Accepted => true,
            State::Cancelled => {
                self.direction == Direction::Outgoing && self.flags.has(Flags::UNREAD)
            }
            _ => false,
        }
    }

    /// Whether the record is an ended incoming record, whose deadline says
    /// when it may go.
    pub fn is_ended_incoming(&self) -> bool {
        self.direction == Direction::Incoming
            && match self.state {
                State::Rejected | State::Expired | State::Cancelled | State::Failed => true,
                State::Complete => self.flags.has(Flags::SAVED) || self.flags.has(Flags::REMOVED),
                _ => false,
            }
    }

    /// Checks the rules of `docs/adr/0014` that one record must keep.
    pub fn check(&self) -> Result<(), StorageError> {
        let bad = Err(StorageError::BadFormat);
        if self.transfer == [0; TRANSFER_ID_LEN] || self.size > MAX_FILE_SIZE {
            return bad;
        }
        if !self.state.fits(self.direction) {
            return bad;
        }
        let failed_or_cancelled = matches!(self.state, State::Failed | State::Cancelled);
        match self.reason {
            Some(reason) if failed_or_cancelled && reason.fits(self.state, self.direction) => {}
            None if !failed_or_cancelled => {}
            _ => return bad,
        }
        let flags = self.flags;
        let outgoing = self.direction == Direction::Outgoing;
        if flags.has(Flags::OFFER_READ) && !outgoing {
            return bad;
        }
        let unread_allowed = match self.state {
            State::Cancelled => true,
            State::Accepted
            | State::Complete
            | State::Rejected
            | State::Expired
            | State::Failed => !outgoing,
            _ => false,
        };
        if flags.has(Flags::UNREAD) && !unread_allowed {
            return bad;
        }
        let complete_incoming = !outgoing && self.state == State::Complete;
        if (flags.has(Flags::SAVED) || flags.has(Flags::SAVING)) && !complete_incoming {
            return bad;
        }
        if flags.has(Flags::SAVED) && flags.has(Flags::SAVING) {
            return bad;
        }
        let ended_incoming_state = !outgoing
            && matches!(
                self.state,
                State::Rejected
                    | State::Expired
                    | State::Cancelled
                    | State::Failed
                    | State::Complete
            );
        if flags.has(Flags::REMOVED) && !ended_incoming_state {
            return bad;
        }
        if self.has_blob() != self.blob.is_some() {
            return bad;
        }
        let digest_needed = outgoing || self.state == State::Complete;
        if digest_needed == (self.digest == [0; DIGEST_LEN]) {
            return bad;
        }
        let deadline_needed = (!outgoing && self.state == State::Offered)
            || self.can_stall()
            || self.is_ended_incoming();
        if deadline_needed == (self.deadline == 0) {
            return bad;
        }
        Ok(())
    }
}

/// The records of the transfer store, as the payload holds them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransferRecords {
    /// Every record, in the order they are kept.
    pub records: Vec<TransferRecord>,
}

impl TransferRecords {
    /// Checks every record and the rules between them: at most
    /// [`MAX_TRANSFER_RECORDS`], no two with the same key, no two with the
    /// same blob.
    pub fn check(&self) -> Result<(), StorageError> {
        if self.records.len() > MAX_TRANSFER_RECORDS {
            return Err(StorageError::TooLarge);
        }
        for (index, record) in self.records.iter().enumerate() {
            record.check()?;
            let later = self
                .records
                .get(index.saturating_add(1)..)
                .unwrap_or_default();
            for other in later {
                let same_key = other.transfer == record.transfer
                    && other.identity == record.identity
                    && other.contact == record.contact
                    && other.direction == record.direction;
                let same_blob = record.blob.is_some() && other.blob == record.blob;
                if same_key || same_blob {
                    return Err(StorageError::BadFormat);
                }
            }
        }
        Ok(())
    }

    /// The payload: version, records length, records. Checks first.
    pub fn encode(&self) -> Result<Zeroizing<Vec<u8>>, StorageError> {
        self.check()?;
        let mut records = Zeroizing::new(Vec::new());
        let count = u16::try_from(self.records.len()).map_err(|_| StorageError::TooLarge)?;
        records.extend_from_slice(&count.to_be_bytes());
        for record in &self.records {
            records.extend_from_slice(&record.transfer);
            records.extend_from_slice(record.identity.as_bytes());
            records.extend_from_slice(record.contact.as_bytes());
            records.push(match record.direction {
                Direction::Outgoing => 0,
                Direction::Incoming => 1,
            });
            records.push(record.state.code());
            records.push(record.flags.bits());
            records.push(record.reason.map_or(0, Reason::code));
            records.extend_from_slice(&record.size.to_be_bytes());
            let name = record.name.as_bytes();
            records.push(u8::try_from(name.len()).map_err(|_| StorageError::TooLarge)?);
            records.extend_from_slice(name);
            records.extend_from_slice(&record.digest);
            records.extend_from_slice(
                record
                    .blob
                    .map_or([0; 16], |blob| *blob.as_bytes())
                    .as_slice(),
            );
            records.extend_from_slice(&record.created.to_be_bytes());
            records.extend_from_slice(&record.deadline.to_be_bytes());
        }
        let mut payload = Zeroizing::new(Vec::new());
        payload.extend_from_slice(&PAYLOAD_VERSION.to_be_bytes());
        payload.extend_from_slice(
            &u32::try_from(records.len())
                .map_err(|_| StorageError::TooLarge)?
                .to_be_bytes(),
        );
        payload.extend_from_slice(&records);
        Ok(payload)
    }

    /// Reads a padded payload. Fails with [`StorageError::BadFormat`] for
    /// anything that breaks the format or a rule, and with
    /// [`StorageError::UnsupportedVersion`] for a later payload version.
    pub fn decode(payload: &[u8]) -> Result<Self, StorageError> {
        let mut reader = Reader(payload);
        let version = u16::from_be_bytes(reader.array()?);
        if version > PAYLOAD_VERSION {
            return Err(StorageError::UnsupportedVersion);
        }
        if version != PAYLOAD_VERSION {
            return Err(StorageError::BadFormat);
        }
        let len = usize::try_from(u32::from_be_bytes(reader.array()?))
            .map_err(|_| StorageError::BadFormat)?;
        let records_bytes = reader.take(len)?;
        if reader.0.iter().any(|byte| *byte != 0) {
            return Err(StorageError::BadFormat);
        }
        let mut reader = Reader(records_bytes);
        let count = usize::from(u16::from_be_bytes(reader.array()?));
        if count > MAX_TRANSFER_RECORDS {
            return Err(StorageError::BadFormat);
        }
        let mut records = Vec::new();
        records
            .try_reserve_exact(count)
            .map_err(|_| StorageError::TooLarge)?;
        for _ in 0..count {
            records.push(read_record(&mut reader)?);
        }
        if !reader.0.is_empty() {
            return Err(StorageError::BadFormat);
        }
        let decoded = Self { records };
        decoded.check()?;
        Ok(decoded)
    }
}

fn read_record(reader: &mut Reader<'_>) -> Result<TransferRecord, StorageError> {
    let transfer = reader.array::<TRANSFER_ID_LEN>()?;
    let identity = IdentityId::from_bytes(reader.array::<RECORD_ID_LEN>()?)?;
    let contact = ContactId::from_bytes(reader.array::<RECORD_ID_LEN>()?)?;
    let direction = match reader.array::<1>()? {
        [0] => Direction::Outgoing,
        [1] => Direction::Incoming,
        _ => return Err(StorageError::BadFormat),
    };
    let [state] = reader.array::<1>()?;
    let state = State::from_code(state).ok_or(StorageError::BadFormat)?;
    let [flags] = reader.array::<1>()?;
    let flags = Flags::from_bits(flags).ok_or(StorageError::BadFormat)?;
    let reason = match reader.array::<1>()? {
        [0] => None,
        [code] => Some(Reason::from_code(code).ok_or(StorageError::BadFormat)?),
    };
    let size = u64::from_be_bytes(reader.array()?);
    let [name_len] = reader.array::<1>()?;
    let name = Filename::from_bytes(reader.take(usize::from(name_len))?)
        .map_err(|_| StorageError::BadFormat)?;
    let digest = reader.array::<DIGEST_LEN>()?;
    let blob = BlobId::from_bytes(reader.array()?);
    let created = u64::from_be_bytes(reader.array()?);
    let deadline = u64::from_be_bytes(reader.array()?);
    Ok(TransferRecord {
        transfer,
        identity,
        contact,
        direction,
        state,
        flags,
        reason,
        size,
        name,
        digest,
        blob,
        created,
        deadline,
    })
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], StorageError> {
        let (head, rest) = self
            .0
            .split_at_checked(len)
            .ok_or(StorageError::BadFormat)?;
        self.0 = rest;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], StorageError> {
        self.take(N)?
            .try_into()
            .map_err(|_| StorageError::BadFormat)
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
fn decrypt(key: &TransferStoreKey, file: &[u8]) -> Result<(u64, Zeroizing<Vec<u8>>), StorageError> {
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
fn seal_file(
    key: &TransferStoreKey,
    generation: u64,
    plaintext: &[u8],
) -> Result<Vec<u8>, StorageError> {
    let padded_len = plaintext
        .len()
        .div_ceil(TRANSFER_STORE_PADDING_BLOCK_LEN)
        .max(1)
        .checked_mul(TRANSFER_STORE_PADDING_BLOCK_LEN)
        .ok_or(StorageError::TooLarge)?;
    let payload_len = padded_len
        .checked_add(TAG_LEN)
        .ok_or(StorageError::TooLarge)?;
    let file_len = HEADER_LEN
        .checked_add(payload_len)
        .ok_or(StorageError::TooLarge)?;
    if u64::try_from(file_len).map_err(|_| StorageError::TooLarge)? > MAX_TRANSFER_STORE_FILE_LEN {
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

/// What opening a transfer store found besides the store itself.
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
    /// Neither file was there, or a `transfers.new` alone that did not
    /// open: an installation made before transfers, or one whose store is
    /// gone. An empty store was made.
    MadeEmpty,
}

/// An open transfer store: its directory, its key, its generation.
pub struct TransferFile<D: VaultDir> {
    dir: D,
    key: TransferStoreKey,
    generation: u64,
}

impl<D: VaultDir> core::fmt::Debug for TransferFile<D> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TransferFile")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl<D: VaultDir> TransferFile<D> {
    /// Opens the store in `dir`, or makes an empty one where there is none,
    /// and returns it with its records.
    ///
    /// The cases of `docs/STORAGE.md` section 3.5 with the names of the
    /// store, as for the message store, with one difference: where neither
    /// file is there, or only a `transfers.new` that does not open, an
    /// empty store is made (`docs/adr/0014`). A `transfers` that does not
    /// authenticate is never removed or overwritten.
    pub fn open(
        dir: D,
        key: TransferStoreKey,
    ) -> Result<(Self, TransferRecords, Recovery), StorageError> {
        let current = dir.read(TRANSFERS, MAX_TRANSFER_STORE_FILE_LEN)?;
        let new = dir.read(TRANSFERS_NEW, MAX_TRANSFER_STORE_FILE_LEN)?;
        let (generation, plaintext, recovery) = match (current, new) {
            (None, None) => return Self::make_empty(dir, key, false),
            (Some(current), None) => {
                let (generation, plaintext) = decrypt(&key, &current)?;
                (generation, plaintext, Recovery::Clean)
            }
            (Some(current), Some(new)) => match decrypt(&key, &current) {
                Ok((generation, plaintext)) => {
                    dir.remove(TRANSFERS_NEW)?;
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
            (None, Some(new)) => match decrypt(&key, &new) {
                Ok((generation, plaintext)) => {
                    dir.link(TRANSFERS_NEW, TRANSFERS)?;
                    dir.remove(TRANSFERS_NEW)?;
                    dir.sync()?;
                    (generation, plaintext, Recovery::CompletedCreation)
                }
                Err(_) => return Self::make_empty(dir, key, true),
            },
        };
        let records = TransferRecords::decode(&plaintext)?;
        Ok((
            Self {
                dir,
                key,
                generation,
            },
            records,
            recovery,
        ))
    }

    fn make_empty(
        dir: D,
        key: TransferStoreKey,
        remove_new: bool,
    ) -> Result<(Self, TransferRecords, Recovery), StorageError> {
        if remove_new {
            dir.remove(TRANSFERS_NEW)?;
            dir.sync()?;
        }
        let records = TransferRecords::default();
        let file = seal_file(&key, 1, &records.encode()?)?;
        dir.write_new(TRANSFERS_NEW, &file)?;
        dir.link(TRANSFERS_NEW, TRANSFERS)?;
        dir.remove(TRANSFERS_NEW)?;
        dir.sync()?;
        Ok((
            Self {
                dir,
                key,
                generation: 1,
            },
            records,
            Recovery::MadeEmpty,
        ))
    }

    /// Replaces the records: checked, sealed with the next generation,
    /// written as `transfers.new`, flushed, renamed to `transfers`, and the
    /// directory flushed. A `transfers.new` left by a write that failed is
    /// removed first.
    pub fn write(&mut self, records: &TransferRecords) -> Result<(), StorageError> {
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(StorageError::TooLarge)?;
        let file = seal_file(&self.key, generation, &records.encode()?)?;
        if self
            .dir
            .read(TRANSFERS_NEW, MAX_TRANSFER_STORE_FILE_LEN)?
            .is_some()
        {
            self.dir.remove(TRANSFERS_NEW)?;
        }
        self.dir.write_new(TRANSFERS_NEW, &file)?;
        self.dir.rename(TRANSFERS_NEW, TRANSFERS)?;
        self.dir.sync()?;
        self.generation = generation;
        Ok(())
    }

    /// The generation of the store on disk as of the last write.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// The directory of the store.
    pub fn dir(&self) -> &D {
        &self.dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dir::{CrashOutcome, MemoryDir};

    fn key() -> TransferStoreKey {
        TransferStoreKey::from_bytes_for_testing([5; 32])
    }

    fn record(direction: Direction, state: State, byte: u8) -> TransferRecord {
        let mut record = TransferRecord {
            transfer: [byte; 16],
            identity: IdentityId::from_bytes([1; 16]).unwrap(),
            contact: ContactId::from_bytes([2; 16]).unwrap(),
            direction,
            state,
            flags: Flags::default(),
            reason: None,
            size: 1000,
            name: Filename::new("report.pdf").unwrap(),
            digest: [0; 32],
            blob: None,
            created: 1_700_000_000,
            deadline: 0,
        };
        if record.has_blob() {
            record.blob = BlobId::from_bytes([byte.wrapping_add(100); 16]);
        }
        if direction == Direction::Outgoing || state == State::Complete {
            record.digest = [9; 32];
        }
        if matches!(state, State::Failed) {
            record.reason = Some(Reason::Storage);
        }
        if matches!(state, State::Cancelled) {
            record.reason = Some(Reason::ByUs);
        }
        if (direction == Direction::Incoming && state == State::Offered)
            || record.can_stall()
            || record.is_ended_incoming()
        {
            record.deadline = 1_700_100_000;
        }
        record
    }

    fn every_state() -> Vec<TransferRecord> {
        let mut records = Vec::new();
        let mut byte = 1;
        for (direction, states) in [
            (
                Direction::Outgoing,
                &[
                    State::Waiting,
                    State::Sending,
                    State::Delivered,
                    State::Rejected,
                    State::Cancelled,
                    State::Failed,
                ][..],
            ),
            (
                Direction::Incoming,
                &[
                    State::Offered,
                    State::Accepted,
                    State::Complete,
                    State::Expired,
                    State::Rejected,
                    State::Cancelled,
                    State::Failed,
                ][..],
            ),
        ] {
            for state in states {
                records.push(record(direction, *state, byte));
                byte += 1;
            }
        }
        records
    }

    #[test]
    fn records_of_every_state_read_back_exactly() {
        let records = TransferRecords {
            records: every_state(),
        };
        let payload = records.encode().unwrap();
        let mut padded = payload.to_vec();
        padded.resize(TRANSFER_STORE_PADDING_BLOCK_LEN, 0);
        assert_eq!(TransferRecords::decode(&padded).unwrap(), records);
    }

    #[test]
    fn every_rule_of_a_record_is_checked() {
        type Break = Box<dyn Fn(&mut TransferRecord)>;
        let breaks: Vec<(&str, Break)> = vec![
            ("zero transfer", Box::new(|r| r.transfer = [0; 16])),
            (
                "size above the format",
                Box::new(|r| r.size = MAX_FILE_SIZE + 1),
            ),
            (
                "state of the other direction",
                Box::new(|r| r.state = State::Offered),
            ),
            (
                "reason without failure",
                Box::new(|r| r.reason = Some(Reason::Storage)),
            ),
            (
                "offer read on incoming",
                Box::new(|r| {
                    r.direction = Direction::Incoming;
                    r.state = State::Rejected;
                    r.digest = [0; 32];
                    r.flags = Flags::from_bits(Flags::OFFER_READ).unwrap();
                }),
            ),
            (
                "unread on waiting",
                Box::new(|r| r.flags = Flags::from_bits(Flags::UNREAD).unwrap()),
            ),
            (
                "saved on outgoing",
                Box::new(|r| r.flags = Flags::from_bits(Flags::SAVED).unwrap()),
            ),
            (
                "removed on outgoing",
                Box::new(|r| r.flags = Flags::from_bits(Flags::REMOVED).unwrap()),
            ),
            ("no blob where one is needed", Box::new(|r| r.blob = None)),
            ("no digest on outgoing", Box::new(|r| r.digest = [0; 32])),
            ("no stall deadline", Box::new(|r| r.deadline = 0)),
        ];
        for (what, change) in breaks {
            let mut waiting = record(Direction::Outgoing, State::Waiting, 1);
            change(&mut waiting);
            assert_eq!(waiting.check(), Err(StorageError::BadFormat), "{what}");
        }
        // Incoming before complete carries no digest; complete and saved
        // carries no blob and a deadline; failed needs its reason.
        let mut accepted = record(Direction::Incoming, State::Accepted, 1);
        accepted.digest = [1; 32];
        assert_eq!(accepted.check(), Err(StorageError::BadFormat));
        let mut saved = record(Direction::Incoming, State::Complete, 1);
        saved.flags = Flags::from_bits(Flags::SAVED).unwrap();
        assert_eq!(saved.check(), Err(StorageError::BadFormat));
        saved.blob = None;
        assert_eq!(saved.check(), Err(StorageError::BadFormat));
        saved.deadline = 1;
        assert_eq!(saved.check(), Ok(()));
        saved.flags = Flags::from_bits(Flags::SAVED | Flags::SAVING).unwrap();
        assert_eq!(saved.check(), Err(StorageError::BadFormat));
        let mut failed = record(Direction::Incoming, State::Failed, 1);
        failed.reason = None;
        assert_eq!(failed.check(), Err(StorageError::BadFormat));
        failed.reason = Some(Reason::ByPeer);
        assert_eq!(failed.check(), Err(StorageError::BadFormat));
        let mut gone = record(Direction::Outgoing, State::Failed, 1);
        gone.reason = Some(Reason::OffererGone);
        assert_eq!(gone.check(), Err(StorageError::BadFormat));
        assert_eq!(Flags::from_bits(1 << 7), None);
    }

    #[test]
    fn two_records_with_one_key_or_one_blob_are_refused() {
        let first = record(Direction::Outgoing, State::Waiting, 1);
        let mut same_key = record(Direction::Outgoing, State::Waiting, 1);
        same_key.blob = BlobId::from_bytes([200; 16]);
        assert_eq!(
            TransferRecords {
                records: vec![first.clone(), same_key]
            }
            .check(),
            Err(StorageError::BadFormat)
        );
        let mut same_blob = record(Direction::Outgoing, State::Waiting, 2);
        same_blob.blob = first.blob;
        assert_eq!(
            TransferRecords {
                records: vec![first.clone(), same_blob]
            }
            .check(),
            Err(StorageError::BadFormat)
        );
        // The same transfer bytes in the other direction are another
        // transfer.
        let other_way = record(Direction::Incoming, State::Offered, 1);
        assert_eq!(
            TransferRecords {
                records: vec![first, other_way]
            }
            .check(),
            Ok(())
        );
    }

    #[test]
    fn a_payload_that_breaks_the_frame_is_refused() {
        let records = TransferRecords {
            records: vec![record(Direction::Outgoing, State::Waiting, 1)],
        };
        let payload = records.encode().unwrap().to_vec();
        let mut padded = payload.clone();
        padded.resize(TRANSFER_STORE_PADDING_BLOCK_LEN, 0);
        let mut nonzero = padded.clone();
        nonzero[payload.len()] = 1;
        assert_eq!(
            TransferRecords::decode(&nonzero),
            Err(StorageError::BadFormat)
        );
        let mut later = padded.clone();
        later[1] = 2;
        assert_eq!(
            TransferRecords::decode(&later),
            Err(StorageError::UnsupportedVersion)
        );
        let mut short = padded.clone();
        short[5] = short[5].wrapping_sub(1);
        assert_eq!(
            TransferRecords::decode(&short),
            Err(StorageError::BadFormat)
        );
        assert_eq!(
            TransferRecords::decode(&payload[..payload.len() - 1]),
            Err(StorageError::BadFormat)
        );
    }

    #[test]
    fn the_largest_store_fits_its_file() {
        let records = TransferRecords {
            records: (0..MAX_TRANSFER_RECORDS)
                .map(|i| {
                    let mut r = record(Direction::Outgoing, State::Delivered, 1);
                    r.transfer = [u8::try_from(i % 255 + 1).unwrap(); 16];
                    r.transfer[0] = u8::try_from(i / 255).unwrap();
                    r.transfer[1] = 1;
                    r.name = Filename::new(&"n".repeat(255)).unwrap();
                    r
                })
                .collect(),
        };
        let dir = MemoryDir::new();
        let (mut file, empty, recovery) = TransferFile::open(dir.clone(), key()).unwrap();
        assert!(empty.records.is_empty());
        assert_eq!(recovery, Recovery::MadeEmpty);
        file.write(&records).unwrap();
        let (_, read, recovery) = TransferFile::open(dir, key()).unwrap();
        assert_eq!(read, records);
        assert_eq!(recovery, Recovery::Clean);
        let too_many = TransferRecords {
            records: vec![
                record(Direction::Outgoing, State::Delivered, 1);
                MAX_TRANSFER_RECORDS + 1
            ],
        };
        assert_eq!(too_many.check(), Err(StorageError::TooLarge));
    }

    #[test]
    fn a_crash_during_a_write_leaves_the_previous_or_the_new_records() {
        let one = TransferRecords {
            records: vec![record(Direction::Outgoing, State::Waiting, 1)],
        };
        let two = TransferRecords {
            records: vec![
                record(Direction::Outgoing, State::Waiting, 1),
                record(Direction::Incoming, State::Offered, 2),
            ],
        };
        let base = MemoryDir::new();
        let (mut file, _, _) = TransferFile::open(base.clone(), key()).unwrap();
        file.write(&one).unwrap();
        let start = base.steps();
        let mut step = start + 1;
        loop {
            let dir = base.restart(CrashOutcome::ALL[3]);
            let (mut file, _, _) = TransferFile::open(dir.clone(), key()).unwrap();
            let begun = dir.steps();
            dir.crash_at(begun + (step - start));
            let done = file.write(&two).is_ok();
            for outcome in CrashOutcome::ALL {
                let after = dir.restart(outcome);
                let (_, read, _) = TransferFile::open(after, key()).unwrap();
                assert!(read == one || read == two, "step {step} {outcome:?}");
                if done {
                    assert_eq!(read, two);
                }
            }
            if done {
                break;
            }
            step += 1;
        }
    }

    #[test]
    fn a_new_store_alone_is_put_in_place_or_replaced_by_an_empty_one() {
        let dir = MemoryDir::new();
        let (mut file, _, _) = TransferFile::open(dir.clone(), key()).unwrap();
        let records = TransferRecords {
            records: vec![record(Direction::Outgoing, State::Waiting, 1)],
        };
        file.write(&records).unwrap();
        // A creation interrupted after the file was complete.
        let bytes = dir
            .read(TRANSFERS, MAX_TRANSFER_STORE_FILE_LEN)
            .unwrap()
            .unwrap();
        dir.remove(TRANSFERS).unwrap();
        dir.write_new(TRANSFERS_NEW, &bytes).unwrap();
        let (_, read, recovery) = TransferFile::open(dir.clone(), key()).unwrap();
        assert_eq!((read, recovery), (records, Recovery::CompletedCreation));
        // A new store alone that does not open.
        dir.remove(TRANSFERS).unwrap();
        dir.write_new(TRANSFERS_NEW, b"torn").unwrap();
        let (_, read, recovery) = TransferFile::open(dir.clone(), key()).unwrap();
        assert!(read.records.is_empty());
        assert_eq!(recovery, Recovery::MadeEmpty);
        assert_eq!(dir.read(TRANSFERS_NEW, 100).unwrap(), None);
    }

    #[test]
    fn a_store_under_another_key_or_changed_does_not_open() {
        let dir = MemoryDir::new();
        let (mut file, _, _) = TransferFile::open(dir.clone(), key()).unwrap();
        file.write(&TransferRecords::default()).unwrap();
        assert_eq!(
            TransferFile::open(
                dir.clone(),
                TransferStoreKey::from_bytes_for_testing([6; 32])
            )
            .map(|_| ()),
            Err(StorageError::AuthenticationFailed)
        );
        let mut bytes = dir
            .read(TRANSFERS, MAX_TRANSFER_STORE_FILE_LEN)
            .unwrap()
            .unwrap();
        bytes[12] ^= 1;
        dir.overwrite(TRANSFERS, &bytes).unwrap();
        assert_eq!(
            TransferFile::open(dir.clone(), key()).map(|_| ()),
            Err(StorageError::AuthenticationFailed)
        );
        // Never removed or overwritten.
        assert_eq!(
            dir.read(TRANSFERS, MAX_TRANSFER_STORE_FILE_LEN).unwrap(),
            Some(bytes)
        );
    }
}
