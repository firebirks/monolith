//! The payload of the transfer store (`docs/adr/0014-transfer-staging.md`):
//! `TransferRecords::decode` on arbitrary bytes either refuses them or gives
//! records that keep every rule of the format (`TransferRecords::check`: at
//! most `MAX_TRANSFER_RECORDS` records, each with the flags, reason, blob,
//! digest and deadline its direction and state allow, no two with one key
//! or one blob); their encoding, with the zero padding the store adds, is
//! the input itself and decodes to the same records. The payload is
//! authenticated before it is read, so this is the parser a changed or
//! damaged store would reach only with the key; it must still never panic
//! or allocate by what it reads.
//!
//! The input is the payload.

#![no_main]

use libfuzzer_sys::fuzz_target;
use monolith_protocol::limits::MAX_TRANSFER_RECORDS;
use monolith_storage::transfers::TransferRecords;

fuzz_target!(|data: &[u8]| {
    let Ok(records) = TransferRecords::decode(data) else {
        return;
    };
    assert!(records.records.len() <= MAX_TRANSFER_RECORDS);
    assert!(records.check().is_ok());
    for record in &records.records {
        assert!(record.check().is_ok());
        assert_eq!(record.has_blob(), record.blob.is_some());
    }
    // One valid encoding: what was read is the encoding, then zeros.
    let encoded = records.encode().unwrap();
    assert!(data.starts_with(&encoded));
    assert!(data[encoded.len()..].iter().all(|byte| *byte == 0));
    assert_eq!(TransferRecords::decode(&encoded).unwrap(), records);
});
