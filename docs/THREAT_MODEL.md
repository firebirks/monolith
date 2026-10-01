# Threat model

Status: draft for review. This document says who Monolith defends against,
how, and where it does not. Section 6 lists the limits. They are part of the
design, not footnotes.

Invariant numbers (S1 and so on) refer to `SECURITY_INVARIANTS.md`.

## 1. What is protected

- Message and file content between two contacts.
- The identity private key and the Onion Service private key.
- The contact list: who the user's contacts are, and that two given
  identities are contacts of each other.
- The user's network location, to the extent Tor protects it.
- The integrity of local state: contacts, pinned keys, endpoint epochs.
- The availability of the application under hostile input, within stated
  bounds.

## 2. Assumptions

- The operating system, the user account and the hardware are not
  compromised. Any process running as the same user is trusted to the same
  degree as Monolith.
- Tor works as designed, within its own threat model. Monolith adds nothing
  to Tor's anonymity and must not subtract from it.
- The cryptographic primitives in `CRYPTOGRAPHY.md` are sound, and the
  implementations used are correct. The session construction and its
  implementation are not chosen yet (ADR 0002).
- The user keeps the storage passphrase secret and exchanges contact cards
  over a channel that suits their own situation.
- A peer is hostile until it has been bounded, parsed, validated,
  authenticated and authorized, in that order. An accepted contact is
  authenticated, not trusted.

## 3. Principles

    The peer controls bytes.
    The peer does not control our memory.
    The peer does not control our filesystem.
    The peer does not control our contact list.
    The peer does not control Tor.
    The peer does not control our GUI.
    The peer does not define its own identity.
    The peer does not make us contact third parties.

## 4. Adversaries

### A. Unknown peer who knows the onion address

Can open streams to the Onion Service and send arbitrary bytes.

With the address alone, and no contact card:

- Under the handshake candidate currently written up, it does not complete
  the normal handshake and is not handed an identity proof, because
  completing it requires the responder's identity key. This is
  opportunistic probing resistance, not access control. The identity key is
  public data: anyone who obtains it, from a contact card or anywhere else,
  is past this point. The session design is provisional (ADR 0002), and
  some of the candidates show the identity key to any party that connects.
- It learns that the service is reachable and that it answers the Monolith
  preamble. Both are inherent in running a service at a known address.
- If it holds a list of candidate identity keys, it can test which of them
  is served at the address, because the responder's second handshake
  message is authenticated under a hash that includes the identity key. It
  cannot learn a key that is not on its list.

With the contact card (address, identity key, possibly an invitation):

- It can authenticate as an identity of its choice and send one contact
  request (S7). It receives the responder's identity proof, which tells it
  that the identity on the card is live at that address, and nothing else:
  no profile, no contact information, no acceptance or rejection signal
  (S23).
- A request without a currently valid invitation is dropped in the default
  mode. The peer cannot tell.
- It cannot find out its standing. An identity that is unknown, blocked,
  declined or a deleted former contact gets the same messages in the same
  order, and there is no response that names a reason (PROTOCOL.md 12.1).
  This is about what is sent. Response times are not equalized and are not
  claimed to be.

Residual: presence. Whoever holds the card can tell when the user is online
by connecting. See section 6.

### B. Malicious accepted contact

Can do everything a contact can: send messages and file offers within the
rate limits, see the profile the user shares, see when a session is up,
send an endpoint update.

- Cannot exceed the per-contact limits; exceeding them ends the session.
- Cannot make a file arrive without acceptance (S13), choose where it is
  written (S14), or have it opened (S16).
- Cannot change its own pinned identity (S9) or roll back its endpoint.
- Cannot learn anything about other contacts (S24).
- Can sign an endpoint binding that names an Onion Service it does not
  control. Monolith will dial it after the user confirms; the handshake
  fails because that service cannot prove the contact's identity. The
  effect is connection attempts on the normal reconnect schedule to an
  address of the contact's choosing, through Tor, for as long as the user
  keeps the contact.
- Can keep, publish or forward everything it receives, including the user's
  contact card. Can record when the user is online.

Residual: everything a conversation partner can do by nature.

### C. Many Tor connections to the Onion Service

- Unauthenticated streams are limited in number and in time; the oldest is
  evicted for a new one. All handshake records are fixed-size.
- The inbound rate is capped globally. Beyond the cap, streams are closed
  at accept.
- Tor's proof-of-work defense and a per-circuit stream cap are requested
  where available.

Residual: a sustained flood can keep legitimate inbound connections out.
Outbound dials are unaffected, so a contact pair stays reachable if either
side's service is. An attacker who holds a contact card can also keep the
small budget for strangers busy, which shuts out other strangers' requests
but not contacts. Monolith does not prevent targeted denial of service
against an Onion Service.

### D. Malformed, truncated, duplicated, reordered or oversized frames

- Lengths are validated before reading (S10). The largest unit ever held is
  one frame (S26). Fields have explicit maxima (S11).
- Frames are authenticated in order; duplication or reordering fails
  authentication and ends the session.
- Truncation ends the session; a partial message is never acted on.
- Decoders cannot panic by construction of the lints and are fuzzed (S27).

### E. Replay

- Within a session: cipher nonces are counters.
- Across sessions: an identity proof is bound to the handshake hash and is
  useless in any other session.
- Chat messages resent after a reconnect are dropped by identifier. The
  record of identifiers is in memory unless history is enabled, so after a
  restart of the receiver a resent message can be delivered twice.
- An old contact card cannot replace a newer endpoint (strictly increasing
  epoch).
- An invitation capability can be used by anyone who holds it. That is its
  definition; it is revocable and it authenticates nobody.

### F. Identity or profile impersonation

- An identity is a key. A peer proves possession by signing the session
  transcript. Nothing a peer claims about itself is believed (S20, S21).
- An endpoint that does not prove a known contact's pinned key is a hard
  failure with a warning (S9). Monolith cannot tell a replaced identity
  from a service that is not Monolith at all, and says so.
- A stranger using a contact's display name is shown as an unknown identity
  with its own fingerprint. Names are not unique and are never matched.
- Out-of-band verification compares the fingerprint. Verified and
  merely-pinned contacts are visibly different.

Residual: a user who accepts a request because of its display name without
looking at the fingerprint has accepted a stranger. An attacker can grind
keys so that a fingerprint agrees with a victim's in some characters;
comparison must cover the whole fingerprint.

### G. Contact-list confusion

- Unknown peers cannot write to the contact store (S7).
- Stored records are typed; text from peers is an opaque value and cannot
  add, split or alter records (S28).
- Deleting a contact does not create trust in a later identity with the
  same name.

### H. Learning whether A communicates with B

As a third party on the network:

- There is no message that asks about another identity (S24) and no
  behavior that depends on a claimed identity. Duplicate-session logic
  looks only at authenticated sessions (S22), and the attacker cannot
  authenticate as B.
- An unknown peer is treated the same whatever contact sessions exist. The
  only state unknown peers share is the budget for strangers, and that
  depends on other strangers alone.
- There is no back-connection to an address supplied by a peer (S8).

As someone who took over A's Onion Service key but not A's identity key: an
initiator reveals its identity only after the responder proved the expected
one. The attacker sees that connections arrive, not from whom.

As a contact of both A and B: it sees when each is online. It does not see
their sessions with each other.

As an observer of Tor traffic: see L.

Residual: correlation of online times and traffic by parties who can
observe both ends is outside what Monolith can address.

### I. Hostile filenames, files, Unicode, links, metadata

- Filenames are validated display strings, never paths (S14).
- Text is validated, rendered as plain text, isolated for bidirectional
  layout, and subject to rendering limits (S15).
- Display names reject bidirectional controls and invisible characters.
- Links are text. Nothing is fetched, previewed or opened (S16, S29).
- No MIME type, timestamp, client name or other metadata is transmitted, so
  there is none to trust.

Residual: a received file can be malicious. Monolith does not make it safe
to open.

### J. Attacker with the storage medium but not the passphrase

- The vault is encrypted and authenticated; the key is derived with
  Argon2id.
- Ephemeral mode leaves nothing.
- History is off unless enabled.

Residual: offline guessing, as slow as the KDF makes it, so a weak
passphrase is a weak vault. File sizes and modification times are visible.
Saved files are plaintext. Rollback to an older copy is undetectable. No
secure deletion.

### K. Compromised Tor relay

Within the Tor threat model. A relay sees no content: Onion Service traffic
is encrypted end to end by Tor and again by the session layer. A relay
cannot impersonate a peer; that would need the identity key. Guards and
directory relays see what Tor lets them see.

### L. Observer capable of traffic timing or correlation

Not defended. Padding to 1 KiB hides the exact size of short messages and
keepalives are randomized, which removes some trivial signals. An adversary
watching both ends of a Tor connection can correlate them.

### M. Compromised peer endpoint

Everything shared with that peer is exposed, including what was sent
earlier and stored there. Forward secrecy protects past sessions against
later key theft on the wire; it does nothing for data at a compromised
endpoint.

### N. Resource exhaustion

Every queue, count, rate and timeout is bounded (`RESOURCE_LIMITS.md`,
S12). A declared size never causes an allocation. Streaming paths apply
backpressure. Peer-influenced memory has a computed ceiling.

### O. Local processes and other machines on a private network

- Local processes can reach the loopback listener without Tor; on Whonix,
  other Workstations on the same Gateway can reach the listener. They are
  treated as any unknown peer. This is why authentication is mandatory.
- File permissions keep other users out of the data directory.
- A process of the same user is inside the trust boundary (section 2).

### P. Compromised Monolith process

If a bug gives an attacker control of the process, the keys in memory are
lost. What the attacker can then do to Tor is limited to what the
platform's control port filter allows (S6). On Tails that is creating Onion
Services that point at one local port. On Whonix it is whatever the
Gateway's merged profile allows, to which Monolith's profile adds the same
thing for the Workstation's own port. On other Linux systems there is no
such limit.

### Q. Supply chain

Dependencies are few, pinned by `Cargo.lock`, checked against the RustSec
database and a licence and source policy in CI, and reviewed on update.
Releases are to be signed and reproducible where practical. No dependency
is fetched at run time.

## 5. Lessons from TorChat

The 2015 analysis of TorChat (Viigipuu, Tallinn University of Technology)
ranked seven findings and described several more. Monolith shares no code
and no protocol with TorChat. The table records which structural property
removes each class, so that the class is hard to reintroduce.

| TorChat finding | Cause | What removes the class in Monolith |
| --- | --- | --- |
| No contact authorization; profile sent to anyone who pings | Contact and profile exchange ran before any consent | Only a user command creates a contact (S7). Unknown peers get one bounded request and no data (S23). |
| Communication confirmation through a spoofed ID | Behavior depended on a claimed address: "double connection" reply or a back-connection | A connection speaks only for an identity it proved. No claimed identities, no back-connection, no third-party fields (S8, S22, S24). |
| No length limits | Reader buffered until a newline; no field limits | Fixed-size handshake, length-prefixed frames checked before reading, per-field maxima, rendering limits (S10, S11, S26). |
| `profile_name` injection into the contact file | Line-based file assembled from peer text | Typed records, no text configuration built from peer input (S28). Names are not identifiers (S20). |
| Predictable handshake cookie | Non-cryptographic generator | One CSPRNG source (S17). Authentication is a signature over the session transcript, not a shared random value. |
| Non-default Tor configuration, outdated bundled Tor | Application shipped and configured Tor | No bundled Tor, no Tor configuration (S4, S5). |
| Links clickable without warning | Chat text rendered as active content | Plain text only; copy, never open (S15, S16). |
| GUI impersonation with a lookalike address and the same name | Name shown as identity; contacts auto-added | Fingerprints in security-relevant UI; requests need acceptance; pinning (S9, S20). |
| File transfer auto-accepted, one window per file | No consent, unbounded UI state | Offer and explicit accept (S13); bounded offers; coalesced notifications. |
| `add_me` flooding | Unbounded requests | Bounded queue, one entry per identity, global rate, optional invitation. |
| Restart detectable by a contact | Handshake value reused across connections | Nothing on the wire outlives a session except the identity key itself. |
| Plaintext traces on disk | Contact list, logs, queues, key in clear files | Encrypted vault, ephemeral mode, no log file, history off by default. |
| Removed contact reappears | Removal depended on a message to the peer | Deletion is local. A deleted peer is unknown again and must be accepted again. |
| Two connections per peer, file data accepted on either | Protocol structure | One authenticated session per peer; duplicates resolved after authentication. |
| No protocol documentation | | `PROTOCOL.md` is normative; code follows it. |

The thesis also notes that presence can be observed through the Onion
Service itself and that there is no fix. That applies to Monolith as well
and is listed below.

## 6. Limits and non-goals

Monolith does not claim any of the following, and its documentation and
interface must not suggest otherwise.

- It cannot eliminate Tor traffic-correlation risks. It is not designed
  against a global passive adversary.
- It cannot protect a user whose operating system is compromised.
- It cannot stop a user from revealing identifying information in a
  conversation.
- It cannot completely prevent targeted denial of service against an Onion
  Service.
- It cannot make a received file safe to open.
- It cannot hide a conversation from the two parties having it.
- It does not promise secure deletion on SSDs or modern filesystems.
- Reachability of the Onion Service reveals presence. Anyone who holds the
  contact card can attempt a connection and observe whether the endpoint is
  reachable. Monolith does not hide endpoint availability from card
  holders. Version 1 uses one Onion Service for all contacts, so every
  contact, and everyone a card was leaked to, can do this until the
  endpoint is rotated. The identity model allows a set of endpoints per
  identity (PROTOCOL.md 11.4), which leaves room for per-contact or
  rotating endpoints later; none of that is in version 1.
- Endpoint rotation cannot notify contacts if every endpoint they know has
  disappeared before a signed update reached them. The recovery is a new
  contact card handed over out of band.
- A contact card is a capability to reach the user. Sharing it is the
  user's decision and cannot be undone except by rotating the endpoint.
- No deniability. Identity proofs are signatures.
- No post-quantum security.
- No recovery from a stolen identity key other than telling contacts out of
  band. There is no revocation in version 1.
- No offline delivery. If the two peers are never online together, nothing
  is delivered. There is no server.
- A decline is invisible to the requester, so a declined requester's client
  keeps retrying at a low rate.
- On Linux without a control port filter, Monolith's user has full control
  of Tor. Monolith restricts itself, but nothing outside it does.
- Secrets are not locked in RAM and the session library does not erase its
  key material. On a system with unencrypted swap, key material can reach
  the disk.
- Tails and Whonix integration has not been tested yet. Until it has, the
  platform documents describe intent.
- Early releases have had no independent audit.

## 7. Sources

Accessed 2026-10-01.

- Rain Viigipuu, "Security Analysis of Instant Messenger TorChat", Master's
  thesis, Tallinn University of Technology, 2015.
  https://digikogu.taltech.ee/en/Item/46eb08f7-ed58-4a2f-abe6-5d645b33a260
- Tor design and threat model: https://spec.torproject.org/
- Gosling design document (separate identity and per-contact endpoints):
  https://gosling.technology/design-doc.xhtml
- Briar rendezvous protocol:
  https://code.briarproject.org/briar/briar-spec/-/blob/master/protocols/BRP.md
