//! Binary contact card decoder: arbitrary bytes either fail or decode to a
//! card that encodes back to exactly the same bytes and whose signature
//! verifies over its signed bytes.

#![no_main]

use libfuzzer_sys::fuzz_target;
use monolith_protocol::card::ContactCard;
use monolith_protocol::limits::MAX_ACTIVE_ENDPOINTS;

fuzz_target!(|data: &[u8]| {
    if let Ok(card) = ContactCard::decode(data) {
        assert_eq!(card.encode(), data);
        assert!(card.endpoints().count() >= 1);
        assert!(card.endpoints().count() <= MAX_ACTIVE_ENDPOINTS);
        assert!(card.epoch().get() >= 1);
        assert!(
            card.identity()
                .verify(&card.signed_bytes(), card.signature())
                .is_ok()
        );
    }
});
