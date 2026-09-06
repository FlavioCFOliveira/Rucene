//! `SegmentMerger` ported from `org.apache.lucene.index`.
//!
//! Merges several segments into one by driving each codec's consumer over a
//! [`MergeState`].

use std::sync::{Arc, Mutex};

use crate::codecs::postings::{PostingsNormsProducer, PostingsNumericDocValues};
use crate::codecs::state::{OwnedSegmentWriteState, SegmentReadState};
use crate::codecs::stub::{BufferedUpdates, FieldInfo};
use crate::codecs::Codec;
use crate::error::{LuceneError, Result};
use crate::index::codec_reader::CodecReader;
use crate::index::merge::{DocMap, MergeState};
use crate::index::segment_info::SegmentInfo;
use crate::index::FieldInfos;
use crate::store::{Directory, IOContext};
use crate::util::InfoStream;

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
/// **Divergence from Lucene 10.5.0.** Java's constructor also takes an
/// `InfoStream`, an intra-merge `Executor` and the `OneMerge`, storing them on
/// the `MergeState` so a codec can parallelise a merge and report progress.
/// This port's `MergeState` carries none of the three, so the merge runs on the
/// calling thread and reports nothing; the sequence of consumer calls and the
/// files written are unchanged. The merged segment's `SegmentInfo` lives on the
/// owned write state rather than on the merge state, because `SegmentInfo` is
/// not `Clone` in this crate and both would otherwise need it.
pub struct SegmentMerger {
    codec: Arc<dyn Codec>,
    write_state: OwnedSegmentWriteState,
    /// The state every consumer merges from.
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
    /// Equivalent to the `SegmentMerger` constructor.
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
        // The write state owns the segment info, so the merge state carries none.
        let merge_state = MergeState::from_readers(
            readers,
            doc_maps,
            None,
            merge_field_infos.clone(),
            needs_index_sort,
        )?;

        let write_state = OwnedSegmentWriteState::new(
            info_stream,
            directory,
            segment_info,
            merge_field_infos,
            BufferedUpdates::default(),
            context,
        );

        Ok(Self {
            codec,
            write_state,
            merge_state,
            merge_state_creation_thread: std::thread::current().id(),
        })
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
        Ok(self.write_state.segment_info.max_doc()? > 0)
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
            self.write_state.segment_info.max_doc()?,
            "numMerged does not match the merged segment's maxDoc"
        );

        let field_infos = &self.write_state.field_infos;
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
            &*self.write_state.directory,
            &self.write_state.segment_info,
            &*self.write_state.context,
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
            &*self.write_state.directory,
            &self.write_state.segment_info,
            &*self.write_state.context,
        )?;
        let merged = writer.merge(self.merge_state())?;
        debug_assert_eq!(merged, self.write_state.segment_info.max_doc()?);
        writer.close()?;
        Ok(merged)
    }

    /// Merges the norms.
    ///
    /// Equivalent to `SegmentMerger.mergeNorms(SegmentWriteState, SegmentReadState)`.
    fn merge_norms(&mut self) -> Result<()> {
        let state = self.write_state.borrow();
        let mut consumer = self.codec.norms_format().norms_consumer(&state)?;
        consumer.merge(self.merge_state())?;
        consumer.close()
    }

    /// Merges the doc values.
    ///
    /// Equivalent to `SegmentMerger.mergeDocValues(SegmentWriteState, SegmentReadState)`.
    fn merge_doc_values(&mut self) -> Result<()> {
        let state = self.write_state.borrow();
        let mut consumer = self.codec.doc_values_format().fields_consumer(&state)?;
        consumer.merge(self.merge_state())?;
        consumer.close()
    }

    /// Merges the points.
    ///
    /// Equivalent to `SegmentMerger.mergePoints(SegmentWriteState, SegmentReadState)`.
    fn merge_points(&mut self) -> Result<()> {
        let state = self.write_state.borrow();
        let mut writer = self.codec.points_format().fields_writer(&state)?;
        writer.merge(self.merge_state())?;
        writer.close()
    }

    /// Merges the KNN vectors.
    ///
    /// Equivalent to `SegmentMerger.mergeVectorValues(SegmentWriteState, SegmentReadState)`.
    fn merge_vector_values(&mut self) -> Result<()> {
        let mut writer = self
            .codec
            .knn_vectors_format()
            .fields_writer(&self.write_state)?;
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
        if self.write_state.info_stream.is_enabled(SM) {
            self.write_state.info_stream.message(
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
        if self.write_state.info_stream.is_enabled(SM) {
            self.write_state.info_stream.message(
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
        let max_doc = self.write_state.segment_info.max_doc()?;
        let state = self.write_state.borrow();

        let norms: Box<dyn PostingsNormsProducer> = if has_norms {
            let read_state = SegmentReadState::new(
                &*self.write_state.directory,
                &self.write_state.segment_info,
                &self.write_state.field_infos,
                &*self.write_state.context,
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

    /// Consumes the merger and returns the merged segment's info.
    ///
    /// The merged `SegmentInfo` lives on the write state, so this is how a
    /// caller recovers it once the merge has run.
    pub fn into_segment_info(self) -> SegmentInfo {
        self.write_state.segment_info
    }

    /// Writes the merged field infos.
    ///
    /// Equivalent to `SegmentMerger.mergeFieldInfos`.
    fn merge_field_infos(&mut self) -> Result<()> {
        self.codec.field_infos_format().write(
            &*self.write_state.directory,
            &self.write_state.segment_info,
            "",
            &self.write_state.field_infos,
            &*self.write_state.context,
        )
    }
}
