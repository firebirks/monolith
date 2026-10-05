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
//!   Only that one key is remembered. A key retired earlier is refused in
//!   its old cards by their epochs; a newer card that states it is treated
//!   like any other new key without continuity.
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
//!
//! A value of this type is the state of one contact. Deciding on a copy of
//! it and keeping the original is the stale admission that
//! `docs/SECURITY_INVARIANTS.md` S50 rules out; the contact store works on
//! the one it keeps, under its lock.

use monolith_identity::{IdentityPublicKey, TransportPublicKey};

use crate::ProtocolError;
use crate::card::{CardChange, ContactCard, InvitationCapability, evaluate_card};

/// What a card is, compared with the credentials of its identity.
///
/// This is the classification of `docs/PROTOCOL.md` section 11.4. It does
/// not depend on where the card came from: a card presented in a
/// handshake, received in an EndpointUpdate, or imported by the user is
/// the same card and gets the same relation. Where it came from decides
/// only what is done about it ([`Source`], [`CredentialChange`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CardRelation {
    /// The statement of the active card: same epoch, transport key and
    /// endpoint set. The capability may differ.
    Same,
    /// An older card that states the active transport key.
    OlderActiveKey,
    /// A newer card that states the active transport key.
    NewerActiveKey,
    /// A card of the key of the authorized successor that is not older
    /// than the announced one. `same` is true for the announced statement
    /// itself.
    Successor {
        /// The card makes the statement of the announced successor.
        same: bool,
    },
    /// A card of another key that is newer than the active card and than
    /// the authorized successor, if there is one.
    NewKey,
    /// The card has the epoch of the active card or of the authorized
    /// successor and states something else.
    Conflict,
    /// The card states the retired key, is a card of another key that is
    /// not newer than the active card or than the authorized successor, or
    /// a card of the key of the authorized successor older than the
    /// announced one.
    Stale,
}

/// Where a card comes from, which decides what may be done with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source<'a> {
    /// A handshake proved that the peer holds the transport key of the
    /// card: the card the peer presented in the third message, or the
    /// card that was dialed.
    Proven,
    /// An EndpointUpdate carried the card on a session whose peer is the
    /// given card, whose transport key the handshake proved.
    Announced(&'a ContactCard),
    /// The user imported the card by hand.
    Imported(Holding),
    /// The user confirmed the card shown as the pending successor.
    Confirmed,
}

/// How the identity of an imported card is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Holding {
    /// The user imported a card of this identity and has not seen an
    /// acceptance. An import is the user's choice of the card to use.
    Requested,
    /// An accepted contact. An import is never a key change.
    Accepted,
}

/// What an event did to the credentials of a contact.
///
/// The outcome for each [`CardRelation`] and [`Source`] is the table of
/// [`Credentials::evaluate`]. [`Source::Proven`] gives `Unchanged`,
/// `Superseded`, `Advanced`, `Promoted`, `Pending`, `Conflict` or `Stale`.
/// [`Source::Announced`] gives `Unchanged`, `Superseded`, `Advanced`,
/// `Authorized`, `Conflict`, `Stale` or `NoContinuity`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CredentialChange {
    /// The card states what is already held. Nothing changes, but for an
    /// import for a requested contact, which takes the capability of the
    /// card.
    Unchanged,
    /// An older card that states the active transport key. The card is
    /// superseded and changes nothing. In a handshake the key is the one
    /// that stands for the contact, so its holder is the contact: a peer
    /// restored from an old backup, or a dial of a card whose newer
    /// endpoints the user has not confirmed yet.
    Superseded,
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
    /// successor, and states something else. The identity signed two
    /// statements for one epoch. Nothing changes; the user is told.
    Conflict,
    /// The card is older than the active card with another key than the
    /// active one, is not newer than the authorized successor, or states
    /// the retired key. Nothing changes.
    Stale,
    /// An EndpointUpdate arrived on a session that was not authenticated
    /// with the active key. It proves no continuity. Nothing changes.
    NoContinuity,
}

/// The result of evaluating a card against the credentials of its
/// identity: what the card is, what it does, and the credentials after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evaluation {
    /// What the card is, whatever its source. `None` for an announcement
    /// on a session without continuity, which is refused before the card
    /// is looked at, and for a confirmation.
    pub relation: Option<CardRelation>,
    /// What the card does to the credentials.
    pub change: CredentialChange,
    /// The credentials after the event, if they change.
    next: Option<Credentials>,
    /// The transport key the event retires, if it promotes a successor.
    retired: Option<TransportPublicKey>,
}

impl Evaluation {
    /// Returns the credentials after the event, if they change.
    pub const fn next(&self) -> Option<&Credentials> {
        self.next.as_ref()
    }

    /// Returns true if the event changes the credentials.
    pub const fn changes_state(&self) -> bool {
        self.next.is_some()
    }

    /// Returns the transport key that the event retires: the active key
    /// before a promotion.
    pub const fn retires(&self) -> Option<&TransportPublicKey> {
        self.retired.as_ref()
    }

    /// Returns true if sessions authenticated with the retired key lose
    /// their standing and have to be withdrawn.
    pub const fn withdraws_sessions(&self) -> bool {
        self.retired.is_some()
    }

    /// Returns true if the event changes the active card: a newer card of
    /// the active key, or a promoted successor. Where Monolith dials still
    /// follows the card the user confirmed for dialing.
    pub const fn updates_active_card(&self) -> bool {
        matches!(
            self.change,
            CredentialChange::Advanced | CredentialChange::Promoted
        )
    }

    /// Returns true if the card is held for the user to confirm: a newer
    /// key without continuity.
    pub const fn needs_confirmation(&self) -> bool {
        matches!(self.change, CredentialChange::Pending)
    }

    /// Returns true if the identity signed two statements for one epoch.
    /// That is reported to the user, and never to the peer.
    pub const fn is_conflict(&self) -> bool {
        matches!(self.change, CredentialChange::Conflict)
    }
}

/// The transport credential of one contact.
///
/// Invariants, kept by every function:
///
/// - Every card held is of the identity of the active card.
/// - A successor has a greater epoch than the active card and another
///   transport key than the active one and the retired one.
/// - The pending successor has a greater epoch than the authorized one,
///   if there is one.
/// - The authorized and the pending successor state different keys.
/// - Only a held pending card can be marked as imported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credentials {
    /// The newest card accepted for the active transport key.
    active: ContactCard,
    authorized: Option<ContactCard>,
    pending: Option<ContactCard>,
    /// The pending card was imported by the user. A card a peer presents
    /// does not take its place.
    pending_imported: bool,
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
            pending_imported: false,
            retired: None,
            invitation,
        }
    }

    /// Rebuilds credentials that were stored, from their parts.
    ///
    /// Fails with [`ProtocolError::InvalidValue`] if the parts break an
    /// invariant of this type, and with [`ProtocolError::IdentityMismatch`]
    /// if a successor is of another identity. Stored credentials are
    /// checked as strictly as the functions that made them, so that a
    /// store that was damaged or written by a faulty version can never
    /// hold a state that no transition could have produced.
    pub fn restore(
        active: ContactCard,
        authorized: Option<ContactCard>,
        pending: Option<ContactCard>,
        pending_imported: bool,
        retired: Option<TransportPublicKey>,
        invitation: Option<InvitationCapability>,
    ) -> Result<Self, ProtocolError> {
        let credentials = Self {
            active,
            authorized,
            pending,
            pending_imported,
            retired,
            invitation,
        };
        for card in [&credentials.authorized, &credentials.pending]
            .into_iter()
            .flatten()
        {
            credentials.check_identity(card)?;
        }
        let mut pruned = credentials.clone();
        pruned.prune();
        if pruned != credentials
            || credentials.retired.as_ref() == Some(credentials.active.transport())
        {
            return Err(ProtocolError::InvalidValue);
        }
        Ok(credentials)
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

    /// Returns true if the pending successor is a card the user imported,
    /// false if a peer presented it or there is none.
    pub const fn pending_was_imported(&self) -> bool {
        self.pending_imported
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
    /// transport key the handshake proved.
    ///
    /// For a session that was admitted with the standing of a contact,
    /// requested or accepted, this is true at admission. When it turns
    /// false, the session has to be withdrawn before anything else is done
    /// with the contact, before the duplicate rule in particular. A session
    /// admitted with any other standing is on the path of a stranger and is
    /// left to it: ending it early would tell the peer that it is held as a
    /// contact (`docs/PROTOCOL.md` section 12.1).
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

    /// True if `card` has the epoch of the authorized successor and states
    /// something else: two statements for one epoch.
    fn contradicts_successor(&self, card: &ContactCard) -> bool {
        self.authorized.as_ref().is_some_and(|successor| {
            successor.epoch() == card.epoch() && !same_statement(successor, card)
        })
    }

    /// Classifies `card` against these credentials. This is the one place
    /// that decides what a card is (`docs/PROTOCOL.md` section 11.4, rule
    /// 1 to 5); every source of a card goes through it.
    ///
    /// Fails with [`ProtocolError::IdentityMismatch`] for a card of
    /// another identity.
    pub fn relation(&self, card: &ContactCard) -> Result<CardRelation, ProtocolError> {
        self.check_identity(card)?;
        if self.is_retired(card) {
            return Ok(CardRelation::Stale);
        }
        match evaluate_card(&self.active, card)? {
            CardChange::Unchanged => return Ok(CardRelation::Same),
            CardChange::Conflict => return Ok(CardRelation::Conflict),
            CardChange::Stale if card.transport() == self.active.transport() => {
                return Ok(CardRelation::OlderActiveKey);
            }
            CardChange::Stale => return Ok(CardRelation::Stale),
            CardChange::Newer => {}
        }
        if self.contradicts_successor(card) {
            return Ok(CardRelation::Conflict);
        }
        if card.transport() == self.active.transport() {
            return Ok(CardRelation::NewerActiveKey);
        }
        let Some(successor) = &self.authorized else {
            return Ok(CardRelation::NewKey);
        };
        if card.transport() == successor.transport() {
            return Ok(match evaluate_card(successor, card)? {
                CardChange::Unchanged => CardRelation::Successor { same: true },
                CardChange::Newer => CardRelation::Successor { same: false },
                CardChange::Conflict => CardRelation::Conflict,
                CardChange::Stale => CardRelation::Stale,
            });
        }
        // Another key that is not newer than the successor the identity
        // announced through the active key: the identity superseded it.
        if successor.epoch().is_superseded_by(card.epoch()) {
            Ok(CardRelation::NewKey)
        } else {
            Ok(CardRelation::Stale)
        }
    }

    /// Decides what `card` from `source` does to these credentials,
    /// without changing them. [`Self::apply`] makes the change.
    ///
    /// | Relation | Proven | Announced | Imported, accepted | Imported, requested |
    /// | --- | --- | --- | --- | --- |
    /// | `Same` | `Unchanged` | `Unchanged` | `Unchanged` | `Unchanged`, capability taken |
    /// | `OlderActiveKey` | `Superseded` | `Superseded` | `Superseded` | `Superseded` |
    /// | `NewerActiveKey` | `Advanced` | `Advanced` | `Advanced` | `Advanced`, capability taken |
    /// | `Successor`, same | `Promoted` | `Unchanged` | `Unchanged` | `Promoted`, capability taken |
    /// | `Successor`, newer | `Promoted` | `Authorized` | `Unchanged` | `Promoted`, capability taken |
    /// | `NewKey` | `Pending` | `Authorized` | `Pending` | `Promoted`, capability taken |
    /// | `Conflict` | `Conflict` | `Conflict` | `Conflict` | `Conflict` |
    /// | `Stale` | `Stale` | `Stale` | `Stale` | `Stale` |
    ///
    /// An announcement on a session that was not made with the active key
    /// is `NoContinuity` before the card is looked at. A confirmation
    /// promotes the pending successor if `card` makes its statement, and
    /// fails with [`ProtocolError::InvalidValue`] otherwise, so that a
    /// pending card that changed after it was shown is never confirmed in
    /// its place.
    ///
    /// A pending card the user imported is not replaced by a presented
    /// one; the result is still `Pending`.
    ///
    /// Fails with [`ProtocolError::IdentityMismatch`] if `card`, or the
    /// card of the session of an announcement, is of another identity.
    pub fn evaluate(
        &self,
        card: &ContactCard,
        source: Source<'_>,
    ) -> Result<Evaluation, ProtocolError> {
        let mut next = self.clone();
        let (relation, change) = next.step(card, source)?;
        let retired = (change == CredentialChange::Promoted).then(|| *self.active.transport());
        let next = (next != *self).then_some(next);
        Ok(Evaluation {
            relation,
            change,
            next,
            retired,
        })
    }

    /// Evaluates `card` from `source` ([`Self::evaluate`]) and records the
    /// change.
    pub fn apply(
        &mut self,
        card: &ContactCard,
        source: Source<'_>,
    ) -> Result<CredentialChange, ProtocolError> {
        let evaluation = self.evaluate(card, source)?;
        if let Some(next) = evaluation.next {
            *self = next;
        }
        Ok(evaluation.change)
    }

    /// The table of [`Self::evaluate`], applied to `self`.
    fn step(
        &mut self,
        card: &ContactCard,
        source: Source<'_>,
    ) -> Result<(Option<CardRelation>, CredentialChange), ProtocolError> {
        self.check_identity(card)?;
        match source {
            Source::Announced(session) => {
                self.check_identity(session)?;
                if session.transport() != self.active.transport() {
                    return Ok((None, CredentialChange::NoContinuity));
                }
            }
            Source::Confirmed => {
                match &self.pending {
                    Some(pending) if same_statement(pending, card) => {}
                    _ => return Err(ProtocolError::InvalidValue),
                }
                self.promote(card.clone());
                return Ok((None, CredentialChange::Promoted));
            }
            Source::Proven | Source::Imported(_) => {}
        }
        let relation = self.relation(card)?;
        let requested = source == Source::Imported(Holding::Requested);
        let change = match relation {
            CardRelation::Conflict => CredentialChange::Conflict,
            CardRelation::Stale => CredentialChange::Stale,
            CardRelation::OlderActiveKey => CredentialChange::Superseded,
            CardRelation::Same => {
                if requested {
                    // P10: the same statement with another capability.
                    self.invitation = card.invitation().cloned();
                }
                CredentialChange::Unchanged
            }
            CardRelation::NewerActiveKey => {
                if requested {
                    self.invitation = card.invitation().cloned();
                }
                self.advance(card.clone());
                CredentialChange::Advanced
            }
            CardRelation::Successor { same } => match source {
                Source::Proven => {
                    self.promote(card.clone());
                    CredentialChange::Promoted
                }
                Source::Announced(_) if same => CredentialChange::Unchanged,
                Source::Announced(_) => {
                    self.authorize(card.clone());
                    CredentialChange::Authorized
                }
                // That key takes over when its holder proves it.
                Source::Imported(Holding::Accepted) => CredentialChange::Unchanged,
                Source::Imported(Holding::Requested) => {
                    self.invitation = card.invitation().cloned();
                    self.promote(card.clone());
                    CredentialChange::Promoted
                }
                Source::Confirmed => return Err(ProtocolError::InvalidValue),
            },
            CardRelation::NewKey => match source {
                Source::Proven => {
                    self.hold_pending(card.clone());
                    CredentialChange::Pending
                }
                Source::Announced(_) => {
                    self.authorize(card.clone());
                    CredentialChange::Authorized
                }
                Source::Imported(Holding::Accepted) => {
                    self.pending = Some(card.clone());
                    self.pending_imported = true;
                    self.prune();
                    CredentialChange::Pending
                }
                Source::Imported(Holding::Requested) => {
                    self.invitation = card.invitation().cloned();
                    self.promote(card.clone());
                    CredentialChange::Promoted
                }
                Source::Confirmed => return Err(ProtocolError::InvalidValue),
            },
        };
        Ok((Some(relation), change))
    }

    /// A handshake proved that the peer holds the transport key of `card`:
    /// the card the peer presented in the third message, or the card that
    /// was dialed. [`Self::apply`] with [`Source::Proven`].
    ///
    /// The active key with an older card is `Superseded`: its holder is
    /// the contact whichever older card of that key it shows, and the card
    /// is not taken. A newer key without continuity is only `Pending`: the
    /// active key is not touched, so whoever holds the identity key alone
    /// cannot take the contact over or lock the holder of the active key
    /// out.
    pub fn admit(&mut self, card: &ContactCard) -> Result<CredentialChange, ProtocolError> {
        self.apply(card, Source::Proven)
    }

    /// An EndpointUpdate with `card` arrived on a session whose peer is
    /// `session`. [`Self::apply`] with [`Source::Announced`]: only a
    /// session made with the active key carries continuity, and a newer
    /// card with another key becomes the authorized successor, replacing an
    /// older one. The active key stays active until the successor is
    /// proven.
    pub fn announce(
        &mut self,
        card: &ContactCard,
        session: &ContactCard,
    ) -> Result<CredentialChange, ProtocolError> {
        self.apply(card, Source::Announced(session))
    }

    /// The user confirmed the pending successor `card`, the card the user
    /// was shown. [`Self::apply`] with [`Source::Confirmed`].
    pub fn confirm(&mut self, card: &ContactCard) -> Result<CredentialChange, ProtocolError> {
        self.apply(card, Source::Confirmed)
    }

    /// The user imported `card` by hand for an identity held as a
    /// requested contact. [`Self::apply`] with
    /// [`Source::Imported`]`(`[`Holding::Requested`]`)`: nothing has been
    /// accepted yet, so a newer card is the user's choice of the card to
    /// use and takes the place of the active one with its capability, and
    /// the same statement with another capability replaces the capability
    /// (P10).
    pub fn replace(&mut self, card: ContactCard) -> Result<CredentialChange, ProtocolError> {
        self.apply(&card, Source::Imported(Holding::Requested))
    }

    /// The user imported `card` by hand for an accepted contact.
    /// [`Self::apply`] with [`Source::Imported`]`(`[`Holding::Accepted`]`)`:
    /// an import is not a key change. A newer key is held as the pending
    /// successor in place of any pending card, and takes over only through
    /// [`Self::confirm`], a separate decision about the key itself. The
    /// capability of an accepted contact does not change.
    pub fn import(&mut self, card: ContactCard) -> Result<CredentialChange, ProtocolError> {
        self.apply(&card, Source::Imported(Holding::Accepted))
    }

    /// A newer card with the active key becomes the active card.
    fn advance(&mut self, card: ContactCard) {
        self.active = card;
        self.prune();
    }

    /// A newer card with another key, announced through the active key,
    /// becomes the authorized successor.
    fn authorize(&mut self, card: ContactCard) {
        self.authorized = Some(card);
        self.prune();
    }

    /// A successor takes over. The previous active key is retired.
    fn promote(&mut self, card: ContactCard) {
        let previous = core::mem::replace(&mut self.active, card);
        self.retired = Some(*previous.transport());
        self.authorized = None;
        self.prune();
    }

    /// Keeps the presented `card` as the pending successor unless the one
    /// held was imported by the user or is at least as new. A pending card
    /// is only ever a candidate for the user.
    fn hold_pending(&mut self, card: ContactCard) {
        let keep = self.pending.as_ref().is_some_and(|held| {
            self.pending_imported || !held.epoch().is_superseded_by(card.epoch())
        });
        if !keep {
            self.pending = Some(card);
            self.pending_imported = false;
        }
        self.prune();
    }

    /// Drops successors that no longer satisfy the invariants: not newer
    /// than the active card, stating the active or the retired key, a
    /// pending card for the key that is authorized, or a pending card that
    /// is not newer than the authorized successor, which the identity
    /// superseded through its active key.
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
        let authorized = self.authorized.clone();
        if !self.pending.as_ref().is_some_and(|card| {
            valid(card)
                && authorized.as_ref().is_none_or(|successor| {
                    successor.transport() != card.transport()
                        && successor.epoch().is_superseded_by(card.epoch())
                })
        }) {
            self.pending = None;
        }
        self.pending_imported &= self.pending.is_some();
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
            assert!(a.epoch() < p.epoch());
        }
        assert_ne!(Some(active.transport()), credentials.retired());
        assert!(!credentials.pending_was_imported() || credentials.pending_successor().is_some());
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
    fn the_active_key_is_admitted_and_older_cards_of_other_keys_are_not() {
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
            Ok(CredentialChange::Superseded)
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
    fn an_older_card_with_the_active_key_is_the_contact() {
        // The holder of the active key shows an older card: the contact,
        // with a card that is not taken.
        let mut credentials = Credentials::new(signed(ALICE, T1, 3, 2, None));
        let before = credentials.clone();
        assert_eq!(
            credentials.admit(&alice(T1, 1)),
            Ok(CredentialChange::Superseded)
        );
        assert_eq!(credentials, before);
        assert!(credentials.authorizes(&alice(T1, 1)));
        // An older card of another key is still stale. An older card of
        // the active key is the same card from every source, superseded,
        // and is not taken as an announcement or an import either.
        assert_eq!(
            credentials.admit(&alice(T2, 2)),
            Ok(CredentialChange::Stale)
        );
        assert_eq!(
            credentials.announce(&alice(T1, 2), &alice(T1, 3)),
            Ok(CredentialChange::Superseded)
        );
        assert_eq!(
            credentials.import(alice(T1, 2)),
            Ok(CredentialChange::Superseded)
        );
        assert_eq!(
            credentials.replace(alice(T1, 2)),
            Ok(CredentialChange::Superseded)
        );
        assert_eq!(credentials, before);
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
        // The previous statement is older now; its key is still active.
        assert_eq!(
            credentials.admit(&alice(T1, 1)),
            Ok(CredentialChange::Superseded)
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
    fn a_proven_key_older_than_the_announced_successor_is_stale() {
        // T1 is active, T2 of epoch 2 was announced and then replaced by T3
        // of epoch 3. A peer that proves T2 in a handshake holds a key the
        // identity has superseded: it is not held as pending, and nothing
        // changes. A key newer than T3 is pending.
        let mut credentials = Credentials::new(alice(T1, 1));
        let session = alice(T1, 1);
        credentials.announce(&alice(T2, 2), &session).unwrap();
        credentials.announce(&alice(T3, 3), &session).unwrap();
        let before = credentials.clone();
        assert_eq!(
            credentials.admit(&alice(T2, 2)),
            Ok(CredentialChange::Stale)
        );
        assert_eq!(credentials, before);
        assert_eq!(credentials.pending_successor(), None);
        assert_eq!(
            credentials.admit(&alice(4, 4)),
            Ok(CredentialChange::Pending)
        );
        assert_eq!(credentials.pending_successor(), Some(&alice(4, 4)));
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
        // Through the active key the same card is authorized. The pending
        // card of epoch 2 is older than the successor the identity has now
        // announced through its active key, so it is dropped: a key the
        // identity superseded is stale from every source.
        assert_eq!(
            credentials.announce(&alice(T3, 3), &alice(T1, 1)),
            Ok(CredentialChange::Authorized)
        );
        assert!(credentials.pending_successor().is_none());
        assert_eq!(
            credentials.admit(&alice(T2, 2)),
            Ok(CredentialChange::Stale)
        );
        // A pending card newer than the successor stays a separate
        // candidate.
        assert_eq!(
            credentials.admit(&alice(T2, 3)),
            Ok(CredentialChange::Conflict)
        );
        assert_eq!(
            credentials.admit(&alice(4, 4)),
            Ok(CredentialChange::Pending)
        );
        assert_eq!(credentials.pending_successor(), Some(&alice(4, 4)));
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
            Ok(CredentialChange::Superseded)
        );
        check(&credentials);
    }

    #[test]
    fn a_pending_card_of_a_peer_does_not_keep_out_the_users_own() {
        // Whoever copied the identity key presents a card with a huge
        // epoch. The user later imports the card Alice handed over out of
        // band, with a lower epoch, and confirms it.
        let mut credentials = Credentials::new(alice(T1, 1));
        let planted = alice(T3, u64::MAX);
        assert_eq!(credentials.admit(&planted), Ok(CredentialChange::Pending));
        assert_eq!(
            credentials.import(alice(T2, 2)),
            Ok(CredentialChange::Pending)
        );
        assert_eq!(credentials.pending_successor(), Some(&alice(T2, 2)));
        assert_eq!(
            credentials.confirm(&planted),
            Err(ProtocolError::InvalidValue)
        );
        assert_eq!(
            credentials.confirm(&alice(T2, 2)),
            Ok(CredentialChange::Promoted)
        );
        assert_eq!(credentials.active(), &alice(T2, 2));
        check(&credentials);
    }

    #[test]
    fn a_presented_card_does_not_displace_the_users_pending_card() {
        let mut credentials = Credentials::new(alice(T1, 1));
        assert_eq!(
            credentials.import(alice(T2, 2)),
            Ok(CredentialChange::Pending)
        );
        assert!(credentials.pending_was_imported());
        // A card with a larger epoch, presented in a handshake.
        assert_eq!(
            credentials.admit(&alice(T3, u64::MAX)),
            Ok(CredentialChange::Pending)
        );
        assert_eq!(credentials.pending_successor(), Some(&alice(T2, 2)));
        assert_eq!(
            credentials.confirm(&alice(T2, 2)),
            Ok(CredentialChange::Promoted)
        );
        assert!(!credentials.pending_was_imported());
        check(&credentials);

        // A presented card replaces a presented one when it is newer.
        let mut credentials = Credentials::new(alice(T1, 1));
        credentials.admit(&alice(T2, 2)).unwrap();
        assert!(!credentials.pending_was_imported());
        credentials.admit(&alice(T3, 3)).unwrap();
        assert_eq!(credentials.pending_successor(), Some(&alice(T3, 3)));
    }

    #[test]
    fn a_card_at_the_epoch_of_the_successor_that_says_otherwise_is_a_conflict() {
        // Alice announced T2 at epoch 2. A card of epoch 2 that keeps T1,
        // or names T3, is a second statement for that epoch.
        let mut credentials = Credentials::new(alice(T1, 1));
        credentials.announce(&alice(T2, 2), &alice(T1, 1)).unwrap();
        let before = credentials.clone();
        let same_epoch_active_key = signed(ALICE, T1, 2, 2, None);
        assert_eq!(
            credentials.admit(&same_epoch_active_key),
            Ok(CredentialChange::Conflict)
        );
        assert_eq!(
            credentials.announce(&same_epoch_active_key, &alice(T1, 1)),
            Ok(CredentialChange::Conflict)
        );
        assert_eq!(
            credentials.import(same_epoch_active_key),
            Ok(CredentialChange::Conflict)
        );
        assert_eq!(
            credentials.admit(&alice(T3, 2)),
            Ok(CredentialChange::Conflict)
        );
        assert_eq!(credentials, before);
        // A later card with the active key still moves on, and drops the
        // successor it supersedes.
        assert_eq!(
            credentials.admit(&signed(ALICE, T1, 3, 2, None)),
            Ok(CredentialChange::Advanced)
        );
        assert!(credentials.authorized_successor().is_none());
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
            Ok(CredentialChange::Superseded)
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
                        | CredentialChange::Superseded
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

    /// The exhaustive table of [`Credentials::evaluate`], against a model
    /// written from `docs/PROTOCOL.md` section 11.4 with plain numbers.
    mod table {
        use super::*;
        use std::collections::HashMap;

        /// A card of the model: transport key, epoch, endpoint.
        type M = (u8, u64, u8);

        /// The credentials of the model.
        #[derive(Clone, Debug, PartialEq, Eq)]
        struct Model {
            active: M,
            authorized: Option<M>,
            pending: Option<M>,
            imported: bool,
            retired: Option<u8>,
        }

        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        enum Src {
            Proven,
            AnnouncedActive,
            AnnouncedOther,
            ImportedRequested,
            ImportedAccepted,
            Confirmed,
        }

        const SOURCES: [Src; 6] = [
            Src::Proven,
            Src::AnnouncedActive,
            Src::AnnouncedOther,
            Src::ImportedRequested,
            Src::ImportedAccepted,
            Src::Confirmed,
        ];

        impl Model {
            /// Rule 1 to 5 of section 11.4, as a receiver applies them
            /// to a card of the identity.
            fn relation(&self, c: M) -> CardRelation {
                let (key, epoch, _) = c;
                if self.retired == Some(key) {
                    return CardRelation::Stale;
                }
                let (akey, aepoch, _) = self.active;
                if epoch < aepoch {
                    return if key == akey {
                        CardRelation::OlderActiveKey
                    } else {
                        CardRelation::Stale
                    };
                }
                if epoch == aepoch {
                    return if c == self.active {
                        CardRelation::Same
                    } else {
                        CardRelation::Conflict
                    };
                }
                if let Some(a) = self.authorized {
                    if epoch == a.1 && c != a {
                        return CardRelation::Conflict;
                    }
                }
                if key == akey {
                    return CardRelation::NewerActiveKey;
                }
                match self.authorized {
                    Some(a) if key == a.0 => {
                        if epoch < a.1 {
                            CardRelation::Stale
                        } else {
                            CardRelation::Successor { same: c == a }
                        }
                    }
                    Some(a) if epoch <= a.1 => CardRelation::Stale,
                    _ => CardRelation::NewKey,
                }
            }

            fn valid_successor(&self, c: M) -> bool {
                c.1 > self.active.1 && c.0 != self.active.0 && Some(c.0) != self.retired
            }

            fn prune(&mut self) {
                if !self.authorized.is_some_and(|a| self.valid_successor(a)) {
                    self.authorized = None;
                }
                let authorized = self.authorized;
                if !self.pending.is_some_and(|p| {
                    self.valid_successor(p) && authorized.is_none_or(|a| a.0 != p.0 && a.1 < p.1)
                }) {
                    self.pending = None;
                }
                self.imported &= self.pending.is_some();
            }

            fn promote(&mut self, c: M) {
                self.retired = Some(self.active.0);
                self.active = c;
                self.authorized = None;
                self.prune();
            }

            /// What the event does, and the model after it. `None` for an
            /// event that fails.
            fn step(&self, c: M, src: Src) -> Option<(CredentialChange, Model)> {
                let mut next = self.clone();
                if src == Src::AnnouncedOther {
                    return Some((CredentialChange::NoContinuity, next));
                }
                if src == Src::Confirmed {
                    if self.pending != Some(c) {
                        return None;
                    }
                    next.promote(c);
                    return Some((CredentialChange::Promoted, next));
                }
                let change = match self.relation(c) {
                    CardRelation::Same => CredentialChange::Unchanged,
                    CardRelation::OlderActiveKey => CredentialChange::Superseded,
                    CardRelation::Conflict => CredentialChange::Conflict,
                    CardRelation::Stale => CredentialChange::Stale,
                    CardRelation::NewerActiveKey => {
                        next.active = c;
                        next.prune();
                        CredentialChange::Advanced
                    }
                    CardRelation::Successor { same } => match src {
                        Src::Proven | Src::ImportedRequested => {
                            next.promote(c);
                            CredentialChange::Promoted
                        }
                        Src::AnnouncedActive if !same => {
                            next.authorized = Some(c);
                            next.prune();
                            CredentialChange::Authorized
                        }
                        _ => CredentialChange::Unchanged,
                    },
                    CardRelation::NewKey => match src {
                        Src::Proven => {
                            let keep = self.pending.is_some_and(|p| self.imported || p.1 >= c.1);
                            if !keep {
                                next.pending = Some(c);
                                next.imported = false;
                            }
                            next.prune();
                            CredentialChange::Pending
                        }
                        Src::AnnouncedActive => {
                            next.authorized = Some(c);
                            next.prune();
                            CredentialChange::Authorized
                        }
                        Src::ImportedAccepted => {
                            next.pending = Some(c);
                            next.imported = true;
                            next.prune();
                            CredentialChange::Pending
                        }
                        _ => {
                            next.promote(c);
                            CredentialChange::Promoted
                        }
                    },
                };
                Some((change, next))
            }
        }

        /// Every card of the universe, signed once.
        struct Cards(HashMap<(M, Option<u8>), ContactCard>);

        impl Cards {
            fn get(&self, c: M, capability: Option<u8>) -> ContactCard {
                self.0[&(c, capability)].clone()
            }
        }

        const KEYS: [u8; 6] = [1, 2, 3, 4, 5, 6];

        fn universe() -> Cards {
            let mut cards = HashMap::new();
            for key in KEYS {
                for epoch in 1..=7 {
                    for place in 1..=2 {
                        for capability in [None, Some(9)] {
                            cards.insert(
                                ((key, epoch, place), capability),
                                signed(ALICE, key, epoch, place, capability),
                            );
                        }
                    }
                }
            }
            Cards(cards)
        }

        fn credentials_of(cards: &Cards, model: &Model) -> Result<Credentials, ProtocolError> {
            Credentials::restore(
                cards.get(model.active, None),
                model.authorized.map(|c| cards.get(c, None)),
                model.pending.map(|c| cards.get(c, None)),
                model.imported,
                model.retired.map(transport),
                None,
            )
        }

        /// The key and the endpoint of the model for each key the cards of
        /// the universe state. Computed once: deriving keys is slow in
        /// unoptimized builds.
        struct Names {
            keys: HashMap<TransportPublicKey, u8>,
            places: Vec<(EndpointSet, u8)>,
        }

        impl Names {
            fn new() -> Self {
                Self {
                    keys: KEYS.into_iter().map(|key| (transport(key), key)).collect(),
                    places: (1..=2)
                        .map(|place| (EndpointSet::single(endpoint(place)), place))
                        .collect(),
                }
            }

            fn card(&self, card: &ContactCard) -> M {
                let place = self
                    .places
                    .iter()
                    .find(|(set, _)| set == card.endpoints())
                    .unwrap()
                    .1;
                (self.keys[card.transport()], card.epoch().get(), place)
            }

            fn model(&self, credentials: &Credentials) -> Model {
                Model {
                    active: self.card(credentials.active()),
                    authorized: credentials.authorized_successor().map(|c| self.card(c)),
                    pending: credentials.pending_successor().map(|c| self.card(c)),
                    imported: credentials.pending_was_imported(),
                    retired: credentials.retired().map(|retired| self.keys[retired]),
                }
            }
        }

        /// Every state the model allows with the active card (1, 3, 1).
        fn states() -> Vec<Model> {
            let mut successors = vec![None];
            for key in [2, 3, 4] {
                for epoch in [4, 5, 6] {
                    for place in [1, 2] {
                        successors.push(Some((key, epoch, place)));
                    }
                }
            }
            let mut states = Vec::new();
            for authorized in &successors {
                for pending in &successors {
                    for imported in [false, true] {
                        for retired in [None, Some(4), Some(5)] {
                            let model = Model {
                                active: (1, 3, 1),
                                authorized: *authorized,
                                pending: *pending,
                                imported,
                                retired,
                            };
                            let mut pruned = model.clone();
                            pruned.prune();
                            if pruned == model {
                                states.push(model);
                            }
                        }
                    }
                }
            }
            states
        }

        #[test]
        fn restore_accepts_exactly_the_states_of_the_invariants() {
            let cards = universe();
            let names = Names::new();
            let states = states();
            assert!(states.len() > 300, "{}", states.len());
            for model in &states {
                let credentials = credentials_of(&cards, model).unwrap();
                assert_eq!(&names.model(&credentials), model);
                check(&credentials);
            }
            // States that break an invariant are refused.
            let broken = [
                Model {
                    active: (1, 3, 1),
                    authorized: Some((2, 3, 1)),
                    pending: None,
                    imported: false,
                    retired: None,
                },
                Model {
                    active: (1, 3, 1),
                    authorized: Some((1, 4, 1)),
                    pending: None,
                    imported: false,
                    retired: None,
                },
                Model {
                    active: (1, 3, 1),
                    authorized: Some((2, 5, 1)),
                    pending: Some((3, 5, 1)),
                    imported: false,
                    retired: None,
                },
                Model {
                    active: (1, 3, 1),
                    authorized: Some((2, 5, 1)),
                    pending: Some((2, 6, 1)),
                    imported: false,
                    retired: None,
                },
                Model {
                    active: (1, 3, 1),
                    authorized: None,
                    pending: Some((4, 5, 1)),
                    imported: false,
                    retired: Some(4),
                },
                Model {
                    active: (1, 3, 1),
                    authorized: None,
                    pending: None,
                    imported: true,
                    retired: None,
                },
                Model {
                    active: (1, 3, 1),
                    authorized: None,
                    pending: None,
                    imported: false,
                    retired: Some(1),
                },
            ];
            for model in broken {
                assert_eq!(
                    credentials_of(&cards, &model),
                    Err(ProtocolError::InvalidValue),
                    "{model:?}"
                );
            }
            assert_eq!(
                Credentials::restore(
                    alice(1, 3),
                    Some(signed(MALLORY, 2, 4, 1, None)),
                    None,
                    false,
                    None,
                    None
                ),
                Err(ProtocolError::IdentityMismatch)
            );
        }

        #[test]
        fn every_card_from_every_source_in_every_state() {
            let cards = universe();
            let names = Names::new();
            let session_active = cards.get((1, 3, 1), None);
            let session_other = cards.get((6, 3, 1), None);
            let mut seen = HashMap::new();
            for model in states() {
                let credentials = credentials_of(&cards, &model).unwrap();
                for (&(c, capability), card) in &cards.0 {
                    for src in SOURCES {
                        let source = match src {
                            Src::Proven => Source::Proven,
                            Src::AnnouncedActive => Source::Announced(&session_active),
                            Src::AnnouncedOther => Source::Announced(&session_other),
                            Src::ImportedRequested => Source::Imported(Holding::Requested),
                            Src::ImportedAccepted => Source::Imported(Holding::Accepted),
                            Src::Confirmed => Source::Confirmed,
                        };
                        let evaluation = credentials.evaluate(card, source);
                        let expected = model.step(c, src);
                        let Some((change, next_model)) = expected else {
                            assert_eq!(evaluation, Err(ProtocolError::InvalidValue));
                            continue;
                        };
                        let evaluation = evaluation.unwrap();
                        assert_eq!(evaluation.change, change, "{model:?} {c:?} {src:?}");
                        if !matches!(src, Src::AnnouncedOther | Src::Confirmed) {
                            assert_eq!(evaluation.relation, Some(model.relation(c)));
                        }
                        let next = evaluation
                            .next()
                            .cloned()
                            .unwrap_or_else(|| credentials.clone());
                        assert_eq!(names.model(&next), next_model, "{model:?} {c:?} {src:?}");
                        check(&next);
                        // The capability a request carries changes only by
                        // an import for a requested contact that takes the
                        // card's statement.
                        let takes_capability = src == Src::ImportedRequested
                            && matches!(
                                change,
                                CredentialChange::Unchanged
                                    | CredentialChange::Advanced
                                    | CredentialChange::Promoted
                            );
                        let expected_invitation = if takes_capability {
                            capability.map(|byte| InvitationCapability::from_bytes([byte; 16]))
                        } else {
                            None
                        };
                        assert_eq!(next.invitation(), expected_invitation.as_ref());
                        // No rollback, from any source: the active epoch
                        // never goes down, the retired key never comes
                        // back, and the active key changes only by a
                        // promotion that retires the previous one.
                        assert!(next.active().epoch() >= credentials.active().epoch());
                        if let Some(retired) = credentials.retired() {
                            assert_ne!(next.active().transport(), retired);
                        }
                        let key_changed =
                            next.active().transport() != credentials.active().transport();
                        assert_eq!(key_changed, change == CredentialChange::Promoted);
                        assert_eq!(evaluation.withdraws_sessions(), key_changed);
                        if key_changed {
                            assert_eq!(next.retired(), Some(credentials.active().transport()));
                            assert_eq!(
                                evaluation.retires(),
                                Some(credentials.active().transport())
                            );
                            // Only a proven successor, a confirmed pending
                            // card or the user's own card for a requested
                            // contact takes over.
                            let allowed = match src {
                                Src::Proven => credentials
                                    .authorized_successor()
                                    .is_some_and(|a| a.transport() == next.active().transport()),
                                Src::Confirmed => credentials
                                    .pending_successor()
                                    .is_some_and(|p| p.transport() == next.active().transport()),
                                Src::ImportedRequested => true,
                                _ => false,
                            };
                            assert!(allowed, "{model:?} {c:?} {src:?}");
                        }
                        // A card never promotes itself because it is newer.
                        if src == Src::Proven && model.relation(c) == CardRelation::NewKey {
                            assert!(!key_changed);
                        }
                        *seen.entry((src, change)).or_insert(0_u32) += 1;
                        // Evaluating changes nothing; applying makes the
                        // evaluated change.
                        let mut applied = credentials.clone();
                        assert_eq!(applied.apply(card, source), Ok(change));
                        assert_eq!(applied, next);
                    }
                }
            }
            // Every outcome of the table occurred.
            for (src, change) in [
                (Src::Proven, CredentialChange::Unchanged),
                (Src::Proven, CredentialChange::Superseded),
                (Src::Proven, CredentialChange::Advanced),
                (Src::Proven, CredentialChange::Promoted),
                (Src::Proven, CredentialChange::Pending),
                (Src::Proven, CredentialChange::Conflict),
                (Src::Proven, CredentialChange::Stale),
                (Src::AnnouncedActive, CredentialChange::Authorized),
                (Src::AnnouncedActive, CredentialChange::Unchanged),
                (Src::AnnouncedActive, CredentialChange::Superseded),
                (Src::AnnouncedOther, CredentialChange::NoContinuity),
                (Src::ImportedAccepted, CredentialChange::Pending),
                (Src::ImportedAccepted, CredentialChange::Unchanged),
                (Src::ImportedRequested, CredentialChange::Promoted),
                (Src::Confirmed, CredentialChange::Promoted),
            ] {
                assert!(seen.contains_key(&(src, change)), "{src:?} {change:?}");
            }
        }

        #[test]
        fn a_card_of_another_identity_is_refused_from_every_source() {
            let credentials = Credentials::new(alice(1, 3));
            let session = alice(1, 3);
            let foreign = signed(MALLORY, 1, 4, 1, None);
            for source in [
                Source::Proven,
                Source::Announced(&session),
                Source::Announced(&foreign),
                Source::Imported(Holding::Requested),
                Source::Imported(Holding::Accepted),
                Source::Confirmed,
            ] {
                assert_eq!(
                    credentials.evaluate(&foreign, source),
                    Err(ProtocolError::IdentityMismatch)
                );
            }
            // A session of another identity carries no announcement.
            assert_eq!(
                credentials.evaluate(&alice(2, 4), Source::Announced(&foreign)),
                Err(ProtocolError::IdentityMismatch)
            );
        }

        #[test]
        fn the_same_card_has_the_same_relation_whatever_its_source() {
            // The residual of Phase 3: a card of another key that is newer
            // than the active card and older than the announced successor
            // was pending when imported and stale when proven. It is stale
            // now from every source, and nothing changes.
            let mut credentials = Credentials::new(alice(1, 3));
            credentials.announce(&alice(2, 6), &alice(1, 3)).unwrap();
            let card = alice(3, 5);
            for source in [
                Source::Proven,
                Source::Announced(&alice(1, 3)),
                Source::Imported(Holding::Requested),
                Source::Imported(Holding::Accepted),
            ] {
                let evaluation = credentials.evaluate(&card, source).unwrap();
                assert_eq!(evaluation.relation, Some(CardRelation::Stale));
                assert_eq!(evaluation.change, CredentialChange::Stale);
                assert!(!evaluation.changes_state());
            }
        }
    }
}
