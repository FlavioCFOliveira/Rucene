//! Port of `com.carrotsearch.randomizedtesting.DeadlineClock`
//! (randomizedtesting 2.8.1, `DeadlineClock.java`).

use crate::jdk::system::nano_time;

/// A clock measuring time to a deadline (`DeadlineClock`), in
/// milliseconds as the runner uses it.
#[derive(Debug, Clone, Copy)]
pub struct DeadlineClock {
    start_time: i64,
    duration_nanos: i64,
}

impl DeadlineClock {
    /// `new DeadlineClock(TimeUnit.MILLISECONDS, duration)`.
    pub fn millis(duration: i64) -> DeadlineClock {
        DeadlineClock {
            start_time: nano_time(),
            duration_nanos: duration.saturating_mul(1_000_000),
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

    /// `timeUntilDeadline(TimeUnit.MILLISECONDS)`: 0 once past the deadline.
    pub fn time_until_deadline_millis(&self) -> i64 {
        let elapsed = nano_time() - self.start_time;
        if elapsed >= self.duration_nanos {
            0
        } else {
            (self.duration_nanos - elapsed) / 1_000_000
        }
    }
}
