//! Every length, count, rate and timeout limit in Monolith.
//!
//! This module is the single source of these numbers. Nothing else in the
//! workspace may define a protocol or resource limit. Each value is explained
//! in `docs/RESOURCE_LIMITS.md`; change the two together.
//!
//! The rule for all of them: check the declared size against the limit first,
//! allocate second.

use core::time::Duration;

// ---------------------------------------------------------------------------
// Protocol version
// ---------------------------------------------------------------------------

/// Protocol version spoken by this implementation. It is not a field on
/// the wire: the label in the handshake prologue carries it, and nothing is
/// negotiated (`docs/PROTOCOL.md` section 3).
pub const PROTOCOL_VERSION: u16 = 1;

/// Virtual port of a Monolith Onion Service. Peers connect to this port on
/// the `.onion` name. Not a limit, but it lives here so that every protocol
/// number has one home.
pub const ONION_VIRTUAL_PORT: u16 = 29170;

// ---------------------------------------------------------------------------
// Handshake messages. All of them have a fixed size; a peer cannot make the
// handshake carry a variable amount of data. See docs/PROTOCOL.md section 4.
// ---------------------------------------------------------------------------

/// Length of an X25519 public key: a transport key or an ephemeral key of
/// the session handshake.
pub const DH_PUBLIC_KEY_LEN: usize = 32;

/// Length of a ChaCha20-Poly1305 authentication tag.
pub const AEAD_TAG_LEN: usize = 16;

/// Length of the label at the start of the handshake prologue. The label
/// names the protocol and its version.
pub const SESSION_LABEL_LEN: usize = 19;

/// Length of the handshake prologue: the label and the identity public key
/// of the responder.
pub const HANDSHAKE_PROLOGUE_LEN: usize = SESSION_LABEL_LEN + 32;

/// Length of the first handshake message: an ephemeral key and the tag of
/// an empty payload.
pub const HANDSHAKE_MSG1_LEN: usize = DH_PUBLIC_KEY_LEN + AEAD_TAG_LEN;

/// Length of the second handshake message: an ephemeral key and the tag of
/// an empty payload.
pub const HANDSHAKE_MSG2_LEN: usize = DH_PUBLIC_KEY_LEN + AEAD_TAG_LEN;

/// Length of the third handshake message: the initiator's transport key,
/// encrypted, with its tag, and the initiator's contact card, encrypted,
/// with its tag. The card is the one without invitation capability.
pub const HANDSHAKE_MSG3_LEN: usize =
    DH_PUBLIC_KEY_LEN + AEAD_TAG_LEN + CONTACT_CARD_BASE_LEN + AEAD_TAG_LEN;

// ---------------------------------------------------------------------------
// Transport frames
// ---------------------------------------------------------------------------

/// Width of the big-endian length prefix in front of every encrypted frame.
pub const FRAME_LENGTH_PREFIX_LEN: usize = 2;

/// Every frame plaintext is padded to a multiple of this many bytes.
///
/// Provisional. The padding mechanism is decided, the block size is not;
/// see `docs/adr/0003-wire-format.md`. Framing code takes the block size as
/// a parameter and does not assume this value.
pub const FRAME_PADDING_BLOCK_LEN: usize = 1024;

/// Largest frame plaintext, padding included: the largest whole number of
/// padding blocks that, with the session overhead, still fits the length
/// prefix.
pub const MAX_FRAME_PLAINTEXT_LEN: usize = (MAX_FRAME_LENGTH_VALUE - FRAME_SESSION_OVERHEAD_LEN)
    / FRAME_PADDING_BLOCK_LEN
    * FRAME_PADDING_BLOCK_LEN;

/// Smallest frame plaintext: one padding block.
pub const MIN_FRAME_PLAINTEXT_LEN: usize = FRAME_PADDING_BLOCK_LEN;

/// Largest value the frame length prefix may carry.
pub const MAX_FRAME_CIPHERTEXT_LEN: usize = MAX_FRAME_PLAINTEXT_LEN + FRAME_SESSION_OVERHEAD_LEN;

/// Smallest value the frame length prefix may carry.
pub const MIN_FRAME_CIPHERTEXT_LEN: usize = MIN_FRAME_PLAINTEXT_LEN + FRAME_SESSION_OVERHEAD_LEN;

/// Largest value the 16-bit frame length prefix can carry.
pub const MAX_FRAME_LENGTH_VALUE: usize = 65_535;

/// Bytes the session layer adds to each frame: the authentication tag of a
/// Noise transport message. Framing code takes the overhead as a parameter.
pub const FRAME_SESSION_OVERHEAD_LEN: usize = AEAD_TAG_LEN;

/// Length of the header inside the plaintext: message type and body length.
pub const MESSAGE_HEADER_LEN: usize = 4;

/// Largest message body.
pub const MAX_MESSAGE_BODY_LEN: usize = MAX_FRAME_PLAINTEXT_LEN - MESSAGE_HEADER_LEN;

// The frame length prefix is 16 bits wide and the session cipher refuses
// messages above 65535 bytes.
const _: () = assert!(MAX_FRAME_CIPHERTEXT_LEN <= MAX_FRAME_LENGTH_VALUE);
const _: () = assert!(MAX_FRAME_PLAINTEXT_LEN % FRAME_PADDING_BLOCK_LEN == 0);

// ---------------------------------------------------------------------------
// Identifiers and fixed-size fields
// ---------------------------------------------------------------------------

/// Length of a chat message identifier.
pub const MESSAGE_ID_LEN: usize = 16;

/// Length of a file transfer identifier.
pub const TRANSFER_ID_LEN: usize = 16;

/// Length of an invitation capability.
pub const INVITATION_CAPABILITY_LEN: usize = 16;

/// Length of a Ping or Pong nonce.
pub const PING_NONCE_LEN: usize = 8;

/// Length of an Ed25519 signature.
pub const SIGNATURE_LEN: usize = 64;

/// Length of a SHA-256 digest.
pub const DIGEST_LEN: usize = 32;

// ---------------------------------------------------------------------------
// Variable-size fields. Lengths are in bytes of UTF-8 unless noted.
// ---------------------------------------------------------------------------

/// Width of the length prefix in front of every variable-size field.
pub const FIELD_LENGTH_PREFIX_LEN: usize = 2;

/// Largest chat message text.
pub const MAX_CHAT_TEXT_LEN: usize = 16 * 1024;

/// Largest introduction text in a contact request.
pub const MAX_INTRODUCTION_TEXT_LEN: usize = 512;

/// Largest display name, in bytes.
pub const MAX_DISPLAY_NAME_LEN: usize = 128;

/// Largest display name, in Unicode scalar values.
pub const MAX_DISPLAY_NAME_SCALARS: usize = 64;

/// Largest profile text.
pub const MAX_PROFILE_TEXT_LEN: usize = 1024;

/// Largest filename in a file offer.
pub const MAX_FILENAME_LEN: usize = 255;

/// Most endpoints one contact card may state. The card format and its
/// signature already describe a set of endpoints; version 1 allows one.
pub const MAX_ACTIVE_ENDPOINTS: usize = 1;

/// Length of the fixed fields of a binary contact card: version, identity
/// key, transport key, epoch, endpoint count, flags and signature.
pub const CONTACT_CARD_FIXED_LEN: usize = 1 + 32 + DH_PUBLIC_KEY_LEN + 8 + 1 + 1 + SIGNATURE_LEN;

/// Offset of the endpoint count in a binary contact card: after the
/// version, the identity key, the transport key and the epoch.
pub const CONTACT_CARD_COUNT_OFFSET: usize = 1 + 32 + DH_PUBLIC_KEY_LEN + 8;

/// Length of one endpoint in a binary contact card.
pub const CONTACT_CARD_ENDPOINT_LEN: usize = 32;

/// Length of a binary contact card with one endpoint and no invitation
/// capability.
pub const CONTACT_CARD_BASE_LEN: usize = CONTACT_CARD_FIXED_LEN + CONTACT_CARD_ENDPOINT_LEN;

/// Length of the longest binary contact card: every endpoint slot used and
/// an invitation capability.
pub const MAX_CONTACT_CARD_LEN: usize = CONTACT_CARD_FIXED_LEN
    + MAX_ACTIVE_ENDPOINTS * CONTACT_CARD_ENDPOINT_LEN
    + INVITATION_CAPABILITY_LEN;

/// Largest body of a message that is legal before a session is confirmed.
/// That message is a ContactRequest: a card without invitation, the
/// presence byte and invitation capability, a display name and an
/// introduction.
pub const MAX_UNCONFIRMED_BODY_LEN: usize = CONTACT_CARD_FIXED_LEN
    + MAX_ACTIVE_ENDPOINTS * CONTACT_CARD_ENDPOINT_LEN
    + 1
    + INVITATION_CAPABILITY_LEN
    + FIELD_LENGTH_PREFIX_LEN
    + MAX_DISPLAY_NAME_LEN
    + FIELD_LENGTH_PREFIX_LEN
    + MAX_INTRODUCTION_TEXT_LEN;

/// Largest textual contact card accepted from the user, in bytes. The
/// longest valid card is 310 bytes, all ASCII; the margin is for whitespace,
/// which is removed wherever it occurs.
pub const MAX_CONTACT_CARD_TEXT_LEN: usize = 512;

/// Largest data field in a file chunk: what fits in a maximum frame next to
/// the transfer identifier and the field length prefix.
pub const MAX_FILE_CHUNK_LEN: usize =
    MAX_MESSAGE_BODY_LEN - TRANSFER_ID_LEN - FIELD_LENGTH_PREFIX_LEN;

/// Largest body of any message other than a FileChunk: a ChatMessage with
/// the longest text. A usable set of frame parameters can carry it. File
/// chunks are cut to what a frame holds.
pub const MAX_NON_CHUNK_BODY_LEN: usize =
    MESSAGE_ID_LEN + FIELD_LENGTH_PREFIX_LEN + MAX_CHAT_TEXT_LEN;

const _: () = {
    assert!(MAX_NON_CHUNK_BODY_LEN <= MAX_MESSAGE_BODY_LEN);
    assert!(MAX_NON_CHUNK_BODY_LEN >= MAX_UNCONFIRMED_BODY_LEN);
    // Profile and FileOffer, the other bodies with variable fields.
    assert!(
        MAX_NON_CHUNK_BODY_LEN
            >= 2 * FIELD_LENGTH_PREFIX_LEN + MAX_DISPLAY_NAME_LEN + MAX_PROFILE_TEXT_LEN
    );
    assert!(
        MAX_NON_CHUNK_BODY_LEN >= TRANSFER_ID_LEN + 8 + FIELD_LENGTH_PREFIX_LEN + MAX_FILENAME_LEN
    );
};

/// Largest file size accepted in an offer. The field is 64 bits wide; larger
/// values are a violation. Local policy sets a lower limit, see
/// [`DEFAULT_MAX_FILE_SIZE`].
pub const MAX_FILE_SIZE: u64 = 4 * 1024 * 1024 * 1024;

/// Default limit on the size of a received file.
pub const DEFAULT_MAX_FILE_SIZE: u64 = 1024 * 1024 * 1024;

const _: () =
    assert!(MAX_CHAT_TEXT_LEN + MESSAGE_ID_LEN + FIELD_LENGTH_PREFIX_LEN <= MAX_MESSAGE_BODY_LEN);
const _: () = assert!(DEFAULT_MAX_FILE_SIZE <= MAX_FILE_SIZE);

// ---------------------------------------------------------------------------
// Session lifetime. A session that reaches any of these is closed and
// replaced by a new handshake. There is no in-band rekey.
// ---------------------------------------------------------------------------

/// Longest time a session may stay open. Deferred while a file transfer is
/// active, see [`MAX_SESSION_LIFETIME_WITH_TRANSFER`].
pub const MAX_SESSION_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

/// Longest time a session may stay open when a file transfer keeps it past
/// [`MAX_SESSION_LIFETIME`].
pub const MAX_SESSION_LIFETIME_WITH_TRANSFER: Duration = Duration::from_secs(48 * 60 * 60);

/// Most frames one side may send in a session. The cipher nonce is a 64-bit
/// counter, so this stays a factor of 2^32 below nonce exhaustion.
pub const MAX_FRAMES_PER_DIRECTION: u64 = 1 << 32;

/// Most ciphertext bytes one side may send in a session.
pub const MAX_BYTES_PER_DIRECTION: u64 = 1 << 40;

// ---------------------------------------------------------------------------
// Timeouts
// ---------------------------------------------------------------------------

/// Time allowed for an outbound connection through Tor to be established.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(120);

/// Time allowed between accepting or opening a transport stream and reaching
/// an authenticated state.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// Longest time a session may stay in `AuthenticatedUnknown`, that is,
/// authenticated but not confirmed as a contact session.
pub const UNKNOWN_SESSION_TIMEOUT: Duration = Duration::from_secs(20);

/// Time a peer with no contact record has to send its first message after
/// authentication.
pub const UNKNOWN_FIRST_MESSAGE_TIMEOUT: Duration = Duration::from_secs(10);

/// Time allowed for the Pong that decides whether an older session is still
/// alive during duplicate-session resolution.
pub const DUPLICATE_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Lower bound of the randomized interval between keepalive pings.
pub const PING_INTERVAL_MIN: Duration = Duration::from_secs(90);

/// Upper bound of the randomized interval between keepalive pings.
pub const PING_INTERVAL_MAX: Duration = Duration::from_secs(150);

/// Time allowed for the Pong that answers a Ping.
pub const PONG_TIMEOUT: Duration = Duration::from_secs(60);

/// A session that delivers no complete frame for this long is closed.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(240);

/// Time allowed to receive the rest of a frame once its length prefix has
/// arrived.
pub const FRAME_READ_TIMEOUT: Duration = Duration::from_secs(60);

/// Time allowed to write one frame.
pub const FRAME_WRITE_TIMEOUT: Duration = Duration::from_secs(60);

/// Time a file offer waits for the user before it is dropped.
pub const FILE_OFFER_TIMEOUT: Duration = Duration::from_secs(10 * 60);

// ---------------------------------------------------------------------------
// Concurrency budgets
// ---------------------------------------------------------------------------

/// Inbound streams that have not completed authentication.
pub const MAX_INBOUND_HANDSHAKES: usize = 16;

/// Sessions with authenticated peers for which no contact record exists,
/// or which are declined or blocked.
pub const MAX_UNKNOWN_SESSIONS: usize = 4;

/// Sessions with identities held as accepted or requested contacts, inbound
/// and outbound together, confirmed or not.
pub const MAX_CONTACT_SESSIONS: usize = 256;

/// Outbound connection attempts in progress at the same time.
pub const MAX_CONCURRENT_DIALS: usize = 4;

/// Accepted contacts.
pub const MAX_CONTACTS: usize = 1000;

/// Blocked identities.
pub const MAX_BLOCKED_IDENTITIES: usize = 10_000;

/// Declined identities. The oldest entry is dropped when the list is full.
pub const MAX_DECLINED_IDENTITIES: usize = 1024;

/// Contact requests waiting for the user.
pub const MAX_PENDING_CONTACT_REQUESTS: usize = 32;

/// Contact requests waiting for the user that arrived with the same
/// invitation capability, or with none.
pub const MAX_PENDING_REQUESTS_PER_INVITATION: usize = 8;

/// Invitation capabilities valid at the same time.
pub const MAX_ACTIVE_INVITATIONS: usize = 16;

/// Events queued for the user interface.
pub const MAX_UI_EVENTS: usize = 256;

/// File offers from one contact waiting for the user.
pub const MAX_PENDING_FILE_OFFERS_PER_CONTACT: usize = 4;

/// File offers waiting for the user, all contacts together.
pub const MAX_PENDING_FILE_OFFERS: usize = 16;

/// File transfers in progress with one contact, both directions together.
pub const MAX_ACTIVE_TRANSFERS_PER_CONTACT: usize = 2;

/// File transfers in progress, all contacts together.
pub const MAX_ACTIVE_TRANSFERS: usize = 4;

/// Identifiers of ended transfers remembered per session, so that messages
/// which crossed an abort or a reject are discarded and not treated as a
/// violation.
pub const MAX_ENDED_TRANSFER_IDS: usize = 64;

/// Free disk space required beyond the declared size before a transfer is
/// accepted.
pub const FILE_FREE_SPACE_MARGIN: u64 = 64 * 1024 * 1024;

/// Chat messages queued for one contact.
pub const MAX_QUEUED_MESSAGES_PER_CONTACT: usize = 256;

/// Bytes of chat text queued for all contacts together.
pub const MAX_QUEUED_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

/// Received message identifiers remembered per contact to drop duplicates.
pub const MESSAGE_DEDUP_WINDOW: usize = 1024;

// ---------------------------------------------------------------------------
// Rates
// ---------------------------------------------------------------------------

/// A token bucket: `burst` tokens of capacity, refilled at `per_minute`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateLimit {
    /// Bucket capacity.
    pub burst: u32,
    /// Tokens added per minute.
    pub per_minute: u32,
}

/// Inbound streams accepted from the Onion Service, all peers together.
pub const INBOUND_CONNECTION_RATE: RateLimit = RateLimit {
    burst: 32,
    per_minute: 120,
};

/// Unknown peers allowed past authentication, all identities together.
pub const UNKNOWN_SESSION_RATE: RateLimit = RateLimit {
    burst: 8,
    per_minute: 12,
};

/// Chat messages received from one contact.
pub const CHAT_MESSAGE_RATE: RateLimit = RateLimit {
    burst: 60,
    per_minute: 300,
};

/// Ping, Pong, MessageAck, EndpointUpdate and file control messages received
/// from one contact, and a ContactRequest or ContactAccept that arrives after
/// the session is confirmed.
pub const CONTROL_MESSAGE_RATE: RateLimit = RateLimit {
    burst: 120,
    per_minute: 600,
};

/// File offers received from one contact.
pub const FILE_OFFER_RATE: RateLimit = RateLimit {
    burst: 4,
    per_minute: 6,
};

/// Profile updates received from one contact.
pub const PROFILE_UPDATE_RATE: RateLimit = RateLimit {
    burst: 8,
    per_minute: 8,
};

/// New authenticated sessions from one identity.
pub const CONTACT_SESSION_RATE: RateLimit = RateLimit {
    burst: 6,
    per_minute: 6,
};

// ---------------------------------------------------------------------------
// Reconnect schedule
// ---------------------------------------------------------------------------

/// Delay before the first retry of a failed outbound connection.
pub const RECONNECT_DELAY_INITIAL: Duration = Duration::from_secs(10);

/// Upper bound of the retry delay before jitter.
pub const RECONNECT_DELAY_MAX: Duration = Duration::from_secs(30 * 60);

/// Factor applied to the delay after each failure.
pub const RECONNECT_BACKOFF_FACTOR: u32 = 2;

/// Each delay is multiplied by a random factor in
/// `[100 - RECONNECT_JITTER_PERCENT, 100 + RECONNECT_JITTER_PERCENT]` percent.
pub const RECONNECT_JITTER_PERCENT: u32 = 50;

/// A session must stay authenticated this long before the retry delay of its
/// contact is reset.
pub const RECONNECT_RESET_AFTER: Duration = Duration::from_secs(60);

/// On startup the first dial to each contact is spread over this window.
pub const STARTUP_DIAL_SPREAD: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// Tor control and SOCKS replies. Local Tor is trusted more than a peer, but
// its replies are still read with bounds.
// ---------------------------------------------------------------------------

/// Longest line accepted from the Tor control port.
pub const MAX_CONTROL_LINE_LEN: usize = 1024;

/// Most lines accepted in one Tor control reply.
pub const MAX_CONTROL_REPLY_LINES: usize = 16;

/// Longest SOCKS5 reply accepted from Tor.
pub const MAX_SOCKS_REPLY_LEN: usize = 262;

/// Time allowed for one Tor control command to be answered.
pub const CONTROL_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Storage. The vault file can be modified by whoever has write access to the
// disk, so its header is read with bounds like any other input.
// ---------------------------------------------------------------------------

/// Largest vault file that will be read.
pub const MAX_VAULT_FILE_LEN: u64 = 16 * 1024 * 1024;

/// Smallest Argon2id memory cost accepted from a vault header, in KiB.
pub const MIN_KDF_MEMORY_KIB: u32 = 64 * 1024;

/// Largest Argon2id memory cost accepted from a vault header, in KiB.
pub const MAX_KDF_MEMORY_KIB: u32 = 1024 * 1024;

/// Argon2id memory cost of a newly created vault, in KiB.
///
/// Provisional, like the other two KDF defaults. They are not frozen until
/// the benchmark in `docs/STORAGE.md` section 3.3 has been run. The vault
/// header stores the parameters actually used, so raising a default never
/// locks anyone out of an existing vault.
pub const DEFAULT_KDF_MEMORY_KIB: u32 = 256 * 1024;

/// Smallest Argon2id iteration count accepted from a vault header.
pub const MIN_KDF_ITERATIONS: u32 = 3;

/// Largest Argon2id iteration count accepted from a vault header.
pub const MAX_KDF_ITERATIONS: u32 = 16;

/// Argon2id iteration count of a newly created vault.
pub const DEFAULT_KDF_ITERATIONS: u32 = 3;

/// Smallest Argon2id parallelism accepted from a vault header.
pub const MIN_KDF_PARALLELISM: u32 = 1;

/// Largest Argon2id parallelism accepted from a vault header.
pub const MAX_KDF_PARALLELISM: u32 = 8;

/// Argon2id parallelism of a newly created vault.
pub const DEFAULT_KDF_PARALLELISM: u32 = 4;

/// The vault plaintext is padded to a multiple of this many bytes.
pub const VAULT_PADDING_BLOCK_LEN: usize = 64 * 1024;

/// Longest storage passphrase, in bytes.
pub const MAX_PASSPHRASE_LEN: usize = 1024;

// ---------------------------------------------------------------------------
// Rendering. These are independent of the protocol limits above and apply to
// what a user interface draws, not to what a peer may send.
// ---------------------------------------------------------------------------

/// Most Unicode scalar values in one grapheme cluster before the cluster is
/// replaced by a placeholder.
pub const MAX_RENDERED_GRAPHEME_SCALARS: usize = 16;

/// Longest run of characters without a break opportunity before the renderer
/// forces one.
pub const MAX_RENDERED_UNBROKEN_RUN: usize = 64;

/// Most lines of one message drawn before the rest is collapsed.
pub const MAX_RENDERED_MESSAGE_LINES: usize = 200;

// ---------------------------------------------------------------------------
// Relations between the limits above. A change that breaks one of them does
// not compile.
// ---------------------------------------------------------------------------

const _: () = {
    assert!(HANDSHAKE_PROLOGUE_LEN == 51);
    assert!(HANDSHAKE_MSG1_LEN == 48);
    assert!(HANDSHAKE_MSG2_LEN == 48);
    assert!(HANDSHAKE_MSG3_LEN == 235);
    // The third handshake message has a fixed size because a card of
    // version 1 has exactly one endpoint. More endpoints per card need a
    // new protocol label, which is where this size would change.
    assert!(MAX_ACTIVE_ENDPOINTS == 1);

    assert!(MAX_FRAME_PLAINTEXT_LEN == 64_512);
    assert!(MAX_FRAME_CIPHERTEXT_LEN == 64_528);
    assert!(MIN_FRAME_CIPHERTEXT_LEN == 1_040);
    assert!(MAX_MESSAGE_BODY_LEN == 64_508);
    assert!(MAX_FILE_CHUNK_LEN == 64_490);

    assert!(MAX_ACTIVE_ENDPOINTS >= 1 && MAX_ACTIVE_ENDPOINTS <= 255);
    assert!(CONTACT_CARD_BASE_LEN == 171);
    assert!(MAX_CONTACT_CARD_LEN == 187);
    assert!(CONTACT_CARD_COUNT_OFFSET == 73);
    assert!(MAX_CONTACT_CARD_LEN <= MAX_MESSAGE_BODY_LEN);

    assert!(MAX_UNCONFIRMED_BODY_LEN == 832);
    // With the working padding block, every message before confirmation
    // fits in one block.
    assert!(MAX_UNCONFIRMED_BODY_LEN + MESSAGE_HEADER_LEN <= FRAME_PADDING_BLOCK_LEN);
    assert!(FRAME_PADDING_BLOCK_LEN > MESSAGE_HEADER_LEN);
    assert!(FRAME_PADDING_BLOCK_LEN + FRAME_SESSION_OVERHEAD_LEN <= MAX_FRAME_LENGTH_VALUE);

    assert!(MAX_UNKNOWN_SESSIONS < MAX_INBOUND_HANDSHAKES);
    assert!(MAX_INBOUND_HANDSHAKES < MAX_CONTACT_SESSIONS);
    assert!(MAX_PENDING_FILE_OFFERS_PER_CONTACT <= MAX_PENDING_FILE_OFFERS);
    assert!(MAX_ACTIVE_TRANSFERS_PER_CONTACT <= MAX_ACTIVE_TRANSFERS);

    assert!(PING_INTERVAL_MIN.as_secs() < PING_INTERVAL_MAX.as_secs());
    assert!(PING_INTERVAL_MAX.as_secs() + PONG_TIMEOUT.as_secs() <= IDLE_TIMEOUT.as_secs());
    assert!(RECONNECT_DELAY_INITIAL.as_secs() < RECONNECT_DELAY_MAX.as_secs());
    assert!(UNKNOWN_SESSION_TIMEOUT.as_secs() <= HANDSHAKE_TIMEOUT.as_secs());
    assert!(UNKNOWN_FIRST_MESSAGE_TIMEOUT.as_secs() <= UNKNOWN_SESSION_TIMEOUT.as_secs());
    assert!(DUPLICATE_PROBE_TIMEOUT.as_secs() <= PONG_TIMEOUT.as_secs());
    assert!(MAX_SESSION_LIFETIME.as_secs() < MAX_SESSION_LIFETIME_WITH_TRANSFER.as_secs());

    assert!(MAX_PENDING_REQUESTS_PER_INVITATION <= MAX_PENDING_CONTACT_REQUESTS);

    // A profile is sent once per confirmed session, so a conforming peer
    // sends profiles at most as often as it may open sessions.
    assert!(PROFILE_UPDATE_RATE.burst >= CONTACT_SESSION_RATE.burst);
    assert!(PROFILE_UPDATE_RATE.per_minute >= CONTACT_SESSION_RATE.per_minute);

    assert!(MIN_KDF_MEMORY_KIB <= DEFAULT_KDF_MEMORY_KIB);
    assert!(DEFAULT_KDF_MEMORY_KIB <= MAX_KDF_MEMORY_KIB);
    assert!(MIN_KDF_ITERATIONS <= DEFAULT_KDF_ITERATIONS);
    assert!(DEFAULT_KDF_ITERATIONS <= MAX_KDF_ITERATIONS);
    assert!(MIN_KDF_PARALLELISM <= DEFAULT_KDF_PARALLELISM);
    assert!(DEFAULT_KDF_PARALLELISM <= MAX_KDF_PARALLELISM);
};
