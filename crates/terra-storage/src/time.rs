//! Timestamps, which the database stores as nanoseconds since the Unix epoch.

use std::time::{Duration, SystemTime};

/// Converts a time to nanoseconds since the Unix epoch, or `None` if it is
/// outside the range an `i64` holds: roughly the years 1678 to 2262.
pub(crate) fn to_nanos(time: SystemTime) -> Option<i64> {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_nanos()).ok(),
        Err(before) => i64::try_from(before.duration().as_nanos())
            .ok()
            .map(|nanos| -nanos),
    }
}

/// Converts nanoseconds since the Unix epoch back to a time.
pub(crate) fn from_nanos(nanos: i64) -> SystemTime {
    let magnitude = Duration::from_nanos(nanos.unsigned_abs());
    if nanos >= 0 {
        SystemTime::UNIX_EPOCH + magnitude
    } else {
        SystemTime::UNIX_EPOCH - magnitude
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nanos_round_trip() {
        for nanos in [0, 1, -1, 1_704_067_200_123_456_789, i64::MAX, i64::MIN + 1] {
            assert_eq!(to_nanos(from_nanos(nanos)), Some(nanos), "{nanos}");
        }
    }

    #[test]
    fn to_nanos_rejects_times_out_of_range() {
        let far = SystemTime::UNIX_EPOCH + Duration::from_secs(300 * 365 * 24 * 60 * 60);
        assert_eq!(to_nanos(far), None);
    }
}
