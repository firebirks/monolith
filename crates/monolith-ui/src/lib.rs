//! The screens of the Monolith desktop client
//! (`docs/adr/0011-ui-security-boundary.md`, `docs/adr/0012-design-system.md`).
//!
//! This crate draws the UI model ([`monolith_ui_model`]) with the
//! components of the design system ([`monolith_ui_kit`]) and turns what
//! the user does into intents of the model. It depends on no Monolith
//! crate and on no toolkit beyond egui; it opens no file, socket or
//! process: what needs one is an [`app::Effect`] the desktop carries out.
//! Screens use components of the design system only, and no colour, size
//! or font of their own (`tests/screens.rs` checks the source).

pub mod app;
mod common;
mod conversation;
mod dialogs;
#[cfg(feature = "fixtures")]
pub mod fixtures;
mod message_list;
mod onboarding;
mod requests;
mod settings;
mod shell;
mod sidebar;

pub use app::{
    App, CloseWarning, Effect, EffectResult, Preferences, SettingsPage, ThemeChoice, TorPreset,
    TorSettings, View, close_warning,
};
pub use shell::show;
