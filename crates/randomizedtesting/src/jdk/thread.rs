//! `java.lang.Thread` and `java.lang.ThreadGroup`, as far as
//! randomizedtesting observes them.
//!
//! `RandomizedContext` finds a thread's context through the thread's
//! `ThreadGroup` and its parents, keeps per-thread state keyed by the
//! `Thread` (weakly), and formats threads in its messages. Rust threads have
//! no group, no Java identifier and no state, so this module gives every
//! thread a [`Thread`] handle that carries them.
//!
//! **Divergences.**
//! * A Java thread inherits its creator's group. Only a thread created with
//!   [`Thread::new`] / [`Thread::with_group`] and started with
//!   [`Thread::start`] does so here; any other thread (for example one from
//!   `std::thread::spawn`) is registered on first use in the `main` group,
//!   whose parent is `system`, as the JVM's main thread is.
//! * Thread identifiers come from a process-wide counter starting at 1; they
//!   are unique like Java's, but not the numbers a JVM would assign.
//! * Only the states `NEW`, `RUNNABLE` and `TERMINATED` are observable: a
//!   blocked thread still reports `RUNNABLE`.
//! * The priority is always `NORM_PRIORITY` (5).

use std::fmt;
use std::sync::atomic::{AtomicI64, AtomicU8, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Weak};

use super::exception;

/// `java.lang.ThreadGroup`: a named node of the group tree.
#[derive(Debug)]
pub struct ThreadGroup {
    name: String,
    parent: Option<Arc<ThreadGroup>>,
}

impl ThreadGroup {
    /// `new ThreadGroup(parent, name)`.
    pub fn new(parent: &Arc<ThreadGroup>, name: impl Into<String>) -> Arc<ThreadGroup> {
        Arc::new(ThreadGroup {
            name: name.into(),
            parent: Some(Arc::clone(parent)),
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

    /// The JVM's `main` group (child of `system`), the group of threads this
    /// module did not start.
    pub fn main() -> &'static Arc<ThreadGroup> {
        static MAIN: LazyLock<Arc<ThreadGroup>> = LazyLock::new(|| {
            let system = Arc::new(ThreadGroup {
                name: "system".to_string(),
                parent: None,
            });
            ThreadGroup::new(&system, "main")
        });
        &MAIN
    }
}

/// `java.lang.Thread.State`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

struct ThreadInner {
    id: i64,
    name: Mutex<String>,
    group: Mutex<Option<Arc<ThreadGroup>>>,
    state: AtomicU8,
    body: Mutex<Option<Body>>,
    handle: Mutex<Option<std::thread::JoinHandle<()>>>,
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
    fn create(name: String, group: Arc<ThreadGroup>, state: State, body: Option<Body>) -> Thread {
        Thread(Arc::new(ThreadInner {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            name: Mutex::new(name),
            group: Mutex::new(Some(group)),
            state: AtomicU8::new(state as u8),
            body: Mutex::new(body),
            handle: Mutex::new(None),
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

    /// `new Thread(runnable, name)`: a `NEW` thread in the current thread's
    /// group.
    pub fn new(name: impl Into<String>, body: impl FnOnce() + Send + 'static) -> Thread {
        let group = Thread::current()
            .thread_group()
            .unwrap_or_else(|| Arc::clone(ThreadGroup::main()));
        Thread::with_group(&group, name, body)
    }

    /// `new Thread(group, runnable, name)`.
    pub fn with_group(
        group: &Arc<ThreadGroup>,
        name: impl Into<String>,
        body: impl FnOnce() + Send + 'static,
    ) -> Thread {
        Thread::create(
            name.into(),
            Arc::clone(group),
            State::New,
            Some(Box::new(body)),
        )
    }

    /// `Thread.start()`.
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
        let me = self.clone();
        let handle = std::thread::Builder::new()
            .name(self.name())
            .spawn(move || {
                CURRENT.with(|slot| *slot.borrow_mut() = Some(CurrentSlot(me)));
                body();
            })
            .expect("spawn thread");
        *self.0.handle.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
    }

    /// `Thread.join()`: waits for the thread; `Err` carries the panic of its
    /// body (Java reports it through the uncaught exception handler).
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
            None => Ok(()),
        }
    }

    fn terminate(&self) {
        self.0
            .state
            .store(State::Terminated as u8, Ordering::Release);
        *self.0.group.lock().unwrap_or_else(|e| e.into_inner()) = None;
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

    /// `Thread.getPriority()`: always `Thread.NORM_PRIORITY`.
    pub fn priority(&self) -> i32 {
        5
    }

    /// `Thread.getState()`.
    pub fn state(&self) -> State {
        State::from_u8(self.0.state.load(Ordering::Acquire))
    }

    /// `Thread.getThreadGroup()`: `None` once the thread terminated.
    pub fn thread_group(&self) -> Option<Arc<ThreadGroup>> {
        self.0
            .group
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// `new WeakReference<Thread>(this)`.
    pub fn downgrade(&self) -> WeakThread {
        WeakThread(Arc::downgrade(&self.0))
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
