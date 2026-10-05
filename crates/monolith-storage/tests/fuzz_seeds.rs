//! Seeds of the fuzz target `vault_payload`: valid payloads built from
//! fixed values, committed under `fuzz/seeds/vault_payload/`. The test
//! fails if a committed seed is not what the current code produces. To
//! write them again:
//!
//!     MONOLITH_WRITE_FUZZ_SEEDS=1 cargo test -p monolith-storage --test fuzz_seeds

// Test code builds its own inputs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::fs;
use std::path::{Path, PathBuf};

use monolith_identity::{EndpointEpoch, IdentitySecretKey, OnionServiceKey, TransportPublicKey};
use monolith_protocol::card::{ContactCard, EndpointSet, InvitationCapability};
use monolith_protocol::contact::{RecordKind, RequestMode};
use monolith_protocol::credential::Credentials;
use monolith_protocol::text::DisplayName;
use monolith_storage::record::{
    Contents, StoredContact, StoredIdentity, StoredInvitation, StoredRotation,
};
use zeroize::Zeroizing;

fn place(seed: u8) -> OnionServiceKey {
    OnionServiceKey::from_bytes(
        IdentitySecretKey::from_seed(&[seed; 32])
            .public_key()
            .as_bytes(),
    )
    .unwrap()
}

fn card(identity: u8, key: u8, epoch: u64) -> ContactCard {
    let mut transport = [key; 32];
    transport[31] = 0x40;
    ContactCard::sign(
        &IdentitySecretKey::from_seed(&[identity; 32]),
        TransportPublicKey::from_bytes(&transport).unwrap(),
        EndpointEpoch::new(epoch).unwrap(),
        EndpointSet::single(place(identity.wrapping_add(1))),
        None,
    )
    .unwrap()
}

fn identity(seed: u8, contacts: bool) -> StoredIdentity {
    let mut held = Credentials::new(card(0x30, 1, 1));
    held.announce(&card(0x30, 2, 2), &card(0x30, 1, 1)).unwrap();
    held.admit(&card(0x30, 3, 3)).unwrap();
    StoredIdentity {
        identity_seed: Zeroizing::new([seed; 32]),
        transport_secret: Zeroizing::new([seed ^ 0xA5; 32]),
        onion_secret: Some(Zeroizing::new([seed; 64])),
        epoch: EndpointEpoch::FIRST,
        endpoint: place(seed),
        rotation: Some(StoredRotation {
            transport_secret: Zeroizing::new([seed ^ 0x5A; 32]),
            epoch: EndpointEpoch::new(2).unwrap(),
            switched: true,
        }),
        request_mode: RequestMode::Open,
        label: Some(DisplayName::new("Label").unwrap()),
        invitations: vec![StoredInvitation {
            capability: InvitationCapability::from_bytes([7; 16]),
            label: Some(DisplayName::new("Website").unwrap()),
        }],
        contacts: if contacts {
            vec![StoredContact {
                kind: RecordKind::Accepted,
                credentials: held,
                dial: card(0x30, 1, 1),
                verified: true,
                alias: Some(DisplayName::new("Alias").unwrap()),
                successor_announced: true,
                successor_promoted: false,
            }]
        } else {
            Vec::new()
        },
        blocked: vec![IdentitySecretKey::from_seed(&[0x40; 32]).public_key()],
        declined: vec![IdentitySecretKey::from_seed(&[0x41; 32]).public_key()],
    }
}

fn seeds() -> Vec<(String, Vec<u8>)> {
    let empty = Contents::default().encode().unwrap().to_vec();
    let one = Contents {
        identities: vec![identity(1, true)],
    }
    .encode()
    .unwrap()
    .to_vec();
    let two = Contents {
        identities: vec![identity(1, true), identity(2, false)],
    }
    .encode()
    .unwrap()
    .to_vec();
    let padded = [one.as_slice(), &[0; 64]].concat();
    vec![
        ("vault_payload/empty.bin".to_owned(), empty),
        ("vault_payload/one_identity.bin".to_owned(), one),
        ("vault_payload/two_identities.bin".to_owned(), two),
        ("vault_payload/padded.bin".to_owned(), padded),
    ]
}

fn seed_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/seeds")
}

#[test]
fn committed_seeds_are_current() {
    let root = seed_root();
    let write = std::env::var_os("MONOLITH_WRITE_FUZZ_SEEDS").is_some();
    let expected = seeds();
    for (path, content) in &expected {
        let file = root.join(path);
        if write {
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, content).unwrap();
        }
        let on_disk =
            fs::read(&file).unwrap_or_else(|error| panic!("seed {path} cannot be read: {error}"));
        assert!(on_disk == *content, "seed {path} is out of date");
    }
    let on_disk = fs::read_dir(root.join("vault_payload")).unwrap().count();
    assert_eq!(
        on_disk,
        expected.len(),
        "stray or missing files in fuzz/seeds"
    );
}

#[test]
fn the_seeds_decode() {
    for (path, content) in seeds() {
        assert!(Contents::decode(&content).is_ok(), "{path}");
    }
}
