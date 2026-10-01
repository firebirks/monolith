# ADR 0001: Identity model

Status: proposed, pending Phase 0 review
Date: 2026-10-01

## Context

A contact has to be recognizable over time. In TorChat, and in Ricochet
Refresh and Cwtch today, the contact identity is the onion address. That
couples three things that change for different reasons: who the contact is,
where the contact can be reached, and the key Tor uses to publish a
service. Rotating an endpoint then means becoming a new contact, and
whoever obtains the Onion Service key becomes the contact.

Briar and Quiet keep the identity separate from the onion address. Gosling,
the newer back end of Ricochet Refresh, separates a long-term identity
service from per-contact endpoint services.

## Decision

1. A Monolith identity is an Ed25519 key pair. The public key is the
   identity. It is separate from the Onion Service key and from every key
   used in the session handshake.

2. The identity key only signs. It signs identity proofs and contact cards,
   each under its own fixed prefix (`CRYPTOGRAPHY.md` section 3). It is
   never used for Diffie-Hellman.

3. Contacts pin the identity public key. A different key is a different
   identity, whatever name comes with it. A pinned key is never replaced
   automatically; there is no trust-on-first-use renewal.

4. The binding from identity to endpoint is a contact card: a fixed-layout
   structure signed by the identity key, carrying the Onion Service public
   key and a monotonic epoch (`PROTOCOL.md` section 11). The card stores the
   32-byte service key, not the address string, so only version 3 services
   can be expressed and a checksum cannot be wrong.

5. An endpoint update is the same structure with a greater epoch, accepted
   only from the pinned identity over an authenticated session or by manual
   import. There is no separate update format and no `previous_epoch`
   field. Rollback protection needs only "strictly greater than the pinned
   epoch"; a second field would add a second thing to validate without
   adding protection.

6. The fingerprint is SHA-256 over a prefix, a key-type byte and the public
   key, shown in base32. A 52-character full form and a 24-character
   compact form (120 bits) exist; verification screens show the full form.

7. Verification status (pinned or verified out of band) is local and is
   never sent.

8. Display names are untrusted text. They are not unique, not identifiers
   and not part of the contact card.

9. Version 1 publishes one Onion Service per identity, shared by all
   contacts.

## Consequences

- An endpoint can change without changing who the contact is.
- Taking over an Onion Service key does not let an attacker pass as the
  contact, and, because the responder proves its identity first, does not
  reveal who connects.
- The identity key is online in every session. There is no offline root
  key. A stolen identity key means impersonation until contacts are told
  out of band; version 1 has no revocation.
- One endpoint for all contacts means every card holder can observe
  presence. Per-contact endpoints, as in Gosling and Briar, would reduce
  that at the cost of one Onion Service per contact and a more complex
  introduction step. Deferred beyond version 1.
- A QR code and a text form carry the card. A URL handler is not
  registered.

## Alternatives considered

- Onion address as identity. Rejected for the coupling described above.
- One key for identity and for the Noise static key (Ed25519 converted to
  X25519). Rejected: the Noise specification advises against using a static
  key outside Noise, and using one key in two algorithms is the kind of
  composition this project avoids.
- An offline root identity that certifies device keys. More machinery than
  version 1 needs; could be added later without changing the pinning rule,
  as a new card version.

## Implementation notes

- `ed25519-dalek`, strict verification only. Version 3.0.0 (July 2026) is
  current; `snow` 0.10 still depends on `curve25519-dalek` 4. Using
  `ed25519-dalek` 2.2 keeps a single curve implementation in the tree.
  Decide at the start of Phase 1, together with the generation of every
  other cryptographic crate (ADR 0005), after checking whether 2.x still
  receives fixes.
- Known advisory: RUSTSEC-2022-0093 (fixed in 2.0). The last public audit of
  the dalek crates was in 2019 and predates the current major versions.

## Open questions

- Epoch handling after restoring an old backup (PROTOCOL.md P3).
- Whether a word-list form of the fingerprint is worth adding.

## Sources

Accessed 2026-10-01.

- https://docs.rs/ed25519-dalek/latest/ed25519_dalek/struct.VerifyingKey.html
- https://rustsec.org/advisories/RUSTSEC-2022-0093.html
- https://blog.quarkslab.com/security-audit-of-dalek-libraries.html
- https://gosling.technology/design-doc.xhtml
- https://docs.cwtch.im/security/components/intro
- https://code.briarproject.org/briar/briar-spec/-/blob/master/protocols/BRP.md
