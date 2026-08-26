//! `MappingMultiPostingsEnum` ported from `org.apache.lucene.index`.
//!
//! Segment merging reads the source segments through a
//! [`MultiPostingsEnum`], whose doc IDs live
//! in the *composite* space (`docBase + local`). The merged segment needs a
//! third space: the one produced by [`MergeState`]'s per-source
//! [`DocMap`]s, which both drop deleted documents and renumber the survivors.
//!
//! [`MappingMultiPostingsEnum`] performs that last translation. It splits the
//! merged postings back into one stream per source segment, hands each stream
//! to the [`DocIDMerger`] together with the segment's `DocMap`, and re-emits
//! the documents in merged order — sequentially when the merged segment keeps
//! the source order, or interleaved by mapped doc ID when the merged segment
//! must honour an index sort.
//!
//! # Reference
//!
//! - `org.apache.lucene.index.MappingMultiPostingsEnum`

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::merge::{DocIDMerger, DocIDMergerSub, DocMap, MergeState};
use crate::index::multi_fields::MultiPostingsEnum;
use crate::index::postings_enum::PostingsEnum;
use crate::index::MAX_POSITION;
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};

/// One source segment's postings, paired with the [`DocMap`] that renumbers its
/// documents into the merged segment.
///
/// Equivalent to `MappingMultiPostingsEnum.MappingPostingsSub`.
struct MappingPostingsSub {
    /// Owner of the doc maps. Java's sub holds a direct reference to its
    /// `MergeState.DocMap`; Rust's [`DocMap`] is not clonable, so the sub keeps
    /// the shared [`MergeState`] and the index of its own map instead.
    merge_state: Arc<MergeState>,
    /// Index of this sub's source segment in [`MergeState::doc_maps`].
    reader_index: usize,
    /// Postings of this source segment, in the segment's local doc-ID space.
    postings: Box<dyn PostingsEnum>,
    /// Current doc ID in the merged segment's space.
    mapped_doc_id: i32,
}

impl DocIDMergerSub for MappingPostingsSub {
    fn next_doc(&mut self) -> Result<i32> {
        // Java wraps the IOException in a RuntimeException here because
        // DocIDMerger.Sub.nextDoc() is declared without `throws`; Rucene's
        // trait returns Result, so the error simply propagates.
        self.postings.next_doc()
    }

    fn doc_map(&self) -> &DocMap {
        &self.merge_state.doc_maps[self.reader_index]
    }

    fn mapped_doc_id(&self) -> i32 {
        self.mapped_doc_id
    }

    fn set_mapped_doc_id(&mut self, doc_id: i32) {
        self.mapped_doc_id = doc_id;
    }
}

/// A [`PostingsEnum`] over several source segments that re-maps every document
/// into the merged segment's doc-ID space.
///
/// Equivalent to `org.apache.lucene.index.MappingMultiPostingsEnum`.
///
/// The enum is forward-only: [`DocIdSetIterator::advance`] is unsupported, as
/// in Lucene, because the merge always walks the postings from start to end.
pub struct MappingMultiPostingsEnum {
    field: String,
    merge_state: Arc<MergeState>,
    doc_id_merger: DocIDMerger<MappingPostingsSub>,
    /// `true` once [`DocIdSetIterator::next_doc`] has positioned the merger on
    /// a document, and `false` again once it is exhausted. This is Java's
    /// `current == null` test, kept separately because [`DocIDMerger`] reports
    /// its first sub as "current" before the first step.
    positioned: bool,
    /// Current doc ID in the merged segment's space.
    doc: i32,
}

impl std::fmt::Debug for MappingMultiPostingsEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MappingMultiPostingsEnum")
            .field("field", &self.field)
            .field("num_subs", &self.doc_id_merger.subs().len())
            .field("doc", &self.doc)
            .finish_non_exhaustive()
    }
}

impl MappingMultiPostingsEnum {
    /// Builds a mapping enum for `field` over the sub-postings of
    /// `postings_enum`.
    ///
    /// Equivalent to Lucene's `MappingMultiPostingsEnum(String, MergeState)`
    /// constructor immediately followed by `reset(MultiPostingsEnum)`. The two
    /// are fused here because Rucene's [`DocIDMerger`] owns its subs, so there
    /// is no merger to build before the postings are known.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if a sub-slice names a source
    /// segment that `merge_state` does not have a [`DocMap`] for.
    pub fn new(
        field: &str,
        merge_state: Arc<MergeState>,
        postings_enum: MultiPostingsEnum,
    ) -> Result<Self> {
        let subs = Self::build_subs(&merge_state, field, postings_enum)?;
        let index_is_sorted = merge_state.needs_index_sort;
        Ok(Self {
            field: field.to_string(),
            merge_state,
            doc_id_merger: DocIDMerger::new(subs, index_is_sorted)?,
            positioned: false,
            doc: -1,
        })
    }

    /// Re-points this enum at a new merged postings list for the same field.
    ///
    /// Equivalent to `MappingMultiPostingsEnum.reset(MultiPostingsEnum)`.
    ///
    /// # Errors
    ///
    /// Same as [`Self::new`].
    pub fn reset(&mut self, postings_enum: MultiPostingsEnum) -> Result<()> {
        let subs = Self::build_subs(&self.merge_state, &self.field, postings_enum)?;
        let index_is_sorted = self.merge_state.needs_index_sort;
        self.doc_id_merger = DocIDMerger::new(subs, index_is_sorted)?;
        self.positioned = false;
        self.doc = -1;
        Ok(())
    }

    /// Returns the field whose postings are being merged.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Splits a [`MultiPostingsEnum`] into one [`MappingPostingsSub`] per
    /// source segment, resolving each sub's `DocMap` through its slice's
    /// reader index.
    fn build_subs(
        merge_state: &Arc<MergeState>,
        field: &str,
        postings_enum: MultiPostingsEnum,
    ) -> Result<Vec<MappingPostingsSub>> {
        let num_readers = merge_state.doc_maps.len();
        let subs = postings_enum.into_subs();
        let mut mapped = Vec::with_capacity(subs.len());
        for sub in subs {
            let reader_index = sub.slice.reader_index;
            if reader_index < 0 || reader_index as usize >= num_readers {
                return Err(LuceneError::IllegalArgument(format!(
                    "field '{field}': sub-reader index {reader_index} is out of range for a \
                     merge of {num_readers} readers"
                )));
            }
            mapped.push(MappingPostingsSub {
                merge_state: Arc::clone(merge_state),
                reader_index: reader_index as usize,
                postings: sub.postings_enum,
                mapped_doc_id: -1,
            });
        }
        Ok(mapped)
    }

    /// Returns the sub whose document is current, or an error when the enum is
    /// unpositioned or exhausted.
    fn current(&self) -> Result<&MappingPostingsSub> {
        if !self.positioned {
            return Err(self.not_positioned());
        }
        self.doc_id_merger
            .current_sub()
            .ok_or_else(|| self.not_positioned())
    }

    /// Mutable counterpart of [`Self::current`].
    fn current_mut(&mut self) -> Result<&mut MappingPostingsSub> {
        if !self.positioned {
            return Err(self.not_positioned());
        }
        let field = self.field.clone();
        self.doc_id_merger.current_sub_mut().ok_or_else(|| {
            LuceneError::IllegalState(format!(
                "MappingMultiPostingsEnum for field '{field}' is not positioned on a document"
            ))
        })
    }

    fn not_positioned(&self) -> LuceneError {
        LuceneError::IllegalState(format!(
            "MappingMultiPostingsEnum for field '{}' is not positioned on a document",
            self.field
        ))
    }
}

impl DocIdSetIterator for MappingMultiPostingsEnum {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        if !self.positioned && self.doc == NO_MORE_DOCS {
            // `DocIdSetIterator` forbids reusing an exhausted iterator, and the
            // underlying `DocIDMerger` panics if stepped past its end, so the
            // exhausted state is answered here.
            return Ok(NO_MORE_DOCS);
        }
        match self.doc_id_merger.next_sub()? {
            Some(sub) => {
                self.doc = sub.mapped_doc_id();
                self.positioned = true;
                Ok(self.doc)
            }
            None => {
                self.positioned = false;
                // **Deliberate divergence**: Java's `docID()` reverts to -1
                // once the merge is exhausted, because it simply reports
                // `current == null`. Rucene reports `NO_MORE_DOCS`, which is
                // what `DocIdSetIterator` documents and what every other
                // iterator in this crate does. The merge path only ever reads
                // the value returned by `next_doc`, so the two behave alike
                // there.
                self.doc = NO_MORE_DOCS;
                Ok(NO_MORE_DOCS)
            }
        }
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "MappingMultiPostingsEnum does not support advance".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.doc_id_merger
            .subs()
            .iter()
            .map(|sub| sub.postings.cost())
            .sum()
    }
}

impl PostingsEnum for MappingMultiPostingsEnum {
    fn freq(&self) -> Result<i32> {
        self.current()?.postings.freq()
    }

    fn next_position(&mut self) -> Result<i32> {
        let field = self.field.clone();
        let sub = self.current_mut()?;
        let pos = sub.postings.next_position()?;
        let mapped_doc_id = sub.mapped_doc_id;
        if pos < 0 {
            return Err(LuceneError::CorruptIndex(format!(
                "position={pos} is negative, field=\"{field}\" doc={mapped_doc_id}"
            )));
        }
        if pos > MAX_POSITION {
            return Err(LuceneError::CorruptIndex(format!(
                "position={pos} is too large (> MAX_POSITION={MAX_POSITION}), \
                 field=\"{field}\" doc={mapped_doc_id}"
            )));
        }
        Ok(pos)
    }

    fn start_offset(&self) -> i32 {
        match self.current() {
            Ok(sub) => sub.postings.start_offset(),
            Err(_) => -1,
        }
    }

    fn end_offset(&self) -> i32 {
        match self.current() {
            Ok(sub) => sub.postings.end_offset(),
            Err(_) => -1,
        }
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        self.current()?.postings.get_payload()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use crate::index::merge::{deletion_doc_map, identity_doc_map};
    use crate::index::multi_fields::EnumWithSlice;
    use crate::index::{FieldInfos, ReaderSlice, SegmentInfo};
    use crate::store::{Directory, RamDirectory};
    use crate::util::{Bits, StringHelper, Version};

    // -----------------------------------------------------------------------
    // Fixtures
    // -----------------------------------------------------------------------

    /// Postings over an explicit list of local doc IDs.
    ///
    /// Each document carries the configured frequency, and every position is
    /// taken from `positions` in order, so a test can inject a value that the
    /// enum must reject as corrupt.
    struct VecPostings {
        docs: Vec<i32>,
        pos: i32,
        freq: i32,
        positions: Vec<i32>,
        position_upto: usize,
        payload: Option<Vec<u8>>,
    }

    impl VecPostings {
        fn boxed(docs: &[i32]) -> Box<dyn PostingsEnum> {
            Box::new(Self {
                docs: docs.to_vec(),
                pos: -1,
                freq: 1,
                positions: Vec::new(),
                position_upto: 0,
                payload: None,
            })
        }

        fn with_details(
            docs: &[i32],
            freq: i32,
            positions: &[i32],
            payload: Option<&[u8]>,
        ) -> Box<dyn PostingsEnum> {
            Box::new(Self {
                docs: docs.to_vec(),
                pos: -1,
                freq,
                positions: positions.to_vec(),
                position_upto: 0,
                payload: payload.map(|p| p.to_vec()),
            })
        }
    }

    impl DocIdSetIterator for VecPostings {
        fn doc_id(&self) -> i32 {
            if self.pos < 0 {
                -1
            } else if (self.pos as usize) >= self.docs.len() {
                NO_MORE_DOCS
            } else {
                self.docs[self.pos as usize]
            }
        }

        fn next_doc(&mut self) -> Result<i32> {
            self.pos += 1;
            self.position_upto = 0;
            if (self.pos as usize) >= self.docs.len() {
                Ok(NO_MORE_DOCS)
            } else {
                Ok(self.docs[self.pos as usize])
            }
        }

        fn advance(&mut self, target: i32) -> Result<i32> {
            while self.pos + 1 < self.docs.len() as i32
                && self.docs[(self.pos + 1) as usize] < target
            {
                self.pos += 1;
            }
            self.next_doc()
        }

        fn cost(&self) -> i64 {
            self.docs.len() as i64
        }
    }

    impl PostingsEnum for VecPostings {
        fn freq(&self) -> Result<i32> {
            Ok(self.freq)
        }

        fn next_position(&mut self) -> Result<i32> {
            let position = self
                .positions
                .get(self.position_upto)
                .copied()
                .unwrap_or(-1);
            self.position_upto += 1;
            Ok(position)
        }

        fn start_offset(&self) -> i32 {
            10
        }

        fn end_offset(&self) -> i32 {
            20
        }

        fn get_payload(&self) -> Result<Option<&[u8]>> {
            Ok(self.payload.as_deref())
        }
    }

    /// Live-docs bits with an explicit deleted set.
    #[derive(Debug)]
    struct LiveBits {
        len: usize,
        deleted: Vec<usize>,
    }

    impl Bits for LiveBits {
        fn get(&self, index: usize) -> bool {
            !self.deleted.contains(&index)
        }
        fn length(&self) -> usize {
            self.len
        }
    }

    fn segment_info(name: &str, max_doc: i32) -> SegmentInfo {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        SegmentInfo::new_without_codec(
            dir,
            Version::LUCENE_10_5_0,
            Some(Version::LUCENE_10_5_0),
            name.to_string(),
            max_doc,
            false,
            false,
            HashMap::new(),
            StringHelper::random_id(),
            HashMap::new(),
            crate::search::Sort::default(),
        )
        .unwrap()
    }

    /// Builds a merge state carrying only what this port reads from it: the
    /// per-source doc maps, the per-source `maxDoc`s and the index-sort flag.
    fn merge_state(
        doc_maps: Vec<DocMap>,
        max_docs: Vec<i32>,
        needs_index_sort: bool,
    ) -> Arc<MergeState> {
        let count = doc_maps.len();
        let mut fields_producers = Vec::with_capacity(count);
        let mut live_docs = Vec::with_capacity(count);
        let mut field_infos = Vec::with_capacity(count);
        for _ in 0..count {
            fields_producers.push(None);
            live_docs.push(None);
            field_infos.push(FieldInfos::empty());
        }
        Arc::new(MergeState::new(
            doc_maps,
            segment_info("_merged", max_docs.iter().sum()),
            FieldInfos::empty(),
            (0..count).map(|_| None).collect(),
            (0..count).map(|_| None).collect(),
            (0..count).map(|_| None).collect(),
            (0..count).map(|_| None).collect(),
            field_infos,
            live_docs,
            fields_producers,
            (0..count).map(|_| None).collect(),
            (0..count).map(|_| None).collect(),
            max_docs,
            needs_index_sort,
        ))
    }

    /// Two source segments of 3 documents each, concatenated without deletions.
    fn identity_merge_state() -> Arc<MergeState> {
        merge_state(
            vec![identity_doc_map(3, 0), identity_doc_map(3, 3)],
            vec![3, 3],
            false,
        )
    }

    fn merged(subs: Vec<EnumWithSlice>) -> MultiPostingsEnum {
        MultiPostingsEnum::new(subs)
    }

    fn drain(postings: &mut MappingMultiPostingsEnum) -> Vec<i32> {
        let mut docs = Vec::new();
        loop {
            let doc = postings.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                return docs;
            }
            docs.push(doc);
        }
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[test]
    fn maps_local_doc_ids_through_each_source_segments_doc_map() {
        let state = identity_merge_state();
        let subs = vec![
            EnumWithSlice::new(VecPostings::boxed(&[0, 2]), ReaderSlice::new(0, 3, 0)),
            EnumWithSlice::new(VecPostings::boxed(&[1]), ReaderSlice::new(3, 3, 1)),
        ];
        let mut postings = MappingMultiPostingsEnum::new("body", state, merged(subs)).unwrap();
        assert_eq!(
            drain(&mut postings),
            vec![0, 2, 4],
            "segment 1's local doc 1 becomes merged doc 4"
        );
    }

    #[test]
    fn drops_documents_the_doc_map_deletes() {
        // Source segment 0 has doc 1 deleted, so its live docs 0 and 2 become
        // merged docs 0 and 1; segment 1 then starts at merged doc 2.
        let live = Box::new(LiveBits {
            len: 3,
            deleted: vec![1],
        }) as Box<dyn Bits>;
        let state = merge_state(
            vec![deletion_doc_map(3, live, 0), identity_doc_map(2, 2)],
            vec![3, 2],
            false,
        );
        let subs = vec![
            EnumWithSlice::new(VecPostings::boxed(&[0, 1, 2]), ReaderSlice::new(0, 3, 0)),
            EnumWithSlice::new(VecPostings::boxed(&[0, 1]), ReaderSlice::new(3, 2, 1)),
        ];
        let mut postings = MappingMultiPostingsEnum::new("body", state, merged(subs)).unwrap();
        assert_eq!(drain(&mut postings), vec![0, 1, 2, 3]);
    }

    #[test]
    fn interleaves_by_mapped_doc_id_when_the_merged_segment_is_sorted() {
        // The merged segment interleaves the two sources: segment 0 owns the
        // even merged doc IDs, segment 1 the odd ones.
        let state = merge_state(
            vec![
                Box::new(|doc: i32| doc * 2) as DocMap,
                Box::new(|doc: i32| doc * 2 + 1) as DocMap,
            ],
            vec![3, 3],
            true,
        );
        let subs = vec![
            EnumWithSlice::new(VecPostings::boxed(&[0, 1, 2]), ReaderSlice::new(0, 3, 0)),
            EnumWithSlice::new(VecPostings::boxed(&[0, 1, 2]), ReaderSlice::new(3, 3, 1)),
        ];
        let mut postings = MappingMultiPostingsEnum::new("body", state, merged(subs)).unwrap();
        assert_eq!(drain(&mut postings), vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn doc_id_is_unpositioned_before_the_first_step_and_exhausted_after_the_last() {
        let state = identity_merge_state();
        let subs = vec![EnumWithSlice::new(
            VecPostings::boxed(&[1]),
            ReaderSlice::new(0, 3, 0),
        )];
        let mut postings = MappingMultiPostingsEnum::new("body", state, merged(subs)).unwrap();
        assert_eq!(postings.doc_id(), -1);
        assert_eq!(postings.next_doc().unwrap(), 1);
        assert_eq!(postings.doc_id(), 1);
        assert_eq!(postings.next_doc().unwrap(), NO_MORE_DOCS);
        assert_eq!(
            postings.doc_id(),
            NO_MORE_DOCS,
            "Rucene honours the DocIdSetIterator contract here; Java reverts to -1"
        );
        assert_eq!(
            postings.next_doc().unwrap(),
            NO_MORE_DOCS,
            "an exhausted enum stays exhausted instead of stepping the merger again"
        );
    }

    #[test]
    fn advance_is_unsupported() {
        let state = identity_merge_state();
        let subs = vec![EnumWithSlice::new(
            VecPostings::boxed(&[0]),
            ReaderSlice::new(0, 3, 0),
        )];
        let mut postings = MappingMultiPostingsEnum::new("body", state, merged(subs)).unwrap();
        assert!(matches!(
            postings.advance(1),
            Err(LuceneError::UnsupportedOperation(_))
        ));
    }

    #[test]
    fn cost_is_the_sum_of_the_sub_postings() {
        let state = identity_merge_state();
        let subs = vec![
            EnumWithSlice::new(VecPostings::boxed(&[0, 1]), ReaderSlice::new(0, 3, 0)),
            EnumWithSlice::new(VecPostings::boxed(&[0, 1, 2]), ReaderSlice::new(3, 3, 1)),
        ];
        let postings = MappingMultiPostingsEnum::new("body", state, merged(subs)).unwrap();
        assert_eq!(postings.cost(), 5);
    }

    #[test]
    fn per_document_accessors_read_the_current_sub() {
        let state = identity_merge_state();
        let subs = vec![
            EnumWithSlice::new(
                VecPostings::with_details(&[0], 7, &[3, 5], Some(b"first")),
                ReaderSlice::new(0, 3, 0),
            ),
            EnumWithSlice::new(
                VecPostings::with_details(&[0], 2, &[1], Some(b"second")),
                ReaderSlice::new(3, 3, 1),
            ),
        ];
        let mut postings = MappingMultiPostingsEnum::new("body", state, merged(subs)).unwrap();

        assert_eq!(postings.next_doc().unwrap(), 0);
        assert_eq!(postings.freq().unwrap(), 7);
        assert_eq!(postings.next_position().unwrap(), 3);
        assert_eq!(postings.next_position().unwrap(), 5);
        assert_eq!(postings.start_offset(), 10);
        assert_eq!(postings.end_offset(), 20);
        assert_eq!(postings.get_payload().unwrap(), Some(&b"first"[..]));

        assert_eq!(postings.next_doc().unwrap(), 3);
        assert_eq!(postings.freq().unwrap(), 2);
        assert_eq!(postings.get_payload().unwrap(), Some(&b"second"[..]));
    }

    #[test]
    fn per_document_accessors_report_an_unpositioned_enum() {
        let state = identity_merge_state();
        let subs = vec![EnumWithSlice::new(
            VecPostings::boxed(&[0]),
            ReaderSlice::new(0, 3, 0),
        )];
        let mut postings = MappingMultiPostingsEnum::new("body", state, merged(subs)).unwrap();
        assert!(matches!(postings.freq(), Err(LuceneError::IllegalState(_))));
        assert_eq!(postings.start_offset(), -1);
        assert_eq!(postings.end_offset(), -1);
        assert!(matches!(
            postings.next_position(),
            Err(LuceneError::IllegalState(_))
        ));
    }

    #[test]
    fn a_negative_position_is_reported_as_a_corrupt_index() {
        let state = identity_merge_state();
        let subs = vec![EnumWithSlice::new(
            VecPostings::with_details(&[0], 1, &[-5], None),
            ReaderSlice::new(0, 3, 0),
        )];
        let mut postings = MappingMultiPostingsEnum::new("body", state, merged(subs)).unwrap();
        assert_eq!(postings.next_doc().unwrap(), 0);
        let err = postings.next_position().unwrap_err();
        assert!(
            matches!(&err, LuceneError::CorruptIndex(m) if m.contains("is negative")),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn a_position_beyond_max_position_is_reported_as_a_corrupt_index() {
        let state = identity_merge_state();
        let subs = vec![EnumWithSlice::new(
            VecPostings::with_details(&[0], 1, &[MAX_POSITION + 1], None),
            ReaderSlice::new(0, 3, 0),
        )];
        let mut postings = MappingMultiPostingsEnum::new("body", state, merged(subs)).unwrap();
        assert_eq!(postings.next_doc().unwrap(), 0);
        let err = postings.next_position().unwrap_err();
        assert!(
            matches!(&err, LuceneError::CorruptIndex(m) if m.contains("too large")),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn a_sub_slice_naming_an_unknown_source_segment_is_rejected() {
        let state = identity_merge_state();
        // The merge has two sources, so reader index 2 cannot be resolved.
        let subs = vec![EnumWithSlice::new(
            VecPostings::boxed(&[0]),
            ReaderSlice::new(0, 3, 2),
        )];
        let err = MappingMultiPostingsEnum::new("body", state, merged(subs)).unwrap_err();
        assert!(
            matches!(&err, LuceneError::IllegalArgument(m) if m.contains("out of range")),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn reset_re_points_the_enum_at_another_term() {
        let state = identity_merge_state();
        let first = vec![EnumWithSlice::new(
            VecPostings::boxed(&[0, 1]),
            ReaderSlice::new(0, 3, 0),
        )];
        let mut postings = MappingMultiPostingsEnum::new("body", state, merged(first)).unwrap();
        assert_eq!(drain(&mut postings), vec![0, 1]);

        let second = vec![EnumWithSlice::new(
            VecPostings::boxed(&[2]),
            ReaderSlice::new(3, 3, 1),
        )];
        postings.reset(merged(second)).unwrap();
        assert_eq!(postings.doc_id(), -1, "reset rewinds the cursor");
        assert_eq!(drain(&mut postings), vec![5]);
        assert_eq!(postings.field(), "body");
    }

    #[test]
    fn a_term_present_in_no_source_segment_is_empty() {
        let state = identity_merge_state();
        let mut postings =
            MappingMultiPostingsEnum::new("body", state, merged(Vec::new())).unwrap();
        assert_eq!(postings.next_doc().unwrap(), NO_MORE_DOCS);
        assert_eq!(postings.cost(), 0);
    }
}
