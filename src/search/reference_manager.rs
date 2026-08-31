//! `ReferenceManager` ported from `org.apache.lucene.search.ReferenceManager`.
//!
//! Utility that safely shares instances of a certain reference type across
//! multiple threads while periodically refreshing them. Each reference is
//! closed only once every thread has finished using it.
//!
//! This is the generic abstraction. The concrete [`crate::index::ReaderManager`]
//! binds it to [`crate::index::DirectoryReader`].
//!
//! Concurrency model (mirrors the Java source at
//! `lucene/core/src/java/org/apache/lucene/search/ReferenceManager.java`):
//!
//! * `current` is the volatile slot holding the managed reference. In Rust it
//!   lives behind an `RwLock<Option<Arc<G>>>`; reads (`acquire`) take a read
//!   lock only long enough to clone the `Arc`, while swaps (`swap_reference`,
//!   `close`) take a brief write lock.
//! * `refresh_lock` serializes refresh attempts. `maybe_refresh` uses
//!   `try_lock` so that at most one thread refreshes and others return
//!   immediately; `maybe_refresh_blocking` uses a blocking `lock`.
//! * `acquire` loops: read `current`, `try_inc_ref`; on failure (the reference
//!   was closed concurrently) re-read `current` and retry. It never returns a
//!   closed reference.
//! * Listeners are snapshotted before notification (mirroring Java's
//!   `CopyOnWriteArrayList`), so a listener may add/remove listeners without
//!   deadlocking.

#![deny(unsafe_code)]

use std::sync::{Arc, Mutex, RwLock};

use crate::error::{LuceneError, Result};

/// Message used when an operation is attempted on a closed manager.
///
/// Equivalent to `ReferenceManager.REFERENCE_MANAGER_IS_CLOSED_MSG`.
const REFERENCE_MANAGER_CLOSED_MSG: &str = "this ReferenceManager is closed";

/// Ref-counting contract that a managed reference type must satisfy.
///
/// Equivalent to the four abstract ref-counting methods on Java's
/// `ReferenceManager<G>` (`decRef`, `tryIncRef`, `getRefCount`). The method
/// names differ from [`crate::index::IndexReader`] to avoid ambiguity when
/// implementing this trait for `dyn DirectoryReader`, but the semantics are
/// identical: `release_ref` decrements and may close, `try_acquire_ref`
/// increments only if still open, `ref_count` reads the live count.
pub trait ManagedReference: Send + Sync {
    /// Decrements the reference count, closing the resource when it reaches
    /// zero.
    ///
    /// Equivalent to `ReferenceManager.decRef(G)`.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying decrement fails (for example, I/O
    /// during close).
    fn release_ref(&self) -> Result<()>;

    /// Attempts to increment the reference count, returning `false` if the
    /// reference is already closed.
    ///
    /// Equivalent to `ReferenceManager.tryIncRef(G)`.
    fn try_acquire_ref(&self) -> bool;

    /// Returns the current reference count.
    ///
    /// Equivalent to `ReferenceManager.getRefCount(G)`.
    fn ref_count(&self) -> i32;
}

/// Produces fresh references and provides subclass-specific lifecycle hooks.
///
/// Equivalent to the abstract method `refreshIfNeeded` and the overridable
/// `afterClose` / `afterMaybeRefresh` hooks on `ReferenceManager<G>`. Splitting
/// them onto a separate trait keeps `ReferenceManager` generic over the
/// reference type while allowing each concrete manager to supply its own
/// refresh strategy.
pub trait RefreshSource<G: ManagedReference + ?Sized>: Send + Sync {
    /// Returns a fresh reference if a refresh is needed, otherwise `None`.
    ///
    /// Implementations must never return the same reference passed in; return
    /// `None` instead when nothing changed.
    ///
    /// Equivalent to `ReferenceManager.refreshIfNeeded(G)`.
    ///
    /// # Errors
    ///
    /// Returns an error if the refresh operation fails.
    fn refresh_if_needed(&self, current: &Arc<G>) -> Result<Option<Arc<G>>>;

    /// Called after the manager is closed. Default is a no-op.
    ///
    /// Equivalent to `ReferenceManager.afterClose()`.
    ///
    /// # Errors
    ///
    /// Implementations may return an error from cleanup work.
    fn after_close(&self) -> Result<()> {
        Ok(())
    }

    /// Called after a refresh was attempted, regardless of whether a new
    /// reference was produced. Default is a no-op.
    ///
    /// Equivalent to `ReferenceManager.afterMaybeRefresh()`.
    ///
    /// # Errors
    ///
    /// Implementations may return an error from post-refresh work.
    fn after_maybe_refresh(&self) -> Result<()> {
        Ok(())
    }
}

/// Listener notified before and after a refresh attempt.
///
/// Equivalent to `ReferenceManager.RefreshListener`.
pub trait RefreshListener: Send + Sync {
    /// Called right before a refresh attempt starts.
    ///
    /// Equivalent to `RefreshListener.beforeRefresh()`.
    ///
    /// # Errors
    ///
    /// Implementations may return an error; it propagates out of the refresh
    /// call.
    fn before_refresh(&self) -> Result<()>;

    /// Called after the attempted refresh. If `did_refresh` is `true`, a new
    /// reference was installed and [`ReferenceManager::acquire`] will return it.
    ///
    /// Equivalent to `RefreshListener.afterRefresh(boolean)`.
    ///
    /// # Errors
    ///
    /// Implementations may return an error; it propagates out of the refresh
    /// call.
    fn after_refresh(&self, did_refresh: bool) -> Result<()>;
}

/// Generic near-real-time reference manager.
///
/// Equivalent to `org.apache.lucene.search.ReferenceManager<G>`.
///
/// Create a manager with [`ReferenceManager::new`], passing the initial
/// reference (the manager steals its ref count) and a [`RefreshSource`] that
/// knows how to produce a fresh reference. Call [`ReferenceManager::acquire`]
/// to obtain a reference (match each `acquire` with [`ReferenceManager::release`])
/// and [`ReferenceManager::maybe_refresh`] periodically to swap in a fresh
/// reference when the underlying index has changed.
pub struct ReferenceManager<G: ManagedReference + ?Sized + 'static> {
    /// The current managed reference, or `None` once the manager is closed.
    ///
    /// Mirrors the `volatile G current` field in Java.
    current: RwLock<Option<Arc<G>>>,
    /// Serializes refresh attempts. Mirrors `refreshLock` in Java.
    refresh_lock: Mutex<()>,
    /// Listeners notified on refresh. Mirrors the `CopyOnWriteArrayList`.
    listeners: RwLock<Vec<Arc<dyn RefreshListener>>>,
    /// Refresh strategy and lifecycle hooks.
    source: Arc<dyn RefreshSource<G>>,
}

impl<G: ManagedReference + ?Sized + 'static> ReferenceManager<G> {
    /// Creates a new manager that steals the given reference.
    ///
    /// The caller transfers one reference count to the manager (matching
    /// Java's `ReaderManager(DirectoryReader)` which assigns `current`
    /// without incrementing).
    ///
    /// Equivalent to constructing a `ReferenceManager` subclass and assigning
    /// `current`.
    pub fn new(current: Arc<G>, source: Arc<dyn RefreshSource<G>>) -> Self {
        Self {
            current: RwLock::new(Some(current)),
            refresh_lock: Mutex::new(()),
            listeners: RwLock::new(Vec::new()),
            source,
        }
    }

    /// Returns `true` if the manager has been closed.
    pub fn is_closed(&self) -> bool {
        self.current
            .read()
            .expect("current lock poisoned")
            .is_none()
    }

    /// Obtains the current reference.
    ///
    /// Every `acquire` must be matched by [`release`]. The returned reference is
    /// guaranteed to be open (its ref count was successfully incremented).
    ///
    /// Equivalent to `ReferenceManager.acquire()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] if the manager is closed.
    ///
    /// [`release`]: ReferenceManager::release
    pub fn acquire(&self) -> Result<Arc<G>> {
        loop {
            let ref_arc = {
                let guard = self.current.read().expect("current lock poisoned");
                match guard.as_ref() {
                    Some(r) => Arc::clone(r),
                    None => {
                        return Err(LuceneError::AlreadyClosed(
                            REFERENCE_MANAGER_CLOSED_MSG.to_string(),
                        ))
                    }
                }
            };
            if ref_arc.try_acquire_ref() {
                return Ok(ref_arc);
            }
            // `tryIncRef` failed: the reference was closed concurrently.
            // If it is still the current reference, the manager is in an
            // illegal state (someone decremented outside the manager).
            if ref_arc.ref_count() == 0 {
                let still_current = {
                    let guard = self.current.read().expect("current lock poisoned");
                    guard.as_ref().is_some_and(|c| Arc::ptr_eq(c, &ref_arc))
                };
                if still_current {
                    return Err(LuceneError::IllegalState(
                        "The managed reference has already closed - this is likely a bug when the reference count is modified outside of the ReferenceManager"
                            .to_string(),
                    ));
                }
            }
            // Re-read `current` and retry.
        }
    }

    /// Releases a reference previously obtained via [`acquire`].
    ///
    /// Consumes the reference (the caller can no longer use it), mirroring the
    /// Java idiom of setting the reference to `null` in a `finally` clause.
    /// It is safe to call after the manager has been closed.
    ///
    /// Equivalent to `ReferenceManager.release(G)`.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying decrement fails.
    ///
    /// [`acquire`]: ReferenceManager::acquire
    pub fn release(&self, reference: Arc<G>) -> Result<()> {
        reference.release_ref()
    }

    /// Periodically refreshes the managed reference if another thread is not
    /// already doing so.
    ///
    /// Returns `true` if the calling thread attempted the refresh (whether or
    /// not anything changed), and `false` if another thread is currently
    /// refreshing.
    ///
    /// Equivalent to `ReferenceManager.maybeRefresh()`.
    ///
    /// # Errors
    ///
    /// Returns an error if refreshing fails, or [`LuceneError::AlreadyClosed`]
    /// if the manager is closed.
    pub fn maybe_refresh(&self) -> Result<bool> {
        self.ensure_open()?;
        match self.refresh_lock.try_lock() {
            Ok(_guard) => {
                self.do_maybe_refresh()?;
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }

    /// Refreshes the managed reference, blocking if another thread is currently
    /// refreshing.
    ///
    /// Equivalent to `ReferenceManager.maybeRefreshBlocking()`.
    ///
    /// # Errors
    ///
    /// Returns an error if refreshing fails, or [`LuceneError::AlreadyClosed`]
    /// if the manager is closed.
    pub fn maybe_refresh_blocking(&self) -> Result<()> {
        self.ensure_open()?;
        let _guard = self.refresh_lock.lock().expect("refresh lock poisoned");
        self.do_maybe_refresh()
    }

    /// Closes the manager to prevent future [`acquire`](Self::acquire) calls.
    ///
    /// The current reference is released; if other threads still hold acquired
    /// references, the underlying resource is only freed once the last one is
    /// released. Calling `close` more than once has no effect.
    ///
    /// Equivalent to `ReferenceManager.close()`.
    ///
    /// # Errors
    ///
    /// Returns an error if releasing the current reference or
    /// [`RefreshSource::after_close`] fails.
    pub fn close(&self) -> Result<()> {
        match self.swap_reference(None) {
            Ok(()) => self.source.after_close(),
            Err(LuceneError::AlreadyClosed(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Adds a listener to be notified when a reference is refreshed or swapped.
    ///
    /// Equivalent to `ReferenceManager.addListener(RefreshListener)`.
    pub fn add_listener(&self, listener: Arc<dyn RefreshListener>) {
        let mut listeners = self.listeners.write().expect("listeners lock poisoned");
        listeners.push(listener);
    }

    /// Removes a listener previously added with [`add_listener`].
    ///
    /// Equivalent to `ReferenceManager.removeListener(RefreshListener)`.
    ///
    /// [`add_listener`]: ReferenceManager::add_listener
    pub fn remove_listener(&self, listener: &Arc<dyn RefreshListener>) {
        let mut listeners = self.listeners.write().expect("listeners lock poisoned");
        listeners.retain(|x| !Arc::ptr_eq(x, listener));
    }

    /// Returns the refresh source supplied at construction.
    ///
    /// Useful for concrete managers that need to invoke the lifecycle hooks
    /// directly.
    pub fn source(&self) -> &Arc<dyn RefreshSource<G>> {
        &self.source
    }

    // -- internals -----------------------------------------------------------

    /// Throws `AlreadyClosed` if the manager is closed.
    fn ensure_open(&self) -> Result<()> {
        if self
            .current
            .read()
            .expect("current lock poisoned")
            .is_none()
        {
            Err(LuceneError::AlreadyClosed(
                REFERENCE_MANAGER_CLOSED_MSG.to_string(),
            ))
        } else {
            Ok(())
        }
    }

    /// Atomically swaps the current reference, releasing the old one.
    ///
    /// Passing `None` clears the slot (used by `close`). The old reference is
    /// released *after* dropping the write lock so that a potentially slow
    /// `do_close` does not block concurrent `acquire` reads.
    ///
    /// Equivalent to `ReferenceManager.swapReference(G)`.
    fn swap_reference(&self, new_ref: Option<Arc<G>>) -> Result<()> {
        let old = {
            let mut current = self.current.write().expect("current lock poisoned");
            if current.is_none() {
                // `new_ref` is dropped here.
                return Err(LuceneError::AlreadyClosed(
                    REFERENCE_MANAGER_CLOSED_MSG.to_string(),
                ));
            }
            let old = current.take();
            *current = new_ref;
            old
        };
        if let Some(old) = old {
            old.release_ref()?;
        }
        Ok(())
    }

    /// Core refresh logic, executed while holding `refresh_lock`.
    ///
    /// Equivalent to `ReferenceManager.doMaybeRefresh()`.
    fn do_maybe_refresh(&self) -> Result<()> {
        let reference = self.acquire()?;
        let mut refreshed = false;
        let inner: Result<()> = (|| {
            self.notify_before_refresh()?;
            let new_ref = self.source.refresh_if_needed(&reference)?;
            if let Some(new_ref) = new_ref {
                if Arc::ptr_eq(&new_ref, &reference) {
                    return Err(LuceneError::IllegalState(
                        "refresh_if_needed should return None if refresh wasn't needed".to_string(),
                    ));
                }
                match self.swap_reference(Some(Arc::clone(&new_ref))) {
                    Ok(()) => {
                        refreshed = true;
                        Ok(())
                    }
                    Err(e) => {
                        // Swap failed (e.g. the manager was closed mid-refresh):
                        // release the fresh reference; the old one is still
                        // current. Mirrors Java's
                        // `finally { if (!refreshed) release(newReference); }`.
                        let _ = new_ref.release_ref();
                        Err(e)
                    }
                }
            } else {
                Ok(())
            }
        })();
        // finally: release the acquired reference and notify listeners.
        let _ = reference.release_ref();
        let notify = self.notify_after_refresh(refreshed);
        match inner {
            Ok(()) => {
                notify?;
                self.source.after_maybe_refresh()?;
                Ok(())
            }
            Err(e) => {
                // Propagate the original error; a listener error is swallowed
                // to avoid masking the real failure.
                let _ = notify;
                Err(e)
            }
        }
    }

    /// Snapshots the listeners and calls `before_refresh` on each.
    fn notify_before_refresh(&self) -> Result<()> {
        let snapshot: Vec<Arc<dyn RefreshListener>> = self
            .listeners
            .read()
            .expect("listeners lock poisoned")
            .clone();
        for listener in &snapshot {
            listener.before_refresh()?;
        }
        Ok(())
    }

    /// Snapshots the listeners and calls `after_refresh` on each.
    fn notify_after_refresh(&self, did_refresh: bool) -> Result<()> {
        let snapshot: Vec<Arc<dyn RefreshListener>> = self
            .listeners
            .read()
            .expect("listeners lock poisoned")
            .clone();
        for listener in &snapshot {
            listener.after_refresh(did_refresh)?;
        }
        Ok(())
    }
}

impl<G: ManagedReference + ?Sized + 'static> Drop for ReferenceManager<G> {
    /// Best-effort cleanup if the manager is dropped without calling
    /// [`close`](Self::close).
    ///
    /// Java does not define a finalizer; in Rust the `Drop` impl ensures the
    /// current reference is released so its ref count does not leak. Errors
    /// from `release_ref` are swallowed (there is no caller to report them
    /// to); callers should invoke [`close`](Self::close) explicitly to observe
    /// errors.
    fn drop(&mut self) {
        let old = self.current.write().expect("current lock poisoned").take();
        if let Some(old) = old {
            let _ = old.release_ref();
        }
    }
}
