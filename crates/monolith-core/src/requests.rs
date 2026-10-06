//! Invitation capabilities and contact requests of one local identity
//! (`docs/PROTOCOL.md` sections 12 to 12.3).
//!
//! The active set holds at most `MAX_ACTIVE_INVITATIONS` capabilities. A
//! capability enters it when the user creates one and leaves it only when
//! the user revokes it; when the set is full, creating another fails and
//! nothing is revoked or evicted. Version 1 has no expiry and no use
//! counter: a capability is valid for any number of requests until it is
//! revoked.
//!
//! A contact request from a peer that is not a contact is decided after
//! the Close was written: it is queued or dropped, and the peer cannot tell
//! which. A queued request holds the sender's card, display name and
//! introduction, and which member of the active set admitted it, as a
//! reference that is never sent. The queue lives in memory: nothing a
//! stranger sends is written to the vault (S7). Accepting a request makes
//! a contact, which is durable; declining or blocking records that.

use core::fmt;

use monolith_identity::IdentityPublicKey;
use monolith_protocol::body::ContactRequest;
use monolith_protocol::card::{ContactCard, InvitationCapability};
use monolith_protocol::contact::{RecordKind, RequestMode};
use monolith_protocol::limits::{
    MAX_ACTIVE_INVITATIONS, MAX_PENDING_CONTACT_REQUESTS, MAX_PENDING_REQUESTS_PER_INVITATION,
};
use monolith_protocol::text::{DisplayName, IntroductionText};
use monolith_storage::record::StoredInvitation;

use crate::contacts::StoreError;

/// A member of the active set, as the local side refers to it. The number
/// is local and never sent; it is not kept across restarts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InvitationId(u64);

struct Invitation {
    id: InvitationId,
    capability: InvitationCapability,
    label: Option<DisplayName>,
    /// The generation that makes the member durable; `u64::MAX` until it
    /// is stamped, 0 for a member read from the vault. A member is listed,
    /// put into a card and admits requests only once it is durable: a
    /// crash before would lose it, and every card handed out for it.
    durable_at: u64,
}

/// The active set of invitation capabilities of one local identity.
pub(crate) struct Invitations {
    members: Vec<Invitation>,
    next: u64,
}

impl fmt::Debug for Invitations {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Invitations")
            .field("members", &self.members.len())
            .finish()
    }
}

impl Invitations {
    pub(crate) fn new() -> Self {
        Self {
            members: Vec::new(),
            next: 0,
        }
    }

    pub(crate) fn load(stored: Vec<StoredInvitation>) -> Result<Self, StoreError> {
        let mut invitations = Self::new();
        for invitation in stored {
            let id = invitations.add(invitation.capability, invitation.label)?;
            // Read from the vault: durable.
            invitations.stamp(id, 0);
        }
        Ok(invitations)
    }

    /// Adds `capability` to the active set, unstamped: it is handed out
    /// once [`Self::stamp`] gave it the generation that makes it durable and
    /// that generation is. Fails with [`StoreError::Full`] when the set
    /// holds `MAX_ACTIVE_INVITATIONS` members, and changes nothing then.
    pub(crate) fn add(
        &mut self,
        capability: InvitationCapability,
        label: Option<DisplayName>,
    ) -> Result<InvitationId, StoreError> {
        if self.members.len() >= MAX_ACTIVE_INVITATIONS {
            return Err(StoreError::Full);
        }
        if self.find(&capability, u64::MAX).is_some() {
            return Err(StoreError::Protocol(
                monolith_protocol::ProtocolError::InvalidValue,
            ));
        }
        let id = InvitationId(self.next);
        self.next = self.next.saturating_add(1);
        self.members.push(Invitation {
            id,
            capability,
            label,
            durable_at: u64::MAX,
        });
        Ok(id)
    }

    /// Records the generation that makes the member `id` durable.
    pub(crate) fn stamp(&mut self, id: InvitationId, generation: u64) {
        if let Some(member) = self.members.iter_mut().find(|member| member.id == id) {
            member.durable_at = generation;
        }
    }

    /// Removes a member. A revoked capability is removed, not remembered.
    pub(crate) fn revoke(&mut self, id: InvitationId) -> Result<(), StoreError> {
        let before = self.members.len();
        self.members.retain(|member| member.id != id);
        if self.members.len() == before {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    /// The member that `capability` is, among the members durable by
    /// `durable`, compared with every member without an early exit. Best
    /// effort against timing; the protocol promises nothing about response
    /// times (`docs/PROTOCOL.md` section 12.1).
    pub(crate) fn find(
        &self,
        capability: &InvitationCapability,
        durable: u64,
    ) -> Option<InvitationId> {
        let mut found = None;
        for member in &self.members {
            // `==` on a capability is a constant-time comparison.
            let equal = member.capability == *capability && member.durable_at <= durable;
            found = if equal && found.is_none() {
                Some(member.id)
            } else {
                found
            };
        }
        found
    }

    /// The capability of a member durable by `durable`, to put into a
    /// card.
    pub(crate) fn capability(
        &self,
        id: InvitationId,
        durable: u64,
    ) -> Option<&InvitationCapability> {
        self.members
            .iter()
            .find(|member| member.id == id && member.durable_at <= durable)
            .map(|member| &member.capability)
    }

    /// The members durable by `durable`, with their labels.
    pub(crate) fn list(&self, durable: u64) -> Vec<(InvitationId, Option<DisplayName>)> {
        self.members
            .iter()
            .filter(|member| member.durable_at <= durable)
            .map(|member| (member.id, member.label.clone()))
            .collect()
    }

    pub(crate) fn snapshot(&self) -> Vec<StoredInvitation> {
        self.members
            .iter()
            .map(|member| StoredInvitation {
                capability: member.capability.clone(),
                label: member.label.clone(),
            })
            .collect()
    }
}

/// A contact request waiting for the user.
#[derive(Clone, PartialEq, Eq)]
pub struct PendingRequest {
    /// The card the sender presented, which is its own.
    pub card: ContactCard,
    /// Its display name.
    pub display_name: DisplayName,
    /// Its introduction.
    pub introduction: IntroductionText,
    /// The member of the active set that admitted it, or `None` for a
    /// request without a capability in open mode. Local; never sent.
    pub admitted_by: Option<InvitationId>,
}

impl fmt::Debug for PendingRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingRequest")
            .field("admitted_by", &self.admitted_by)
            .finish_non_exhaustive()
    }
}

impl PendingRequest {
    /// The identity of the sender.
    pub fn identity(&self) -> &IdentityPublicKey {
        self.card.identity()
    }
}

/// Why a request was dropped. Local only: counted and shown to the user,
/// never sent, never logged with peer data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dropped {
    /// The sender is blocked.
    Blocked,
    /// The sender was declined.
    Declined,
    /// The sender has a record as a contact; its session was not a
    /// contact's (a stale card or a pending key).
    Contact,
    /// The request mode does not admit it.
    Mode,
    /// Its capability is not in the active set.
    Capability,
    /// A request from this identity is already waiting.
    AlreadyPending,
    /// `MAX_PENDING_REQUESTS_PER_INVITATION` requests with the same
    /// capability, or without one, are waiting.
    Quota,
    /// The queue is full.
    QueueFull,
}

/// The pending requests of one local identity.
#[derive(Debug, Default)]
pub(crate) struct Requests {
    queue: Vec<PendingRequest>,
}

impl Requests {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Decides whether `request` from a peer whose record is `kind` goes
    /// into the queue (`docs/PROTOCOL.md` section 12, step 3). Called after
    /// the Close went out.
    pub(crate) fn consider(
        &mut self,
        request: &ContactRequest,
        kind: RecordKind,
        mode: RequestMode,
        invitations: &Invitations,
        durable: u64,
    ) -> Result<(), Dropped> {
        match kind {
            RecordKind::Blocked => return Err(Dropped::Blocked),
            RecordKind::Declined => return Err(Dropped::Declined),
            RecordKind::Requested | RecordKind::Accepted => return Err(Dropped::Contact),
            RecordKind::None => {}
        }
        // Every member is compared whatever the mode, so that the work
        // does not depend on it.
        let admitted_by = request
            .invitation
            .as_ref()
            .map(|capability| invitations.find(capability, durable));
        let admitted_by = match (mode, admitted_by) {
            (RequestMode::Closed, _) => return Err(Dropped::Mode),
            (_, Some(None)) => return Err(Dropped::Capability),
            (RequestMode::Invitation, None) => return Err(Dropped::Mode),
            (_, Some(Some(id))) => Some(id),
            (RequestMode::Open, None) => None,
        };
        let identity = request.card.identity();
        if self.queue.iter().any(|held| held.identity() == identity) {
            return Err(Dropped::AlreadyPending);
        }
        let same = self
            .queue
            .iter()
            .filter(|held| held.admitted_by == admitted_by)
            .count();
        if same >= MAX_PENDING_REQUESTS_PER_INVITATION {
            return Err(Dropped::Quota);
        }
        if self.queue.len() >= MAX_PENDING_CONTACT_REQUESTS {
            return Err(Dropped::QueueFull);
        }
        self.queue.push(PendingRequest {
            card: request.card.clone(),
            display_name: request.display_name.clone(),
            introduction: request.introduction.clone(),
            admitted_by,
        });
        Ok(())
    }

    /// Takes the request of `identity` out of the queue.
    pub(crate) fn take(&mut self, identity: &IdentityPublicKey) -> Option<PendingRequest> {
        let position = self
            .queue
            .iter()
            .position(|held| held.identity() == identity)?;
        Some(self.queue.remove(position))
    }

    /// Discards the requests that `id` admitted: the separate action of
    /// `docs/PROTOCOL.md` section 12.3. Returns how many.
    pub(crate) fn discard_admitted_by(&mut self, id: InvitationId) -> usize {
        let before = self.queue.len();
        self.queue.retain(|held| held.admitted_by != Some(id));
        before.saturating_sub(self.queue.len())
    }

    /// Discards a request from `identity`, if one waits: the identity
    /// became a contact or was blocked another way.
    pub(crate) fn forget(&mut self, identity: &IdentityPublicKey) {
        self.queue.retain(|held| held.identity() != identity);
    }

    pub(crate) fn list(&self) -> Vec<PendingRequest> {
        self.queue.clone()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_capability_is_handed_out_and_admits_only_once_durable() {
        let mut invitations = Invitations::new();
        let capability = InvitationCapability::from_bytes([7; 16]);
        let id = invitations.add(capability.clone(), None).unwrap();
        // Unstamped: nothing, whatever is durable.
        assert_eq!(invitations.find(&capability, u64::MAX - 1), None);
        invitations.stamp(id, 5);
        for durable in [0, 4] {
            assert_eq!(invitations.find(&capability, durable), None);
            assert!(invitations.capability(id, durable).is_none());
            assert!(invitations.list(durable).is_empty());
        }
        assert_eq!(invitations.find(&capability, 5), Some(id));
        assert_eq!(invitations.capability(id, 5), Some(&capability));
        assert_eq!(invitations.list(5), vec![(id, None)]);
        // A capability is in the set once, durable or not.
        assert_eq!(
            invitations.add(capability, None).err(),
            Some(StoreError::Protocol(
                monolith_protocol::ProtocolError::InvalidValue
            ))
        );
    }
}
