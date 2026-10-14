//! What can go wrong, in words for the user. The bridge maps every error
//! of the runtime to one of these; nothing else reaches the screen, and no
//! `Debug` or `Display` of an error is ever shown.

/// A failure, as the user is told of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Failure {
    /// The passphrase is wrong, or the files were changed.
    WrongPassphrase,
    /// The passphrase cannot be used (empty or too long).
    UnusablePassphrase,
    /// Another Monolith uses the data directory.
    InUse,
    /// The data directory is open to other users, or not the user's.
    UnsafePermissions,
    /// An interrupted write needs a decision outside the app.
    RecoveryNeeded,
    /// The stored data does not validate.
    Damaged,
    /// The message store belongs to another vault.
    MessageStoreMismatch,
    /// The disk failed.
    Disk,
    /// The text breaks the rules of a message or a name.
    InvalidText,
    /// The message is longer than a message may be.
    TooLong,
    /// The identity is gone.
    NoSuchIdentity,
    /// The contact is gone, or was made again.
    NoSuchContact,
    /// The request was answered or dropped.
    NoSuchRequest,
    /// The invitation was revoked.
    NoSuchInvitation,
    /// The record is not a contact that can receive messages.
    NotAContact,
    /// The card does not parse or does not verify.
    InvalidCard,
    /// The card is one of the user's own identities.
    OwnCard,
    /// The identity is blocked.
    Blocked,
    /// A limit is reached (identities, invitations, contacts).
    Full,
    /// Too many messages wait to be delivered.
    QueueFull,
    /// The message store is full.
    StoreFull,
    /// Something could not be saved. Nothing more is saved until Monolith
    /// starts again.
    Storage,
    /// Tor did not do what was asked.
    Tor,
    /// Monolith is shutting down.
    ShuttingDown,
    /// The file could not be read or written.
    File,
    /// The file is too large to be a card.
    FileTooLarge,
    /// The Tor settings are incomplete, or name a place off this computer.
    TorSettings,
    /// Something the app did not expect.
    Unexpected,
}

impl Failure {
    /// The sentence the user reads.
    pub const fn message(self) -> &'static str {
        match self {
            Self::WrongPassphrase => "That passphrase does not open this profile.",
            Self::UnusablePassphrase => "Choose a passphrase that is not empty and not too long.",
            Self::InUse => "Monolith is already open with this profile.",
            Self::UnsafePermissions => {
                "The profile folder can be read by other users. Monolith will not open it. \
                 Make the folder readable only by you, then start Monolith again."
            }
            Self::RecoveryNeeded => {
                "A save was interrupted, and Monolith cannot tell which copy of the profile to \
                 keep. Nothing was changed, and Monolith does not repair it. Keep the profile \
                 folder; to start a new profile, move the folder elsewhere and start Monolith \
                 again."
            }
            Self::Damaged => {
                "The profile is damaged and cannot be opened. Nothing was changed, and Monolith \
                 does not repair it. Keep the profile folder; to start a new profile, move the \
                 folder elsewhere and start Monolith again."
            }
            Self::MessageStoreMismatch => {
                "The saved messages belong to a different profile. Nothing was changed."
            }
            Self::Disk => "The disk could not be read or written.",
            Self::InvalidText => "This text contains characters that cannot be used here.",
            Self::TooLong => "This message is too long.",
            Self::NoSuchIdentity => "This identity no longer exists.",
            Self::NoSuchContact => "This contact no longer exists.",
            Self::NoSuchRequest => "This request is no longer waiting.",
            Self::NoSuchInvitation => "This invitation is no longer active.",
            Self::NotAContact => "Messages cannot be sent to this contact.",
            Self::InvalidCard => "This is not a valid Monolith card.",
            Self::OwnCard => "This is the card of one of your own identities.",
            Self::Blocked => {
                "You blocked this person. Unblock them in Settings > Identities first."
            }
            Self::Full => {
                "A limit is reached (for example 16 active invitations). Remove one first."
            }
            Self::QueueFull => {
                "Too many messages are waiting to be delivered to this contact. Wait until some \
                 are delivered."
            }
            Self::StoreFull => {
                "There is no more room for messages. Delete older conversations to make room."
            }
            Self::Storage => "Monolith could not save. Restart Monolith to continue.",
            Self::Tor => {
                "Monolith could not reach Tor. Open Tor Browser and keep it open, or start the \
                 Tor service, then try again."
            }
            Self::ShuttingDown => "Monolith is closing.",
            Self::File => "The file could not be read or written.",
            Self::FileTooLarge => "This file is too large to be a Monolith card.",
            Self::TorSettings => {
                "These Tor settings cannot be used. Tor must be on this computer, and the \
                 cookie file needs its full path."
            }
            Self::Unexpected => "Something went wrong.",
        }
    }
}
