# Security policy

## Status

Monolith is in its design phase. No release exists. No part of it has been
audited. Pre-audit builds are for development and testing only and make no
promise of security.

## Supported versions

None yet. Once releases exist, only the latest release receives security
fixes until stated otherwise here.

## Reporting a vulnerability

Report privately. Do not open a public issue for a vulnerability.

Use the private vulnerability reporting function of the repository host. A
dedicated contact address and a key for encrypted reports will be listed
here before the first release.

Please include:

- what is affected (document section, crate, file);
- how to reproduce it, or the reasoning if it is a design flaw;
- what an attacker gains.

You will get an acknowledgement, and the fix and its disclosure will be
coordinated with you. There is no bug bounty.

## Scope

In scope:

- the protocol and cryptographic design in `docs/`;
- all code in this repository;
- the platform integration files in `integrations/`;
- anything that breaks a statement in `docs/SECURITY_INVARIANTS.md`.

Out of scope, because the design does not claim to defend against them
(`docs/THREAT_MODEL.md`, section 6):

- traffic correlation and other attacks on Tor itself; report those to the
  Tor Project;
- a compromised operating system or user account;
- denial of service by flooding an Onion Service at the Tor level;
- vulnerabilities in Tails, Whonix or Tor; report those upstream.

Design flaws are as welcome as implementation bugs, in particular anything
concerning the rules that bind the session handshake to identities
(`docs/CRYPTOGRAPHY.md` section 5.2). They are Monolith's own and have had
no external review.
