//! The transport credential a contact is trusted with, and how it changes.
//!
//! A contact card says "this identity issued this credential": the
//! identity key signed the transport key, the endpoints and the epoch. That
//! is necessary for a transport key to stand for a contact, and it is not
//! enough. Which transport key stands for a contact today is local state,
//! and [`Credentials`] holds it. A valid signature alone never replaces it;
//! a newer key takes over only through continuity from the current one or
//! through an explicit confirmation by the user (rule F4,
//! `docs/PROTOCOL.md` section 11.4).
//!
//! The states of a transport key, for one contact:
//!
//! - Active. The key that authenticates the contact. There is exactly one.
//! - Authorized successor. A newer key that the identity announced in an
//!   EndpointUpdate on a session made with the active key. The active key
//!   stays active. The successor takes over when its holder proves it in a
//!   handshake.
//! - Pending successor. A newer key that arrived any other way: presented
//!   in a handshake without such an announcement, or imported by hand for
//!   an accepted contact. It gives no standing. It takes over only when the
//!   user confirms it.
//! - Retired. The key that was active before the last change. A card that
//!   states it never opens a contact session again, whatever its epoch.
//!
//! When a successor takes over, the previous key is retired at once, and
//! every session that was authenticated with it loses its standing:
//! [`Credentials::authorizes`] answers false for it from then on.
//!
//! Epochs still prevent rollback: a card older than the active card, or
//! older than the authorized successor for the same key, changes nothing.
//! What an epoch no longer does is replace the active key by itself.
//!
//! An invitation capability in a card plays no part in any of this. The
//! capability that a request to the contact carries is kept next to the
//! credentials and changes only with a card the user imports.

use monolith_identity::{IdentityPublicKey, TransportPublicKey};

use crate::ProtocolError;
use crate::card::{CardChange, ContactCard, InvitationCapability, evaluate_card};

/// What an event did to the credentials of a contact.
///
/// [`Credentials::admit`] returns `Unchanged`, `Advanced`, `Promoted`,
/// `Pending`, `Conflict` or `Stale`. [`Credentials::announce`] returns
/// `Unchanged`, `Advanced`, `Authorized`, `Conflict`, `Stale` or
/// `NoContinuity`. The import and confirmation functions say what they
/// return.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CredentialChange {
    /// The card states what is already held. Nothing changes.
    Unchanged,
    /// A newer card with the active transport key. It is now the active
    /// card. The transport credential is the same; the endpoints may have
    /// changed, and where Monolith dials is still the user's decision.
    Advanced,
    /// A newer card with another transport key, announced on a session
    /// made with the active key. The key is the authorized successor; the
    /// active key stays active.
    Authorized,
    /// A newer card with another transport key that did not come through
    /// the active key. It is held as the pending successor and gives no
    /// standing until the user confirms it.
    Pending,
    /// A successor took over: its holder proved it in a handshake, or the
    /// user confirmed it, or imported it for a contact that had not
    /// accepted yet. The previous active key is retired, and sessions
    /// authenticated with it no longer stand for the contact.
    Promoted,
    /// The card has the epoch of the active card, or of the authorized
    /// successor for its key, and states something else. The identity
    /// signed two statements for one epoch. Nothing changes; the user is
    /// told.
    Conflict,
    /// The card is older than the active card, older than the authorized
    /// successor for its key, or states the retired key. Nothing changes.
    Stale,
    /// An EndpointUpdate arrived on a session that was not authenticated
    /// with the active key. It proves no continuity. Nothing changes.
    NoContinuity,
}

/// The transport credential of one contact.
///
/// Invariants, kept by every function:
///
/// - Every card held is of the identity of the active card.
/// - A successor has a greater epoch than the active card and another
///   transport key than the active one and the retired one.
/// - The authorized and the pending successor state different keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credentials {
    /// The newest card accepted for the active transport key.
    active: ContactCard,
    authorized: Option<ContactCard>,
    pending: Option<ContactCard>,
    retired: Option<TransportPublicKey>,
    /// The capability a contact request to this identity carries: the one
    /// in the card the user was given, if any.
    invitation: Option<InvitationCapability>,
}

/// True if two cards make the same statement: transport key, epoch and
/// endpoint set. The capability and with it the signature may differ.
fn same_statement(a: &ContactCard, b: &ContactCard) -> bool {
    a.epoch() == b.epoch() && a.transport() == b.transport() && a.endpoints() == b.endpoints()
}

impl Credentials {
    /// The credentials of an identity whose card the user imported or
    /// accepted. The transport key of the card is active, and the
    /// capability in the card, if any, is what a request carries.
    pub fn new(card: ContactCard) -> Self {
        let invitation = card.invitation().cloned();
        Self {
            active: card,
            authorized: None,
            pending: None,
            retired: None,
            invitation,
        }
    }

    /// Returns the identity.
    pub const fn identity(&self) -> &IdentityPublicKey {
        self.active.identity()
    }

    /// Returns the active card: the newest card accepted for the active
    /// transport key.
    pub const fn active(&self) -> &ContactCard {
        &self.active
    }

    /// Returns the authorized successor, if there is one.
    pub const fn authorized_successor(&self) -> Option<&ContactCard> {
        self.authorized.as_ref()
    }

    /// Returns the pending successor, if there is one.
    pub const fn pending_successor(&self) -> Option<&ContactCard> {
        self.pending.as_ref()
    }

    /// Returns the transport key that was active before the last change,
    /// if there was a change.
    pub const fn retired(&self) -> Option<&TransportPublicKey> {
        self.retired.as_ref()
    }

    /// Returns the capability a contact request to this identity carries.
    pub const fn invitation(&self) -> Option<&InvitationCapability> {
        self.invitation.as_ref()
    }

    /// Returns true if a session whose peer is `card` stands for this
    /// contact: the card is of this identity and states the active
    /// transport key. `card` is the card of the session
    /// (`AuthenticatedSession::peer_card` in the session crate), whose
    /// transport key the handshake proved. A session for which this turns
    /// false has to end before anything else is done with it, before the
    /// duplicate rule in particular.
    pub fn authorizes(&self, card: &ContactCard) -> bool {
        card.identity() == self.identity() && card.transport() == self.active.transport()
    }

    fn check_identity(&self, card: &ContactCard) -> Result<(), ProtocolError> {
        if card.identity() == self.identity() {
            Ok(())
        } else {
            Err(ProtocolError::IdentityMismatch)
        }
    }

    fn is_retired(&self, card: &ContactCard) -> bool {
        self.retired.as_ref() == Some(card.transport())
    }

    /// A handshake proved that the peer holds the transport key of `card`:
    /// the card the peer presented in the third message, or the card that
    /// was dialed. Decides what that means for the contact and records it.
    ///
    /// - The active key with the active card: `Unchanged`.
    /// - The active key with a newer card: `Advanced`.
    /// - The key of the authorized successor, with a card not older than
    ///   the announced one: `Promoted`. The previous key is retired.
    /// - Another key with a newer card: `Pending`. The active key is not
    ///   touched, so whoever holds the identity key alone cannot take the
    ///   contact over or lock the holder of the active key out.
    /// - An older card, a contradicting one, or the retired key: `Stale` or
    ///   `Conflict`.
    ///
    /// Fails with [`ProtocolError::IdentityMismatch`] for a card of
    /// another identity.
    pub fn admit(&mut self, card: &ContactCard) -> Result<CredentialChange, ProtocolError> {
        self.check_identity(card)?;
        if self.is_retired(card) {
            return Ok(CredentialChange::Stale);
        }
        match evaluate_card(&self.active, card)? {
            CardChange::Unchanged => return Ok(CredentialChange::Unchanged),
            CardChange::Conflict => return Ok(CredentialChange::Conflict),
            CardChange::Stale => return Ok(CredentialChange::Stale),
            CardChange::Newer => {}
        }
        if card.transport() == self.active.transport() {
            self.advance(card.clone());
            return Ok(CredentialChange::Advanced);
        }
        let proven = match &self.authorized {
            Some(successor) if card.transport() == successor.transport() => {
                Some(evaluate_card(successor, card)?)
            }
            _ => None,
        };
        if let Some(proven) = proven {
            return Ok(match proven {
                CardChange::Stale => CredentialChange::Stale,
                CardChange::Conflict => CredentialChange::Conflict,
                CardChange::Unchanged | CardChange::Newer => {
                    self.promote(card.clone());
                    CredentialChange::Promoted
                }
            });
        }
        self.hold_pending(card.clone());
        Ok(CredentialChange::Pending)
    }

    /// An EndpointUpdate with `card` arrived on a session whose peer is
    /// `session`: the card of that session, whose transport key the
    /// handshake proved.
    ///
    /// Only a session made with the active key carries continuity. On any
    /// other the result is `NoContinuity` and nothing changes. On such a
    /// session a newer card with the active key is `Advanced`, and a newer
    /// card with another key becomes the authorized successor
    /// (`Authorized`), replacing an older one. The active key stays active
    /// until the successor is proven.
    ///
    /// Fails with [`ProtocolError::IdentityMismatch`] if `card` or
    /// `session` is of another identity.
    pub fn announce(
        &mut self,
        card: &ContactCard,
        session: &ContactCard,
    ) -> Result<CredentialChange, ProtocolError> {
        self.check_identity(card)?;
        self.check_identity(session)?;
        if session.transport() != self.active.transport() {
            return Ok(CredentialChange::NoContinuity);
        }
        if self.is_retired(card) {
            return Ok(CredentialChange::Stale);
        }
        match evaluate_card(&self.active, card)? {
            CardChange::Unchanged => return Ok(CredentialChange::Unchanged),
            CardChange::Conflict => return Ok(CredentialChange::Conflict),
            CardChange::Stale => return Ok(CredentialChange::Stale),
            CardChange::Newer => {}
        }
        if card.transport() == self.active.transport() {
            self.advance(card.clone());
            return Ok(CredentialChange::Advanced);
        }
        if let Some(successor) = &self.authorized {
            if card.transport() == successor.transport() {
                match evaluate_card(successor, card)? {
                    CardChange::Unchanged => return Ok(CredentialChange::Unchanged),
                    CardChange::Conflict => return Ok(CredentialChange::Conflict),
                    CardChange::Stale => return Ok(CredentialChange::Stale),
                    CardChange::Newer => {}
                }
            } else if card.epoch() == successor.epoch() {
                return Ok(CredentialChange::Conflict);
            } else if !successor.epoch().is_superseded_by(card.epoch()) {
                return Ok(CredentialChange::Stale);
            }
        }
        self.authorized = Some(card.clone());
        self.prune();
        Ok(CredentialChange::Authorized)
    }

    /// The user confirmed the pending successor `card`: it takes over and
    /// the previous key is retired (`Promoted`).
    ///
    /// `card` is the card the user was shown. Fails with
    /// [`ProtocolError::InvalidValue`] if it is not the pending successor,
    /// so that a pending card that changed after it was shown is never
    /// confirmed in its place, and with
    /// [`ProtocolError::IdentityMismatch`] for a card of another identity.
    pub fn confirm(&mut self, card: &ContactCard) -> Result<CredentialChange, ProtocolError> {
        self.check_identity(card)?;
        match &self.pending {
            Some(pending) if same_statement(pending, card) => {}
            _ => return Err(ProtocolError::InvalidValue),
        }
        self.promote(card.clone());
        Ok(CredentialChange::Promoted)
    }

    /// The user imported `card` by hand for an identity that is held as a
    /// requested contact, not an accepted one.
    ///
    /// Nothing has been accepted yet, so the import is the user's choice
    /// of the card to use. A card with the same statement and another
    /// capability replaces the capability (`Unchanged`; the replacement of
    /// `docs/DESIGN_QUESTIONS.md` P10). A newer card takes the place of
    /// the active one with its capability: `Advanced` with the same key,
    /// `Promoted` with another, which retires the previous key. An older
    /// or contradicting card, or one with the retired key, changes nothing.
    ///
    /// Fails with [`ProtocolError::IdentityMismatch`] for a card of
    /// another identity.
    pub fn replace(&mut self, card: ContactCard) -> Result<CredentialChange, ProtocolError> {
        self.check_identity(&card)?;
        if self.is_retired(&card) {
            return Ok(CredentialChange::Stale);
        }
        let change = match evaluate_card(&self.active, &card)? {
            CardChange::Stale => return Ok(CredentialChange::Stale),
            CardChange::Conflict => return Ok(CredentialChange::Conflict),
            CardChange::Unchanged => {
                self.invitation = card.invitation().cloned();
                return Ok(CredentialChange::Unchanged);
            }
            CardChange::Newer if card.transport() == self.active.transport() => {
                CredentialChange::Advanced
            }
            CardChange::Newer => CredentialChange::Promoted,
        };
        self.invitation = card.invitation().cloned();
        if change == CredentialChange::Promoted {
            self.promote(card);
        } else {
            self.advance(card);
        }
        Ok(change)
    }

    /// The user imported `card` by hand for an accepted contact.
    ///
    /// An import is not a key change. A newer card with the active key is
    /// `Advanced`: the endpoints it states are the user's choice of where
    /// to connect. A newer card with another key is held as the pending
    /// successor (`Pending`); the key takes over only through
    /// [`Self::confirm`], a separate decision about the key itself. A card
    /// with the key of the authorized successor changes nothing: that key
    /// takes over when it is proven. The capability of an accepted contact
    /// is not used and does not change. An older or contradicting card, or
    /// one with the retired key, changes nothing.
    ///
    /// Fails with [`ProtocolError::IdentityMismatch`] for a card of
    /// another identity.
    pub fn import(&mut self, card: ContactCard) -> Result<CredentialChange, ProtocolError> {
        self.check_identity(&card)?;
        if self.is_retired(&card) {
            return Ok(CredentialChange::Stale);
        }
        match evaluate_card(&self.active, &card)? {
            CardChange::Unchanged => return Ok(CredentialChange::Unchanged),
            CardChange::Conflict => return Ok(CredentialChange::Conflict),
            CardChange::Stale => return Ok(CredentialChange::Stale),
            CardChange::Newer => {}
        }
        if card.transport() == self.active.transport() {
            self.advance(card);
            return Ok(CredentialChange::Advanced);
        }
        if self
            .authorized
            .as_ref()
            .is_some_and(|successor| successor.transport() == card.transport())
        {
            return Ok(CredentialChange::Unchanged);
        }
        self.hold_pending(card);
        Ok(CredentialChange::Pending)
    }

    /// A newer card with the active key becomes the active card.
    fn advance(&mut self, card: ContactCard) {
        self.active = card;
        self.prune();
    }

    /// A successor takes over. The previous active key is retired.
    fn promote(&mut self, card: ContactCard) {
        let previous = core::mem::replace(&mut self.active, card);
        self.retired = Some(*previous.transport());
        self.authorized = None;
        self.prune();
    }

    /// Keeps `card` as the pending successor unless the one held is at
    /// least as new. A pending card is only ever a candidate for the user.
    fn hold_pending(&mut self, card: ContactCard) {
        let keep = self
            .pending
            .as_ref()
            .is_some_and(|held| !held.epoch().is_superseded_by(card.epoch()));
        if !keep {
            self.pending = Some(card);
        }
        self.prune();
    }

    /// Drops successors that no longer satisfy the invariants: not newer
    /// than the active card, stating the active or the retired key, or a
    /// pending card for the key that is authorized.
    fn prune(&mut self) {
        let active = self.active.clone();
        let retired = self.retired;
        let valid = |card: &ContactCard| {
            active.epoch().is_superseded_by(card.epoch())
                && card.transport() != active.transport()
                && Some(*card.transport()) != retired
        };
        if !self.authorized.as_ref().is_some_and(valid) {
            self.authorized = None;
        }
        let authorized = self.authorized.as_ref().map(|card| *card.transport());
        if !self
            .pending
            .as_ref()
            .is_some_and(|card| valid(card) && Some(*card.transport()) != authorized)
        {
            self.pending = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::EndpointSet;
    use monolith_identity::{EndpointEpoch, IdentitySecretKey, OnionServiceKey};
    use proptest::prelude::*;

    /// The identity of the contact in these tests.
    const ALICE: u8 = 0x10;
    /// Another identity.
    const MALLORY: u8 = 0x77;

    fn secret(seed: u8) -> IdentitySecretKey {
        IdentitySecretKey::from_seed(&[seed; 32])
    }

    fn transport(seed: u8) -> TransportPublicKey {
        let mut bytes = [seed; 32];
        bytes[31] = 0x40;
        TransportPublicKey::from_bytes(&bytes).unwrap()
    }

    fn endpoint(seed: u8) -> OnionServiceKey {
        OnionServiceKey::from_bytes(secret(seed.wrapping_add(100)).public_key().as_bytes()).unwrap()
    }

    /// A card of `identity` with transport key `key`, `epoch` and endpoint
    /// `place`, optionally with a capability.
    fn signed(identity: u8, key: u8, epoch: u64, place: u8, capability: Option<u8>) -> ContactCard {
        ContactCard::sign(
            &secret(identity),
            transport(key),
            EndpointEpoch::new(epoch).unwrap(),
            EndpointSet::single(endpoint(place)),
            capability.map(|byte| InvitationCapability::from_bytes([byte; 16])),
        )
        .unwrap()
    }

    /// A card of Alice with transport key `key` at `epoch`, at her usual
    /// endpoint.
    fn alice(key: u8, epoch: u64) -> ContactCard {
        signed(ALICE, key, epoch, 1, None)
    }

    /// The keys of Alice's cards, by name.
    const T1: u8 = 1;
    const T2: u8 = 2;
    const T3: u8 = 3;

    fn check(credentials: &Credentials) {
        let active = credentials.active();
        for card in [
            credentials.authorized_successor(),
            credentials.pending_successor(),
        ]
        .into_iter()
        .flatten()
        {
            assert_eq!(card.identity(), active.identity());
            assert!(active.epoch() < card.epoch());
            assert_ne!(card.transport(), active.transport());
            assert_ne!(Some(card.transport()), credentials.retired());
        }
        if let (Some(a), Some(p)) = (
            credentials.authorized_successor(),
            credentials.pending_successor(),
        ) {
            assert_ne!(a.transport(), p.transport());
        }
        assert_ne!(Some(active.transport()), credentials.retired());
    }

    #[test]
    fn the_card_of_a_new_contact_is_active() {
        let card = signed(ALICE, T1, 1, 1, Some(0xC4));
        let credentials = Credentials::new(card.clone());
        assert_eq!(credentials.active(), &card);
        assert_eq!(credentials.identity(), card.identity());
        assert_eq!(
            credentials.invitation(),
            Some(&InvitationCapability::from_bytes([0xC4; 16]))
        );
        assert!(credentials.authorized_successor().is_none());
        assert!(credentials.pending_successor().is_none());
        assert!(credentials.retired().is_none());
        assert!(credentials.authorizes(&alice(T1, 1)));
        assert!(!credentials.authorizes(&alice(T2, 2)));
        assert!(!credentials.authorizes(&signed(MALLORY, T1, 1, 1, None)));
        check(&credentials);
    }

    #[test]
    fn the_active_key_is_admitted_and_older_cards_are_not() {
        let mut credentials = Credentials::new(alice(T1, 3));
        assert_eq!(
            credentials.admit(&alice(T1, 3)),
            Ok(CredentialChange::Unchanged)
        );
        // The capability is not part of the statement.
        assert_eq!(
            credentials.admit(&signed(ALICE, T1, 3, 1, Some(9))),
            Ok(CredentialChange::Unchanged)
        );
        assert_eq!(
            credentials.admit(&alice(T1, 2)),
            Ok(CredentialChange::Stale)
        );
        assert_eq!(
            credentials.admit(&alice(T2, 2)),
            Ok(CredentialChange::Stale)
        );
        assert_eq!(
            credentials.admit(&alice(T2, 3)),
            Ok(CredentialChange::Conflict)
        );
        assert_eq!(
            credentials.admit(&signed(ALICE, T1, 3, 2, None)),
            Ok(CredentialChange::Conflict)
        );
        assert_eq!(
            credentials.admit(&signed(MALLORY, T1, 3, 1, None)),
            Err(ProtocolError::IdentityMismatch)
        );
        assert_eq!(credentials, Credentials::new(alice(T1, 3)));
    }

    #[test]
    fn a_newer_card_with_the_active_key_advances_it() {
        // An endpoint change: the transport credential stays.
        let mut credentials = Credentials::new(alice(T1, 1));
        let moved = signed(ALICE, T1, 2, 2, None);
        assert_eq!(credentials.admit(&moved), Ok(CredentialChange::Advanced));
        assert_eq!(credentials.active(), &moved);
        assert!(credentials.retired().is_none());
        assert!(credentials.authorizes(&alice(T1, 1)));
        // The previous statement is older now.
        assert_eq!(
            credentials.admit(&alice(T1, 1)),
            Ok(CredentialChange::Stale)
        );
        check(&credentials);
    }

    #[test]
    fn a_successor_announced_through_the_active_key_is_authorized() {
        // Test A of the review. A session made with T1 delivers the card of
        // T2. T2 is authorized; T1 stays active and keeps working.
        let mut credentials = Credentials::new(alice(T1, 1));
        let session = alice(T1, 1);
        assert_eq!(
            credentials.announce(&alice(T2, 2), &session),
            Ok(CredentialChange::Authorized)
        );
        assert_eq!(credentials.authorized_successor(), Some(&alice(T2, 2)));
        assert_eq!(credentials.active(), &alice(T1, 1));
        assert!(credentials.authorizes(&session));
        assert_eq!(
            credentials.admit(&alice(T1, 1)),
            Ok(CredentialChange::Unchanged)
        );
        // The same announcement again changes nothing, an older one is
        // stale, a contradicting one for the same key is a conflict.
        assert_eq!(
            credentials.announce(&alice(T2, 2), &session),
            Ok(CredentialChange::Unchanged)
        );
        assert_eq!(
            credentials.announce(&signed(ALICE, T2, 2, 2, None), &session),
            Ok(CredentialChange::Conflict)
        );
        assert_eq!(
            credentials.announce(&alice(T3, 2), &session),
            Ok(CredentialChange::Conflict)
        );
        // A newer announcement replaces the authorized successor.
        assert_eq!(
            credentials.announce(&alice(T3, 3), &session),
            Ok(CredentialChange::Authorized)
        );
        assert_eq!(credentials.authorized_successor(), Some(&alice(T3, 3)));
        assert_eq!(
            credentials.announce(&alice(T2, 2), &session),
            Ok(CredentialChange::Stale)
        );
        check(&credentials);
    }

    #[test]
    fn a_proven_successor_is_promoted_and_the_old_key_retired() {
        // Tests B, C and D of the review.
        let mut credentials = Credentials::new(alice(T1, 1));
        let old_session = alice(T1, 1);
        credentials.announce(&alice(T2, 2), &old_session).unwrap();
        assert_eq!(
            credentials.admit(&alice(T2, 2)),
            Ok(CredentialChange::Promoted)
        );
        assert_eq!(credentials.active(), &alice(T2, 2));
        assert_eq!(credentials.retired(), Some(&transport(T1)));
        assert!(credentials.authorized_successor().is_none());
        // A session made with T1 no longer stands for Alice, and a new one
        // is refused, with any card that states T1.
        assert!(!credentials.authorizes(&old_session));
        assert!(credentials.authorizes(&alice(T2, 2)));
        assert_eq!(
            credentials.admit(&alice(T1, 1)),
            Ok(CredentialChange::Stale)
        );
        assert_eq!(
            credentials.admit(&alice(T1, 9)),
            Ok(CredentialChange::Stale)
        );
        assert_eq!(
            credentials.announce(&alice(T3, 3), &old_session),
            Ok(CredentialChange::NoContinuity)
        );
        check(&credentials);
    }

    #[test]
    fn a_successor_card_older_than_the_announced_one_is_stale() {
        let mut credentials = Credentials::new(alice(T1, 1));
        credentials.announce(&alice(T2, 3), &alice(T1, 1)).unwrap();
        // The key of the successor with an older statement.
        assert_eq!(
            credentials.admit(&alice(T2, 2)),
            Ok(CredentialChange::Stale)
        );
        assert_eq!(
            credentials.admit(&signed(ALICE, T2, 3, 2, None)),
            Ok(CredentialChange::Conflict)
        );
        assert_eq!(credentials.active(), &alice(T1, 1));
        // A newer statement for the authorized key is proven and promoted.
        assert_eq!(
            credentials.admit(&signed(ALICE, T2, 4, 2, None)),
            Ok(CredentialChange::Promoted)
        );
        assert_eq!(credentials.active(), &signed(ALICE, T2, 4, 2, None));
        check(&credentials);
    }

    #[test]
    fn a_newer_key_without_continuity_is_only_pending() {
        // Test E of the review. Whoever holds Alice's identity key, and not
        // T1, signs a card for T2 and completes a handshake with it.
        let mut credentials = Credentials::new(alice(T1, 1));
        assert_eq!(
            credentials.admit(&alice(T2, 2)),
            Ok(CredentialChange::Pending)
        );
        assert_eq!(credentials.pending_successor(), Some(&alice(T2, 2)));
        // Nothing else moved: T1 is active and is not locked out.
        assert_eq!(credentials.active(), &alice(T1, 1));
        assert!(credentials.retired().is_none());
        assert!(credentials.authorizes(&alice(T1, 1)));
        assert!(!credentials.authorizes(&alice(T2, 2)));
        assert_eq!(
            credentials.admit(&alice(T1, 1)),
            Ok(CredentialChange::Unchanged)
        );
        // Presenting it again keeps it pending. A higher epoch does not
        // help.
        assert_eq!(
            credentials.admit(&alice(T2, 2)),
            Ok(CredentialChange::Pending)
        );
        assert_eq!(
            credentials.admit(&alice(T3, 50)),
            Ok(CredentialChange::Pending)
        );
        assert_eq!(credentials.pending_successor(), Some(&alice(T3, 50)));
        assert_eq!(
            credentials.admit(&alice(T2, 3)),
            Ok(CredentialChange::Pending)
        );
        assert_eq!(credentials.pending_successor(), Some(&alice(T3, 50)));
        assert_eq!(credentials.active(), &alice(T1, 1));
        check(&credentials);
    }

    #[test]
    fn a_pending_successor_takes_over_only_when_confirmed() {
        // Test F of the review.
        let mut credentials = Credentials::new(alice(T1, 1));
        credentials.admit(&alice(T2, 2)).unwrap();
        // Only the card that is pending can be confirmed.
        assert_eq!(
            credentials.confirm(&alice(T3, 2)),
            Err(ProtocolError::InvalidValue)
        );
        assert_eq!(
            credentials.confirm(&alice(T2, 3)),
            Err(ProtocolError::InvalidValue)
        );
        assert_eq!(
            credentials.confirm(&signed(MALLORY, T2, 2, 1, None)),
            Err(ProtocolError::IdentityMismatch)
        );
        assert_eq!(credentials.active(), &alice(T1, 1));

        assert_eq!(
            credentials.confirm(&alice(T2, 2)),
            Ok(CredentialChange::Promoted)
        );
        assert_eq!(credentials.active(), &alice(T2, 2));
        assert_eq!(credentials.retired(), Some(&transport(T1)));
        assert!(credentials.pending_successor().is_none());
        assert!(!credentials.authorizes(&alice(T1, 1)));
        assert_eq!(
            credentials.confirm(&alice(T2, 2)),
            Err(ProtocolError::InvalidValue)
        );
        check(&credentials);

        // Nothing to confirm without a pending card.
        let mut fresh = Credentials::new(alice(T1, 1));
        assert_eq!(
            fresh.confirm(&alice(T1, 1)),
            Err(ProtocolError::InvalidValue)
        );
    }

    #[test]
    fn invalid_cards_never_become_successors() {
        // Test G of the review. A card with a bad signature or a mismatched
        // transport key never reaches this type: decoding verifies the
        // signature, and the handshake refuses a card whose key is not the
        // one it proved. What reaches it is checked for the identity.
        let mut credentials = Credentials::new(alice(T1, 1));
        let foreign = signed(MALLORY, T2, 2, 1, None);
        assert_eq!(
            credentials.admit(&foreign),
            Err(ProtocolError::IdentityMismatch)
        );
        assert_eq!(
            credentials.announce(&foreign, &alice(T1, 1)),
            Err(ProtocolError::IdentityMismatch)
        );
        assert_eq!(
            credentials.announce(&alice(T2, 2), &signed(MALLORY, T1, 1, 1, None)),
            Err(ProtocolError::IdentityMismatch)
        );
        assert_eq!(
            credentials.import(foreign.clone()),
            Err(ProtocolError::IdentityMismatch)
        );
        assert_eq!(
            credentials.replace(foreign),
            Err(ProtocolError::IdentityMismatch)
        );
        assert_eq!(credentials, Credentials::new(alice(T1, 1)));
    }

    #[test]
    fn an_announcement_needs_a_session_made_with_the_active_key() {
        let mut credentials = Credentials::new(alice(T1, 1));
        // A session with another key of Alice: a pending one, or one that
        // was never held.
        credentials.admit(&alice(T2, 2)).unwrap();
        assert_eq!(
            credentials.announce(&alice(T3, 3), &alice(T2, 2)),
            Ok(CredentialChange::NoContinuity)
        );
        assert!(credentials.authorized_successor().is_none());
        // Through the active key the same card is authorized, and the
        // pending card stays a separate candidate.
        assert_eq!(
            credentials.announce(&alice(T3, 3), &alice(T1, 1)),
            Ok(CredentialChange::Authorized)
        );
        assert_eq!(credentials.pending_successor(), Some(&alice(T2, 2)));
        // An announcement of the pending key through the active one
        // authorizes it, and it is no longer pending.
        assert_eq!(
            credentials.announce(&alice(T2, 4), &alice(T1, 1)),
            Ok(CredentialChange::Authorized)
        );
        assert_eq!(credentials.authorized_successor(), Some(&alice(T2, 4)));
        assert!(credentials.pending_successor().is_none());
        check(&credentials);
    }

    #[test]
    fn a_newer_active_card_drops_successors_it_supersedes() {
        let mut credentials = Credentials::new(alice(T1, 1));
        credentials.announce(&alice(T2, 2), &alice(T1, 1)).unwrap();
        credentials.admit(&alice(T3, 3)).unwrap();
        // Alice moves on with T1 at epoch 3: the successor of epoch 2 is
        // gone, the pending card of epoch 3 too.
        assert_eq!(
            credentials.admit(&signed(ALICE, T1, 3, 2, None)),
            Ok(CredentialChange::Advanced)
        );
        assert!(credentials.authorized_successor().is_none());
        assert!(credentials.pending_successor().is_none());
        check(&credentials);
    }

    #[test]
    fn importing_for_an_accepted_contact_never_replaces_the_key() {
        let mut credentials = Credentials::new(signed(ALICE, T1, 1, 1, Some(1)));
        // An endpoint change is taken.
        let moved = signed(ALICE, T1, 2, 2, None);
        assert_eq!(
            credentials.import(moved.clone()),
            Ok(CredentialChange::Advanced)
        );
        assert_eq!(credentials.active(), &moved);
        // A new key is only pending, and a capability does not change.
        assert_eq!(
            credentials.import(signed(ALICE, T2, 3, 2, Some(2))),
            Ok(CredentialChange::Pending)
        );
        assert_eq!(credentials.active(), &moved);
        assert_eq!(
            credentials.invitation(),
            Some(&InvitationCapability::from_bytes([1; 16]))
        );
        assert_eq!(
            credentials.import(signed(ALICE, T1, 2, 2, Some(3))),
            Ok(CredentialChange::Unchanged)
        );
        assert_eq!(
            credentials.invitation(),
            Some(&InvitationCapability::from_bytes([1; 16]))
        );
        // The key of the authorized successor changes nothing.
        credentials
            .announce(&signed(ALICE, T3, 4, 2, None), &moved)
            .unwrap();
        assert_eq!(
            credentials.import(signed(ALICE, T3, 5, 2, None)),
            Ok(CredentialChange::Unchanged)
        );
        assert_eq!(
            credentials.authorized_successor(),
            Some(&signed(ALICE, T3, 4, 2, None))
        );
        assert_eq!(
            credentials.import(alice(T1, 1)),
            Ok(CredentialChange::Stale)
        );
        check(&credentials);
    }

    #[test]
    fn importing_for_a_requested_contact_replaces_the_card() {
        // P10: the same statement with another capability replaces the
        // capability, in every direction.
        let mut credentials = Credentials::new(signed(ALICE, T1, 1, 1, Some(1)));
        for (capability, expected) in [
            (Some(2), Some(InvitationCapability::from_bytes([2; 16]))),
            (None, None),
            (Some(3), Some(InvitationCapability::from_bytes([3; 16]))),
        ] {
            assert_eq!(
                credentials.replace(signed(ALICE, T1, 1, 1, capability)),
                Ok(CredentialChange::Unchanged)
            );
            assert_eq!(credentials.invitation(), expected.as_ref());
        }
        // A newer card with the same key advances, with another key it
        // takes over and retires the previous key.
        assert_eq!(
            credentials.replace(signed(ALICE, T1, 2, 2, None)),
            Ok(CredentialChange::Advanced)
        );
        assert_eq!(credentials.invitation(), None);
        assert_eq!(
            credentials.replace(signed(ALICE, T2, 3, 2, Some(4))),
            Ok(CredentialChange::Promoted)
        );
        assert_eq!(credentials.retired(), Some(&transport(T1)));
        assert_eq!(
            credentials.invitation(),
            Some(&InvitationCapability::from_bytes([4; 16]))
        );
        // Older, contradicting and retired cards change nothing.
        let before = credentials.clone();
        assert_eq!(
            credentials.replace(signed(ALICE, T1, 9, 2, None)),
            Ok(CredentialChange::Stale)
        );
        assert_eq!(
            credentials.replace(signed(ALICE, T2, 2, 2, None)),
            Ok(CredentialChange::Stale)
        );
        assert_eq!(
            credentials.replace(signed(ALICE, T3, 3, 2, None)),
            Ok(CredentialChange::Conflict)
        );
        assert_eq!(credentials, before);
        check(&credentials);
    }

    #[test]
    fn simultaneous_rotation_on_both_sides_completes() {
        // Alice rotates T1 to T2 while Bob rotates U1 to U2. Both keep the
        // old key until the successors are exchanged. Each side's view of
        // the other is a `Credentials`.
        const BOB: u8 = 0x51;
        const U1: u8 = 11;
        const U2: u8 = 12;
        let bob = |key: u8, epoch: u64| signed(BOB, key, epoch, 2, None);
        let mut alice_at_bob = Credentials::new(alice(T1, 1));
        let mut bob_at_alice = Credentials::new(bob(U1, 1));

        // A session between T1 and U1, in either direction, carries both
        // EndpointUpdates.
        assert_eq!(
            alice_at_bob.announce(&alice(T2, 2), &alice(T1, 1)),
            Ok(CredentialChange::Authorized)
        );
        assert_eq!(
            bob_at_alice.announce(&bob(U2, 2), &bob(U1, 1)),
            Ok(CredentialChange::Authorized)
        );
        // Alice dials Bob's active key U1 with her new key T2: Bob promotes
        // T2, and for Alice U1 is still the active key.
        assert_eq!(
            alice_at_bob.admit(&alice(T2, 2)),
            Ok(CredentialChange::Promoted)
        );
        assert_eq!(
            bob_at_alice.admit(&bob(U1, 1)),
            Ok(CredentialChange::Unchanged)
        );
        // Bob dials Alice's new active key T2 with U2: Alice promotes U2.
        assert_eq!(
            bob_at_alice.admit(&bob(U2, 2)),
            Ok(CredentialChange::Promoted)
        );
        assert_eq!(
            alice_at_bob.admit(&alice(T2, 2)),
            Ok(CredentialChange::Unchanged)
        );
        assert_eq!(alice_at_bob.retired(), Some(&transport(T1)));
        assert_eq!(bob_at_alice.retired(), Some(&transport(U1)));
        check(&alice_at_bob);
        check(&bob_at_alice);
    }

    #[test]
    fn rotation_without_the_old_keys_fails_safely() {
        // Both drop the old key before either successor card has reached
        // the other. The new keys arrive without continuity: pending, no
        // standing, nothing taken over. Recovery is a card exchanged out of
        // band and confirmed.
        let mut alice_at_bob = Credentials::new(alice(T1, 1));
        assert_eq!(
            alice_at_bob.admit(&alice(T2, 2)),
            Ok(CredentialChange::Pending)
        );
        assert_eq!(alice_at_bob.active(), &alice(T1, 1));
        assert_eq!(
            alice_at_bob.import(alice(T2, 2)),
            Ok(CredentialChange::Pending)
        );
        assert_eq!(
            alice_at_bob.confirm(&alice(T2, 2)),
            Ok(CredentialChange::Promoted)
        );
        assert!(alice_at_bob.authorizes(&alice(T2, 2)));
    }

    /// One event in the life of the credentials of Alice.
    #[derive(Clone, Debug)]
    enum Event {
        Admit(u8, u64),
        Announce(u8, u64, u8),
        Confirm(u8, u64),
        Import(u8, u64),
        Replace(u8, u64),
    }

    fn event() -> impl Strategy<Value = Event> {
        let key = 1_u8..5;
        let epoch = 1_u64..7;
        prop_oneof![
            (key.clone(), epoch.clone()).prop_map(|(k, e)| Event::Admit(k, e)),
            (key.clone(), epoch.clone(), 1_u8..5).prop_map(|(k, e, s)| Event::Announce(k, e, s)),
            (key.clone(), epoch.clone()).prop_map(|(k, e)| Event::Confirm(k, e)),
            (key.clone(), epoch.clone()).prop_map(|(k, e)| Event::Import(k, e)),
            (key, epoch).prop_map(|(k, e)| Event::Replace(k, e)),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn the_active_key_changes_only_through_continuity_or_the_user(
            events in proptest::collection::vec(event(), 1..24),
        ) {
            // Whatever happens: the active epoch never goes down, the
            // invariants hold, and the active key changes only by
            // promoting an authorized successor that is proven, by
            // confirming the pending one, or by a replacement the user
            // made for a contact that had not accepted.
            let mut credentials = Credentials::new(alice(1, 1));
            for event in events {
                let before = credentials.clone();
                let change = match &event {
                    Event::Admit(key, epoch) => credentials.admit(&alice(*key, *epoch)),
                    Event::Announce(key, epoch, session) => credentials
                        .announce(&alice(*key, *epoch), &alice(*session, before.active().epoch().get())),
                    Event::Confirm(key, epoch) => credentials.confirm(&alice(*key, *epoch)),
                    Event::Import(key, epoch) => credentials.import(alice(*key, *epoch)),
                    Event::Replace(key, epoch) => credentials.replace(alice(*key, *epoch)),
                };
                check(&credentials);
                prop_assert!(before.active().epoch() <= credentials.active().epoch());
                let key_changed = before.active().transport() != credentials.active().transport();
                prop_assert_eq!(key_changed, change == Ok(CredentialChange::Promoted));
                if key_changed {
                    let new_key = credentials.active().transport();
                    let allowed = match &event {
                        Event::Admit(..) => before
                            .authorized_successor()
                            .is_some_and(|card| card.transport() == new_key),
                        Event::Confirm(..) => before
                            .pending_successor()
                            .is_some_and(|card| card.transport() == new_key),
                        Event::Replace(..) => true,
                        Event::Announce(..) | Event::Import(..) => false,
                    };
                    prop_assert!(allowed, "{:?}", event);
                    prop_assert_eq!(credentials.retired(), Some(before.active().transport()));
                }
                // Nothing but these changes leaves the credentials as they
                // were.
                if matches!(
                    change,
                    Ok(CredentialChange::Unchanged
                        | CredentialChange::Conflict
                        | CredentialChange::Stale
                        | CredentialChange::NoContinuity)
                ) && !matches!(event, Event::Replace(..))
                {
                    prop_assert_eq!(&credentials, &before);
                }
            }
        }
    }
}
