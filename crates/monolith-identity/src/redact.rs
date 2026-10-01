//! Wrappers that keep sensitive values out of log output.
//!
//! Invariant S18 forbids secrets, messages, onion addresses, identity keys
//! and filenames in ordinary logs. Types that carry such values do not derive
//! `Debug` or implement `Display`. They implement `Debug` by hand and print a
//! fixed label, or they are wrapped in [`Redacted`].

use core::fmt;

/// Text printed in place of a redacted value.
pub const REDACTED: &str = "[redacted]";

/// Holds a value whose `Debug` and `Display` output is a fixed label.
///
/// The inner value is reachable only through [`Redacted::expose`], so every
/// use that could end up in output is visible at the call site.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Redacted<T>(T);

impl<T> Redacted<T> {
    /// Wraps a value.
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    /// Returns the inner value. Callers must not log it.
    pub const fn expose(&self) -> &T {
        &self.0
    }

    /// Unwraps the inner value. Callers must not log it.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> fmt::Debug for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl<T> fmt::Display for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_display_hide_the_value() {
        let wrapped = Redacted::new("correct horse battery staple");
        assert_eq!(format!("{wrapped:?}"), REDACTED);
        assert_eq!(format!("{wrapped}"), REDACTED);
        assert_eq!(format!("{wrapped:#?}"), REDACTED);
    }

    #[test]
    fn expose_returns_the_value() {
        let wrapped = Redacted::new(7_u8);
        assert_eq!(*wrapped.expose(), 7);
        assert_eq!(wrapped.into_inner(), 7);
    }
}
