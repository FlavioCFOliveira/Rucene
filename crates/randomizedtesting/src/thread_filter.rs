//! Port of `com.carrotsearch.randomizedtesting.ThreadFilter`
//! (randomizedtesting 2.8.1, `ThreadFilter.java`).

use crate::jdk::thread::Thread;

/// Excludes threads from the leak checks (`ThreadFilter`).
pub trait ThreadFilter: Send + Sync {
    /// `reject(t)`: `true` to exclude `t` from the checks.
    fn reject(&self, t: &Thread) -> bool;
}
