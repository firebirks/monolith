//! State storage.
//!
//! The storage design is in `docs/STORAGE.md` and
//! `docs/adr/0005-storage-encryption.md`. This crate stores typed records
//! handed to it by `monolith-core`. It never interprets peer input and never
//! changes protocol state on its own.
//!
//! Phase 0 defines the policy types only. No file format is implemented yet.

use core::fmt;

/// Whether identity and contact state survives the end of the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StateMode {
    /// State lives in memory and is gone when the process exits. This is a
    /// supported mode of operation, not a degraded one, and it is the default
    /// on Tails.
    Ephemeral,
    /// State is kept in an encrypted vault file chosen by the user.
    Persistent,
}

/// Whether chat history is kept. Independent of [`StateMode::Persistent`]:
/// keeping an identity does not imply keeping conversations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HistoryPolicy {
    /// Messages are held in memory for the running session only.
    None,
    /// Messages are kept in encrypted storage.
    Encrypted,
}

impl HistoryPolicy {
    /// Returns true if this history policy can be combined with `mode`.
    ///
    /// Encrypted history needs a persistent vault to hold its key.
    pub const fn is_valid_with(self, mode: StateMode) -> bool {
        !matches!((self, mode), (Self::Encrypted, StateMode::Ephemeral))
    }
}

/// Why a storage operation failed.
///
/// The variants carry no paths and no stored content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StorageError {
    /// No vault exists at the configured location.
    NotFound,
    /// Another process holds the vault lock.
    Locked,
    /// The passphrase is wrong or the vault was modified. The two cases are
    /// deliberately indistinguishable.
    AuthenticationFailed,
    /// The vault header is malformed or outside the accepted parameter range.
    BadFormat,
    /// The vault was written by a newer format version.
    UnsupportedVersion,
    /// The operating system reported an I/O failure.
    Io,
    /// The operation is not available in the current state mode.
    NotAvailable,
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotFound => "vault not found",
            Self::Locked => "vault is locked by another process",
            Self::AuthenticationFailed => "wrong passphrase or modified vault",
            Self::BadFormat => "vault format error",
            Self::UnsupportedVersion => "unsupported vault version",
            Self::Io => "I/O error",
            Self::NotAvailable => "not available in this storage mode",
        })
    }
}

impl core::error::Error for StorageError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_requires_a_persistent_vault() {
        assert!(HistoryPolicy::None.is_valid_with(StateMode::Ephemeral));
        assert!(HistoryPolicy::None.is_valid_with(StateMode::Persistent));
        assert!(HistoryPolicy::Encrypted.is_valid_with(StateMode::Persistent));
        assert!(!HistoryPolicy::Encrypted.is_valid_with(StateMode::Ephemeral));
    }
}
