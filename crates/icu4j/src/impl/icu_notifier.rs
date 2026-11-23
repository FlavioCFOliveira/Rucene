// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.ICUNotifier`.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

use rucene::error::{LuceneError, Result};

/// The function that notifies one listener (`notifyListener`).
pub type NotifyFn<L> = Arc<dyn Fn(&Arc<L>) + Send + Sync>;

/// `NotifyThread` (private): the daemon thread that notifies the queued
/// listener lists in order.
struct NotifyThread<L: ?Sized> {
    queue: Mutex<VecDeque<Vec<Arc<L>>>>,
    cv: Condvar,
}

impl<L: ?Sized + Send + Sync + 'static> NotifyThread<L> {
    /// `queue(EventListener[] list)`: queues a list of listeners to notify.
    fn queue(&self, list: Vec<Arc<L>>) {
        let mut q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        q.push_back(list);
        self.cv.notify_one();
    }

    /// `run()`: waits for queued lists and notifies their listeners.
    fn run(self: Arc<Self>, notify: NotifyFn<L>) {
        loop {
            let list = {
                let mut q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
                while q.is_empty() {
                    q = self.cv.wait(q).unwrap_or_else(|e| e.into_inner());
                }
                q.pop_front().expect("the queue is not empty")
            };
            for l in &list {
                notify(l);
            }
        }
    }
}

/// The state `notifyLock` guards.
struct State<L: ?Sized> {
    notify_thread: Option<Arc<NotifyThread<L>>>,
    listeners: Option<Vec<Arc<L>>>,
}

/// Abstract implementation of a notification facility: clients add
/// listeners, and the notifier notifies them of changes on a separate
/// daemon thread, in order.
///
/// Equivalent to the abstract class `com.ibm.icu.impl.ICUNotifier`: the
/// listener type `L` stands for the `EventListener` subtype the subclass
/// accepts (`acceptsListener`), and the subclass's `notifyListener` is the
/// function given to [`notify_changed`](Self::notify_changed). Listener
/// identity is `Arc` identity, as Java's `==`.
pub struct ICUNotifier<L: ?Sized> {
    notify_lock: Mutex<State<L>>,
}

impl<L: ?Sized + Send + Sync + 'static> Default for ICUNotifier<L> {
    fn default() -> Self {
        Self::new()
    }
}

impl<L: ?Sized + Send + Sync + 'static> ICUNotifier<L> {
    /// A notifier without listeners.
    pub fn new() -> Self {
        Self {
            notify_lock: Mutex::new(State {
                notify_thread: None,
                listeners: None,
            }),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State<L>> {
        self.notify_lock.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `addListener(EventListener l)`: adds a listener once (by identity),
    /// when `accepts_listener` accepts it.
    ///
    /// # Errors
    ///
    /// `IllegalStateException("Listener invalid for this notifier.")`.
    pub fn add_listener(&self, l: Arc<L>, accepts_listener: bool) -> Result<()> {
        if accepts_listener {
            let mut st = self.state();
            match &mut st.listeners {
                None => st.listeners = Some(vec![l]),
                Some(listeners) => {
                    // identity equality check
                    if listeners.iter().any(|ll| Arc::ptr_eq(ll, &l)) {
                        return Ok(());
                    }
                    listeners.push(l);
                }
            }
            Ok(())
        } else {
            Err(LuceneError::illegal_state("Listener invalid for this notifier."))
        }
    }

    /// `removeListener(EventListener l)`: removes a listener (by identity).
    pub fn remove_listener(&self, l: &Arc<L>) {
        let mut st = self.state();
        if let Some(listeners) = &mut st.listeners {
            // identity equality check
            if let Some(i) = listeners.iter().position(|ll| Arc::ptr_eq(ll, l)) {
                listeners.remove(i);
                if listeners.is_empty() {
                    st.listeners = None;
                }
            }
        }
    }

    /// `notifyChanged()`: queues a notification of all current listeners,
    /// starting the daemon notification thread on first use;
    /// `notify_listener` is the subclass's `notifyListener`.
    pub fn notify_changed(&self, notify_listener: NotifyFn<L>) {
        let mut st = self.state();
        if let Some(listeners) = st.listeners.clone() {
            if st.notify_thread.is_none() {
                let t = Arc::new(NotifyThread {
                    queue: Mutex::new(VecDeque::new()),
                    cv: Condvar::new(),
                });
                let tt = t.clone();
                // notifyThread.setDaemon(true): the thread is detached.
                std::thread::spawn(move || tt.run(notify_listener));
                st.notify_thread = Some(t);
            }
            st.notify_thread.as_ref().expect("set above").queue(listeners);
        }
    }
}
