//! Seeds of the fuzz targets `vault_payload` and `message_store`: valid
//! payloads built from fixed values, committed under
//! `fuzz/seeds/vault_payload/` and `fuzz/seeds/message_store/`. The test
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
use monolith_protocol::body::MessageId;
use monolith_protocol::card::{ContactCard, EndpointSet, InvitationCapability};
use monolith_protocol::contact::{RecordKind, RequestMode};
use monolith_protocol::credential::Credentials;
use monolith_protocol::text::{ChatText, DisplayName};
use monolith_storage::message_records::{
    EntryKind, MessageContents, StoredConversation, StoredEntry, StoredMessages,
};
use monolith_storage::record::{
    ContactId, Contents, IdentityId, PayloadVersion, StoredContact, StoredIdentity,
    StoredInvitation, StoredRotation,
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

fn identity(seed: u8, contacts: bool, ids: bool) -> StoredIdentity {
    let mut held = Credentials::new(card(0x30, 1, 1));
    held.announce(&card(0x30, 2, 2), &card(0x30, 1, 1)).unwrap();
    held.admit(&card(0x30, 3, 3)).unwrap();
    StoredIdentity {
        id: ids.then(|| IdentityId::from_bytes([seed; 16]).unwrap()),
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
                id: ids.then(|| ContactId::from_bytes([seed ^ 0xC0; 16]).unwrap()),
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
    // Version 1, of Phase 4, as before, and version 2 with identifiers.
    let encode = |version, identities| {
        Contents {
            version,
            identities,
        }
        .encode()
        .unwrap()
        .to_vec()
    };
    let empty = encode(PayloadVersion::V1, Vec::new());
    let one = encode(PayloadVersion::V1, vec![identity(1, true, false)]);
    let two = encode(
        PayloadVersion::V1,
        vec![identity(1, true, false), identity(2, false, false)],
    );
    let padded = [one.as_slice(), &[0; 64]].concat();
    let empty_2 = encode(PayloadVersion::V2, Vec::new());
    let two_2 = encode(
        PayloadVersion::V2,
        vec![identity(1, true, true), identity(2, true, true)],
    );
    vec![
        ("vault_payload/empty.bin".to_owned(), empty),
        ("vault_payload/one_identity.bin".to_owned(), one),
        ("vault_payload/two_identities.bin".to_owned(), two),
        ("vault_payload/padded.bin".to_owned(), padded),
        ("vault_payload/empty_v2.bin".to_owned(), empty_2),
        ("vault_payload/two_identities_v2.bin".to_owned(), two_2),
    ]
}

fn message_seeds() -> Vec<(String, Vec<u8>)> {
    let entry = |sequence: u64, kind, id: u8, text: &str| StoredEntry {
        sequence,
        kind,
        id: MessageId::from_bytes([id; 16]),
        time: 1_791_700_000 + sequence,
        text: ChatText::new(text).unwrap(),
    };
    let conversation = |contact: u8, peer: u8| StoredConversation {
        contact: ContactId::from_bytes([contact; 16]).unwrap(),
        peer: IdentitySecretKey::from_seed(&[peer; 32]).public_key(),
        next_sequence: 5,
        forgotten: vec![MessageId::from_bytes([0xF0; 16])],
        entries: vec![
            entry(1, EntryKind::Delivered, 1, "hello"),
            entry(2, EntryKind::Received, 1, "hello back"),
            entry(4, EntryKind::Queued, 2, "line one\nline two"),
        ],
    };
    let empty = MessageContents::default().encode().unwrap().to_vec();
    let two = MessageContents {
        identities: vec![
            StoredMessages {
                identity: IdentityId::from_bytes([1; 16]).unwrap(),
                conversations: vec![conversation(0x10, 0x30), conversation(0x11, 0x31)],
            },
            StoredMessages {
                identity: IdentityId::from_bytes([2; 16]).unwrap(),
                conversations: vec![conversation(0x20, 0x30)],
            },
        ],
    }
    .encode()
    .unwrap()
    .to_vec();
    let padded = [two.as_slice(), &[0; 64]].concat();
    vec![
        ("message_store/empty.bin".to_owned(), empty),
        ("message_store/two_identities.bin".to_owned(), two),
        ("message_store/padded.bin".to_owned(), padded),
    ]
}

fn seed_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/seeds")
}

#[test]
fn committed_seeds_are_current() {
    let root = seed_root();
    let write = std::env::var_os("MONOLITH_WRITE_FUZZ_SEEDS").is_some();
    let mut expected = seeds();
    let vault_seeds = expected.len();
    expected.extend(message_seeds());
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
    assert_eq!(on_disk, vault_seeds, "stray or missing files in fuzz/seeds");
    let on_disk = fs::read_dir(root.join("message_store")).unwrap().count();
    assert_eq!(
        on_disk,
        expected.len() - vault_seeds,
        "stray or missing files in fuzz/seeds"
    );
}

#[test]
fn the_seeds_decode() {
    for (path, content) in seeds() {
        assert!(Contents::decode(&content).is_ok(), "{path}");
    }
    for (path, content) in message_seeds() {
        assert!(MessageContents::decode(&content).is_ok(), "{path}");
    }
}
