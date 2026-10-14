//! The boundary between a front end and Monolith
//! (`docs/adr/0009-session-manager.md`, `docs/ARCHITECTURE.md` section 4).
//!
//! A front end depends on this crate alone. It sends commands and runs
//! queries through a [`Messenger`] and follows its [`Events`]. Everything
//! that crosses the boundary is a plain value of [`api`]: opaque
//! identifiers, display data, states, local times and message text. No
//! local identity, installation, link, session, key, capability or handle
//! of the contact store crosses it, so a front end cannot act with an
//! identity's authority or around the rules of the core: the dependency
//! itself keeps them out of reach.

use std::path::Path;

pub use monolith_core::api;
use monolith_core::api::{
    CommandError, ContactId, ContactSummary, ConversationSummary, Event, IdentityId,
    IdentitySummary, InvitationRef, InvitationSummary, MessageRef, MessageView, RequestMode,
    RequestRef, RequestSummary, SendError, ServiceState, TorCheck,
};
use monolith_core::budget::Budgets;
use monolith_core::identity::{Installation, InstallationError};
use monolith_core::runtime::{Runtime, RuntimeOptions};
use monolith_storage::StorageError;
use monolith_storage::dir::DiskDir;
use monolith_storage::vault::{KdfParams, Passphrase};
pub use monolith_tor::{SystemTorBackend, SystemTorConfig, TorBackend};
use tokio::sync::broadcast;

use core::fmt;
use core::time::Duration;

/// Why a data directory could not be opened or created.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OpenError {
    /// The passphrase is wrong, or the vault or the message store was
    /// changed. The two cannot be told apart.
    AuthenticationFailed,
    /// The passphrase is empty or too long.
    BadPassphrase,
    /// Another process uses the data directory.
    Locked,
    /// No vault in the directory.
    NotFound,
    /// A vault exists already where one was to be created.
    Exists,
    /// The directory or a file in it is open to others, a link, or not the
    /// user's.
    Permissions,
    /// A write was interrupted and the user has to decide which file is the
    /// right one (`docs/STORAGE.md` section 3.5).
    RecoveryNeeded,
    /// The vault or the message store holds something that does not
    /// validate. Nothing was repaired.
    Invalid,
    /// The message store beside the vault holds the messages of another
    /// vault: the vault is one from before Phase 5 put back beside a newer
    /// store. Nothing was changed; the user decides which to keep.
    MessageStore,
    /// The disk failed.
    Io,
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::AuthenticationFailed => "wrong passphrase or changed files",
            Self::BadPassphrase => "unusable passphrase",
            Self::Locked => "the data directory is in use",
            Self::NotFound => "no vault in the data directory",
            Self::Exists => "a vault exists already",
            Self::Permissions => "unsafe permissions on the data directory",
            Self::RecoveryNeeded => "an interrupted write needs a decision",
            Self::Invalid => "the stored state does not validate",
            Self::MessageStore => "the message store belongs to another vault",
            Self::Io => "disk failure",
        })
    }
}

impl core::error::Error for OpenError {}

fn storage(error: StorageError) -> OpenError {
    match error {
        StorageError::AuthenticationFailed => OpenError::AuthenticationFailed,
        StorageError::BadPassphrase => OpenError::BadPassphrase,
        StorageError::Locked => OpenError::Locked,
        StorageError::NotFound => OpenError::NotFound,
        StorageError::Exists => OpenError::Exists,
        StorageError::Permissions => OpenError::Permissions,
        StorageError::RecoveryNeeded => OpenError::RecoveryNeeded,
        StorageError::Io => OpenError::Io,
        _ => OpenError::Invalid,
    }
}

fn installation(error: InstallationError) -> OpenError {
    match error {
        InstallationError::Storage(error) => storage(error),
        InstallationError::MessageStore => OpenError::MessageStore,
        _ => OpenError::Invalid,
    }
}

/// Events, in the order they happened.
#[derive(Debug)]
pub struct Events(broadcast::Receiver<Event>);

/// What a front end that fell behind is told.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Missed {
    /// This many events were dropped because the front end did not keep
    /// up. It reads the state again with the queries.
    Lagged(u64),
    /// The messenger was shut down.
    Closed,
}

impl Events {
    /// The next event.
    pub async fn next(&mut self) -> Result<Event, Missed> {
        self.0.recv().await.map_err(|error| match error {
            broadcast::error::RecvError::Lagged(count) => Missed::Lagged(count),
            broadcast::error::RecvError::Closed => Missed::Closed,
        })
    }
}

/// Monolith as a front end sees it. See the crate documentation.
pub struct Messenger<B: TorBackend + 'static = SystemTorBackend> {
    runtime: Runtime<B>,
}

impl<B: TorBackend + 'static> fmt::Debug for Messenger<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Messenger").finish_non_exhaustive()
    }
}

impl<B: TorBackend + 'static> Messenger<B> {
    fn start(installation: Installation, backend: B) -> Self {
        Self {
            runtime: Runtime::start(
                installation,
                backend,
                Budgets::new(),
                RuntimeOptions::default(),
            ),
        }
    }

    /// A messenger that writes nothing: its identities, contacts and
    /// messages are gone when it stops. Runs on the current tokio runtime.
    pub fn ephemeral(backend: B) -> Self {
        Self::start(Installation::ephemeral(), backend)
    }

    /// Creates a new vault and message store in the data directory at
    /// `path`, under `passphrase`. Runs on the current tokio runtime.
    pub fn create(path: &Path, passphrase: &str, backend: B) -> Result<Self, OpenError> {
        let passphrase = Passphrase::new(passphrase).map_err(storage)?;
        let dir = DiskDir::open(path, true).map_err(storage)?;
        let installation = Installation::create(Box::new(dir), &passphrase, KdfParams::DEFAULT)
            .map_err(installation)?;
        Ok(Self::start(installation, backend))
    }

    /// Opens the vault and the message store in the data directory at
    /// `path` with `passphrase`. Runs on the current tokio runtime.
    pub fn open(path: &Path, passphrase: &str, backend: B) -> Result<Self, OpenError> {
        let passphrase = Passphrase::new(passphrase).map_err(storage)?;
        let dir = DiskDir::open(path, false).map_err(storage)?;
        let (installation, _) =
            Installation::open(Box::new(dir), &passphrase).map_err(installation)?;
        Ok(Self::start(installation, backend))
    }

    /// Returns true if the data directory was opened without its message
    /// store, though the vault holds identities (a copy of the vault alone,
    /// or a store that was removed): the messages from before are not
    /// there. A front end tells the user.
    pub fn message_store_missing(&self) -> bool {
        self.runtime.message_store_missing()
    }

    /// Follows the events.
    pub fn events(&self) -> Events {
        Events(self.runtime.subscribe())
    }

    /// The local identities.
    pub fn identities(&self) -> Vec<IdentitySummary> {
        self.runtime.identities()
    }

    /// Makes a new local identity with an optional local label.
    pub async fn create_identity(&self, label: Option<&str>) -> Result<IdentityId, CommandError> {
        self.runtime.create_identity(label).await
    }

    /// Deletes a local identity: its keys, contacts and messages.
    pub async fn delete_identity(&self, identity: &IdentityId) -> Result<(), CommandError> {
        self.runtime.delete_identity(identity).await
    }

    /// The contact card of an identity, in its text form.
    pub fn own_card(&self, identity: &IdentityId) -> Result<String, CommandError> {
        self.runtime.own_card(identity)
    }

    /// The contacts of an identity.
    pub fn contacts(&self, identity: &IdentityId) -> Result<Vec<ContactSummary>, CommandError> {
        self.runtime.contacts(identity)
    }

    /// Imports a contact card given in its text form.
    pub async fn add_contact(
        &self,
        identity: &IdentityId,
        card: &str,
    ) -> Result<ContactId, CommandError> {
        self.runtime.add_contact(identity, card).await
    }

    /// Blocks a contact. Its queue stays stored and is never sent.
    pub async fn block_contact(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
    ) -> Result<(), CommandError> {
        self.runtime.block_contact(identity, contact).await
    }

    /// Deletes a contact. Its queue stays stored and is never sent.
    pub async fn delete_contact(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
    ) -> Result<(), CommandError> {
        self.runtime.delete_contact(identity, contact).await
    }

    /// The conversations of an identity.
    pub fn conversations(
        &self,
        identity: &IdentityId,
    ) -> Result<Vec<ConversationSummary>, CommandError> {
        self.runtime.conversations(identity)
    }

    /// A page of a conversation, newest last.
    pub fn messages(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<MessageView>, CommandError> {
        self.runtime.messages(identity, contact, before, limit)
    }

    /// One message.
    pub fn message(&self, reference: &MessageRef) -> Result<Option<MessageView>, CommandError> {
        self.runtime.message(reference)
    }

    /// Queues a message. Done once it is durably queued; delivered when the
    /// contact is reached.
    pub async fn send_message(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
        text: &str,
    ) -> Result<MessageRef, SendError> {
        self.runtime.send_message(identity, contact, text).await
    }

    /// Deletes the messages of a conversation from this installation.
    pub async fn delete_conversation(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
    ) -> Result<bool, CommandError> {
        self.runtime.delete_conversation(identity, contact).await
    }

    /// Sets the local name of a contact, or removes it with `None` or an
    /// empty name. The name is never sent.
    pub async fn set_contact_name(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
        name: Option<&str>,
    ) -> Result<(), CommandError> {
        self.runtime.set_contact_name(identity, contact, name).await
    }

    /// Records whether the user compared the fingerprint of a contact out
    /// of band.
    pub async fn set_contact_verified(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
        verified: bool,
    ) -> Result<(), CommandError> {
        self.runtime
            .set_contact_verified(identity, contact, verified)
            .await
    }

    /// The contact requests of an identity waiting for an answer.
    pub fn requests(&self, identity: &IdentityId) -> Result<Vec<RequestSummary>, CommandError> {
        self.runtime.requests(identity)
    }

    /// Accepts a contact request. Returns the contact it made.
    pub async fn accept_request(&self, request: &RequestRef) -> Result<ContactId, CommandError> {
        self.runtime.accept_request(request).await
    }

    /// Declines a contact request. Nothing is sent.
    pub async fn decline_request(&self, request: &RequestRef) -> Result<(), CommandError> {
        self.runtime.decline_request(request).await
    }

    /// Blocks the sender of a contact request.
    pub async fn block_request(&self, request: &RequestRef) -> Result<(), CommandError> {
        self.runtime.block_request(request).await
    }

    /// The active invitations of an identity.
    pub fn invitations(
        &self,
        identity: &IdentityId,
    ) -> Result<Vec<InvitationSummary>, CommandError> {
        self.runtime.invitations(identity)
    }

    /// Makes an invitation. Returns it with its card in text form.
    pub async fn create_invitation(
        &self,
        identity: &IdentityId,
        label: Option<&str>,
    ) -> Result<(InvitationRef, String), CommandError> {
        self.runtime.create_invitation(identity, label).await
    }

    /// The card of an invitation again, in text form.
    pub fn invitation_card(&self, invitation: &InvitationRef) -> Result<String, CommandError> {
        self.runtime.invitation_card(invitation)
    }

    /// Revokes an invitation, and with `discard_requests` the requests it
    /// admitted. Returns how many requests were discarded.
    pub async fn revoke_invitation(
        &self,
        invitation: &InvitationRef,
        discard_requests: bool,
    ) -> Result<usize, CommandError> {
        self.runtime
            .revoke_invitation(invitation, discard_requests)
            .await
    }

    /// Which requests from strangers an identity considers.
    pub fn request_mode(&self, identity: &IdentityId) -> Result<RequestMode, CommandError> {
        self.runtime.request_mode(identity)
    }

    /// Sets which requests from strangers an identity considers.
    pub async fn set_request_mode(
        &self,
        identity: &IdentityId,
        mode: RequestMode,
    ) -> Result<(), CommandError> {
        self.runtime.set_request_mode(identity, mode).await
    }

    /// The state of the Onion Service of an identity.
    pub fn service_state(&self, identity: &IdentityId) -> Result<ServiceState, CommandError> {
        self.runtime.service_state(identity)
    }

    /// Asks the system Tor how it is.
    pub async fn check_tor(&self) -> TorCheck {
        self.runtime.check_tor().await
    }

    /// Stops: sessions close and services are removed within `deadline`.
    pub async fn shutdown(&self, deadline: Duration) {
        self.runtime.shutdown(deadline).await;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use monolith_tor::MockNetwork;

    #[test]
    fn an_ephemeral_messenger_answers_its_queries_with_plain_values() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap()
            .block_on(async {
                let network = MockNetwork::new();
                let messenger = Messenger::ephemeral(network.backend());
                let identity = messenger.create_identity(Some("Personal")).await.unwrap();
                let listed = messenger.identities();
                assert_eq!(listed.len(), 1);
                assert_eq!(listed[0].identity, identity);
                assert_eq!(listed[0].label.as_deref(), Some("Personal"));
                assert!(
                    messenger
                        .own_card(&identity)
                        .unwrap()
                        .starts_with("MONOLITH")
                );
                assert!(messenger.conversations(&identity).unwrap().is_empty());
                messenger.shutdown(Duration::from_secs(1)).await;
            });
    }
}
