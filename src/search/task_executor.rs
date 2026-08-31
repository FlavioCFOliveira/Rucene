//! Concurrent task execution, ported from
//! `org.apache.lucene.search.TaskExecutor`.

#![deny(unsafe_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use crate::error::{LuceneError, Result};
use crate::util::SameThreadExecutorService;

/// Runs submitted work, possibly on another thread.
///
/// Equivalent to `java.util.concurrent.Executor`, which Lucene's
/// [`TaskExecutor`] is built on. An implementation that cannot accept a command
/// must run it on the calling thread, so that a rejecting executor only reduces
/// parallelism and never loses work — that is exactly the guarantee Java's
/// `TaskExecutor` constructor installs by catching `RejectedExecutionException`.
pub trait Executor: Send + Sync {
    /// Runs the given command at some point, on this executor's threads or on
    /// the calling thread.
    ///
    /// Equivalent to `Executor.execute(Runnable)`.
    fn execute(&self, command: Box<dyn FnOnce() + Send + 'static>);
}

impl Executor for SameThreadExecutorService {
    fn execute(&self, command: Box<dyn FnOnce() + Send + 'static>) {
        if self.is_shutdown() {
            // A shut-down service rejects the command without running it. Java's
            // TaskExecutor wrapper reacts to a rejection by running the command
            // on the calling thread, which is what this service would have done
            // anyway.
            command();
        } else {
            let _ = SameThreadExecutorService::execute(self, command);
        }
    }
}

/// The state of one submitted task.
///
/// Equivalent to what Java's `TaskExecutor.Task` — a `FutureTask` with a
/// `startedOrCancelled` flag — holds.
enum Slot<T> {
    /// Not started and not cancelled yet.
    Pending(Box<dyn FnOnce() -> Result<T> + Send + 'static>),
    /// Claimed by a thread and currently running.
    Running,
    /// Finished, successfully or not.
    Done(Result<T>),
    /// Cancelled before it started; it has no result to return, which is fine
    /// because a cancellation only ever happens after another task failed and
    /// the results are about to be discarded.
    Cancelled,
}

/// The tasks of one [`TaskExecutor::invoke_all`] call and their shared
/// bookkeeping.
struct TaskGroup<T> {
    slots: Vec<Mutex<Slot<T>>>,
    /// The index of the first task that no thread has claimed yet.
    ///
    /// Equivalent to the `AtomicInteger taskId` of `TaskExecutor.invokeAll`.
    task_id: AtomicUsize,
    /// How many slots have not reached a terminal state yet.
    remaining: Mutex<usize>,
    settled: Condvar,
}

impl<T> TaskGroup<T> {
    fn new(callables: Vec<Box<dyn FnOnce() -> Result<T> + Send + 'static>>) -> Self {
        let count = callables.len();
        Self {
            slots: callables
                .into_iter()
                .map(|callable| Mutex::new(Slot::Pending(callable)))
                .collect(),
            task_id: AtomicUsize::new(0),
            remaining: Mutex::new(count),
            settled: Condvar::new(),
        }
    }

    fn lock_slot(&self, id: usize) -> std::sync::MutexGuard<'_, Slot<T>> {
        self.slots[id]
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn settle_one(&self) {
        let mut remaining = self
            .remaining
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *remaining -= 1;
        if *remaining == 0 {
            self.settled.notify_all();
        }
    }

    /// Runs the task at `id`, unless it has already been started or cancelled.
    ///
    /// Equivalent to `TaskExecutor.Task.run()`, whose
    /// `startedOrCancelled.compareAndSet(false, true)` guard is the transition
    /// out of [`Slot::Pending`] here.
    fn run(&self, id: usize) {
        let callable = {
            let mut slot = self.lock_slot(id);
            match std::mem::replace(&mut *slot, Slot::Running) {
                Slot::Pending(callable) => Some(callable),
                other => {
                    *slot = other;
                    None
                }
            }
        };

        let Some(callable) = callable else {
            return;
        };

        let outcome = callable();
        let failed = outcome.is_err();
        *self.lock_slot(id) = Slot::Done(outcome);
        self.settle_one();

        if failed {
            // Equivalent to TaskExecutor.Task.setException, which cancels every
            // sibling so that no needless computation is performed: their
            // results would not be exposed anyway.
            self.cancel_all();
        }
    }

    /// Cancels every task that has not started.
    ///
    /// Equivalent to the private `TaskExecutor.cancelAll`, whose `cancel(false)`
    /// completes a not-yet-started task with a `null` result and leaves a
    /// running one alone.
    fn cancel_all(&self) {
        for id in 0..self.slots.len() {
            let cancelled = {
                let mut slot = self.lock_slot(id);
                if matches!(&*slot, Slot::Pending(_)) {
                    *slot = Slot::Cancelled;
                    true
                } else {
                    false
                }
            };
            if cancelled {
                self.settle_one();
            }
        }
    }

    /// Blocks until every task has reached a terminal state.
    ///
    /// Equivalent to the `future.get()` calls in
    /// `TaskExecutor.collectResults`, which are what make `invokeAll` leave no
    /// running task behind.
    fn await_settled(&self) {
        let mut remaining = self
            .remaining
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while *remaining > 0 {
            remaining = self
                .settled
                .wait(remaining)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Collects the results in submission order, or the first failure.
    ///
    /// Equivalent to `TaskExecutor.collectResults`. Java adds the subsequent
    /// failures to the first one as suppressed exceptions; [`LuceneError`] has
    /// no suppression channel, so the first failure is returned and the others
    /// are dropped, exactly as Java's rethrown exception hides them from the
    /// caller.
    fn collect_results(&self) -> Result<Vec<T>> {
        let mut results = Vec::with_capacity(self.slots.len());
        let mut first_error: Option<LuceneError> = None;
        for id in 0..self.slots.len() {
            let taken = std::mem::replace(&mut *self.lock_slot(id), Slot::Cancelled);
            match taken {
                Slot::Done(Ok(value)) => results.push(value),
                Slot::Done(Err(err)) => {
                    if first_error.is_none() {
                        first_error = Some(err);
                    }
                }
                Slot::Cancelled => {}
                Slot::Pending(_) | Slot::Running => {
                    if first_error.is_none() {
                        first_error = Some(LuceneError::IllegalState(
                            "some tasks are still running".to_string(),
                        ));
                    }
                }
            }
        }
        match first_error {
            Some(err) => Err(err),
            None => Ok(results),
        }
    }
}

/// Executor wrapper responsible for the execution of concurrent tasks.
///
/// Equivalent to the `final org.apache.lucene.search.TaskExecutor`, used to
/// parallelise search across segments as well as query rewrite in some cases.
/// [`invoke_all`](Self::invoke_all) takes a collection of tasks and executes
/// them concurrently: once all but one have been submitted to the executor, it
/// runs as many as it can on the calling thread, then waits for the ones
/// running in parallel and returns the results.
///
/// **Divergence from Lucene 10.5.0.** Java's `invokeAll` accepts
/// `Callable<T>`s that may capture arbitrary references, because the JVM's
/// garbage collector keeps them alive. Rust has no such guarantee for a task
/// handed to a `dyn Executor` that may outlive the call, so the tasks must be
/// `'static`; callers share what they need through [`Arc`] instead, which is
/// what [`IndexSearcher`](crate::search::IndexSearcher) does.
pub struct TaskExecutor {
    executor: Option<Arc<dyn Executor>>,
}

impl std::fmt::Debug for TaskExecutor {
    /// Renders as `TaskExecutor.toString()` does, naming the executor.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let executor = if self.executor.is_some() {
            "executor"
        } else {
            "current thread"
        };
        write!(f, "TaskExecutor(executor={executor})")
    }
}

impl TaskExecutor {
    /// Creates a task executor running its tasks on the given executor.
    ///
    /// Equivalent to `new TaskExecutor(Executor)`.
    pub fn new(executor: Arc<dyn Executor>) -> Self {
        Self {
            executor: Some(executor),
        }
    }

    /// Creates a task executor running every task on the calling thread.
    ///
    /// Equivalent to `new TaskExecutor(Runnable::run)`, which is what
    /// `IndexSearcher` installs when no executor is supplied.
    pub fn same_thread() -> Self {
        Self { executor: None }
    }

    fn submit(&self, command: Box<dyn FnOnce() + Send + 'static>) {
        match self.executor.as_ref() {
            Some(executor) => executor.execute(command),
            None => command(),
        }
    }

    /// Executes all the given tasks, waits for them to complete and returns the
    /// obtained results.
    ///
    /// Equivalent to `TaskExecutor.invokeAll(Collection<Callable<T>>)`. If more
    /// than one task fails, the first failure is returned. Additionally, when
    /// one task fails, every task of the same group that has not started yet is
    /// cancelled, to avoid needless computation whose results would not be
    /// exposed anyway.
    ///
    /// # Errors
    ///
    /// Returns the first failure raised by any of the tasks.
    pub fn invoke_all<T>(
        &self,
        callables: Vec<Box<dyn FnOnce() -> Result<T> + Send + 'static>>,
    ) -> Result<Vec<T>>
    where
        T: Send + 'static,
    {
        let count = callables.len();
        if count == 0 {
            return Ok(Vec::new());
        }
        let group = Arc::new(TaskGroup::new(callables));

        // Fork count - 1 tasks, so that at least one task runs on the current
        // thread, minimising needless forking and blocking of that thread.
        if count > 1 {
            for _ in 0..count - 1 {
                let group = Arc::clone(&group);
                self.submit(Box::new(move || {
                    let id = group.task_id.fetch_add(1, Ordering::SeqCst);
                    if id < count {
                        group.run(id);
                    }
                }));
            }
        }

        // Run as many tasks as possible on the current thread, to minimise
        // context switching for long-running concurrent tasks and to avoid
        // dead-locking when the current thread belongs to an executor with
        // limited or no parallelism.
        loop {
            let id = group.task_id.fetch_add(1, Ordering::SeqCst);
            if id >= count {
                break;
            }
            group.run(id);
            if id >= count - 1 {
                // Save a redundant compare-and-swap when this was the last task.
                break;
            }
        }

        group.await_settled();
        // The results are taken out of the shared slots rather than out of an
        // unwrapped `Arc`: a forked runnable that claimed an out-of-range id
        // may still be holding its handle even though every task has settled.
        group.collect_results()
    }
}
