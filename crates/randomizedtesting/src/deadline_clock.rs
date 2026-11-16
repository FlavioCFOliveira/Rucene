//! Port of `com.carrotsearch.randomizedtesting.DeadlineClock`
//! (randomizedtesting 2.8.1, `DeadlineClock.java`).

use crate::jdk::concurrent::TimeUnit;
use crate::jdk::system::nano_time;

/// A clock measuring the time left to a deadline set at construction.
///
/// Port of `DeadlineClock` (`DeadlineClock.java:5-30`).
#[derive(Debug, Clone, Copy)]
pub struct DeadlineClock {
    start_time: i64,
    duration_nanos: i64,
}

impl DeadlineClock {
    /// Port of `DeadlineClock(TimeUnit unit, long duration)`
    /// (`DeadlineClock.java:9-12`): starts now, with the deadline `duration`
    /// `unit`s away (`unit.toNanos(duration)`, saturated).
    pub fn new(unit: TimeUnit, duration: i64) -> DeadlineClock {
        DeadlineClock {
            start_time: nano_time(),
            duration_nanos: unit.to_nanos(duration),
        }
    }

    /// `isBeforeDeadline()`.
    pub fn is_before_deadline(&self) -> bool {
        !self.is_after_deadline()
    }

    /// `isAfterDeadline()`.
    pub fn is_after_deadline(&self) -> bool {
        nano_time() - self.start_time >= self.duration_nanos
    }

    /// Port of `timeUntilDeadline(TimeUnit unit)` (`DeadlineClock.java:22-29`):
    /// `0` once past the deadline, else the time left converted to `unit`
    /// (`unit.convert(durationNanos - elapsedNanos, NANOSECONDS)`, truncated).
    pub fn time_until_deadline(&self, unit: TimeUnit) -> i64 {
        let elapsed_nanos = nano_time() - self.start_time;
        if elapsed_nanos >= self.duration_nanos {
            0
        } else {
            unit.convert(self.duration_nanos - elapsed_nanos, TimeUnit::Nanoseconds)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DeadlineClock;
    use crate::jdk::concurrent::TimeUnit;

    /// The constructor converts with `toNanos` (saturating) and
    /// `timeUntilDeadline` converts the remainder to the asked unit,
    /// truncating; a non-positive duration is past its deadline at once.
    #[test]
    fn time_unit_api_matches_java() {
        let clock = DeadlineClock::new(TimeUnit::Seconds, 3600);
        assert_eq!(clock.duration_nanos, 3_600_000_000_000);
        assert!(clock.is_before_deadline());
        assert_eq!(clock.time_until_deadline(TimeUnit::Hours), 0);
        assert_eq!(clock.time_until_deadline(TimeUnit::Minutes), 59);
        let left = clock.time_until_deadline(TimeUnit::Milliseconds);
        assert!(left > 3_590_000 && left < 3_600_000, "{left}");
        assert!(clock.time_until_deadline(TimeUnit::Nanoseconds) > left * 1_000_000);

        let saturated = DeadlineClock::new(TimeUnit::Days, i64::MAX);
        assert_eq!(saturated.duration_nanos, i64::MAX);
        assert!(saturated.is_before_deadline());

        for clock in [
            DeadlineClock::new(TimeUnit::Milliseconds, 0),
            DeadlineClock::new(TimeUnit::Milliseconds, -5),
        ] {
            assert!(clock.is_after_deadline());
            assert_eq!(clock.time_until_deadline(TimeUnit::Nanoseconds), 0);
        }
        assert_eq!(
            DeadlineClock::new(TimeUnit::Microseconds, -5).duration_nanos,
            -5_000
        );
    }
}
