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

/// The `NullPointerException` Java raises when a method is invoked on a sub
/// whose `postings` field is `null`, reached through `expression`.
///
/// The JDK names the dereferenced expression: `this.postings` inside
/// `MappingPostingsSub.nextDoc()` (`MappingMultiPostingsEnum.java:48`), and
/// `this.current.postings` in the enum's own per-document methods
/// (`MappingMultiPostingsEnum.java:81`, `:110`, `:142`). Measured against
/// `lucene-core-10.5.0.jar` with a `current` sub whose `postings` is `null`.
fn null_sub_postings(method: &str, expression: &str) -> LuceneError {
    LuceneError::null_pointer_with_message(format!(
        "Cannot invoke \"org.apache.lucene.index.PostingsEnum.{method}\" because \"{expression}\" is null"
    ))
}

/// One source segment's postings, paired with the [`DocMap`] that renumbers its
/// documents into the merged segment.
///
/// Equivalent to `MappingMultiPostingsEnum.MappingPostingsSub`
/// (`MappingMultiPostingsEnum.java:38-52`), a `DocIDMerger.Sub` holding its
/// `docMap` and a `public PostingsEnum postings` that may be `null`.
struct MappingPostingsSub {
    /// Java's `DocIDMerger.Sub.docMap`.
    doc_map: DocMap,
    /// Java's `postings`.
    postings: Option<Box<dyn PostingsEnum>>,
    /// Java's `DocIDMerger.Sub.mappedDocID`.
    mapped_doc_id: i32,
    /// Index of this sub in `MappingMultiPostingsEnum::all_subs` — the source
    /// segment's reader index — so that the sub can go back to its slot.
    reader_index: usize,
}

impl DocIDMergerSub for MappingPostingsSub {
    fn next_doc(&mut self) -> Result<i32> {
        // Java wraps the IOException in a RuntimeException here because
        // DocIDMerger.Sub.nextDoc() is declared without `throws`; Rucene's
        // trait returns Result, so the error simply propagates.
        self.postings
            .as_mut()
            .ok_or_else(|| null_sub_postings("nextDoc()", "this.postings"))?
            .next_doc()
    }

    fn doc_map(&self) -> &DocMap {
        &self.doc_map
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
/// Equivalent to `org.apache.lucene.index.MappingMultiPostingsEnum`. It is
/// built once per field ([`new`](Self::new)) and [`reset`](Self::reset) on the
/// [`MultiPostingsEnum`] of every term.
///
/// The enum is forward-only: [`DocIdSetIterator::advance`] is unsupported, as
/// in Lucene, because the merge always walks the postings from start to end.
///
/// Java's subs alias three holders at once: `allSubs[readerIndex]`, the
/// `subs` list the [`DocIDMerger`] reads, and — through their `postings` —
/// the slots of `multiDocsAndPositionsEnum`. Rust gives each one owner: a sub
/// lives in `all_subs` until [`reset`](Self::reset) moves it into the
/// merger's list, and a sub's postings live in the [`MultiPostingsEnum`] slot
/// they came from until `reset` moves them into the sub; they go back before
/// the [`MultiPostingsEnum`] is next offered for reuse
/// (`take_multi_docs_and_positions_enum`).
pub struct MappingMultiPostingsEnum {
    /// Java's `MultiPostingsEnum multiDocsAndPositionsEnum`, `null` until the
    /// first [`reset`](Self::reset). It is kept behind the trait object so that
    /// the very allocation goes back to `MultiTermsEnum::postings` as `reuse`,
    /// as Java passes the same object; it is always a [`MultiPostingsEnum`].
    multi_docs_and_positions_enum: Option<Box<dyn PostingsEnum>>,
    /// Java's `final String field`.
    field: String,
    /// Java's `final DocIDMerger<MappingPostingsSub> docIDMerger`, which owns
    /// Java's `subs` list.
    doc_id_merger: DocIDMerger<MappingPostingsSub>,
    /// `true` while Java's `current` is not `null`: the merger has positioned
    /// a sub on a document, which [`DocIDMerger::current_sub`] then names.
    positioned: bool,
    /// The current doc ID in the merged segment's space: Java's
    /// `current.mappedDocID`.
    doc: i32,
    /// Java's `final MappingPostingsSub[] allSubs`, one per source segment; a
    /// slot is `None` while its sub is in the merger's list.
    all_subs: Vec<Option<MappingPostingsSub>>,
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
    /// Creates the enum for `field`, with one sub per source segment of the
    /// merge.
    ///
    /// Equivalent to `MappingMultiPostingsEnum(String, MergeState)`
    /// (`MappingMultiPostingsEnum.java:55-63`): `allSubs` has one entry per
    /// `mergeState.fieldsProducers`, each holding its segment's doc map, and the
    /// merger is `DocIDMerger.of(subs, allSubs.length, mergeState.needsIndexSort)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) — Java's
    /// `ArrayIndexOutOfBoundsException` — when `merge_state` has fewer doc maps
    /// than fields producers.
    pub fn new(field: &str, merge_state: Arc<MergeState>) -> Result<Self> {
        let count = merge_state.fields_producers.len();
        let mut all_subs = Vec::with_capacity(count);
        for i in 0..count {
            let doc_map = merge_state.doc_maps.get(i).ok_or_else(|| {
                LuceneError::illegal_argument(format!(
                    "Index {i} out of bounds for length {}",
                    merge_state.doc_maps.len()
                ))
            })?;
            all_subs.push(Some(MappingPostingsSub {
                doc_map: Arc::clone(doc_map),
                postings: None,
                mapped_doc_id: -1,
                reader_index: i,
            }));
        }
        let doc_id_merger = DocIDMerger::of(Vec::new(), count, merge_state.needs_index_sort)?;
        Ok(Self {
            multi_docs_and_positions_enum: None,
            field: field.to_string(),
            doc_id_merger,
            positioned: false,
            doc: -1,
            all_subs,
        })
    }

    /// Re-points this enum at the merged postings of the next term.
    ///
    /// Equivalent to `MappingMultiPostingsEnum.reset(MultiPostingsEnum)`
    /// (`MappingMultiPostingsEnum.java:65-77`): `postings_enum` becomes
    /// `multiDocsAndPositionsEnum`, the merger's list is cleared, every live
    /// slot of `postings_enum` puts the sub of its slice's reader index on the
    /// list with the slot's postings, and the merger is reset. As in Java,
    /// the current document is not touched; the next
    /// [`next_doc`](DocIdSetIterator::next_doc) moves on to the new term.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalState`](crate::error::LuceneErrorKind::IllegalState) — Java's `ClassCastException` —
    /// when `postings_enum` is not a [`MultiPostingsEnum`];
    /// [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) — Java's
    /// `ArrayIndexOutOfBoundsException` — for a slice whose reader index the
    /// merge has no source segment for, and [`LuceneErrorKind::IllegalState`](crate::error::LuceneErrorKind::IllegalState) for a
    /// reader index that appears twice, which Java would alias; and propagates
    /// whatever resetting the merger raises.
    pub fn reset(&mut self, mut postings_enum: Box<dyn PostingsEnum>) -> Result<&mut Self> {
        if postings_enum
            .as_any_mut()
            .and_then(|any| any.downcast_mut::<MultiPostingsEnum>())
            .is_none()
        {
            return Err(LuceneError::illegal_state(format!(
                "{postings_enum} cannot be cast to class org.apache.lucene.index.MultiPostingsEnum"
            )));
        }
        // Rust-only: hand the previous term's postings back to the
        // MultiPostingsEnum they came from, where Java's aliases keep them.
        self.return_sub_postings();
        self.multi_docs_and_positions_enum = Some(postings_enum);

        // subs.clear()
        let subs = std::mem::take(self.doc_id_merger.subs_mut());
        for mut sub in subs {
            sub.postings = None;
            let reader_index = sub.reader_index;
            self.all_subs[reader_index] = Some(sub);
        }

        let multi = self
            .multi_docs_and_positions_enum
            .as_mut()
            .and_then(|postings_enum| postings_enum.as_any_mut())
            .and_then(|any| any.downcast_mut::<MultiPostingsEnum>())
            .ok_or_else(|| {
                LuceneError::illegal_state(
                    "INVARIANT: the enum was just checked to be a MultiPostingsEnum".to_string(),
                )
            })?;
        let count = multi.get_num_subs();
        let num_readers = self.all_subs.len();
        let subs_array = multi.get_subs_mut();
        for slot in subs_array.iter_mut().take(count) {
            let reader_index = slot.slice.reader_index;
            let index = usize::try_from(reader_index)
                .ok()
                .filter(|&index| index < num_readers)
                .ok_or_else(|| {
                    LuceneError::illegal_argument(format!(
                        "Index {reader_index} out of bounds for length {num_readers}"
                    ))
                })?;
            let mut sub = self.all_subs[index].take().ok_or_else(|| {
                LuceneError::illegal_state(format!(
                    "field \"{}\": reader index {reader_index} appears twice in one term's \
                     postings",
                    self.field
                ))
            })?;
            sub.postings = slot.postings_enum.take();
            self.doc_id_merger.subs_mut().push(sub);
        }
        self.doc_id_merger.reset()?;
        Ok(self)
    }

    /// Moves every sub's postings back into the slot of
    /// `multiDocsAndPositionsEnum` it was taken from by
    /// [`reset`](Self::reset): the sub at position `i` of the merger's list
    /// came from live slot `i`.
    fn return_sub_postings(&mut self) {
        let Some(multi) = self
            .multi_docs_and_positions_enum
            .as_mut()
            .and_then(|postings_enum| postings_enum.as_any_mut())
            .and_then(|any| any.downcast_mut::<MultiPostingsEnum>())
        else {
            return;
        };
        let slots = multi.get_subs_mut();
        for (i, sub) in self.doc_id_merger.subs_mut().iter_mut().enumerate() {
            if let (Some(postings), Some(slot)) = (sub.postings.take(), slots.get_mut(i)) {
                slot.postings_enum = Some(postings);
            }
        }
    }

    /// Returns the [`MultiPostingsEnum`] of the last [`reset`](Self::reset),
    /// with every sub's postings back in it, for the caller to offer as
    /// `reuse`; `None` before the first reset.
    ///
    /// Equivalent to reading the `multiDocsAndPositionsEnum` field, as
    /// `MappedMultiTermsEnum.postings` does to pass it to
    /// `MultiTermsEnum.postings` (`MappedMultiFields.java:129-131`). Java
    /// passes the field and later overwrites it; Rust moves the enum out, and
    /// the [`reset`](Self::reset) that follows puts the returned one in.
    pub(crate) fn take_multi_docs_and_positions_enum(&mut self) -> Option<Box<dyn PostingsEnum>> {
        self.return_sub_postings();
        self.multi_docs_and_positions_enum.take()
    }

    /// Returns the [`MultiPostingsEnum`] of the last [`reset`](Self::reset),
    /// without moving it: Java's package-private `multiDocsAndPositionsEnum`
    /// as a test reads it.
    #[cfg(test)]
    pub(crate) fn multi_docs_and_positions_enum(&self) -> Option<&dyn PostingsEnum> {
        self.multi_docs_and_positions_enum.as_deref()
    }

    /// Returns the field whose postings are being merged.
    ///
    /// Equivalent to reading the `MappingMultiPostingsEnum.field` field, which
    /// `MappedMultiTermsEnum.postings` compares before reusing an enum
    /// (`MappedMultiFields.java:120`).
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns the sub whose document is current, or Java's
    /// `NullPointerException` when `current` is `null`.
    fn current(&self) -> Result<&MappingPostingsSub> {
        if !self.positioned {
            return Err(null_current());
        }
        self.doc_id_merger.current_sub().ok_or_else(null_current)
    }

    /// Mutable counterpart of [`Self::current`].
    fn current_mut(&mut self) -> Result<&mut MappingPostingsSub> {
        if !self.positioned {
            return Err(null_current());
        }
        self.doc_id_merger
            .current_sub_mut()
            .ok_or_else(null_current)
    }
}

/// The `NullPointerException` Java raises when a per-document method reads
/// `current.postings` while `current` is `null`.
fn null_current() -> LuceneError {
    LuceneError::null_pointer_with_message(
        "Cannot read field \"postings\" because \"this.current\" is null".to_string(),
    )
}

impl DocIdSetIterator for MappingMultiPostingsEnum {
    /// Equivalent to `MappingMultiPostingsEnum.docID()`
    /// (`MappingMultiPostingsEnum.java:84-91`): `-1` whenever no sub is
    /// current, which is both before the first [`next_doc`](Self::next_doc)
    /// and once the merge is exhausted.
    fn doc_id(&self) -> i32 {
        if self.positioned {
            self.doc
        } else {
            -1
        }
    }

    /// Equivalent to `MappingMultiPostingsEnum.nextDoc()`
    /// (`MappingMultiPostingsEnum.java:98-106`).
    ///
    /// # Errors
    ///
    /// Propagates whatever the merger raises — including, as in Java, the
    /// `NullPointerException` of a call after the merge answered
    /// `NO_MORE_DOCS`, which steps the exhausted merger again.
    fn next_doc(&mut self) -> Result<i32> {
        match self.doc_id_merger.next_sub()? {
            Some(sub) => {
                self.doc = sub.mapped_doc_id();
                self.positioned = true;
                Ok(self.doc)
            }
            None => {
                // `current == null`: `doc_id` reports -1 from here on, as
                // Java's does.
                self.positioned = false;
                self.doc = NO_MORE_DOCS;
                Ok(NO_MORE_DOCS)
            }
        }
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::unsupported_operation(
            "MappingMultiPostingsEnum does not support advance".to_string(),
        ))
    }

    /// Equivalent to `MappingMultiPostingsEnum.cost()`: the sum over the subs
    /// of the current term. A sub with no postings contributes nothing, where
    /// Java would dereference `null`.
    fn cost(&self) -> i64 {
        self.doc_id_merger
            .subs()
            .iter()
            .filter_map(|sub| sub.postings.as_ref())
            .map(|postings| postings.cost())
            .sum()
    }
}

impl std::fmt::Display for MappingMultiPostingsEnum {
    /// `org.apache.lucene.index.MappingMultiPostingsEnum` declares no `toString()` in Lucene 10.5.0, so
    /// `java.lang.Object`'s applies: the binary class name, `@`, and the
    /// identity hash. See
    /// [`java_object_to_string`](crate::index::index_reader::java_object_to_string)
    /// for why only the hexadecimal suffix diverges.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.MappingMultiPostingsEnum",
            self,
        ))
    }
}

impl PostingsEnum for MappingMultiPostingsEnum {
    fn freq(&self) -> Result<i32> {
        self.current()?
            .postings
            .as_ref()
            .ok_or_else(|| null_sub_postings("freq()", "this.current.postings"))?
            .freq()
    }

    /// Equivalent to `MappingMultiPostingsEnum.nextPosition()`
    /// (`MappingMultiPostingsEnum.java:108-128`), whose two
    /// `CorruptIndexException`s carry Java's messages — the first one's
    /// unbalanced quote included — and the sub's postings as the resource.
    fn next_position(&mut self) -> Result<i32> {
        let field = self.field.clone();
        let sub = self.current_mut()?;
        let mapped_doc_id = sub.mapped_doc_id;
        let postings = sub
            .postings
            .as_mut()
            .ok_or_else(|| null_sub_postings("nextPosition()", "this.current.postings"))?;
        let pos = postings.next_position()?;
        if pos < 0 {
            return Err(LuceneError::corrupt_index(
                format!("position={pos} is negative, field=\"{field} doc={mapped_doc_id}"),
                postings.to_string(),
            ));
        } else if pos > MAX_POSITION {
            return Err(LuceneError::corrupt_index(
                format!(
                    "position={pos} is too large (> IndexWriter.MAX_POSITION={MAX_POSITION}), \
                     field=\"{field}\" doc={mapped_doc_id}"
                ),
                postings.to_string(),
            ));
        }
        Ok(pos)
    }

    fn start_offset(&self) -> i32 {
        match self.current() {
            Ok(MappingPostingsSub {
                postings: Some(postings),
                ..
            }) => postings.start_offset(),
            _ => -1,
        }
    }

    fn end_offset(&self) -> i32 {
        match self.current() {
            Ok(MappingPostingsSub {
                postings: Some(postings),
                ..
            }) => postings.end_offset(),
            _ => -1,
        }
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        self.current()?
            .postings
            .as_ref()
            .ok_or_else(|| null_sub_postings("getPayload()", "this.current.postings"))?
            .get_payload()
    }

    /// This enum answers with itself: it is the `true` arm of Java's
    /// `reuse instanceof MappingMultiPostingsEnum postings`
    /// (`MappedMultiFields.java:119`).
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::LuceneErrorKind;

    use std::collections::HashMap;

    use crate::index::merge::{deletion_doc_map, identity_doc_map};
    use crate::index::multi_fields::{EnumWithSlice, MultiTermsEnum};
    use crate::index::{FieldInfos, ReaderSlice, SegmentInfo};
    use crate::store::{ByteBuffersDirectory, Directory};
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

    impl std::fmt::Display for VecPostings {
        /// Test double; it renders under its own name. `Display` is required
        /// because `TermsEnum`/`PostingsEnum` carry it as a supertrait, standing in
        /// for the `toString()` every Java enumeration inherits from `Object`.
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("VecPostings")
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
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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
            None,
            needs_index_sort,
            None,
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

    /// The merged postings of one term, as `MultiTermsEnum.postings` hands
    /// them out: a [`MultiPostingsEnum`] reset on `subs`.
    fn merged(subs: Vec<EnumWithSlice>) -> Box<dyn PostingsEnum> {
        let parent = MultiTermsEnum::new(&[]).unwrap();
        let mut postings = MultiPostingsEnum::new(&parent, subs.len());
        postings.reset(subs);
        Box::new(postings)
    }

    /// `new MappingMultiPostingsEnum("body", mergeState).reset(merged(subs))`.
    fn mapping(
        state: Arc<MergeState>,
        subs: Vec<EnumWithSlice>,
    ) -> Result<MappingMultiPostingsEnum> {
        let mut postings = MappingMultiPostingsEnum::new("body", state)?;
        postings.reset(merged(subs))?;
        Ok(postings)
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
        let mut postings = mapping(state, subs).unwrap();
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
        let mut postings = mapping(state, subs).unwrap();
        assert_eq!(drain(&mut postings), vec![0, 1, 2, 3]);
    }

    #[test]
    fn interleaves_by_mapped_doc_id_when_the_merged_segment_is_sorted() {
        // The merged segment interleaves the two sources: segment 0 owns the
        // even merged doc IDs, segment 1 the odd ones.
        let state = merge_state(
            vec![
                Arc::new(|doc: i32| doc * 2) as DocMap,
                Arc::new(|doc: i32| doc * 2 + 1) as DocMap,
            ],
            vec![3, 3],
            true,
        );
        let subs = vec![
            EnumWithSlice::new(VecPostings::boxed(&[0, 1, 2]), ReaderSlice::new(0, 3, 0)),
            EnumWithSlice::new(VecPostings::boxed(&[0, 1, 2]), ReaderSlice::new(3, 3, 1)),
        ];
        let mut postings = mapping(state, subs).unwrap();
        assert_eq!(drain(&mut postings), vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn doc_id_is_minus_one_before_the_first_step_and_after_the_last() {
        let state = identity_merge_state();
        let subs = vec![EnumWithSlice::new(
            VecPostings::boxed(&[1]),
            ReaderSlice::new(0, 3, 0),
        )];
        let mut postings = mapping(state, subs).unwrap();
        assert_eq!(postings.doc_id(), -1);
        assert_eq!(postings.next_doc().unwrap(), 1);
        assert_eq!(postings.doc_id(), 1);
        assert_eq!(postings.next_doc().unwrap(), NO_MORE_DOCS);
        assert_eq!(
            postings.doc_id(),
            -1,
            "Java's docID() reports -1 once no sub is current (MappingMultiPostingsEnum.java:84-91)"
        );
        // Java's `nextDoc()` steps the exhausted merger again, which
        // dereferences its `null` current sub (`DocIDMerger.java:123`, `DocIDMerger.java:188`).
        let err = postings.next_doc().unwrap_err();
        assert!(
            matches!(
                err.kind(),
                LuceneErrorKind::Unchecked {
                    class: "java.lang.NullPointerException",
                    ..
                }
            ) && err.to_string().contains("because \"this.current\" is null"),
            "unexpected error: {err:?}"
        );
    }

    /// The per-document methods read `current.postings`; with `null` there,
    /// the JDK names that expression. Measured against
    /// `lucene-core-10.5.0.jar` (`NpeTextOracle`, which sets `current` to a
    /// sub whose `postings` is `null` by reflection).
    #[test]
    fn per_document_methods_report_javas_npe_for_a_null_current_postings() {
        let npe = |method: &str| {
            LuceneError::null_pointer_with_message(format!(
                "Cannot invoke \"org.apache.lucene.index.PostingsEnum.{method}\" because \"this.current.postings\" is null"
            ))
            .to_string()
        };
        let state = identity_merge_state();
        let subs = vec![EnumWithSlice::new(
            VecPostings::boxed(&[1]),
            ReaderSlice::new(0, 3, 0),
        )];
        let mut postings = mapping(state, subs).unwrap();
        assert_eq!(postings.next_doc().unwrap(), 1);
        postings
            .doc_id_merger
            .current_sub_mut()
            .expect("a current sub")
            .postings = None;
        assert_eq!(postings.freq().unwrap_err().to_string(), npe("freq()"));
        assert_eq!(
            postings.next_position().unwrap_err().to_string(),
            npe("nextPosition()")
        );
        assert_eq!(
            postings.get_payload().unwrap_err().to_string(),
            npe("getPayload()")
        );
    }

    #[test]
    fn advance_is_unsupported() {
        let state = identity_merge_state();
        let subs = vec![EnumWithSlice::new(
            VecPostings::boxed(&[0]),
            ReaderSlice::new(0, 3, 0),
        )];
        let mut postings = mapping(state, subs).unwrap();
        assert!(matches!(
            postings.advance(1),
            Err(LuceneError {
                kind: LuceneErrorKind::UnsupportedOperation { .. },
                ..
            })
        ));
    }

    #[test]
    fn cost_is_the_sum_of_the_sub_postings() {
        let state = identity_merge_state();
        let subs = vec![
            EnumWithSlice::new(VecPostings::boxed(&[0, 1]), ReaderSlice::new(0, 3, 0)),
            EnumWithSlice::new(VecPostings::boxed(&[0, 1, 2]), ReaderSlice::new(3, 3, 1)),
        ];
        let postings = mapping(state, subs).unwrap();
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
        let mut postings = mapping(state, subs).unwrap();

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
        let mut postings = mapping(state, subs).unwrap();
        // `current.postings` with `current == null`: Java's
        // `NullPointerException` (`MappingMultiPostingsEnum.java:81, 110`).
        let is_npe = |result: Result<i32>| {
            matches!(result, Err(LuceneError {
                kind: LuceneErrorKind::Unchecked { class, .. },
                ..
            }) if class == "java.lang.NullPointerException")
        };
        assert!(is_npe(postings.freq()));
        assert_eq!(postings.start_offset(), -1);
        assert_eq!(postings.end_offset(), -1);
        assert!(is_npe(postings.next_position()));
    }

    #[test]
    fn a_negative_position_is_reported_as_a_corrupt_index() {
        let state = identity_merge_state();
        let subs = vec![EnumWithSlice::new(
            VecPostings::with_details(&[0], 1, &[-5], None),
            ReaderSlice::new(0, 3, 0),
        )];
        let mut postings = mapping(state, subs).unwrap();
        assert_eq!(postings.next_doc().unwrap(), 0);
        let err = postings.next_position().unwrap_err();
        assert!(
            matches!(&err, LuceneError { kind: LuceneErrorKind::CorruptIndex(m), .. } if m.contains("is negative")),
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
        let mut postings = mapping(state, subs).unwrap();
        assert_eq!(postings.next_doc().unwrap(), 0);
        let err = postings.next_position().unwrap_err();
        assert!(
            matches!(&err, LuceneError { kind: LuceneErrorKind::CorruptIndex(m), .. } if m.contains("too large")),
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
        let err = mapping(state, subs).unwrap_err();
        assert!(
            matches!(&err, LuceneError { kind: LuceneErrorKind::IllegalArgument { message: m, .. }, .. } if m == "Index 2 out of bounds for length 2"),
            "`allSubs[2]` throws Java's ArrayIndexOutOfBoundsException: {err:?}"
        );
    }

    #[test]
    fn reset_re_points_the_enum_at_another_term() {
        let state = identity_merge_state();
        let first = vec![EnumWithSlice::new(
            VecPostings::boxed(&[0, 1]),
            ReaderSlice::new(0, 3, 0),
        )];
        let mut postings = mapping(state, first).unwrap();
        assert_eq!(drain(&mut postings), vec![0, 1]);

        let second = vec![EnumWithSlice::new(
            VecPostings::boxed(&[2]),
            ReaderSlice::new(3, 3, 1),
        )];
        postings.reset(merged(second)).unwrap();
        assert_eq!(
            postings.doc_id(),
            -1,
            "the exhausted cursor stays unpositioned: Java's reset leaves `current` null"
        );
        assert_eq!(drain(&mut postings), vec![5]);
        assert_eq!(postings.field(), "body");
    }

    /// Over postings with no sub, `reset` leaves the merger's `current`
    /// `null`, so Java's first `nextDoc()` throws `NullPointerException` from
    /// `DocIDMerger.next()` (`DocIDMerger.java:123`, `DocIDMerger.java:188`);
    /// `cost()` sums nothing.
    #[test]
    fn a_term_present_in_no_source_segment_reports_javas_npe() {
        let state = identity_merge_state();
        let mut postings = mapping(state, Vec::new()).unwrap();
        assert_eq!(postings.cost(), 0);
        let err = postings.next_doc().unwrap_err();
        assert!(
            matches!(
                err.kind(),
                LuceneErrorKind::Unchecked {
                    class: "java.lang.NullPointerException",
                    ..
                }
            ) && err.to_string().contains("because \"this.current\" is null"),
            "unexpected error: {err:?}"
        );
    }

    fn identity<T: ?Sized>(object: &T) -> *const () {
        object as *const T as *const ()
    }

    /// The identities of the enums in the live slots of a merged postings.
    fn slot_identities(postings: &mut Box<dyn PostingsEnum>) -> Vec<Option<*const ()>> {
        postings
            .as_any_mut()
            .and_then(|any| any.downcast_mut::<MultiPostingsEnum>())
            .expect("a MultiPostingsEnum")
            .get_subs()
            .iter()
            .map(|slot| slot.postings_enum.as_deref().map(identity))
            .collect()
    }

    /// Java's subs keep aliasing the `MultiPostingsEnum` slots their postings
    /// came from, so the enum offered back to `MultiTermsEnum.postings` as
    /// `reuse` still holds every sub-enum in place. Here `reset` moves the
    /// postings into the subs, and taking the enum back must return them to
    /// their slots.
    #[test]
    fn the_multi_postings_enum_taken_back_holds_its_sub_enums_again() {
        let state = identity_merge_state();
        let mut multi = merged(vec![
            EnumWithSlice::new(VecPostings::boxed(&[0, 2]), ReaderSlice::new(0, 3, 0)),
            EnumWithSlice::new(VecPostings::boxed(&[1]), ReaderSlice::new(3, 3, 1)),
        ]);
        let multi_identity = identity(multi.as_ref());
        let before = slot_identities(&mut multi);
        assert!(before.iter().all(Option::is_some));

        let mut postings = MappingMultiPostingsEnum::new("body", state).unwrap();
        assert!(postings.take_multi_docs_and_positions_enum().is_none());
        postings.reset(multi).unwrap();
        assert_eq!(drain(&mut postings), vec![0, 2, 4]);

        let mut taken = postings
            .take_multi_docs_and_positions_enum()
            .expect("reset stored the MultiPostingsEnum");
        assert_eq!(identity(taken.as_ref()), multi_identity);
        assert_eq!(slot_identities(&mut taken), before);
    }

    #[test]
    fn reset_rejects_an_enum_that_is_not_a_multi_postings_enum() {
        let mut postings = MappingMultiPostingsEnum::new("body", identity_merge_state()).unwrap();
        let err = match postings.reset(VecPostings::boxed(&[0])) {
            Err(err) => err,
            Ok(_) => panic!("Java's cast throws ClassCastException"),
        };
        assert!(
            err.to_string()
                .contains("cannot be cast to class org.apache.lucene.index.MultiPostingsEnum"),
            "{err:?}"
        );
    }
}
