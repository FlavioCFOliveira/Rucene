//! Port of the `java.util.concurrent.ThreadPoolExecutor` configuration the
//! Lucene core tests build: `new ThreadPoolExecutor(corePoolSize,
//! maximumPoolSize, keepAliveTime, unit, new LinkedBlockingQueue<Runnable>(),
//! threadFactory)`.
//!
//! **Divergence.** The JDK class takes any `BlockingQueue`; this port's queue
//! is the unbounded `LinkedBlockingQueue` those tests pass, whose `offer`
//! never fails. `execute` therefore follows the JDK's three steps with the
//! third one unreachable: below `corePoolSize` a new worker starts with the
//! command; otherwise the command is queued, and a worker is started when
//! none is alive. Core threads never time out (`allowCoreThreadTimeOut`
//! is `false`), so `keepAliveTime` only bounds threads above the core size,
//! which this queue never starts.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use randomizedtesting::jdk::thread::Thread;
use rucene::search::{Command, Executor, RejectedExecutionException};
use rucene::util::{InterruptedException, NamedThreadFactory};

use super::executor_service::ExecutorService;

/// The pool's bookkeeping, guarded as Java's `mainLock` and `ctl` guard it.
struct PoolState {
    /// The work queue, `LinkedBlockingQueue<Runnable>`.
    queue: VecDeque<Command>,
    /// The live workers, Java's `workers`.
    workers: Vec<Thread>,
    /// The number of live workers, Java's `workerCount`.
    worker_count: usize,
    /// `SHUTDOWN` or a later run state.
    shut_down: bool,
}

struct Pool {
    state: Mutex<PoolState>,
    /// Signalled when a command is queued or the pool shuts down
    /// (`LinkedBlockingQueue.notEmpty`).
    not_empty: Condvar,
    /// Signalled when the last worker exits (`termination`).
    termination: Condvar,
    core_pool_size: usize,
    thread_factory: NamedThreadFactory,
}

/// Port of `java.util.concurrent.ThreadPoolExecutor` over an unbounded
/// `LinkedBlockingQueue`.
#[derive(Clone)]
pub struct ThreadPoolExecutor {
    pool: Arc<Pool>,
}

impl std::fmt::Debug for ThreadPoolExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThreadPoolExecutor")
            .field("core_pool_size", &self.pool.core_pool_size)
            .finish_non_exhaustive()
    }
}

impl ThreadPoolExecutor {
    /// `new ThreadPoolExecutor(corePoolSize, maximumPoolSize, keepAliveTime,
    /// unit, new LinkedBlockingQueue<Runnable>(), threadFactory)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` when `corePoolSize < 0`,
    /// `maximumPoolSize <= 0` or `maximumPoolSize < corePoolSize`.
    pub fn new(
        core_pool_size: i32,
        maximum_pool_size: i32,
        keep_alive_time: Duration,
        thread_factory: NamedThreadFactory,
    ) -> Self {
        if core_pool_size < 0 || maximum_pool_size <= 0 || maximum_pool_size < core_pool_size {
            panic!("java.lang.IllegalArgumentException");
        }
        let _ = keep_alive_time;
        Self {
            pool: Arc::new(Pool {
                state: Mutex::new(PoolState {
                    queue: VecDeque::new(),
                    workers: Vec::new(),
                    worker_count: 0,
                    shut_down: false,
                }),
                not_empty: Condvar::new(),
                termination: Condvar::new(),
                core_pool_size: core_pool_size as usize,
                thread_factory,
            }),
        }
    }

    /// `addWorker(firstTask, core)`: starts a worker thread, named by the
    /// thread factory, that runs `first` and then the queue.
    fn add_worker(&self, state: &mut PoolState, first: Option<Command>) {
        state.worker_count += 1;
        let pool = Arc::clone(&self.pool);
        let thread = Thread::new(self.pool.thread_factory.next_thread_name(), move || {
            run_worker(&pool, first);
        });
        thread.start();
        state.workers.push(thread);
    }

    /// `shutdownNow()`: no new command is accepted, the queued ones are
    /// drained and returned, and the workers are interrupted.
    pub fn shutdown_now(&self) -> Vec<Command> {
        let mut state = self
            .pool
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.shut_down = true;
        let drained: Vec<Command> = state.queue.drain(..).collect();
        for worker in &state.workers {
            worker.interrupt();
        }
        self.pool.not_empty.notify_all();
        drained
    }
}

/// `runWorker(w)`: runs the first command, then takes from the queue until
/// the pool shuts down and the queue is empty.
fn run_worker(pool: &Pool, first: Option<Command>) {
    /// `processWorkerExit` of a worker whose command completed abruptly.
    struct AbruptExit<'a>(&'a Pool);
    impl Drop for AbruptExit<'_> {
        fn drop(&mut self) {
            if std::thread::panicking() {
                let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
                state.worker_count -= 1;
                if state.worker_count == 0 {
                    self.0.termination.notify_all();
                }
            }
        }
    }
    let _abrupt_exit = AbruptExit(pool);
    let mut task = first;
    loop {
        if let Some(command) = task.take() {
            command();
        }
        // `getTask()`: a core worker waits for the queue without a timeout.
        let mut state = pool.state.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(command) = state.queue.pop_front() {
                task = Some(command);
                break;
            }
            if state.shut_down {
                // `processWorkerExit`, then `tryTerminate`.
                state.worker_count -= 1;
                if state.worker_count == 0 {
                    pool.termination.notify_all();
                }
                return;
            }
            state = pool
                .not_empty
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

impl Executor for ThreadPoolExecutor {
    /// `execute(command)`: start a core worker with the command, or queue
    /// it; once shut down, the command is rejected.
    fn execute(&self, command: Command) -> Result<(), RejectedExecutionException> {
        let mut state = self
            .pool
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if state.shut_down {
            return Err(RejectedExecutionException {
                command,
                message: "Task rejected from a shut-down ThreadPoolExecutor".to_string(),
            });
        }
        if state.worker_count < self.pool.core_pool_size {
            self.add_worker(&mut state, Some(command));
            return Ok(());
        }
        state.queue.push_back(command);
        self.pool.not_empty.notify_one();
        if state.worker_count == 0 {
            self.add_worker(&mut state, None);
        }
        Ok(())
    }
}

impl ExecutorService for ThreadPoolExecutor {
    /// `shutdown()`: previously submitted commands run, no new one is
    /// accepted.
    fn shutdown(&self) {
        let mut state = self
            .pool
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.shut_down = true;
        self.pool.not_empty.notify_all();
    }

    /// `awaitTermination(timeout, unit)`.
    fn await_termination(&self, timeout: Duration) -> Result<bool, InterruptedException> {
        let deadline = Instant::now() + timeout;
        let mut state = self
            .pool
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while !(state.shut_down && state.worker_count == 0) {
            let now = Instant::now();
            if now >= deadline {
                return Ok(false);
            }
            state = self
                .pool
                .termination
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        Ok(true)
    }
}
