//! Port of `com.carrotsearch.randomizedtesting.RunnerContainer`
//! (randomizedtesting 2.8.1, `RunnerContainer.java`).

/// The container the runner runs in (`RunnerContainer`): some containers
/// make assumptions about runners.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunnerContainer {
    /// `ECLIPSE`.
    #[allow(dead_code)] // Detected from the JVM's stack only; see RandomizedRunner.
    Eclipse,
    /// `IDEA`.
    #[allow(dead_code)] // Detected from the JVM's stack only; see RandomizedRunner.
    Idea,
    /// `UNKNOWN`.
    Unknown,
}
