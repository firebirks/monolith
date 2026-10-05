//! Which cards of a contact are dialed, in which order
//! (`docs/PROTOCOL.md` section 11.4, dialing).
//!
//! Two things decide it and are kept apart: the credentials, which say
//! which transport key stands for the contact, and the card the user
//! confirmed for dialing, which says where to connect. The plan is:
//!
//! 1. The authorized successor, if the identity announced one through its
//!    active key and it states the endpoints the user confirmed. A
//!    successor that answers is promoted by that handshake.
//! 2. The active key at the endpoints the user confirmed: the active card
//!    if it states them, otherwise the confirmed card if it states the
//!    active key (an older statement of the key that stands for the
//!    contact, which the responder answers as the contact).
//!
//! Never dialed: a pending successor, which has no standing until the user
//! confirms it; a card of the retired key; and any card at endpoints the
//! user did not confirm. A promotion that changed the endpoints leaves the
//! contact undialed until the user confirms the new card for dialing; a
//! promotion that kept them makes the promoted card the one dialed. The
//! plan never goes back to an older key: every card in it states the
//! active key or its announced successor.

use monolith_protocol::card::ContactCard;
use monolith_protocol::credential::Credentials;

/// The cards to dial for a contact, in order. Empty if the contact cannot
/// be dialed until the user confirms where to connect.
pub fn plan(credentials: &Credentials, confirmed: &ContactCard) -> Vec<ContactCard> {
    let mut cards = Vec::with_capacity(2);
    let endpoints = confirmed.endpoints();
    if let Some(successor) = credentials.authorized_successor() {
        if successor.endpoints() == endpoints {
            cards.push(successor.clone());
        }
    }
    let active = credentials.active();
    if active.endpoints() == endpoints {
        cards.push(active.clone());
    } else if credentials.authorizes(confirmed) {
        cards.push(confirmed.clone());
    }
    cards
}

#[cfg(test)]
mod tests {
    use super::*;
    use monolith_identity::{
        EndpointEpoch, IdentitySecretKey, OnionServiceKey, TransportPublicKey,
    };
    use monolith_protocol::card::EndpointSet;

    fn transport(seed: u8) -> TransportPublicKey {
        let mut bytes = [seed; 32];
        bytes[31] = 0x40;
        TransportPublicKey::from_bytes(&bytes).unwrap()
    }

    fn place(seed: u8) -> OnionServiceKey {
        OnionServiceKey::from_bytes(
            IdentitySecretKey::from_seed(&[seed; 32])
                .public_key()
                .as_bytes(),
        )
        .unwrap()
    }

    /// A card of Bob with key `key`, epoch `epoch`, at endpoint `at`.
    fn bob(key: u8, epoch: u64, at: u8) -> ContactCard {
        ContactCard::sign(
            &IdentitySecretKey::from_seed(&[0x51; 32]),
            transport(key),
            EndpointEpoch::new(epoch).unwrap(),
            EndpointSet::single(place(at)),
            None,
        )
        .unwrap()
    }

    #[test]
    fn the_active_key_at_the_confirmed_endpoints() {
        let held = Credentials::new(bob(1, 1, 1));
        assert_eq!(plan(&held, &bob(1, 1, 1)), vec![bob(1, 1, 1)]);
    }

    #[test]
    fn a_newer_card_with_other_endpoints_waits_for_the_user() {
        // Bob moved to endpoint 2 at epoch 2: the newer card is active, but
        // the user confirmed endpoint 1, so the card of epoch 1, which
        // states the active key, is still dialed there.
        let mut held = Credentials::new(bob(1, 1, 1));
        held.admit(&bob(1, 2, 2)).unwrap();
        assert_eq!(plan(&held, &bob(1, 1, 1)), vec![bob(1, 1, 1)]);
        // Once the user confirms the new card, it is dialed.
        assert_eq!(plan(&held, &bob(1, 2, 2)), vec![bob(1, 2, 2)]);
    }

    #[test]
    fn an_announced_successor_is_dialed_first() {
        let mut held = Credentials::new(bob(1, 1, 1));
        held.announce(&bob(2, 2, 1), &bob(1, 1, 1)).unwrap();
        assert_eq!(plan(&held, &bob(1, 1, 1)), vec![bob(2, 2, 1), bob(1, 1, 1)]);
        // A successor at other endpoints is not dialed; the active key is.
        let mut moved = Credentials::new(bob(1, 1, 1));
        moved.announce(&bob(2, 2, 2), &bob(1, 1, 1)).unwrap();
        assert_eq!(plan(&moved, &bob(1, 1, 1)), vec![bob(1, 1, 1)]);
    }

    #[test]
    fn a_pending_successor_is_never_dialed() {
        let mut held = Credentials::new(bob(1, 1, 1));
        held.admit(&bob(3, 5, 1)).unwrap();
        assert!(held.pending_successor().is_some());
        assert_eq!(plan(&held, &bob(1, 1, 1)), vec![bob(1, 1, 1)]);
        let mut imported = Credentials::new(bob(1, 1, 1));
        imported.import(bob(3, 5, 1)).unwrap();
        assert_eq!(plan(&imported, &bob(1, 1, 1)), vec![bob(1, 1, 1)]);
    }

    #[test]
    fn after_a_promotion_the_retired_key_is_never_dialed() {
        // Promoted at the same endpoints: the promoted card is dialed.
        let mut held = Credentials::new(bob(1, 1, 1));
        held.announce(&bob(2, 2, 1), &bob(1, 1, 1)).unwrap();
        held.admit(&bob(2, 2, 1)).unwrap();
        assert_eq!(plan(&held, &bob(1, 1, 1)), vec![bob(2, 2, 1)]);
        // Promoted to other endpoints: nothing until the user confirms.
        let mut moved = Credentials::new(bob(1, 1, 1));
        moved.import(bob(2, 3, 2)).unwrap();
        moved.confirm(&bob(2, 3, 2)).unwrap();
        assert!(plan(&moved, &bob(1, 1, 1)).is_empty());
        assert_eq!(plan(&moved, &bob(2, 3, 2)), vec![bob(2, 3, 2)]);
    }

    #[test]
    fn every_planned_card_states_the_active_key_or_its_successor() {
        // Every combination of a few events and confirmed cards.
        let cards: Vec<ContactCard> = [(1, 1, 1), (1, 2, 2), (2, 3, 1), (3, 4, 2), (4, 5, 1)]
            .into_iter()
            .map(|(key, epoch, at)| bob(key, epoch, at))
            .collect();
        for first in &cards {
            for second in &cards {
                let mut held = Credentials::new(cards[0].clone());
                let _ = held.announce(first, &cards[0]);
                let _ = held.admit(second);
                for confirmed in &cards {
                    for card in plan(&held, confirmed) {
                        let successor = held
                            .authorized_successor()
                            .is_some_and(|successor| successor == &card);
                        assert!(held.authorizes(&card) || successor);
                        assert_eq!(card.endpoints(), confirmed.endpoints());
                        assert_ne!(Some(card.transport()), held.retired());
                        assert_ne!(
                            held.pending_successor().map(ContactCard::transport),
                            Some(card.transport())
                        );
                    }
                }
            }
        }
    }
}
