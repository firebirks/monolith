//! The formats of `docs/adr/0014-transfer-staging.md`, checked against a
//! reader and a writer written here from that ADR alone, calling the
//! primitives directly: a blob and a transfer store built by the ADR from
//! fixed inputs open with the code, and what the code writes opens by the
//! ADR. The two files built here are pinned by their SHA-256, the test
//! vectors of the formats.

// Test code builds its own inputs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation
)]

use std::io::Read;

use chacha20poly1305::aead::AeadInOut;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use monolith_protocol::text::Filename;
use monolith_storage::blob::{Blob, BlobId, BlobReader, StagingKey, stage};
use monolith_storage::dir::{MemoryDir, TRANSFERS, VaultDir};
use monolith_storage::record::{ContactId, IdentityId};
use monolith_storage::transfers::{
    Direction, Flags, State, TransferFile, TransferRecord, TransferRecords, TransferStoreKey,
};
use sha2::{Digest, Sha256};

const L: usize = 64_490;
const STAGING: [u8; 32] = [0x11; 32];
const BLOB_ID: [u8; 16] = [0x22; 16];
const STORE: [u8; 32] = [0x55; 32];

fn blob_key(staging: &[u8; 32], id: &[u8; 16]) -> [u8; 32] {
    let mut info = b"MONOLITH-TRANSFER-BLOB-V1".to_vec();
    info.extend_from_slice(id);
    let mut out = [0; 32];
    Hkdf::<Sha256>::new(None, staging)
        .expand(&info, &mut out)
        .unwrap();
    out
}

fn blob_header(id: &[u8; 16]) -> Vec<u8> {
    let mut header = b"MNLTBLB\0".to_vec();
    header.extend_from_slice(&1_u16.to_be_bytes());
    header.extend_from_slice(&[0, 0]);
    header.extend_from_slice(id);
    header.extend_from_slice(&(L as u32).to_be_bytes());
    header
}

fn seal(key: &[u8; 32], nonce: [u8; 24], associated: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let mut buffer = plaintext.to_vec();
    let tag = XChaCha20Poly1305::new_from_slice(key)
        .unwrap()
        .encrypt_inout_detached(
            &XNonce::from(nonce),
            associated,
            buffer.as_mut_slice().into(),
        )
        .unwrap();
    buffer.extend_from_slice(&tag);
    buffer
}

fn open(key: &[u8; 32], nonce: &[u8], associated: &[u8], sealed: &[u8]) -> Vec<u8> {
    let nonce: [u8; 24] = nonce.try_into().unwrap();
    let (body, tag) = sealed.split_at(sealed.len() - 16);
    let tag: [u8; 16] = tag.try_into().unwrap();
    let mut buffer = body.to_vec();
    XChaCha20Poly1305::new_from_slice(key)
        .unwrap()
        .decrypt_inout_detached(
            &XNonce::from(nonce),
            associated,
            buffer.as_mut_slice().into(),
            &tag.into(),
        )
        .unwrap();
    buffer
}

fn file_bytes(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 % 256) as u8).collect()
}

/// A blob of `data` built by the ADR, with fixed nonces.
fn build_blob(staging: &[u8; 32], id: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let key = blob_key(staging, id);
    let header = blob_header(id);
    let size = data.len() as u64;
    let count = data.len().div_ceil(L);
    let mut blob = header.clone();
    for (index, part) in data.chunks(L).enumerate() {
        let mut ad = header.clone();
        ad.extend_from_slice(&size.to_be_bytes());
        ad.extend_from_slice(&(index as u64).to_be_bytes());
        ad.push(u8::from(index + 1 == count));
        let mut plaintext = part.to_vec();
        plaintext.resize(L, 0);
        let nonce = [index as u8 + 0x33; 24];
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&seal(&key, nonce, &ad, &plaintext));
    }
    blob
}

/// The file a blob holds, read by the ADR.
fn read_blob(staging: &[u8; 32], blob: &[u8], size: u64) -> Vec<u8> {
    let header = &blob[..32];
    assert_eq!(&header[..8], b"MNLTBLB\0");
    assert_eq!(&header[8..12], &[0, 1, 0, 0]);
    let id: [u8; 16] = header[12..28].try_into().unwrap();
    assert_eq!(&header[28..32], &(L as u32).to_be_bytes());
    let key = blob_key(staging, &id);
    let count = (size as usize).div_ceil(L);
    assert_eq!(blob.len(), 32 + count * (24 + L + 16));
    let mut data = Vec::new();
    for index in 0..count {
        let segment = &blob[32 + index * (24 + L + 16)..32 + (index + 1) * (24 + L + 16)];
        let mut ad = header.to_vec();
        ad.extend_from_slice(&size.to_be_bytes());
        ad.extend_from_slice(&(index as u64).to_be_bytes());
        ad.push(u8::from(index + 1 == count));
        let plaintext = open(&key, &segment[..24], &ad, &segment[24..]);
        let keep = (size as usize - index * L).min(L);
        assert!(plaintext[keep..].iter().all(|byte| *byte == 0));
        data.extend_from_slice(&plaintext[..keep]);
    }
    data
}

#[test]
fn a_blob_built_by_the_adr_opens_and_is_pinned() {
    let data = file_bytes(L + 10);
    let built = build_blob(&STAGING, &BLOB_ID, &data);
    assert_eq!(
        hex(&Sha256::digest(&built)),
        "79c5b06312b6ecd37e90dcf42ec97c4bdd5902e6ecb6894856b808bfd1ad950a"
    );
    let id = BlobId::from_bytes(BLOB_ID).unwrap();
    let dir = MemoryDir::new();
    let mut file = dir.create_reserved(&id.name(), 0, 0).unwrap();
    file.write_at(0, &built).unwrap();
    let key = StagingKey::from_bytes_for_testing(STAGING);
    let mut blob = Blob::open(&dir, &key, id, data.len() as u64)
        .unwrap()
        .unwrap();
    assert_eq!(blob.scan().unwrap(), 2);
    let digest: [u8; 32] = Sha256::digest(&data).into();
    assert_eq!(blob.digest().unwrap(), digest);
    let mut read = Vec::new();
    BlobReader::new(blob, digest)
        .read_to_end(&mut read)
        .unwrap();
    assert_eq!(read, data);
}

#[test]
fn a_blob_the_code_writes_opens_by_the_adr() {
    for len in [0, 5, L, 2 * L + 1] {
        let data = file_bytes(len);
        let dir = MemoryDir::new();
        let key = StagingKey::from_bytes_for_testing(STAGING);
        let (id, digest) = stage(&dir, &key, len as u64, 0, &mut data.as_slice()).unwrap();
        assert_eq!(digest, <[u8; 32]>::from(Sha256::digest(&data)));
        let bytes = dir.content(&id.name()).unwrap();
        assert_eq!(read_blob(&STAGING, &bytes, len as u64), data, "{len}");
    }
}

#[test]
fn every_segment_the_code_writes_has_a_nonce_of_its_own_and_no_plaintext_shows() {
    // The same file staged twice under one key: each segment of either
    // blob has a nonce drawn for it, so no two share one, and no run of
    // the file's bytes is found in either blob.
    let len = 3 * L + 5;
    let data: Vec<u8> = (0..len).map(|i| (i * 31 % 251) as u8).collect();
    let dir = MemoryDir::new();
    let key = StagingKey::from_bytes_for_testing(STAGING);
    let mut nonces = Vec::new();
    let mut blobs = Vec::new();
    for _ in 0..2 {
        let (id, _) = stage(&dir, &key, len as u64, 0, &mut data.as_slice()).unwrap();
        let bytes = dir.content(&id.name()).unwrap();
        for index in 0..len.div_ceil(L) {
            let at = 32 + index * (24 + L + 16);
            nonces.push(bytes[at..at + 24].to_vec());
        }
        blobs.push(bytes);
    }
    let mut distinct = nonces.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), nonces.len(), "a nonce was used twice");
    for blob in &blobs {
        for run in data.chunks(32).step_by(400) {
            assert!(
                !blob.windows(run.len()).any(|window| window == run),
                "the file's bytes are in a blob"
            );
        }
    }
}

fn record() -> TransferRecord {
    TransferRecord {
        transfer: [0x77; 16],
        identity: IdentityId::from_bytes([0x01; 16]).unwrap(),
        contact: ContactId::from_bytes([0x02; 16]).unwrap(),
        direction: Direction::Outgoing,
        state: State::Waiting,
        flags: Flags::from_bits(Flags::OFFER_READ).unwrap(),
        reason: None,
        size: 1000,
        name: Filename::new("report.pdf").unwrap(),
        digest: [0x09; 32],
        blob: BlobId::from_bytes(BLOB_ID),
        created: 1_700_000_000,
        deadline: 1_700_604_800,
    }
}

/// The records of [`record`], as the ADR lays them out.
fn record_bytes() -> Vec<u8> {
    let mut bytes = 1_u16.to_be_bytes().to_vec();
    bytes.extend_from_slice(&[0x77; 16]);
    bytes.extend_from_slice(&[0x01; 16]);
    bytes.extend_from_slice(&[0x02; 16]);
    bytes.extend_from_slice(&[0, 1, 0b1, 0]);
    bytes.extend_from_slice(&1000_u64.to_be_bytes());
    bytes.push(10);
    bytes.extend_from_slice(b"report.pdf");
    bytes.extend_from_slice(&[0x09; 32]);
    bytes.extend_from_slice(&BLOB_ID);
    bytes.extend_from_slice(&1_700_000_000_u64.to_be_bytes());
    bytes.extend_from_slice(&1_700_604_800_u64.to_be_bytes());
    bytes
}

#[test]
fn a_transfer_store_built_by_the_adr_opens_and_is_pinned() {
    let records = record_bytes();
    let mut plaintext = 1_u16.to_be_bytes().to_vec();
    plaintext.extend_from_slice(&(records.len() as u32).to_be_bytes());
    plaintext.extend_from_slice(&records);
    plaintext.resize(64 * 1024, 0);
    let nonce = [0x66; 24];
    let mut header = b"MNLTXFR\0".to_vec();
    header.extend_from_slice(&1_u16.to_be_bytes());
    header.extend_from_slice(&[0, 0]);
    header.extend_from_slice(&7_u64.to_be_bytes());
    header.extend_from_slice(&nonce);
    header.extend_from_slice(&((64 * 1024 + 16) as u32).to_be_bytes());
    let mut file = header.clone();
    file.extend_from_slice(&seal(&STORE, nonce, &header, &plaintext));
    assert_eq!(
        hex(&Sha256::digest(&file)),
        "31570ddc04be43caed0554bfc751648f3f26deda1f64c701823d56d66a3301fc"
    );

    let dir = MemoryDir::new();
    dir.write_new(TRANSFERS, &file).unwrap();
    let (store, read, _) =
        TransferFile::open(dir, TransferStoreKey::from_bytes_for_testing(STORE)).unwrap();
    assert_eq!(store.generation(), 7);
    assert_eq!(
        read,
        TransferRecords {
            records: vec![record()]
        }
    );
}

#[test]
fn a_transfer_store_the_code_writes_opens_by_the_adr() {
    let dir = MemoryDir::new();
    let (mut store, _, _) =
        TransferFile::open(dir.clone(), TransferStoreKey::from_bytes_for_testing(STORE)).unwrap();
    store
        .write(&TransferRecords {
            records: vec![record()],
        })
        .unwrap();
    let file = dir.read(TRANSFERS, 1024 * 1024).unwrap().unwrap();
    assert_eq!(&file[..12], b"MNLTXFR\0\0\x01\0\0");
    assert_eq!(&file[12..20], &2_u64.to_be_bytes());
    let payload_len = u32::from_be_bytes(file[44..48].try_into().unwrap()) as usize;
    assert_eq!(file.len(), 48 + payload_len);
    let plaintext = open(&STORE, &file[20..44], &file[..48], &file[48..]);
    assert_eq!(plaintext.len() % (64 * 1024), 0);
    assert_eq!(&plaintext[..2], &[0, 1]);
    let len = u32::from_be_bytes(plaintext[2..6].try_into().unwrap()) as usize;
    assert_eq!(&plaintext[6..6 + len], record_bytes().as_slice());
    assert!(plaintext[6 + len..].iter().all(|byte| *byte == 0));
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
