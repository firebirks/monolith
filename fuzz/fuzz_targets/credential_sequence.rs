//! Credentials of a contact: an arbitrary sequence of admissions,
//! announcements, confirmations and imports keeps the invariants, never
//! moves the active epoch back, and changes the active transport key only
//! by promoting a proven authorized successor, by confirming the pending
//! one, or by a replacement for a contact that has not accepted.
//!
//! Every event is three bytes. The first selects the event, its value
//! modulo 5: an admission (0), an announcement (1), a confirmation (2), an
//! import for an accepted contact (3) or a replacement for a requested
//! contact (4). For an announcement, the first byte divided by 5, modulo
//! 5, selects the key of the session it arrives on: 0 for the active key,
//! otherwise one of the four keys. The second byte selects the card: its
//! low two bits one of four transport keys, the next bit one of two
//! endpoints. The third byte is the epoch: 255 is the largest epoch, any
//! other value its remainder modulo 8, plus 1. Every card is a card of the
//! same identity.

#![no_main]

use std::sync::LazyLock;

use libfuzzer_sys::fuzz_target;
use monolith_identity::{EndpointEpoch, IdentitySecretKey, OnionServiceKey, TransportPublicKey};
use monolith_protocol::card::{ContactCard, EndpointSet};
use monolith_protocol::credential::{CredentialChange, Credentials};
use monolith_protocol::session::PeerRecord;

const IDENTITY: [u8; 32] = [0x10; 32];

/// The epochs a card can have, by index: 1 to 8, then the largest.
const EPOCHS: [u64; 9] = [1, 2, 3, 4, 5, 6, 7, 8, u64::MAX];

fn transport(key: usize) -> TransportPublicKey {
    let mut bytes = [u8::try_from(key).unwrap() + 1; 32];
    bytes[31] = 0x40;
    TransportPublicKey::from_bytes(&bytes).unwrap()
}

fn endpoint(place: usize) -> OnionServiceKey {
    let seed = [0x60 + u8::try_from(place).unwrap(); 32];
    OnionServiceKey::from_bytes(IdentitySecretKey::from_seed(&seed).public_key().as_bytes())
        .unwrap()
}

/// Every card the input can name, signed once: four keys, nine epochs, two
/// endpoints.
static CARDS: LazyLock<Vec<ContactCard>> = LazyLock::new(|| {
    let secret = IdentitySecretKey::from_seed(&IDENTITY);
    let mut cards = Vec::new();
    for key in 0..4 {
        for epoch in EPOCHS {
            for place in 0..2 {
                cards.push(
                    ContactCard::sign(
                        &secret,
                        transport(key),
                        EndpointEpoch::new(epoch).unwrap(),
                        EndpointSet::single(endpoint(place)),
                        None,
                    )
                    .unwrap(),
                );
            }
        }
    }
    cards
});

fn card(key: usize, epoch_index: usize, place: usize) -> &'static ContactCard {
    &CARDS[(key * EPOCHS.len() + epoch_index) * 2 + place]
}

fn epoch_index(byte: u8) -> usize {
    if byte == 255 {
        EPOCHS.len() - 1
    } else {
        usize::from(byte % 8)
    }
}

/// The index of an epoch in `EPOCHS`.
fn index_of(epoch: EndpointEpoch) -> usize {
    EPOCHS
        .iter()
        .position(|value| *value == epoch.get())
        .unwrap()
}

fn check(credentials: &Credentials) {
    let active = credentials.active();
    for held in [
        credentials.authorized_successor(),
        credentials.pending_successor(),
    ]
    .into_iter()
    .flatten()
    {
        assert_eq!(held.identity(), active.identity());
        assert!(active.epoch() < held.epoch());
        assert_ne!(held.transport(), active.transport());
        assert_ne!(Some(held.transport()), credentials.retired());
    }
    if let (Some(authorized), Some(pending)) = (
        credentials.authorized_successor(),
        credentials.pending_successor(),
    ) {
        assert_ne!(authorized.transport(), pending.transport());
    }
    assert_ne!(Some(active.transport()), credentials.retired());
    assert!(credentials.authorizes(active));
}

fuzz_target!(|data: &[u8]| {
    let mut credentials = Credentials::new(card(0, 0, 0).clone());
    for event in data.chunks_exact(3) {
        let (selector, key_byte, epoch_byte) = (event[0], event[1], event[2]);
        let presented = card(
            usize::from(key_byte & 3),
            epoch_index(epoch_byte),
            usize::from((key_byte >> 2) & 1),
        );
        let before = credentials.clone();
        let operation = selector % 5;
        let change = match operation {
            0 => {
                // Through the record, so that the standing is checked too.
                let admission = PeerRecord::Accepted(&mut credentials)
                    .admit(presented)
                    .unwrap();
                let contact = matches!(
                    admission.change,
                    Some(
                        CredentialChange::Unchanged
                            | CredentialChange::Advanced
                            | CredentialChange::Promoted
                    )
                );
                assert_eq!(admission.standing.is_contact_record(), contact);
                assert!(!matches!(
                    admission.change,
                    Some(CredentialChange::Authorized | CredentialChange::NoContinuity) | None
                ));
                admission.change.ok_or(())
            }
            1 => {
                let session_key = usize::from((selector / 5) % 5);
                let session = if session_key == 0 {
                    before.active().clone()
                } else {
                    card(session_key - 1, index_of(before.active().epoch()), 0).clone()
                };
                let change = credentials.announce(presented, &session).unwrap();
                assert!(!matches!(
                    change,
                    CredentialChange::Promoted | CredentialChange::Pending
                ));
                if session.transport() != before.active().transport() {
                    assert_eq!(change, CredentialChange::NoContinuity);
                }
                Ok(change)
            }
            2 => credentials.confirm(presented).map_err(|_| ()),
            3 => Ok(credentials.import(presented.clone()).unwrap()),
            _ => Ok(credentials.replace(presented.clone()).unwrap()),
        };
        check(&credentials);
        assert!(before.active().epoch() <= credentials.active().epoch());

        let key_changed = before.active().transport() != credentials.active().transport();
        assert_eq!(key_changed, change == Ok(CredentialChange::Promoted));
        if key_changed {
            let new_key = credentials.active().transport();
            let allowed = match operation {
                0 => before
                    .authorized_successor()
                    .is_some_and(|held| held.transport() == new_key),
                2 => before
                    .pending_successor()
                    .is_some_and(|held| held.transport() == new_key),
                4 => true,
                _ => false,
            };
            assert!(allowed);
            assert_eq!(credentials.retired(), Some(before.active().transport()));
            assert!(!credentials.authorizes(before.active()));
        }
        // What changes nothing leaves the credentials as they were.
        if matches!(
            change,
            Ok(CredentialChange::Unchanged
                | CredentialChange::Conflict
                | CredentialChange::Stale
                | CredentialChange::NoContinuity)
                | Err(())
        ) {
            assert!(credentials == before);
        }
        // A card that states the retired key never stands for the contact.
        if let Some(retired) = credentials.retired() {
            assert_ne!(credentials.active().transport(), retired);
        }
    }
});
