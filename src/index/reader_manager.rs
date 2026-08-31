//! `ReaderManager` ported from `org.apache.lucene.index.ReaderManager`.
//!
//! Near-real-time helper that safely shares [`DirectoryReader`] instances
//! across multiple threads while periodically reopening. Each reader is closed
//! only once every thread has finished using it.
//!
//! This is the concrete binding of [`crate::search::ReferenceManager`] to
//! `dyn DirectoryReader`. Refresh is driven by
//! [`DirectoryReader::do_open_if_changed`] via the free function
//! [`open_if_changed`].

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::Result;
use crate::index::directory_reader::{open, open_if_changed, DirectoryReader};
use crate::search::reference_manager::{ManagedReference, ReferenceManager, RefreshSource};
use crate::store::Directory;

// ---------------------------------------------------------------------------
// ManagedReference for dyn DirectoryReader
// ---------------------------------------------------------------------------

/// Bridges `dyn DirectoryReader` to [`ManagedReference`] by forwarding to the
/// ref-counting methods inherited from
/// [`IndexReader`](crate::index::IndexReader).
///
/// The method names differ from `IndexReader` to avoid ambiguity inside the
/// `impl` block; the semantics are identical.
impl ManagedReference for dyn DirectoryReader {
    #[inline]
    fn release_ref(&self) -> Result<()> {
        self.dec_ref()
    }

    #[inline]
    fn try_acquire_ref(&self) -> bool {
        self.try_inc_ref()
    }

    #[inline]
    fn ref_count(&self) -> i32 {
        self.get_ref_count()
    }
}

// ---------------------------------------------------------------------------
// RefreshSource for dyn DirectoryReader
// ---------------------------------------------------------------------------

/// Refresh strategy that reopens via [`open_if_changed`].
///
/// Equivalent to `ReaderManager.refreshIfNeeded` calling
/// `DirectoryReader.openIfChanged`.
struct DirectoryReaderRefreshSource;

impl RefreshSource<dyn DirectoryReader> for DirectoryReaderRefreshSource {
    fn refresh_if_needed(
        &self,
        current: &Arc<dyn DirectoryReader>,
    ) -> Result<Option<Arc<dyn DirectoryReader>>> {
        open_if_changed(Arc::clone(current))
    }
}

// ---------------------------------------------------------------------------
// ReaderManager
// ---------------------------------------------------------------------------

/// Near-real-time manager for [`DirectoryReader`].
///
/// Equivalent to `org.apache.lucene.index.ReaderManager` (a final subclass of
/// `ReferenceManager<DirectoryReader>`).
///
/// Create a manager with [`ReaderManager::open`] (from a [`Directory`]) or
/// [`ReaderManager::from_reader`] (from an already-opened `DirectoryReader`,
/// stealing its reference). Call [`ReferenceManager::acquire`] to obtain a
/// reader and [`ReferenceManager::release`] when done; call
/// [`ReferenceManager::maybe_refresh`] periodically to pick up new commits.
pub type ReaderManager = ReferenceManager<dyn DirectoryReader>;

impl ReaderManager {
    /// Creates and returns a new `ReaderManager` from the given [`Directory`].
    ///
    /// Equivalent to `ReaderManager(Directory)`.
    ///
    /// # Errors
    ///
    /// Returns an error if opening the initial reader fails.
    pub fn open(directory: Arc<dyn Directory>) -> Result<Self> {
        let reader = open(directory)?;
        Self::from_reader(reader)
    }

    /// Creates a new `ReaderManager` from an already-opened [`DirectoryReader`],
    /// stealing the incoming reference.
    ///
    /// Equivalent to `ReaderManager(DirectoryReader)`. The caller transfers one
    /// reference count to the manager; they must not release the reader
    /// afterwards.
    ///
    /// # Errors
    ///
    /// Currently infallible; returns `Result` for parity with
    /// [`ReaderManager::open`] and the Java signature.
    pub fn from_reader(reader: Arc<dyn DirectoryReader>) -> Result<Self> {
        Ok(ReferenceManager::new(
            reader,
            Arc::new(DirectoryReaderRefreshSource),
        ))
    }

    // TODO(task: IndexWriter): implement the `ReaderManager(IndexWriter,
    // apply_all_deletes, write_all_deletes)` constructor once IndexWriter is
    // ported. It should call
    // `DirectoryReader::open_if_changed_with_writer` (or the NRT `open`
    // equivalent) for refresh, instead of the directory-based
    // `DirectoryReaderRefreshSource`. Do NOT add a fake IndexWriter stub.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::segment_info::SegmentInfoFormat;
    use crate::codecs::tests::{test_segment_info, DummyCodec};
    use crate::codecs::{register_codec, FilterCodec, Lucene99SegmentInfoFormat};
    use crate::error::LuceneError;
    use crate::index::index_file_names::segment_file_name;
    use crate::index::index_file_names::FIELD_INFO_EXTENSION;
    use crate::index::{SegmentCommitInfo, SegmentInfos};
    use crate::search::reference_manager::RefreshListener;
    use crate::store::{Directory, RamDirectory};
    use crate::util::string_helper::StringHelper;
    use crate::util::Version;
    use std::collections::{HashMap, HashSet};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex as StdMutex;

    // -- test index helpers (mirror directory_reader.rs tests) --------------

    fn test_codec() -> Arc<dyn crate::codecs::Codec> {
        static REGISTER: std::sync::Once = std::sync::Once::new();
        let inner: Arc<dyn crate::codecs::Codec> = Arc::new(DummyCodec::new("Dummy"));
        let codec: Arc<dyn crate::codecs::Codec> = Arc::new(
            FilterCodec::new("ReaderManagerTestCodec", Arc::clone(&inner))
                .with_segment_info_format(Lucene99SegmentInfoFormat::new()),
        );
        REGISTER.call_once(|| {
            let registered =
                FilterCodec::new("ReaderManagerTestCodec", Arc::new(DummyCodec::new("Dummy")))
                    .with_segment_info_format(Lucene99SegmentInfoFormat::new());
            // Ignore double-registration when multiple tests share the binary.
            let _ = register_codec("ReaderManagerTestCodec", registered);
        });
        codec
    }

    fn write_segment_info_file(directory: &dyn Directory, info: &crate::index::SegmentInfo) {
        let format = Lucene99SegmentInfoFormat::new();
        format
            .write(directory, info, &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
    }

    fn make_segment_info(
        directory: &dyn Directory,
        name: &str,
        max_doc: i32,
    ) -> crate::index::SegmentInfo {
        let mut info = test_segment_info(name, max_doc);
        info.set_codec(test_codec());
        info.set_index_sort(
            crate::search::Sort::new_fields(vec![crate::search::SortField::new(
                Some("id".to_string()),
                crate::search::SortFieldType::String,
            )
            .unwrap()])
            .unwrap(),
        );
        info.set_files(HashSet::from([segment_file_name(
            name,
            "",
            FIELD_INFO_EXTENSION,
        )]));
        write_segment_info_file(directory, &info);
        info
    }

    fn make_sci(directory: &dyn Directory, name: &str, max_doc: i32) -> SegmentCommitInfo {
        let info = make_segment_info(directory, name, max_doc);
        SegmentCommitInfo::new(info, 0, 0, -1, -1, -1, StringHelper::random_id()).unwrap()
    }

    /// Commits a single segment of `max_doc` docs and returns a reader.
    fn commit_single_segment(directory: Arc<dyn Directory>) -> Arc<dyn DirectoryReader> {
        let mut sis = SegmentInfos::new(Version::LATEST.major as i32).unwrap();
        sis.counter = 1;
        sis.changed();
        sis.user_data = HashMap::from([("user".to_string(), "data".to_string())]);
        let sci = make_sci(directory.as_ref(), "_0", 42);
        sis.add(sci).unwrap();
        sis.commit(directory.as_ref()).unwrap();
        open(directory).unwrap()
    }

    /// Writes an additional segment and commits, producing a newer version.
    fn add_second_commit(directory: &Arc<dyn Directory>, extra_doc: i32) {
        let latest = SegmentInfos::read_latest_commit(directory.as_ref()).unwrap();
        let mut sis = latest.clone();
        sis.set_next_write_generation(latest.generation() + 1)
            .unwrap();
        sis.changed();
        let name = format!("_{}", sis.counter);
        sis.counter += 1;
        let sci = make_sci(directory.as_ref(), &name, extra_doc);
        sis.add(sci).unwrap();
        sis.commit(directory.as_ref()).unwrap();
    }

    // -- a recording RefreshListener ----------------------------------------

    struct RecordingListener {
        before: StdMutex<Vec<()>>,
        after: StdMutex<Vec<bool>>,
    }

    impl RecordingListener {
        fn new() -> Self {
            Self {
                before: StdMutex::new(Vec::new()),
                after: StdMutex::new(Vec::new()),
            }
        }
        fn before_count(&self) -> usize {
            self.before.lock().unwrap().len()
        }
        fn after_snapshots(&self) -> Vec<bool> {
            self.after.lock().unwrap().clone()
        }
    }

    impl RefreshListener for RecordingListener {
        fn before_refresh(&self) -> Result<()> {
            self.before.lock().unwrap().push(());
            Ok(())
        }
        fn after_refresh(&self, did_refresh: bool) -> Result<()> {
            self.after.lock().unwrap().push(did_refresh);
            Ok(())
        }
    }

    // -- tests --------------------------------------------------------------

    #[test]
    fn open_and_acquire_from_directory() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let reader = commit_single_segment(Arc::clone(&dir));
        // Steal the reader's ref into the manager.
        let manager = ReaderManager::from_reader(reader).unwrap();
        assert!(!manager.is_closed());

        let acquired = manager.acquire().unwrap();
        assert_eq!(acquired.max_doc(), 42);
        // Manager holds refCount 1; acquire added one more.
        assert_eq!(acquired.get_ref_count(), 2);
        manager.release(acquired).unwrap();
    }

    #[test]
    fn open_from_directory_directly() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        commit_single_segment(Arc::clone(&dir));
        let manager = ReaderManager::open(dir).unwrap();
        let acquired = manager.acquire().unwrap();
        assert_eq!(acquired.max_doc(), 42);
        assert_eq!(acquired.get_ref_count(), 2);
        manager.release(acquired).unwrap();
        manager.close().unwrap();
        assert!(manager.is_closed());
    }

    #[test]
    fn acquire_after_close_returns_already_closed() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let reader = commit_single_segment(Arc::clone(&dir));
        let manager = ReaderManager::from_reader(reader).unwrap();
        manager.close().unwrap();
        assert!(manager.is_closed());
        let err = manager.acquire().unwrap_err();
        assert!(matches!(err, LuceneError::AlreadyClosed(_)), "got {err:?}");
    }

    #[test]
    fn close_is_idempotent() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let reader = commit_single_segment(Arc::clone(&dir));
        let manager = ReaderManager::from_reader(reader).unwrap();
        manager.close().unwrap();
        // Second close must not error.
        manager.close().unwrap();
        assert!(manager.is_closed());
    }

    #[test]
    fn refresh_to_latest_commit_swaps_reference() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let reader = commit_single_segment(Arc::clone(&dir));
        let manager = ReaderManager::from_reader(reader).unwrap();

        let first = manager.acquire().unwrap();
        let first_version = first.version().unwrap();
        let first_refcount = first.get_ref_count();
        assert_eq!(first_refcount, 2); // manager + this acquire
        manager.release(first).unwrap();

        // Write a second commit.
        add_second_commit(&dir, 8);

        // maybeRefresh should swap in the new reader.
        let did_refresh = manager.maybe_refresh().unwrap();
        assert!(did_refresh, "calling thread should have refreshed");

        let second = manager.acquire().unwrap();
        assert!(second.version().unwrap() > first_version);
        assert_eq!(second.max_doc(), 50);
        // `leaves` consumes the `Arc`, so clone for the count check.
        assert_eq!(Arc::clone(&second).leaves().len(), 2);

        // The old reader's ref count must have dropped to the references still
        // held externally (none here) -> it is closed (refCount 0).
        // We cannot query the old reader directly after close (it is closed),
        // but `second` is open with refCount 2.
        assert_eq!(second.get_ref_count(), 2);
        manager.release(second).unwrap();
        manager.close().unwrap();
    }

    #[test]
    fn refresh_is_noop_when_unchanged() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let reader = commit_single_segment(Arc::clone(&dir));
        let manager = ReaderManager::from_reader(reader).unwrap();

        let before = manager.acquire().unwrap();
        let before_version = before.version().unwrap();
        manager.release(before).unwrap();

        // No new commit: maybeRefresh runs but does not swap.
        let did_refresh = manager.maybe_refresh().unwrap();
        assert!(did_refresh, "calling thread should have attempted refresh");

        let after = manager.acquire().unwrap();
        assert_eq!(after.version().unwrap(), before_version);
        // Same reference identity: the manager did not swap.
        // (open_if_changed returns None when unchanged, so no swap occurs.)
        assert_eq!(after.max_doc(), 42);
        manager.release(after).unwrap();
        manager.close().unwrap();
    }

    #[test]
    fn maybe_refresh_returns_false_when_another_thread_is_refreshing() {
        // Use a controllable refresh source that blocks until signalled, so we
        // can guarantee a second `maybe_refresh` sees `refresh_lock` held and
        // returns `false` immediately. A `Barrier(2)` gates the refresher.
        use std::sync::Barrier;

        struct BlockingSource {
            gate: Arc<Barrier>,
        }
        impl RefreshSource<dyn DirectoryReader> for BlockingSource {
            fn refresh_if_needed(
                &self,
                _current: &Arc<dyn DirectoryReader>,
            ) -> Result<Option<Arc<dyn DirectoryReader>>> {
                // Block while holding `refresh_lock`; the test releases us by
                // arriving at the same barrier after observing that a
                // concurrent `maybe_refresh` returned false.
                self.gate.wait();
                Ok(None)
            }
        }

        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let reader = commit_single_segment(Arc::clone(&dir));
        let gate = Arc::new(Barrier::new(2));
        let source = Arc::new(BlockingSource {
            gate: Arc::clone(&gate),
        });
        let manager = Arc::new(ReaderManager::from_reader_with_source(reader, source).unwrap());

        // Refresher thread: acquires refresh_lock and blocks inside refresh.
        let manager_ref = Arc::clone(&manager);
        let refresher = std::thread::spawn(move || manager_ref.maybe_refresh().unwrap());

        // Give the refresher a moment to enter `refresh_if_needed` (holding the
        // lock). The 50 ms sleep is a pragmatic barrier; the `Barrier`
        // guarantees the refresher holds the lock until we release it.
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Concurrent non-blocking refresh must return `false` without waiting.
        let got = manager.maybe_refresh().unwrap();
        assert!(
            !got,
            "second maybe_refresh should return false while another thread is refreshing"
        );

        // Release the refresher by arriving at the barrier, then let it finish.
        gate.wait();
        let first_result = refresher.join().unwrap();
        assert!(first_result, "refresher thread should report it refreshed");

        manager.close().unwrap();
    }

    #[test]
    fn listeners_notified_on_refresh() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let reader = commit_single_segment(Arc::clone(&dir));
        let manager = ReaderManager::from_reader(reader).unwrap();
        let listener = Arc::new(RecordingListener::new());
        let typed_for_add = Arc::clone(&listener);
        let dyn_listener: Arc<dyn RefreshListener> = typed_for_add;
        manager.add_listener(dyn_listener);

        // No change: before + after(false).
        manager.maybe_refresh().unwrap();
        assert_eq!(listener.before_count(), 1);
        assert_eq!(listener.after_snapshots(), vec![false]);

        // New commit: before + after(true).
        add_second_commit(&dir, 8);
        manager.maybe_refresh().unwrap();
        assert_eq!(listener.before_count(), 2);
        assert_eq!(listener.after_snapshots(), vec![false, true]);

        let typed_for_remove = Arc::clone(&listener);
        let dyn_for_remove: Arc<dyn RefreshListener> = typed_for_remove;
        manager.remove_listener(&dyn_for_remove);
        // After removal, no further notifications.
        manager.maybe_refresh().unwrap();
        assert_eq!(listener.before_count(), 2);

        manager.close().unwrap();
    }

    #[test]
    fn release_drops_refcount_and_close_releases_manager_ref() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let reader = commit_single_segment(Arc::clone(&dir));
        // reader has refCount 1.
        assert_eq!(reader.get_ref_count(), 1);
        let manager = ReaderManager::from_reader(reader).unwrap();
        // Manager stole the ref: refCount still 1.

        let a = manager.acquire().unwrap();
        assert_eq!(a.get_ref_count(), 2);
        let b = manager.acquire().unwrap();
        assert_eq!(b.get_ref_count(), 3);
        assert!(Arc::ptr_eq(&a, &b));

        manager.release(a).unwrap();
        assert_eq!(b.get_ref_count(), 2);
        manager.release(b).unwrap();

        // Close releases the manager's ref -> refCount 0 -> reader closed.
        manager.close().unwrap();
    }

    #[test]
    fn concurrent_acquire_release_and_refresh() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let reader = commit_single_segment(Arc::clone(&dir));
        let manager = Arc::new(ReaderManager::from_reader(reader).unwrap());

        // Add a second commit up front so refresh has something to pick up,
        // then keep adding commits during the run.
        add_second_commit(&dir, 4);
        let dir_clone = Arc::clone(&dir);

        let iterations = 200;
        let num_threads = 4;

        let refresh_done = Arc::new(AtomicUsize::new(0));

        std::thread::scope(|s| {
            // One thread keeps producing commits so refreshes find changes.
            let producer = s.spawn(move || {
                for i in 0..5 {
                    add_second_commit(&dir_clone, 4 + i);
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            });

            for _ in 0..num_threads {
                let manager = Arc::clone(&manager);
                let refresh_done = Arc::clone(&refresh_done);
                s.spawn(move || {
                    for _ in 0..iterations {
                        // acquire / release
                        let r = manager.acquire().unwrap();
                        let _ = r.max_doc();
                        manager.release(r).unwrap();
                        // maybe_refresh (non-blocking)
                        if manager.maybe_refresh().unwrap() {
                            refresh_done.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                });
            }

            producer.join().unwrap();
        });

        // All acquire/release paired; the manager should not be closed.
        assert!(!manager.is_closed());
        // At least one refresh should have run.
        assert!(refresh_done.load(Ordering::Relaxed) > 0);

        // Final state: acquire a reader, release it, then close cleanly.
        let final_reader = manager.acquire().unwrap();
        assert!(final_reader.max_doc() >= 42);
        manager.release(final_reader).unwrap();
        manager.close().unwrap();
        assert!(manager.is_closed());
    }

    // -- extra helper: construct with a custom source (test-only) -----------

    /// Test-only extension allowing a custom [`RefreshSource`].
    impl ReaderManager {
        fn from_reader_with_source(
            reader: Arc<dyn DirectoryReader>,
            source: Arc<dyn RefreshSource<dyn DirectoryReader>>,
        ) -> Result<Self> {
            Ok(ReferenceManager::new(reader, source))
        }
    }
}
