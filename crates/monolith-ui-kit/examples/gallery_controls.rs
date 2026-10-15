//! Renders sheets of the small controls in every state, light and dark, to
//! PNG files:
//! `cargo run -p monolith-ui-kit --example gallery_controls -- <directory>`.
//!
//! A sheet that calls [`Probe::hot`] is rendered a second time with the
//! pointer over that widget, so that hover (and the tooltip) shows.

#![allow(clippy::print_stderr, clippy::expect_used)]

use std::path::{Path, PathBuf};

use egui::{Context, Pos2, RichText, Ui, Vec2};
use monolith_ui_kit::badge::{Badge, Pill, Tone};
use monolith_ui_kit::checkbox::Checkbox;
use monolith_ui_kit::composer::Composer;
use monolith_ui_kit::divider::Divider;
use monolith_ui_kit::icon_button::IconButton;
use monolith_ui_kit::search_field::SearchField;
use monolith_ui_kit::section_header::SectionHeader;
use monolith_ui_kit::segmented::Segmented;
use monolith_ui_kit::toggle::Toggle;
use monolith_ui_kit::{Theme, fonts, icons, interact, theme, tooltip};
use monolith_ui_snapshot::{Options, render};

/// Where the pointer goes in the second render of a sheet.
#[derive(Default)]
struct Probe {
    hot: Option<Pos2>,
}

impl Probe {
    fn hot(&mut self, response: &egui::Response) {
        self.hot = Some(response.rect.center());
    }
}

fn sheet(name: &str, size: Vec2, dir: &Path, mut body: impl FnMut(&mut Ui, &mut Probe)) {
    for (suffix, theme) in [("light", Theme::light()), ("dark", Theme::dark())] {
        let background = theme.color.bg_surface;
        let mut options = Options {
            size,
            background,
            ..Options::default()
        };
        let mut run = |options: &Options, probe: &mut Probe| {
            // A fresh context per render: the renderer keeps no textures
            // between calls, and egui sends the font atlas only once.
            let ctx = Context::default();
            fonts::install(&ctx, &[]);
            theme::install(&ctx, theme.clone().with_reduced_motion(true));
            // Focus set by hand shows its ring, as if Tab had moved it.
            interact::set_focus_visible(&ctx, true);
            render(&ctx, options, |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(background).inner_margin(24))
                    .show(ctx, |ui| body(ui, probe));
            })
        };
        let mut probe = Probe::default();
        let mut image = run(&options, &mut probe);
        if let Some(hot) = probe.hot {
            // Moved once, in the first frame, so that the pointer counts as still
            // and tooltips show.
            options.events = vec![egui::Event::PointerMoved(hot)];
            image = run(&options, &mut Probe::default());
        }
        let path = dir.join(format!("{name}-{suffix}.png"));
        image.save_png(&path).expect("write the PNG");
        eprintln!("{}", path.display());
    }
}

fn caption(ui: &mut Ui, text: &str) {
    let theme = theme::current(ui.ctx());
    ui.label(
        RichText::new(text)
            .font(theme.text.micro.font_id())
            .color(theme.color.text_tertiary),
    );
}

fn row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        add(ui);
    });
}

fn icon_buttons(ui: &mut Ui, probe: &mut Probe) {
    caption(
        ui,
        "Ghost: normal, hovered, selected, focused, disabled, busy (medium, small, tiny)",
    );
    for (n, build) in [
        (|i, l| IconButton::new(i, l)) as fn(_, _) -> IconButton<'static>,
        |i, l| IconButton::new(i, l).small(),
        |i, l| IconButton::new(i, l).tiny(),
    ]
    .into_iter()
    .enumerate()
    {
        row(ui, |ui| {
            ui.add(build(icons::SETTINGS, "Settings").id_salt(("g", n, 0)));
            ui.add(
                build(icons::SEARCH, "Search")
                    .selected(true)
                    .id_salt(("g", n, 1)),
            );
            let focused = ui.add(build(icons::COPY, "Copy").id_salt(("g", n, 2)));
            if n == 0 {
                focused.request_focus();
            }
            ui.add(
                build(icons::TRASH_2, "Delete")
                    .enabled(false)
                    .id_salt(("g", n, 3)),
            );
            ui.add(
                build(icons::REFRESH_CW, "Refresh")
                    .busy(true)
                    .id_salt(("g", n, 4)),
            );
            ui.add(
                build(icons::TRASH_2, "Delete")
                    .enabled(false)
                    .selected(true)
                    .id_salt(("g", n, 5)),
            );
        });
    }
    ui.add_space(8.0);
    caption(ui, "Filled and danger");
    row(ui, |ui| {
        ui.add(IconButton::filled(icons::SEND, "Send").id_salt("f0"));
        ui.add(
            IconButton::filled(icons::SEND, "Send")
                .small()
                .id_salt("f1"),
        );
        ui.add(
            IconButton::filled(icons::SEND, "Send")
                .enabled(false)
                .id_salt("f2"),
        );
        ui.add(
            IconButton::filled(icons::SEND, "Send")
                .busy(true)
                .id_salt("f3"),
        );
        ui.add(IconButton::danger(icons::TRASH_2, "Delete").id_salt("d0"));
        ui.add(
            IconButton::danger(icons::BAN, "Block")
                .small()
                .id_salt("d1"),
        );
        ui.add(
            IconButton::danger(icons::TRASH_2, "Delete")
                .enabled(false)
                .id_salt("d2"),
        );
        ui.add(
            IconButton::danger(icons::TRASH_2, "Delete")
                .selected(true)
                .id_salt("d3"),
        );
    });
    ui.add_space(8.0);
    caption(ui, "Hovered (with its tooltip)");
    row(ui, |ui| {
        ui.add(IconButton::new(icons::SETTINGS, "Settings").id_salt("h0"));
        let hot = ui.add(IconButton::new(icons::USER_PLUS, "Add contact").id_salt("h1"));
        probe.hot(&hot);
        ui.add(IconButton::new(icons::QR_CODE, "Show QR code").id_salt("h2"));
    });
    ui.add_space(40.0);
    caption(ui, "The tooltip, in place");
    tooltip::paint_inline(ui, "Add contact");
}

fn search(ui: &mut Ui, probe: &mut Probe) {
    let mut empty = String::new();
    let mut typed = "Alice".to_owned();
    let mut focused = "Ali".to_owned();
    let mut compact = String::new();
    let mut compact_typed = "Bob at work".to_owned();
    let mut off = String::new();
    let mut hovered = String::new();
    ui.set_max_width(360.0);
    caption(ui, "Empty, with text, focused with text");
    SearchField::new("a", &mut empty).show(ui);
    SearchField::new("b", &mut typed).show(ui);
    SearchField::new("c", &mut focused)
        .request_focus(true)
        .show(ui);
    ui.add_space(8.0);
    caption(ui, "Compact (sidebar): empty, with text");
    SearchField::new("d", &mut compact).compact(true).show(ui);
    SearchField::new("e", &mut compact_typed)
        .compact(true)
        .show(ui);
    ui.add_space(8.0);
    caption(ui, "Disabled, hovered");
    SearchField::new("f", &mut off).enabled(false).show(ui);
    let out = SearchField::new("g", &mut hovered).show(ui);
    probe.hot(&out.response);
}

const DRAFT_LONG: &str = "Notes for tomorrow:\n- Move the call to 10:30\n- Send the signed copy back before noon\n- Ask Dana about the venue and the budget\n- Check that the new device is paired\n- Bring the printed list\n- Do not forget the charger\n- Confirm the dinner\n- Book the train\n- Water the plants\n- Call the landlord about the heating, again, and keep a record of when it was reported\n- Send the photos";

fn composer(ui: &mut Ui, probe: &mut Probe) {
    // The composer is focused by the conversation, not by Tab: a quiet
    // border and no ring.
    interact::set_focus_visible(ui.ctx(), false);
    ui.set_max_width(560.0);
    let mut empty = String::new();
    let mut short = "On my way, see you in ten minutes".to_owned();
    let mut lines =
        "First line\nSecond line, which is a little longer than the first\nThird".to_owned();
    let mut long = DRAFT_LONG.to_owned();
    let mut near = "x".repeat(3600);
    let mut over = "y".repeat(4120);
    let mut blocked = "I wanted to say something".to_owned();
    let mut busy = "Sending this one".to_owned();
    let mut focused = String::new();
    let mut hovered = String::new();
    let mut unsendable = "Waiting for the connection".to_owned();

    caption(
        ui,
        "Empty, focused by the conversation (no ring), hovered empty",
    );
    Composer::new("a", &mut empty).max_bytes(4000).show(ui);
    ui.add_space(8.0);
    Composer::new("b", &mut focused)
        .max_bytes(4000)
        .request_focus(true)
        .show(ui);
    ui.add_space(8.0);
    let out = Composer::new("c", &mut hovered).max_bytes(4000).show(ui);
    probe.hot(&out.response);
    ui.add_space(12.0);
    caption(ui, "A short draft, three lines");
    Composer::new("d", &mut short).max_bytes(4000).show(ui);
    ui.add_space(8.0);
    Composer::new("e", &mut lines).max_bytes(4000).show(ui);
    ui.add_space(12.0);
    caption(ui, "Long draft (grown to the limit, scrolling)");
    Composer::new("f", &mut long).max_bytes(4000).show(ui);
    ui.add_space(12.0);
    caption(ui, "Near the limit, over the limit");
    Composer::new("g", &mut near).max_bytes(4000).show(ui);
    ui.add_space(8.0);
    Composer::new("h", &mut over).max_bytes(4000).show(ui);
    ui.add_space(12.0);
    caption(ui, "Blocked, busy, not sendable without a reason");
    Composer::new("i", &mut blocked)
        .max_bytes(4000)
        .sendable(false)
        .blocked_reason(Some("This contact is blocked"))
        .show(ui);
    ui.add_space(8.0);
    Composer::new("j", &mut busy)
        .max_bytes(4000)
        .busy(true)
        .show(ui);
    ui.add_space(8.0);
    Composer::new("k", &mut unsendable)
        .max_bytes(4000)
        .sendable(false)
        .show(ui);
}

fn toggles(ui: &mut Ui, probe: &mut Probe) {
    ui.set_max_width(420.0);
    let (mut on, mut off) = (true, false);
    caption(ui, "On, off, focused, hovered, disabled on, disabled off");
    row(ui, |ui| {
        ui.add(Toggle::new(&mut on).name("On").id_salt(1));
        ui.add(Toggle::new(&mut off).name("Off").id_salt(2));
        let mut f = true;
        let r = ui.add(Toggle::new(&mut f).name("Focused").id_salt(3));
        r.request_focus();
        let mut h = false;
        let r = ui.add(Toggle::new(&mut h).name("Hovered").id_salt(4));
        probe.hot(&r);
        let mut d1 = true;
        ui.add(
            Toggle::new(&mut d1)
                .name("Disabled")
                .enabled(false)
                .id_salt(5),
        );
        let mut d2 = false;
        ui.add(
            Toggle::new(&mut d2)
                .name("Disabled")
                .enabled(false)
                .id_salt(6),
        );
    });
    ui.add_space(12.0);
    caption(ui, "With labels");
    let (mut a, mut b, mut c, mut d) = (true, false, true, false);
    ui.add(
        Toggle::new(&mut a)
            .label("Notifications")
            .full_width(true)
            .id_salt(7),
    );
    ui.add(
        Toggle::new(&mut b)
            .label("Show previews")
            .full_width(true)
            .id_salt(8),
    );
    ui.add(
        Toggle::new(&mut c)
            .label("Locked")
            .enabled(false)
            .full_width(true)
            .id_salt(9),
    );
    ui.add(
        Toggle::new(&mut d)
            .label("Locked")
            .enabled(false)
            .full_width(true)
            .id_salt(10),
    );
    ui.add(Toggle::new(&mut a).label("Natural width").id_salt(11));
}

fn segmented(ui: &mut Ui, probe: &mut Probe) {
    #[derive(Clone, Copy, PartialEq, Eq, Hash)]
    enum Mode {
        System,
        Light,
        Dark,
    }
    let options = [
        (Mode::System, "System"),
        (Mode::Light, "Light"),
        (Mode::Dark, "Dark"),
    ];
    ui.set_max_width(420.0);
    caption(ui, "Each selection");
    for (n, start) in [Mode::System, Mode::Light, Mode::Dark]
        .into_iter()
        .enumerate()
    {
        let mut value = start;
        ui.add(Segmented::new(("s", n), &mut value, &options));
    }
    ui.add_space(8.0);
    caption(ui, "Disabled, full width");
    let mut value = Mode::Light;
    ui.add(Segmented::new("d", &mut value, &options).enabled(false));
    let mut value = Mode::Dark;
    ui.add(Segmented::new("w", &mut value, &options).full_width(true));
    ui.add_space(8.0);
    caption(ui, "Focused segment, hovered segment");
    let mut value = Mode::System;
    ui.add(Segmented::new("f", &mut value, &options));
    // A segment is keyed by its option value under the control's id.
    let segment = egui::Id::new(("monolith-segmented", "f")).with(("segment", Mode::Light));
    ui.memory_mut(|memory| memory.request_focus(segment));
    let mut value = Mode::System;
    let hot = ui.add(Segmented::new("h", &mut value, &options));
    // The pointer lands on the last segment of this one.
    probe.hot = Some(Pos2::new(hot.rect.right() - 30.0, hot.rect.center().y));
}

fn checkboxes(ui: &mut Ui, probe: &mut Probe) {
    ui.set_max_width(360.0);
    caption(ui, "Unchecked, checked, focused, hovered, disabled, error");
    let (mut a, mut b, mut c, mut d, mut e, mut f, mut g) =
        (false, true, false, false, false, true, false);
    ui.add(Checkbox::new(&mut a, "Remember this device").id_salt(1));
    ui.add(Checkbox::new(&mut b, "Remember this device").id_salt(2));
    let r = ui.add(Checkbox::new(&mut c, "Focused").id_salt(3));
    r.request_focus();
    let r = ui.add(Checkbox::new(&mut d, "Hovered").id_salt(4));
    probe.hot(&r);
    ui.add(Checkbox::new(&mut e, "Disabled").enabled(false).id_salt(5));
    ui.add(
        Checkbox::new(&mut f, "Disabled and checked")
            .enabled(false)
            .id_salt(6),
    );
    ui.add(
        Checkbox::new(&mut g, "I understand that this cannot be undone")
            .error(true)
            .id_salt(7),
    );
    ui.add_space(8.0);
    caption(ui, "A label that wraps");
    let mut w = true;
    ui.add(
        Checkbox::new(
            &mut w,
            "Keep the history of this conversation on this device, even after the contact is removed from the list",
        )
        .id_salt(8),
    );
}

fn badges(ui: &mut Ui, _probe: &mut Probe) {
    caption(ui, "Badges: 1, 3, 42, 99, 120, muted");
    row(ui, |ui| {
        for count in [1_u32, 3, 42, 99, 120] {
            ui.add(Badge::new(count));
        }
        ui.add(Badge::new(7).muted(true));
        ui.add(Badge::new(120).muted(true));
    });
    ui.add_space(12.0);
    caption(ui, "Pills");
    row(ui, |ui| {
        ui.add(Pill::new(Tone::Neutral, "Former contact"));
        ui.add(Pill::new(Tone::Accent, "New"));
        ui.add(Pill::new(Tone::Success, "Connected").dot(true));
        ui.add(Pill::new(Tone::Warning, "Connecting").dot(true));
        ui.add(Pill::new(Tone::Danger, "Blocked").icon(icons::BAN));
    });
    row(ui, |ui| {
        ui.add(Pill::new(Tone::Neutral, "Verified").icon(icons::BADGE_CHECK));
        ui.add(Pill::new(Tone::Accent, "Pending").icon(icons::HOURGLASS));
        ui.add(Pill::new(Tone::Success, "Delivered").icon(icons::CHECK));
        ui.add(Pill::new(Tone::Warning, "Not verified").icon(icons::TRIANGLE_ALERT));
        ui.add(Pill::new(Tone::Danger, "Failed").dot(true));
    });
    ui.add_space(12.0);
    caption(ui, "In context");
    row(ui, |ui| {
        let theme = theme::current(ui.ctx());
        ui.label(
            RichText::new("Alice Rossi")
                .font(theme.text.body_strong.font_id())
                .color(theme.color.text),
        );
        ui.add(Badge::new(3));
        // Monolith never says whether a contact is online.
        ui.add(Pill::new(Tone::Neutral, "Verified").icon(icons::BADGE_CHECK));
    });
}

fn dividers(ui: &mut Ui, probe: &mut Probe) {
    ui.set_max_width(420.0);
    caption(ui, "Divider, inset, labelled");
    ui.add(Divider::horizontal());
    ui.add(Divider::horizontal().inset(16.0));
    ui.add(Divider::labelled("or"));
    ui.add(Divider::labelled("or use a code").inset(24.0));
    ui.horizontal(|ui| {
        ui.label("Left");
        ui.add(Divider::vertical().length(20.0));
        ui.label("Middle");
        ui.add(Divider::vertical().length(20.0).inset(4.0));
        ui.label("Right");
    });
    ui.add_space(12.0);
    caption(ui, "Section headers");
    SectionHeader::new("Requests").count(2).show(ui);
    SectionHeader::new("Invitations")
        .action(icons::PLUS, "New invitation", "inv")
        .show(ui);
    SectionHeader::new("Conversations").show(ui);
    let out = SectionHeader::new("Contacts")
        .count(14)
        .action(icons::USER_PLUS, "Add contact", "contacts")
        .show(ui);
    let _ = (probe, out);
}

fn main() {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "gallery-controls".to_owned()),
    );
    sheet("icon-buttons", Vec2::new(560.0, 520.0), &dir, icon_buttons);
    sheet("search", Vec2::new(420.0, 400.0), &dir, search);
    sheet("composer", Vec2::new(620.0, 1320.0), &dir, composer);
    sheet("toggles", Vec2::new(480.0, 340.0), &dir, toggles);
    sheet("segmented", Vec2::new(480.0, 380.0), &dir, segmented);
    sheet("checkboxes", Vec2::new(420.0, 400.0), &dir, checkboxes);
    sheet("badges", Vec2::new(520.0, 300.0), &dir, badges);
    sheet("dividers", Vec2::new(480.0, 440.0), &dir, dividers);
}
