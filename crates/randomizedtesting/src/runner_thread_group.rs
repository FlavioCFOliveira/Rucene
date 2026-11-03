//! Port of `com.carrotsearch.randomizedtesting.RunnerThreadGroup`
//! (randomizedtesting 2.8.1, `RunnerThreadGroup.java`).

use std::sync::Arc;

use crate::jdk::thread::ThreadGroup;
use crate::randomized_runner::{augment_stack_trace, try_run};

/// The thread group a suite runs in (`RunnerThreadGroup`, package-private):
/// it augments uncaught exceptions with the seed before passing them on.
pub(crate) struct RunnerThreadGroup;

impl RunnerThreadGroup {
    /// `new RunnerThreadGroup(name)`: a child of the current thread's group.
    #[allow(clippy::new_ret_no_self)] // A ThreadGroup subclass is a ThreadGroup here.
    pub(crate) fn new(name: &str) -> Arc<ThreadGroup> {
        ThreadGroup::with_uncaught_hook(
            name,
            Arc::new(|group, t, e| {
                // The seed of the context of the dying thread; when the
                // context is gone, the failure is suppressed on `e`.
                let original = e.clone();
                let e = match try_run(|| augment_stack_trace(e, &[])) {
                    Ok(a) => a,
                    Err(ignore) => {
                        let mut o = original;
                        o.add_suppressed(ignore);
                        o
                    }
                };
                group.default_uncaught(t, e);
            }),
        )
    }
}
