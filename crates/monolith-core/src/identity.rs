//! A local identity, and the installation that holds the local identities
//! of one data directory.
//!
//! A [`LocalIdentity`] is the context of one identity (`ARCHITECTURE.md`
//! section 1.1): its keys and the parties made from them, its contact
//! store, its active set of invitation capabilities and its pending
//! requests, its budget for strangers and its handshake and contact
//! session budgets, and the isolation group of each of its contacts.
//! Nothing in it is shared with another identity, and nothing in it is
//! process-wide state (S40 to S46).
//!
//! An [`Installation`] holds the identities and where their durable state
//! goes: nowhere, in ephemeral mode, or one vault for all of them. It is
//! the only owner of the vault; a change of any identity is written by
//! writing the state of the installation ([`crate::persist`]).

use core::fmt;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use monolith_identity::{EndpointEpoch, IdentityPublicKey, IdentitySecretKey, OnionServiceKey};
use monolith_protocol::body::{ContactRequest, Message};
use monolith_protocol::card::{ContactCard, EndpointSet, InvitationCapability};
use monolith_protocol::contact::{RecordKind, RequestMode};
use monolith_protocol::credential::CredentialChange;
use monolith_protocol::limits::{
    INVITATION_CAPABILITY_LEN, MAX_CONTACT_SESSIONS, MAX_INBOUND_HANDSHAKES, MAX_LOCAL_IDENTITIES,
    MAX_UNKNOWN_SESSIONS,
};
use monolith_protocol::session::Action;
use monolith_protocol::text::DisplayName;
use monolith_session::{
    Admitted, InboundPeer, LocalParty, OutboundAdmission, OutboundPeer, Received, SessionError,
    TransportSecretKey,
};
use monolith_storage::StorageError;
use monolith_storage::dir::VaultDir;
use monolith_storage::record::{self, Contents, ONION_SECRET_LEN, StoredIdentity, StoredRotation};
use monolith_storage::vault::{KdfParams, Passphrase, Recovery, Vault};
use monolith_tor::{IsolationGroup, OnionServiceSecret, TorError};
use tokio::sync::{Mutex as AsyncMutex, Semaphore, watch};
use zeroize::{Zeroize, Zeroizing};

use crate::contacts::{ContactStore, ContactView, ImportOutcome, StoreError};
use crate::dialplan;
use crate::link::Withdrawal;
use crate::persist::{CommitError, Durability, Store, Write};
use crate::requests::{Dropped, InvitationId, Invitations, PendingRequest, Requests};
use crate::strangers::Strangers;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn random<const N: usize>() -> Result<Zeroizing<[u8; N]>, StoreError> {
    let mut bytes = Zeroizing::new([0_u8; N]);
    getrandom::fill(bytes.as_mut_slice())
        .map_err(|_| StoreError::Commit(CommitError::Storage(StorageError::Randomness)))?;
    Ok(bytes)
}

/// The local party of `seed` with the transport key `transport`, at
/// `epoch`, reachable at `endpoint`. The secrets have to be ones the vault
/// accepts when it reads them back (`record::is_usable_secret`): whatever
/// is issued here may be written there.
fn issue(
    seed: &[u8; 32],
    transport: &[u8; 32],
    epoch: EndpointEpoch,
    endpoint: OnionServiceKey,
) -> Result<Arc<LocalParty>, StoreError> {
    if !record::is_usable_secret(seed) || !record::is_usable_secret(transport) {
        return Err(StoreError::Protocol(
            monolith_protocol::ProtocolError::InvalidKey,
        ));
    }
    let secret = TransportSecretKey::from_bytes(transport)
        .map_err(|_| StoreError::Protocol(monolith_protocol::ProtocolError::InvalidKey))?;
    LocalParty::issue(
        &IdentitySecretKey::from_seed(seed),
        secret,
        epoch,
        EndpointSet::single(endpoint),
    )
    .map(Arc::new)
    .map_err(|_| StoreError::Protocol(monolith_protocol::ProtocolError::InvalidKey))
}

/// A change of the local transport key in progress (`docs/PROTOCOL.md`
/// section 11.4, changing the transport key).
struct Rotation {
    transport: Zeroizing<[u8; 32]>,
    epoch: EndpointEpoch,
    party: Arc<LocalParty>,
    /// The generation that makes the new key and its card durable. The
    /// successor is announced only once it is: a crash must not lose a
    /// successor a contact may already hold. `u64::MAX` while it is being
    /// stamped; 0 for a rotation read from the vault.
    begun: u64,
    /// The identity answers with the new key.
    switched: bool,
    /// The generation that makes the switch durable. The new key is used
    /// only once it is.
    switched_at: u64,
}

impl Rotation {
    /// The successor may be announced: it is durable.
    const fn announced_from(&self, durable: u64) -> bool {
        durable >= self.begun
    }

    /// The switch to the new key is made and durable.
    const fn switched_by(&self, durable: u64) -> bool {
        self.switched && durable >= self.switched_at
    }
}

/// The secret material of a local identity and the parties made from it.
///
/// The key types of the identity and session crates cannot be exported.
/// The bytes they were made from are kept here, in buffers that are
/// erased when dropped, because the vault has to hold them; nothing else
/// reads them.
struct Keys {
    seed: Zeroizing<[u8; 32]>,
    transport: Zeroizing<[u8; 32]>,
    onion: Option<Zeroizing<[u8; ONION_SECRET_LEN]>>,
    epoch: EndpointEpoch,
    endpoint: OnionServiceKey,
    party: Arc<LocalParty>,
    rotation: Option<Rotation>,
}

impl Keys {
    /// The party that answers, with `durable` the generation on disk.
    fn answering(&self, durable: u64) -> Arc<LocalParty> {
        match &self.rotation {
            Some(rotation) if rotation.switched_by(durable) => rotation.party.clone(),
            _ => self.party.clone(),
        }
    }
}

/// Where a local rotation stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RotationState {
    /// No rotation.
    None,
    /// The new key exists and is announced to contacts on sessions of the
    /// old key; the identity still answers with the old key.
    Announcing,
    /// The identity answers with the new key, and dials with it every
    /// contact that holds it as the announced successor.
    Switched,
}

struct Settings {
    request_mode: RequestMode,
    label: Option<DisplayName>,
}

/// What applying a received message did, for the caller and the user.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// A requested contact accepted.
    pub accepted: bool,
    /// What an EndpointUpdate did to the credentials of the contact.
    pub announced: Option<CredentialChange>,
    /// What became of a contact request from a stranger.
    pub request: Option<Result<(), Dropped>>,
    /// The contact confirmed a session made with the new key of a local
    /// rotation.
    pub promoted_successor: bool,
}

/// What the store needs to know about one session to apply what arrived
/// on it. Only a link makes one ([`crate::link::Link::session_ref`]), and
/// only the identity whose contact store admitted the session accepts it:
/// a session of one local identity changes nothing at another, nor at a
/// later identity of the same keys (S40).
#[derive(Clone, Copy, Debug)]
pub struct SessionRef<'a> {
    /// The card that stands for the peer.
    pub(crate) peer: &'a ContactCard,
    /// The local card the session was made with.
    pub(crate) local: &'a ContactCard,
    /// The withdrawal of its link.
    pub(crate) withdrawal: &'a Withdrawal,
}

/// What to dial for a contact: with which local party, and which cards.
#[derive(Clone)]
pub struct DialPlan {
    /// The local party to dial with.
    pub local: Arc<LocalParty>,
    /// The cards to dial, in order. Empty if the user has to confirm where
    /// to connect first.
    pub cards: Vec<ContactCard>,
}

impl fmt::Debug for DialPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DialPlan")
            .field("cards", &self.cards.len())
            .finish_non_exhaustive()
    }
}

/// A step a test runs at a named point of an operation.
#[cfg(test)]
type Hook = Box<dyn FnOnce() + Send>;

/// One local identity.
///
/// Its [`Installation`] owns it. Once the installation is dropped, the
/// identity refuses every change and admits nobody
/// ([`StoreError::Failed`]): what it decides could no longer be made
/// durable. Keep the installation for as long as the identity is used.
pub struct LocalIdentity {
    identity: IdentityPublicKey,
    keys: Mutex<Keys>,
    settings: Mutex<Settings>,
    pub(crate) contacts: ContactStore,
    invitations: Mutex<Invitations>,
    requests: Mutex<Requests>,
    pub(crate) strangers: Strangers,
    pub(crate) handshakes: Arc<Semaphore>,
    pub(crate) contact_sessions: Arc<Semaphore>,
    isolation: Mutex<HashMap<IdentityPublicKey, IsolationGroup>>,
    shared: Weak<Shared>,
    durability: Arc<Durability>,
    /// Turns true when the identity is deleted.
    deleted: watch::Sender<bool>,
    /// Steps a test runs at named points, to place another operation
    /// exactly there.
    #[cfg(test)]
    hooks: Mutex<HashMap<&'static str, Hook>>,
}

impl fmt::Debug for LocalIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalIdentity").finish_non_exhaustive()
    }
}

impl LocalIdentity {
    fn new(
        keys: Keys,
        settings: Settings,
        contacts: ContactStore,
        invitations: Invitations,
        shared: Weak<Shared>,
        durability: Arc<Durability>,
    ) -> Self {
        Self {
            identity: *keys.party.identity(),
            keys: Mutex::new(keys),
            settings: Mutex::new(settings),
            contacts,
            invitations: Mutex::new(invitations),
            requests: Mutex::new(Requests::new()),
            strangers: Strangers::new(MAX_UNKNOWN_SESSIONS),
            handshakes: Arc::new(Semaphore::new(MAX_INBOUND_HANDSHAKES)),
            contact_sessions: Arc::new(Semaphore::new(MAX_CONTACT_SESSIONS)),
            isolation: Mutex::new(HashMap::new()),
            shared,
            durability,
            deleted: watch::Sender::new(false),
            #[cfg(test)]
            hooks: Mutex::new(HashMap::new()),
        }
    }

    /// Runs the step a test placed at `point`, once.
    #[cfg(test)]
    fn hook(&self, point: &'static str) {
        let step = lock(&self.hooks).remove(point);
        if let Some(step) = step {
            step();
        }
    }

    #[cfg(not(test))]
    #[expect(clippy::unused_self, reason = "a point where tests place a step")]
    const fn hook(&self, _point: &'static str) {}

    /// The identity.
    pub const fn identity(&self) -> &IdentityPublicKey {
        &self.identity
    }

    /// The card of the key the identity answers with: what the user hands
    /// to others, without a capability.
    pub fn card(&self) -> ContactCard {
        let durable = self.durability.durable();
        lock(&self.keys).answering(durable).card().clone()
    }

    /// The endpoint of the identity: its Onion Service.
    pub fn endpoint(&self) -> OnionServiceKey {
        lock(&self.keys).endpoint
    }

    /// The secret of the Onion Service, to publish it again after Tor lost
    /// it. `None` before Tor returned one, and once the identity was
    /// deleted.
    pub fn onion_secret(&self) -> Option<OnionServiceSecret> {
        if self.is_deleted() {
            return None;
        }
        lock(&self.keys)
            .onion
            .as_ref()
            .map(|bytes| OnionServiceSecret::from_bytes(bytes))
    }

    /// The budget for strangers of this identity.
    pub const fn strangers(&self) -> &Strangers {
        &self.strangers
    }

    /// The party that answers inbound handshakes. Fails with
    /// [`StoreError::Failed`] once the identity was deleted: its keys are
    /// used for nothing more.
    pub fn answering_party(&self) -> Result<Arc<LocalParty>, StoreError> {
        let durable = self.durability.durable();
        let keys = lock(&self.keys);
        // Read under the lock of the keys, which the deletion takes to
        // erase them after it set the flag.
        if self.is_deleted() {
            return Err(StoreError::Failed);
        }
        Ok(keys.answering(durable))
    }

    /// Refuses a change of a deleted identity, or of an installation that
    /// failed, before anything is changed.
    fn check_open(&self) -> Result<(), StoreError> {
        if self.is_deleted() || self.durability.is_failed() {
            Err(StoreError::Failed)
        } else {
            Ok(())
        }
    }

    /// Returns true once the identity was deleted. A deleted identity
    /// refuses every change and admits nobody, and its state is in no
    /// vault.
    pub fn is_deleted(&self) -> bool {
        self.contacts.is_closed()
    }

    /// Follows the deletion of the identity, for its supervisor.
    pub(crate) fn deletion(&self) -> watch::Receiver<bool> {
        self.deleted.subscribe()
    }

    /// Closes the identity before it is removed from its installation.
    /// From here on nothing that holds or uses one of its secrets is
    /// handed out: the copies of the secret bytes it kept for the vault
    /// are erased, the capabilities and the pending requests are dropped,
    /// and every getter of a secret, a party or a signed card fails or
    /// returns nothing. A party that was handed out earlier, to a
    /// handshake in progress, keeps its transport key until it is dropped.
    fn close(&self) {
        self.contacts.close();
        {
            let mut keys = lock(&self.keys);
            keys.seed.zeroize();
            keys.transport.zeroize();
            keys.onion = None;
            keys.rotation = None;
        }
        *lock(&self.invitations) = Invitations::new();
        *lock(&self.requests) = Requests::new();
        self.deleted.send_replace(true);
    }

    async fn commit(&self, generation: u64) -> Result<(), StoreError> {
        self.wait_durable(generation)
            .await
            .map_err(StoreError::Commit)
    }

    /// Waits until the state a decision of this identity depended on is
    /// durable. The link waits for it before it uses an admission. A
    /// deleted identity has nothing durable: the wait fails if it was
    /// deleted before it began or before it ended.
    pub(crate) async fn wait_durable(&self, generation: u64) -> Result<(), CommitError> {
        if self.is_deleted() {
            return Err(CommitError::Failed);
        }
        let shared = self.shared.upgrade().ok_or(CommitError::Failed)?;
        shared.commit(generation).await?;
        if self.is_deleted() {
            return Err(CommitError::Failed);
        }
        Ok(())
    }

    pub(crate) fn admit_inbound(
        &self,
        peer: InboundPeer,
        withdrawal: &Withdrawal,
    ) -> Result<(Admitted, u64), SessionError> {
        self.contacts.admit_inbound(peer, withdrawal)
    }

    pub(crate) fn admit_outbound(
        &self,
        peer: OutboundPeer,
        withdrawal: &Withdrawal,
    ) -> Result<(OutboundAdmission, u64), SessionError> {
        self.contacts.admit_outbound(peer, withdrawal)
    }

    /// The isolation group for `contact`: one per pair of this identity
    /// and that contact, made on first use and kept for the life of the
    /// process (S42). Never written anywhere.
    pub fn isolation(&self, contact: &IdentityPublicKey) -> Result<IsolationGroup, TorError> {
        let mut groups = lock(&self.isolation);
        if let Some(group) = groups.get(contact) {
            return Ok(group.clone());
        }
        let group = IsolationGroup::generate()?;
        groups.insert(*contact, group.clone());
        Ok(group)
    }

    // --- Contacts -------------------------------------------------------

    /// The user imported `card`. Durable when this returns.
    pub async fn import(&self, card: &ContactCard) -> Result<ImportOutcome, StoreError> {
        let (outcome, generation) = self.contacts.import(card)?;
        lock(&self.requests).forget(card.identity());
        self.commit(generation).await?;
        Ok(outcome)
    }

    /// The user confirmed `card`, shown as the pending successor of a
    /// contact. Durable when this returns.
    pub async fn confirm_pending(&self, card: &ContactCard) -> Result<(), StoreError> {
        let generation = self.contacts.confirm_pending(card)?;
        self.commit(generation).await
    }

    /// The user confirmed `card`, the active card of a contact, for
    /// dialing. Durable when this returns.
    pub async fn confirm_dial(&self, card: &ContactCard) -> Result<(), StoreError> {
        let generation = self.contacts.confirm_dial(card)?;
        self.commit(generation).await
    }

    /// Blocks `identity`. Its sessions as a contact are withdrawn at once;
    /// durable when this returns.
    pub async fn block(&self, identity: &IdentityPublicKey) -> Result<(), StoreError> {
        let generation = self.contacts.block(identity)?;
        lock(&self.requests).forget(identity);
        self.commit(generation).await
    }

    /// Unblocks `identity`. Durable when this returns.
    pub async fn unblock(&self, identity: &IdentityPublicKey) -> Result<(), StoreError> {
        let generation = self.contacts.unblock(identity)?;
        self.commit(generation).await
    }

    /// Deletes the contact `identity`. Its sessions are withdrawn at once;
    /// durable when this returns.
    pub async fn delete(&self, identity: &IdentityPublicKey) -> Result<(), StoreError> {
        let generation = self.contacts.delete(identity)?;
        self.commit(generation).await
    }

    /// Marks the verification of a contact. Durable when this returns.
    pub async fn set_verified(
        &self,
        identity: &IdentityPublicKey,
        verified: bool,
    ) -> Result<(), StoreError> {
        let generation = self.contacts.set_verified(identity, verified)?;
        self.commit(generation).await
    }

    /// What is held about `identity`.
    pub fn contact(&self, identity: &IdentityPublicKey) -> Option<ContactView> {
        self.contacts.view(identity)
    }

    /// Every remote identity with a record.
    pub fn known(&self) -> Vec<IdentityPublicKey> {
        self.contacts.identities()
    }

    /// The kind of record of `identity`.
    pub fn kind(&self, identity: &IdentityPublicKey) -> RecordKind {
        self.contacts.kind(identity)
    }

    /// What to dial for `identity`, or `None` if it is not a contact or
    /// the local identity was deleted.
    pub fn dial_plan(&self, identity: &IdentityPublicKey) -> Option<DialPlan> {
        let view = self.contacts.view(identity)?;
        let credentials = view.credentials?;
        let dial = view.dial?;
        let cards = dialplan::plan(&credentials, &dial);
        let durable = self.durability.durable();
        let keys = lock(&self.keys);
        if self.is_deleted() {
            return None;
        }
        // During a rotation, a contact that holds the new key as the
        // announced successor is dialed with it once the identity has
        // switched, durably; any other contact with the old key, on whose
        // session the successor is announced.
        let local = match &keys.rotation {
            Some(rotation) if rotation.switched_by(durable) && view.successor_announced => {
                rotation.party.clone()
            }
            _ => keys.party.clone(),
        };
        Some(DialPlan { local, cards })
    }

    // --- Messages -------------------------------------------------------

    /// Applies what a received message changes in the durable state, in
    /// the store and only while its session stands for the contact:
    /// `MarkAccepted`, an EndpointUpdate, a contact request from a
    /// stranger, and the confirmation of a session made with the new key
    /// of a local rotation. Durable when this returns; the caller carries
    /// out the visible actions of the message afterwards.
    pub async fn apply(
        &self,
        session: SessionRef<'_>,
        received: &Received,
    ) -> Result<Applied, StoreError> {
        if !self.contacts.admitted(session.withdrawal) {
            return Err(StoreError::OtherIdentity);
        }
        let mut applied = Applied::default();
        let mut generation = self.durability.applied();
        if received.actions.contains(&Action::MarkAccepted) {
            generation = self
                .contacts
                .mark_accepted(session.peer, session.withdrawal)?;
            applied.accepted = true;
        }
        if received.actions.contains(&Action::Deliver) {
            if let Message::EndpointUpdate(card) = &received.message {
                let (change, depends) =
                    self.contacts
                        .announce(session.peer, card, session.withdrawal)?;
                applied.announced = Some(change);
                generation = depends;
            }
        }
        if received.actions.contains(&Action::ConsiderRequest) {
            if let Message::ContactRequest(request) = &received.message {
                applied.request = Some(self.consider(request));
            }
        }
        if received.actions.contains(&Action::Confirmed) {
            let new_key = lock(&self.keys)
                .rotation
                .as_ref()
                .is_some_and(|rotation| rotation.party.card() == session.local);
            if new_key
                && self
                    .contacts
                    .session_stands(session.peer, session.withdrawal)
            {
                generation =
                    self.contacts
                        .set_rotation_marks(session.peer.identity(), None, Some(true))?;
                applied.promoted_successor = true;
            }
        }
        self.commit(generation).await?;
        Ok(applied)
    }

    // --- Requests and invitations ----------------------------------------
    //
    // The queue of requests and the records of their senders change
    // together: considering a request reads the record of its sender with
    // the queue locked, and answering one changes the record and takes the
    // request out with the queue locked. Every change of a record that
    // drops a request (an import, a block) takes it out afterwards. So the
    // operations on a request run in one order, and a request is taken out
    // exactly when its answer was recorded, never lost when the answer
    // fails, and never left queued for a sender that is a contact or
    // blocked by then.

    /// Decides what becomes of a contact request from a stranger.
    fn consider(&self, request: &ContactRequest) -> Result<(), Dropped> {
        let mode = lock(&self.settings).request_mode;
        let durable = self.durability.durable();
        let invitations = lock(&self.invitations);
        let mut requests = lock(&self.requests);
        let kind = self.contacts.kind(request.card.identity());
        self.hook("consider");
        requests.consider(request, kind, mode, &invitations, durable)
    }

    /// The pending contact requests.
    pub fn requests(&self) -> Vec<PendingRequest> {
        lock(&self.requests).list()
    }

    /// Runs `answer` on the request of `identity` and takes the request out
    /// if it succeeds, with the queue locked throughout. A failed answer
    /// leaves the request waiting.
    fn answer_request(
        &self,
        identity: &IdentityPublicKey,
        answer: impl FnOnce(&PendingRequest) -> Result<u64, StoreError>,
    ) -> Result<u64, StoreError> {
        self.check_open()?;
        let mut requests = lock(&self.requests);
        let request = requests.get(identity).ok_or(StoreError::NotFound)?;
        let generation = answer(request)?;
        requests.forget(identity);
        Ok(generation)
    }

    /// The user accepted the request of `identity`: it becomes an accepted
    /// contact with the card of the request. Durable when this returns.
    pub async fn accept_request(&self, identity: &IdentityPublicKey) -> Result<(), StoreError> {
        let generation = self.answer_request(identity, |request| {
            self.contacts.accept_request(&request.card)
        })?;
        self.commit(generation).await
    }

    /// The user declined the request of `identity`. Nothing is sent.
    pub async fn decline_request(&self, identity: &IdentityPublicKey) -> Result<(), StoreError> {
        let generation = self.answer_request(identity, |_| self.contacts.decline(identity))?;
        self.commit(generation).await
    }

    /// The user blocked the sender of the request of `identity`. Its
    /// sessions as a contact are withdrawn at once; durable when this
    /// returns.
    pub async fn block_request(&self, identity: &IdentityPublicKey) -> Result<(), StoreError> {
        let generation = self.answer_request(identity, |_| self.contacts.block(identity))?;
        self.commit(generation).await
    }

    /// Creates a new invitation capability with a local label and returns
    /// it with the card that carries it. The capability is in the active
    /// set, durably, before the card is returned. Fails with
    /// [`StoreError::Full`] when the active set is full; no capability is
    /// revoked to make room.
    pub async fn create_invitation(
        &self,
        label: Option<DisplayName>,
    ) -> Result<(InvitationId, ContactCard), StoreError> {
        self.check_open()?;
        let capability = InvitationCapability::from_bytes(*random::<INVITATION_CAPABILITY_LEN>()?);
        let card = self.card_with(Some(capability.clone()))?;
        let (id, generation) = {
            let mut invitations = lock(&self.invitations);
            let id = invitations.add(capability, label)?;
            let generation = self.durability.bump();
            invitations.stamp(id, generation);
            (id, generation)
        };
        self.commit(generation).await?;
        Ok((id, card))
    }

    /// The card that carries the capability `id`, again. Fails with
    /// [`StoreError::Failed`] once the identity was deleted.
    pub fn invitation_card(&self, id: InvitationId) -> Result<ContactCard, StoreError> {
        if self.is_deleted() {
            return Err(StoreError::Failed);
        }
        let durable = self.durability.durable();
        let capability = lock(&self.invitations)
            .capability(id, durable)
            .cloned()
            .ok_or(StoreError::NotFound)?;
        self.card_with(Some(capability))
    }

    /// The members of the active set and their labels: those that are
    /// durable. A capability whose write is pending is handed out nowhere.
    pub fn invitations(&self) -> Vec<(InvitationId, Option<DisplayName>)> {
        let durable = self.durability.durable();
        lock(&self.invitations).list(durable)
    }

    /// Revokes `id`: requests that carry it are dropped from now on.
    /// Accepted contacts, sessions and pending requests are not touched.
    pub async fn revoke_invitation(&self, id: InvitationId) -> Result<(), StoreError> {
        self.check_open()?;
        lock(&self.invitations).revoke(id)?;
        let generation = self.durability.bump();
        self.commit(generation).await
    }

    /// Revokes `id` and discards the pending requests it admitted, the
    /// separate action of `docs/PROTOCOL.md` section 12.3. Returns how many
    /// requests were discarded.
    pub async fn revoke_and_discard(&self, id: InvitationId) -> Result<usize, StoreError> {
        self.check_open()?;
        lock(&self.invitations).revoke(id)?;
        let discarded = lock(&self.requests).discard_admitted_by(id);
        let generation = self.durability.bump();
        self.commit(generation).await?;
        Ok(discarded)
    }

    /// Sets which requests from strangers are considered.
    pub async fn set_request_mode(&self, mode: RequestMode) -> Result<(), StoreError> {
        self.check_open()?;
        lock(&self.settings).request_mode = mode;
        let generation = self.durability.bump();
        self.commit(generation).await
    }

    /// The request mode.
    pub fn request_mode(&self) -> RequestMode {
        lock(&self.settings).request_mode
    }

    /// The local label of the identity. It is never sent.
    pub fn label(&self) -> Option<DisplayName> {
        lock(&self.settings).label.clone()
    }

    /// A card of the key the identity answers with, with `capability`,
    /// signed with the identity key. Never for a deleted identity.
    fn card_with(
        &self,
        capability: Option<InvitationCapability>,
    ) -> Result<ContactCard, StoreError> {
        let durable = self.durability.durable();
        let keys = lock(&self.keys);
        if self.is_deleted() {
            return Err(StoreError::Failed);
        }
        let party = keys.answering(durable);
        let card = party.card();
        ContactCard::sign(
            &IdentitySecretKey::from_seed(&keys.seed),
            *card.transport(),
            card.epoch(),
            card.endpoints().clone(),
            capability,
        )
        .map_err(StoreError::Protocol)
    }

    // --- Rotation -------------------------------------------------------

    /// Where a local rotation stands, as decided. What reaches a peer
    /// follows a step only once it is durable: the announcement of the
    /// successor ([`Self::announcement_for`]), and the key the identity
    /// answers and dials with after the switch.
    pub fn rotation(&self) -> RotationState {
        match &lock(&self.keys).rotation {
            None => RotationState::None,
            Some(rotation) if rotation.switched => RotationState::Switched,
            Some(_) => RotationState::Announcing,
        }
    }

    /// The successor card of a rotation in progress: what is announced in
    /// an EndpointUpdate on sessions of the old key, once it is durable.
    pub fn successor_card(&self) -> Option<ContactCard> {
        lock(&self.keys)
            .rotation
            .as_ref()
            .map(|rotation| rotation.party.card().clone())
    }

    /// Begins a change of the transport key: a new key, generated
    /// independently of the others, and the successor card for it at the
    /// next epoch. Both are durable before this returns, so a crash never
    /// loses a successor that was already announced. The identity goes on
    /// answering with the old key.
    pub async fn begin_rotation(&self) -> Result<ContactCard, StoreError> {
        self.check_open()?;
        let transport = random::<32>()?;
        let card = {
            let mut keys = lock(&self.keys);
            if keys.rotation.is_some() {
                return Err(StoreError::NotThatCard);
            }
            let epoch = keys.epoch.next().ok_or(StoreError::Full)?;
            let party = issue(&keys.seed, &transport, epoch, keys.endpoint)?;
            if party.card().transport() == keys.party.card().transport() {
                return Err(StoreError::Protocol(
                    monolith_protocol::ProtocolError::InvalidKey,
                ));
            }
            let card = party.card().clone();
            keys.rotation = Some(Rotation {
                transport,
                epoch,
                party,
                begun: u64::MAX,
                switched: false,
                switched_at: u64::MAX,
            });
            card
        };
        self.contacts.clear_rotation_marks();
        let generation = self.durability.bump();
        if let Some(rotation) = lock(&self.keys).rotation.as_mut() {
            // Only the rotation this call made, which is still unstamped.
            if rotation.begun == u64::MAX {
                rotation.begun = generation;
            }
        }
        self.commit(generation).await?;
        Ok(card)
    }

    /// The successor card to announce on the confirmed session `session`,
    /// if one is due there: a rotation is in progress, the session was made
    /// with the old key, it still stands for the contact, and the contact
    /// was not sent the successor yet (`docs/PROTOCOL.md` section 11.4,
    /// steps 3 and 5). The caller sends it in an EndpointUpdate and then
    /// calls [`Self::mark_announced`].
    pub fn announcement_for(&self, session: SessionRef<'_>) -> Option<ContactCard> {
        if !self.contacts.admitted(session.withdrawal) {
            return None;
        }
        let durable = self.durability.durable();
        let successor = {
            let keys = lock(&self.keys);
            if self.is_deleted() {
                return None;
            }
            let rotation = keys.rotation.as_ref()?;
            if keys.party.card() != session.local || !rotation.announced_from(durable) {
                return None;
            }
            rotation.party.card().clone()
        };
        let view = self.contacts.view(session.peer.identity())?;
        (view.kind == RecordKind::Accepted
            && !view.successor_announced
            && self
                .contacts
                .session_stands(session.peer, session.withdrawal))
        .then_some(successor)
    }

    /// The successor was sent to `contact` on a confirmed session of the
    /// old key.
    pub async fn mark_announced(&self, contact: &IdentityPublicKey) -> Result<(), StoreError> {
        self.check_open()?;
        let generation = self
            .contacts
            .set_rotation_marks(contact, Some(true), None)?;
        self.commit(generation).await
    }

    /// Switches to answering with the new key (step 4). The policy of
    /// `docs/DESIGN_QUESTIONS.md` section 11: once every accepted contact
    /// was sent the successor, or when the user says so (`force`).
    /// Returns false if the switch is not due yet.
    pub async fn switch_rotation(&self, force: bool) -> Result<bool, StoreError> {
        self.check_open()?;
        let due = force || self.every_accepted(|view| view.successor_announced);
        if !due {
            return Ok(false);
        }
        let generation = {
            let mut keys = lock(&self.keys);
            let Some(rotation) = keys.rotation.as_mut() else {
                return Err(StoreError::NotFound);
            };
            if !rotation.switched {
                rotation.switched = true;
                rotation.switched_at = u64::MAX;
            }
            let generation = self.durability.bump();
            if rotation.switched_at == u64::MAX {
                rotation.switched_at = generation;
            }
            generation
        };
        self.commit(generation).await?;
        Ok(true)
    }

    /// Ends the rotation (step 5): the old key is dropped and the new one
    /// is the only one. Due once every accepted contact confirmed a session
    /// with the new key, or when the user says so (`force`), and in any case
    /// only after a switch that is durable: the new key is then in use and
    /// stored, so ending the rotation hands out nothing the vault could
    /// lose. Returns false if it is not due yet.
    pub async fn finish_rotation(&self, force: bool) -> Result<bool, StoreError> {
        self.check_open()?;
        let due = force || self.every_accepted(|view| view.successor_promoted);
        let durable = self.durability.durable();
        {
            let mut keys = lock(&self.keys);
            let Some(rotation) = keys.rotation.as_ref() else {
                return Err(StoreError::NotFound);
            };
            if !due || !rotation.switched_by(durable) {
                return Ok(false);
            }
            let Some(rotation) = keys.rotation.take() else {
                return Err(StoreError::NotFound);
            };
            keys.transport = rotation.transport;
            keys.epoch = rotation.epoch;
            keys.party = rotation.party;
        }
        self.contacts.clear_rotation_marks();
        let generation = self.durability.bump();
        self.commit(generation).await?;
        Ok(true)
    }

    fn every_accepted(&self, condition: impl Fn(&ContactView) -> bool) -> bool {
        self.contacts.identities().iter().all(|identity| {
            self.contacts
                .view(identity)
                .is_none_or(|view| view.kind != RecordKind::Accepted || condition(&view))
        })
    }

    // --- Storage ----------------------------------------------------------

    fn snapshot(&self) -> StoredIdentity {
        let (contacts, blocked, declined) = self.contacts.snapshot();
        let keys = lock(&self.keys);
        let settings = lock(&self.settings);
        StoredIdentity {
            identity_seed: keys.seed.clone(),
            transport_secret: keys.transport.clone(),
            onion_secret: keys.onion.clone(),
            epoch: keys.epoch,
            endpoint: keys.endpoint,
            rotation: keys.rotation.as_ref().map(|rotation| StoredRotation {
                transport_secret: rotation.transport.clone(),
                epoch: rotation.epoch,
                switched: rotation.switched,
            }),
            request_mode: settings.request_mode,
            label: settings.label.clone(),
            invitations: lock(&self.invitations).snapshot(),
            contacts,
            blocked,
            declined,
        }
    }

    /// The public keys of the identity: identity key, transport keys and
    /// Onion Service key, for the check that no two local identities share
    /// one (S39).
    fn public_keys(&self) -> Vec<[u8; 32]> {
        let keys = lock(&self.keys);
        let mut public = vec![
            *self.identity.as_bytes(),
            *keys.party.card().transport().as_bytes(),
            *keys.endpoint.as_bytes(),
        ];
        if let Some(rotation) = &keys.rotation {
            public.push(*rotation.party.card().transport().as_bytes());
        }
        public
    }

    fn shares_a_key_with(&self, other: &Self) -> bool {
        let theirs = other.public_keys();
        self.public_keys().iter().any(|key| theirs.contains(key))
    }
}

/// What all identities of an installation share: the durable state and
/// the identities themselves.
pub(crate) struct Shared {
    durability: Arc<Durability>,
    store: Store,
    identities: Mutex<Vec<Arc<LocalIdentity>>>,
    /// One identity is created or deleted at a time.
    changes: AsyncMutex<()>,
}

impl Shared {
    async fn commit(self: &Arc<Self>, generation: u64) -> Result<(), CommitError> {
        let result = self
            .store
            .wait(&self.durability, generation, || self.write())
            .await;
        if result.is_err() {
            // Nothing can be made durable any more: every session is
            // withdrawn and every change refused (fail closed).
            self.withdraw_all();
        }
        result
    }

    /// What a write of the installation needs from it.
    fn write(self: &Arc<Self>) -> Write {
        let reading = self.clone();
        let failing = Arc::downgrade(self);
        let again = Arc::downgrade(self);
        Write {
            snapshot: Box::new(move || {
                // The generation first: the state read after it holds every
                // change up to it.
                let covered = reading.durability.applied();
                let contents = reading.contents();
                Ok((covered, contents.encode()?))
            }),
            // Nothing can be made durable any more: every session is
            // withdrawn, even if no waiter is left.
            failed: Box::new(move || {
                if let Some(shared) = failing.upgrade() {
                    shared.withdraw_all();
                }
            }),
            // Changes came after the snapshot: they are written next, even
            // if nothing waits for them. The job runs this on the blocking
            // pool, where the runtime is at hand; without one (the runtime
            // shuts down) the next wait writes them.
            again: Box::new(move || {
                if let (Some(shared), Ok(runtime)) =
                    (again.upgrade(), tokio::runtime::Handle::try_current())
                {
                    let _ = shared
                        .store
                        .start(&shared.durability, &runtime, || shared.write());
                }
            }),
        }
    }

    /// Withdraws every session of every identity.
    fn withdraw_all(&self) {
        for identity in lock(&self.identities).iter() {
            identity.contacts.withdraw_all();
        }
    }

    fn contents(&self) -> Contents {
        Contents {
            identities: lock(&self.identities)
                .iter()
                .map(|identity| identity.snapshot())
                .collect(),
        }
    }
}

/// The key material of a local identity, for
/// [`Installation::restore_identity`]. Erased when dropped.
pub struct IdentityKeys {
    /// The seed of the identity key.
    pub seed: Zeroizing<[u8; 32]>,
    /// The bytes of the transport secret key.
    pub transport: Zeroizing<[u8; 32]>,
    /// The secret of the Onion Service, in Tor's format.
    pub onion: Option<Zeroizing<[u8; ONION_SECRET_LEN]>>,
    /// The epoch of the local card.
    pub epoch: EndpointEpoch,
    /// The Onion Service.
    pub endpoint: OnionServiceKey,
}

impl fmt::Debug for IdentityKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("IdentityKeys([redacted])")
    }
}

/// Whether an installation keeps its state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// In memory only: gone when the process ends. Nothing is written.
    Ephemeral,
    /// In the vault.
    Persistent,
}

/// The local identities of one data directory, or of one process in
/// ephemeral mode.
#[derive(Clone)]
pub struct Installation {
    shared: Arc<Shared>,
    mode: Mode,
}

impl fmt::Debug for Installation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Installation")
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

/// Why an installation could not be opened or changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallationError {
    /// The vault failed.
    Storage(StorageError),
    /// A change could not be made durable, or the installation failed.
    Store(StoreError),
    /// The vault holds an identity that shares a key with another one, or
    /// one whose keys do not make a valid party (S39).
    Invalid,
    /// `MAX_LOCAL_IDENTITIES` identities exist.
    Full,
}

impl fmt::Display for InstallationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => write!(f, "{error}"),
            Self::Store(error) => write!(f, "{error}"),
            Self::Invalid => f.write_str("invalid identity in the vault"),
            Self::Full => f.write_str("too many local identities"),
        }
    }
}

impl core::error::Error for InstallationError {}

impl From<StorageError> for InstallationError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl From<StoreError> for InstallationError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

/// The vault of an installation, in whatever directory.
pub type BoxedDir = Box<dyn VaultDir + Send>;

impl Installation {
    fn with_store(store: Store, mode: Mode) -> Self {
        Self {
            shared: Arc::new(Shared {
                durability: Durability::new(),
                store,
                identities: Mutex::new(Vec::new()),
                changes: AsyncMutex::new(()),
            }),
            mode,
        }
    }

    /// An installation that writes nothing.
    pub fn ephemeral() -> Self {
        Self::with_store(Store::Ephemeral, Mode::Ephemeral)
    }

    /// Creates a new, empty vault in `dir`.
    pub fn create(
        dir: BoxedDir,
        passphrase: &Passphrase,
        params: KdfParams,
    ) -> Result<Self, InstallationError> {
        let plaintext = Contents::default().encode()?;
        let vault = Vault::create(dir, passphrase, params, &plaintext)?;
        Ok(Self::with_store(
            Store::vault(Box::new(vault)),
            Mode::Persistent,
        ))
    }

    /// Opens the vault in `dir` and rebuilds every identity in it.
    pub fn open(
        dir: BoxedDir,
        passphrase: &Passphrase,
    ) -> Result<(Self, Recovery), InstallationError> {
        let (vault, plaintext, recovery) = Vault::open(dir, passphrase)?;
        let contents = Contents::decode(&plaintext)?;
        let installation = Self::with_store(Store::vault(Box::new(vault)), Mode::Persistent);
        {
            let mut identities = lock(&installation.shared.identities);
            for stored in contents.identities {
                let identity = installation.restore(stored)?;
                if identities
                    .iter()
                    .any(|held| held.shares_a_key_with(&identity))
                {
                    return Err(InstallationError::Invalid);
                }
                identities.push(Arc::new(identity));
            }
        }
        Ok((installation, recovery))
    }

    fn restore(&self, stored: StoredIdentity) -> Result<LocalIdentity, InstallationError> {
        let party = issue(
            &stored.identity_seed,
            &stored.transport_secret,
            stored.epoch,
            stored.endpoint,
        )
        .map_err(|_| InstallationError::Invalid)?;
        let rotation = match stored.rotation {
            Some(rotation) => Some(Rotation {
                party: issue(
                    &stored.identity_seed,
                    &rotation.transport_secret,
                    rotation.epoch,
                    stored.endpoint,
                )
                .ok()
                // The new key is another key, not the old one in another
                // form: X25519 clamps, so two secrets can be one key.
                .filter(|new| new.card().transport() != party.card().transport())
                .ok_or(InstallationError::Invalid)?,
                transport: rotation.transport_secret,
                epoch: rotation.epoch,
                // Read from the vault: durable.
                begun: 0,
                switched: rotation.switched,
                switched_at: 0,
            }),
            None => None,
        };
        let identity = *party.identity();
        let contacts = ContactStore::load(
            identity,
            self.shared.durability.clone(),
            stored.contacts,
            &stored.blocked,
            &stored.declined,
        )
        .map_err(|_| InstallationError::Invalid)?;
        let keys = Keys {
            seed: stored.identity_seed,
            transport: stored.transport_secret,
            onion: stored.onion_secret,
            epoch: stored.epoch,
            endpoint: stored.endpoint,
            party,
            rotation,
        };
        Ok(LocalIdentity::new(
            keys,
            Settings {
                request_mode: stored.request_mode,
                label: stored.label,
            },
            contacts,
            Invitations::load(stored.invitations).map_err(|_| InstallationError::Invalid)?,
            Arc::downgrade(&self.shared),
            self.shared.durability.clone(),
        ))
    }

    /// Whether the installation keeps its state.
    pub const fn mode(&self) -> Mode {
        self.mode
    }

    /// Returns true once a write failed. Every change is refused then.
    pub fn is_failed(&self) -> bool {
        self.shared.durability.is_failed()
    }

    /// Creates a local identity whose Onion Service is `endpoint`, with the
    /// secret Tor returned for it. The identity key and the transport key
    /// are generated here, independently, from the random source of the
    /// operating system. An identity that would share a key with another
    /// local identity is refused (S39). The identity and its Onion Service
    /// key are durable before this returns, so the card is never shown for
    /// an identity a crash could lose.
    pub async fn create_identity(
        &self,
        endpoint: OnionServiceKey,
        onion: Option<&OnionServiceSecret>,
        label: Option<DisplayName>,
    ) -> Result<Arc<LocalIdentity>, InstallationError> {
        let _change = self.shared.changes.lock().await;
        if onion.is_some_and(|secret| !record::is_usable_secret(secret.expose())) {
            return Err(InstallationError::Invalid);
        }
        let seed = random::<32>()?;
        let transport = random::<32>()?;
        let party = issue(&seed, &transport, EndpointEpoch::FIRST, endpoint)?;
        let id = *party.identity();
        let keys = Keys {
            seed,
            transport,
            onion: onion.map(|secret| Zeroizing::new(*secret.expose())),
            epoch: EndpointEpoch::FIRST,
            endpoint,
            party,
            rotation: None,
        };
        let identity = Arc::new(LocalIdentity::new(
            keys,
            Settings {
                request_mode: RequestMode::default(),
                label,
            },
            ContactStore::new(id, self.shared.durability.clone()),
            Invitations::new(),
            Arc::downgrade(&self.shared),
            self.shared.durability.clone(),
        ));
        self.add(identity).await
    }

    /// Restores a local identity from its key material: a backup, or an
    /// identity moved from another installation. The same rules as for
    /// [`Self::create_identity`] apply: the keys must make a valid party,
    /// and none of them may be held by another local identity (S39). The
    /// bytes must have come from a cryptographically secure random source
    /// when the identity was first made.
    pub async fn restore_identity(
        &self,
        keys: IdentityKeys,
        label: Option<DisplayName>,
    ) -> Result<Arc<LocalIdentity>, InstallationError> {
        let _change = self.shared.changes.lock().await;
        if keys
            .onion
            .as_ref()
            .is_some_and(|onion| !record::is_usable_secret(onion.as_slice()))
        {
            return Err(InstallationError::Invalid);
        }
        let party = issue(&keys.seed, &keys.transport, keys.epoch, keys.endpoint)
            .map_err(|_| InstallationError::Invalid)?;
        let id = *party.identity();
        let identity = Arc::new(LocalIdentity::new(
            Keys {
                seed: keys.seed,
                transport: keys.transport,
                onion: keys.onion,
                epoch: keys.epoch,
                endpoint: keys.endpoint,
                party,
                rotation: None,
            },
            Settings {
                request_mode: RequestMode::default(),
                label,
            },
            ContactStore::new(id, self.shared.durability.clone()),
            Invitations::new(),
            Arc::downgrade(&self.shared),
            self.shared.durability.clone(),
        ));
        self.add(identity).await
    }

    async fn add(
        &self,
        identity: Arc<LocalIdentity>,
    ) -> Result<Arc<LocalIdentity>, InstallationError> {
        {
            let mut identities = lock(&self.shared.identities);
            if identities.len() >= MAX_LOCAL_IDENTITIES {
                return Err(InstallationError::Full);
            }
            if identities
                .iter()
                .any(|held| held.shares_a_key_with(&identity))
            {
                return Err(InstallationError::Invalid);
            }
            identities.push(identity.clone());
        }
        let generation = self.shared.durability.bump();
        self.shared.commit(generation).await?;
        Ok(identity)
    }

    /// Every local identity.
    pub fn identities(&self) -> Vec<Arc<LocalIdentity>> {
        lock(&self.shared.identities).clone()
    }

    /// The local identity `identity`.
    pub fn identity(&self, identity: &IdentityPublicKey) -> Option<Arc<LocalIdentity>> {
        lock(&self.shared.identities)
            .iter()
            .find(|held| held.identity() == identity)
            .cloned()
    }

    /// Deletes a local identity: its keys, its Onion Service key, its
    /// capabilities and its contact state, and nothing of any other. Its
    /// sessions are withdrawn. Durable when this returns.
    pub async fn delete_identity(
        &self,
        identity: &IdentityPublicKey,
    ) -> Result<(), InstallationError> {
        let _change = self.shared.changes.lock().await;
        {
            let mut identities = lock(&self.shared.identities);
            let position = identities
                .iter()
                .position(|held| held.identity() == identity)
                .ok_or(InstallationError::Store(StoreError::NotFound))?;
            // Closed before it leaves the installation, so that nothing it
            // does from here on can be taken for durable by a write that no
            // longer holds it.
            if let Some(removed) = identities.get(position) {
                removed.close();
            }
            identities.remove(position);
        }
        let generation = self.shared.durability.bump();
        self.shared
            .commit(generation)
            .await
            .map_err(|error| InstallationError::Store(StoreError::Commit(error)))
    }
}

impl From<CommitError> for InstallationError {
    fn from(error: CommitError) -> Self {
        Self::Store(StoreError::Commit(error))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn run<F: core::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn endpoint(seed: u8) -> OnionServiceKey {
        OnionServiceKey::from_bytes(
            IdentitySecretKey::from_seed(&[seed.wrapping_add(100); 32])
                .public_key()
                .as_bytes(),
        )
        .unwrap()
    }

    fn keys(seed: u8) -> IdentityKeys {
        IdentityKeys {
            seed: Zeroizing::new([seed; 32]),
            transport: Zeroizing::new([seed ^ 0xA5; 32]),
            onion: Some(Zeroizing::new([seed; ONION_SECRET_LEN])),
            epoch: EndpointEpoch::FIRST,
            endpoint: endpoint(seed),
        }
    }

    /// The card of a stranger of seed `seed`.
    fn stranger(seed: u8) -> ContactCard {
        LocalParty::issue(
            &IdentitySecretKey::from_seed(&[seed; 32]),
            TransportSecretKey::from_bytes(&[seed ^ 0xA5; 32]).unwrap(),
            EndpointEpoch::FIRST,
            EndpointSet::single(endpoint(seed)),
        )
        .unwrap()
        .card()
        .clone()
    }

    fn request(card: &ContactCard) -> ContactRequest {
        ContactRequest {
            card: card.clone(),
            invitation: None,
            display_name: monolith_protocol::text::DisplayName::new("Carol").unwrap(),
            introduction: monolith_protocol::text::IntroductionText::new("hello").unwrap(),
        }
    }

    #[test]
    fn a_request_and_a_block_of_its_sender_run_in_one_order() {
        // Carol's request is being considered: her record was read, and
        // nothing decided yet. Right then the user blocks her, on another
        // thread. Either order is fine, but not a mix of the two: her
        // request must not stay queued once she is blocked.
        let installation = Installation::ephemeral();
        let identity = run(installation.restore_identity(keys(1), None)).unwrap();
        run(identity.set_request_mode(RequestMode::Open)).unwrap();
        let carol = stranger(3);
        let carol_id = *carol.identity();
        let thread = Arc::new(Mutex::new(None));
        let step: Hook = Box::new({
            let identity = identity.clone();
            let thread = thread.clone();
            move || {
                let blocking = identity.clone();
                *lock(&thread) = Some(std::thread::spawn(move || {
                    run(blocking.block(&carol_id)).unwrap();
                }));
                // Blocked in the store; the thread may still be on its way
                // to the queue.
                let started = std::time::Instant::now();
                while identity.kind(&carol_id) != RecordKind::Blocked {
                    assert!(started.elapsed() < core::time::Duration::from_secs(10));
                    std::thread::yield_now();
                }
            }
        });
        lock(&identity.hooks).insert("consider", step);
        let _ = identity.consider(&request(&carol));
        let blocked = lock(&thread).take().unwrap();
        blocked.join().unwrap();
        assert_eq!(identity.kind(&carol_id), RecordKind::Blocked);
        assert!(identity.requests().is_empty());
    }

    #[test]
    fn a_deleted_identity_keeps_no_copy_of_its_secrets() {
        run(async {
            let installation = Installation::ephemeral();
            let identity = installation.restore_identity(keys(1), None).await.unwrap();
            identity.begin_rotation().await.unwrap();
            installation
                .delete_identity(identity.identity())
                .await
                .unwrap();
            let keys = lock(&identity.keys);
            assert_eq!(*keys.seed, [0; 32]);
            assert_eq!(*keys.transport, [0; 32]);
            assert!(keys.onion.is_none());
            assert!(keys.rotation.is_none());
        });
    }
}
