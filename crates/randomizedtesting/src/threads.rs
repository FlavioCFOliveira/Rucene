//! Port of `com.carrotsearch.randomizedtesting.Threads`
//! (randomizedtesting 2.8.1, `Threads.java`).
//!
//! **Divergence.** `append(StringBuilder, ThreadInfo)` (`Threads.java:50`) formats a
//! `java.lang.management.ThreadInfo` of the JVM's thread MXBean; Rust has
//! no thread MXBean, so it is not provided and the runner's timeout dumps
//! use the stack-less thread list (`formatThreadStacks`).

use std::sync::Arc;

use crate::jdk::exception::runtime;
use crate::jdk::thread::{Thread, ThreadGroup};

/// Thread utilities: `Threads` (package-private).
///
/// Public here because the runner's own tests (`Utils`, same package in
/// Java) reach it from the ported integration test target outside the crate.
pub struct Threads;

impl Threads {
    /// `threadName(Thread)`: `Thread[id=N, name=..., state=..., group=...]`,
    /// JVM vendor insensitive.
    pub fn thread_name(t: &Thread) -> String {
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

    /// `getAllThreads()`: the threads of the root group, recursively.
    pub fn get_all_threads() -> Vec<Thread> {
        Threads::get_threads(&Threads::get_top_thread_group())
    }

    /// `getThreads(ThreadGroup)`: the live threads of `tg` and its
    /// descendants.
    pub fn get_threads(tg: &ThreadGroup) -> Vec<Thread> {
        tg.enumerate()
    }

    /// `getTopThreadGroup()`: the root of the current thread's group tree.
    pub fn get_top_thread_group() -> Arc<ThreadGroup> {
        let Some(mut tg) = Thread::current().thread_group() else {
            runtime(format!(
                "No root ThreadGroup for thread: {}",
                Thread::current()
            ))
        };
        while let Some(p) = tg.parent().cloned() {
            tg = p;
        }
        tg
    }
}
