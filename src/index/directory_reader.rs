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

    /// Returns whether this writer's merge policy keeps a segment whose
    /// documents have all been deleted.
    ///
    /// Equivalent to the `writer.getConfig().mergePolicy.keepFullyDeletedSegment(...)`
    /// call inside `StandardDirectoryReader.open(IndexWriter, IOFunction,
    /// SegmentInfos, boolean, boolean)` (`StandardDirectoryReader.java:137`).
    /// The trait carries it because a `DirectoryReader` cannot reach the
    /// writer's configuration otherwise; the default answer is
    /// `MergePolicy.keepFullyDeletedSegment`'s own, which is `false`.
    ///
    /// # Errors
    ///
    /// Propagates whatever asking the merge policy raises.
    fn keep_fully_deleted_segment(&self) -> Result<bool> {
        Ok(false)
    }
}

// -----------------------------------------------------------------------------
// DirectoryReader
// -----------------------------------------------------------------------------

/// Reader that can read indexes in a `Directory`.
///
/// Equivalent to `org.apache.lucene.index.DirectoryReader`.
pub trait DirectoryReader: CompositeReader {
    /// Returns this reader as `&dyn Any`, for the one downcast Java expresses
    /// with `instanceof FilterDirectoryReader`.
    ///
    /// Equivalent to the `instanceof` check inside
    /// `FilterDirectoryReader.unwrap(DirectoryReader)`. Rust has no
    /// `instanceof` over a trait object, so every implementation provides this
    /// escape hatch instead.
    fn as_any(&self) -> &dyn std::any::Any;

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
        old_readers: Option<&[Arc<SegmentReader>]>,
        writer: Option<Arc<dyn IndexWriter>>,
    ) -> Result<Arc<dyn DirectoryReader>> {
        let segment_readers = Self::create_segment_readers(&infos, old_readers)?;
        Ok(Arc::new(StandardDirectoryReader::new(
            directory,
            segment_readers,
            infos,
            writer,
            false,
            false,
        )?))
    }

    /// Opens a near-real-time reader over the segments a writer has just
    /// flushed.
    ///
    /// `reader_function` decides how each segment's reader is obtained, which
    /// is how `IndexWriter.getReader` keeps track of the readers it pooled and
    /// reuses the ones a point-in-time merge already opened. Segments that hold
    /// no live document are dropped from the returned reader's `SegmentInfos`,
    /// unless the writer's merge policy asks for them to be kept.
    ///
    /// Equivalent to the package-private
    /// `StandardDirectoryReader.open(IndexWriter, IOFunction<SegmentCommitInfo,
    /// SegmentReader>, SegmentInfos, boolean, boolean)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java also hands the reader the
    /// comparator from `writer.getConfig().getLeafSorter()`
    /// (`StandardDirectoryReader.java:155`). This port's
    /// `StandardDirectoryReader` carries no leaf sorter at all, so there is
    /// nothing to pass on.
    ///
    /// # Errors
    ///
    /// Propagates whatever `reader_function`, releasing a dropped reader, or
    /// incrementing the writer's deleter references raises. Every reader
    /// already opened is released before the error escapes, as Java's `catch`
    /// does.
    pub(crate) fn open_from_writer(
        writer: Arc<dyn IndexWriter>,
        reader_function: &mut dyn FnMut(
            &crate::index::segment_info::SegmentCommitInfo,
        ) -> Result<Arc<SegmentReader>>,
        infos: &SegmentInfos,
        apply_all_deletes: bool,
        write_all_deletes: bool,
    ) -> Result<Arc<StandardDirectoryReader>> {
        // IndexWriter synchronizes externally before calling us, which ensures
        // infos will not change; so there's no need to process segments in
        // reverse order.
        let num_segments = infos.size();

        let mut readers: Vec<Arc<SegmentReader>> = Vec::with_capacity(num_segments);
        let dir = writer.get_directory();

        let mut segment_infos = infos.clone();
        let mut infos_upto = 0usize;
        // Equivalent to the `try` of Java's method: on any failure every reader
        // already opened must still be released before the error escapes.
        let collected: Result<()> = (|| {
            for i in 0..num_segments {
                // NOTE: important that we use infos not segment_infos here, so
                // that we are passing the actual instance of
                // SegmentCommitInfo in IndexWriter's segmentInfos.
                let info = infos.info(i);
                let reader = reader_function(info)?;
                if LeafReader::num_docs(reader.as_ref()) > 0
                    || writer.keep_fully_deleted_segment()?
                {
                    // Steal the ref:
                    readers.push(reader);
                    infos_upto += 1;
                } else {
                    reader.dec_ref()?;
                    segment_infos.remove_at(infos_upto);
                }
            }
            Ok(())
        })();
        if let Err(error) = collected {
            return Err(Self::dec_ref_while_suppressing_exception(error, &readers));
        }

        if let Err(error) = writer.inc_ref_deleter(&segment_infos) {
            return Err(Self::dec_ref_while_suppressing_exception(error, &readers));
        }

        match StandardDirectoryReader::new(
            dir,
            readers.clone(),
            segment_infos,
            Some(writer),
            apply_all_deletes,
            write_all_deletes,
        ) {
            Ok(reader) => Ok(Arc::new(reader)),
            Err(error) => Err(Self::dec_ref_while_suppressing_exception(error, &readers)),
        }
    }

    /// Builds one reader per segment, reusing an old reader wherever the
    /// segment has not changed.
    ///
    /// Equivalent to `StandardDirectoryReader.createSegmentReaders(SegmentInfos, List, ExecutorService)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java opens the readers on an
    /// `ExecutorService` when one is configured, newest segment first, and
    /// falls back to the calling thread otherwise. This port always takes the
    /// sequential path; the reuse decision, which is what makes a reopen cheap,
    /// is identical either way.
    fn create_segment_readers(
        infos: &SegmentInfos,
        old_readers: Option<&[Arc<SegmentReader>]>,
    ) -> Result<Vec<Arc<SegmentReader>>> {
        // The old readers go into a map so one can be looked up by segment name.
        let previous_segment_readers = Self::map_previous_readers(old_readers);
        let index_created_version = infos.index_created_version_major();
        let mut readers: Vec<Arc<SegmentReader>> = Vec::with_capacity(infos.size());
        // Equivalent to the `try`/`catch (Throwable t)` of the sequential branch
        // of `StandardDirectoryReader.createSegmentReaders`: on any failure,
        // every reader already opened must still be released before the error
        // escapes.
        let outcome: Result<()> = (|| {
            for i in 0..infos.size() {
                let commit_info = infos.info(i);
                let old_reader = Self::get_old_segment_reader(
                    old_readers,
                    previous_segment_readers
                        .get(&commit_info.info.name)
                        .copied(),
                    commit_info,
                )?;
                readers.push(Arc::new(Self::create_or_reuse_segment_reader(
                    commit_info,
                    old_reader,
                    index_created_version,
                )?));
            }
            Ok(())
        })();
        match outcome {
            Ok(()) => Ok(readers),
            Err(error) => Err(Self::dec_ref_while_suppressing_exception(error, &readers)),
        }
    }

    /// Releases every reader already opened in `readers`, folding any error
    /// releasing them into `error` instead of letting it replace it.
    ///
    /// Equivalent to the private
    /// `StandardDirectoryReader.decRefWhileSuppressingException(Throwable,
    /// SegmentReader[])`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java attaches every release failure
    /// to `error` through `Throwable.addSuppressed`, which `LuceneError` has no
    /// counterpart for; this folds their messages into `error`'s instead, so
    /// the original error is still the one returned, with the same
    /// information appended rather than attached.
    fn dec_ref_while_suppressing_exception(
        error: LuceneError,
        readers: &[Arc<SegmentReader>],
    ) -> LuceneError {
        let mut suppressed = Vec::new();
        for reader in readers {
            if let Err(release_error) = reader.dec_ref() {
                suppressed.push(release_error.to_string());
            }
        }
        if suppressed.is_empty() {
            error
        } else {
            LuceneError::Other(format!(
                "{error} (suppressed while releasing readers: {})",
                suppressed.join("; ")
            ))
        }
    }

    /// Indexes the old readers by segment name.
    ///
    /// Equivalent to `StandardDirectoryReader.mapPreviousReaders(List)`.
    fn map_previous_readers(old_readers: Option<&[Arc<SegmentReader>]>) -> HashMap<String, usize> {
        let Some(old_readers) = old_readers else {
            return HashMap::new();
        };
        old_readers
            .iter()
            .enumerate()
            .map(|(i, reader)| (reader.get_segment_name().to_string(), i))
            .collect()
    }

    /// Returns the old reader for a segment, when there is one that can be
    /// reopened.
    ///
    /// Equivalent to `StandardDirectoryReader.getOldSegmentReader(List, Integer, SegmentCommitInfo)`.
    /// The identifier check is Lucene's best-effort detection of an index that
    /// was deleted and rebuilt underneath an open reader.
    fn get_old_segment_reader<'a>(
        old_readers: Option<&'a [Arc<SegmentReader>]>,
        old_reader_index: Option<usize>,
        commit_info: &crate::index::segment_info::SegmentCommitInfo,
    ) -> Result<Option<&'a Arc<SegmentReader>>> {
        // No entry means this is a new segment, so no old reader can be reused.
        let Some((old_readers, index)) = old_readers.zip(old_reader_index) else {
            return Ok(None);
        };
        let old_reader = &old_readers[index];
        if commit_info.info.id() != old_reader.get_segment_info().info.id() {
            return Err(LuceneError::IllegalState(format!(
                "same segment {} has invalid doc count change; likely you are re-opening a reader \
                 after illegally removing index files yourself and building a new index in their \
                 place.  Use IndexWriter.deleteAll or open a new IndexWriter using OpenMode.CREATE \
                 instead",
                commit_info.info.name
            )));
        }
        Ok(Some(old_reader))
    }

    /// Reopens `old_reader` over `commit_info` when it can be reused, and opens
    /// a fresh reader when it cannot.
    ///
    /// Equivalent to `StandardDirectoryReader.createOrReuseSegmentReader(SegmentCommitInfo, SegmentReader, int)`.
    /// Reuse shares the segment's core readers — the postings, norms, points
    /// and vectors, which have not changed — and reloads only the live docs and
    /// the doc-values generations, which have.
    fn create_or_reuse_segment_reader(
        commit_info: &crate::index::segment_info::SegmentCommitInfo,
        old_reader: Option<&Arc<SegmentReader>>,
        index_created_version_major: i32,
    ) -> Result<SegmentReader> {
        let Some(old_reader) = old_reader else {
            return SegmentReader::new(
                commit_info.clone(),
                index_created_version_major,
                &*DEFAULT_IO_CONTEXT,
            );
        };
        if commit_info.info.get_use_compound_file()
            != old_reader.get_segment_info().info.get_use_compound_file()
        {
            // The segment was rewritten into (or out of) a compound file, so
            // nothing of the old reader can be shared.
            return SegmentReader::new(
                commit_info.clone(),
                index_created_version_major,
                &*DEFAULT_IO_CONTEXT,
            );
        }

        let del_gen_unchanged =
            commit_info.get_del_gen() == old_reader.get_segment_info().get_del_gen();
        let dv_gen_unchanged = commit_info.get_field_infos_gen()
            == old_reader.get_segment_info().get_field_infos_gen()
            && commit_info.get_doc_values_gen()
                == old_reader.get_segment_info().get_doc_values_gen();

        if del_gen_unchanged && dv_gen_unchanged && !old_reader.is_nrt() {
            // Nothing changed: reuse the reader's live docs as they are.
            return SegmentReader::new_shared(
                commit_info.clone(),
                old_reader,
                old_reader.get_live_docs(),
                old_reader.get_hard_live_docs(),
                LeafReader::num_docs(old_reader.as_ref()),
                false,
            );
        }

        if del_gen_unchanged && !old_reader.is_nrt() {
            // Only the doc values changed; the live docs carry over.
            return SegmentReader::new_shared(
                commit_info.clone(),
                old_reader,
                old_reader.get_live_docs(),
                old_reader.get_hard_live_docs(),
                LeafReader::num_docs(old_reader.as_ref()),
                false,
            );
        }

        // The live docs changed, or the old reader was an NRT one whose live
        // docs live in memory: both must be read from disk.
        let live_docs: Option<Box<dyn crate::util::Bits>> = if commit_info.has_deletions() {
            let codec = commit_info.info.codec().ok_or_else(|| {
                LuceneError::IllegalState(format!("segment {} has no codec", commit_info.info.name))
            })?;
            Some(codec.live_docs_format().read_live_docs(
                commit_info.info.directory.as_ref(),
                commit_info,
                &*crate::store::READONCE_IO_CONTEXT,
            )?)
        } else {
            None
        };
        let num_docs = commit_info.info.max_doc()? - commit_info.get_del_count();
        // Java hands the same `liveDocs` object in as both the live and the
        // hard live docs; the shared handle is how that sharing is expressed
        // here, so the bits are read once and not copied.
        let live_docs = live_docs.map(crate::index::segment_reader::SharedBits::new);
        SegmentReader::new_shared(
            commit_info.clone(),
            old_reader,
            live_docs
                .clone()
                .map(|bits| Box::new(bits) as Box<dyn crate::util::Bits>),
            live_docs.map(|bits| Box::new(bits) as Box<dyn crate::util::Bits>),
            num_docs,
            false,
        )
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
        let mut sub_fields = Vec::with_capacity(self.segment_readers.len());
        let mut starts = Vec::with_capacity(self.segment_readers.len());
        let mut doc_base = 0;
        for reader in &self.segment_readers {
            sub_fields.push(None);
            starts.push(doc_base);
            doc_base += LeafReader::max_doc(reader.as_ref());
        }
        Ok(Box::new(CompositeStoredFields {
            sub_readers: self.segment_readers.clone(),
            starts,
            sub_fields: Mutex::new(sub_fields),
        }))
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

/// The [`StoredFields`] a composite reader hands out.
///
/// Equivalent to the anonymous `StoredFields` that
/// `BaseCompositeReader.storedFields()` returns
/// (`BaseCompositeReader.java:171-194`): it finds the sub-reader that owns the
/// document, translates the document id into that sub-reader's own numbering by
/// subtracting the sub-reader's start, and dispatches. Like Java's, it opens
/// each sub-reader's `StoredFields` only when a document of that sub-reader is
/// first asked for, and reuses it afterwards.
///
/// Java caches the sub-`StoredFields` in a plain array captured by the
/// anonymous class, because the object it returns is documented as
/// single-threaded and is never shared. This port must still satisfy the
/// `Send + Sync` bound the [`StoredFields`] trait carries, and the trait's
/// methods take `&self`, so the cache sits behind a [`Mutex`]. Every handle is
/// otherwise the same object Java's is: a per-call view over the sub-readers.
#[derive(Debug)]
struct CompositeStoredFields {
    sub_readers: Vec<Arc<SegmentReader>>,
    /// First document id of each sub-reader, i.e. Java's `starts`.
    starts: Vec<i32>,
    sub_fields: Mutex<Vec<Option<Box<dyn StoredFields>>>>,
}

impl CompositeStoredFields {
    /// Returns the index of the sub-reader holding `doc_id`.
    ///
    /// Equivalent to `BaseCompositeReader.readerIndex(int)`, which Java
    /// implements as a binary search over `starts`
    /// (`BaseCompositeReader.java:96-104`, `ReaderUtil.subIndex`).
    fn reader_index(&self, doc_id: i32) -> Result<usize> {
        if doc_id < 0 || self.starts.is_empty() {
            return Err(LuceneError::IllegalArgument(format!(
                "docID must be >= 0 and < maxDoc; got {doc_id}"
            )));
        }
        // `starts` is ascending, so the owning sub-reader is the last one whose
        // start is <= doc_id.
        Ok(match self.starts.binary_search(&doc_id) {
            Ok(index) => index,
            Err(insertion) => insertion - 1,
        })
    }

    /// Runs `body` against the [`StoredFields`] of the sub-reader owning
    /// `doc_id`, opening it on first use, and hands it the sub-reader-local id.
    fn with_sub<T>(
        &self,
        doc_id: i32,
        body: impl FnOnce(&mut dyn StoredFields, i32) -> Result<T>,
    ) -> Result<T> {
        let index = self.reader_index(doc_id)?;
        let mut cache = self.sub_fields.lock().map_err(|_| {
            LuceneError::IllegalState("stored fields lock was poisoned".to_string())
        })?;
        if cache[index].is_none() {
            cache[index] = Some(LeafReader::stored_fields(self.sub_readers[index].as_ref())?);
        }
        let sub = cache[index]
            .as_mut()
            .expect("INVARIANT: the entry was just filled");
        body(sub.as_mut(), doc_id - self.starts[index])
    }
}

impl StoredFields for CompositeStoredFields {
    fn prefetch(&mut self, doc_id: i32) -> Result<()> {
        self.with_sub(doc_id, |sub, local| sub.prefetch(local))
    }

    fn document_with_visitor(
        &self,
        doc_id: i32,
        visitor: &mut dyn crate::codecs::stub::StoredFieldVisitor,
    ) -> Result<()> {
        self.with_sub(doc_id, |sub, local| {
            sub.document_with_visitor(local, visitor)
        })
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
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

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
        let latest = SegmentInfos::read_latest_commit(Arc::clone(&self.directory))?;
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
        if self.writer.is_some() {
            self.do_open_from_writer(None)
        } else {
            self.do_open_no_writer(None)
        }
    }

    fn do_open_if_changed_from_commit(
        &self,
        commit: &dyn IndexCommit,
    ) -> Result<Option<Arc<dyn DirectoryReader>>> {
        self.ensure_open()?;
        // If we were obtained by writer.getReader(), re-ask the writer to get a
        // new reader.
        if self.writer.is_some() {
            self.do_open_from_writer(Some(commit))
        } else {
            self.do_open_no_writer(Some(commit))
        }
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
                return self.do_open_from_writer(None);
            }
        }
        // Different writer or different applyAllDeletes setting: ask the writer.
        Ok(Some(writer.get_reader(apply_all_deletes, false)?))
    }
}

impl StandardDirectoryReader {
    /// Reopens over the writer that produced this reader, when one did.
    ///
    /// Equivalent to the private
    /// `StandardDirectoryReader.doOpenFromWriter(IndexCommit, ExecutorService)`
    /// without the executor (see [`Self::create_segment_readers`]'s divergence
    /// note).
    ///
    /// # Panics
    ///
    /// Panics if this reader was not opened from a writer; every caller in
    /// this crate checks `self.writer.is_some()` first, as Java's callers rely
    /// on the same invariant through `writer != null`.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`Self::do_open_from_commit`] or the writer's
    /// `get_reader` raises.
    fn do_open_from_writer(
        &self,
        commit: Option<&dyn IndexCommit>,
    ) -> Result<Option<Arc<dyn DirectoryReader>>> {
        if commit.is_some() {
            return self.do_open_from_commit(commit);
        }
        let writer = self
            .writer
            .as_ref()
            .expect("INVARIANT: do_open_from_writer is only called when this reader has a writer");
        if writer.nrt_is_current(&self.segment_infos) {
            return Ok(None);
        }
        let reader = writer.get_reader(self.apply_all_deletes, self.write_all_deletes)?;
        // If in fact no changes took place, return null.
        if reader.version()? == self.segment_infos.version() {
            reader.dec_ref()?;
            return Ok(None);
        }
        Ok(Some(reader))
    }

    /// Reopens without a writer, from `commit` or from the directory's latest
    /// commit.
    ///
    /// Equivalent to the private
    /// `StandardDirectoryReader.doOpenNoWriter(IndexCommit, ExecutorService)`
    /// without the executor.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] when `commit` names a different directory,
    /// and otherwise propagates whatever [`Self::do_open_from_commit`] raises.
    fn do_open_no_writer(
        &self,
        commit: Option<&dyn IndexCommit>,
    ) -> Result<Option<Arc<dyn DirectoryReader>>> {
        match commit {
            None => {
                if self.is_current()? {
                    return Ok(None);
                }
            }
            Some(commit) => {
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
            }
        }
        self.do_open_from_commit(commit)
    }

    /// Reopens from `commit`'s segments file, or the directory's latest one
    /// when `commit` is `None`.
    ///
    /// Equivalent to the private
    /// `StandardDirectoryReader.doOpenFromCommit(IndexCommit, ExecutorService)`
    /// without the executor: Java's anonymous `SegmentInfos.FindSegmentsFile`
    /// plays the same "specific commit, or whatever is latest" role that
    /// [`SegmentInfos::read_commit`]/[`SegmentInfos::read_latest_commit`]
    /// already give this port directly.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the segment infos or
    /// [`Self::open_from_infos`] raises.
    fn do_open_from_commit(
        &self,
        commit: Option<&dyn IndexCommit>,
    ) -> Result<Option<Arc<dyn DirectoryReader>>> {
        let infos = match commit {
            Some(commit) => SegmentInfos::read_commit(
                Arc::clone(&self.directory),
                &commit.get_segments_file_name(),
            )?,
            None => SegmentInfos::read_latest_commit(Arc::clone(&self.directory))?,
        };
        Self::open_from_infos(Arc::clone(&self.directory), infos, None, None).map(Some)
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
    let infos = SegmentInfos::read_latest_commit(Arc::clone(&directory))?;
    StandardDirectoryReader::open_from_infos(directory, infos, None, None)
}

/// Returns a `DirectoryReader` for the given `IndexCommit`.
///
/// Equivalent to `DirectoryReader.open(IndexCommit)`.
pub fn open_with_commit(commit: &dyn IndexCommit) -> Result<Arc<dyn DirectoryReader>> {
    let directory = commit.get_directory();
    let infos =
        SegmentInfos::read_commit(Arc::clone(&directory), &commit.get_segments_file_name())?;
    StandardDirectoryReader::open_from_infos(directory, infos, None, None)
}

/// Returns a near-real-time reader covering every change `writer` has made so
/// far, including changes that have not been committed.
///
/// Equivalent to `DirectoryReader.open(IndexWriter)`, which applies all deletes
/// but does not write them.
///
/// # Errors
///
/// Propagates whatever the writer's `get_reader` raises.
pub fn open_with_writer(writer: &dyn IndexWriter) -> Result<Arc<dyn DirectoryReader>> {
    open_with_writer_flags(writer, true, false)
}

/// Returns a near-real-time reader covering every change `writer` has made so
/// far, controlling whether buffered deletes are applied and written.
///
/// Setting `apply_all_deletes` to `false` makes the reopen cheaper at the cost
/// of returning documents that have already been deleted;
/// `write_all_deletes` additionally forces the applied deletes and doc-values
/// updates through to disk, which is what a replica reading the same directory
/// needs.
///
/// Equivalent to `DirectoryReader.open(IndexWriter, boolean, boolean)`.
///
/// # Errors
///
/// Propagates whatever the writer's `get_reader` raises, including the
/// `IllegalArgument` it reports when `write_all_deletes` is set without
/// `apply_all_deletes`.
pub fn open_with_writer_flags(
    writer: &dyn IndexWriter,
    apply_all_deletes: bool,
    write_all_deletes: bool,
) -> Result<Arc<dyn DirectoryReader>> {
    writer.get_reader(apply_all_deletes, write_all_deletes)
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
    let latest = SegmentInfos::read_latest_commit_with_min_version(Arc::clone(&directory), 0)?;
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
                match SegmentInfos::read_commit_with_min_version(Arc::clone(&directory), &file, 0) {
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
        let latest = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
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
        let latest = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
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
                SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap(),
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
        let latest = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
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
        let mut sis = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
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
            SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap(),
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
