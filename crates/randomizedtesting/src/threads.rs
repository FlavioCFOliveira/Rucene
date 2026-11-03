//! Port of `com.carrotsearch.randomizedtesting.Threads.threadName`
//! (randomizedtesting 2.8.1, `Threads.java:19-34`).
//!
//! Only `threadName` and `groupName` are ported: `RandomizedContext` and
//! `CloseableResourceInfo` call them. The thread-dump members of `Threads`
//! belong to the runner's thread-leak control (later tasks).

use crate::jdk::thread::Thread;

/// Thread utilities: `Threads` (package-private).
pub(crate) struct Threads;

impl Threads {
    /// `threadName(Thread)`: `Thread[id=N, name=..., state=..., group=...]`,
    /// JVM vendor insensitive.
    pub(crate) fn thread_name(t: &Thread) -> String {
        format!(
            "Thread[id={}, name={}, state={}, group={}]",
            t.id(),
            t.name(),
            t.state(),
            t.thread_group()
                .as_ref()
                .map(|g| g.name().to_string())
                .unwrap_or_else(|| "{null group}".to_string())
        )
    }
}
