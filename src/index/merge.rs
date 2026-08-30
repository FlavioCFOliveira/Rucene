//! Segment merge abstractions ported from `org.apache.lucene.index`.
//!
//! This module provides [`MergeState`], the container of per-segment readers and
//! doc-id mappings used during merging, and [`DocIDMerger`], the iterator that
//! produces the global document order.

#![deny(unsafe_code)]

use crate::codecs::{
    DocValuesProducer, FieldsProducer, KnnVectorsReader, NormsProducer, PointsReader,
    StoredFieldsReader, TermVectorsReader,
};
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::codec_reader::CodecReader;
use crate::index::{FieldInfos, SegmentInfo};
use crate::search::NO_MORE_DOCS;
use crate::util::Bits;

/// Maps document IDs from an old segment to the merged segment.
///
/// Returns the mapped doc ID or `-1` if the document is not mapped (deleted).
///
/// Equivalent to `MergeState.DocMap`.
pub type DocMap = Box<dyn Fn(i32) -> i32 + Send + Sync>;

/// Helper that builds an identity doc map (no deletions) for a segment of
/// `max_doc` documents, rebasing by `doc_base`.
pub fn identity_doc_map(max_doc: i32, doc_base: i32) -> DocMap {
    Box::new(move |doc_id: i32| {
        if doc_id < 0 || doc_id >= max_doc {
            -1
        } else {
            doc_base + doc_id
        }
    })
}

/// Helper that builds a doc map skipping deleted documents.
pub fn deletion_doc_map(max_doc: i32, live_docs: Box<dyn Bits>, doc_base: i32) -> DocMap {
    let mut new_doc = 0i32;
    let mut mapping = vec![-1i32; max_doc as usize];
    for old_doc in 0..max_doc {
        if live_docs.get(old_doc as usize) {
            mapping[old_doc as usize] = doc_base + new_doc;
            new_doc += 1;
        }
    }
    Box::new(move |doc_id: i32| {
        if doc_id < 0 || doc_id >= max_doc {
            -1
        } else {
            mapping[doc_id as usize]
        }
    })
}

/// Common state used during segment merging.
///
/// Equivalent to `org.apache.lucene.index.MergeState`.
pub struct MergeState {
    /// Maps document IDs from each source segment to the merged segment.
    pub doc_maps: Vec<DocMap>,
    /// Segment info for the newly merged segment.
    pub segment_info: Option<SegmentInfo>,
    /// Field infos for the newly merged segment.
    pub merge_field_infos: FieldInfos,
    /// Stored-fields readers for each source segment.
    pub stored_fields_readers: Vec<Option<Box<dyn StoredFieldsReader>>>,
    /// Term-vectors readers for each source segment.
    pub term_vectors_readers: Vec<Option<Box<dyn TermVectorsReader>>>,
    /// Norms producers for each source segment.
    pub norms_producers: Vec<Option<Arc<dyn NormsProducer>>>,
    /// Doc-values producers for each source segment.
    pub doc_values_producers: Vec<Option<Arc<dyn DocValuesProducer>>>,
    /// Field infos for each source segment.
    pub field_infos: Vec<FieldInfos>,
    /// Live docs for each source segment.
    pub live_docs: Vec<Option<Box<dyn Bits>>>,
    /// Postings producers for each source segment.
    pub fields_producers: Vec<Option<Arc<dyn FieldsProducer>>>,
    /// Points readers for each source segment.
    pub points_readers: Vec<Option<Arc<dyn PointsReader>>>,
    /// Vector readers for each source segment.
    pub knn_vectors_readers: Vec<Option<Arc<dyn KnnVectorsReader>>>,
    /// Maximum document count (exclusive) for each source segment.
    pub max_docs: Vec<i32>,
    /// Whether the merged segment needs to preserve index sort order.
    pub needs_index_sort: bool,
}

impl MergeState {
    /// Creates a merge state from per-segment components.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        doc_maps: Vec<DocMap>,
        segment_info: SegmentInfo,
        merge_field_infos: FieldInfos,
        stored_fields_readers: Vec<Option<Box<dyn StoredFieldsReader>>>,
        term_vectors_readers: Vec<Option<Box<dyn TermVectorsReader>>>,
        norms_producers: Vec<Option<Arc<dyn NormsProducer>>>,
        doc_values_producers: Vec<Option<Arc<dyn DocValuesProducer>>>,
        field_infos: Vec<FieldInfos>,
        live_docs: Vec<Option<Box<dyn Bits>>>,
        fields_producers: Vec<Option<Arc<dyn FieldsProducer>>>,
        points_readers: Vec<Option<Arc<dyn PointsReader>>>,
        knn_vectors_readers: Vec<Option<Arc<dyn KnnVectorsReader>>>,
        max_docs: Vec<i32>,
        needs_index_sort: bool,
    ) -> Self {
        Self {
            doc_maps,
            segment_info: Some(segment_info),
            merge_field_infos,
            stored_fields_readers,
            term_vectors_readers,
            norms_producers,
            doc_values_producers,
            field_infos,
            live_docs,
            fields_producers,
            points_readers,
            knn_vectors_readers,
            max_docs,
            needs_index_sort,
        }
    }

    /// Returns the number of source segments.
    /// Creates a merge state carrying only the postings producers and document
    /// counts, for a codec that merges one kind of data.
    pub fn from_fields_producers(
        fields_producers: Vec<Option<Arc<dyn FieldsProducer>>>,
        max_docs: Vec<i32>,
    ) -> Self {
        let n = fields_producers.len();
        Self {
            doc_maps: Vec::new(),
            segment_info: None,
            merge_field_infos: FieldInfos::default(),
            stored_fields_readers: (0..n).map(|_| None).collect(),
            term_vectors_readers: (0..n).map(|_| None).collect(),
            norms_producers: (0..n).map(|_| None).collect(),
            doc_values_producers: (0..n).map(|_| None).collect(),
            field_infos: Vec::new(),
            live_docs: (0..n).map(|_| None).collect(),
            fields_producers,
            points_readers: (0..n).map(|_| None).collect(),
            knn_vectors_readers: (0..n).map(|_| None).collect(),
            max_docs,
            needs_index_sort: false,
        }
    }

    /// Attaches per-segment and merged field metadata to this merge state.
    pub fn with_field_infos(
        mut self,
        field_infos: Vec<FieldInfos>,
        merge_field_infos: FieldInfos,
    ) -> Self {
        self.field_infos = field_infos;
        self.merge_field_infos = merge_field_infos;
        self
    }

    /// Called periodically by a long-running merge so it can stop early.
    ///
    /// Equivalent to `MergeState.checkAborted()`, which raises
    /// `MergePolicy.MergeAbortedException` once the writer has aborted the
    /// merge. Nothing aborts a merge yet, so this always succeeds.
    pub fn check_aborted(&self) -> Result<()> {
        Ok(())
    }

    /// Builds a `MergeState` from the readers being merged.
    ///
    /// Equivalent to Lucene 10.5.0's package-private
    /// `MergeState(List<CodecReader> readers, SegmentInfo segmentInfo, InfoStream
    /// infoStream, Executor intraMergeTaskExecutor)`: it pulls each producer off
    /// the reader rather than making the caller thread fourteen parallel vectors
    /// through by hand.
    pub fn from_readers(
        readers: &[Arc<dyn CodecReader>],
        doc_maps: Vec<DocMap>,
        segment_info: Option<SegmentInfo>,
        merge_field_infos: FieldInfos,
        needs_index_sort: bool,
    ) -> Result<Self> {
        let n = readers.len();
        let mut stored_fields_readers = Vec::with_capacity(n);
        let mut term_vectors_readers = Vec::with_capacity(n);
        let mut norms_producers = Vec::with_capacity(n);
        let mut doc_values_producers = Vec::with_capacity(n);
        let mut field_infos = Vec::with_capacity(n);
        let mut live_docs = Vec::with_capacity(n);
        let mut fields_producers = Vec::with_capacity(n);
        let mut points_readers = Vec::with_capacity(n);
        let mut knn_vectors_readers = Vec::with_capacity(n);
        let mut max_docs = Vec::with_capacity(n);

        for reader in readers {
            stored_fields_readers.push(reader.get_fields_reader()?);
            term_vectors_readers.push(reader.get_term_vectors_reader()?);
            norms_producers.push(reader.get_norms_reader()?);
            doc_values_producers.push(reader.get_doc_values_reader()?);
            fields_producers.push(reader.get_postings_reader()?);
            points_readers.push(reader.get_points_reader()?);
            knn_vectors_readers.push(reader.get_vector_reader()?);
            field_infos.push(reader.get_field_infos().clone());
            live_docs.push(reader.get_live_docs());
            max_docs.push(reader.max_doc());
        }

        Ok(Self {
            doc_maps,
            segment_info,
            merge_field_infos,
            stored_fields_readers,
            term_vectors_readers,
            norms_producers,
            doc_values_producers,
            field_infos,
            live_docs,
            fields_producers,
            points_readers,
            knn_vectors_readers,
            max_docs,
            needs_index_sort,
        })
    }

    pub fn num_readers(&self) -> usize {
        self.max_docs.len()
    }
}

impl std::fmt::Debug for MergeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MergeState")
            .field("num_readers", &self.num_readers())
            .field("max_docs", &self.max_docs)
            .field("needs_index_sort", &self.needs_index_sort)
            .finish_non_exhaustive()
    }
}

/// A sub-reader being merged by [`DocIDMerger`].
///
/// Equivalent to `DocIDMerger.Sub`.
pub trait DocIDMergerSub {
    /// Returns the next document ID from this sub reader, or [`NO_MORE_DOCS`]
    /// when exhausted.
    fn next_doc(&mut self) -> Result<i32>;

    /// Returns the doc-map used by this sub.
    fn doc_map(&self) -> &DocMap;

    /// Returns the current mapped doc ID.
    fn mapped_doc_id(&self) -> i32;

    /// Sets the current mapped doc ID.
    fn set_mapped_doc_id(&mut self, doc_id: i32);

    /// Advances to the next mapped document, skipping unmapped (deleted) docs.
    fn next_mapped_doc(&mut self) -> Result<i32> {
        loop {
            let doc = self.next_doc()?;
            if doc == NO_MORE_DOCS {
                self.set_mapped_doc_id(NO_MORE_DOCS);
                return Ok(NO_MORE_DOCS);
            }
            let mapped = (self.doc_map())(doc);
            if mapped != -1 {
                self.set_mapped_doc_id(mapped);
                return Ok(mapped);
            }
        }
    }
}

/// Produces the global doc-id order during a merge.
///
/// Equivalent to `org.apache.lucene.index.DocIDMerger`.
pub struct DocIDMerger<T: DocIDMergerSub> {
    subs: Vec<T>,
    state: MergerState,
}

enum MergerState {
    Sequential {
        current: Option<usize>,
        next_index: usize,
    },
    Sorted {
        current: Option<usize>,
        queue: Vec<usize>,
        queue_min_doc_id: i32,
    },
    Empty,
}

impl<T: DocIDMergerSub> DocIDMerger<T> {
    /// Creates a merger for the given sub-readers.
    pub fn new(mut subs: Vec<T>, index_is_sorted: bool) -> Result<Self> {
        let state = if subs.is_empty() {
            MergerState::Empty
        } else if index_is_sorted && subs.len() > 1 {
            let mut queue = Vec::with_capacity(subs.len() - 1);
            let current = 0usize;
            // The first sub is held out as `current`; initialize the rest.
            for (i, sub) in subs.iter_mut().enumerate().skip(1) {
                sub.set_mapped_doc_id(-1);
                if sub.next_mapped_doc()? != NO_MORE_DOCS {
                    queue.push(i);
                }
            }
            let queue_min_doc_id = if queue.is_empty() {
                NO_MORE_DOCS
            } else {
                // Find the smallest mapped doc id in the queue.
                let mut min = NO_MORE_DOCS;
                for &idx in &queue {
                    let doc = subs[idx].mapped_doc_id();
                    if doc < min {
                        min = doc;
                    }
                }
                min
            };
            // Simple heap ordering by mapped doc id.
            queue.sort_by_key(|&idx| subs[idx].mapped_doc_id());
            MergerState::Sorted {
                current: Some(current),
                queue,
                queue_min_doc_id,
            }
        } else {
            MergerState::Sequential {
                current: Some(0),
                next_index: 1,
            }
        };

        Ok(Self { subs, state })
    }

    /// Returns every sub-reader handed to this merger, in construction order.
    ///
    /// Java's `DocIDMerger` keeps the caller's `List<Sub>` alive and lets the
    /// caller walk it (for example `MappingMultiPostingsEnum.cost()`); here the
    /// merger owns the subs, so it exposes them instead.
    pub fn subs(&self) -> &[T] {
        &self.subs
    }

    /// Returns the sub positioned by the most recent [`Self::next_sub`] call,
    /// or `None` once the merge is exhausted.
    ///
    /// Before the first call to [`Self::next_sub`] this reports the first sub,
    /// which is not yet positioned on a document; callers that need Java's
    /// "no current sub yet" state must track it themselves, exactly as
    /// `MappingMultiPostingsEnum` does with its own `current` field.
    pub fn current_sub(&self) -> Option<&T> {
        self.current_index().map(|idx| &self.subs[idx])
    }

    /// Mutable counterpart of [`Self::current_sub`].
    pub fn current_sub_mut(&mut self) -> Option<&mut T> {
        self.current_index().map(|idx| &mut self.subs[idx])
    }

    /// Index into `subs` of the sub returned by the last [`Self::next_sub`].
    fn current_index(&self) -> Option<usize> {
        match &self.state {
            MergerState::Empty => None,
            MergerState::Sequential { current, .. } | MergerState::Sorted { current, .. } => {
                *current
            }
        }
    }

    /// Resets the merger to the beginning.
    pub fn reset(&mut self) -> Result<()> {
        self.state = if self.subs.is_empty() {
            MergerState::Empty
        } else if matches!(self.state, MergerState::Sorted { .. }) {
            let mut queue = Vec::with_capacity(self.subs.len() - 1);
            let current = 0usize;
            for i in 1..self.subs.len() {
                self.subs[i].set_mapped_doc_id(-1);
                if self.subs[i].next_mapped_doc()? != NO_MORE_DOCS {
                    queue.push(i);
                }
            }
            queue.sort_by_key(|&idx| self.subs[idx].mapped_doc_id());
            let queue_min_doc_id = queue
                .first()
                .map(|&idx| self.subs[idx].mapped_doc_id())
                .unwrap_or(NO_MORE_DOCS);
            MergerState::Sorted {
                current: Some(current),
                queue,
                queue_min_doc_id,
            }
        } else {
            MergerState::Sequential {
                current: Some(0),
                next_index: 1,
            }
        };
        Ok(())
    }

    /// Returns the next sub reader in merge order, or `None` when done.
    pub fn next_sub(&mut self) -> Result<Option<&mut T>> {
        match &mut self.state {
            MergerState::Empty => Ok(None),
            MergerState::Sequential {
                current,
                next_index,
            } => loop {
                let idx = current.ok_or_else(|| {
                    LuceneError::IllegalState("DocIDMerger is exhausted".to_string())
                })?;
                if self.subs[idx].next_mapped_doc()? != NO_MORE_DOCS {
                    return Ok(Some(&mut self.subs[idx]));
                }
                if *next_index >= self.subs.len() {
                    *current = None;
                    return Ok(None);
                }
                *current = Some(*next_index);
                *next_index += 1;
            },
            MergerState::Sorted {
                current,
                queue,
                queue_min_doc_id,
            } => {
                let current_idx = current.expect("Sorted merger has no current sub");
                let next_doc = self.subs[current_idx].next_mapped_doc()?;
                if next_doc < *queue_min_doc_id {
                    return Ok(Some(&mut self.subs[current_idx]));
                }

                if next_doc == NO_MORE_DOCS {
                    if queue.is_empty() {
                        *current = None;
                    } else {
                        *current = Some(queue.remove(0));
                    }
                } else {
                    // Replace the top of the queue with current and re-sort.
                    if let Some(top) = queue.first().copied() {
                        let old_top_doc = self.subs[top].mapped_doc_id();
                        assert_eq!(*queue_min_doc_id, old_top_doc);
                        assert!(next_doc > old_top_doc);
                    }
                    if let Some(top) = queue.first_mut() {
                        let previous_top = *top;
                        *top = current_idx;
                        self.subs[current_idx].set_mapped_doc_id(next_doc);
                        *current = Some(previous_top);
                    }
                    queue.sort_by_key(|&idx| self.subs[idx].mapped_doc_id());
                }

                *queue_min_doc_id = queue
                    .first()
                    .map(|&idx| self.subs[idx].mapped_doc_id())
                    .unwrap_or(NO_MORE_DOCS);

                Ok(current.map(|idx| &mut self.subs[idx]))
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::MatchAllBits;

    struct VecSub {
        docs: Vec<i32>,
        idx: i32,
        map: DocMap,
        mapped: i32,
    }

    impl VecSub {
        fn new(docs: Vec<i32>, map: DocMap) -> Self {
            Self {
                docs,
                idx: -1,
                map,
                mapped: -1,
            }
        }
    }

    impl DocIDMergerSub for VecSub {
        fn next_doc(&mut self) -> Result<i32> {
            self.idx += 1;
            if self.idx as usize >= self.docs.len() {
                Ok(NO_MORE_DOCS)
            } else {
                Ok(self.docs[self.idx as usize])
            }
        }

        fn doc_map(&self) -> &DocMap {
            &self.map
        }

        fn mapped_doc_id(&self) -> i32 {
            self.mapped
        }

        fn set_mapped_doc_id(&mut self, doc_id: i32) {
            self.mapped = doc_id;
        }
    }

    #[test]
    fn sequential_merger_concatenates_segments() {
        let subs = vec![
            VecSub::new(vec![0, 1, 2], identity_doc_map(3, 0)),
            VecSub::new(vec![0, 1], identity_doc_map(2, 3)),
        ];
        let mut merger = DocIDMerger::new(subs, false).unwrap();
        let mut order = Vec::new();
        while let Some(sub) = merger.next_sub().unwrap() {
            order.push(sub.mapped_doc_id());
        }
        assert_eq!(order, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn sequential_merger_skips_deleted_docs() {
        #[derive(Debug)]
        struct SparseBits {
            len: usize,
            deleted: Vec<bool>,
        }
        impl Bits for SparseBits {
            fn get(&self, index: usize) -> bool {
                !self.deleted[index]
            }
            fn length(&self) -> usize {
                self.len
            }
        }
        // First segment: docs 0 and 2 are live, doc 1 is deleted.
        let live_a = Box::new(SparseBits {
            len: 3,
            deleted: vec![false, true, false],
        }) as Box<dyn Bits>;
        // Second segment: all docs live.
        let live_b = Box::new(MatchAllBits::new(2)) as Box<dyn Bits>;
        let subs = vec![
            VecSub::new(vec![0, 1, 2], deletion_doc_map(3, live_a, 0)),
            VecSub::new(vec![0, 1], deletion_doc_map(2, live_b, 2)),
        ];
        let mut merger = DocIDMerger::new(subs, false).unwrap();
        let mut order = Vec::new();
        while let Some(sub) = merger.next_sub().unwrap() {
            order.push(sub.mapped_doc_id());
        }
        assert_eq!(order, vec![0, 1, 2, 3]);
    }

    #[test]
    fn sorted_merger_interleaves_by_mapped_id() {
        // Sub 0 maps docs 0,1,2 -> 0,2,4 (even)
        // Sub 1 maps docs 0,1,2 -> 1,3,5 (odd)
        let subs = vec![
            VecSub::new(vec![0, 1, 2], Box::new(|doc: i32| doc * 2)),
            VecSub::new(vec![0, 1, 2], Box::new(|doc: i32| doc * 2 + 1)),
        ];
        let mut merger = DocIDMerger::new(subs, true).unwrap();
        let mut order = Vec::new();
        while let Some(sub) = merger.next_sub().unwrap() {
            order.push(sub.mapped_doc_id());
        }
        assert_eq!(order, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn empty_merger_returns_none() {
        let subs: Vec<VecSub> = vec![];
        let mut merger = DocIDMerger::new(subs, false).unwrap();
        assert!(merger.next_sub().unwrap().is_none());
    }

    #[test]
    fn single_sub_sorted_is_sequential() {
        // A single sub in sorted mode still follows the sub's own order.
        let subs = vec![VecSub::new(vec![0, 2, 3], identity_doc_map(4, 0))];
        let mut merger = DocIDMerger::new(subs, true).unwrap();
        let mut order = Vec::new();
        while let Some(sub) = merger.next_sub().unwrap() {
            order.push(sub.mapped_doc_id());
        }
        assert_eq!(order, vec![0, 2, 3]);
    }

    #[test]
    fn current_sub_tracks_the_sub_returned_by_the_last_step() {
        let subs = vec![
            VecSub::new(vec![0, 1], identity_doc_map(2, 0)),
            VecSub::new(vec![0], identity_doc_map(1, 2)),
        ];
        let mut merger = DocIDMerger::new(subs, false).unwrap();
        assert_eq!(merger.subs().len(), 2);

        assert_eq!(merger.next_sub().unwrap().unwrap().mapped_doc_id(), 0);
        assert_eq!(merger.current_sub().unwrap().mapped_doc_id(), 0);
        assert_eq!(merger.next_sub().unwrap().unwrap().mapped_doc_id(), 1);
        assert_eq!(merger.next_sub().unwrap().unwrap().mapped_doc_id(), 2);
        assert_eq!(
            merger.current_sub_mut().unwrap().mapped_doc_id(),
            2,
            "the mutable accessor sees the same sub"
        );
        assert!(merger.next_sub().unwrap().is_none());
        assert!(
            merger.current_sub().is_none(),
            "an exhausted merger has no current sub"
        );
    }

    #[test]
    fn current_sub_is_none_for_an_empty_merger() {
        let subs: Vec<VecSub> = vec![];
        let merger = DocIDMerger::new(subs, false).unwrap();
        assert!(merger.current_sub().is_none());
        assert!(merger.subs().is_empty());
    }

    #[test]
    fn deletion_doc_map_skips_unmapped() {
        #[derive(Debug)]
        struct SparseBits {
            len: usize,
            deleted: Vec<bool>,
        }
        impl Bits for SparseBits {
            fn get(&self, index: usize) -> bool {
                !self.deleted[index]
            }
            fn length(&self) -> usize {
                self.len
            }
        }
        let bits = Box::new(SparseBits {
            len: 5,
            deleted: vec![false, true, false, false, true],
        }) as Box<dyn Bits>;
        let map = deletion_doc_map(5, bits, 10);
        assert_eq!(map(0), 10);
        assert_eq!(map(1), -1);
        assert_eq!(map(2), 11);
        assert_eq!(map(3), 12);
        assert_eq!(map(4), -1);
    }
}
