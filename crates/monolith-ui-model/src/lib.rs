//! The state of the Monolith desktop client
//! (`docs/adr/0011-ui-security-boundary.md`): what the runtime said, kept
//! per local identity; what the user is doing; and the commands and
//! updates between the model and the bridge to the runtime.
//!
//! This crate depends on no Monolith crate and on no GUI toolkit. It names
//! everything by handles of its own ([`ids`]), holds no object of the
//! runtime, and changes what it shows of the runtime's state only with the
//! answers of the runtime's queries.

pub mod data;
pub mod failure;
pub mod ids;
pub mod model;
pub mod present;
pub mod protocol;

pub use data::{
    BlockedView, ContactView, ConversationView, Deferral, IdentityView, InvitationView,
    MessageView, RequestMode, RequestView, RuntimeEvent, Service, Status, TorCheck,
};
pub use failure::Failure;
pub use ids::{UiBlockedId, UiContactId, UiIdentityId, UiInvitationId, UiMessageId, UiRequestId};
pub use model::{
    Dialog, Model, Notice, NoticeKind, Phase, RowState, Selection, SidebarRow, Slice, Thread,
    TorIndicator, Waiting,
};
pub use protocol::{Command, CommandId, IdentitySnapshot, Outcome, ProfileOnDisk, Update};
