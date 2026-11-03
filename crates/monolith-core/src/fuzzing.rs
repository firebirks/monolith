//! Entry points for the fuzz targets in `fuzz/`.
//!
//! Compiled only into the tests of this crate and into builds made with
//! `--cfg fuzzing`. They expose the transfer state machine
//! (`transfers.rs`), which is not part of the interface of the crate, with
//! transfers named by small numbers, so that a fuzz target can drive it
//! with any sequence of operations and check it against rules written
//! independently.

use monolith_protocol::text::Filename;
use monolith_storage::blob::BlobId;
use monolith_storage::record::{ContactId, IdentityId};
use monolith_storage::transfers::{Direction, Reason, TransferRecords};

use crate::api::{TransferStatus, TransferView};
use crate::transfers::{Answer, Key, Limits, Offer, Staged, TransferState};

/// A transfer by small numbers: the local identity (0 or 1), the contact
/// record of that identity (0 or 1), the direction, and the identifier
/// (0 to 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Named {
    /// The local identity, 0 or 1.
    pub identity: u8,
    /// The contact record of the identity, 0 or 1.
    pub contact: u8,
    /// Whether the peer offered it.
    pub incoming: bool,
    /// The identifier, 0 to 7.
    pub transfer: u8,
}

impl Named {
    /// The transfer a byte names: bit 0 the identity, bit 1 the contact,
    /// bit 2 the direction, bits 3 to 5 the identifier.
    pub const fn from_byte(byte: u8) -> Self {
        Self {
            identity: byte & 1,
            contact: (byte >> 1) & 1,
            incoming: (byte >> 2) & 1 == 1,
            transfer: (byte >> 3) & 7,
        }
    }

    fn identity_id(identity: u8) -> Option<IdentityId> {
        IdentityId::from_bytes([identity.wrapping_add(1); 16]).ok()
    }

    fn contact_id(identity: u8, contact: u8) -> Option<ContactId> {
        ContactId::from_bytes(
            [0x10_u8
                .wrapping_add(identity.wrapping_mul(2))
                .wrapping_add(contact); 16],
        )
        .ok()
    }

    fn key(self) -> Option<Key> {
        Some(Key {
            identity: Self::identity_id(self.identity)?,
            contact: Self::contact_id(self.identity, self.contact)?,
            direction: if self.incoming {
                Direction::Incoming
            } else {
                Direction::Outgoing
            },
            transfer: [self.transfer.wrapping_add(1); 16],
        })
    }

    /// The transfer `view` names, if it is one these numbers name.
    pub fn of(view: &TransferView) -> Option<Self> {
        let reference = view.reference;
        let identity = reference.identity.as_bytes().first()?.checked_sub(1)?;
        let contact = reference
            .contact
            .as_bytes()
            .first()?
            .checked_sub(0x10)?
            .checked_sub(identity.checked_mul(2)?)?;
        let transfer = reference.transfer().first()?.checked_sub(1)?;
        let named = Self {
            identity,
            contact,
            incoming: reference.incoming,
            transfer,
        };
        (named.key()?.reference() == reference).then_some(named)
    }

    /// The digest of the file of this transfer.
    pub const fn digest(self) -> [u8; 32] {
        [self.transfer.wrapping_add(0x40); 32]
    }

    fn blob(self) -> Option<BlobId> {
        let mut bytes = [0_u8; 16];
        bytes[..4].copy_from_slice(&[
            self.identity,
            self.contact,
            u8::from(self.incoming),
            self.transfer.wrapping_add(1),
        ]);
        BlobId::from_bytes(bytes)
    }
}

/// What an offer of the peer got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offered {
    /// Recorded, for the user to decide.
    New,
    /// Answered with nothing.
    Nothing,
    /// Answered with FileAccept and the bytes held.
    Accept(u64),
    /// Answered with FileComplete.
    Complete,
    /// Answered with FileReject.
    Reject,
    /// Answered with FileAbort.
    Abort,
    /// Its blob is to be scanned first.
    Scan,
    /// It differs from its record.
    Violation,
}

/// The transfer state of one installation.
#[derive(Debug)]
pub struct Transfers {
    state: TransferState,
}

impl Default for Transfers {
    fn default() -> Self {
        Self::new()
    }
}

impl Transfers {
    /// An empty state with the limits of an installation with a vault.
    pub const fn new() -> Self {
        Self {
            state: TransferState::new(Limits::persistent()),
        }
    }

    /// The user offers a staged file of `size` bytes as `named`, outgoing.
    /// Returns whether it was recorded.
    pub fn offer(&mut self, named: Named, size: u64, now: u64) -> bool {
        let (Some(key), Some(blob), Ok(name)) = (named.key(), named.blob(), Filename::new("f.bin"))
        else {
            return false;
        };
        if named.incoming {
            return false;
        }
        let staged = Staged {
            identity: key.identity,
            contact: key.contact,
            transfer: key.transfer,
            size,
            name,
            digest: named.digest(),
            blob,
        };
        self.state.add_outgoing(staged, now).is_ok()
    }

    /// The peer offers `named`, incoming, of `size` bytes.
    pub fn offered(&mut self, named: Named, size: u64, now: u64) -> Offered {
        let (Some(key), Ok(name)) = (named.key(), Filename::new("f.bin")) else {
            return Offered::Violation;
        };
        if !named.incoming {
            return Offered::Violation;
        }
        match self.state.offered(&key, size, &name, now) {
            Offer::New => Offered::New,
            Offer::Scan(_) => Offered::Scan,
            Offer::Violation => Offered::Violation,
            Offer::Answer(Answer::Nothing) => Offered::Nothing,
            Offer::Answer(Answer::Accept(held)) => Offered::Accept(held),
            Offer::Answer(Answer::Complete(_)) => Offered::Complete,
            Offer::Answer(Answer::Reject) => Offered::Reject,
            Offer::Answer(Answer::Abort) => Offered::Abort,
        }
    }

    /// The user accepts `named`. Returns whether it was taken.
    pub fn accept(&mut self, named: Named, now: u64) -> bool {
        match (named.key(), named.blob()) {
            (Some(key), Some(blob)) => self.state.accept(&key, blob, now).is_ok(),
            _ => false,
        }
    }

    /// The user rejects `named`. Returns whether it was taken.
    pub fn reject(&mut self, named: Named, now: u64) -> bool {
        named
            .key()
            .is_some_and(|key| self.state.reject(&key, now).is_ok())
    }

    /// The user cancels `named`. Returns whether it was taken.
    pub fn cancel(&mut self, named: Named, now: u64) -> bool {
        named
            .key()
            .is_some_and(|key| self.state.cancel(&key, now).is_ok())
    }

    /// The user removes `named` from the list. Returns whether it was
    /// taken.
    pub fn remove(&mut self, named: Named, now: u64) -> bool {
        named
            .key()
            .is_some_and(|key| self.state.remove(&key, now).is_ok())
    }

    /// The peer accepted `named` holding `offset` bytes.
    pub fn accepted_by_peer(&mut self, named: Named, offset: u64, now: u64) {
        if let Some(key) = named.key() {
            self.state.accepted_by_peer(&key, offset, now);
        }
    }

    /// The peer completed `named`, with the digest of its file or another.
    /// Returns whether the completion matched.
    pub fn completed_by_peer(&mut self, named: Named, right: bool, now: u64) -> bool {
        let digest = if right { named.digest() } else { [0xEE; 32] };
        named
            .key()
            .is_some_and(|key| self.state.completed_by_peer(&key, &digest, now).1)
    }

    /// The peer rejected `named`.
    pub fn rejected_by_peer(&mut self, named: Named, now: u64) {
        if let Some(key) = named.key() {
            self.state.rejected_by_peer(&key, now);
        }
    }

    /// The peer cancelled `named`.
    pub fn aborted_by_peer(&mut self, named: Named, now: u64) {
        if let Some(key) = named.key() {
            self.state.aborted_by_peer(&key, now);
        }
    }

    /// `bytes` of `named` are staged durably.
    pub fn staged(&mut self, named: Named, bytes: u64, now: u64) {
        if let Some(key) = named.key() {
            self.state.staged(&key, bytes, now);
        }
    }

    /// The whole of `named` is staged and verified, with the digest of its
    /// file.
    pub fn complete(&mut self, named: Named, now: u64) {
        if let Some(key) = named.key() {
            self.state.complete(&key, named.digest(), now);
        }
    }

    /// `named` fails: its staged data cannot be written or read.
    pub fn fail(&mut self, named: Named, now: u64) {
        if let Some(key) = named.key() {
            self.state.fail(&key, Reason::Storage, now);
        }
    }

    /// What the time does at `now`.
    pub fn tick(&mut self, now: u64) {
        self.state.tick(now);
    }

    /// The contact record `contact` of `identity` is blocked, or deleted.
    pub fn contact_ended(&mut self, identity: u8, contact: u8, delete: bool, now: u64) {
        if let (Some(id), Some(record)) = (
            Named::identity_id(identity),
            Named::contact_id(identity, contact),
        ) {
            self.state.contact_ended(&id, &record, delete, now);
        }
    }

    /// The local identity `identity` goes.
    pub fn remove_identity(&mut self, identity: u8) {
        if let Some(id) = Named::identity_id(identity) {
            self.state.remove_identity(&id);
        }
    }

    /// The installation opens again from the encoded store. Returns
    /// whether the store encoded and decoded.
    pub fn restart(&mut self) -> bool {
        let Ok(encoded) = self.state.records().encode() else {
            return false;
        };
        let Ok(records) = TransferRecords::decode(&encoded) else {
            return false;
        };
        self.state = TransferState::load(records, Limits::persistent());
        true
    }

    /// The transfers of `identity` the interface shows.
    pub fn views(&self, identity: u8, now: u64) -> Vec<TransferView> {
        Named::identity_id(identity)
            .map(|id| self.state.views(&id, now))
            .unwrap_or_default()
    }

    /// Where `named` stands, unless it is not listed.
    pub fn status(&self, named: Named, now: u64) -> Option<TransferStatus> {
        named
            .key()
            .and_then(|key| self.state.view(&key, now))
            .map(|view| view.status)
    }

    /// The records held, removed ones included.
    pub fn records(&self) -> usize {
        self.state.records().records.len()
    }

    /// The blobs the records name.
    pub fn blobs(&self) -> usize {
        self.state.blobs().len()
    }
}
