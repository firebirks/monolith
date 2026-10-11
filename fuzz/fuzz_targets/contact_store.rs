//! The contact store of one local identity under an arbitrary sequence of
//! user operations on three remote identities: imports, confirmations of
//! a pending card and of a dial card, block, unblock, deletion and the
//! verification mark. After every operation the store holds what a model
//! built on `contact::decide` alone holds, every record keeps the
//! invariants of the credentials (the key retired last is never the
//! active one), a contact's active epoch never goes down while it stays a
//! contact, and the card confirmed for dialing is always one of the
//! contact. Every operation here is the user's; that a key never regains
//! standing by itself, through an admission or an announcement, is the
//! property of `credential_sequence`.
//!
//! Every operation is three bytes. The first selects it, modulo 7: import
//! (0), confirm the pending card (1), confirm a dial card (2), block (3),
//! unblock (4), delete (5), mark verified (6). The second selects the
//! remote identity, modulo 3. The third selects a card of that identity:
//! its low two bits one of four transport keys, the next three bits,
//! modulo 5, the epoch from 1 to 5, the next bit one of two endpoints.

#![no_main]

use std::sync::LazyLock;

use libfuzzer_sys::fuzz_target;
use monolith_core::contacts::StoreError;
use monolith_core::identity::{IdentityKeys, Installation};
use monolith_identity::{
    EndpointEpoch, IdentityPublicKey, IdentitySecretKey, OnionServiceKey, TransportPublicKey,
};
use monolith_protocol::card::{ContactCard, EndpointSet};
use monolith_protocol::contact::{self, Context, RecordKind};
use monolith_protocol::credential::{CardRelation, Credentials};

const REMOTES: usize = 3;
const KEYS: usize = 4;
const EPOCHS: usize = 5;
const PLACES: usize = 2;

fn secret(remote: usize) -> IdentitySecretKey {
    IdentitySecretKey::from_seed(&[0x30 + u8::try_from(remote).unwrap(); 32])
}

fn transport(remote: usize, key: usize) -> TransportPublicKey {
    let mut bytes = [u8::try_from(remote * KEYS + key).unwrap() + 1; 32];
    bytes[31] = 0x40;
    TransportPublicKey::from_bytes(&bytes).unwrap()
}

fn place(index: usize) -> OnionServiceKey {
    let seed = [0x70 + u8::try_from(index).unwrap(); 32];
    OnionServiceKey::from_bytes(IdentitySecretKey::from_seed(&seed).public_key().as_bytes())
        .unwrap()
}

/// Every card the input can name, signed once.
static CARDS: LazyLock<Vec<ContactCard>> = LazyLock::new(|| {
    let mut cards = Vec::new();
    for remote in 0..REMOTES {
        for key in 0..KEYS {
            for epoch in 1..=EPOCHS {
                for at in 0..PLACES {
                    cards.push(
                        ContactCard::sign(
                            &secret(remote),
                            transport(remote, key),
                            EndpointEpoch::new(u64::try_from(epoch).unwrap()).unwrap(),
                            EndpointSet::single(place(at)),
                            None,
                        )
                        .unwrap(),
                    );
                }
            }
        }
    }
    cards
});

fn card(remote: usize, byte: u8) -> &'static ContactCard {
    let key = usize::from(byte & 3);
    let epoch = usize::from((byte >> 2) & 7) % EPOCHS;
    let at = usize::from((byte >> 5) & 1);
    &CARDS[((remote * KEYS + key) * EPOCHS + epoch) * PLACES + at]
}

/// What the model holds about one remote identity.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Model {
    kind: RecordKind,
    credentials: Option<Credentials>,
    dial: Option<ContactCard>,
    verified: bool,
}

impl Model {
    const NONE: Self = Self {
        kind: RecordKind::None,
        credentials: None,
        dial: None,
        verified: false,
    };

    fn contact(&self) -> bool {
        self.kind.is_contact()
    }
}

/// The model's outcome of one operation: what the store has to return,
/// as success or the kind of refusal.
fn step(model: &mut Model, operation: u8, presented: &ContactCard) -> Result<(), StoreError> {
    match operation {
        0 => {
            let decision =
                match contact::decide(model.kind, model.credentials.as_ref(), presented, Context::Import)
                    .unwrap()
                {
                    Ok(decision) => decision,
                    Err(_) => return Err(StoreError::Blocked),
                };
            if decision.before != decision.after {
                *model = Model {
                    kind: decision.after,
                    credentials: decision.next_credentials().cloned(),
                    dial: Some(presented.clone()),
                    verified: false,
                };
            } else {
                if let Some(next) = decision.next_credentials() {
                    model.credentials = Some(next.clone());
                }
                if decision.updates_active_card() {
                    model.dial = model.credentials.as_ref().map(|held| held.active().clone());
                }
            }
            Ok(())
        }
        1 => {
            if !model.contact() {
                return Err(StoreError::NotFound);
            }
            let decision = match contact::decide(
                model.kind,
                model.credentials.as_ref(),
                presented,
                Context::Confirmation,
            ) {
                Ok(Ok(decision)) => decision,
                _ => return Err(StoreError::NotThatCard),
            };
            model.credentials = decision.next_credentials().cloned();
            model.dial = Some(presented.clone());
            Ok(())
        }
        2 => {
            let Some(held) = model.credentials.as_ref().filter(|_| model.contact()) else {
                return Err(StoreError::NotFound);
            };
            if held.relation(presented).unwrap() != CardRelation::Same {
                return Err(StoreError::NotThatCard);
            }
            model.dial = Some(presented.clone());
            Ok(())
        }
        3 => {
            *model = Model {
                kind: RecordKind::Blocked,
                ..Model::NONE
            };
            Ok(())
        }
        4 => {
            if model.kind != RecordKind::Blocked {
                return Err(StoreError::NotFound);
            }
            *model = Model::NONE;
            Ok(())
        }
        5 => {
            if !model.contact() {
                return Err(StoreError::NotFound);
            }
            *model = Model::NONE;
            Ok(())
        }
        _ => {
            if !model.contact() {
                return Err(StoreError::NotFound);
            }
            model.verified = true;
            Ok(())
        }
    }
}

fn check(credentials: &Credentials) {
    let restored = Credentials::restore(
        credentials.active().clone(),
        credentials.authorized_successor().cloned(),
        credentials.pending_successor().cloned(),
        credentials.pending_was_imported(),
        credentials.retired().copied(),
        credentials.invitation().cloned(),
    );
    assert_eq!(restored.as_ref(), Ok(credentials));
}

fuzz_target!(|data: &[u8]| {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(async {
        let installation = Installation::ephemeral();
        let local = installation
            .restore_identity(
                IdentityKeys {
                    seed: zeroize::Zeroizing::new([1; 32]),
                    transport: zeroize::Zeroizing::new([2; 32]),
                    onion: None,
                    epoch: EndpointEpoch::FIRST,
                    endpoint: place(7),
                },
                None,
            )
            .await
            .unwrap();
        let identities: Vec<IdentityPublicKey> =
            (0..REMOTES).map(|remote| secret(remote).public_key()).collect();
        let mut models = vec![Model::NONE; REMOTES];
        for operation in data.chunks_exact(3) {
            let (selector, remote, card_byte) = (operation[0] % 7, operation[1], operation[2]);
            let remote = usize::from(remote) % REMOTES;
            let identity = &identities[remote];
            let presented = card(remote, card_byte);
            let before = models[remote].clone();
            let expected = step(&mut models[remote], selector, presented);
            let got = match selector {
                0 => local.import(presented).await.map(|_| ()),
                1 => local.confirm_pending(presented).await,
                2 => local.confirm_dial(presented).await,
                3 => local.block(identity).await,
                4 => local.unblock(identity).await,
                5 => local.delete(identity).await,
                _ => local.set_verified(identity, true).await,
            };
            assert_eq!(got, expected, "{selector}");
            let model = &models[remote];
            let view = local.contact(identity);
            match view {
                None => assert_eq!(model.kind, RecordKind::None),
                Some(view) => {
                    assert_eq!(view.kind, model.kind);
                    assert_eq!(view.credentials, model.credentials);
                    assert_eq!(view.dial, model.dial);
                    assert_eq!(view.verified, model.verified);
                    if let (Some(held), Some(dial)) = (&view.credentials, &view.dial) {
                        check(held);
                        assert_eq!(dial.identity(), held.identity());
                    }
                }
            }
            // No rollback while the identity stays a contact.
            if let (Some(old), Some(new)) = (&before.credentials, &model.credentials) {
                if before.contact() && model.contact() {
                    assert!(new.active().epoch() >= old.active().epoch());
                    if new.active().transport() != old.active().transport() {
                        assert_eq!(new.retired(), Some(old.active().transport()));
                    }
                }
            }
        }
    });
});
