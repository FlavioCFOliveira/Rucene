//! `DirectoryReader` and `StandardDirectoryReader` ported from
//! `org.apache.lucene.index`.
//!
//! This module provides the top-level reader used to open a point-in-time view
//! of an index stored in a `Directory`. It loads the current `SegmentInfos`,
//! creates one `SegmentReader` per segment, supports reopen via `open_if_changed`,
//! and exposes the commit points present in the directory.
//!
//! The `SegmentReader` placeholder used here is the minimal stub defined in
//! `segment_reader.rs`; task 93 will replace it with the full Lucene-compatible
//! implementation.

#![deny(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::fmt::{Debug, Formatter};
use std::io;
use std::sync::{Arc, Mutex, Weak};

use crate::error::{LuceneError, Result};
use crate::index::index_commit::IndexCommit;
use crate::index::index_file_names::SEGMENTS;
use crate::index::index_reader::{
    build_composite_context, CacheHelper, CacheKey, ClosedListener, CompositeReader, IndexReader,
    IndexReaderCore, StoredFields,
};
use crate::index::leaf_reader::{LeafReader, TermVectors};
use crate::index::reader_context::IndexReaderContext;
use crate::index::segment_infos::SegmentInfos;
use crate::index::segment_reader::SegmentReader;
use crate::index::Term;
use crate::store::{Directory, DEFAULT_IO_CONTEXT};

// -----------------------------------------------------------------------------
// IndexWriter (minimal trait for NRT signatures)
// -----------------------------------------------------------------------------

/// Minimal trait used by `DirectoryReader` for near-real-time interactions.
///
/// This is **not** a port of `IndexWriter`; it exists only to let
/// `StandardDirectoryReader` express the NRT API contract. The real
/// `IndexWriter` port will implement this trait in a later task.
pub trait IndexWriter: Send + Sync + Debug {
    /// Returns `true` if the reader opened from this writer is up to date with
    /// the provided `SegmentInfos`.
    fn nrt_is_current(&self, infos: &SegmentInfos) -> bool;

    /// Returns a new reader covering all changes made by this writer so far.
    fn get_reader(
        &self,
        apply_all_deletes: bool,
        write_all_deletes: bool,
    ) -> Result<Arc<dyn DirectoryReader>>;

    /// Returns `true` if the writer is closed.
    fn is_closed(&self) -> bool;

    /// Returns the directory the writer is indexing into.
    fn get_directory(&self) -> Arc<dyn Directory>;

    /// Increments the deleter ref count for the given `SegmentInfos`.
    fn inc_ref_deleter(&self, infos: &SegmentInfos) -> Result<()>;

    /// Decrements the deleter ref count for the given `SegmentInfos`.
    fn dec_ref_deleter(&self, infos: &SegmentInfos) -> Result<()>;
}

// -----------------------------------------------------------------------------
// DirectoryReader
// -----------------------------------------------------------------------------

/// Reader that can read indexes in a `Directory`.
///
/// Equivalent to `org.apache.lucene.index.DirectoryReader`.
pub trait DirectoryReader: CompositeReader {
    /// Returns the directory this index resides in.
    fn directory(&self) -> Arc<dyn Directory>;

    /// Returns the version recorded in the commit this reader opened.
    fn version(&self) -> Result<i64>;

    /// Returns `true` if no changes have occurred to the index since this reader
    /// was opened.
    fn is_current(&self) -> Result<bool>;

    /// Returns the `IndexCommit` that this reader has opened.
    fn index_commit(&self) -> Result<Box<dyn IndexCommit>>;

    /// Implements `open_if_changed(DirectoryReader)`.
    fn do_open_if_changed(&self) -> Result<Option<Arc<dyn DirectoryReader>>>;

    /// Implements `open_if_changed(DirectoryReader, IndexCommit)`.
    fn do_open_if_changed_from_commit(
        &self,
        commit: &dyn IndexCommit,
    ) -> Result<Option<Arc<dyn DirectoryReader>>>;

    /// Implements `open_if_changed(DirectoryReader, IndexWriter, boolean)`.
    fn do_open_if_changed_from_writer(
        &self,
        writer: &dyn IndexWriter,
        apply_all_deletes: bool,
    ) -> Result<Option<Arc<dyn DirectoryReader>>>;
}

// -----------------------------------------------------------------------------
// StandardDirectoryReader
// -----------------------------------------------------------------------------

/// Default implementation of `DirectoryReader`.
///
/// Equivalent to `org.apache.lucene.index.StandardDirectoryReader`.
pub struct StandardDirectoryReader {
    core: IndexReaderCore,
    directory: Arc<dyn Directory>,
    segment_readers: Vec<Arc<SegmentReader>>,
    segment_infos: SegmentInfos,
    writer: Option<Arc<dyn IndexWriter>>,
    apply_all_deletes: bool,
    write_all_deletes: bool,
    cache_helper: Arc<DirectoryReaderCacheHelper>,
}

impl Debug for StandardDirectoryReader {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StandardDirectoryReader")
            .field("directory_type", &self.directory.directory_type_name())
            .field("segments", &self.segment_infos.size())
            .field("version", &self.segment_infos.version())
            .finish()
    }
}

impl StandardDirectoryReader {
    /// Package-private constructor used by the static `open` helpers.
    fn new(
        directory: Arc<dyn Directory>,
        segment_readers: Vec<Arc<SegmentReader>>,
        segment_infos: SegmentInfos,
        writer: Option<Arc<dyn IndexWriter>>,
        apply_all_deletes: bool,
        write_all_deletes: bool,
    ) -> Result<Self> {
        let total_max_doc: i32 = segment_readers
            .iter()
            .map(|r| LeafReader::max_doc(r.as_ref()))
            .sum();
        let expected = segment_infos.total_max_doc();
        if total_max_doc != expected {
            return Err(LuceneError::CorruptIndex(format!(
                "SegmentReaders total maxDoc {total_max_doc} does not match SegmentInfos total maxDoc {expected}"
            )));
        }
        Ok(Self {
            core: IndexReaderCore::new(),
            directory,
            segment_readers,
            segment_infos,
            writer,
            apply_all_deletes,
            write_all_deletes,
            cache_helper: Arc::new(DirectoryReaderCacheHelper::new()),
        })
    }

    /// Opens a reader from an already-loaded `SegmentInfos`.
    ///
    /// `old_readers` is used for resource sharing on reopen; for the initial
    /// `open` it is `None`. This placeholder always creates fresh
    /// `SegmentReader`s; full reader reuse is left to task 93.
    fn open_from_infos(
        directory: Arc<dyn Directory>,
        infos: SegmentInfos,
        _old_readers: Option<&[Arc<SegmentReader>]>,
        writer: Option<Arc<dyn IndexWriter>>,
    ) -> Result<Arc<dyn DirectoryReader>> {
        let index_created_version = infos.index_created_version_major();
        let mut segment_readers = Vec::with_capacity(infos.size());
        for i in 0..infos.size() {
            let sci = infos.info(i);
            let reader = Arc::new(SegmentReader::new(
                sci.clone(),
                index_created_version,
                &*DEFAULT_IO_CONTEXT,
            )?);
            segment_readers.push(reader);
        }

        // TODO(task 93): reuse old readers when delGen/fieldInfosGen are unchanged.

        Ok(Arc::new(StandardDirectoryReader::new(
            directory,
            segment_readers,
            infos,
            writer,
            false,
            false,
        )?))
    }

    /// Returns the `SegmentInfos` for this reader.
    ///
    /// Equivalent to `StandardDirectoryReader.getSegmentInfos()`.
    pub fn segment_infos(&self) -> &SegmentInfos {
        &self.segment_infos
    }
}

impl IndexReader for StandardDirectoryReader {
    fn core(&self) -> &IndexReaderCore {
        &self.core
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        Err(LuceneError::UnsupportedOperation(
            "DirectoryReader does not support termVectors".to_string(),
        ))
    }

    fn num_docs(&self) -> i32 {
        self.segment_readers
            .iter()
            .map(|r| LeafReader::num_docs(r.as_ref()))
            .sum()
    }

    fn max_doc(&self) -> i32 {
        self.segment_readers
            .iter()
            .map(|r| LeafReader::max_doc(r.as_ref()))
            .sum()
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        Err(LuceneError::UnsupportedOperation(
            "DirectoryReader does not support storedFields".to_string(),
        ))
    }

    fn do_close(&self) -> Result<()> {
        if let Some(ref writer) = self.writer {
            let _ = writer.dec_ref_deleter(&self.segment_infos);
        }
        for reader in &self.segment_readers {
            let _ = reader.dec_ref();
        }
        Ok(())
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        Some(Box::new(Arc::clone(&self.cache_helper)))
    }

    fn doc_freq(&self, _term: &Term) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "DirectoryReader does not support docFreq".to_string(),
        ))
    }

    fn total_term_freq(&self, _term: &Term) -> Result<i64> {
        Err(LuceneError::UnsupportedOperation(
            "DirectoryReader does not support totalTermFreq".to_string(),
        ))
    }

    fn get_sum_doc_freq(&self, _field: &str) -> Result<i64> {
        Err(LuceneError::UnsupportedOperation(
            "DirectoryReader does not support getSumDocFreq".to_string(),
        ))
    }

    fn get_doc_count(&self, _field: &str) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "DirectoryReader does not support getDocCount".to_string(),
        ))
    }

    fn get_sum_total_term_freq(&self, _field: &str) -> Result<i64> {
        Err(LuceneError::UnsupportedOperation(
            "DirectoryReader does not support getSumTotalTermFreq".to_string(),
        ))
    }

    fn notify_reader_closed_listeners(&self) -> Result<()> {
        self.cache_helper.notify_closed_listeners()
    }

    fn build_context(
        self: Arc<Self>,
        parent: Option<Weak<dyn IndexReaderContext>>,
        ord_in_parent: i32,
        doc_base_in_parent: i32,
        leaf_ord: i32,
        leaf_doc_base: i32,
    ) -> Arc<dyn IndexReaderContext> {
        let composite: Arc<dyn CompositeReader> = self;
        build_composite_context(
            composite,
            parent,
            ord_in_parent,
            doc_base_in_parent,
            leaf_ord,
            leaf_doc_base,
        )
    }
}

impl CompositeReader for StandardDirectoryReader {
    fn get_sequential_sub_readers(&self) -> Vec<Arc<dyn IndexReader>> {
        self.segment_readers
            .iter()
            .map(|r| Arc::clone(r) as Arc<dyn IndexReader>)
            .collect()
    }
}

impl DirectoryReader for StandardDirectoryReader {
    fn directory(&self) -> Arc<dyn Directory> {
        Arc::clone(&self.directory)
    }

    fn version(&self) -> Result<i64> {
        self.ensure_open()?;
        Ok(self.segment_infos.version())
    }

    fn is_current(&self) -> Result<bool> {
        self.ensure_open()?;
        if let Some(ref writer) = self.writer {
            if !writer.is_closed() {
                return Ok(writer.nrt_is_current(&self.segment_infos));
            }
        }
        let latest = SegmentInfos::read_latest_commit(self.directory.as_ref())?;
        Ok(latest.version() == self.segment_infos.version())
    }

    fn index_commit(&self) -> Result<Box<dyn IndexCommit>> {
        self.ensure_open()?;
        // TODO: when this reader is held in an Arc, store a weak self-reference
        // so that `IndexCommit::get_reader()` can return the producing reader.
        Ok(Box::new(ReaderCommit::new(
            None,
            self.segment_infos.clone(),
            Arc::clone(&self.directory),
        )?))
    }

    fn do_open_if_changed(&self) -> Result<Option<Arc<dyn DirectoryReader>>> {
        self.ensure_open()?;
        if let Some(ref writer) = self.writer {
            return self.do_open_if_changed_from_writer(writer.as_ref(), self.apply_all_deletes);
        }
        if self.is_current()? {
            return Ok(None);
        }
        let latest = SegmentInfos::read_latest_commit(self.directory.as_ref())?;
        Self::open_from_infos(Arc::clone(&self.directory), latest, None, None).map(Some)
    }

    fn do_open_if_changed_from_commit(
        &self,
        commit: &dyn IndexCommit,
    ) -> Result<Option<Arc<dyn DirectoryReader>>> {
        self.ensure_open()?;
        if !Arc::ptr_eq(&commit.get_directory(), &self.directory) {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the specified commit does not match the specified Directory",
            )));
        }
        if let Some(ref segments_file) = self.segment_infos.segments_file_name() {
            if segments_file == &commit.get_segments_file_name() {
                return Ok(None);
            }
        }
        let infos =
            SegmentInfos::read_commit(self.directory.as_ref(), &commit.get_segments_file_name())?;
        Self::open_from_infos(Arc::clone(&self.directory), infos, None, None).map(Some)
    }

    fn do_open_if_changed_from_writer(
        &self,
        writer: &dyn IndexWriter,
        apply_all_deletes: bool,
    ) -> Result<Option<Arc<dyn DirectoryReader>>> {
        self.ensure_open()?;
        if let Some(ref my_writer) = self.writer {
            if std::ptr::eq(my_writer.as_ref(), writer)
                && apply_all_deletes == self.apply_all_deletes
            {
                if writer.nrt_is_current(&self.segment_infos) {
                    return Ok(None);
                }
                let new_reader =
                    writer.get_reader(self.apply_all_deletes, self.write_all_deletes)?;
                if new_reader.version()? == self.segment_infos.version() {
                    return Ok(None);
                }
                return Ok(Some(new_reader));
            }
        }
        // Different writer or different applyAllDeletes setting: ask the writer.
        Ok(Some(writer.get_reader(apply_all_deletes, false)?))
    }
}

// -----------------------------------------------------------------------------
// ReaderCommit
// -----------------------------------------------------------------------------

/// `IndexCommit` implementation used by `StandardDirectoryReader`.
///
/// Equivalent to the package-private
/// `org.apache.lucene.index.StandardDirectoryReader.ReaderCommit`.
struct ReaderCommit {
    segments_file_name: String,
    files: HashSet<String>,
    directory: Arc<dyn Directory>,
    generation: i64,
    user_data: HashMap<String, String>,
    segment_count: i32,
    reader: Option<Weak<StandardDirectoryReader>>,
}

impl Debug for ReaderCommit {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReaderCommit")
            .field("segments_file_name", &self.segments_file_name)
            .field("generation", &self.generation)
            .field("segment_count", &self.segment_count)
            .finish()
    }
}

impl ReaderCommit {
    /// Builds a commit point over `infos`.
    ///
    /// # Divergence: a commit with no `segments_N` name
    ///
    /// `IndexCommit::getSegmentsFileName()` is `String` in Java and may be
    /// `null`: it forwards to `SegmentInfos.getSegmentsFileName()`, which calls
    /// `IndexFileNames.fileNameFromGeneration`, and that returns `null` for
    /// generation `-1` (`IndexFileNames.java:56-59`). Rucene's
    /// [`IndexCommit::get_segments_file_name`] returns a plain `String`, so the
    /// missing name is rendered as `"segments"` — which is also the *legitimate*
    /// name of generation `0` (`fileNameFromGeneration` with `gen == 0`), making
    /// the two cases indistinguishable.
    ///
    /// This is unreachable through the two public entry points that build a
    /// `ReaderCommit` today ([`list_commits`] and
    /// [`DirectoryReader::index_commit`]): both feed it a [`SegmentInfos`] that
    /// was read from an actual `segments_N`, whose `last_generation` is
    /// therefore `>= 0`. It becomes reachable only for an uncommitted
    /// `SegmentInfos`, i.e. once a near-real-time reader can be opened from a
    /// writer that has never committed — and note that Java would not reach it
    /// even then, because its `SegmentInfos.generation`/`lastGeneration` fields
    /// default to `0` (`SegmentInfos.java:132-133`, no initialiser) where
    /// Rucene's [`SegmentInfos::new`] starts them at `-1`.
    fn new(
        reader: Option<Weak<StandardDirectoryReader>>,
        infos: SegmentInfos,
        directory: Arc<dyn Directory>,
    ) -> Result<Self> {
        let segments_file_name = infos
            .segments_file_name()
            .unwrap_or_else(|| SEGMENTS.to_string());
        let generation = infos.generation();
        let user_data = infos.user_data().clone();
        let segment_count = infos.size() as i32;
        let files = infos.files(true)?;
        Ok(Self {
            segments_file_name,
            files,
            directory,
            generation,
            user_data,
            segment_count,
            reader,
        })
    }
}

impl IndexCommit for ReaderCommit {
    fn get_segments_file_name(&self) -> String {
        self.segments_file_name.clone()
    }

    fn get_file_names(&self) -> Result<HashSet<String>> {
        Ok(self.files.clone())
    }

    fn get_directory(&self) -> Arc<dyn Directory> {
        Arc::clone(&self.directory)
    }

    fn delete(&self) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "This IndexCommit does not support deletions".to_string(),
        ))
    }

    fn is_deleted(&self) -> bool {
        false
    }

    fn get_segment_count(&self) -> i32 {
        self.segment_count
    }

    fn get_generation(&self) -> i64 {
        self.generation
    }

    fn get_user_data(&self) -> Result<HashMap<String, String>> {
        Ok(self.user_data.clone())
    }

    fn get_reader(&self) -> Option<Arc<StandardDirectoryReader>> {
        self.reader.as_ref().and_then(Weak::upgrade)
    }
}

// -----------------------------------------------------------------------------
// Cache helper
// -----------------------------------------------------------------------------

/// Reader-level cache helper for `StandardDirectoryReader`.
struct DirectoryReaderCacheHelper {
    key: CacheKey,
    listeners: Mutex<Vec<Box<dyn ClosedListener>>>,
}

impl Debug for DirectoryReaderCacheHelper {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectoryReaderCacheHelper")
            .field("key", &self.key)
            .field(
                "listener_count",
                &self.listeners.lock().map_or(0, |g| g.len()),
            )
            .finish()
    }
}

impl DirectoryReaderCacheHelper {
    fn new() -> Self {
        Self {
            key: CacheKey,
            listeners: Mutex::new(Vec::new()),
        }
    }

    fn notify_closed_listeners(&self) -> Result<()> {
        let guard = self.listeners.lock().map_err(|_| {
            LuceneError::IllegalState("reader closed listeners lock poisoned".to_string())
        })?;
        for l in guard.iter() {
            // `CacheKey` is a unit struct, so we can pass a fresh instance.
            l.on_close(CacheKey)?;
        }
        Ok(())
    }
}

impl CacheHelper for DirectoryReaderCacheHelper {
    fn get_key(&self) -> &CacheKey {
        &self.key
    }

    fn add_closed_listener(&self, listener: Box<dyn ClosedListener>) {
        if let Ok(mut listeners) = self.listeners.lock() {
            listeners.push(listener);
        }
    }
}

impl CacheHelper for Arc<DirectoryReaderCacheHelper> {
    fn get_key(&self) -> &CacheKey {
        self.as_ref().get_key()
    }

    fn add_closed_listener(&self, listener: Box<dyn ClosedListener>) {
        self.as_ref().add_closed_listener(listener);
    }
}

// -----------------------------------------------------------------------------
// Static helpers mirroring Java DirectoryReader static methods
// -----------------------------------------------------------------------------

/// Returns a `DirectoryReader` reading the index in the given `Directory`.
///
/// Equivalent to `DirectoryReader.open(Directory)`.
pub fn open(directory: Arc<dyn Directory>) -> Result<Arc<dyn DirectoryReader>> {
    let infos = SegmentInfos::read_latest_commit(directory.as_ref())?;
    StandardDirectoryReader::open_from_infos(directory, infos, None, None)
}

/// Returns a `DirectoryReader` for the given `IndexCommit`.
///
/// Equivalent to `DirectoryReader.open(IndexCommit)`.
pub fn open_with_commit(commit: &dyn IndexCommit) -> Result<Arc<dyn DirectoryReader>> {
    let directory = commit.get_directory();
    let infos = SegmentInfos::read_commit(directory.as_ref(), &commit.get_segments_file_name())?;
    StandardDirectoryReader::open_from_infos(directory, infos, None, None)
}

/// If the index has changed since `old_reader` was opened, returns a new reader;
/// otherwise returns `None`.
///
/// Equivalent to `DirectoryReader.openIfChanged(DirectoryReader)`.
pub fn open_if_changed(
    old_reader: Arc<dyn DirectoryReader>,
) -> Result<Option<Arc<dyn DirectoryReader>>> {
    old_reader.do_open_if_changed()
}

/// If the provided commit differs from what `old_reader` is searching, returns a
/// new reader; otherwise returns `None`.
///
/// Equivalent to `DirectoryReader.openIfChanged(DirectoryReader, IndexCommit)`.
pub fn open_if_changed_with_commit(
    old_reader: Arc<dyn DirectoryReader>,
    commit: &dyn IndexCommit,
) -> Result<Option<Arc<dyn DirectoryReader>>> {
    old_reader.do_open_if_changed_from_commit(commit)
}

/// If the provided writer has changes, returns a new reader covering them.
///
/// Equivalent to `DirectoryReader.openIfChanged(DirectoryReader, IndexWriter, boolean)`.
pub fn open_if_changed_with_writer(
    old_reader: Arc<dyn DirectoryReader>,
    writer: &dyn IndexWriter,
    apply_all_deletes: bool,
) -> Result<Option<Arc<dyn DirectoryReader>>> {
    old_reader.do_open_if_changed_from_writer(writer, apply_all_deletes)
}

/// Returns all commit points that exist in the directory, sorted from oldest
/// to latest.
///
/// The result is shared as `Arc<dyn IndexCommit>` so that it can be handed
/// straight to an [`IndexDeletionPolicy`](crate::index::IndexDeletionPolicy),
/// which may retain individual commit points (see
/// [`SnapshotDeletionPolicy`](crate::index::SnapshotDeletionPolicy)).
///
/// Equivalent to `DirectoryReader.listCommits(Directory)`.
///
/// # Errors
///
/// Returns an error if the directory cannot be listed or a commit cannot be
/// read.
pub fn list_commits(directory: Arc<dyn Directory>) -> Result<Vec<Arc<dyn IndexCommit>>> {
    let files = directory.list_all()?;
    let latest = SegmentInfos::read_latest_commit_with_min_version(directory.as_ref(), 0)?;
    let current_gen = latest.generation();

    let mut commits: Vec<Arc<dyn IndexCommit>> = Vec::new();
    commits.push(Arc::new(ReaderCommit::new(
        None,
        latest,
        Arc::clone(&directory),
    )?));

    for file in files {
        // Java does *not* filter `segments.gen` out here, unlike
        // `SegmentInfos.getLastCommitGeneration`: the legacy file reaches
        // `generationFromSegmentsFileName`, which rejects it with
        // `IllegalArgumentException` (`SegmentInfos.java:246-249`). That is
        // deliberate — it is how Lucene reports "this looks like a pre-4.0
        // index" instead of silently listing a partial set of commits — so
        // Rucene lets the same error surface.
        if file.starts_with(SEGMENTS) {
            let gen = SegmentInfos::generation_from_segments_file_name(&file)?;
            if gen < current_gen {
                match SegmentInfos::read_commit_with_min_version(directory.as_ref(), &file, 0) {
                    Ok(infos) => {
                        commits.push(Arc::new(ReaderCommit::new(
                            None,
                            infos,
                            Arc::clone(&directory),
                        )?));
                    }
                    Err(LuceneError::Io(err)) if err.kind() == io::ErrorKind::NotFound => {
                        // Stale directory listing; ignore this file.
                    }
                    Err(err) => return Err(err),
                }
            }
        }
    }

    // Every commit above was built from the same `directory`, so
    // `IndexCommit::compare_to` can never fail here and reduces to comparing
    // generations. Sorting by generation keeps the total order that `sort_by`
    // requires, which an `Ordering::Equal` fallback would not.
    commits.sort_by_key(|commit| commit.get_generation());

    Ok(commits)
}

/// Returns `true` if an index likely exists at the specified directory.
///
/// Equivalent to `DirectoryReader.indexExists(Directory)`.
pub fn index_exists(directory: &dyn Directory) -> Result<bool> {
    let prefix = format!("{}_", SEGMENTS);
    for file in directory.list_all()? {
        if file.starts_with(&prefix) {
            return Ok(true);
        }
    }
    Ok(false)
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::segment_info::SegmentInfoFormat;
    use crate::codecs::tests::{test_segment_info, DummyCodec};
    use crate::codecs::{register_codec, FilterCodec, Lucene99SegmentInfoFormat};
    use crate::index::index_file_names::SEGMENTS;
    use crate::index::segment_infos::OLD_SEGMENTS_GEN;
    use crate::index::{SegmentCommitInfo, SegmentInfos};
    use crate::search::{Sort, SortField, SortFieldType};
    use crate::store::{Directory, RamDirectory};
    use crate::util::string_helper::StringHelper;
    use crate::util::Version;
    use std::collections::{HashMap, HashSet};

    fn test_codec() -> Arc<dyn crate::codecs::Codec> {
        static REGISTER: std::sync::Once = std::sync::Once::new();
        let inner: Arc<dyn crate::codecs::Codec> = Arc::new(DummyCodec::new("Dummy"));
        let codec: Arc<dyn crate::codecs::Codec> = Arc::new(
            FilterCodec::new("DirectoryReaderTestCodec", Arc::clone(&inner))
                .with_segment_info_format(Lucene99SegmentInfoFormat::new()),
        );
        REGISTER.call_once(|| {
            let registered = FilterCodec::new(
                "DirectoryReaderTestCodec",
                Arc::new(DummyCodec::new("Dummy")),
            )
            .with_segment_info_format(Lucene99SegmentInfoFormat::new());
            // Ignore double-registration when multiple tests share the binary.
            let _ = register_codec("DirectoryReaderTestCodec", registered);
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
            Sort::new_fields(vec![SortField::new(
                Some("id".to_string()),
                SortFieldType::String,
            )
            .unwrap()])
            .unwrap(),
        );
        info.set_files(HashSet::from([
            crate::index::index_file_names::segment_file_name(
                name,
                "",
                crate::index::FIELD_INFO_EXTENSION,
            ),
        ]));
        write_segment_info_file(directory, &info);
        info
    }

    fn make_sci(
        directory: &dyn Directory,
        name: &str,
        max_doc: i32,
    ) -> crate::index::SegmentCommitInfo {
        let info = make_segment_info(directory, name, max_doc);
        SegmentCommitInfo::new(info, 0, 0, -1, -1, -1, StringHelper::random_id()).unwrap()
    }

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

    #[test]
    fn opens_empty_directory() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let mut sis = SegmentInfos::new(Version::LATEST.major as i32).unwrap();
        sis.commit(dir.as_ref()).unwrap();
        let reader = open(Arc::clone(&dir)).unwrap();
        assert_eq!(reader.max_doc(), 0);
        assert_eq!(reader.num_docs(), 0);
        assert!(!reader.has_deletions());
        let leaves = reader.leaves();
        assert_eq!(leaves.len(), 0);
    }

    #[test]
    fn opens_single_segment_directory() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let reader = commit_single_segment(Arc::clone(&dir));
        assert_eq!(reader.max_doc(), 42);
        assert_eq!(reader.num_docs(), 42);
        let leaves = reader.leaves();
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].doc_base(), 0);
    }

    #[test]
    fn open_if_changed_detects_new_commit() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let reader = commit_single_segment(Arc::clone(&dir));
        let first_version = reader.version().unwrap();
        assert!(reader.is_current().unwrap());

        // Write a second commit with an additional segment.
        let latest = SegmentInfos::read_latest_commit(dir.as_ref()).unwrap();
        let mut sis = latest.clone();
        sis.set_next_write_generation(latest.generation() + 1)
            .unwrap();
        sis.changed();
        let name = format!("_{}", sis.counter);
        sis.counter += 1;
        let sci = make_sci(dir.as_ref(), &name, 8);
        sis.add(sci).unwrap();
        sis.commit(dir.as_ref()).unwrap();

        assert!(!reader.is_current().unwrap());
        let new_reader = open_if_changed(reader).unwrap();
        assert!(new_reader.is_some());
        let new_reader = new_reader.unwrap();
        assert!(new_reader.version().unwrap() > first_version);
        assert_eq!(new_reader.max_doc(), 50);
        assert_eq!(Arc::clone(&new_reader).leaves().len(), 2);
        assert!(new_reader.is_current().unwrap());
    }

    #[test]
    fn open_if_changed_returns_none_when_unchanged() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let reader = commit_single_segment(Arc::clone(&dir));
        let same = open_if_changed(reader).unwrap();
        assert!(same.is_none());
    }

    #[test]
    fn open_if_changed_from_commit() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let reader = commit_single_segment(Arc::clone(&dir));

        // Read the first commit back and reopen from it: should be a no-op.
        let first_commit = reader.index_commit().unwrap();
        let same = open_if_changed_with_commit(
            Arc::clone(&reader) as Arc<dyn DirectoryReader>,
            first_commit.as_ref(),
        )
        .unwrap();
        assert!(same.is_none());

        // Write a new commit and reopen from it.
        let latest = SegmentInfos::read_latest_commit(dir.as_ref()).unwrap();
        let mut sis = latest.clone();
        sis.set_next_write_generation(latest.generation() + 1)
            .unwrap();
        sis.changed();
        let name = format!("_{}", sis.counter);
        sis.counter += 1;
        let sci = make_sci(dir.as_ref(), &name, 8);
        sis.add(sci).unwrap();
        sis.commit(dir.as_ref()).unwrap();

        let new_commit = Box::new(
            ReaderCommit::new(
                None,
                SegmentInfos::read_latest_commit(dir.as_ref()).unwrap(),
                Arc::clone(&dir),
            )
            .unwrap(),
        );
        let reopened = open_if_changed_with_commit(
            Arc::clone(&reader) as Arc<dyn DirectoryReader>,
            new_commit.as_ref(),
        )
        .unwrap();
        assert!(reopened.is_some());
        let reopened = reopened.unwrap();
        assert_eq!(reopened.max_doc(), 50);
    }

    #[test]
    fn index_commit_matches_reader() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let reader = commit_single_segment(Arc::clone(&dir));
        let commit = reader.index_commit().unwrap();
        assert_eq!(commit.get_segment_count(), 1);
        assert!(commit.get_segments_file_name().starts_with(SEGMENTS));
        assert_eq!(commit.get_generation(), 1);
        assert_eq!(
            commit.get_user_data().unwrap().get("user"),
            Some(&"data".to_string())
        );
        assert!(!commit.is_deleted());
        assert!(matches!(
            commit.delete(),
            Err(LuceneError::UnsupportedOperation(_))
        ));
    }

    #[test]
    fn list_commits_returns_sorted_commits() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let _reader = commit_single_segment(Arc::clone(&dir));

        // Write a second commit.
        let latest = SegmentInfos::read_latest_commit(dir.as_ref()).unwrap();
        let mut sis = latest.clone();
        sis.set_next_write_generation(latest.generation() + 1)
            .unwrap();
        sis.changed();
        let name = format!("_{}", sis.counter);
        sis.counter += 1;
        let sci = make_sci(dir.as_ref(), &name, 5);
        sis.add(sci).unwrap();
        sis.commit(dir.as_ref()).unwrap();

        let commits = list_commits(Arc::clone(&dir)).unwrap();
        assert_eq!(commits.len(), 2);
        assert!(commits[0].get_generation() < commits[1].get_generation());
        assert_eq!(commits[1].get_segment_count(), 2);
    }

    #[test]
    fn list_commits_rejects_a_legacy_segments_gen_file() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let _reader = commit_single_segment(Arc::clone(&dir));
        assert_eq!(list_commits(Arc::clone(&dir)).unwrap().len(), 1);

        // A pre-4.0 index leaves a `segments.gen` behind. Java's
        // `DirectoryReader.listCommits` does not filter it out
        // (`DirectoryReader.java:459-460`), so it reaches
        // `generationFromSegmentsFileName`, which rejects it
        // (`SegmentInfos.java:246-249`). Silently skipping it would list a
        // partial set of commits instead of reporting the old index.
        {
            let mut out = dir
                .create_output(OLD_SEGMENTS_GEN, &*DEFAULT_IO_CONTEXT)
                .unwrap();
            out.write_byte(0).unwrap();
            out.close().unwrap();
        }

        let err = list_commits(Arc::clone(&dir))
            .expect_err("a segments.gen file must be reported, not skipped");
        assert!(
            matches!(&err, LuceneError::IllegalArgument(msg)
                if msg.contains("segments.gen") && msg.contains("not a valid segment file name")),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn index_exists_matches_java_semantics() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        assert!(!index_exists(dir.as_ref()).unwrap());

        let mut sis = SegmentInfos::new(Version::LATEST.major as i32).unwrap();
        sis.commit(dir.as_ref()).unwrap();
        // First commit writes pending_segments_1 -> segments_1, so "segments_" exists.
        assert!(index_exists(dir.as_ref()).unwrap());

        // A second commit is still detected.
        let mut sis = SegmentInfos::read_latest_commit(dir.as_ref()).unwrap();
        sis.set_next_write_generation(2).unwrap();
        sis.changed();
        sis.commit(dir.as_ref()).unwrap();
        assert!(index_exists(dir.as_ref()).unwrap());
    }

    #[test]
    fn directory_reader_to_string_is_debuggable() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let reader = commit_single_segment(dir);
        let s = format!("{:?}", reader);
        assert!(s.contains("StandardDirectoryReader"));
    }

    // Near-real-time smoke test using a mock IndexWriter.
    struct MockIndexWriter {
        directory: Arc<dyn Directory>,
        current: Mutex<SegmentInfos>,
        closed: std::sync::atomic::AtomicBool,
    }

    impl Debug for MockIndexWriter {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("MockIndexWriter")
                .field("directory_type", &self.directory.directory_type_name())
                .field(
                    "closed",
                    &self.closed.load(std::sync::atomic::Ordering::SeqCst),
                )
                .finish()
        }
    }

    impl IndexWriter for MockIndexWriter {
        fn nrt_is_current(&self, infos: &SegmentInfos) -> bool {
            self.current.lock().unwrap().version() == infos.version()
        }

        fn get_reader(
            &self,
            _apply_all_deletes: bool,
            _write_all_deletes: bool,
        ) -> Result<Arc<dyn DirectoryReader>> {
            let infos = self.current.lock().unwrap().clone();
            StandardDirectoryReader::open_from_infos(
                Arc::clone(&self.directory),
                infos,
                None,
                Some(Arc::new(MockIndexWriter {
                    directory: Arc::clone(&self.directory),
                    current: Mutex::new(self.current.lock().unwrap().clone()),
                    closed: std::sync::atomic::AtomicBool::new(false),
                })),
            )
        }

        fn is_closed(&self) -> bool {
            self.closed.load(std::sync::atomic::Ordering::SeqCst)
        }

        fn get_directory(&self) -> Arc<dyn Directory> {
            Arc::clone(&self.directory)
        }

        fn inc_ref_deleter(&self, _infos: &SegmentInfos) -> Result<()> {
            Ok(())
        }

        fn dec_ref_deleter(&self, _infos: &SegmentInfos) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn is_current_with_mock_writer() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let mut sis = SegmentInfos::new(Version::LATEST.major as i32).unwrap();
        sis.counter = 1;
        sis.changed();
        let sci = make_sci(dir.as_ref(), "_0", 10);
        sis.add(sci).unwrap();
        sis.commit(dir.as_ref()).unwrap();

        let writer_impl = Arc::new(MockIndexWriter {
            directory: Arc::clone(&dir),
            current: Mutex::new(sis),
            closed: std::sync::atomic::AtomicBool::new(false),
        });
        let writer: Arc<dyn IndexWriter> = Arc::clone(&writer_impl) as Arc<dyn IndexWriter>;

        let reader = StandardDirectoryReader::open_from_infos(
            Arc::clone(&dir),
            SegmentInfos::read_latest_commit(dir.as_ref()).unwrap(),
            None,
            Some(writer),
        )
        .unwrap();

        assert!(reader.is_current().unwrap());

        // Mutate the writer's view of the index.
        writer_impl.current.lock().unwrap().changed();
        assert!(!reader.is_current().unwrap());

        let new_reader = open_if_changed_with_writer(
            Arc::clone(&reader) as Arc<dyn DirectoryReader>,
            writer_impl.as_ref(),
            false,
        )
        .unwrap();
        assert!(new_reader.is_some());
        let new_reader = new_reader.unwrap();
        assert!(new_reader.is_current().unwrap());
    }
}
