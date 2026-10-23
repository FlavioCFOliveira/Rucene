//! `SegmentMerger` ported from `org.apache.lucene.index`.
//!
//! Merges several segments into one by driving each codec's consumer over a
//! [`MergeState`].

use std::sync::{Arc, Mutex};

use crate::codecs::postings::{PostingsNormsProducer, PostingsNumericDocValues};
use crate::codecs::state::{OwnedSegmentWriteState, SegmentReadState, SegmentWriteState};
use crate::codecs::stub::{BufferedUpdates, FieldInfo};
use crate::codecs::Codec;
use crate::error::{LuceneError, Result};
use crate::index::codec_reader::CodecReader;
use crate::index::merge::{DocMap, MergeState};
use crate::index::segment_info::SegmentInfo;
use crate::index::FieldInfos;
use crate::search::Sort;
use crate::store::{Context, Directory, IOContext};
use crate::util::{InfoStream, Version};

/// Component name used in this merger's info-stream messages, as Java does.
const SM: &str = "SM";

/// Adapts a [`crate::codecs::norms::NormsProducer`] to the postings module's own
/// [`PostingsNormsProducer`] trait.
///
/// **Divergence from Lucene 10.5.0.** Java has a single `NormsProducer`
/// interface that both the norms format and the postings merge use. This port
/// grew two: [`crate::codecs::norms::NormsProducer`], the faithful port, whose
/// `get_norms` returns the iterator-shaped
/// [`NumericDocValues`](crate::index::NumericDocValues) of the `index` module;
/// and [`PostingsNormsProducer`], which corresponds to no Lucene type and whose
/// `get_norms` returns the random-access [`PostingsNumericDocValues`]. Merging
/// postings needs the second while the codec supplies the first, so this adapter
/// materialises each field's norms into a dense vector once and serves random
/// access from it. Unifying the two traits is the faithful fix and is tracked
/// separately.
struct PostingsNormsAdapter {
    inner: Mutex<Box<dyn crate::codecs::norms::NormsProducer>>,
    max_doc: i32,
}

impl PostingsNormsAdapter {
    fn new(inner: Box<dyn crate::codecs::norms::NormsProducer>, max_doc: i32) -> Self {
        Self {
            inner: Mutex::new(inner),
            max_doc,
        }
    }
}

/// A dense norms vector, indexed by document number.
struct DenseNorms {
    values: Vec<i64>,
}

impl PostingsNumericDocValues for DenseNorms {
    fn get(&self, doc_id: i32) -> Result<i64> {
        self.values.get(doc_id as usize).copied().ok_or_else(|| {
            LuceneError::IllegalArgument(format!("docID {doc_id} is out of range for norms"))
        })
    }
}

impl PostingsNormsProducer for PostingsNormsAdapter {
    fn get_norms(&self, field_info: &FieldInfo) -> Result<Box<dyn PostingsNumericDocValues>> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| LuceneError::IllegalState("norms producer lock poisoned".to_string()))?;
        let mut source = guard.get_norms(field_info)?;
        let mut values = vec![0i64; self.max_doc.max(0) as usize];
        loop {
            let doc_id = source.next_doc()?;
            if doc_id == crate::search::NO_MORE_DOCS {
                break;
            }
            if doc_id >= 0 && (doc_id as usize) < values.len() {
                values[doc_id as usize] = source.long_value()?;
            }
        }
        Ok(Box::new(DenseNorms { values }))
    }
}

/// A norms producer for a segment that has no norms.
struct NoNorms;

impl PostingsNormsProducer for NoNorms {
    fn get_norms(&self, _field_info: &FieldInfo) -> Result<Box<dyn PostingsNumericDocValues>> {
        Ok(Box::new(DenseNorms { values: Vec::new() }))
    }
}

/// Merges several segments into one.
///
/// Equivalent to `org.apache.lucene.index.SegmentMerger`.
///
/// Java shares one `SegmentInfo` object between the merger, its `MergeState`
/// and the `SegmentWriteState` every codec writer receives, and the merge
/// mutates it: `minVersion` here (`SegmentMerger.java:89`), the stored-fields
/// mode attribute in `Lucene90StoredFieldsFormat.fieldsWriter`
/// (`Lucene90StoredFieldsFormat.java:142`). Here the merge state owns that one
/// `SegmentInfo` and every write state borrows it, so every phase writes into,
/// and reads from, the same object; [`into_segment_info`](Self::into_segment_info)
/// hands it back to the caller.
///
/// **Divergence from Lucene 10.5.0.** Two things differ, both confined to this
/// type:
///
/// * Java's constructor also takes an intra-merge `Executor` and the
///   `OneMerge`, storing them on the `MergeState` so a codec can parallelise a
///   merge and check for an abort. This port's `MergeState` carries neither, so
///   the merge runs on the calling thread; the sequence of consumer calls and
///   the files written are unchanged.
/// * The vectors writer receives an [`OwnedSegmentWriteState`], whose
///   `SegmentInfo` is a copy taken when the vectors phase starts, because
///   [`KnnVectorsFormat::fields_writer`](crate::codecs::KnnVectorsFormat::fields_writer)
///   takes an owned state: its writer outlives the call that creates it. No
///   vectors writer mutates the `SegmentInfo` — Lucene's only read `maxDoc` —
///   so nothing that writer could do is lost with the copy.
pub struct SegmentMerger {
    codec: Arc<dyn Codec>,
    /// The info stream every phase reports to.
    info_stream: Arc<dyn InfoStream>,
    /// The directory the merged segment is written into.
    ///
    /// Java's `directory`, which is not the `SegmentInfo`'s own directory.
    directory: Arc<dyn Directory>,
    /// The context every merged file is written with.
    context: Arc<dyn IOContext>,
    /// What Java's `SegmentWriteState` receives as `segUpdates`: nothing.
    seg_updates: BufferedUpdates,
    /// The state every consumer merges from; it owns the merged segment's
    /// `SegmentInfo` and merged `FieldInfos`, which every write state borrows.
    pub merge_state: MergeState,
    /// The thread that built [`Self::merge_state`], which is the only thread
    /// allowed to reach it through [`Self::merge_state`](Self::merge_state).
    ///
    /// Equivalent to `SegmentMerger.mergeStateCreationThread`.
    merge_state_creation_thread: std::thread::ThreadId,
}

impl SegmentMerger {
    /// Builds a merger over `readers`.
    ///
    /// Equivalent to the `SegmentMerger` constructor
    /// (`SegmentMerger.java:55-96`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `context` is not a merge
    /// context, as Java does, and whatever building the merge state raises.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        readers: &[Arc<dyn CodecReader>],
        segment_info: SegmentInfo,
        merge_field_infos: FieldInfos,
        doc_maps: Vec<DocMap>,
        needs_index_sort: bool,
        codec: Arc<dyn Codec>,
        directory: Arc<dyn Directory>,
        context: Arc<dyn IOContext>,
        info_stream: Arc<dyn InfoStream>,
    ) -> Result<Self> {
        // `SegmentMerger.java:65-68`.
        if context.context() != Context::Merge {
            let got = match context.context() {
                Context::Merge => "MERGE",
                Context::Flush => "FLUSH",
                Context::Default => "DEFAULT",
            };
            return Err(LuceneError::IllegalArgument(format!(
                "IOContext.context should be MERGE; got: {got}"
            )));
        }
        let mut merge_state = MergeState::from_readers(
            readers,
            doc_maps,
            Some(segment_info),
            merge_field_infos,
            needs_index_sort,
            Arc::clone(&info_stream),
        )?;

        // `SegmentMerger.java:76-89`: the merged segment's minimum version is
        // the oldest of its sources', or unknown as soon as one source does
        // not know its own.
        let mut min_version = Some(Version::LATEST);
        for reader in readers {
            let Some(leaf_min_version) = reader.get_meta_data().min_version() else {
                min_version = None;
                break;
            };
            if let Some(current) = min_version {
                if current.on_or_after(&leaf_min_version) {
                    min_version = Some(leaf_min_version);
                }
            }
        }
        let segment_info = merge_state
            .segment_info
            .as_mut()
            .expect("the merge state was built with the merged segment's info");
        debug_assert!(
            segment_info.min_version().is_none(),
            "The min version should be set by SegmentMerger for merged segments"
        );
        segment_info.set_min_version(min_version);
        // `SegmentMerger.java:90-95`. Java tests `getIndexSort() != null`; this
        // crate spells the null sort `Sort::new()`, as `SegmentInfo` declares
        // (`SegmentInfo::to_string_verbose`).
        if merge_state.info_stream.is_enabled("SM") && *segment_info.index_sort() != Sort::new() {
            merge_state.info_stream.message(
                "SM",
                &format!("index sort during merge: {}", segment_info.index_sort()),
            );
        }

        Ok(Self {
            codec,
            info_stream,
            directory,
            context,
            seg_updates: BufferedUpdates,
            merge_state,
            merge_state_creation_thread: std::thread::current().id(),
        })
    }

    /// Returns the merged segment's `SegmentInfo`: the one object the merge
    /// state holds and every write state borrows.
    ///
    /// Equivalent to reading `mergeState.segmentInfo`.
    fn segment_info(&self) -> &SegmentInfo {
        self.merge_state
            .segment_info
            .as_ref()
            .expect("the merge state was built with the merged segment's info")
    }

    /// Returns the write state every codec writer of this merge receives.
    ///
    /// Equivalent to the `SegmentWriteState` `SegmentMerger.merge()` builds
    /// over `mergeState.segmentInfo` and `mergeState.mergeFieldInfos`: it
    /// borrows both from the merge state, so a codec writer mutating the
    /// `SegmentInfo` mutates the merged segment's own.
    fn segment_write_state(&self) -> SegmentWriteState<'_> {
        SegmentWriteState::new(
            &*self.info_stream,
            &*self.directory,
            self.segment_info(),
            &self.merge_state.merge_field_infos,
            &self.seg_updates,
            &*self.context,
        )
    }

    /// Returns the state every consumer merges from, checking that the caller
    /// is the thread that created it.
    ///
    /// Equivalent to the private `SegmentMerger.mergeState()`, which guards the
    /// field with `assert Thread.currentThread() == mergeStateCreationThread`:
    /// the merge state is not safe to reach from an intra-merge worker, which
    /// must be handed the pieces it needs instead. The check is a
    /// `debug_assert!` here, as Java's is an `assert`.
    fn merge_state(&self) -> &MergeState {
        debug_assert_eq!(
            std::thread::current().id(),
            self.merge_state_creation_thread,
            "the merge state may only be read from the thread that created it"
        );
        &self.merge_state
    }

    /// Returns whether the merge is worth running: Java skips a merge whose
    /// output would hold no documents.
    ///
    /// Equivalent to `SegmentMerger.shouldMerge()`.
    pub fn should_merge(&self) -> Result<bool> {
        Ok(self.segment_info().max_doc()? > 0)
    }

    /// Runs the merge, writing every kind of data the merged field infos say is
    /// present.
    ///
    /// Equivalent to `SegmentMerger.merge()`. The phase order is Lucene's:
    /// norms, postings, doc values, points, vectors, then the field infos.
    pub fn merge(&mut self) -> Result<()> {
        if !self.should_merge()? {
            return Err(LuceneError::IllegalState(
                "Merge would result in 0 document segment".to_string(),
            ));
        }

        let num_merged = self.merge_with_logging_count(Self::merge_fields, "stored fields")?;
        debug_assert_eq!(
            num_merged,
            self.segment_info().max_doc()?,
            "numMerged does not match the merged segment's maxDoc"
        );

        let field_infos = &self.merge_state.merge_field_infos;
        let has_norms = field_infos.has_norms();
        let has_doc_values = field_infos.has_doc_values();
        let has_points = field_infos.has_point_values();
        let has_vectors = field_infos.has_vector_values();
        let has_term_vectors = field_infos.has_term_vectors();

        if has_norms {
            self.merge_with_logging(Self::merge_norms, "norms", num_merged)?;
        }

        // Lucene merges the postings unconditionally: the terms writer must run
        // even for a segment with no indexed field, so that the empty terms
        // files exist and the reader finds them.
        let has_norms_for_terms = has_norms;
        self.merge_with_logging(
            move |merger| merger.merge_terms(has_norms_for_terms),
            "postings",
            num_merged,
        )?;

        if has_doc_values {
            self.merge_with_logging(Self::merge_doc_values, "doc values", num_merged)?;
        }

        if has_points {
            self.merge_with_logging(Self::merge_points, "points", num_merged)?;
        }

        if has_vectors {
            self.merge_with_logging(Self::merge_vector_values, "numeric vectors", num_merged)?;
        }

        if has_term_vectors {
            self.merge_with_logging_count(Self::merge_term_vectors, "term vectors")?;
        }

        // Write the merged infos.
        self.merge_with_logging(
            |merger| merger.merge_field_infos(),
            "field infos",
            num_merged,
        )
    }

    /// Copies the stored fields of every document into the merged segment.
    ///
    /// Equivalent to `SegmentMerger.mergeFields()`. Returns how many documents
    /// were written.
    fn merge_fields(&mut self) -> Result<i32> {
        let mut writer = self.codec.stored_fields_format().fields_writer(
            &*self.directory,
            self.segment_info(),
            &*self.context,
        )?;
        let merged = writer.merge(self.merge_state())?;
        writer.close()?;
        Ok(merged)
    }

    /// Copies the term vectors of every document into the merged segment.
    ///
    /// Equivalent to `SegmentMerger.mergeTermVectors()`.
    fn merge_term_vectors(&mut self) -> Result<i32> {
        let mut writer = self.codec.term_vectors_format().vectors_writer(
            &*self.directory,
            self.segment_info(),
            &*self.context,
        )?;
        let merged = writer.merge(self.merge_state())?;
        debug_assert_eq!(merged, self.segment_info().max_doc()?);
        writer.close()?;
        Ok(merged)
    }

    /// Merges the norms.
    ///
    /// Equivalent to `SegmentMerger.mergeNorms(SegmentWriteState, SegmentReadState)`.
    fn merge_norms(&mut self) -> Result<()> {
        let state = self.segment_write_state();
        let mut consumer = self.codec.norms_format().norms_consumer(&state)?;
        consumer.merge(self.merge_state())?;
        consumer.close()
    }

    /// Merges the doc values.
    ///
    /// Equivalent to `SegmentMerger.mergeDocValues(SegmentWriteState, SegmentReadState)`.
    fn merge_doc_values(&mut self) -> Result<()> {
        let state = self.segment_write_state();
        let mut consumer = self.codec.doc_values_format().fields_consumer(&state)?;
        consumer.merge(self.merge_state())?;
        consumer.close()
    }

    /// Merges the points.
    ///
    /// Equivalent to `SegmentMerger.mergePoints(SegmentWriteState, SegmentReadState)`.
    fn merge_points(&mut self) -> Result<()> {
        let state = self.segment_write_state();
        let mut writer = self.codec.points_format().fields_writer(&state)?;
        writer.merge(self.merge_state())?;
        writer.close()
    }

    /// Merges the KNN vectors.
    ///
    /// Equivalent to `SegmentMerger.mergeVectorValues(SegmentWriteState, SegmentReadState)`.
    ///
    /// The vectors writer takes an owned state, holding a copy of the merged
    /// segment's `SegmentInfo` taken here; see the type's documentation.
    fn merge_vector_values(&mut self) -> Result<()> {
        let state = OwnedSegmentWriteState::new(
            Arc::clone(&self.info_stream),
            Arc::clone(&self.directory),
            self.segment_info().clone(),
            self.merge_state.merge_field_infos.clone(),
            self.seg_updates.clone(),
            Arc::clone(&self.context),
        );
        let mut writer = self.codec.knn_vectors_format().fields_writer(&state)?;
        writer.merge(self.merge_state())?;
        writer.close()
    }

    /// Runs one merge phase and reports how long it took.
    ///
    /// Equivalent to `SegmentMerger.mergeWithLogging(VoidMerger, SegmentWriteState, SegmentReadState, String, int)`.
    fn merge_with_logging<F>(&mut self, merger: F, format_name: &str, num_merged: i32) -> Result<()>
    where
        F: FnOnce(&mut Self) -> Result<()>,
    {
        let start = std::time::Instant::now();
        merger(self)?;
        if self.info_stream.is_enabled(SM) {
            self.info_stream.message(
                SM,
                &format!(
                    "{} ms to merge {format_name} [{num_merged} docs]",
                    start.elapsed().as_millis()
                ),
            );
        }
        Ok(())
    }

    /// Runs one merge phase that reports a document count, and logs it.
    ///
    /// Equivalent to `SegmentMerger.mergeWithLogging(Merger, String)`.
    fn merge_with_logging_count<F>(&mut self, merger: F, format_name: &str) -> Result<i32>
    where
        F: FnOnce(&mut Self) -> Result<i32>,
    {
        let start = std::time::Instant::now();
        let num_merged = merger(self)?;
        if self.info_stream.is_enabled(SM) {
            self.info_stream.message(
                SM,
                &format!(
                    "{} ms to merge {format_name} [{num_merged} docs]",
                    start.elapsed().as_millis()
                ),
            );
        }
        Ok(num_merged)
    }

    /// Merges the postings, supplying the norms the terms writer needs.
    fn merge_terms(&mut self, has_norms: bool) -> Result<()> {
        let max_doc = self.segment_info().max_doc()?;
        let state = self.segment_write_state();

        let norms: Box<dyn PostingsNormsProducer> = if has_norms {
            let read_state = SegmentReadState::new(
                &*self.directory,
                self.segment_info(),
                &self.merge_state.merge_field_infos,
                &*self.context,
            );
            let producer = self.codec.norms_format().norms_producer(&read_state)?;
            Box::new(PostingsNormsAdapter::new(producer, max_doc))
        } else {
            Box::new(NoNorms)
        };

        let mut consumer = self.codec.postings_format().fields_consumer(&state)?;
        consumer.merge(self.merge_state(), norms.as_ref())?;
        consumer.close()?;
        Ok(())
    }

    /// Consumes the merger and returns the merged segment's info, carrying
    /// every change the merge made to it.
    ///
    /// Java's caller keeps the `SegmentInfo` object it handed to the merger and
    /// sees those changes in place; the merger owns it here, so this is how the
    /// caller gets it back.
    pub fn into_segment_info(self) -> SegmentInfo {
        self.merge_state
            .segment_info
            .expect("the merge state was built with the merged segment's info")
    }

    /// Writes the merged field infos.
    ///
    /// Equivalent to `SegmentMerger.mergeFieldInfos`.
    fn merge_field_infos(&mut self) -> Result<()> {
        self.codec.field_infos_format().write(
            &*self.directory,
            self.segment_info(),
            "",
            &self.merge_state.merge_field_infos,
            &*self.context,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{
        Document, IntPoint, KnnFloatVectorField, NumericDocValuesField, Store, StoredField,
        StringField,
    };
    use crate::index::index_writer::IndexWriter;
    use crate::index::index_writer_config::IndexWriterConfig;
    use crate::index::leaf_reader::LeafReader;
    use crate::index::segment_infos::SegmentInfos;
    use crate::index::segment_reader::SegmentReader;
    use crate::index::{FieldInfosBuilder, FieldNumbers, SegmentCommitInfo};
    use crate::search::NO_MORE_DOCS;
    use crate::store::{RamDirectory, DEFAULT_IO_CONTEXT};

    /// Writes `segments` committed segments of one stored-only document each
    /// and returns the directory with their commit infos.
    fn stored_only_segments(segments: usize) -> (Arc<dyn Directory>, Vec<SegmentCommitInfo>) {
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        for i in 0..segments {
            let mut doc = Document::new();
            doc.add(Box::new(
                StoredField::new_string("id", format!("doc-{i}")).unwrap(),
            ));
            writer.add_document(&doc).unwrap();
            writer.commit().unwrap();
        }
        writer.close().unwrap();
        let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
        assert_eq!(infos.size(), segments);
        (dir, infos.iter().cloned().collect())
    }

    /// Builds the merger of `readers` into a segment `_m` of `target`, with a
    /// merge context, no index sort and no info stream.
    fn merger_over(readers: &[Arc<dyn CodecReader>], target: &Arc<dyn Directory>) -> SegmentMerger {
        let context = Arc::from(crate::store::merge_io_context(
            crate::store::MergeInfo::new(readers.iter().map(|r| r.num_docs()).sum(), -1, false, -1),
        ));
        merger_with(
            readers,
            target,
            context,
            Sort::new(),
            Arc::new(crate::util::NoOutputInfoStream),
        )
        .unwrap()
    }

    /// Builds the merger of `readers` into a segment `_m` of `target` sorted
    /// by `index_sort`, writing with `context` and reporting to `info_stream`.
    fn merger_with(
        readers: &[Arc<dyn CodecReader>],
        target: &Arc<dyn Directory>,
        context: Arc<dyn IOContext>,
        index_sort: Sort,
        info_stream: Arc<dyn InfoStream>,
    ) -> Result<SegmentMerger> {
        let mut builder = FieldInfosBuilder::new(Arc::new(FieldNumbers::new(None, None).unwrap()));
        for reader in readers {
            for field in reader.get_field_infos().iter() {
                builder.add(field).unwrap();
            }
        }
        let max_docs: Vec<i32> = readers.iter().map(|r| r.max_doc()).collect();
        let live_docs: Vec<_> = readers.iter().map(|r| r.get_live_docs()).collect();
        let num_docs: Vec<i32> = readers.iter().map(|r| r.num_docs()).collect();
        let codec = crate::codecs::for_name("Lucene104").unwrap();
        let segment_info = SegmentInfo::new(
            Arc::clone(target),
            Version::LATEST,
            None,
            "_m".to_string(),
            num_docs.iter().sum(),
            false,
            false,
            Arc::clone(&codec),
            Default::default(),
            [3u8; 16],
            Default::default(),
            index_sort,
        )
        .unwrap();
        SegmentMerger::new(
            readers,
            segment_info,
            builder.finish().unwrap(),
            MergeState::build_doc_maps(&live_docs, &max_docs, &num_docs),
            false,
            codec,
            Arc::clone(target),
            context,
            info_stream,
        )
    }

    /// Records the messages of the components it is enabled for.
    #[derive(Debug, Default)]
    struct RecordingInfoStream {
        messages: Mutex<Vec<(String, String)>>,
    }

    impl InfoStream for RecordingInfoStream {
        fn java_class_name(&self) -> &'static str {
            "org.apache.lucene.tests.util.RecordingInfoStream"
        }

        fn message(&self, component: &str, message: &str) {
            self.messages
                .lock()
                .unwrap()
                .push((component.to_string(), message.to_string()));
        }

        fn is_enabled(&self, component: &str) -> bool {
            component == "SM"
        }

        fn close(&self) {}
    }

    /// Opens a reader over `info` after recording `min_version` on it, as if
    /// the segment had been written, or last merged, by that version.
    fn reader_with_min_version(
        info: &SegmentCommitInfo,
        min_version: Option<Version>,
        created_version_major: i32,
    ) -> Arc<dyn CodecReader> {
        let mut info = info.clone();
        info.info.set_min_version(min_version);
        Arc::new(SegmentReader::new(info, created_version_major, &*DEFAULT_IO_CONTEXT).unwrap())
    }

    /// A merger refuses any context but a merge one, with Java's message
    /// (`SegmentMerger.java:65-68`); the merge context is accepted.
    #[test]
    fn a_merger_needs_a_merge_context() {
        let (_dir, infos) = stored_only_segments(2);
        let readers = vec![
            reader_with_min_version(&infos[0], Some(Version::LUCENE_10_5_0), 10),
            reader_with_min_version(&infos[1], Some(Version::LUCENE_10_5_0), 10),
        ];
        let target: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let no_output = || Arc::new(crate::util::NoOutputInfoStream) as Arc<dyn InfoStream>;
        let refused = |context: Arc<dyn IOContext>| match merger_with(
            &readers,
            &target,
            context,
            Sort::new(),
            no_output(),
        ) {
            Err(LuceneError::IllegalArgument(message)) => message,
            other => panic!("expected an IllegalArgument, got {:?}", other.map(|_| ())),
        };
        assert_eq!(
            refused(Arc::new(
                crate::store::DefaultIOContext::new(Vec::new()).unwrap()
            )),
            "IOContext.context should be MERGE; got: DEFAULT"
        );
        assert_eq!(
            refused(Arc::from(crate::store::flush_io_context(
                crate::store::FlushInfo::new(2, 0)
            ))),
            "IOContext.context should be MERGE; got: FLUSH"
        );
        let merge_context = Arc::from(crate::store::merge_io_context(
            crate::store::MergeInfo::new(2, -1, false, -1),
        ));
        assert!(merger_with(&readers, &target, merge_context, Sort::new(), no_output()).is_ok());
    }

    /// A merge into a sorted segment reports the sort on the `SM` component
    /// with Java's text (`SegmentMerger.java:90-95`); an unsorted one reports
    /// nothing there.
    #[test]
    fn a_sorted_merge_reports_its_index_sort() {
        let (_dir, infos) = stored_only_segments(2);
        let readers = vec![
            reader_with_min_version(&infos[0], Some(Version::LUCENE_10_5_0), 10),
            reader_with_min_version(&infos[1], Some(Version::LUCENE_10_5_0), 10),
        ];
        let target: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let messages = |index_sort: Sort| {
            let stream = Arc::new(RecordingInfoStream::default());
            let context = Arc::from(crate::store::merge_io_context(
                crate::store::MergeInfo::new(2, -1, false, -1),
            ));
            merger_with(
                &readers,
                &target,
                context,
                index_sort,
                Arc::clone(&stream) as Arc<dyn InfoStream>,
            )
            .unwrap();
            let messages = stream.messages.lock().unwrap().clone();
            messages
        };
        let sorted = Sort::new_fields(vec![crate::search::SortField::new(
            Some("id".to_string()),
            crate::search::SortFieldType::String,
        )
        .unwrap()])
        .unwrap();
        assert_eq!(
            messages(sorted),
            vec![(
                "SM".to_string(),
                "index sort during merge: <string: \"id\">".to_string()
            )]
        );
        assert_eq!(
            messages(Sort::new()),
            Vec::new(),
            "no index sort, no message"
        );
    }

    /// The merged segment records the oldest `minVersion` of its sources, and
    /// the merger leaves that value on both the merge state and the segment it
    /// returns (`SegmentMerger.java:76-89`).
    #[test]
    fn the_merged_segment_records_the_oldest_source_min_version() {
        let (_dir, infos) = stored_only_segments(3);
        let readers = vec![
            reader_with_min_version(&infos[0], Some(Version::LUCENE_10_4_0), 10),
            reader_with_min_version(&infos[1], Some(Version::LUCENE_9_12_1), 10),
            reader_with_min_version(&infos[2], Some(Version::LUCENE_10_5_0), 10),
        ];
        let target: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let mut merger = merger_over(&readers, &target);
        assert_eq!(
            merger
                .merge_state
                .segment_info
                .as_ref()
                .unwrap()
                .min_version(),
            Some(Version::LUCENE_9_12_1)
        );
        merger.merge().unwrap();
        assert_eq!(
            merger.into_segment_info().min_version(),
            Some(Version::LUCENE_9_12_1),
            "the oldest of 10.4.0, 9.12.1 and 10.5.0"
        );
    }

    /// The merge state and every write state share one `SegmentInfo`: what a
    /// codec writer records on it during the merge is on the merge state's,
    /// and on the one the merger hands back.
    ///
    /// Java passes `mergeState.segmentInfo` to the stored-fields format, which
    /// records its mode there (`SegmentMerger.java:247-249`,
    /// `Lucene90StoredFieldsFormat.java:142`).
    #[test]
    fn the_merge_state_and_the_codec_writers_share_one_segment_info() {
        let (_dir, infos) = stored_only_segments(2);
        let readers = vec![
            reader_with_min_version(&infos[0], Some(Version::LATEST), 10),
            reader_with_min_version(&infos[1], Some(Version::LATEST), 10),
        ];
        let target: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let mut merger = merger_over(&readers, &target);
        let mode_key = crate::codecs::lucene90::stored_fields::Lucene90StoredFieldsFormat::MODE_KEY;
        assert_eq!(
            merger
                .merge_state
                .segment_info
                .as_ref()
                .unwrap()
                .get_attribute(mode_key),
            None,
            "nothing is recorded before the merge runs"
        );
        merger.merge().unwrap();
        assert_eq!(
            merger
                .merge_state
                .segment_info
                .as_ref()
                .unwrap()
                .get_attribute(mode_key),
            Some("BEST_SPEED".to_string()),
            "the stored-fields writer recorded its mode on the merge state's SegmentInfo"
        );
        let merged = merger.into_segment_info();
        assert_eq!(
            merged.get_attribute(mode_key),
            Some("BEST_SPEED".to_string())
        );
        assert_eq!(merged.max_doc().unwrap(), 2);
        assert_eq!(merged.min_version(), Some(Version::LATEST));
    }

    /// One source that does not know its minimum version makes the merged
    /// segment's unknown too: Java's loop sets `minVersion = null` and stops.
    #[test]
    fn one_source_without_a_min_version_leaves_the_merged_one_unknown() {
        let (_dir, infos) = stored_only_segments(2);
        // A segment of an index created by Lucene 6 may lack `minVersion`;
        // `LeafMetaData` allows that only for that major.
        let readers = vec![
            reader_with_min_version(&infos[0], Some(Version::LUCENE_10_0_0), 6),
            reader_with_min_version(&infos[1], None, 6),
        ];
        let target: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let merger = merger_over(&readers, &target);
        assert_eq!(merger.into_segment_info().min_version(), None);
    }

    /// A segment without a vector field answers "absent" for it through its
    /// per-field reader, search and merge instances alike — Java's `null`
    /// (`PerFieldKnnVectorsFormat.java:285-303`) — and a search over it
    /// collects nothing (`PerFieldKnnVectorsFormat.java:304-326`). The
    /// scalar-quantized merge relies on the first answer to skip that segment
    /// (`Lucene104ScalarQuantizedVectorsWriter.java:404-406, 449-452`).
    #[test]
    fn a_segment_without_a_vector_field_answers_absent_for_it() {
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        let mut doc = Document::new();
        doc.add(Box::new(
            KnnFloatVectorField::with_euclidean("v1", &[0.0, 1.0]).unwrap(),
        ));
        doc.add(Box::new(
            KnnFloatVectorField::with_euclidean("v2", &[1.0, 0.0]).unwrap(),
        ));
        writer.add_document(&doc).unwrap();
        writer.commit().unwrap();
        let mut doc = Document::new();
        doc.add(Box::new(
            KnnFloatVectorField::with_euclidean("v1", &[0.0, 2.0]).unwrap(),
        ));
        writer.add_document(&doc).unwrap();
        writer.commit().unwrap();
        writer.close().unwrap();
        let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
        assert_eq!(infos.size(), 2);
        let vectors_of = |i: usize| {
            SegmentReader::new(
                infos.info(i).clone(),
                infos.index_created_version_major(),
                &*DEFAULT_IO_CONTEXT,
            )
            .unwrap()
            .get_vector_reader()
            .unwrap()
            .unwrap()
        };

        // Positive control: the segment that has the field answers it.
        let with_v2 = vectors_of(0);
        assert_eq!(
            with_v2
                .get_float_vector_values("v2")
                .unwrap()
                .unwrap()
                .size(),
            1
        );

        let without_v2 = vectors_of(1);
        let merge_instance = without_v2.get_merge_instance().unwrap();
        for reader in [without_v2.as_ref(), merge_instance.as_ref()] {
            assert!(reader.get_float_vector_values("v2").unwrap().is_none());
            assert!(reader.get_byte_vector_values("v2").unwrap().is_none());
            let mut collector = crate::search::knn::TopKnnCollector::new(1, i64::MAX);
            let mut accept_docs = crate::search::from_live_docs(None, 1).unwrap();
            reader
                .search("v2", &[1.0, 0.0], &mut collector, &mut accept_docs)
                .unwrap();
            assert!(collector.top_docs_with_scores().is_empty());
        }
    }

    /// The merge instance of every producer a merge state holds reads the
    /// same data as the search instance it came from.
    ///
    /// The per-field postings, doc-values and vectors readers answer
    /// `getMergeInstance()` with a copy whose formats are their own merge
    /// instances (`PerFieldPostingsFormat.java:276-294`,
    /// `PerFieldDocValuesFormat.java:263-279`,
    /// `PerFieldKnnVectorsFormat.java:243-250`), and the block-tree terms and
    /// points readers inherit the `return this` of their base classes
    /// (`FieldsProducer.java:50-53`, `PointsReader.java:56-58`).
    #[test]
    fn every_merge_instance_reads_what_its_search_instance_reads() {
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        for i in 0..5i32 {
            let mut doc = Document::new();
            doc.add(Box::new(
                StringField::new("id", format!("doc-{i}"), Store::NO).unwrap(),
            ));
            doc.add(Box::new(IntPoint::new("rank", &[i * 3]).unwrap()));
            doc.add(Box::new(NumericDocValuesField::new("dv", i64::from(i) * 7)));
            doc.add(Box::new(
                KnnFloatVectorField::with_euclidean("v", &[i as f32, 1.0]).unwrap(),
            ));
            writer.add_document(&doc).unwrap();
        }
        writer.commit().unwrap();
        writer.close().unwrap();
        let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
        let reader = SegmentReader::new(
            infos.info(0).clone(),
            infos.index_created_version_major(),
            &*DEFAULT_IO_CONTEXT,
        )
        .unwrap();
        let field_infos = LeafReader::get_field_infos(&reader);

        // Postings: the per-field reader over the block-tree terms dictionary.
        let postings = reader.get_postings_reader().unwrap().unwrap();
        let merge_postings = postings.get_merge_instance().unwrap();
        assert_eq!(
            merge_postings.iterator().collect::<Vec<_>>(),
            postings.iterator().collect::<Vec<_>>()
        );
        let terms = |fields: &dyn crate::codecs::postings::FieldsProducer| {
            let terms = fields.terms("id").unwrap().expect("field id has terms");
            (terms.size(), terms.doc_count())
        };
        assert_eq!(terms(merge_postings.as_ref()), terms(postings.as_ref()));
        assert_eq!(terms(merge_postings.as_ref()), (5, 5));

        // Points.
        let points = reader.get_points_reader().unwrap().unwrap();
        let merge_points = points.get_merge_instance().unwrap();
        assert_eq!(merge_points.get_values("rank").unwrap().size().unwrap(), 5);
        assert_eq!(
            merge_points.get_values("rank").unwrap().size().unwrap(),
            points.get_values("rank").unwrap().size().unwrap()
        );

        // Doc values.
        let doc_values = reader.get_doc_values_reader().unwrap().unwrap();
        let merge_doc_values = doc_values.get_merge_instance().unwrap();
        let dv_field = field_infos.field_info("dv").unwrap().clone();
        let numeric = |producer: &dyn crate::codecs::DocValuesProducer| {
            let mut values = producer.get_numeric(&dv_field).unwrap();
            let mut out = Vec::new();
            loop {
                let doc = values.next_doc().unwrap();
                if doc == NO_MORE_DOCS {
                    break;
                }
                out.push((doc, values.long_value().unwrap()));
            }
            out
        };
        let expected: Vec<(i32, i64)> = (0..5).map(|i| (i, i64::from(i) * 7)).collect();
        assert_eq!(numeric(&*merge_doc_values), expected);
        assert_eq!(numeric(&*doc_values), expected);

        // Vectors: the per-field reader gives the field its own merge instance.
        let vectors = reader.get_vector_reader().unwrap().unwrap();
        let merge_vectors = vectors.get_merge_instance().unwrap();
        let read_vectors = |reader: &dyn crate::codecs::KnnVectorsReader| {
            let values = reader.get_float_vector_values("v").unwrap().unwrap();
            let mut iterator = values.iterator().unwrap();
            let mut out = Vec::new();
            loop {
                let doc = iterator.next_doc().unwrap();
                if doc == NO_MORE_DOCS {
                    break;
                }
                out.push((doc, values.vector_value(iterator.index()).unwrap()));
            }
            out
        };
        let expected: Vec<(i32, Vec<f32>)> = (0..5).map(|i| (i, vec![i as f32, 1.0])).collect();
        assert_eq!(read_vectors(merge_vectors.as_ref()), expected);
        assert_eq!(read_vectors(vectors.as_ref()), expected);
    }
}
