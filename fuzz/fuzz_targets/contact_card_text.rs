//! Text contact card parser: arbitrary text either fails or parses to a card
//! whose canonical text form parses to the same card.

#![no_main]

use libfuzzer_sys::fuzz_target;
use monolith_protocol::card::ContactCard;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = core::str::from_utf8(data) else {
        return;
    };
    if let Ok(card) = ContactCard::from_text(text) {
        let canonical = card.to_text();
        assert!(!canonical.chars().any(|c| c.is_ascii_lowercase()));
        assert_eq!(ContactCard::from_text(&canonical).as_ref(), Ok(&card));
    }
});
