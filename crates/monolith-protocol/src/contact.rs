//! The one decision about a card of a known identity.
//!
//! A card reaches the local side in five ways: a peer presents it in an
//! inbound handshake, an outbound handshake proves the key of the card
//! that was dialed, a contact announces it in an EndpointUpdate, the user
//! imports it, or the user confirms it as the pending successor. Each of
//! them asks the same question: given what the local side holds about the
//! identity, and this card, what does the card mean and what may change?
//! [`decide`] answers it, for every path, from the kind of record
//! ([`RecordKind`]) and the credentials of a contact
//! ([`Credentials::evaluate`], which classifies the card the same way
//! whatever its source). Nothing else interprets a card of a known
//! identity: `PeerRecord::admit` in the session logic and the contact
//! store of the core both call it.
//!
//! The decision is a value. It says what the card is, the standing a
//! session gets, and the credentials after the event; it changes nothing
//! by itself. The caller applies it under the lock of the contact it was
//! made for, in the same step, so that what was decided is what is
//! recorded.

use crate::ProtocolError;
use crate::card::ContactCard;
use crate::credential::{CardRelation, CredentialChange, Credentials, Evaluation, Holding, Source};
use crate::session::Standing;
use monolith_identity::TransportPublicKey;

/// What the local side holds about an identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RecordKind {
    /// No record: never seen, or a contact the user deleted.
    None,
    /// The user declined a request from this identity.
    Declined,
    /// The user blocked this identity.
    Blocked,
    /// The user imported a card of this identity and has not seen an
    /// acceptance.
    Requested,
    /// An accepted contact.
    Accepted,
}

impl RecordKind {
    /// Returns true for the kinds that hold credentials: requested and
    /// accepted contacts.
    pub const fn is_contact(self) -> bool {
        matches!(self, Self::Requested | Self::Accepted)
    }

    /// The standing of a session with a peer of this kind whose card
    /// stands for it.
    const fn as_standing(self) -> Standing {
        match self {
            Self::None => Standing::None,
            Self::Declined => Standing::Declined,
            Self::Blocked => Standing::Blocked,
            Self::Requested => Standing::Requested,
            Self::Accepted => Standing::Accepted,
        }
    }
}

/// Which contact requests from peers that are not contacts are considered
/// (`docs/PROTOCOL.md` section 12). The mode belongs to a local identity,
/// not to a card, because a request does not say which card it came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RequestMode {
    /// A request is considered only with a capability of the active set.
    /// The default.
    #[default]
    Invitation,
    /// A request is considered with or without a capability; one that
    /// carries a capability outside the active set is dropped.
    Open,
    /// No request is considered.
    Closed,
}

/// Why a card is looked at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context<'a> {
    /// An inbound handshake: the peer presented the card in message 3.
    Inbound,
    /// An outbound handshake: message 2 proved the key of the card that
    /// was dialed. Message 3 follows only if the decision allows it.
    Outbound,
    /// An EndpointUpdate carried the card on a session whose peer is the
    /// given card.
    Announcement(&'a ContactCard),
    /// The user imported the card by hand.
    Import,
    /// The user confirmed the card that was shown as the pending
    /// successor.
    Confirmation,
}

/// What a card means for the record of its identity, and what may change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    /// The kind of record before the event.
    pub before: RecordKind,
    /// The kind of record after the event. An import of a card of an
    /// identity without a record, or a declined one, makes it a requested
    /// contact.
    pub after: RecordKind,
    /// For a contact: what the card is and what it does to the
    /// credentials.
    pub evaluation: Option<Evaluation>,
    /// For a handshake: the standing of the session.
    pub standing: Option<Standing>,
    /// The credentials of a record the event creates.
    created: Option<Credentials>,
}

impl Decision {
    /// Returns what the card is, compared with the credentials of a
    /// contact.
    pub fn relation(&self) -> Option<CardRelation> {
        self.evaluation
            .as_ref()
            .and_then(|evaluation| evaluation.relation)
    }

    /// Returns what the card does to the credentials of a contact.
    pub fn change(&self) -> Option<CredentialChange> {
        self.evaluation.as_ref().map(|evaluation| evaluation.change)
    }

    /// Returns the credentials after the event, if the event changes or
    /// creates them.
    pub fn next_credentials(&self) -> Option<&Credentials> {
        self.created
            .as_ref()
            .or_else(|| self.evaluation.as_ref().and_then(Evaluation::next))
    }

    /// Returns true if the durable state of the identity changes: its
    /// credentials, or the kind of its record.
    pub fn changes_state(&self) -> bool {
        self.before != self.after || self.next_credentials().is_some()
    }

    /// Returns true if a peer with the standing of this decision may learn
    /// the local identity: its proven key stands for an identity held as a
    /// requested or accepted contact. Message 3 is written only then.
    pub fn may_learn_local_identity(&self) -> bool {
        self.standing
            .is_some_and(|standing| standing.may_learn_local_identity())
    }

    /// Returns true if the active card changes: a newer card of the active
    /// key or a promoted successor. Where Monolith dials still follows the
    /// card the user confirmed for dialing.
    pub fn updates_active_card(&self) -> bool {
        self.evaluation
            .as_ref()
            .is_some_and(Evaluation::updates_active_card)
    }

    /// Returns true if a card is held for the user to confirm.
    pub fn needs_confirmation(&self) -> bool {
        self.evaluation
            .as_ref()
            .is_some_and(Evaluation::needs_confirmation)
    }

    /// Returns the transport key the event retires, if any.
    pub fn retires(&self) -> Option<&TransportPublicKey> {
        self.evaluation.as_ref().and_then(Evaluation::retires)
    }

    /// Returns true if sessions of the identity may lose their standing:
    /// a key was retired. The caller withdraws every session admitted as a
    /// contact's whose card no longer stands for the contact.
    pub fn withdraws_sessions(&self) -> bool {
        self.retires().is_some()
    }

    /// Returns true if the identity signed two statements for one epoch.
    /// It is reported to the user and never to the peer.
    pub fn is_conflict(&self) -> bool {
        self.evaluation
            .as_ref()
            .is_some_and(Evaluation::is_conflict)
    }

    fn unchanged(kind: RecordKind, standing: Option<Standing>) -> Self {
        Self {
            before: kind,
            after: kind,
            evaluation: None,
            standing,
            created: None,
        }
    }
}

/// Why an import was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Refusal {
    /// The identity is blocked. The user unblocks it first; an import does
    /// not do that on the side.
    Blocked,
}

/// Decides what `card` means for an identity whose record is of `kind`,
/// with `credentials` for a requested or accepted contact, when the card
/// arrives through `context`.
///
/// - Handshakes ([`Context::Inbound`], [`Context::Outbound`]): for an
///   identity that is not a contact the standing is that of its record
///   and nothing changes. For a contact the card is evaluated as proven
///   ([`Source::Proven`]) and the standing follows from the outcome: the
///   standing of the record for the active key, an older card of it, a
///   newer card of it and a promoted successor;
///   [`Standing::PendingSuccessor`] for a newer key without continuity;
///   [`Standing::StaleCard`] for a conflict and a stale card. This is the
///   table of `docs/PROTOCOL.md` section 6.2.
/// - An announcement is evaluated for a contact only
///   ([`Source::Announced`]). Without a contact record nothing changes.
/// - An import ([`Source::Imported`]) for a requested contact is the
///   user's choice of the card, and for an accepted one never a key
///   change. For an identity without a record or a declined one it makes
///   a requested contact with the card. For a blocked identity it fails
///   with [`Refusal::Blocked`].
/// - A confirmation ([`Source::Confirmed`]) promotes the pending
///   successor of a contact. Without a contact record it fails with
///   [`ProtocolError::InvalidValue`].
///
/// Fails with [`ProtocolError::IdentityMismatch`] if the card, or the
/// card of the session of an announcement, is of another identity than
/// the credentials, and with [`ProtocolError::InvalidValue`] if `kind`
/// and `credentials` do not belong together.
pub fn decide(
    kind: RecordKind,
    credentials: Option<&Credentials>,
    card: &ContactCard,
    context: Context<'_>,
) -> Result<Result<Decision, Refusal>, ProtocolError> {
    let held = match (kind.is_contact(), credentials) {
        (true, Some(held)) => Some(held),
        (false, None) => None,
        _ => return Err(ProtocolError::InvalidValue),
    };
    let Some(held) = held else {
        return Ok(match context {
            Context::Inbound | Context::Outbound => {
                Ok(Decision::unchanged(kind, Some(kind.as_standing())))
            }
            Context::Announcement(_) => Ok(Decision::unchanged(kind, None)),
            Context::Import if kind == RecordKind::Blocked => Err(Refusal::Blocked),
            Context::Import => Ok(Decision {
                before: kind,
                after: RecordKind::Requested,
                evaluation: None,
                standing: None,
                created: Some(Credentials::new(card.clone())),
            }),
            Context::Confirmation => return Err(ProtocolError::InvalidValue),
        });
    };
    let holding = if kind == RecordKind::Accepted {
        Holding::Accepted
    } else {
        Holding::Requested
    };
    let source = match context {
        Context::Inbound | Context::Outbound => Source::Proven,
        Context::Announcement(session) => Source::Announced(session),
        Context::Import => Source::Imported(holding),
        Context::Confirmation => Source::Confirmed,
    };
    let evaluation = held.evaluate(card, source)?;
    let standing = match context {
        Context::Inbound | Context::Outbound => Some(match evaluation.change {
            CredentialChange::Unchanged
            | CredentialChange::Superseded
            | CredentialChange::Advanced
            | CredentialChange::Promoted => kind.as_standing(),
            CredentialChange::Pending => Standing::PendingSuccessor,
            CredentialChange::Conflict
            | CredentialChange::Stale
            | CredentialChange::Authorized
            | CredentialChange::NoContinuity => Standing::StaleCard,
        }),
        Context::Announcement(_) | Context::Import | Context::Confirmation => None,
    };
    Ok(Ok(Decision {
        before: kind,
        after: kind,
        evaluation: Some(evaluation),
        standing,
        created: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{EndpointSet, InvitationCapability};
    use monolith_identity::{EndpointEpoch, IdentitySecretKey, OnionServiceKey};

    const ALICE: u8 = 0x10;
    const MALLORY: u8 = 0x77;

    fn transport(seed: u8) -> TransportPublicKey {
        let mut bytes = [seed; 32];
        bytes[31] = 0x40;
        TransportPublicKey::from_bytes(&bytes).unwrap()
    }

    fn card(identity: u8, key: u8, epoch: u64, capability: Option<u8>) -> ContactCard {
        let endpoint = IdentitySecretKey::from_seed(&[0x99; 32]).public_key();
        ContactCard::sign(
            &IdentitySecretKey::from_seed(&[identity; 32]),
            transport(key),
            EndpointEpoch::new(epoch).unwrap(),
            EndpointSet::single(OnionServiceKey::from_bytes(endpoint.as_bytes()).unwrap()),
            capability.map(|byte| InvitationCapability::from_bytes([byte; 16])),
        )
        .unwrap()
    }

    const KINDS: [RecordKind; 5] = [
        RecordKind::None,
        RecordKind::Declined,
        RecordKind::Blocked,
        RecordKind::Requested,
        RecordKind::Accepted,
    ];

    /// Alice's credentials: key 1 active at epoch 3, key 2 announced at
    /// epoch 5, key 3 pending at epoch 6, key 4 retired.
    fn held() -> Credentials {
        Credentials::restore(
            card(ALICE, 1, 3, None),
            Some(card(ALICE, 2, 5, None)),
            Some(card(ALICE, 3, 6, None)),
            false,
            Some(transport(4)),
            Some(InvitationCapability::from_bytes([7; 16])),
        )
        .unwrap()
    }

    /// One card of each relation to `held`.
    fn candidates() -> Vec<(ContactCard, CardRelation)> {
        vec![
            (card(ALICE, 1, 3, None), CardRelation::Same),
            (card(ALICE, 1, 2, None), CardRelation::OlderActiveKey),
            (card(ALICE, 1, 4, None), CardRelation::NewerActiveKey),
            (
                card(ALICE, 2, 5, None),
                CardRelation::Successor { same: true },
            ),
            (
                card(ALICE, 2, 7, None),
                CardRelation::Successor { same: false },
            ),
            (card(ALICE, 5, 8, None), CardRelation::NewKey),
            (card(ALICE, 5, 3, None), CardRelation::Conflict),
            (card(ALICE, 5, 5, None), CardRelation::Conflict),
            (card(ALICE, 5, 4, None), CardRelation::Stale),
            (card(ALICE, 2, 4, None), CardRelation::Stale),
            (card(ALICE, 4, 9, None), CardRelation::Stale),
            (card(ALICE, 5, 1, None), CardRelation::Stale),
        ]
    }

    fn contexts(session: &ContactCard) -> [Context<'_>; 5] {
        [
            Context::Inbound,
            Context::Outbound,
            Context::Announcement(session),
            Context::Import,
            Context::Confirmation,
        ]
    }

    #[test]
    fn every_kind_of_record_in_every_context() {
        let session = card(ALICE, 1, 3, None);
        let held = held();
        for kind in KINDS {
            let credentials = kind.is_contact().then(|| held.clone());
            for (candidate, relation) in candidates() {
                for context in contexts(&session) {
                    let result = decide(kind, credentials.as_ref(), &candidate, context);
                    if !kind.is_contact() {
                        match context {
                            Context::Inbound | Context::Outbound => {
                                // Not a contact: the standing of the record,
                                // nothing is recorded, no message 3.
                                let decision = result.unwrap().unwrap();
                                assert_eq!(decision.standing, Some(kind.as_standing()));
                                assert!(!decision.changes_state());
                                assert!(!decision.may_learn_local_identity());
                                assert!(decision.evaluation.is_none());
                            }
                            Context::Announcement(_) => {
                                let decision = result.unwrap().unwrap();
                                assert!(!decision.changes_state());
                                assert_eq!(decision.standing, None);
                            }
                            Context::Import if kind == RecordKind::Blocked => {
                                assert_eq!(result, Ok(Err(Refusal::Blocked)));
                            }
                            Context::Import => {
                                // A new requested contact with this card.
                                let decision = result.unwrap().unwrap();
                                assert_eq!(decision.after, RecordKind::Requested);
                                assert_eq!(
                                    decision.next_credentials(),
                                    Some(&Credentials::new(candidate.clone()))
                                );
                            }
                            Context::Confirmation => {
                                assert_eq!(result, Err(ProtocolError::InvalidValue));
                            }
                        }
                        continue;
                    }
                    if context == Context::Confirmation {
                        // Only the pending card is confirmed.
                        assert_eq!(result, Err(ProtocolError::InvalidValue));
                        continue;
                    }
                    let decision = result.unwrap().unwrap();
                    assert_eq!(decision.after, kind);
                    // The same card has the same relation in every context.
                    assert_eq!(decision.relation(), Some(relation), "{kind:?} {context:?}");
                    let change = decision.change().unwrap();
                    match context {
                        Context::Inbound | Context::Outbound => {
                            let standing = decision.standing.unwrap();
                            let expected = match relation {
                                CardRelation::Same
                                | CardRelation::OlderActiveKey
                                | CardRelation::NewerActiveKey
                                | CardRelation::Successor { .. } => kind.as_standing(),
                                CardRelation::NewKey => Standing::PendingSuccessor,
                                CardRelation::Conflict | CardRelation::Stale => Standing::StaleCard,
                            };
                            assert_eq!(standing, expected, "{relation:?}");
                            assert_eq!(
                                decision.may_learn_local_identity(),
                                expected.is_contact_record()
                            );
                            // A newer key never takes over by being newer.
                            if relation == CardRelation::NewKey {
                                assert_eq!(change, CredentialChange::Pending);
                                assert!(decision.retires().is_none());
                                assert!(decision.needs_confirmation());
                            }
                            if let CardRelation::Successor { .. } = relation {
                                assert_eq!(change, CredentialChange::Promoted);
                                assert_eq!(decision.retires(), Some(&transport(1)));
                                assert!(decision.withdraws_sessions());
                            }
                            if relation == CardRelation::Conflict {
                                assert!(decision.is_conflict());
                            }
                        }
                        Context::Announcement(_) | Context::Import | Context::Confirmation => {
                            assert_eq!(decision.standing, None);
                        }
                    }
                    if matches!(relation, CardRelation::Conflict | CardRelation::Stale) {
                        assert!(!decision.changes_state(), "{relation:?} {context:?}");
                    }
                    // No rollback, in every context.
                    if let Some(next) = decision.next_credentials() {
                        assert!(next.active().epoch() >= held.active().epoch());
                        assert_ne!(Some(next.active().transport()), held.retired());
                    }
                }
            }
        }
    }

    #[test]
    fn the_pending_card_is_confirmed_and_nothing_else() {
        let held = held();
        for kind in [RecordKind::Requested, RecordKind::Accepted] {
            let decision = decide(
                kind,
                Some(&held),
                &card(ALICE, 3, 6, None),
                Context::Confirmation,
            )
            .unwrap()
            .unwrap();
            assert_eq!(decision.change(), Some(CredentialChange::Promoted));
            assert_eq!(decision.retires(), Some(&transport(1)));
            assert_eq!(
                decision.next_credentials().unwrap().active(),
                &card(ALICE, 3, 6, None)
            );
        }
    }

    #[test]
    fn a_kind_without_matching_credentials_is_refused() {
        let held = held();
        let candidate = card(ALICE, 1, 3, None);
        assert_eq!(
            decide(RecordKind::Accepted, None, &candidate, Context::Inbound),
            Err(ProtocolError::InvalidValue)
        );
        assert_eq!(
            decide(
                RecordKind::Blocked,
                Some(&held),
                &candidate,
                Context::Inbound
            ),
            Err(ProtocolError::InvalidValue)
        );
        assert_eq!(
            decide(
                RecordKind::Accepted,
                Some(&held),
                &card(MALLORY, 1, 3, None),
                Context::Import
            ),
            Err(ProtocolError::IdentityMismatch)
        );
    }

    #[test]
    fn import_and_admission_agree_on_every_card() {
        // The residual of Phase 3 closed: the evaluation of a card does not
        // depend on whether the user imported it or a handshake proved it.
        let held = held();
        for kind in [RecordKind::Requested, RecordKind::Accepted] {
            for (candidate, _) in candidates() {
                let admitted = decide(kind, Some(&held), &candidate, Context::Inbound)
                    .unwrap()
                    .unwrap();
                let imported = decide(kind, Some(&held), &candidate, Context::Import)
                    .unwrap()
                    .unwrap();
                assert_eq!(admitted.relation(), imported.relation());
                // A card that gives no standing in a handshake never
                // changes the active key through an import for an
                // accepted contact either.
                if kind == RecordKind::Accepted && !admitted.may_learn_local_identity() {
                    assert!(!imported.updates_active_card());
                }
            }
        }
    }
}
