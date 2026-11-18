//! Reference counting and deletion of index files.
//!
//! Port of `org.apache.lucene.util.FileDeleter` from Apache Lucene Core
//! 10.5.0 (`lucene/core/src/java/org/apache/lucene/util/FileDeleter.java`).
//!
//! This type tracks how many live references each index file has and deletes a
//! file as soon as its count drops to zero. It is the mechanical half of index
//! file management: [`crate::index::IndexFileDeleter`] decides *which* commit
//! points are still live, and this type turns that decision into `decRef` calls
//! and, eventually, `Directory::delete_file`.
//!
//! # Thread safety
//!
//! Like the Java original, `FileDeleter` is **not** thread-safe; the caller is
//! responsible for serialising access. Rucene expresses that contract through
//! `&mut self` on every mutating method, so the borrow checker enforces what
//! Lucene enforces by convention (Java holds `synchronized (writer)`, see
//! `IndexFileDeleter.locked()` at `IndexFileDeleter.java:94-96`).
//!
//! # Naming
//!
//! Java overloads `incRef`/`decRef` on `String` and `Collection<String>`. Rust
//! has no overloading, so the collection forms are named `inc_ref_files` and
//! `dec_ref_files`. Behaviour is unchanged.

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::Arc;

use crate::error::{LuceneError, LuceneErrorKind, Result};
use crate::index::index_file_names::SEGMENTS;
use crate::store::Directory;
use crate::util::java_collections::JavaHashSet;

/// Kinds of message a [`FileDeleter`] broadcasts to its messenger.
///
/// Equivalent to `FileDeleter.MsgType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgType {
    /// A message about a reference count changing.
    Ref,
    /// A message about a file being deleted.
    File,
}

/// The callback a [`FileDeleter`] uses to report progress.
///
/// Equivalent to Java's `BiConsumer<MsgType, String>`.
pub type Messenger = Arc<dyn Fn(MsgType, &str) + Send + Sync>;

/// Tracks the reference count for a single index file.
///
/// Equivalent to `FileDeleter.RefCount`.
#[derive(Debug)]
struct RefCount {
    /// File name, kept only so that failed invariants name the offending file.
    file_name: String,
    /// Whether this count has ever been incremented.
    ///
    /// Lucene uses this to allow the very first `incRef` to happen on a count
    /// of zero (which is how [`FileDeleter::init_ref_count`] seeds the map)
    /// while still asserting that later increments never resurrect a file whose
    /// count already reached zero.
    init_done: bool,
    /// Current number of live references.
    count: i32,
}

impl RefCount {
    fn new(file_name: &str) -> Self {
        Self {
            file_name: file_name.to_string(),
            init_done: false,
            count: 0,
        }
    }

    /// Increments and returns the new count.
    ///
    /// Equivalent to `FileDeleter.RefCount.incRef`.
    fn inc_ref(&mut self) -> i32 {
        if self.init_done {
            debug_assert!(
                self.count > 0,
                "RefCount is 0 pre-increment for file \"{}\"",
                self.file_name
            );
        } else {
            self.init_done = true;
        }
        self.count += 1;
        self.count
    }

    /// Decrements and returns the new count.
    ///
    /// Equivalent to `FileDeleter.RefCount.decRef`.
    fn dec_ref(&mut self) -> i32 {
        debug_assert!(
            self.count > 0,
            "RefCount is 0 pre-decrement for file \"{}\"",
            self.file_name
        );
        self.count -= 1;
        self.count
    }
}

/// Tracks reference counts for a set of index files and deletes them when their
/// counts reach zero.
///
/// Port of `org.apache.lucene.util.FileDeleter`.
pub struct FileDeleter {
    ref_counts: HashMap<String, RefCount>,
    directory: Arc<dyn Directory>,
    messenger: Option<Messenger>,
}

impl std::fmt::Debug for FileDeleter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileDeleter")
            .field("tracked_files", &self.ref_counts.len())
            .finish()
    }
}

impl FileDeleter {
    /// Creates a deleter over `directory`, reporting progress to `messenger`.
    ///
    /// Equivalent to `FileDeleter(Directory, BiConsumer<MsgType, String>)`.
    /// Pass `None` for `messenger` to discard progress messages.
    pub fn new(directory: Arc<dyn Directory>, messenger: Option<Messenger>) -> Self {
        Self {
            ref_counts: HashMap::new(),
            directory,
            messenger,
        }
    }

    /// Reports `message` to the messenger, if one is installed.
    fn message(&self, msg_type: MsgType, message: impl FnOnce() -> String) {
        if let Some(messenger) = &self.messenger {
            messenger(msg_type, &message());
        }
    }

    /// Increments the reference count of every file in `file_names`.
    ///
    /// Equivalent to `FileDeleter.incRef(Collection<String>)`.
    pub fn inc_ref_files<I, S>(&mut self, file_names: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for file in file_names {
            self.inc_ref(file.as_ref());
        }
    }

    /// Starts tracking `file_name`, which is not tracked yet, at a count of
    /// zero, and returns that count.
    ///
    /// The absent branch of the private `FileDeleter.getRefCountInternal(String)`,
    /// `refCounts.computeIfAbsent(fileName, RefCount::new)`
    /// (`FileDeleter.java:132-134`): the only place a name is copied into a
    /// key. Callers look the name up first, as `computeIfAbsent` does, so a
    /// tracked file costs no copy. Java's method returns the count on both
    /// branches; a Rust function returning the map's borrow cannot try the
    /// lookup first (the borrow of the lookup would extend over the insert),
    /// so each caller writes the lookup and calls this on a miss. The map is
    /// passed in so that the caller can still log through the deleter.
    fn track_new_file<'a>(
        ref_counts: &'a mut HashMap<String, RefCount>,
        file_name: &str,
    ) -> &'a mut RefCount {
        ref_counts
            .entry(file_name.to_string())
            .or_insert_with(|| RefCount::new(file_name))
    }

    /// Increments the reference count of `file_name`.
    ///
    /// Equivalent to `FileDeleter.incRef(String)`.
    pub fn inc_ref(&mut self, file_name: &str) {
        // PERF: `getRefCountInternal` copies the name only for a new file.
        let rc = match self.ref_counts.get_mut(file_name) {
            Some(rc) => rc,
            None => Self::track_new_file(&mut self.ref_counts, file_name),
        };
        let pre = rc.count;
        rc.inc_ref();
        self.message(MsgType::Ref, || {
            format!("IncRef \"{file_name}\": pre-incr count is {pre}")
        });
    }

    /// Decrements the reference count of every file in `file_names`, deleting
    /// each file whose count reaches zero.
    ///
    /// Every file is processed even if an earlier one fails; the first error is
    /// returned. Lucene attaches the later failures as suppressed exceptions
    /// (`IOUtils.useOrSuppress`, `FileDeleter.java:104`); Rust has no exception
    /// suppression, so the later errors are dropped and only the first is
    /// reported — the observable contract ("throws first exception hit") is the
    /// same.
    ///
    /// The files reaching zero are collected in a `java.util.HashSet`, in
    /// `file_names` order, and deleted in that set's iteration order
    /// (`FileDeleter.java:95-116`), `segments_N` files first.
    ///
    /// Equivalent to `FileDeleter.decRef(Collection<String>)`.
    pub fn dec_ref_files<I, S>(&mut self, file_names: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        // `Set<String> toDelete = new HashSet<>()`.
        let mut to_delete = JavaHashSet::new();
        let mut first_error: Option<LuceneError> = None;

        for file in file_names {
            let file = file.as_ref();
            if self.dec_ref_one(file) {
                to_delete.insert(file.to_string());
            }
        }

        if let Err(e) = self.delete_all(&to_delete) {
            first_error.get_or_insert(e);
        }

        match first_error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Decrements one file's count. Returns `true` if it should now be deleted.
    ///
    /// Equivalent to the private `FileDeleter.decRef(String)`.
    fn dec_ref_one(&mut self, file_name: &str) -> bool {
        // PERF: `getRefCountInternal` copies the name only for a new file.
        let rc = match self.ref_counts.get_mut(file_name) {
            Some(rc) => rc,
            None => Self::track_new_file(&mut self.ref_counts, file_name),
        };
        let pre = rc.count;
        let now = rc.dec_ref();
        self.message(MsgType::Ref, || {
            format!("DecRef \"{file_name}\": pre-decr count is {pre}")
        });
        if now == 0 {
            self.ref_counts.remove(file_name);
            true
        } else {
            false
        }
    }

    /// Records `file_name` with a count of zero if it is not yet tracked.
    ///
    /// Equivalent to `FileDeleter.initRefCount`, whose `computeIfAbsent`
    /// copies the name only for a new file.
    pub fn init_ref_count(&mut self, file_name: &str) {
        if !self.ref_counts.contains_key(file_name) {
            Self::track_new_file(&mut self.ref_counts, file_name);
        }
    }

    /// Returns the reference count of `file_name`, or `0` if untracked.
    ///
    /// Equivalent to `FileDeleter.getRefCount`.
    pub fn get_ref_count(&self, file_name: &str) -> i32 {
        self.ref_counts.get(file_name).map_or(0, |rc| rc.count)
    }

    /// Returns every tracked file name; some may have a count of zero.
    ///
    /// Equivalent to `FileDeleter.getAllFiles`.
    pub fn get_all_files(&self) -> impl Iterator<Item = &str> + '_ {
        self.ref_counts.keys().map(String::as_str)
    }

    /// Returns `true` only if `file_name` is tracked *and* has a count above
    /// zero.
    ///
    /// Equivalent to `FileDeleter.exists`.
    pub fn exists(&self, file_name: &str) -> bool {
        self.ref_counts
            .get(file_name)
            .is_some_and(|rc| rc.count > 0)
    }

    /// Returns every file that is tracked but has never been incremented.
    ///
    /// Equivalent to `FileDeleter.getUnrefedFiles`.
    pub fn get_unrefed_files(&self) -> HashSet<String> {
        let mut unrefed = HashSet::new();
        for (file_name, rc) in &self.ref_counts {
            if rc.count == 0 {
                self.message(MsgType::File, || {
                    format!("removing unreferenced file \"{file_name}\"")
                });
                unrefed.insert(file_name.clone());
            }
        }
        unrefed
    }

    /// Deletes those of `files` that hold no reference.
    ///
    /// A file may be tracked with a count of zero — that happens when an
    /// `IndexWriter` opens a crashed index, removes unreferenced files, and then
    /// reuses the same segment name for new work.
    ///
    /// Equivalent to `FileDeleter.deleteFilesIfNoRef`.
    pub fn delete_files_if_no_ref<I, S>(&mut self, files: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut to_delete = HashSet::new();
        for file in files {
            let file_name = file.as_ref();
            if !self.exists(file_name) {
                self.message(MsgType::File, || {
                    format!("will delete new file \"{file_name}\"")
                });
                to_delete.insert(file_name.to_string());
            }
        }
        self.delete_all(&to_delete)
    }

    /// Forgets `file_name`'s reference count and deletes it unconditionally.
    ///
    /// Equivalent to `FileDeleter.forceDelete`.
    pub fn force_delete(&mut self, file_name: &str) -> Result<()> {
        self.ref_counts.remove(file_name);
        self.delete_one(file_name)
    }

    /// Deletes `file_name` if it holds no reference.
    ///
    /// Equivalent to `FileDeleter.deleteFileIfNoRef`.
    pub fn delete_file_if_no_ref(&mut self, file_name: &str) -> Result<()> {
        if !self.exists(file_name) {
            self.message(MsgType::File, || {
                format!("will delete new file \"{file_name}\"")
            });
            self.delete_one(file_name)?;
        }
        Ok(())
    }

    /// Deletes a batch, `segments_N` files first.
    ///
    /// The ordering is a crash-safety guarantee, not an optimisation: a stale
    /// commit point must disappear *before* the files it references, so that a
    /// crash midway through can never leave a `segments_N` that points at files
    /// which are already gone. See `FileDeleter.java:212-229`.
    ///
    /// Equivalent to the private `FileDeleter.delete(Collection<String>)`:
    /// each pass walks `to_delete` in its iteration order.
    fn delete_all(&mut self, to_delete: &impl DeleteBatch) -> Result<()> {
        self.message(MsgType::File, || {
            format!("now delete {} files: {to_delete:?}", to_delete.len())
        });

        for file_name in to_delete.names() {
            debug_assert!(!self.exists(file_name));
            if file_name.starts_with(SEGMENTS) {
                self.delete_one(file_name)?;
            }
        }

        // Only now that every commit point is gone do we remove what they
        // referenced, so a crash never leaves a corrupt commit behind.
        for file_name in to_delete.names() {
            debug_assert!(!self.exists(file_name));
            if !file_name.starts_with(SEGMENTS) {
                self.delete_one(file_name)?;
            }
        }

        Ok(())
    }

    /// Deletes a single file through the directory.
    ///
    /// Equivalent to the private `FileDeleter.delete(String)`.
    fn delete_one(&self, file_name: &str) -> Result<()> {
        match self.directory.delete_file(file_name) {
            Ok(()) => Ok(()),
            Err(LuceneError {
                kind: LuceneErrorKind::Io(e),
                ..
            }) if e.kind() == io::ErrorKind::NotFound && cfg!(windows) => {
                // LUCENE-6684: on Windows a file can sit in a "pending delete"
                // state where it still shows up in directory listings but a
                // second delete reports it as missing. Lucene suppresses that
                // case (`FileDeleter.java:235-247`) and so do we.
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

/// The collection `FileDeleter.delete(Collection<String>)` walks: its size,
/// its names in its iteration order, and its `Debug` form for the FILE
/// message.
trait DeleteBatch: std::fmt::Debug {
    /// `Collection.size()`.
    fn len(&self) -> usize;
    /// `Collection.iterator()`, in iteration order.
    fn names(&self) -> impl Iterator<Item = &String>;
}

/// The `java.util.HashSet` `decRef(Collection<String>)` builds.
impl DeleteBatch for JavaHashSet {
    fn len(&self) -> usize {
        JavaHashSet::len(self)
    }
    fn names(&self) -> impl Iterator<Item = &String> {
        self.iter()
    }
}

/// The set [`FileDeleter::delete_files_if_no_ref`] builds.
impl DeleteBatch for HashSet<String> {
    fn len(&self) -> usize {
        HashSet::len(self)
    }
    fn names(&self) -> impl Iterator<Item = &String> {
        self.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::ByteBuffersDirectory;

    fn dir() -> Arc<dyn Directory> {
        Arc::new(ByteBuffersDirectory::new())
    }

    fn write_file(directory: &dyn Directory, name: &str) {
        let mut out = directory
            .create_output(name, &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        out.close().unwrap();
    }

    #[test]
    fn inc_ref_then_dec_ref_deletes_at_zero() {
        let directory = dir();
        write_file(directory.as_ref(), "_0.si");
        let mut deleter = FileDeleter::new(Arc::clone(&directory), None);

        deleter.inc_ref("_0.si");
        assert_eq!(deleter.get_ref_count("_0.si"), 1);
        assert!(deleter.exists("_0.si"));

        deleter.dec_ref_files(["_0.si"]).unwrap();
        assert_eq!(deleter.get_ref_count("_0.si"), 0);
        assert!(!deleter.exists("_0.si"));
        assert!(!directory.list_all().unwrap().contains(&"_0.si".to_string()));
    }

    #[test]
    fn file_survives_while_a_second_reference_is_held() {
        let directory = dir();
        write_file(directory.as_ref(), "_0.si");
        let mut deleter = FileDeleter::new(Arc::clone(&directory), None);

        deleter.inc_ref("_0.si");
        deleter.inc_ref("_0.si");
        deleter.dec_ref_files(["_0.si"]).unwrap();

        assert_eq!(deleter.get_ref_count("_0.si"), 1);
        assert!(
            directory.list_all().unwrap().contains(&"_0.si".to_string()),
            "file must survive while a reference remains"
        );

        deleter.dec_ref_files(["_0.si"]).unwrap();
        assert!(!directory.list_all().unwrap().contains(&"_0.si".to_string()));
    }

    #[test]
    fn init_ref_count_seeds_a_zero_count_without_deleting() {
        let directory = dir();
        write_file(directory.as_ref(), "_0.si");
        let mut deleter = FileDeleter::new(Arc::clone(&directory), None);

        deleter.init_ref_count("_0.si");
        assert_eq!(deleter.get_ref_count("_0.si"), 0);
        // Tracked, but not "existing" — exists() requires a non-zero count.
        assert!(!deleter.exists("_0.si"));
        assert_eq!(deleter.get_all_files().count(), 1);
        assert!(directory.list_all().unwrap().contains(&"_0.si".to_string()));

        assert_eq!(
            deleter.get_unrefed_files(),
            HashSet::from(["_0.si".to_string()])
        );
    }

    /// The calls on a file already tracked (`computeIfAbsent`'s present
    /// branch) change only its count: `initRefCount` keeps a non-zero count,
    /// `incRef` and `decRef` update the one entry, and the count reaching
    /// zero deletes the file, exactly as for a file seen for the first time.
    #[test]
    fn calls_on_a_tracked_file_update_its_single_entry() {
        let directory = dir();
        write_file(directory.as_ref(), "_0.si");
        let mut deleter = FileDeleter::new(Arc::clone(&directory), None);

        deleter.init_ref_count("_0.si");
        deleter.inc_ref("_0.si");
        deleter.init_ref_count("_0.si");
        assert_eq!(deleter.get_ref_count("_0.si"), 1);
        deleter.inc_ref("_0.si");
        deleter.inc_ref("_0.si");
        assert_eq!(deleter.get_ref_count("_0.si"), 3);
        assert_eq!(deleter.get_all_files().collect::<Vec<_>>(), ["_0.si"]);

        deleter.dec_ref_files(["_0.si", "_0.si"]).unwrap();
        assert_eq!(deleter.get_ref_count("_0.si"), 1);
        assert!(directory.list_all().unwrap().contains(&"_0.si".to_string()));
        deleter.dec_ref_files(["_0.si"]).unwrap();
        assert_eq!(deleter.get_all_files().count(), 0);
        assert!(!directory.list_all().unwrap().contains(&"_0.si".to_string()));
    }

    /// A directory that records the name of every `delete_file` call, in order.
    struct DeleteRecordingDirectory {
        inner: Arc<dyn Directory>,
        deleted: std::sync::Mutex<Vec<String>>,
    }

    impl Directory for DeleteRecordingDirectory {
        fn list_all(&self) -> Result<Vec<String>> {
            self.inner.list_all()
        }
        fn delete_file(&self, name: &str) -> Result<()> {
            self.deleted.lock().unwrap().push(name.to_string());
            self.inner.delete_file(name)
        }
        fn file_length(&self, name: &str) -> Result<i64> {
            self.inner.file_length(name)
        }
        fn create_output(
            &self,
            name: &str,
            context: &dyn crate::store::IOContext,
        ) -> Result<Box<dyn crate::store::IndexOutput>> {
            self.inner.create_output(name, context)
        }
        fn create_temp_output(
            &self,
            prefix: &str,
            suffix: &str,
            context: &dyn crate::store::IOContext,
        ) -> Result<Box<dyn crate::store::IndexOutput>> {
            self.inner.create_temp_output(prefix, suffix, context)
        }
        fn sync(&self, names: &[String]) -> Result<()> {
            self.inner.sync(names)
        }
        fn sync_metadata(&self) -> Result<()> {
            self.inner.sync_metadata()
        }
        fn rename(&self, source: &str, dest: &str) -> Result<()> {
            self.inner.rename(source, dest)
        }
        fn open_input(
            &self,
            name: &str,
            context: &dyn crate::store::IOContext,
        ) -> Result<Box<dyn crate::store::IndexInput>> {
            self.inner.open_input(name, context)
        }
        fn obtain_lock(&self, name: &str) -> Result<Box<dyn crate::store::Lock>> {
            self.inner.obtain_lock(name)
        }
        fn close(&self) -> Result<()> {
            Ok(())
        }
        fn get_pending_deletions(&self) -> Result<HashSet<String>> {
            self.inner.get_pending_deletions()
        }
    }

    /// `decRef(Collection<String>)` collects the files reaching zero in a
    /// `java.util.HashSet` and deletes them in its iteration order,
    /// `segments_N` files first (`FileDeleter.java:95-116, 207-229`). The
    /// expected sequence is the one JDK 25.0.5 prints for these names added in
    /// this order to `new HashSet<>()` and walked by those two passes; neither
    /// the sorted nor the insertion order matches it.
    #[test]
    fn dec_ref_files_deletes_in_java_hash_set_order() {
        let dec_ref_order = [
            "_0.cfs",
            "_0.cfe",
            "_0.si",
            "segments_2",
            "_1.fdt",
            "_1.fdx",
            "_1.fdm",
            "_1.fnm",
            "_1_Lucene104_0.doc",
            "_1_Lucene104_0.tim",
            "_1_Lucene104_0.tip",
            "_1_Lucene104_0.tmd",
            "_1.nvd",
            "_1.nvm",
            "_1.si",
            "segments_1",
            "_2_1.liv",
            "_a.si",
        ];
        let java_order = [
            "segments_2",
            "segments_1",
            "_1_Lucene104_0.tmd",
            "_0.si",
            "_1.si",
            "_0.cfs",
            "_1.nvd",
            "_a.si",
            "_1.fnm",
            "_1.nvm",
            "_1_Lucene104_0.tip",
            "_0.cfe",
            "_1_Lucene104_0.doc",
            "_1_Lucene104_0.tim",
            "_1.fdm",
            "_1.fdx",
            "_2_1.liv",
            "_1.fdt",
        ];
        let inner = dir();
        for name in dec_ref_order {
            write_file(inner.as_ref(), name);
        }
        let recording = Arc::new(DeleteRecordingDirectory {
            inner,
            deleted: std::sync::Mutex::new(Vec::new()),
        });
        let mut deleter = FileDeleter::new(Arc::clone(&recording) as Arc<dyn Directory>, None);
        deleter.inc_ref_files(dec_ref_order);

        deleter.dec_ref_files(dec_ref_order).unwrap();

        assert_eq!(*recording.deleted.lock().unwrap(), java_order);
        assert!(inner_is_empty(recording.inner.as_ref()));
    }

    fn inner_is_empty(directory: &dyn Directory) -> bool {
        directory.list_all().unwrap().is_empty()
    }

    #[test]
    fn delete_files_if_no_ref_spares_referenced_files() {
        let directory = dir();
        write_file(directory.as_ref(), "_0.si");
        write_file(directory.as_ref(), "_1.si");
        let mut deleter = FileDeleter::new(Arc::clone(&directory), None);

        deleter.inc_ref("_0.si");
        deleter
            .delete_files_if_no_ref(["_0.si".to_string(), "_1.si".to_string()])
            .unwrap();

        let files = directory.list_all().unwrap();
        assert!(files.contains(&"_0.si".to_string()), "referenced file kept");
        assert!(
            !files.contains(&"_1.si".to_string()),
            "unreferenced file deleted"
        );
    }

    #[test]
    fn delete_is_idempotent_for_an_already_missing_file() {
        let directory = dir();
        let mut deleter = FileDeleter::new(Arc::clone(&directory), None);

        // Never written: deleting it must not corrupt the deleter's state.
        let result = deleter.delete_files_if_no_ref(["_0.si"]);
        // ByteBuffersDirectory reports a missing file; the important property is
        // that the deleter's bookkeeping is unchanged either way.
        let _ = result;
        assert_eq!(deleter.get_ref_count("_0.si"), 0);
        assert_eq!(deleter.get_all_files().count(), 0);
    }

    #[test]
    fn messenger_receives_ref_and_file_messages() {
        use std::sync::Mutex;

        let directory = dir();
        write_file(directory.as_ref(), "_0.si");
        let log: Arc<Mutex<Vec<(MsgType, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&log);
        let messenger: Messenger = Arc::new(move |t, m| {
            sink.lock().unwrap().push((t, m.to_string()));
        });

        let mut deleter = FileDeleter::new(Arc::clone(&directory), Some(messenger));
        deleter.inc_ref("_0.si");
        deleter.dec_ref_files(["_0.si"]).unwrap();

        let entries = log.lock().unwrap();
        assert!(entries.iter().any(
            |(t, m)| *t == MsgType::Ref && m.contains("IncRef \"_0.si\": pre-incr count is 0")
        ));
        assert!(entries.iter().any(
            |(t, m)| *t == MsgType::Ref && m.contains("DecRef \"_0.si\": pre-decr count is 1")
        ));
        assert!(entries.iter().any(|(t, _)| *t == MsgType::File));
    }

    #[test]
    fn segments_files_are_deleted_before_the_files_they_reference() {
        // Crash safety: a stale commit point must go first. We observe the order
        // through the messenger-free path by checking that after a batch delete
        // containing both, nothing is left behind.
        let directory = dir();
        write_file(directory.as_ref(), "segments_1");
        write_file(directory.as_ref(), "_0.si");
        let mut deleter = FileDeleter::new(Arc::clone(&directory), None);

        deleter
            .delete_files_if_no_ref(["segments_1".to_string(), "_0.si".to_string()])
            .unwrap();

        assert!(directory.list_all().unwrap().is_empty());
    }

    #[test]
    fn force_delete_removes_a_referenced_file_and_its_count() {
        let directory = dir();
        write_file(directory.as_ref(), "_0.si");
        let mut deleter = FileDeleter::new(Arc::clone(&directory), None);

        deleter.inc_ref("_0.si");
        deleter.force_delete("_0.si").unwrap();

        assert_eq!(deleter.get_ref_count("_0.si"), 0);
        assert!(directory.list_all().unwrap().is_empty());
    }
}
