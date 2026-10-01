//! The duplicate-session rule.
//!
//! Both peers may dial each other at the same time, so two confirmed
//! sessions with the same contact can exist for a moment. The rule of
//! `docs/PROTOCOL.md` section 14 decides which one stays. It looks only at
//! confirmed sessions; an unauthenticated stream never takes part.
//!
//! The decision depends on the two identity keys and on who initiated each
//! session, and on nothing else. When both sessions are alive, both ends
//! therefore keep the same one without exchanging a message.

use core::cmp::Ordering;

use monolith_identity::IdentityPublicKey;

use crate::ProtocolError;

/// Which side opened a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Initiator {
    /// The local side dialed.
    Local,
    /// The peer dialed.
    Remote,
}

/// What to do when a second session with the same contact is confirmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Resolution {
    /// Keep the newer session. Close the older one.
    CloseOlder,
    /// The older session is the preferred one, but it may be dead without
    /// the local side having noticed. Send a Ping on it and wait up to
    /// `DUPLICATE_PROBE_TIMEOUT`. Then call [`after_probe`].
    ProbeOlder,
}

/// Decides between an older and a newer confirmed session with the same
/// contact.
///
/// Fails if the two identities are equal. A session with oneself is rejected
/// long before this point, so that is an internal error, not a peer's doing.
pub fn resolve(
    local: &IdentityPublicKey,
    remote: &IdentityPublicKey,
    older: Initiator,
    newer: Initiator,
) -> Result<Resolution, ProtocolError> {
    let preferred = preferred_initiator(local, remote)?;
    if older == newer {
        // A side opens a second session only after it considers the first
        // one dead, so the older session is stale.
        return Ok(Resolution::CloseOlder);
    }
    if newer == preferred {
        Ok(Resolution::CloseOlder)
    } else {
        Ok(Resolution::ProbeOlder)
    }
}

/// Which session to close once the probe of the older session has finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProbeOutcome {
    /// The older session answered. Keep it and close the newer one.
    CloseNewer,
    /// The older session did not answer in time. Close it and keep the newer
    /// one.
    CloseOlder,
}

/// Turns the result of the probe into a decision.
pub const fn after_probe(older_answered: bool) -> ProbeOutcome {
    if older_answered {
        ProbeOutcome::CloseNewer
    } else {
        ProbeOutcome::CloseOlder
    }
}

/// Returns the side whose sessions are preferred: the one with the smaller
/// identity key, compared as 32-byte strings.
pub fn preferred_initiator(
    local: &IdentityPublicKey,
    remote: &IdentityPublicKey,
) -> Result<Initiator, ProtocolError> {
    match local.cmp(remote) {
        Ordering::Less => Ok(Initiator::Local),
        Ordering::Greater => Ok(Initiator::Remote),
        Ordering::Equal => Err(ProtocolError::InvalidValue),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use monolith_identity::IdentitySecretKey;

    fn identity(seed: u8) -> IdentityPublicKey {
        IdentitySecretKey::from_seed(&[seed; 32]).public_key()
    }

    /// Two identities, the first with the smaller key.
    fn ordered_pair() -> (IdentityPublicKey, IdentityPublicKey) {
        let a = identity(1);
        let b = identity(2);
        if a < b { (a, b) } else { (b, a) }
    }

    /// The session a side keeps, named from the point of view of the side
    /// with the smaller key: true if it is the session that side initiated.
    fn kept_is_small_initiated(
        local: &IdentityPublicKey,
        remote: &IdentityPublicKey,
        older: Initiator,
        newer: Initiator,
        local_is_small: bool,
    ) -> bool {
        let kept = match resolve(local, remote, older, newer).unwrap() {
            Resolution::CloseOlder => newer,
            // Both sessions are alive in this test, so the probe succeeds.
            Resolution::ProbeOlder => match after_probe(true) {
                ProbeOutcome::CloseNewer => older,
                ProbeOutcome::CloseOlder => newer,
            },
        };
        // "Local" on the small side and "Remote" on the large side both
        // mean "initiated by the small side".
        (kept == Initiator::Local) == local_is_small
    }

    #[test]
    fn the_smaller_key_is_preferred() {
        let (small, large) = ordered_pair();
        assert_eq!(preferred_initiator(&small, &large), Ok(Initiator::Local));
        assert_eq!(preferred_initiator(&large, &small), Ok(Initiator::Remote));
        assert_eq!(
            preferred_initiator(&small, &small),
            Err(ProtocolError::InvalidValue)
        );
    }

    #[test]
    fn same_direction_keeps_the_newer_session() {
        // T-DUP-3.
        let (small, large) = ordered_pair();
        for (local, remote) in [(&small, &large), (&large, &small)] {
            for initiator in [Initiator::Local, Initiator::Remote] {
                assert_eq!(
                    resolve(local, remote, initiator, initiator),
                    Ok(Resolution::CloseOlder)
                );
            }
        }
    }

    #[test]
    fn cross_direction_prefers_the_session_of_the_smaller_key() {
        let (small, large) = ordered_pair();
        // On the small side: its own session is preferred.
        assert_eq!(
            resolve(&small, &large, Initiator::Remote, Initiator::Local),
            Ok(Resolution::CloseOlder)
        );
        assert_eq!(
            resolve(&small, &large, Initiator::Local, Initiator::Remote),
            Ok(Resolution::ProbeOlder)
        );
        // On the large side: the session the peer initiated is preferred.
        assert_eq!(
            resolve(&large, &small, Initiator::Local, Initiator::Remote),
            Ok(Resolution::CloseOlder)
        );
        assert_eq!(
            resolve(&large, &small, Initiator::Remote, Initiator::Local),
            Ok(Resolution::ProbeOlder)
        );
    }

    #[test]
    fn both_ends_keep_the_same_session_whatever_the_order() {
        // T-DUP-1, T-DUP-2. The two sides may see the two sessions confirmed
        // in different orders. With both sessions alive they still agree.
        let (small, large) = ordered_pair();
        let orders = [
            (Initiator::Local, Initiator::Remote),
            (Initiator::Remote, Initiator::Local),
        ];
        for (small_older, small_newer) in orders {
            for (large_older, large_newer) in orders {
                let small_keeps =
                    kept_is_small_initiated(&small, &large, small_older, small_newer, true);
                let large_keeps =
                    kept_is_small_initiated(&large, &small, large_older, large_newer, false);
                assert!(small_keeps, "the small side keeps its own session");
                assert!(large_keeps, "the large side keeps the peer's session");
            }
        }
    }

    #[test]
    fn a_dead_preferred_session_loses_to_the_newer_one() {
        // T-DUP-7. The preferred session is half-open and does not answer.
        let (small, large) = ordered_pair();
        assert_eq!(
            resolve(&small, &large, Initiator::Local, Initiator::Remote),
            Ok(Resolution::ProbeOlder)
        );
        assert_eq!(after_probe(false), ProbeOutcome::CloseOlder);
        assert_eq!(after_probe(true), ProbeOutcome::CloseNewer);
    }

    #[test]
    fn a_session_with_oneself_is_an_error() {
        let me = identity(1);
        assert_eq!(
            resolve(&me, &me, Initiator::Local, Initiator::Remote),
            Err(ProtocolError::InvalidValue)
        );
    }
}
