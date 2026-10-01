//! Binary contact card decoder: arbitrary bytes either fail or decode to a
//! card that encodes back to exactly the same bytes, whose signature
//! verifies over its signed bytes, and whose three keys are distinct.

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
        // Key separation: no endpoint is the identity key, and the
        // transport key is neither of them in Montgomery form.
        let identity = card.identity().as_bytes();
        assert!(!card.transport().is_montgomery_form_of(identity));
        for endpoint in card.endpoints().iter() {
            assert_ne!(endpoint.as_bytes(), identity);
            assert!(!card.transport().is_montgomery_form_of(endpoint.as_bytes()));
        }
        assert!(
            card.identity()
                .verify(&card.signed_bytes(), card.signature())
                .is_ok()
        );
    }
});
