//! The vault format of `docs/STORAGE.md` section 3.2, checked against a
//! reader and a writer written here from that section alone, calling the
//! primitives directly: what `Vault` writes opens by the specification,
//! and a file built by the specification from fixed inputs opens with
//! `Vault`. That file is pinned by its SHA-256, the test vector of the
//! format: a change of the format cannot pass unnoticed.

// Test code builds its own inputs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::AeadInOut;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use monolith_storage::dir::{MemoryDir, VAULT, VaultDir};
use monolith_storage::vault::{KdfParams, Passphrase, Recovery, Vault};
use sha2::{Digest, Sha256};

const INFO: &[u8] = b"MONOLITH-VAULT-PAYLOAD-V1";
const BLOCK: usize = 64 * 1024;
const TAG: usize = 16;

fn kek(passphrase: &[u8], salt: &[u8], params: KdfParams) -> [u8; 32] {
    let params = Params::new(
        params.memory_kib,
        params.iterations,
        params.parallelism,
        Some(32),
    )
    .unwrap();
    let mut out = [0; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(passphrase, salt, &mut out)
        .unwrap();
    out
}

fn payload_key(vault_key: &[u8]) -> [u8; 32] {
    let mut out = [0; 32];
    Hkdf::<Sha256>::new(None, vault_key)
        .expand(INFO, &mut out)
        .unwrap();
    out
}

fn cipher(key: &[u8; 32]) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new_from_slice(key).unwrap()
}

fn seal(key: &[u8; 32], nonce: &[u8], associated: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let nonce: [u8; 24] = nonce.try_into().unwrap();
    let mut buffer = plaintext.to_vec();
    let tag = cipher(key)
        .encrypt_inout_detached(
            &XNonce::from(nonce),
            associated,
            buffer.as_mut_slice().into(),
        )
        .unwrap();
    buffer.extend_from_slice(&tag);
    buffer
}

fn open(key: &[u8; 32], nonce: &[u8], associated: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    let nonce: [u8; 24] = nonce.try_into().unwrap();
    let (body, tag) = sealed.split_at(sealed.len() - TAG);
    let tag: [u8; TAG] = tag.try_into().unwrap();
    let mut buffer = body.to_vec();
    cipher(key)
        .decrypt_inout_detached(
            &XNonce::from(nonce),
            associated,
            buffer.as_mut_slice().into(),
            &tag.into(),
        )
        .ok()?;
    Some(buffer)
}

fn be32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

/// Opens `file` by section 3.2: the vault key, then the padded plaintext.
fn open_by_specification(file: &[u8], passphrase: &[u8]) -> Option<([u8; 32], Vec<u8>)> {
    let params = KdfParams {
        memory_kib: be32(file, 12),
        iterations: be32(file, 16),
        parallelism: be32(file, 20),
    };
    let kek = kek(passphrase, &file[24..40], params);
    let vault_key: [u8; 32] = open(&kek, &file[40..64], &file[..64], &file[64..112])?
        .try_into()
        .unwrap();
    let plaintext = open(
        &payload_key(&vault_key),
        &file[120..144],
        &file[..148],
        &file[148..],
    )?;
    Some((vault_key, plaintext))
}

#[test]
fn what_the_vault_writes_opens_by_the_specification() {
    // The passphrase as typed, with a decomposed accent, and as Argon2id
    // gets it: normalized to NFC.
    let typed = "e\u{301}te\u{301} passphrase";
    let normalized = "\u{e9}t\u{e9} passphrase";
    let dir = MemoryDir::new();
    let mut vault = Vault::create(
        dir.clone(),
        &Passphrase::new(typed).unwrap(),
        KdfParams::FLOOR,
        b"first",
    )
    .unwrap();
    vault.write(b"the second payload").unwrap();
    let file = dir.read(VAULT, u64::MAX).unwrap().unwrap();

    assert_eq!(&file[0..8], b"MNLTVLT\0");
    assert_eq!(&file[8..10], &1_u16.to_be_bytes());
    assert_eq!(file[10], 1, "Argon2id, version 0x13");
    assert_eq!(file[11], 0);
    assert_eq!(be32(&file, 12), KdfParams::FLOOR.memory_kib);
    assert_eq!(be32(&file, 16), KdfParams::FLOOR.iterations);
    assert_eq!(be32(&file, 20), KdfParams::FLOOR.parallelism);
    assert_eq!(&file[112..120], &2_u64.to_be_bytes(), "the generation");
    let payload_len = usize::try_from(be32(&file, 144)).unwrap();
    assert_eq!(payload_len, BLOCK + TAG, "one padding block and the tag");
    assert_eq!(file.len(), 148 + payload_len);

    let (_, plaintext) = open_by_specification(&file, normalized.as_bytes()).unwrap();
    assert_eq!(plaintext.len(), BLOCK);
    assert_eq!(&plaintext[..18], b"the second payload");
    assert!(plaintext[18..].iter().all(|byte| *byte == 0));

    // Not the passphrase as typed: it is normalized first.
    assert!(open_by_specification(&file, typed.as_bytes()).is_none());
    // Every header byte is associated data of the payload. In the KDF
    // fields only the lowest bit is flipped, which keeps the cost of the
    // derivation within reach: this reader has no bounds.
    for offset in [0, 9, 11, 15, 19, 23, 112, 119, 120, 147] {
        let mut changed = file.clone();
        changed[offset] ^= 1;
        assert!(
            open_by_specification(&changed, normalized.as_bytes()).is_none(),
            "byte {offset}"
        );
    }
}

/// The vault file of fixed inputs, built by section 3.2.
fn test_vector() -> Vec<u8> {
    let params = KdfParams::FLOOR;
    let salt = [0x11_u8; 16];
    let wrap_nonce = [0x22_u8; 24];
    let vault_key = [0x33_u8; 32];
    let generation = 7_u64;
    let payload_nonce = [0x44_u8; 24];
    let mut plaintext = b"built by the specification".to_vec();
    plaintext.resize(BLOCK, 0);

    let mut file = Vec::new();
    file.extend_from_slice(b"MNLTVLT\0");
    file.extend_from_slice(&1_u16.to_be_bytes());
    file.push(1);
    file.push(0);
    file.extend_from_slice(&params.memory_kib.to_be_bytes());
    file.extend_from_slice(&params.iterations.to_be_bytes());
    file.extend_from_slice(&params.parallelism.to_be_bytes());
    file.extend_from_slice(&salt);
    file.extend_from_slice(&wrap_nonce);
    assert_eq!(file.len(), 64);
    let kek = kek(b"test vector passphrase", &salt, params);
    let wrapped = seal(&kek, &wrap_nonce, &file, &vault_key);
    file.extend_from_slice(&wrapped);
    file.extend_from_slice(&generation.to_be_bytes());
    file.extend_from_slice(&payload_nonce);
    file.extend_from_slice(&u32::try_from(BLOCK + TAG).unwrap().to_be_bytes());
    assert_eq!(file.len(), 148);
    let payload = seal(&payload_key(&vault_key), &payload_nonce, &file, &plaintext);
    file.extend_from_slice(&payload);
    file
}

#[test]
fn a_file_built_by_the_specification_opens_with_the_vault() {
    let file = test_vector();
    let digest: String = Sha256::digest(&file)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(
        digest,
        "3a644ceea65f7794f9e7b470e67f6d2e36f0e35474c05c9a5449579d0120f3f1"
    );
    let dir = MemoryDir::new();
    dir.write_new(VAULT, &file).unwrap();
    let (vault, plaintext, recovery) =
        Vault::open(dir, &Passphrase::new("test vector passphrase").unwrap()).unwrap();
    assert_eq!(recovery, Recovery::Clean);
    assert_eq!(vault.generation(), 7);
    assert_eq!(&plaintext[..26], b"built by the specification");
    assert!(plaintext[26..].iter().all(|byte| *byte == 0));
}
