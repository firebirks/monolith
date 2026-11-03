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
    /// The person was unblocked already.
    NotBlocked,
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
    /// starts again. The runtime reports it with an event too, and the
    /// window says it once, for as long as it lasts: it is not said where
    /// it was met ([`Failure::said_in_place`]).
    Storage,
    /// Tor did not do what was asked.
    Tor,
    /// Monolith is shutting down.
    ShuttingDown,
    /// The file could not be read or written.
    File,
    /// The file is too large to be a card.
    FileTooLarge,
    /// The transfer is gone, or belongs to a contact that is gone.
    NoSuchTransfer,
    /// Files go to and come from accepted contacts only.
    NotAcceptedForFiles,
    /// The file to send is not a regular file of this computer that
    /// Monolith can read, or it changed while it was read.
    FileNotSendable,
    /// The file to send is larger than a transfer may be.
    FileTooLargeToSend,
    /// Too many files wait for an answer, or are being received.
    TooManyFiles,
    /// The received files staged take all the room they may.
    StagingFull,
    /// The list of transfers is full of ones that cannot go yet.
    TransfersFull,
    /// Another file is being prepared or saved.
    FileBusy,
    /// A file of that name exists where the file was to be saved.
    FileExists,
    /// A file cannot be saved there.
    SavePlaceRefused,
    /// The received file did not read back whole.
    ReceivedFileDamaged,
    /// The Tor settings are incomplete, or name a place off this computer.
    TorSettings,
    /// The settings file named when Monolith was started was refused: on
    /// Tails it is not in the unlocked Persistent Storage.
    SettingsNotKept,
    /// Something the app did not expect.
    Unexpected,
}

impl Failure {
    /// Whether the sentence is said where the failure was met: under a
    /// field, in a dialog, in a notice. A failed store is not; the window
    /// says it once, across its top, until Monolith starts again.
    pub const fn said_in_place(self) -> bool {
        !matches!(self, Self::Storage)
    }

    /// The sentence the user reads.
    pub const fn message(self) -> &'static str {
        match self {
            Self::WrongPassphrase => "That passphrase does not open this profile.",
            // The fields take at most `present::MAX_PASSPHRASE_CHARS`.
            Self::UnusablePassphrase => {
                "Choose a passphrase that is not empty and has at most 256 characters."
            }
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
                "The saved messages belong to a different profile. Nothing was changed. Keep \
                 the profile folder; to start a new profile, move the folder elsewhere and start \
                 Monolith again."
            }
            Self::Disk => {
                "The disk could not be read or written. Check that it is connected and has free \
                 space, then try again."
            }
            Self::InvalidText => "This text contains characters that cannot be used here.",
            Self::TooLong => "This message is too long.",
            Self::NoSuchIdentity => "This identity no longer exists.",
            Self::NoSuchContact => "This contact no longer exists.",
            Self::NoSuchRequest => "This request is no longer waiting.",
            Self::NoSuchInvitation => "This invitation is no longer active.",
            Self::NotBlocked => "This person is no longer blocked.",
            Self::NotAContact => "Messages cannot be sent to this contact.",
            Self::InvalidCard => "This is not a valid Monolith card.",
            Self::OwnCard => "This is the card of one of your own identities.",
            Self::Blocked => {
                "You blocked this person. Unblock them in Settings\u{a0}>\u{a0}Identities first."
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
            Self::Storage => "Monolith can no longer save. Restart Monolith to continue.",
            Self::Tor => {
                "Monolith could not reach Tor. Open Tor Browser and keep it open, or start the \
                 Tor service, then try again."
            }
            Self::ShuttingDown => "Monolith is closing.",
            Self::File => "The file could not be read or written.",
            Self::FileTooLarge => "This file is too large to be a Monolith card.",
            Self::NoSuchTransfer => "This file transfer is no longer available.",
            Self::NotAcceptedForFiles => "Files can be sent once this contact has accepted you.",
            Self::FileNotSendable => {
                "This file cannot be sent. Choose a file on this computer, not a folder, a \
                 link or a file kept only online."
            }
            Self::FileTooLargeToSend => {
                "This file is larger than 256\u{a0}MiB, the most Monolith sends."
            }
            Self::TooManyFiles => {
                "Too many files are waiting or being received. Try again when one is done."
            }
            Self::StagingFull => {
                "Received files take all the room Monolith keeps for them. Save or remove some \
                 first."
            }
            Self::TransfersFull => {
                "The list of file transfers is full. Remove finished transfers first."
            }
            Self::FileBusy => "Another file is being prepared or saved. Try again when it is done.",
            Self::FileExists => "A file with this name is already there. Choose another name.",
            Self::SavePlaceRefused => {
                "Files cannot be saved there. Choose a folder on this computer and a file name."
            }
            Self::ReceivedFileDamaged => {
                "The received file no longer reads back as it arrived, so it cannot be saved."
            }
            Self::SettingsNotKept => {
                "The settings file Monolith was started with is not in the unlocked Persistent \
                 Storage, so settings last until Monolith closes."
            }
            Self::TorSettings => {
                "These Tor settings cannot be used. Tor must be on this computer, and the \
                 cookie file needs its full path."
            }
            Self::Unexpected => {
                "Something went wrong. Try again; if it happens again, restart Monolith."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_that_stops_the_user_says_what_to_do_next() {
        assert!(Failure::Disk.message().contains("then try again"));
        assert!(
            Failure::MessageStoreMismatch
                .message()
                .contains("move the folder elsewhere and start Monolith again")
        );
        assert!(Failure::Unexpected.message().contains("Try again"));
        // The limit is the one of the fields.
        let limit = format!(
            "at most {} characters",
            crate::present::MAX_PASSPHRASE_CHARS
        );
        assert!(Failure::UnusablePassphrase.message().contains(&limit));
    }

    #[test]
    fn a_failed_store_has_the_words_of_the_window_and_is_said_by_it_alone() {
        assert_eq!(
            Failure::Storage.message(),
            "Monolith can no longer save. Restart Monolith to continue."
        );
        assert!(!Failure::Storage.said_in_place());
        assert!(Failure::StoreFull.said_in_place());
    }
}
