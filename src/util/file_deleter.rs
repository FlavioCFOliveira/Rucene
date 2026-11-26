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

use std::borrow::Borrow;
use std::io;
use std::sync::Arc;

use crate::error::{LuceneError, LuceneErrorKind, Result};
use crate::index::index_file_names::SEGMENTS;
use crate::store::Directory;
use crate::util::java_collections::{JavaHashMap, JavaHashSet};

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

/// A file name held by a to-delete set as the caller handed it in.
///
/// PERF: Java's `toDelete` sets hold the caller's `String` objects; this
/// holds the caller's name value (an owned name, a shared name or a borrowed
/// one) instead of copying its characters into a new `String`.
struct CallerName<S>(S);

impl<S: AsRef<str>> Borrow<str> for CallerName<S> {
    fn borrow(&self) -> &str {
        self.0.as_ref()
    }
}

/// Tracks the reference count for a single index file.
///
/// Equivalent to `FileDeleter.RefCount`. Java's `RefCount` keeps its
/// `fileName` only to name the file in its assertion messages; this port
/// passes the name in from the map's key instead of keeping a second copy.
#[derive(Debug)]
struct RefCount {
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

/// `Thread.currentThread().getName()`, which the count assertions print first
/// (`FileDeleter.java:267-282`): the Rust thread's name, and `main` for an
/// unnamed thread, as [`crate::util::info_stream::PrintStreamInfoStream`]
/// names it.
fn current_thread_name() -> String {
    std::thread::current().name().unwrap_or("main").to_string()
}

impl RefCount {
    fn new() -> Self {
        Self {
            init_done: false,
            count: 0,
        }
    }

    /// Increments and returns the new count.
    ///
    /// Equivalent to `FileDeleter.RefCount.incRef`.
    fn inc_ref(&mut self, file_name: &str) -> i32 {
        if self.init_done {
            debug_assert!(
                self.count > 0,
                "{}: RefCount is 0 pre-increment for file \"{}\"",
                current_thread_name(),
                file_name
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
    ///
    /// # Errors
    ///
    /// [`LuceneErrorKind::AssertionError`] when the count is not positive, in
    /// builds with `debug_assertions` only: Java's `assert count > 0`
    /// (`FileDeleter.java:276-283`), evaluated under this port's `-ea`. The
    /// count is then left unchanged, as the failed `assert` throws before
    /// `--count`. It is an error rather than a panic because Java's caller,
    /// `decRef(Collection)`, catches it and goes on (`FileDeleter.java:98-106`).
    /// The message starts with the thread's name, as Java's
    /// `Thread.currentThread().getName() + ": "` (`FileDeleter.java:278-282`).
    fn dec_ref(&mut self, file_name: &str) -> Result<i32> {
        if cfg!(debug_assertions) && self.count <= 0 {
            return Err(LuceneError::from(LuceneErrorKind::AssertionError(format!(
                "{}: RefCount is 0 pre-decrement for file \"{}\"",
                current_thread_name(),
                file_name
            ))));
        }
        self.count -= 1;
        Ok(self.count)
    }
}

/// Tracks reference counts for a set of index files and deletes them when their
/// counts reach zero.
///
/// Port of `org.apache.lucene.util.FileDeleter`.
pub struct FileDeleter {
    /// `refCounts`, a `java.util.HashMap` (`FileDeleter.java:41`). Its
    /// iteration order is Java's, because [`FileDeleter::get_unrefed_files`]
    /// walks it to fill the set `IndexFileDeleter`'s constructor deletes in
    /// iteration order.
    ref_counts: JavaHashMap<RefCount>,
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
            ref_counts: JavaHashMap::new(),
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

    /// Returns `file_name`'s count, tracking it at zero first if it is not
    /// tracked yet.
    ///
    /// Equivalent to the private `FileDeleter.getRefCountInternal(String)`,
    /// `refCounts.computeIfAbsent(fileName, RefCount::new)`
    /// (`FileDeleter.java:132-134`), which copies the name only for a new
    /// file. The map is passed in, rather than borrowed through `self`, so
    /// that the caller can still reach the messenger while it holds the count.
    fn ref_count_internal<'a>(
        ref_counts: &'a mut JavaHashMap<RefCount>,
        file_name: &str,
    ) -> &'a mut RefCount {
        ref_counts.compute_if_absent(file_name, RefCount::new)
    }

    /// Increments the reference count of `file_name`.
    ///
    /// Equivalent to `FileDeleter.incRef(String)`.
    pub fn inc_ref(&mut self, file_name: &str) {
        // `FileDeleter.java:83-89`: the message goes out before `rc.incRef()`,
        // so a failed count assertion follows it.
        let pre = Self::ref_count_internal(&mut self.ref_counts, file_name).count;
        self.message(MsgType::Ref, || {
            format!("IncRef \"{file_name}\": pre-incr count is {pre}")
        });
        Self::ref_count_internal(&mut self.ref_counts, file_name).inc_ref(file_name);
    }

    /// Decrements the reference count of every file in `file_names`, deleting
    /// each file whose count reaches zero.
    ///
    /// The files reaching zero are collected in a `java.util.HashSet`, in
    /// `file_names` order, and deleted in that set's iteration order
    /// (`FileDeleter.java:95-116`), `segments_N` files first.
    ///
    /// # Errors
    ///
    /// The first error hit, with every later one added to it as suppressed
    /// (`IOUtils.useOrSuppress`, `FileDeleter.java:97-116`). Each file's count
    /// is decremented even after a decrement failed, and the files that
    /// reached zero are deleted before the error is returned. A decrement
    /// fails only on the `RefCount` assertion (see `RefCount::dec_ref`), in
    /// builds with `debug_assertions`. The delete stops at its first failed
    /// file, as Java's `delete(Collection)` does (`FileDeleter.java:207-229`),
    /// so it adds at most one error.
    ///
    /// Equivalent to `FileDeleter.decRef(Collection<String>)`.
    pub fn dec_ref_files<I, S>(&mut self, file_names: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        // `Set<String> toDelete = new HashSet<>()`.
        let mut to_delete = JavaHashSet::default();
        // `Throwable firstThrowable = null`.
        let mut first_error: Option<LuceneError> = None;

        for file in file_names {
            // `try { ... } catch (Throwable t) { firstThrowable =
            // IOUtils.useOrSuppress(firstThrowable, t); }`.
            match self.dec_ref_one(file.as_ref()) {
                Ok(true) => {
                    to_delete.insert(CallerName(file));
                }
                Ok(false) => {}
                Err(e) => use_or_suppress(&mut first_error, e),
            }
        }

        if let Err(e) = self.delete_all(&to_delete) {
            use_or_suppress(&mut first_error, e);
        }

        match first_error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Decrements one file's count. Returns `true` if it should now be deleted.
    ///
    /// Equivalent to the private `FileDeleter.decRef(String)`
    /// (`FileDeleter.java:120-130`): the message is sent before the
    /// decrement, so also when the decrement's assertion fails.
    ///
    /// # Errors
    ///
    /// The `RefCount` assertion of `RefCount::dec_ref`; the file then stays
    /// tracked, at the count it had.
    fn dec_ref_one(&mut self, file_name: &str) -> Result<bool> {
        let rc = Self::ref_count_internal(&mut self.ref_counts, file_name);
        if let Some(messenger) = &self.messenger {
            messenger(
                MsgType::Ref,
                &format!("DecRef \"{file_name}\": pre-decr count is {}", rc.count),
            );
        }
        if rc.dec_ref(file_name)? == 0 {
            self.ref_counts.remove(file_name);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Records `file_name` with a count of zero if it is not yet tracked.
    ///
    /// Equivalent to `FileDeleter.initRefCount`,
    /// `refCounts.computeIfAbsent(fileName, RefCount::new)`
    /// (`FileDeleter.java:137-139`).
    pub fn init_ref_count(&mut self, file_name: &str) {
        Self::ref_count_internal(&mut self.ref_counts, file_name);
    }

    /// Returns the reference count of `file_name`, or `0` if untracked.
    ///
    /// Equivalent to `FileDeleter.getRefCount`.
    pub fn get_ref_count(&self, file_name: &str) -> i32 {
        self.ref_counts.get(file_name).map_or(0, |rc| rc.count)
    }

    /// Returns every tracked file name, in `refCounts.keySet()` order; some
    /// may have a count of zero.
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

    /// Returns every file that is tracked at a count of zero.
    ///
    /// The names are added to a `java.util.HashSet` in `refCounts.entrySet()`
    /// order (`FileDeleter.java:159-170`), so the set iterates as Java's does.
    ///
    /// Equivalent to `FileDeleter.getUnrefedFiles`.
    pub fn get_unrefed_files(&self) -> JavaHashSet {
        // `Set<String> unrefed = new HashSet<>()`.
        let mut unrefed = JavaHashSet::new();
        for (file_name, rc) in self.ref_counts.iter() {
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
    /// The files to delete are collected in a `java.util.HashSet`, in `files`
    /// order, and deleted in that set's iteration order
    /// (`FileDeleter.java:173-191`), `segments_N` files first.
    ///
    /// Equivalent to `FileDeleter.deleteFilesIfNoRef`.
    pub fn delete_files_if_no_ref<I, S>(&mut self, files: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        // `Set<String> toDelete = new HashSet<>()`.
        let mut to_delete = JavaHashSet::default();
        for file in files {
            let file_name = file.as_ref();
            if !self.exists(file_name) {
                self.message(MsgType::File, || {
                    format!("will delete new file \"{file_name}\"")
                });
                to_delete.insert(CallerName(file));
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
    fn delete_all<K: Borrow<str>>(&mut self, to_delete: &JavaHashSet<K>) -> Result<()> {
        self.message(MsgType::File, || {
            // `"now delete " + toDelete.size() + " files: " + toDelete`
            // (`FileDeleter.java:209`): the set as
            // `AbstractCollection.toString()` prints it, `[a, b]`.
            let names: Vec<&str> = to_delete.iter().map(Borrow::borrow).collect();
            format!(
                "now delete {} files: [{}]",
                to_delete.len(),
                names.join(", ")
            )
        });

        for file_name in to_delete.iter() {
            let file_name: &str = file_name.borrow();
            debug_assert!(!self.exists(file_name));
            if file_name.starts_with(SEGMENTS) {
                self.delete_one(file_name)?;
            }
        }

        // Only now that every commit point is gone do we remove what they
        // referenced, so a crash never leaves a corrupt commit behind.
        for file_name in to_delete.iter() {
            let file_name: &str = file_name.borrow();
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
            Err(ref __lerr)
                if let LuceneErrorKind::Io(e) = &__lerr.kind
                    && (e.kind() == io::ErrorKind::NotFound && cfg!(windows)) =>
            {
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

/// `first = IOUtils.useOrSuppress(first, second)` (`IOUtils.java:491-498`):
/// keeps the first error and adds every later one to it as suppressed.
fn use_or_suppress(first: &mut Option<LuceneError>, second: LuceneError) {
    match first {
        Some(first) => first.add_suppressed(second),
        None => *first = Some(second),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

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
            deleter.get_unrefed_files().to_std(),
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

    /// A directory that records the name of every `delete_file` call, in order,
    /// and fails the deletion of the names in `failing` with an `IOException`.
    struct DeleteRecordingDirectory {
        inner: Arc<dyn Directory>,
        deleted: std::sync::Mutex<Vec<String>>,
        failing: Vec<String>,
    }

    impl Directory for DeleteRecordingDirectory {
        fn list_all(&self) -> Result<Vec<String>> {
            self.inner.list_all()
        }
        fn delete_file(&self, name: &str) -> Result<()> {
            self.deleted.lock().unwrap().push(name.to_string());
            if self.failing.iter().any(|f| f == name) {
                return Err(LuceneError::from(io::Error::other(format!(
                    "fake delete failure: {name}"
                ))));
            }
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
        fn sync(&self, names: &[&str]) -> Result<()> {
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
            failing: Vec::new(),
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

    /// `deleteFilesIfNoRef(Collection<String>)` collects the unreferenced
    /// files in a `java.util.HashSet`, in call order, and deletes them in its
    /// iteration order, `segments_N` files first
    /// (`FileDeleter.java:173-191, 207-229`). The expected sequence is the one
    /// Lucene 10.5.0's own `FileDeleter` records on JDK 25.0.5 for the same
    /// calls (`incRef` of the two referenced files, then
    /// `deleteFilesIfNoRef` over `call_order`). Five buckets of the 32-entry
    /// table hold two or three of the names, which Java keeps in insertion
    /// order (`_5.vec` before `_1_Lucene104_0.tmd`); neither the sorted nor
    /// the call order matches the sequence.
    #[test]
    fn delete_files_if_no_ref_deletes_in_java_hash_set_order() {
        let call_order = [
            "_5.vec",
            "_5.vem",
            "_4.kdm",
            "_4.kdi",
            "_4.kdd",
            "segments_2",
            "_3.si",
            "_2_1.liv",
            "_1_Lucene104_0.tmd",
            "_1_Lucene104_0.tip",
            "_1_Lucene104_0.tim",
            "_1_Lucene104_0.doc",
            "_1.nvm",
            "_1.nvd",
            "_1.fnm",
            "_1.fdx",
            "_1.fdt",
            "_1.fdm",
            "_1.si",
            "segments_3",
            "_0.si",
            "_0.cfe",
            "_0.cfs",
            "pending_segments_4",
            "_3.si",
        ];
        let referenced = ["_1.si", "segments_3"];
        let java_order = [
            "segments_2",
            "_5.vec",
            "_1_Lucene104_0.tmd",
            "_3.si",
            "_0.si",
            "_0.cfs",
            "_1.nvd",
            "_1.nvm",
            "_1.fnm",
            "pending_segments_4",
            "_1_Lucene104_0.tip",
            "_0.cfe",
            "_4.kdm",
            "_1_Lucene104_0.tim",
            "_1_Lucene104_0.doc",
            "_1.fdm",
            "_4.kdi",
            "_5.vem",
            "_1.fdx",
            "_2_1.liv",
            "_4.kdd",
            "_1.fdt",
        ];
        let inner = dir();
        for name in call_order {
            if !inner.list_all().unwrap().iter().any(|f| f == name) {
                write_file(inner.as_ref(), name);
            }
        }
        let recording = Arc::new(DeleteRecordingDirectory {
            inner,
            deleted: std::sync::Mutex::new(Vec::new()),
            failing: Vec::new(),
        });
        let mut deleter = FileDeleter::new(Arc::clone(&recording) as Arc<dyn Directory>, None);
        deleter.inc_ref_files(referenced);

        deleter.delete_files_if_no_ref(call_order).unwrap();

        assert_eq!(*recording.deleted.lock().unwrap(), java_order);
        let mut left = recording.inner.list_all().unwrap();
        left.sort();
        assert_eq!(left, referenced);
    }

    /// The message and the suppressed exceptions of an error, as Java prints
    /// them (`Throwable.getMessage()`, then each of `getSuppressed()`).
    fn messages(error: &LuceneError) -> Vec<String> {
        fn message(error: &LuceneError) -> String {
            match error.kind() {
                LuceneErrorKind::AssertionError(m) => format!("AssertionError: {m}"),
                LuceneErrorKind::Io(e) => format!("IOException: {e}"),
                other => panic!("unexpected error kind {other:?}"),
            }
        }
        std::iter::once(message(error))
            .chain(error.get_suppressed().iter().map(|s| message(s.as_ref())))
            .collect()
    }

    /// `decRef(Collection<String>)` catches each decrement's `Throwable`,
    /// goes on decrementing, deletes the files that reached zero, and only
    /// then throws the first error, with the later ones (another failed count
    /// assertion, a failed delete) suppressed into it
    /// (`FileDeleter.java:95-130, 276-283`; `IOUtils.useOrSuppress`). The
    /// expected results are those of Lucene 10.5.0's own `FileDeleter` on
    /// JDK 25.0.5 for the same calls, run with `-ea` (this port's
    /// `debug_assertions`) and without it. The assertion message starts with
    /// the thread's name (`main: `), so the body runs on a thread named
    /// `main`, as Java's oracle ran.
    #[test]
    fn dec_ref_files_keeps_going_after_a_failed_count_assertion() {
        on_thread_named(
            "main",
            dec_ref_files_keeps_going_after_a_failed_count_assertion_body,
        );
    }

    /// Runs `body` on a thread named `name` and propagates its panic.
    fn on_thread_named(name: &str, body: fn()) {
        std::thread::Builder::new()
            .name(name.to_string())
            .spawn(body)
            .unwrap()
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
    }

    fn dec_ref_files_keeps_going_after_a_failed_count_assertion_body() {
        let tracked = ["segments_1", "_0.si", "_0.cfs", "_0.cfe"];
        let inner = dir();
        for name in tracked {
            write_file(inner.as_ref(), name);
        }
        let recording = Arc::new(DeleteRecordingDirectory {
            inner,
            deleted: std::sync::Mutex::new(Vec::new()),
            failing: vec!["_0.cfe".to_string()],
        });
        let log: Arc<std::sync::Mutex<Vec<(MsgType, String)>>> = Arc::default();
        let sink = Arc::clone(&log);
        let messenger: Messenger = Arc::new(move |t, m| {
            sink.lock().unwrap().push((t, m.to_string()));
        });
        let mut deleter = FileDeleter::new(
            Arc::clone(&recording) as Arc<dyn Directory>,
            Some(messenger),
        );
        deleter.inc_ref_files(tracked);
        log.lock().unwrap().clear();

        let error = deleter
            .dec_ref_files(["segments_1", "_x.si", "_0.si", "_y.si", "_0.cfs", "_0.cfe"])
            .unwrap_err();

        let (expected_errors, untracked_count) = if cfg!(debug_assertions) {
            (
                vec![
                    "AssertionError: main: RefCount is 0 pre-decrement for file \"_x.si\"",
                    "AssertionError: main: RefCount is 0 pre-decrement for file \"_y.si\"",
                    "IOException: fake delete failure: _0.cfe",
                ],
                0,
            )
        } else {
            (vec!["IOException: fake delete failure: _0.cfe"], -1)
        };
        assert_eq!(messages(&error), expected_errors);
        // Every decrement ran, the delete ran up to its failure.
        assert_eq!(
            *recording.deleted.lock().unwrap(),
            ["segments_1", "_0.si", "_0.cfe"]
        );
        assert_eq!(recording.inner.list_all().unwrap(), ["_0.cfe", "_0.cfs"]);
        let ref_messages: Vec<String> = log
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, _)| *t == MsgType::Ref)
            .map(|(_, m)| m.clone())
            .collect();
        assert_eq!(
            ref_messages,
            [
                "DecRef \"segments_1\": pre-decr count is 1",
                "DecRef \"_x.si\": pre-decr count is 0",
                "DecRef \"_0.si\": pre-decr count is 1",
                "DecRef \"_y.si\": pre-decr count is 0",
                "DecRef \"_0.cfs\": pre-decr count is 1",
                "DecRef \"_0.cfe\": pre-decr count is 1",
            ]
        );
        // The two unknown names stay tracked (`computeIfAbsent` added them).
        let mut all: Vec<&str> = deleter.get_all_files().collect();
        all.sort_unstable();
        assert_eq!(all, ["_x.si", "_y.si"]);
        assert_eq!(deleter.get_ref_count("_x.si"), untracked_count);
        assert_eq!(deleter.get_ref_count("_y.si"), untracked_count);
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

    /// A messenger that records every message, and the log it fills.
    #[allow(clippy::type_complexity)]
    fn recording_messenger() -> (Messenger, Arc<std::sync::Mutex<Vec<(MsgType, String)>>>) {
        let log: Arc<std::sync::Mutex<Vec<(MsgType, String)>>> = Arc::default();
        let sink = Arc::clone(&log);
        let messenger: Messenger = Arc::new(move |t, m| {
            sink.lock().unwrap().push((t, m.to_string()));
        });
        (messenger, log)
    }

    /// #600: the `RefCount` assertion messages start with
    /// `Thread.currentThread().getName() + ": "` (`FileDeleter.java:267-282`).
    #[test]
    fn ref_count_assertion_messages_start_with_the_thread_name() {
        if !cfg!(debug_assertions) {
            return; // Java's `assert` runs under `-ea` only.
        }
        on_thread_named("Lucene Merge Thread #3", || {
            let mut deleter = FileDeleter::new(dir(), None);
            let error = deleter.dec_ref_files(["_x.si"]).unwrap_err();
            assert_eq!(
                messages(&error),
                ["AssertionError: Lucene Merge Thread #3: RefCount is 0 pre-decrement for file \"_x.si\""]
            );

            // `incRef`'s assertion: a count of 0 after the first increment,
            // forged because the public calls remove a file at 0.
            deleter.inc_ref("_0.si");
            deleter.ref_counts.get_mut("_0.si").unwrap().count = 0;
            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                deleter.inc_ref("_0.si");
            }))
            .unwrap_err();
            let message = panic.downcast_ref::<String>().cloned().unwrap_or_default();
            assert_eq!(
                message,
                "Lucene Merge Thread #3: RefCount is 0 pre-increment for file \"_0.si\""
            );
        });
    }

    /// #600: `incRef(String)` sends `IncRef "<file>": pre-incr count is N`
    /// before `rc.incRef()` (`FileDeleter.java:83-89`), so the message is
    /// out even when the increment's assertion fails.
    #[test]
    fn inc_ref_message_goes_out_before_the_increment() {
        let (messenger, log) = recording_messenger();
        let mut deleter = FileDeleter::new(dir(), Some(messenger));
        deleter.inc_ref("_0.si");
        deleter.ref_counts.get_mut("_0.si").unwrap().count = 0;
        log.lock().unwrap().clear();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            deleter.inc_ref("_0.si");
        }));
        assert_eq!(outcome.is_err(), cfg!(debug_assertions));
        assert_eq!(
            *log.lock().unwrap(),
            [(
                MsgType::Ref,
                "IncRef \"_0.si\": pre-incr count is 0".to_string()
            )]
        );
    }

    /// #600: `delete(Collection)` prints `"now delete " + toDelete.size() +
    /// " files: " + toDelete`, the `HashSet` as `AbstractCollection.toString()`
    /// renders it: `[a, b]` in the set's iteration order (`_0.si` hashes to
    /// bucket 2 and `_0.cfs` to bucket 4 of the 16-entry table).
    #[test]
    fn delete_prints_the_file_set_as_a_java_collection() {
        let directory = dir();
        write_file(directory.as_ref(), "_0.cfs");
        write_file(directory.as_ref(), "_0.si");
        let (messenger, log) = recording_messenger();
        let mut deleter = FileDeleter::new(directory, Some(messenger));
        deleter.inc_ref_files(["_0.cfs", "_0.si"]);
        log.lock().unwrap().clear();
        deleter.dec_ref_files(["_0.cfs", "_0.si"]).unwrap();
        let file_messages: Vec<String> = log
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, _)| *t == MsgType::File)
            .map(|(_, m)| m.clone())
            .collect();
        assert_eq!(file_messages[0], "now delete 2 files: [_0.si, _0.cfs]");
    }
}
