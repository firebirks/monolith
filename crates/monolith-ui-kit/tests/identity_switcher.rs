//! The identity switcher: opening, choosing an entry, closing.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

mod common;

use common::{Rig, paints};
use egui::{Context, Id, Key, Rect, pos2};
use monolith_ui_kit::identity_switcher::{IdentityItem, IdentitySwitcher, is_open, set_open};

const ITEMS: [IdentityItem<'static, u32>; 3] = [
    IdentityItem {
        key: 10,
        name: "Personal",
        detail: "aaaa bbbb",
        seed: &[1],
    },
    IdentityItem {
        key: 20,
        name: "Work",
        detail: "cccc dddd",
        seed: &[2],
    },
    IdentityItem {
        key: 30,
        name: "Spare",
        detail: "eeee ffff",
        seed: &[3],
    },
];

/// What a frame of the switcher reported, summed over the frames.
struct Seen {
    trigger: Rect,
    selected: Vec<u32>,
    new_identity: bool,
    manage: bool,
}

impl Default for Seen {
    fn default() -> Self {
        Self {
            trigger: Rect::NOTHING,
            selected: Vec::new(),
            new_identity: false,
            manage: false,
        }
    }
}

fn show(ctx: &Context, current: u32, seen: &mut Seen) {
    egui::CentralPanel::default().show(ctx, |ui| {
        let out =
            IdentitySwitcher::new("sw", &current, &ITEMS, "New identity", "Manage identities")
                .caption("3 contacts")
                .show(ui);
        seen.trigger = out.response.rect;
        seen.selected.extend(out.selected);
        seen.new_identity |= out.new_identity;
        seen.manage |= out.manage;
    });
}

fn entry_id(kind: &str, key: impl core::hash::Hash) -> Id {
    let trigger = Id::new(("monolith-identity-switcher", "sw"));
    Id::new((trigger, kind, key))
}

fn closed() -> (Rig, Seen) {
    let mut rig = Rig::new();
    let mut seen = Seen::default();
    rig.settle(2, |ctx| show(ctx, 10, &mut seen));
    (rig, seen)
}

fn opened() -> (Rig, Seen) {
    let (mut rig, mut seen) = closed();
    let at = seen.trigger.center();
    rig.click(at, |ctx| show(ctx, 10, &mut seen));
    rig.settle(3, |ctx| show(ctx, 10, &mut seen));
    assert!(is_open(&rig.ctx, "sw"));
    (rig, seen)
}

fn rect_of(rig: &Rig, id: Id) -> Rect {
    rig.ctx.read_response(id).expect("shown in the popup").rect
}

#[test]
fn the_trigger_shows_the_current_identity_and_the_popup_is_closed() {
    let (mut rig, mut seen) = closed();
    let out = rig.frame(vec![], |ctx| show(ctx, 20, &mut seen));
    assert!(paints(&out, "Work"));
    assert!(paints(&out, "3 contacts"));
    assert!(!paints(&out, "Personal"));
    assert!(!is_open(&rig.ctx, "sw"));
}

#[test]
fn a_click_on_the_trigger_opens_the_list() {
    let (mut rig, mut seen) = opened();
    let out = rig.frame(vec![], |ctx| show(ctx, 10, &mut seen));
    for text in [
        "Personal",
        "Work",
        "Spare",
        "New identity",
        "Manage identities",
    ] {
        assert!(paints(&out, text), "{text}");
    }
}

#[test]
fn choosing_an_entry_returns_its_key_and_closes_the_list() {
    for item in &ITEMS {
        let (mut rig, mut seen) = opened();
        let at = rect_of(&rig, entry_id("item", item.key)).center();
        rig.click(at, |ctx| show(ctx, 10, &mut seen));
        assert_eq!(seen.selected, vec![item.key]);
        assert!(!is_open(&rig.ctx, "sw"));
        assert!(!seen.new_identity && !seen.manage);
    }
}

#[test]
fn choosing_the_current_identity_still_reports_its_key() {
    let (mut rig, mut seen) = opened();
    let at = rect_of(&rig, entry_id("item", 10_u32)).center();
    rig.click(at, |ctx| show(ctx, 10, &mut seen));
    assert_eq!(seen.selected, vec![10]);
}

#[test]
fn the_new_and_manage_entries_report_themselves_and_close_the_list() {
    let (mut rig, mut seen) = opened();
    let at = rect_of(&rig, entry_id("entry", "new")).center();
    rig.click(at, |ctx| show(ctx, 10, &mut seen));
    assert!(seen.new_identity && !seen.manage && seen.selected.is_empty());
    assert!(!is_open(&rig.ctx, "sw"));

    let (mut rig, mut seen) = opened();
    let at = rect_of(&rig, entry_id("entry", "manage")).center();
    rig.click(at, |ctx| show(ctx, 10, &mut seen));
    assert!(seen.manage && !seen.new_identity && seen.selected.is_empty());
    assert!(!is_open(&rig.ctx, "sw"));
}

#[test]
fn the_card_entry_is_shown_only_when_asked_and_reports_itself() {
    let (mut rig, mut seen) = opened();
    let out = rig.frame(vec![], |ctx| show(ctx, 10, &mut seen));
    assert!(!paints(&out, "Share my card"));

    let mut card = false;
    let mut trigger = Rect::NOTHING;
    let with_card = |ctx: &Context, card: &mut bool, trigger: &mut Rect| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let out =
                IdentitySwitcher::new("sw", &10_u32, &ITEMS, "New identity", "Manage identities")
                    .card_entry("Share my card")
                    .show(ui);
            *trigger = out.response.rect;
            *card |= out.card;
            assert!(out.selected.is_none() && !out.new_identity && !out.manage);
        });
    };
    let mut rig = Rig::new();
    rig.settle(2, |ctx| with_card(ctx, &mut card, &mut trigger));
    let at = trigger.center();
    rig.click(at, |ctx| with_card(ctx, &mut card, &mut trigger));
    let out = rig.frame(vec![], |ctx| with_card(ctx, &mut card, &mut trigger));
    assert!(paints(&out, "Share my card"));
    let at = rect_of(&rig, entry_id("entry", "card")).center();
    rig.click(at, |ctx| with_card(ctx, &mut card, &mut trigger));
    assert!(card);
    assert!(!is_open(&rig.ctx, "sw"));
}

#[test]
fn a_click_outside_closes_the_list_without_choosing() {
    let (mut rig, mut seen) = opened();
    rig.click(pos2(390.0, 590.0), |ctx| show(ctx, 10, &mut seen));
    assert!(!is_open(&rig.ctx, "sw"));
    assert!(seen.selected.is_empty() && !seen.new_identity && !seen.manage);
}

#[test]
fn escape_closes_the_list() {
    let (mut rig, mut seen) = opened();
    rig.key(Key::Escape, |ctx| show(ctx, 10, &mut seen));
    assert!(!is_open(&rig.ctx, "sw"));
}

#[test]
fn a_second_click_on_the_trigger_closes_the_list() {
    let (mut rig, mut seen) = opened();
    let at = seen.trigger.center();
    rig.click(at, |ctx| show(ctx, 10, &mut seen));
    assert!(!is_open(&rig.ctx, "sw"));
}

#[test]
fn set_open_opens_and_closes_the_list_from_outside() {
    let (mut rig, mut seen) = closed();
    set_open(&rig.ctx, "sw", true);
    rig.settle(3, |ctx| show(ctx, 10, &mut seen));
    let out = rig.frame(vec![], |ctx| show(ctx, 10, &mut seen));
    assert!(is_open(&rig.ctx, "sw"));
    assert!(paints(&out, "Spare"));
    set_open(&rig.ctx, "sw", false);
    let out = rig.frame(vec![], |ctx| show(ctx, 10, &mut seen));
    assert!(!is_open(&rig.ctx, "sw"));
    assert!(!paints(&out, "Spare"));
}
