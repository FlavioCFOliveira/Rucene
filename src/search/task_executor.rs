//! Concurrent task execution, ported from
//! `org.apache.lucene.search.TaskExecutor`.

#![deny(unsafe_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use crate::error::{LuceneError, Result};
use crate::util::SameThreadExecutorService;

/// A unit of work handed to an [`Executor`].
///
/// Equivalent to `java.lang.Runnable`.
pub type Command = Box<dyn FnOnce() + Send + 'static>;

/// The value a [`Callable`] computes, type-erased: Java's `Callable<T>`
/// result `T`.
pub type TaskValue = Box<dyn std::any::Any + Send>;

/// A task that returns a result and may fail.
///
/// Equivalent to `java.util.concurrent.Callable<V>`, with `V` type-erased to
/// [`TaskValue`]: [`Executor`] is a trait object, so its
/// [`submit`](Executor::submit) cannot be generic.
pub type Callable = Box<dyn FnOnce() -> Result<TaskValue> + Send + 'static>;

/// The shared completion state of a [`Future`].
struct FutureState {
    /// `None` until the task completed; then its value, or the cause of its
    /// failure.
    outcome: Mutex<Option<std::result::Result<TaskValue, LuceneError>>>,
    done: Condvar,
}

impl FutureState {
    fn complete(&self, outcome: std::result::Result<TaskValue, LuceneError>) {
        let mut slot = self.outcome.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.is_none() {
            *slot = Some(outcome);
            self.done.notify_all();
        }
    }
}

/// The result of an asynchronous computation.
///
/// Equivalent to `java.util.concurrent.Future<V>` as
/// [`Executor::submit`] returns it, with `V` type-erased to [`TaskValue`].
pub struct Future {
    state: Arc<FutureState>,
}

impl std::fmt::Debug for Future {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Future").finish_non_exhaustive()
    }
}

impl Future {
    /// A future already completed with the failure `cause`.
    ///
    /// Equivalent to `new CompletableFuture<>()` followed by
    /// `completeExceptionally(cause)`.
    pub fn failed(cause: LuceneError) -> Self {
        let state = Arc::new(FutureState {
            outcome: Mutex::new(Some(Err(cause))),
            done: Condvar::new(),
        });
        Self { state }
    }

    /// Waits for the computation to complete, then retrieves its result.
    ///
    /// Equivalent to `Future.get()`.
    ///
    /// # Errors
    ///
    /// Returns the cause of the failure, the `getCause()` of the
    /// `ExecutionException` Java throws, when the computation failed.
    pub fn get(self) -> std::result::Result<TaskValue, LuceneError> {
        let mut slot = self
            .state
            .outcome
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(outcome) = slot.take() {
                return outcome;
            }
            slot = self
                .state
                .done
                .wait(slot)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// A cancellable asynchronous computation: runs a [`Callable`] and
/// completes the [`Future`] created with it.
///
/// Equivalent to `java.util.concurrent.FutureTask<V>`, as
/// `AbstractExecutorService.newTaskFor(Callable)` creates it.
pub struct FutureTask {
    callable: Callable,
    state: Arc<FutureState>,
}

impl std::fmt::Debug for FutureTask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FutureTask").finish_non_exhaustive()
    }
}

impl FutureTask {
    /// Creates a task that runs `callable`, and the future its result
    /// completes.
    ///
    /// Equivalent to `new FutureTask<>(callable)`; the task is its own
    /// `Future` in Java.
    pub fn new(callable: Callable) -> (Self, Future) {
        let state = Arc::new(FutureState {
            outcome: Mutex::new(None),
            done: Condvar::new(),
        });
        let future = Future {
            state: Arc::clone(&state),
        };
        (Self { callable, state }, future)
    }

    /// Runs the callable and completes the future with its value, or with
    /// the throwable it raised.
    ///
    /// Equivalent to `FutureTask.run()`, which catches any `Throwable`
    /// (`setException`); a panic is this crate's carrier of an unchecked
    /// throwable, so it completes the future with
    /// [`panic_to_error`](crate::util::concurrent::panic_to_error) of its
    /// payload.
    pub fn run(self) {
        let callable = self.callable;
        let outcome = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(callable)) {
            Ok(outcome) => outcome,
            Err(payload) => Err(crate::util::concurrent::panic_to_error(payload.as_ref())),
        };
        self.state.complete(outcome);
    }
}

/// An [`Executor`] refused a command.
///
/// Equivalent to `java.util.concurrent.RejectedExecutionException`. Java's
/// caller still holds the `Runnable` it submitted and may run it itself, as
/// `TaskExecutor` does (`TaskExecutor.java:55-69`); a Rust command is moved
/// into [`Executor::execute`], so the rejection hands it back in
/// [`command`](Self::command).
pub struct RejectedExecutionException {
    /// The command that was not accepted.
    pub command: Command,
    /// The exception's message, Java's `getMessage()`.
    pub message: String,
}

impl std::fmt::Debug for RejectedExecutionException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RejectedExecutionException")
            .field("message", &self.message)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for RejectedExecutionException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RejectedExecutionException {}

/// Runs submitted work, possibly on another thread.
///
/// Equivalent to `java.util.concurrent.Executor`, which Lucene's
/// [`TaskExecutor`] is built on.
pub trait Executor: Send + Sync {
    /// Runs the given command at some point, on this executor's threads or on
    /// the calling thread.
    ///
    /// Equivalent to `Executor.execute(Runnable)`.
    ///
    /// # Errors
    ///
    /// Returns [`RejectedExecutionException`], carrying the command back, when
    /// the executor cannot accept it — for example once it is shut down.
    fn execute(&self, command: Command) -> std::result::Result<(), RejectedExecutionException>;

    /// Submits a value-returning task for execution and returns a
    /// [`Future`] representing its pending result.
    ///
    /// Equivalent to `ExecutorService.submit(Callable)` as
    /// `AbstractExecutorService` implements it (`newTaskFor(task)` then
    /// `execute(ftask)`); an executor overrides it as a Java
    /// `ExecutorService` overrides `submit`.
    ///
    /// # Errors
    ///
    /// Returns [`RejectedExecutionException`] when [`execute`](Self::execute)
    /// rejects the task.
    fn submit(&self, task: Callable) -> std::result::Result<Future, RejectedExecutionException> {
        let (task, future) = FutureTask::new(task);
        self.execute(Box::new(move || task.run()))?;
        Ok(future)
    }

    /// The executor as [`Any`](std::any::Any), for Java's `instanceof` on an
    /// `Executor`: `None` by default, `Some(self)` for the concrete types a
    /// caller tests ([`SameThreadExecutorService`]).
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }
}

impl Executor for SameThreadExecutorService {
    /// Equivalent to `SameThreadExecutorService.execute(Runnable)`
    /// (`SameThreadExecutorService.java:34-37,68-72`): once shut down, the
    /// command is rejected with `"Executor is shut down."`.
    fn execute(&self, command: Command) -> std::result::Result<(), RejectedExecutionException> {
        if self.is_shutdown() {
            return Err(RejectedExecutionException {
                command,
                message: "Executor is shut down.".to_string(),
            });
        }
        command();
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
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

    /// Checks whether every task has reached a terminal state.
    ///
    /// Equivalent to the private
    /// `TaskExecutor.assertAllFuturesCompleted(Collection)`, which
    /// `collectResults` runs through a Java `assert` right after every future
    /// has been waited on, as a debug-only sanity check that no task is left
    /// running.
    fn all_futures_completed(&self) -> bool {
        (0..self.slots.len())
            .all(|id| matches!(&*self.lock_slot(id), Slot::Done(_) | Slot::Cancelled))
    }

    /// Collects the results in submission order, or the first failure.
    ///
    /// Equivalent to `TaskExecutor.collectResults` (`TaskExecutor.java:120-137`):
    /// the subsequent failures are added to the first one as suppressed
    /// exceptions (`IOUtils.useOrSuppress`).
    fn collect_results(&self) -> Result<Vec<T>> {
        debug_assert!(
            self.all_futures_completed(),
            "Some tasks are still running?"
        );
        let mut results = Vec::with_capacity(self.slots.len());
        let mut first_error: Option<LuceneError> = None;
        for id in 0..self.slots.len() {
            let taken = std::mem::replace(&mut *self.lock_slot(id), Slot::Cancelled);
            match taken {
                Slot::Done(Ok(value)) => results.push(value),
                Slot::Done(Err(err)) => match &mut first_error {
                    None => first_error = Some(err),
                    Some(first) => first.add_suppressed(err),
                },
                Slot::Cancelled => {}
                Slot::Pending(_) | Slot::Running => {
                    if first_error.is_none() {
                        first_error = Some(LuceneError::illegal_state(
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

    /// Equivalent to the executor `new TaskExecutor(Executor)` installs
    /// (`TaskExecutor.java:55-69`): a rejected command runs on the current
    /// thread, so a rejecting executor only reduces parallelism.
    fn submit(&self, command: Command) {
        match self.executor.as_ref() {
            Some(executor) => {
                if let Err(rejected) = executor.execute(command) {
                    (rejected.command)();
                }
            }
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
