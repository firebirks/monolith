//! The design system of the Monolith desktop client
//! (`docs/adr/0012-design-system.md`): the tokens of [`theme`], the
//! [`fonts`] and [`icons`], and the components the screens are built from.
//!
//! This crate depends on egui only. It knows nothing of Monolith's state;
//! it draws what it is given and reports what the user did.

pub mod avatar;
pub mod badge;
pub mod banner;
pub mod bubble;
pub mod button;
pub mod checkbox;
pub mod chrome;
pub mod composer;
pub mod divider;
pub mod empty_state;
pub mod fingerprint;
pub mod fonts;
pub mod icon_button;
pub mod icons;
pub mod identity_switcher;
pub mod interact;
pub mod invitation_row;
pub mod line;
pub mod mark;
pub mod menu;
pub mod message_status;
pub mod modal;
pub mod qr;
pub mod request_card;
pub mod rows;
pub mod search_field;
pub mod section_header;
pub mod segmented;
pub mod settings_row;
pub mod spinner;
pub mod text;
pub mod text_field;
pub mod their_words;
pub mod theme;
pub mod toast;
pub mod toggle;
pub mod tooltip;
pub mod tor_status;

pub use button::Button;
pub use icons::Icon;
pub use line::Line;
pub use text_field::TextField;
pub use theme::Theme;
