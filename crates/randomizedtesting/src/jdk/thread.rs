//! `java.lang.Thread` and `java.lang.ThreadGroup`, as far as
//! randomizedtesting observes them.
//!
//! `RandomizedContext` finds a thread's context through the thread's
//! `ThreadGroup` and its parents, keeps per-thread state keyed by the
//! `Thread` (weakly), and formats threads in its messages; the runner's
//! thread-leak control enumerates the live threads of a group, interrupts
//! them, joins them with a timeout, and turns their uncaught exceptions into
//! failures. Rust threads have no group, no Java identifier, no state and no
//! interruption, so this module gives every thread a [`Thread`] handle that
//! carries them.
//!
//! **Divergences.**
//! * A Java thread inherits its creator's group. Here a thread created with
//!   [`Thread::new`] / [`Thread::with_group`] and started with
//!   [`Thread::start`] does so, and so does a thread whose creator captured
//!   [`Thread::start_context`] and which runs its body through it (`rucene`
//!   does so on every thread it starts once the hook is installed with
//!   `rucene::util::concurrent::set_thread_start_hook`); it is then adopted
//!   in its creator's group, with its creator's daemon status. Any other
//!   thread (for example one from a bare `std::thread::spawn`) is adopted on
//!   first use in the `main` group, whose parent is `system`, as the JVM's
//!   main thread is.
//! * [`ThreadGroup::enumerate`] (`ThreadGroup.enumerate(Thread[], true)`)
//!   sees the threads started with [`Thread::start`] and the threads adopted
//!   through [`Thread::start_context`]: as a Java `Thread` of the group, an
//!   adopted thread is live while its body runs and leaves the live set when
//!   the body ends (`Threads.java:103-116` of randomizedtesting enumerates
//!   the group's live threads), so a `rucene` thread left running counts as
//!   leaked. A thread adopted on first use by [`Thread::current`] (one the
//!   Rust test harness owns, or a bare `std::thread::spawn`) is not
//!   enumerated, so threads the harness owns never count as leaked threads.
//! * Thread identifiers come from a process-wide counter starting at 1; they
//!   are unique like Java's, but not the numbers a JVM would assign.
//! * Only the states `NEW`, `RUNNABLE` and `TERMINATED` are observable: a
//!   blocked thread still reports `RUNNABLE`.
//! * Interruption is cooperative, as Java's is for code that does not block:
//!   [`Thread::interrupt`] sets the flag and wakes the blocking calls of this
//!   module ([`Thread::sleep`], [`Thread::join_millis`],
//!   [`Thread::park_nanos`]), which then throw (`park_nanos` only returns)
//!   `InterruptedException`; a thread blocked anywhere else sees only the
//!   flag.
//! * [`Thread::get_stack_trace`] is empty: Rust cannot walk another thread's
//!   stack.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU8, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard, RwLock, Weak};
use std::time::{Duration, Instant};

use super::exception::{self, Throwable};

/// `Thread.UncaughtExceptionHandler`.
pub trait UncaughtExceptionHandler: Send + Sync {
    /// `uncaughtException(Thread t, Throwable e)`.
    fn uncaught_exception(&self, t: &Thread, e: Throwable);
}

/// The override of `ThreadGroup.uncaughtException` a `ThreadGroup`
/// subclass declares (`RunnerThreadGroup`): it receives the group, the
/// thread and the throwable, and may call [`ThreadGroup::default_uncaught`].
pub type GroupUncaughtHook = Arc<dyn Fn(&ThreadGroup, &Thread, Throwable) + Send + Sync>;

/// `java.lang.ThreadGroup`: a named node of the group tree.
pub struct ThreadGroup {
    name: String,
    parent: Option<Arc<ThreadGroup>>,
    uncaught: Option<GroupUncaughtHook>,
}

impl fmt::Debug for ThreadGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ThreadGroup")
            .field("name", &self.name)
            .field("parent", &self.parent.as_ref().map(|p| p.name.clone()))
            .finish()
    }
}

impl ThreadGroup {
    /// `new ThreadGroup(parent, name)`.
    pub fn new(parent: &Arc<ThreadGroup>, name: impl Into<String>) -> Arc<ThreadGroup> {
        Arc::new(ThreadGroup {
            name: name.into(),
            parent: Some(Arc::clone(parent)),
            uncaught: None,
        })
    }

    /// `new ThreadGroup(name)`: a child of the current thread's group.
    pub fn new_in_current(name: impl Into<String>) -> Arc<ThreadGroup> {
        let parent = Thread::current()
            .thread_group()
            .unwrap_or_else(|| Arc::clone(ThreadGroup::main()));
        ThreadGroup::new(&parent, name)
    }

    /// A subclass of `ThreadGroup` (a child of the current thread's group)
    /// that overrides `uncaughtException` with `hook`.
    pub fn with_uncaught_hook(
        name: impl Into<String>,
        hook: GroupUncaughtHook,
    ) -> Arc<ThreadGroup> {
        let parent = Thread::current()
            .thread_group()
            .unwrap_or_else(|| Arc::clone(ThreadGroup::main()));
        Arc::new(ThreadGroup {
            name: name.into(),
            parent: Some(parent),
            uncaught: Some(hook),
        })
    }

    /// `ThreadGroup.getName()`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// `ThreadGroup.getParent()`.
    pub fn parent(&self) -> Option<&Arc<ThreadGroup>> {
        self.parent.as_ref()
    }

    /// Whether this group is `other` or one of its descendants
    /// (`other.parentOf(this)`).
    pub fn is_descendant_of(&self, other: &ThreadGroup) -> bool {
        let mut g = Some(self);
        while let Some(group) = g {
            if std::ptr::eq(group, other) {
                return true;
            }
            g = group.parent.as_deref();
        }
        false
    }

    /// `ThreadGroup.uncaughtException(t, e)`: the subclass override when
    /// there is one, else [`default_uncaught`](Self::default_uncaught).
    pub fn uncaught_exception(&self, t: &Thread, e: Throwable) {
        match &self.uncaught {
            Some(hook) => hook(self, t, e),
            None => self.default_uncaught(t, e),
        }
    }

    /// `ThreadGroup.uncaughtException` as `ThreadGroup` itself implements
    /// it: delegate to the parent group; at the root, to the default
    /// handler; without one, print `Exception in thread "name" ` and the
    /// throwable to `System.err`.
    pub fn default_uncaught(&self, t: &Thread, e: Throwable) {
        if let Some(parent) = &self.parent {
            parent.uncaught_exception(t, e);
        } else if let Some(handler) = Thread::get_default_uncaught_exception_handler() {
            handler.uncaught_exception(t, e);
        } else if !e.is_instance("java.lang.ThreadDeath") {
            super::system::err_print(&format!(
                "Exception in thread \"{}\" {}\n",
                t.name(),
                super::system::format_stack_trace(&e)
            ));
        }
    }

    /// `ThreadGroup.enumerate(Thread[] list, true)`, as a set: the live
    /// threads of this group and of its descendants, in start order.
    pub fn enumerate(&self) -> Vec<Thread> {
        live_threads()
            .into_iter()
            .filter(|t| t.thread_group().is_some_and(|g| g.is_descendant_of(self)))
            .collect()
    }

    /// The JVM's `main` group (child of `system`), the group of threads this
    /// module did not start.
    pub fn main() -> &'static Arc<ThreadGroup> {
        static MAIN: LazyLock<Arc<ThreadGroup>> = LazyLock::new(|| {
            let system = Arc::new(ThreadGroup {
                name: "system".to_string(),
                parent: None,
                uncaught: None,
            });
            ThreadGroup::new(&system, "main")
        });
        &MAIN
    }
}

/// `java.lang.Thread.State`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum State {
    /// `NEW`: created, not started.
    New,
    /// `RUNNABLE`: started, not terminated.
    Runnable,
    /// `TERMINATED`: its body returned or panicked.
    Terminated,
}

impl State {
    fn from_u8(v: u8) -> State {
        match v {
            0 => State::New,
            1 => State::Runnable,
            _ => State::Terminated,
        }
    }
}

impl fmt::Display for State {
    /// `Enum.toString()`: the constant's name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            State::New => "NEW",
            State::Runnable => "RUNNABLE",
            State::Terminated => "TERMINATED",
        })
    }
}

type Body = Box<dyn FnOnce() + Send>;

/// The thread's blocking state: the interrupt flag, the `LockSupport`
/// permit and termination, guarded by one mutex so that every blocking
/// call of this module wakes on any of them.
#[derive(Default)]
struct Signals {
    interrupted: bool,
    permit: bool,
}

struct ThreadInner {
    id: i64,
    name: Mutex<String>,
    group: Mutex<Option<Arc<ThreadGroup>>>,
    state: AtomicU8,
    priority: AtomicI32,
    daemon: AtomicBool,
    body: Mutex<Option<Body>>,
    handle: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// The OS thread's identity, once started.
    os_thread_id: std::sync::OnceLock<std::thread::ThreadId>,
    signals: Mutex<Signals>,
    wake: Condvar,
    handler: Mutex<Option<Arc<dyn UncaughtExceptionHandler>>>,
}

/// A `java.lang.Thread`: a cheap, clonable handle whose equality is
/// identity.
#[derive(Clone)]
pub struct Thread(Arc<ThreadInner>);

/// A `WeakReference<Thread>`.
#[derive(Clone)]
pub struct WeakThread(Weak<ThreadInner>);

static NEXT_ID: AtomicI64 = AtomicI64::new(1);
static NEXT_THREAD_NUM: AtomicI64 = AtomicI64::new(0);

/// The threads started with [`Thread::start`] that have not terminated, in
/// start order.
static LIVE: Mutex<Vec<Thread>> = Mutex::new(Vec::new());

/// `Thread.defaultUncaughtExceptionHandler`.
static DEFAULT_HANDLER: RwLock<Option<Arc<dyn UncaughtExceptionHandler>>> = RwLock::new(None);

fn live() -> MutexGuard<'static, Vec<Thread>> {
    LIVE.lock().unwrap_or_else(|e| e.into_inner())
}

fn live_threads() -> Vec<Thread> {
    live().clone()
}

/// Marks the thread terminated when its thread-local slot is destroyed,
/// which is when the OS thread exits.
struct CurrentSlot(Thread);

impl Drop for CurrentSlot {
    fn drop(&mut self) {
        self.0.terminate();
    }
}

thread_local! {
    static CURRENT: std::cell::RefCell<Option<CurrentSlot>> = const { std::cell::RefCell::new(None) };
}

impl Thread {
    /// `Thread.MIN_PRIORITY`.
    pub const MIN_PRIORITY: i32 = 1;
    /// `Thread.NORM_PRIORITY`.
    pub const NORM_PRIORITY: i32 = 5;
    /// `Thread.MAX_PRIORITY`.
    pub const MAX_PRIORITY: i32 = 10;

    fn create(name: String, group: Arc<ThreadGroup>, state: State, body: Option<Body>) -> Thread {
        Thread(Arc::new(ThreadInner {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            name: Mutex::new(name),
            group: Mutex::new(Some(group)),
            state: AtomicU8::new(state as u8),
            priority: AtomicI32::new(Thread::NORM_PRIORITY),
            daemon: AtomicBool::new(false),
            body: Mutex::new(body),
            handle: Mutex::new(None),
            os_thread_id: std::sync::OnceLock::new(),
            signals: Mutex::new(Signals::default()),
            wake: Condvar::new(),
            handler: Mutex::new(None),
        }))
    }

    /// `Thread.currentThread()`.
    pub fn current() -> Thread {
        CURRENT.with(|slot| {
            let mut slot = slot.borrow_mut();
            if let Some(current) = slot.as_ref() {
                return current.0.clone();
            }
            let name = std::thread::current()
                .name()
                .map(str::to_string)
                .unwrap_or_else(|| {
                    format!("Thread-{}", NEXT_THREAD_NUM.fetch_add(1, Ordering::Relaxed))
                });
            let thread =
                Thread::create(name, Arc::clone(ThreadGroup::main()), State::Runnable, None);
            *slot = Some(CurrentSlot(thread.clone()));
            thread
        })
    }

    /// Captures, on the calling (creating) thread, what a Java thread takes
    /// from its creator — its `ThreadGroup` and daemon status — and returns
    /// what the created thread runs its body with: it adopts that thread,
    /// named after its OS thread name, in the creator's group, then runs the
    /// body; a throwable that escapes the body (a panic) goes to the thread's
    /// uncaught-exception handler, as the JVM dispatches it
    /// (`Thread.dispatchUncaughtException` → `ThreadGroup.uncaughtException`,
    /// where the runner's `RunnerThreadGroup` records it), and then ends the
    /// thread with the same payload.
    ///
    /// The adopted thread is live — enumerated by [`ThreadGroup::enumerate`]
    /// — while the body runs, and terminates when the body returns or
    /// panics, as a Java thread of the group does. A thread that already has
    /// a [`Thread`] handle keeps it, and its liveness is not changed.
    #[allow(clippy::type_complexity)]
    pub fn start_context() -> Box<dyn FnOnce(Box<dyn FnOnce() + Send>) + Send> {
        let creator = Thread::current();
        let group = creator
            .thread_group()
            .unwrap_or_else(|| Arc::clone(ThreadGroup::main()));
        let daemon = creator.is_daemon();
        Box::new(move |body: Box<dyn FnOnce() + Send>| {
            let adopted = CURRENT.with(|slot| {
                let mut slot = slot.borrow_mut();
                if slot.is_some() {
                    return None;
                }
                let name = std::thread::current()
                    .name()
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        format!("Thread-{}", NEXT_THREAD_NUM.fetch_add(1, Ordering::Relaxed))
                    });
                let thread = Thread::create(name, group, State::Runnable, None);
                thread.0.daemon.store(daemon, Ordering::Relaxed);
                // Live while the body runs, as a started Java thread is.
                live().push(thread.clone());
                *slot = Some(CurrentSlot(thread.clone()));
                Some(thread)
            });
            // The body has ended: the thread leaves the live set.
            let end = |adopted: &Option<Thread>| {
                if let Some(thread) = adopted {
                    thread.terminate();
                }
            };
            if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
                let t = match payload.downcast_ref::<Throwable>() {
                    Some(t) => t.clone(),
                    None => Throwable::from_text(&exception::panic_message(&*payload)),
                };
                let me = Thread::current();
                let handler = me.get_uncaught_exception_handler();
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    handler.dispatch(&me, t)
                }));
                end(&adopted);
                std::panic::resume_unwind(payload);
            }
            end(&adopted);
        })
    }

    /// `new Thread(runnable, name)`: a `NEW` thread in the current thread's
    /// group.
    pub fn new(name: impl Into<String>, body: impl FnOnce() + Send + 'static) -> Thread {
        let group = Thread::current()
            .thread_group()
            .unwrap_or_else(|| Arc::clone(ThreadGroup::main()));
        Thread::with_group(&group, name, body)
    }

    /// `new Thread(runnable)`: named `Thread-N`, as Java numbers unnamed
    /// threads.
    pub fn unnamed(body: impl FnOnce() + Send + 'static) -> Thread {
        let name = format!("Thread-{}", NEXT_THREAD_NUM.fetch_add(1, Ordering::Relaxed));
        Thread::new(name, body)
    }

    /// `new Thread(group, runnable)`: named `Thread-N`, in `group`.
    pub fn unnamed_in(group: &Arc<ThreadGroup>, body: impl FnOnce() + Send + 'static) -> Thread {
        let name = format!("Thread-{}", NEXT_THREAD_NUM.fetch_add(1, Ordering::Relaxed));
        Thread::with_group(group, name, body)
    }

    /// `new Thread(group, runnable, name)`.
    pub fn with_group(
        group: &Arc<ThreadGroup>,
        name: impl Into<String>,
        body: impl FnOnce() + Send + 'static,
    ) -> Thread {
        let creator = Thread::current();
        let t = Thread::create(
            name.into(),
            Arc::clone(group),
            State::New,
            Some(Box::new(body)),
        );
        // A Java thread inherits its creator's daemon status.
        t.0.daemon.store(creator.is_daemon(), Ordering::Relaxed);
        t
    }

    /// `Thread.start()`.
    ///
    /// The body runs inside `catch_unwind`; a throwable that escapes it goes
    /// to [`get_uncaught_exception_handler`](Self::get_uncaught_exception_handler),
    /// as the JVM dispatches it, and then ends the OS thread with the same
    /// payload, so that [`join`](Self::join) still reports it.
    ///
    /// # Panics
    ///
    /// `IllegalThreadStateException` when the thread was already started,
    /// as Java throws.
    pub fn start(&self) {
        let body = self.0.body.lock().unwrap_or_else(|e| e.into_inner()).take();
        let Some(body) = body.filter(|_| self.state() == State::New) else {
            exception::throw_without_message(exception::ILLEGAL_THREAD_STATE_EXCEPTION)
        };
        self.0.state.store(State::Runnable as u8, Ordering::Release);
        live().push(self.clone());
        let me = self.clone();
        let handle = std::thread::Builder::new()
            .name(self.name())
            .spawn(move || {
                CURRENT.with(|slot| *slot.borrow_mut() = Some(CurrentSlot(me.clone())));
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
                if let Err(payload) = result {
                    let t = Throwable::from_panic(payload);
                    let handler = me.get_uncaught_exception_handler();
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        handler.dispatch(&me, t.clone())
                    }));
                    me.terminate();
                    std::panic::resume_unwind(Box::new(t));
                }
                me.terminate();
            })
            .expect("spawn thread");
        let _ = self.0.os_thread_id.set(handle.thread().id());
        *self.0.handle.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
    }

    /// The identity of the OS thread this `Thread` runs on, once started:
    /// what code that keys threads by `std::thread::ThreadId` (Lucene's
    /// identity maps of `Thread`s, such as
    /// `DocumentsWriterStallControl.waiting`) knows the thread by.
    pub fn os_thread_id(&self) -> Option<std::thread::ThreadId> {
        self.0.os_thread_id.get().copied()
    }

    /// `Thread.join()`: waits for the thread; `Err` carries the throwable
    /// that ended its body (Java reports it through the uncaught exception
    /// handler only, which this thread did too).
    pub fn join(&self) -> std::thread::Result<()> {
        let handle = self
            .0
            .handle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        match handle {
            Some(h) => {
                let result = h.join();
                self.terminate();
                result
            }
            None => {
                // Joined before (or never started): wait for termination.
                while self.is_alive() {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Ok(())
            }
        }
    }

    /// `Thread.join(millis)`: waits at most `millis` milliseconds (`0`
    /// waits forever) for the thread to terminate.
    ///
    /// # Panics
    ///
    /// `InterruptedException` (`null` message) when the calling thread is
    /// interrupted, which clears its flag; `IllegalArgumentException`
    /// (`timeout value is negative`) for a negative timeout.
    pub fn join_millis(&self, millis: i64) {
        if millis < 0 {
            exception::illegal_argument("timeout value is negative");
        }
        let me = Thread::current();
        let deadline = (millis > 0).then(|| Instant::now() + Duration::from_millis(millis as u64));
        while self.is_alive() {
            let slice = match deadline {
                None => Duration::from_millis(10),
                Some(d) => {
                    let now = Instant::now();
                    if now >= d {
                        return;
                    }
                    (d - now).min(Duration::from_millis(10))
                }
            };
            if me.wait_on(slice) {
                exception::throw_without_message(exception::INTERRUPTED_EXCEPTION);
            }
        }
    }

    fn terminate(&self) {
        let was = self.0.state.swap(State::Terminated as u8, Ordering::AcqRel);
        *self.0.group.lock().unwrap_or_else(|e| e.into_inner()) = None;
        if was != State::Terminated as u8 {
            live().retain(|t| t != self);
        }
        let _g = self.0.signals.lock().unwrap_or_else(|e| e.into_inner());
        self.0.wake.notify_all();
    }

    /// `Thread.threadId()` / `Thread.getId()`.
    pub fn id(&self) -> i64 {
        self.0.id
    }

    /// `Thread.getName()`.
    pub fn name(&self) -> String {
        self.0
            .name
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// `Thread.setName(name)`.
    pub fn set_name(&self, name: impl Into<String>) {
        *self.0.name.lock().unwrap_or_else(|e| e.into_inner()) = name.into();
    }

    /// `Thread.getPriority()`.
    pub fn priority(&self) -> i32 {
        self.0.priority.load(Ordering::Relaxed)
    }

    /// `Thread.setPriority(priority)`: recorded only (the port does not
    /// schedule by priority).
    pub fn set_priority(&self, priority: i32) {
        if !(Thread::MIN_PRIORITY..=Thread::MAX_PRIORITY).contains(&priority) {
            exception::throw_without_message(exception::ILLEGAL_ARGUMENT_EXCEPTION);
        }
        self.0.priority.store(priority, Ordering::Relaxed);
    }

    /// `Thread.isDaemon()`.
    pub fn is_daemon(&self) -> bool {
        self.0.daemon.load(Ordering::Relaxed)
    }

    /// `Thread.setDaemon(on)`: recorded only (a Rust process does not wait
    /// for its threads).
    ///
    /// # Panics
    ///
    /// `IllegalThreadStateException` once the thread is started.
    pub fn set_daemon(&self, on: bool) {
        if self.state() != State::New {
            exception::throw_without_message(exception::ILLEGAL_THREAD_STATE_EXCEPTION);
        }
        self.0.daemon.store(on, Ordering::Relaxed);
    }

    /// `Thread.getState()`.
    pub fn state(&self) -> State {
        State::from_u8(self.0.state.load(Ordering::Acquire))
    }

    /// `Thread.isAlive()`: started and not terminated.
    pub fn is_alive(&self) -> bool {
        self.state() == State::Runnable
    }

    /// `Thread.getThreadGroup()`: `None` once the thread terminated.
    pub fn thread_group(&self) -> Option<Arc<ThreadGroup>> {
        self.0
            .group
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// `Thread.getStackTrace()`: always empty (see the module
    /// documentation).
    pub fn get_stack_trace(&self) -> Vec<exception::StackTraceElement> {
        Vec::new()
    }

    /// `new WeakReference<Thread>(this)`.
    pub fn downgrade(&self) -> WeakThread {
        WeakThread(Arc::downgrade(&self.0))
    }

    fn signals(&self) -> MutexGuard<'_, Signals> {
        self.0.signals.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `Thread.interrupt()`: sets the interrupt flag and wakes the thread
    /// from the blocking calls of this module.
    pub fn interrupt(&self) {
        let mut s = self.signals();
        s.interrupted = true;
        self.0.wake.notify_all();
    }

    /// `Thread.isInterrupted()`.
    pub fn is_interrupted(&self) -> bool {
        self.signals().interrupted
    }

    /// `Thread.interrupted()`: the current thread's flag, cleared.
    pub fn interrupted() -> bool {
        let me = Thread::current();
        let mut s = me.signals();
        std::mem::replace(&mut s.interrupted, false)
    }

    /// Blocks the calling thread (which must be `self`) for at most
    /// `timeout`, until it is interrupted or notified. Returns whether it
    /// was interrupted, clearing the flag.
    fn wait_on(&self, timeout: Duration) -> bool {
        let mut s = self.signals();
        if !s.interrupted && !timeout.is_zero() {
            s = self
                .0
                .wake
                .wait_timeout(s, timeout)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        std::mem::replace(&mut s.interrupted, false)
    }

    /// `Thread.sleep(millis)`.
    ///
    /// # Panics
    ///
    /// `InterruptedException` (`sleep interrupted`) when the thread is (or
    /// becomes) interrupted, which clears its flag; `IllegalArgumentException`
    /// (`timeout value is negative`) for a negative time.
    pub fn sleep(millis: i64) {
        if let Err(e) = Thread::try_sleep(millis) {
            e.throw()
        }
    }

    /// [`sleep`](Self::sleep) that returns the `InterruptedException` instead
    /// of throwing it.
    pub fn try_sleep(millis: i64) -> Result<(), Throwable> {
        if millis < 0 {
            exception::illegal_argument("timeout value is negative");
        }
        let me = Thread::current();
        let deadline = Instant::now() + Duration::from_millis(millis as u64);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if me.wait_on(remaining) {
                return Err(Throwable::new(
                    exception::INTERRUPTED_EXCEPTION,
                    "sleep interrupted",
                ));
            }
            if Instant::now() >= deadline {
                return Ok(());
            }
        }
    }

    /// `LockSupport.parkNanos(nanos)`: blocks the current thread until
    /// unparked, interrupted (the flag stays set) or the time elapses.
    pub fn park_nanos(nanos: i64) {
        if nanos <= 0 {
            return;
        }
        let me = Thread::current();
        let mut s = me.signals();
        if s.permit {
            s.permit = false;
            return;
        }
        if s.interrupted {
            return;
        }
        let (mut s, _) =
            me.0.wake
                .wait_timeout(s, Duration::from_nanos(nanos as u64))
                .unwrap_or_else(|e| e.into_inner());
        s.permit = false;
    }

    /// `LockSupport.unpark(thread)`.
    pub fn unpark(&self) {
        let mut s = self.signals();
        s.permit = true;
        self.0.wake.notify_all();
    }

    /// `Thread.setUncaughtExceptionHandler(handler)`.
    pub fn set_uncaught_exception_handler(
        &self,
        handler: Option<Arc<dyn UncaughtExceptionHandler>>,
    ) {
        *self.0.handler.lock().unwrap_or_else(|e| e.into_inner()) = handler;
    }

    /// `Thread.getUncaughtExceptionHandler()`: the thread's own handler, or
    /// its group (`ThreadGroup` is a handler too).
    pub fn get_uncaught_exception_handler(&self) -> UncaughtDispatch {
        if let Some(h) = self
            .0
            .handler
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            return UncaughtDispatch::Handler(h);
        }
        match self.thread_group() {
            Some(g) => UncaughtDispatch::Group(g),
            None => UncaughtDispatch::None,
        }
    }

    /// `Thread.setDefaultUncaughtExceptionHandler(handler)`.
    pub fn set_default_uncaught_exception_handler(
        handler: Option<Arc<dyn UncaughtExceptionHandler>>,
    ) {
        *DEFAULT_HANDLER.write().unwrap_or_else(|e| e.into_inner()) = handler;
    }

    /// `Thread.getDefaultUncaughtExceptionHandler()`.
    pub fn get_default_uncaught_exception_handler() -> Option<Arc<dyn UncaughtExceptionHandler>> {
        DEFAULT_HANDLER
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Whether two handler references are the same object (`==`).
    pub fn same_handler(
        a: &Arc<dyn UncaughtExceptionHandler>,
        b: &Arc<dyn UncaughtExceptionHandler>,
    ) -> bool {
        std::ptr::eq(Arc::as_ptr(a) as *const u8, Arc::as_ptr(b) as *const u8)
    }
}

/// What `Thread.getUncaughtExceptionHandler()` returns: the thread's own
/// handler or its group.
pub enum UncaughtDispatch {
    /// A handler set with `setUncaughtExceptionHandler`.
    Handler(Arc<dyn UncaughtExceptionHandler>),
    /// The thread's group.
    Group(Arc<ThreadGroup>),
    /// A terminated thread: Java returns `null`.
    None,
}

impl UncaughtDispatch {
    /// `getUncaughtExceptionHandler().uncaughtException(t, e)`.
    pub fn dispatch(&self, t: &Thread, e: Throwable) {
        match self {
            UncaughtDispatch::Handler(h) => h.uncaught_exception(t, e),
            UncaughtDispatch::Group(g) => g.uncaught_exception(t, e),
            UncaughtDispatch::None => {}
        }
    }
}

impl WeakThread {
    /// `WeakReference.get()`.
    pub fn get(&self) -> Option<Thread> {
        self.0.upgrade().map(Thread)
    }

    /// Whether this reference points at `thread`, without upgrading it.
    pub fn is(&self, thread: &Thread) -> bool {
        std::ptr::eq(self.0.as_ptr(), Arc::as_ptr(&thread.0))
    }
}

impl PartialEq for Thread {
    fn eq(&self, other: &Thread) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Thread {}

impl std::hash::Hash for Thread {
    /// `Object.hashCode()`: identity.
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.0) as usize).hash(state)
    }
}

impl fmt::Display for Thread {
    /// `Thread.toString()` of JDK 25: `Thread[#id,name,priority,group]`,
    /// with an empty group for a terminated thread.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let group = self.thread_group();
        write!(
            f,
            "Thread[#{},{},{},{}]",
            self.id(),
            self.name(),
            self.priority(),
            group.as_ref().map(|g| g.name()).unwrap_or("")
        )
    }
}

impl fmt::Debug for Thread {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
