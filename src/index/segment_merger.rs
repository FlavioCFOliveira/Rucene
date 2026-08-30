//! `SegmentMerger` ported from `org.apache.lucene.index`.
//!
//! Merges several segments into one by driving each codec's consumer over a
//! [`MergeState`].

use std::sync::{Arc, Mutex};

use crate::codecs::postings::{
    NormsProducer as PostingsNormsProducer, NumericDocValues as PostingsNumericDocValues,
};
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

/// Adapts a [`crate::codecs::norms::NormsProducer`] to the postings module's own
/// `NormsProducer` trait.
///
/// **Divergence from Lucene 10.5.0.** Java has a single `NormsProducer`
/// interface that both the norms format and the postings merge use. This port
/// grew two: `codecs::norms::NormsProducer`, whose `get_norms` returns the
/// iterator-shaped `NumericDocValues` of the `index` module, and
/// `codecs::postings::NormsProducer`, whose `get_norms` returns a random-access
/// `NumericDocValues`. Merging postings needs the second while the codec
/// supplies the first, so this adapter materialises each field's norms into a
/// dense vector once and serves random access from it. Unifying the two traits
/// is the faithful fix and is tracked separately.
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
        })
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

        let field_infos = &self.write_state.field_infos;
        let has_norms = field_infos.has_norms();
        let has_doc_values = field_infos.has_doc_values();
        let has_points = field_infos.has_point_values();
        let has_vectors = field_infos.has_vector_values();
        let has_postings = field_infos.has_postings();

        if has_norms {
            let state = self.write_state.borrow();
            let mut consumer = self.codec.norms_format().norms_consumer(&state)?;
            consumer.merge(&self.merge_state)?;
            consumer.close()?;
        }

        if has_postings {
            self.merge_terms(has_norms)?;
        }

        if has_doc_values {
            let state = self.write_state.borrow();
            let mut consumer = self.codec.doc_values_format().fields_consumer(&state)?;
            consumer.merge(&self.merge_state)?;
            consumer.close()?;
        }

        if has_points {
            let state = self.write_state.borrow();
            let mut writer = self.codec.points_format().fields_writer(&state)?;
            writer.merge(&self.merge_state)?;
            writer.close()?;
        }

        if has_vectors {
            let mut writer = self
                .codec
                .knn_vectors_format()
                .fields_writer(&self.write_state)?;
            writer.merge(&self.merge_state)?;
            writer.close()?;
        }

        self.merge_field_infos()
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
        consumer.merge(&self.merge_state, norms.as_ref())?;
        consumer.close()?;
        Ok(())
    }

    /// Writes the merged field infos.
    ///
    /// Equivalent to `SegmentMerger.mergeFieldInfos`.
    fn merge_field_infos(&self) -> Result<()> {
        self.codec.field_infos_format().write(
            &*self.write_state.directory,
            &self.write_state.segment_info,
            "",
            &self.write_state.field_infos,
            &*self.write_state.context,
        )
    }
}
