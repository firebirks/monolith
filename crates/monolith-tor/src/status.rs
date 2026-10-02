//! What Monolith knows about the state of Tor.

use core::cmp::Ordering;
use core::fmt;

/// A Tor version: `major.minor.micro.patch`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TorVersion {
    parts: [u16; 4],
}

/// The oldest Tor with every feature Monolith uses: the proof-of-work
/// keywords of `ADD_ONION` and the `<torS0X>0` SOCKS isolation format
/// (`docs/TOR_INTEGRATION.md` section 1).
pub const FEATURE_BASELINE: TorVersion = TorVersion::new(0, 4, 9, 5);

/// The release recommended at the time of writing; it fixes
/// TROVE-2026-053. Older releases at or above the baseline work, but a
/// diagnostic says that they are older.
pub const RECOMMENDED: TorVersion = TorVersion::new(0, 4, 9, 13);

impl TorVersion {
    /// A version from its four numbers.
    pub const fn new(major: u16, minor: u16, micro: u16, patch: u16) -> Self {
        Self {
            parts: [major, minor, micro, patch],
        }
    }

    /// Parses the version Tor reports, such as `0.4.9.13`,
    /// `0.5.0.1-alpha` or `0.4.9.13 (git-0123abcd)`. Four numbers separated
    /// by dots, each at most five digits, optionally followed by `-` or a
    /// space and anything else, which is ignored.
    pub fn parse(text: &[u8]) -> Option<Self> {
        let end = text
            .iter()
            .position(|byte| *byte == b'-' || *byte == b' ')
            .unwrap_or(text.len());
        let numbers = text.get(..end)?;
        let mut parts = [0_u16; 4];
        let mut count = 0_usize;
        for field in numbers.split(|byte| *byte == b'.') {
            if field.is_empty() || field.len() > 5 || !field.iter().all(u8::is_ascii_digit) {
                return None;
            }
            let value = core::str::from_utf8(field).ok()?.parse::<u16>().ok()?;
            *parts.get_mut(count)? = value;
            count = count.saturating_add(1);
        }
        (count == 4).then_some(Self { parts })
    }

    /// Returns the four numbers.
    pub const fn parts(&self) -> [u16; 4] {
        self.parts
    }
}

impl PartialOrd for TorVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TorVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.parts.cmp(&other.parts)
    }
}

impl fmt::Display for TorVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [major, minor, micro, patch] = self.parts;
        write!(f, "{major}.{minor}.{micro}.{patch}")
    }
}

/// Bootstrap progress, as far as Monolith reads it.
///
/// Only the number and whether Tor says it is done are kept. Readiness
/// does not depend on this: the specification guarantees only the tags
/// `starting` and `done`, and Tor may revisit earlier phases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bootstrap {
    /// Tor did not say, or a filter refused the question.
    Unknown,
    /// Percent, 0 to 99, or 100 without the `done` tag.
    InProgress(u8),
    /// Tor reported the `done` tag.
    Done,
}

/// What the control endpoint said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlStatus {
    /// The control endpoint could not be reached.
    Unavailable,
    /// It was reached, but did not offer the configured authentication.
    AuthenticationUnavailable,
    /// Authentication failed.
    AuthenticationFailed,
    /// It answered with something that is not a control reply Monolith
    /// accepts.
    InvalidResponse,
    /// It reported a Tor below the feature baseline.
    UnsupportedVersion(TorVersion),
    /// It answered every question.
    Reachable {
        /// The version reported by `PROTOCOLINFO`.
        version: TorVersion,
        /// Whether Tor has established a circuit.
        circuit_established: bool,
        /// Bootstrap progress.
        bootstrap: Bootstrap,
    },
}

/// What the SOCKS endpoint said to a greeting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocksStatus {
    /// It could not be reached.
    Unavailable,
    /// It answered, but not with a SOCKS5 reply that selects username and
    /// password authentication.
    InvalidResponse,
    /// It answered as Monolith needs.
    Reachable,
}

/// The state of Tor as a status query found it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TorStatus {
    /// The control endpoint.
    pub control: ControlStatus,
    /// The SOCKS endpoint.
    pub socks: SocksStatus,
}

/// A summary of [`TorStatus`] for the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    /// Neither endpoint works. Monolith is offline.
    Unavailable,
    /// The control endpoint does not work: no Onion Service can be
    /// published.
    ControlUnavailable,
    /// The SOCKS endpoint does not work: no peer can be dialed.
    SocksUnavailable,
    /// Tor is reachable but has no circuit yet.
    NotReady(Bootstrap),
    /// Tor has a circuit and both endpoints work.
    Ready,
}

impl TorStatus {
    /// Summarizes the status.
    pub const fn readiness(&self) -> Readiness {
        let socks = matches!(self.socks, SocksStatus::Reachable);
        match self.control {
            ControlStatus::Reachable {
                circuit_established,
                bootstrap,
                ..
            } => {
                if !socks {
                    Readiness::SocksUnavailable
                } else if circuit_established {
                    Readiness::Ready
                } else {
                    Readiness::NotReady(bootstrap)
                }
            }
            _ => {
                if socks {
                    Readiness::ControlUnavailable
                } else {
                    Readiness::Unavailable
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_as_tor_reports_them() {
        assert_eq!(
            TorVersion::parse(b"0.4.9.13"),
            Some(TorVersion::new(0, 4, 9, 13))
        );
        assert_eq!(
            TorVersion::parse(b"0.5.0.1-alpha"),
            Some(TorVersion::new(0, 5, 0, 1))
        );
        assert_eq!(
            TorVersion::parse(b"0.4.9.13 (git-0123abcd)"),
            Some(TorVersion::new(0, 4, 9, 13))
        );
        for bad in [
            &b""[..],
            b"0.4.9",
            b"0.4.9.13.1",
            b"0..9.13",
            b"a.4.9.13",
            b"0.4.9.999999",
            b"0.4.9.70000",
            b"-0.4.9.13",
            b" 0.4.9.13",
        ] {
            assert_eq!(TorVersion::parse(bad), None, "{bad:?}");
        }
        assert_eq!(TorVersion::new(0, 4, 9, 13).to_string(), "0.4.9.13");
    }

    #[test]
    fn versions_order_by_their_numbers() {
        assert!(TorVersion::new(0, 4, 9, 4) < FEATURE_BASELINE);
        assert!(TorVersion::new(0, 4, 8, 25) < FEATURE_BASELINE);
        assert!(TorVersion::new(0, 4, 9, 5) == FEATURE_BASELINE);
        assert!(TorVersion::new(0, 4, 10, 0) > FEATURE_BASELINE);
        assert!(TorVersion::new(0, 5, 0, 1) > RECOMMENDED);
        assert!(FEATURE_BASELINE < RECOMMENDED);
    }

    #[test]
    fn readiness_follows_both_endpoints() {
        let reachable = |circuit_established| ControlStatus::Reachable {
            version: RECOMMENDED,
            circuit_established,
            bootstrap: Bootstrap::InProgress(40),
        };
        let status = |control, socks| TorStatus { control, socks }.readiness();
        assert_eq!(
            status(reachable(true), SocksStatus::Reachable),
            Readiness::Ready
        );
        assert_eq!(
            status(reachable(false), SocksStatus::Reachable),
            Readiness::NotReady(Bootstrap::InProgress(40))
        );
        assert_eq!(
            status(reachable(true), SocksStatus::Unavailable),
            Readiness::SocksUnavailable
        );
        assert_eq!(
            status(ControlStatus::Unavailable, SocksStatus::Reachable),
            Readiness::ControlUnavailable
        );
        assert_eq!(
            status(
                ControlStatus::AuthenticationFailed,
                SocksStatus::InvalidResponse
            ),
            Readiness::Unavailable
        );
    }
}
