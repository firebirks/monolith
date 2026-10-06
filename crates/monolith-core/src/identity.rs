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
use std::sync::atomic::{AtomicU64, Ordering};
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

use crate::contacts::{ContactStore, ContactView, ImportOutcome, Progress, StoreError};
use crate::dialplan;
use crate::link::Withdrawal;
use crate::persist::{CommitError, Durability, Store, Write};
use crate::requests::{Dropped, InvitationId, Invitations, PendingRequest, RequestId, Requests};
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

/// A number for a new instance of a local identity, drawn at random: the
/// handles an instance gives out (an invitation, a rotation) carry it, so
/// that another identity, or another instance of the same keys, made after
/// a deletion or when the vault is opened again, refuses them.
fn new_instance() -> Result<u64, StoreError> {
    random::<8>().map(|bytes| u64::from_be_bytes(*bytes))
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
    /// Which rotation of the identity this is. The progress contacts make
    /// is recorded for it, and counts for no other.
    id: RotationId,
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
    /// The instance of the identity these keys belong to.
    instance: u64,
    /// The number of the next rotation of this instance.
    rotations: u64,
}

impl Keys {
    /// A number for a new rotation, one no earlier rotation of this
    /// identity had in this process.
    const fn next_rotation(&mut self) -> RotationId {
        let id = RotationId {
            instance: self.instance,
            number: self.rotations,
        };
        self.rotations = self.rotations.saturating_add(1);
        id
    }

    /// The rotation in progress, if any.
    fn rotation_id(&self) -> Option<RotationId> {
        self.rotation.as_ref().map(|rotation| rotation.id)
    }

    /// The party that answers, with `durable` the generation on disk.
    fn answering(&self, durable: u64) -> Arc<LocalParty> {
        match &self.rotation {
            Some(rotation) if rotation.switched_by(durable) => rotation.party.clone(),
            _ => self.party.clone(),
        }
    }
}

/// One local rotation, as its identity numbers them: progress recorded for
/// one rotation never counts for another, of this identity or of another.
/// Each identity starts its numbers at random. Local, and not kept across
/// a restart; a rotation read from the vault gets a new number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RotationId {
    instance: u64,
    number: u64,
}

/// A successor card to announce on one session, with what its completion
/// ([`LocalIdentity::mark_announced`]) is checked against: the rotation it
/// belongs to, the contact it is for (that very contact, not another one
/// made later for the same identity) and the session it goes out on. Only
/// [`LocalIdentity::announcement_for`] makes one.
#[derive(Clone)]
pub struct Announcement {
    card: ContactCard,
    rotation: RotationId,
    peer: ContactCard,
    withdrawal: Withdrawal,
    contact: u64,
}

impl Announcement {
    /// The successor card, to send in an EndpointUpdate.
    pub const fn card(&self) -> &ContactCard {
        &self.card
    }

    /// The rotation it belongs to.
    pub const fn rotation(&self) -> RotationId {
        self.rotation
    }
}

impl fmt::Debug for Announcement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Announcement")
            .field("rotation", &self.rotation)
            .finish_non_exhaustive()
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

/// One local identity.
///
/// Its [`Installation`] owns it. Once the installation is closed (its last
/// handle dropped), the identity is closed as if it were deleted
/// ([`Self::is_closed`]): it refuses every change
/// ([`StoreError::Failed`]), admits nobody, withdraws its sessions and
/// hands out no party, secret or signed card; what it decided could no
/// longer be made durable. Keep the installation for as long as the
/// identity is used.
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
    /// Turns true when the identity is closed.
    closed: watch::Sender<bool>,
    /// The generation that makes the identity durable in its installation:
    /// it is listed only from then on. 0 for an identity read from the
    /// vault; `u64::MAX` while it is being stamped.
    added_at: AtomicU64,
    /// Steps a test runs at named points, to place another operation
    /// exactly there.
    #[cfg(test)]
    #[allow(clippy::type_complexity)]
    hooks: Mutex<HashMap<&'static str, Box<dyn FnOnce() + Send>>>,
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
            requests: Mutex::new(Requests::new(invitations.instance())),
            invitations: Mutex::new(invitations),
            strangers: Strangers::new(MAX_UNKNOWN_SESSIONS),
            handshakes: Arc::new(Semaphore::new(MAX_INBOUND_HANDSHAKES)),
            contact_sessions: Arc::new(Semaphore::new(MAX_CONTACT_SESSIONS)),
            isolation: Mutex::new(HashMap::new()),
            shared,
            durability,
            closed: watch::Sender::new(false),
            added_at: AtomicU64::new(0),
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
        if self.is_closed() {
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
        if self.is_closed() {
            return Err(StoreError::Failed);
        }
        Ok(keys.answering(durable))
    }

    /// Refuses a change of a deleted identity, or of an installation that
    /// failed, before anything is changed.
    fn check_open(&self) -> Result<(), StoreError> {
        if self.is_closed() || self.durability.is_failed() {
            Err(StoreError::Failed)
        } else {
            Ok(())
        }
    }

    /// Returns true once the identity is closed: deleted, or its
    /// installation closed. A closed identity refuses every change, admits
    /// nobody and hands out nothing that holds or uses a secret; this
    /// object stays a closed one for good, also if the same identity is
    /// opened again in another installation object. A deleted identity's
    /// state is in no vault.
    pub fn is_closed(&self) -> bool {
        self.contacts.is_closed()
    }

    /// Follows the closing of the identity, for its supervisor.
    pub(crate) fn closing(&self) -> watch::Receiver<bool> {
        self.closed.subscribe()
    }

    /// Closes the identity: before it is removed from its installation, or
    /// when its installation closes. From here on nothing that holds or uses one of its secrets is
    /// handed out: the copies of the secret bytes it kept for the vault
    /// are erased, the capabilities and the pending requests are dropped,
    /// and every getter of a secret, a party or a signed card fails or
    /// returns nothing. A party that was handed out earlier, to a
    /// handshake in progress, keeps its transport key until it is dropped,
    /// and so do the parties this object holds, until it is dropped.
    fn close(&self) {
        self.contacts.close();
        {
            let mut keys = lock(&self.keys);
            keys.seed.zeroize();
            keys.transport.zeroize();
            keys.onion = None;
            keys.rotation = None;
        }
        let mut invitations = lock(&self.invitations);
        *invitations = Invitations::new(invitations.instance());
        drop(invitations);
        let mut requests = lock(&self.requests);
        *requests = Requests::new(requests.instance());
        drop(requests);
        self.closed.send_replace(true);
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
        if self.is_closed() {
            return Err(CommitError::Failed);
        }
        let shared = self.shared.upgrade().ok_or(CommitError::Failed)?;
        shared.commit(generation).await?;
        if self.is_closed() {
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
        let (outcome, generation) = {
            // The queue is locked across the change of the record and the
            // removal of the sender's request, so that a request queued
            // after this, for a record changed again, is not taken out.
            let mut requests = lock(&self.requests);
            let imported = self.contacts.import(card)?;
            self.hook("forgetting");
            requests.forget(card.identity());
            imported
        };
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
        let generation = {
            // As for an import: one step with the queue locked.
            let mut requests = lock(&self.requests);
            let generation = self.contacts.block(identity)?;
            self.hook("forgetting");
            requests.forget(identity);
            generation
        };
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
        let keys = lock(&self.keys);
        self.contacts.view(identity, keys.rotation_id())
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
        let durable = self.durability.durable();
        let keys = lock(&self.keys);
        if self.is_closed() {
            return None;
        }
        let view = self.contacts.view(identity, keys.rotation_id())?;
        let credentials = view.credentials?;
        let dial = view.dial?;
        let cards = dialplan::plan(&credentials, &dial);
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
        // A session can outlive the identity it belongs to; nothing it
        // brings is taken then.
        self.check_open()?;
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
                applied.request = Some(self.consider(request)?);
            }
        }
        if received.actions.contains(&Action::Confirmed) {
            // The rotation whose new key the session was made with, read
            // and marked with the keys held: the mark is for that rotation
            // and no other.
            let keys = lock(&self.keys);
            if let Some(rotation) = keys
                .rotation
                .as_ref()
                .filter(|rotation| rotation.party.card() == session.local)
            {
                if self
                    .contacts
                    .session_stands(session.peer, session.withdrawal)
                {
                    generation = self.contacts.mark_rotation(
                        session.peer.identity(),
                        rotation.id,
                        Progress::Promoted,
                    )?;
                    applied.promoted_successor = true;
                }
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
    /// Fails with [`StoreError::Failed`] if the identity was deleted.
    fn consider(&self, request: &ContactRequest) -> Result<Result<(), Dropped>, StoreError> {
        self.hook("considering");
        let mode = lock(&self.settings).request_mode;
        let durable = self.durability.durable();
        let invitations = lock(&self.invitations);
        let mut requests = lock(&self.requests);
        // Looked at with the queue locked: the deletion sets the flag
        // before it drops the queue, so a request is never queued into the
        // queue of a deleted identity.
        if self.is_closed() {
            return Err(StoreError::Failed);
        }
        let kind = self.contacts.kind(request.card.identity());
        self.hook("consider");
        Ok(requests.consider(request, kind, mode, &invitations, durable))
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
        request: RequestId,
        answer: impl FnOnce(&PendingRequest) -> Result<u64, StoreError>,
    ) -> Result<u64, StoreError> {
        self.check_open()?;
        let mut requests = lock(&self.requests);
        let pending = requests.get(request).ok_or(StoreError::NotFound)?;
        let sender = *pending.identity();
        let generation = answer(pending)?;
        requests.forget(&sender);
        Ok(generation)
    }

    /// The user accepted the request `request`: its sender becomes an
    /// accepted contact with the card of that request. A request that no
    /// longer waits, also when its sender has queued another since, is
    /// [`StoreError::NotFound`]. Durable when this returns.
    pub async fn accept_request(&self, request: RequestId) -> Result<(), StoreError> {
        let generation = self.answer_request(request, |pending| {
            self.contacts.accept_request(&pending.card)
        })?;
        self.commit(generation).await
    }

    /// The user declined the request `request`. Nothing is sent.
    pub async fn decline_request(&self, request: RequestId) -> Result<(), StoreError> {
        let generation =
            self.answer_request(request, |pending| self.contacts.decline(pending.identity()))?;
        self.commit(generation).await
    }

    /// The user blocked the sender of the request `request`. Its
    /// sessions as a contact are withdrawn at once; durable when this
    /// returns.
    pub async fn block_request(&self, request: RequestId) -> Result<(), StoreError> {
        let generation =
            self.answer_request(request, |pending| self.contacts.block(pending.identity()))?;
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
        // A card can be made for it: checked before anything changes.
        self.card_with(Some(capability.clone()))?;
        let (id, generation) = {
            let mut invitations = lock(&self.invitations);
            let id = invitations.add(capability.clone(), label)?;
            let generation = self.durability.bump();
            invitations.stamp(id, generation);
            (id, generation)
        };
        self.commit(generation).await?;
        // Signed now, with the key the identity answers with once the
        // capability is durable: a switch made durable meanwhile is in it.
        let card = self.card_with(Some(capability))?;
        Ok((id, card))
    }

    /// The card that carries the capability `id`, again. Fails with
    /// [`StoreError::Failed`] once the identity was deleted.
    pub fn invitation_card(&self, id: InvitationId) -> Result<ContactCard, StoreError> {
        if self.is_closed() {
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
        if self.is_closed() {
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

    /// The rotation in progress, for [`Self::mark_announced`].
    pub fn rotation_id(&self) -> Option<RotationId> {
        lock(&self.keys).rotation_id()
    }

    /// The successor card of a rotation in progress: what is announced in
    /// an EndpointUpdate on sessions of the old key, once it is durable.
    ///
    /// `None` until the beginning of the rotation is durable: the card is
    /// handed out nowhere before a crash can no longer lose it.
    pub fn successor_card(&self) -> Option<ContactCard> {
        let durable = self.durability.durable();
        lock(&self.keys)
            .rotation
            .as_ref()
            .filter(|rotation| rotation.announced_from(durable))
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
        let (card, id) = {
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
            // A number no earlier rotation of this identity had: what
            // contacts did for an earlier one does not count for it.
            let id = keys.next_rotation();
            keys.rotation = Some(Rotation {
                id,
                transport,
                epoch,
                party,
                begun: u64::MAX,
                switched: false,
                switched_at: u64::MAX,
            });
            (card, id)
        };
        let generation = self.durability.bump();
        if let Some(rotation) = lock(&self.keys).rotation.as_mut() {
            // Only the rotation this call made.
            if rotation.id == id {
                rotation.begun = generation;
            }
        }
        self.commit(generation).await?;
        Ok(card)
    }

    /// The successor card to announce on the confirmed session `session`,
    /// with the rotation it belongs to, if one is due there: a rotation is
    /// in progress, the session was made with the old key, it still stands
    /// for the contact, and the contact was not sent the successor of this
    /// rotation yet (`docs/PROTOCOL.md` section 11.4, steps 3 and 5). The
    /// caller sends the card in an EndpointUpdate and then calls
    /// [`Self::mark_announced`] with the rotation.
    pub fn announcement_for(&self, session: SessionRef<'_>) -> Option<Announcement> {
        if !self.contacts.admitted(session.withdrawal) {
            return None;
        }
        let durable = self.durability.durable();
        let keys = lock(&self.keys);
        if self.is_closed() {
            return None;
        }
        let rotation = keys.rotation.as_ref()?;
        if keys.party.card() != session.local || !rotation.announced_from(durable) {
            return None;
        }
        let view = self
            .contacts
            .view(session.peer.identity(), Some(rotation.id))?;
        let contact = self.contacts.contact_instance(session.peer.identity())?;
        (view.kind == RecordKind::Accepted
            && !view.successor_announced
            && self
                .contacts
                .session_stands(session.peer, session.withdrawal))
        .then(|| Announcement {
            card: rotation.party.card().clone(),
            rotation: rotation.id,
            peer: session.peer.clone(),
            withdrawal: session.withdrawal.clone(),
            contact,
        })
    }

    /// The successor of `announcement` was sent on its session. Recorded
    /// only while what it was made for still holds: its rotation is the one
    /// in progress, the record of the peer is the very contact it was made
    /// for, and its session still stands for that contact. A completion
    /// that comes after any of them changed (the rotation ended, the
    /// contact was deleted, blocked or made again, the session withdrawn)
    /// counts for nothing and returns false; the successor is announced
    /// again on a later session. Durable when this returns true.
    pub async fn mark_announced(&self, announcement: &Announcement) -> Result<bool, StoreError> {
        self.check_open()?;
        let generation = {
            let keys = lock(&self.keys);
            if keys
                .rotation
                .as_ref()
                .is_none_or(|held| held.id != announcement.rotation)
            {
                return Ok(false);
            }
            let marked = self.contacts.mark_announced_on(
                &announcement.peer,
                &announcement.withdrawal,
                announcement.contact,
                announcement.rotation,
            )?;
            let Some(generation) = marked else {
                return Ok(false);
            };
            generation
        };
        self.commit(generation).await?;
        Ok(true)
    }

    /// Switches the rotation `rotation`, which must be the one in progress,
    /// to answering with the new key (step 4). The policy of
    /// `docs/DESIGN_QUESTIONS.md` section 11: once every accepted contact
    /// was sent the successor of this rotation, or when the user says so
    /// (`force`). The condition is read with the keys held, against the
    /// rotation it switches. Returns false if the switch is not due yet.
    pub async fn switch_rotation(
        &self,
        rotation: RotationId,
        force: bool,
    ) -> Result<bool, StoreError> {
        self.check_open()?;
        let named = rotation;
        let generation = {
            let mut keys = lock(&self.keys);
            let Some(rotation) = keys.rotation.as_mut() else {
                return Err(StoreError::NotFound);
            };
            // A decision for another rotation changes nothing here.
            if rotation.id != named {
                return Ok(false);
            }
            if !force
                && !self
                    .contacts
                    .every_accepted(rotation.id, Progress::Announced)
            {
                return Ok(false);
            }
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

    /// Ends the rotation `rotation`, which must be the one in progress
    /// (step 5): the old key is dropped and the new one
    /// is the only one. Due once every accepted contact confirmed a session
    /// with the new key of this rotation, or when the user says so
    /// (`force`), and in any case only after a switch that is durable: the
    /// new key is then in use and stored, so ending the rotation hands out
    /// nothing the vault could lose. The condition is read with the keys
    /// held. Returns false if it is not due yet.
    pub async fn finish_rotation(
        &self,
        rotation: RotationId,
        force: bool,
    ) -> Result<bool, StoreError> {
        self.check_open()?;
        let named = rotation;
        let durable = self.durability.durable();
        let generation = {
            let mut keys = lock(&self.keys);
            let Some(rotation) = keys.rotation.as_ref() else {
                return Err(StoreError::NotFound);
            };
            // A decision for another rotation changes nothing here.
            if rotation.id != named {
                return Ok(false);
            }
            let due = force
                || self
                    .contacts
                    .every_accepted(rotation.id, Progress::Promoted);
            if !due || !rotation.switched_by(durable) {
                return Ok(false);
            }
            let Some(rotation) = keys.rotation.take() else {
                return Err(StoreError::NotFound);
            };
            keys.transport = rotation.transport;
            keys.epoch = rotation.epoch;
            keys.party = rotation.party;
            self.durability.bump()
        };
        self.commit(generation).await?;
        Ok(true)
    }

    // --- Storage ----------------------------------------------------------

    /// The state of the identity to write, in one cut: the keys, the
    /// rotation in progress, the settings, the active set and every record
    /// with the progress of that rotation are read with all of their locks
    /// held, so no operation is seen half done and no progress of another
    /// rotation is joined to this one.
    fn snapshot(&self) -> StoredIdentity {
        self.hook("snapshot");
        let keys = lock(&self.keys);
        let settings = lock(&self.settings);
        let invitations = lock(&self.invitations);
        let (contacts, blocked, declined) = self.contacts.snapshot(keys.rotation_id());
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
            invitations: invitations.snapshot(),
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

/// The installation closes when its last handle, and the last wait for a
/// write it started, are gone: its identities close with it, in this
/// step, so that a handle kept of one holds no authority any more. A write
/// already under way holds the vault, not this, and ends on its own.
impl Drop for Shared {
    fn drop(&mut self) {
        for identity in lock(&self.identities).iter() {
            identity.close();
        }
    }
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
        // Held weakly between the writes of a job, so that the job keeps
        // nothing of the installation, its keys included, while it writes.
        let reading = Arc::downgrade(self);
        let failing = Arc::downgrade(self);
        Write {
            snapshot: Box::new(move || {
                let shared = reading.upgrade().ok_or(StorageError::Internal)?;
                // The generation first: the state read after it holds every
                // change up to it.
                let covered = shared.durability.applied();
                let contents = shared.contents();
                Ok((covered, contents.encode()?))
            }),
            // Nothing can be made durable any more: every session is
            // withdrawn, even if no waiter is left.
            failed: Box::new(move || {
                if let Some(shared) = failing.upgrade() {
                    shared.withdraw_all();
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
        let instance = new_instance()?;
        let party = issue(
            &stored.identity_seed,
            &stored.transport_secret,
            stored.epoch,
            stored.endpoint,
        )
        .map_err(|_| InstallationError::Invalid)?;
        let rotation = match stored.rotation {
            Some(rotation) => Some(Rotation {
                // The first number of this identity in this process: the
                // marks stored with the rotation are read back with it.
                id: RotationId {
                    instance,
                    number: 0,
                },
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
            rotation.as_ref().map(|rotation| rotation.id),
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
            instance,
            rotations: 1,
        };
        Ok(LocalIdentity::new(
            keys,
            Settings {
                request_mode: stored.request_mode,
                label: stored.label,
            },
            contacts,
            Invitations::load(stored.invitations, instance)
                .map_err(|_| InstallationError::Invalid)?,
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
        let instance = new_instance()?;
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
            instance,
            rotations: 0,
        };
        let identity = Arc::new(LocalIdentity::new(
            keys,
            Settings {
                request_mode: RequestMode::default(),
                label,
            },
            ContactStore::new(id, self.shared.durability.clone()),
            Invitations::new(instance),
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
        let instance = new_instance()?;
        let identity = Arc::new(LocalIdentity::new(
            Keys {
                seed: keys.seed,
                transport: keys.transport,
                onion: keys.onion,
                epoch: keys.epoch,
                endpoint: keys.endpoint,
                party,
                rotation: None,
                instance,
                rotations: 0,
            },
            Settings {
                request_mode: RequestMode::default(),
                label,
            },
            ContactStore::new(id, self.shared.durability.clone()),
            Invitations::new(instance),
            Arc::downgrade(&self.shared),
            self.shared.durability.clone(),
        ));
        self.add(identity).await
    }

    async fn add(
        &self,
        identity: Arc<LocalIdentity>,
    ) -> Result<Arc<LocalIdentity>, InstallationError> {
        let generation = {
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
            identity.added_at.store(u64::MAX, Ordering::SeqCst);
            identities.push(identity.clone());
            let generation = self.shared.durability.bump();
            identity.added_at.store(generation, Ordering::SeqCst);
            generation
        };
        self.shared.commit(generation).await?;
        Ok(identity)
    }

    /// Every local identity.
    ///
    /// Only those that are durable: an identity whose creation is still
    /// being written, or was cancelled before it was, is listed once its
    /// write is done, and never after a crash before that.
    pub fn identities(&self) -> Vec<Arc<LocalIdentity>> {
        let durable = self.shared.durability.durable();
        lock(&self.shared.identities)
            .iter()
            .filter(|held| held.added_at.load(Ordering::SeqCst) <= durable)
            .cloned()
            .collect()
    }

    /// The local identity `identity`.
    pub fn identity(&self, identity: &IdentityPublicKey) -> Option<Arc<LocalIdentity>> {
        let durable = self.shared.durability.durable();
        lock(&self.shared.identities)
            .iter()
            .find(|held| {
                held.identity() == identity && held.added_at.load(Ordering::SeqCst) <= durable
            })
            .cloned()
    }

    /// Deletes the local identity `identity`: its keys, its Onion Service
    /// key, its capabilities and its contact state, and nothing of any
    /// other. Its sessions are withdrawn. Durable when this returns. It is
    /// that instance which is deleted: once it is gone, a deletion of it
    /// finds nothing ([`StoreError::NotFound`]), also when an identity of
    /// the same keys was made again since.
    pub async fn delete_identity(&self, identity: &LocalIdentity) -> Result<(), InstallationError> {
        let _change = self.shared.changes.lock().await;
        {
            let mut identities = lock(&self.shared.identities);
            let position = identities
                .iter()
                .position(|held| core::ptr::eq(Arc::as_ptr(held), identity))
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

    /// A step placed at a point of an operation (`LocalIdentity::hooks`).
    type Hook = Box<dyn FnOnce() + Send>;

    /// Marks `contact` as sent the successor of the rotation in progress,
    /// as a completion of an announcement does.
    fn mark(identity: &LocalIdentity, contact: &IdentityPublicKey) {
        let keys = lock(&identity.keys);
        let rotation = keys.rotation_id().unwrap();
        identity
            .contacts
            .mark_rotation(contact, rotation, Progress::Announced)
            .unwrap();
    }

    impl RotationId {
        /// The rotation `number` of the instance `instance`.
        pub(crate) const fn for_test(instance: u64, number: u64) -> Self {
            Self { instance, number }
        }
    }

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
                // With the queue locked here, the block waits for the
                // consideration to end. Where it is not, the block runs
                // now, and its change of the record is waited for.
                let started = std::time::Instant::now();
                while identity.kind(&carol_id) != RecordKind::Blocked {
                    if matches!(
                        identity.requests.try_lock(),
                        Err(std::sync::TryLockError::WouldBlock)
                    ) && identity.kind(&carol_id) != RecordKind::Blocked
                    {
                        break;
                    }
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
    fn a_snapshot_never_joins_the_progress_of_one_rotation_to_another() {
        // Bob was sent the successor of rotation R1. While the state is
        // read for a write, R1 ends and R2 begins. What is written holds
        // R2 and none of R1's progress: after a crash, R2 must not take
        // Bob for announced.
        let installation = Installation::ephemeral();
        let identity = run(installation.restore_identity(keys(1), None)).unwrap();
        let bob = stranger(2);
        identity.contacts.accept_request(&bob).unwrap();
        run(identity.begin_rotation()).unwrap();
        // Bob was sent the successor (marked as its completion marks it).
        mark(&identity, bob.identity());
        let step: Hook = Box::new({
            let identity = identity.clone();
            move || {
                assert!(
                    run(identity.switch_rotation(identity.rotation_id().unwrap(), true)).unwrap()
                );
                assert!(
                    run(identity.finish_rotation(identity.rotation_id().unwrap(), true)).unwrap()
                );
                run(identity.begin_rotation()).unwrap();
            }
        });
        lock(&identity.hooks).insert("snapshot", step);
        let stored = identity.snapshot();
        let rotation = stored.rotation.as_ref().unwrap();
        assert_eq!(rotation.epoch, EndpointEpoch::new(3).unwrap());
        let held = stored
            .contacts
            .iter()
            .find(|contact| contact.credentials.identity() == bob.identity())
            .unwrap();
        assert!(!held.successor_announced);
        assert!(!held.successor_promoted);
    }

    #[test]
    fn rotations_marks_and_snapshots_on_many_threads_stay_one_cut() {
        // One thread runs rotation after rotation, another records the
        // announcement to Bob for whatever rotation it finds, a third
        // takes snapshots. No snapshot holds progress without the rotation
        // it belongs to, and every mark recorded for a rotation is read
        // back for that rotation only.
        let installation = Installation::ephemeral();
        let identity = run(installation.restore_identity(keys(1), None)).unwrap();
        let bob = stranger(2);
        identity.contacts.accept_request(&bob).unwrap();
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let rotating = std::thread::spawn({
            let identity = identity.clone();
            let done = done.clone();
            move || {
                for _ in 0..300 {
                    run(identity.begin_rotation()).unwrap();
                    run(identity.switch_rotation(identity.rotation_id().unwrap(), true)).unwrap();
                    run(identity.finish_rotation(identity.rotation_id().unwrap(), true)).unwrap();
                }
                done.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        });
        let marking = std::thread::spawn({
            let identity = identity.clone();
            let done = done.clone();
            let bob = *bob.identity();
            move || {
                while !done.load(std::sync::atomic::Ordering::SeqCst) {
                    if let Some(rotation) = identity.rotation_id() {
                        // As a completion marks it: for the rotation in
                        // progress, with the keys held.
                        let keys = lock(&identity.keys);
                        let recorded = keys.rotation_id() == Some(rotation)
                            && identity
                                .contacts
                                .mark_rotation(&bob, rotation, Progress::Announced)
                                .is_ok();
                        // Recorded only for the rotation that was in
                        // progress; while it still is, the mark shows.
                        if recorded && keys.rotation_id() == Some(rotation) {
                            let view = identity.contacts.view(&bob, keys.rotation_id());
                            assert!(view.unwrap().successor_announced);
                        }
                    }
                }
            }
        });
        while !done.load(std::sync::atomic::Ordering::SeqCst) {
            let stored = identity.snapshot();
            if stored.rotation.is_none() {
                assert!(
                    stored
                        .contacts
                        .iter()
                        .all(|contact| !contact.successor_announced && !contact.successor_promoted)
                );
            }
        }
        rotating.join().unwrap();
        marking.join().unwrap();
    }

    #[test]
    fn a_request_considered_as_the_identity_is_deleted_is_not_queued() {
        // A session of the identity passed its check, and the identity is
        // deleted, wholly, before the request is considered. The deleted
        // identity queues nothing.
        let installation = Installation::ephemeral();
        let identity = run(installation.restore_identity(keys(1), None)).unwrap();
        run(identity.set_request_mode(RequestMode::Open)).unwrap();
        let step: Hook = Box::new({
            let installation = installation.clone();
            let deleted = identity.clone();
            move || {
                std::thread::spawn(move || run(installation.delete_identity(&deleted)).unwrap())
                    .join()
                    .unwrap();
            }
        });
        lock(&identity.hooks).insert("considering", step);
        assert!(identity.consider(&request(&stranger(3))).is_err());
        assert!(identity.is_closed());
        assert!(identity.requests().is_empty());
    }

    #[test]
    fn a_block_or_an_import_takes_out_no_request_queued_after_it() {
        // The user blocks Carol, or imports her card. Between the change of
        // her record and the removal of her pending request, the change is
        // undone (she is unblocked, or deleted) and she asks again: her new
        // request is queued for a record that is neither, and must not be
        // taken out.
        for blocking in [true, false] {
            a_request_queued_meanwhile_stays(blocking);
        }
    }

    fn a_request_queued_meanwhile_stays(blocking: bool) {
        let installation = Installation::ephemeral();
        let identity = run(installation.restore_identity(keys(1), None)).unwrap();
        run(identity.set_request_mode(RequestMode::Open)).unwrap();
        let carol = stranger(3);
        let carol_id = *carol.identity();
        let thread = Arc::new(Mutex::new(None));
        let step: Hook = Box::new({
            let identity = identity.clone();
            let thread = thread.clone();
            let carol = carol.clone();
            move || {
                let other = identity.clone();
                let asking = carol.clone();
                *lock(&thread) = Some(std::thread::spawn(move || {
                    if blocking {
                        run(other.unblock(&carol_id)).unwrap();
                    } else {
                        run(other.delete(&carol_id)).unwrap();
                    }
                    assert_eq!(other.consider(&request(&asking)), Ok(Ok(())));
                }));
                // Unblocked; the request is queued, unless the queue is
                // locked here, and so waits for the block to end.
                let started = std::time::Instant::now();
                loop {
                    assert!(started.elapsed() < core::time::Duration::from_secs(10));
                    if identity.kind(&carol_id) != RecordKind::None {
                        std::thread::yield_now();
                        continue;
                    }
                    match identity.requests.try_lock() {
                        Err(std::sync::TryLockError::WouldBlock) => break,
                        Ok(queue) if queue.list().len() == 1 => break,
                        _ => std::thread::yield_now(),
                    }
                }
            }
        });
        lock(&identity.hooks).insert("forgetting", step);
        if blocking {
            run(identity.block(&carol_id)).unwrap();
        } else {
            run(identity.import(&carol)).unwrap();
        }
        let other = lock(&thread).take().unwrap();
        other.join().unwrap();
        assert_eq!(identity.kind(&carol_id), RecordKind::None);
        assert_eq!(
            identity.requests().len(),
            1,
            "the request queued after the block was lost"
        );
    }

    #[test]
    fn a_deleted_identity_keeps_no_copy_of_its_secrets() {
        run(async {
            let installation = Installation::ephemeral();
            let identity = installation.restore_identity(keys(1), None).await.unwrap();
            identity.begin_rotation().await.unwrap();
            installation.delete_identity(&identity).await.unwrap();
            let keys = lock(&identity.keys);
            assert_eq!(*keys.seed, [0; 32]);
            assert_eq!(*keys.transport, [0; 32]);
            assert!(keys.onion.is_none());
            assert!(keys.rotation.is_none());
        });
    }
}
