//! The design system of the Monolith desktop client
//! (`docs/adr/0012-design-system.md`): the tokens of [`theme`], the
//! [`fonts`] and [`icons`], and the components the screens are built from.
//!
//! This crate depends on egui only. It knows nothing of Monolith's state;
//! it draws what it is given and reports what the user did.

pub mod button;
pub mod fonts;
pub mod icons;
pub mod interact;
pub mod spinner;
pub mod text_field;
pub mod theme;

pub use button::Button;
pub use icons::Icon;
pub use text_field::TextField;
pub use theme::Theme;
