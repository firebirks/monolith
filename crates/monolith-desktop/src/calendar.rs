//! The local calendar of the system: its time zone, from its zoneinfo on
//! Linux and from a database compiled in on Windows.

use std::time::{SystemTime, UNIX_EPOCH};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use monolith_ui_model::present::{Calendar, LocalTime, utc};

/// The calendar of the system's time zone.
pub struct SystemCalendar {
    zone: TimeZone,
}

impl core::fmt::Debug for SystemCalendar {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SystemCalendar").finish_non_exhaustive()
    }
}

impl SystemCalendar {
    /// The system's time zone, or UTC if it cannot be read.
    pub fn new() -> Self {
        Self {
            zone: TimeZone::try_system().unwrap_or(TimeZone::UTC),
        }
    }
}

impl Default for SystemCalendar {
    fn default() -> Self {
        Self::new()
    }
}

impl Calendar for SystemCalendar {
    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_secs())
            .unwrap_or(0)
    }

    fn local(&self, seconds: u64) -> LocalTime {
        let Some(time) = i64::try_from(seconds)
            .ok()
            .and_then(|seconds| Timestamp::from_second(seconds).ok())
        else {
            return utc(seconds);
        };
        let local = self.zone.to_datetime(time);
        let narrow = |value: i8| u8::try_from(value).unwrap_or(0);
        LocalTime {
            year: i32::from(local.year()),
            month: narrow(local.month()),
            day: narrow(local.day()),
            hour: narrow(local.hour()),
            minute: narrow(local.minute()),
            weekday: narrow(local.weekday().to_monday_zero_offset()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_utc_it_agrees_with_the_fallback() {
        let calendar = SystemCalendar {
            zone: TimeZone::UTC,
        };
        for seconds in [0, 86_399, 951_782_400, 1_792_002_600, 4_102_444_800] {
            assert_eq!(calendar.local(seconds), utc(seconds), "{seconds}");
        }
    }
}
