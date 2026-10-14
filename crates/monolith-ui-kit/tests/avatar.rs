//! Avatars: initials, or a person for a contact with no name of its own.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

mod common;

use common::{Rig, node, paints, texts};
use egui::Id;
use monolith_ui_kit::avatar::Avatar;
use monolith_ui_kit::icons;

#[test]
fn an_anonymous_avatar_draws_a_person_and_keeps_its_name_for_a_screen_reader() {
    let glyph = icons::USER.char().to_string();
    for anonymous in [false, true] {
        let mut rig = Rig::new();
        rig.ctx.enable_accesskit();
        let mut id = Id::NULL;
        let out = rig.settle(2, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                id = ui
                    .add(Avatar::new("Contact C7TQ", &[7]).anonymous(anonymous))
                    .id;
            });
        });
        assert_eq!(paints(&out, &glyph), anonymous);
        assert_eq!(texts(&out).iter().any(|text| text == "CC"), !anonymous);
        // egui gives a label's text as the value of its node.
        let node = node(&out, id).expect("the avatar has a node");
        assert_eq!(node.value(), Some("Contact C7TQ"));
    }
}

#[test]
fn an_avatar_shows_initials_unless_it_is_asked_not_to() {
    let mut rig = Rig::new();
    let out = rig.settle(2, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add(Avatar::new("Alice Martin", &[1]));
        });
    });
    assert!(texts(&out).iter().any(|text| text == "AM"));
}
