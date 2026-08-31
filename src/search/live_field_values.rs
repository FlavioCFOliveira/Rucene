//! Live field values across reopens, ported from
//! `org.apache.lucene.search.LiveFieldValues`.

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};

use crate::error::Result;
use crate::search::reference_manager::{ManagedReference, ReferenceManager, RefreshListener};

/// Reads a value back from a searcher, once the update has been flushed and
/// opened in a near-real-time reader.
///
/// Equivalent to the abstract
/// `LiveFieldValues.lookupFromSearcher(S, String)`. Rust has no implementation
/// inheritance, so the one abstract method of the class becomes this trait.
pub trait LiveFieldValuesLookup<G: ManagedReference + ?Sized, T>: Send + Sync {
    /// Looks the value of `id` up in `searcher` — through doc values, stored
    /// fields, and so on — returning `None` when the document does not exist.
    ///
    /// Equivalent to `LiveFieldValues.lookupFromSearcher(S, String)`, which
    /// returns `null` in that case.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading.
    fn lookup_from_searcher(&self, searcher: &Arc<G>, id: &str) -> Result<Option<T>>;
}

/// Tracks live field values across near-real-time reader reopens.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.LiveFieldValues<S, T>`, which implements
/// `ReferenceManager.RefreshListener` and `Closeable`.
///
/// It holds a map of every updated id since the last reader reopen, and prunes
/// the map once the reader has been reopened. You must therefore reopen the
/// reader periodically, otherwise this structure grows unbounded.
///
/// You must also ensure that the same id is never updated by two threads at
/// once, because in that case you cannot in general know which thread won.
///
/// **Divergence from Lucene 10.5.0.** Java detects a deleted document with
/// `value == missingValue`, a *reference* comparison against a sentinel object.
/// Rust values have no such identity, so the sentinel is compared by value and
/// `T` must be [`PartialEq`]; a caller therefore has to choose a missing value
/// that no real value can equal, which is the same requirement Java's sentinel
/// expresses.
pub struct LiveFieldValues<G, T, L>
where
    G: ManagedReference + ?Sized + 'static,
    T: Clone + PartialEq + Send + Sync,
    L: LiveFieldValuesLookup<G, T>,
{
    current: RwLock<HashMap<String, T>>,
    old: RwLock<HashMap<String, T>>,
    mgr: Arc<ReferenceManager<G>>,
    missing_value: T,
    lookup: L,
}

impl<G, T, L> std::fmt::Debug for LiveFieldValues<G, T, L>
where
    G: ManagedReference + ?Sized + 'static,
    T: Clone + PartialEq + Send + Sync + 'static,
    L: LiveFieldValuesLookup<G, T> + 'static,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveFieldValues")
            .field("size", &self.size())
            .finish_non_exhaustive()
    }
}

impl<G, T, L> LiveFieldValues<G, T, L>
where
    G: ManagedReference + ?Sized + 'static,
    T: Clone + PartialEq + Send + Sync + 'static,
    L: LiveFieldValuesLookup<G, T> + 'static,
{
    /// Creates the tracker and registers it with `mgr` as a refresh listener.
    ///
    /// Equivalent to
    /// `LiveFieldValues(ReferenceManager<S>, T)`, which ends with
    /// `mgr.addListener(this)`; the listener registration needs a shared handle
    /// in Rust, so the constructor returns one.
    pub fn new(mgr: Arc<ReferenceManager<G>>, missing_value: T, lookup: L) -> Arc<Self> {
        let values = Arc::new(Self {
            current: RwLock::new(HashMap::new()),
            old: RwLock::new(HashMap::new()),
            mgr: Arc::clone(&mgr),
            missing_value,
            lookup,
        });
        mgr.add_listener(Arc::clone(&values) as Arc<dyn RefreshListener>);
        values
    }

    /// Unregisters this tracker from its manager.
    ///
    /// Equivalent to `LiveFieldValues.close()`.
    pub fn close(self: &Arc<Self>) {
        let listener: Arc<dyn RefreshListener> = Arc::clone(self) as Arc<dyn RefreshListener>;
        self.mgr.remove_listener(&listener);
    }

    /// Records the value a field was just set to, after the document was
    /// successfully added to the index.
    ///
    /// Equivalent to `LiveFieldValues.add(String, T)`.
    pub fn add(&self, id: impl Into<String>, value: T) {
        self.current
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.into(), value);
    }

    /// Records a deletion, after the document was successfully deleted from the
    /// index.
    ///
    /// Equivalent to `LiveFieldValues.delete(String)`, which stores the missing
    /// value.
    pub fn delete(&self, id: impl Into<String>) {
        self.current
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.into(), self.missing_value.clone());
    }

    /// Returns the approximate number of id/value pairs buffered in RAM.
    ///
    /// Equivalent to `LiveFieldValues.size()`.
    pub fn size(&self) -> usize {
        let current = self
            .current
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        let old = self
            .old
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        current + old
    }

    /// Returns the current value for `id`, or `None` when the id is not in the
    /// index or was deleted.
    ///
    /// Equivalent to `LiveFieldValues.get(String)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while acquiring the searcher or reading
    /// the value.
    pub fn get(&self, id: &str) -> Result<Option<T>> {
        // First try to get the "live" value.
        let live = self
            .current
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned();
        if let Some(value) = live {
            if value == self.missing_value {
                // Deleted, but the deletion is not yet reflected in the reader.
                return Ok(None);
            }
            return Ok(Some(value));
        }

        let previous = self
            .old
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned();
        if let Some(value) = previous {
            if value == self.missing_value {
                return Ok(None);
            }
            return Ok(Some(value));
        }

        // It either does not exist in the index, or it was already flushed and
        // the near-real-time reader was opened on the segment, so fall back to
        // the current searcher.
        let searcher = self.mgr.acquire()?;
        let result = self.lookup.lookup_from_searcher(&searcher, id);
        self.mgr.release(searcher)?;
        result
    }
}

impl<G, T, L> RefreshListener for LiveFieldValues<G, T, L>
where
    G: ManagedReference + ?Sized + 'static,
    T: Clone + PartialEq + Send + Sync,
    L: LiveFieldValuesLookup<G, T>,
{
    fn before_refresh(&self) -> Result<()> {
        let mut current = self.current.write().unwrap_or_else(PoisonError::into_inner);
        let mut old = self.old.write().unwrap_or_else(PoisonError::into_inner);
        // Start sending all updates after this point to the new map. While the
        // reopen runs, a lookup first tries the new map, then falls back to the
        // old one, and then to the current searcher.
        *old = std::mem::take(&mut current);
        Ok(())
    }

    fn after_refresh(&self, _did_refresh: bool) -> Result<()> {
        // Now drop all the old values, because they are visible through the
        // searcher that was just opened. If did_refresh is false it is possible
        // that `old` still holds entries, which is fine: it means they were
        // already included in the previously opened reader, so clearing is
        // safe.
        self.old
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        Ok(())
    }
}
