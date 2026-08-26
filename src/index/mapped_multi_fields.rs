//! `MappedMultiFields` ported from `org.apache.lucene.index`.
//!
//! This is the [`Fields`] view a segment merge reads from. It layers two
//! translations on top of the source segments:
//!
//! 1. [`MultiFields`] / [`MultiTerms`] / [`MultiTermsEnum`] merge the source
//!    segments' term dictionaries into one sorted, deduplicated stream;
//! 2. [`MappingMultiPostingsEnum`] re-maps each term's postings out of the
//!    composite doc-ID space and into the merged segment's, dropping deleted
//!    documents and applying any index sort.
//!
//! The per-term statistics (`doc_freq`, `total_term_freq`, and the field-level
//! sums) are deliberately unavailable: during a merge they would have to be
//! recomputed after deletions, and the codec that consumes this view derives
//! them from the postings it actually writes. Lucene enforces that by throwing
//! `UnsupportedOperationException`; see the note on the module's private
//! `MappedMultiTerms` for how this port reports the same thing under an
//! infallible signature.
//!
//! # Reference
//!
//! - `org.apache.lucene.index.MappedMultiFields`
//! - `org.apache.lucene.index.FilterLeafReader.FilterFields` /
//!   `FilterTerms` / `FilterTermsEnum` (the delegation bases it extends)

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::mapping_multi_postings_enum::MappingMultiPostingsEnum;
use crate::index::merge::MergeState;
use crate::index::multi_fields::{MultiFields, MultiTerms, MultiTermsEnum, SlowImpactsEnum};
use crate::index::postings_enum::{ImpactsEnum, PostingsEnum};
use crate::index::terms::{EmptyTermsEnum, Fields, SeekStatus, TermState, Terms, TermsEnum};
use crate::util::attribute::AttributeSource;
use crate::util::automaton::CompiledAutomaton;
use crate::util::BytesRef;

/// A [`Fields`] view that merges several segments' fields into one and maps
/// around deleted documents. Used for merging.
///
/// Equivalent to `org.apache.lucene.index.MappedMultiFields`.
///
/// **Deliberate divergence**: Java derives this class from
/// `FilterLeafReader.FilterFields`, a generic delegating base. Rucene has no
/// `FilterLeafReader` yet, and this type only ever wraps a [`MultiFields`] —
/// its constructor takes nothing else — so it holds one concretely and
/// delegates the two inherited methods directly. That also removes the two
/// downcasts (`(MultiTerms) in.terms(f)` and `(MultiTermsEnum) in.iterator()`)
/// that the Java class needs to get back to the concrete types.
pub struct MappedMultiFields {
    merge_state: Arc<MergeState>,
    inner: MultiFields,
}

impl std::fmt::Debug for MappedMultiFields {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MappedMultiFields")
            .field("num_subs", &self.inner.get_sub_fields().len())
            .field("merge_state", &self.merge_state)
            .finish_non_exhaustive()
    }
}

impl MappedMultiFields {
    /// Creates a `MappedMultiFields` for merging, over `merge_state` and the
    /// merged view of terms in `multi_fields`.
    ///
    /// Equivalent to `MappedMultiFields(MergeState, MultiFields)`.
    ///
    /// `merge_state` is shared with every terms enum and postings enum this
    /// view hands out, which is why it arrives as an [`Arc`]: Java passes the
    /// same `MergeState` object by reference for exactly the same reason.
    pub fn new(merge_state: Arc<MergeState>, multi_fields: MultiFields) -> Self {
        Self {
            merge_state,
            inner: multi_fields,
        }
    }

    /// Returns the merged, unmapped view this instance wraps.
    pub fn inner(&self) -> &MultiFields {
        &self.inner
    }
}

impl Fields for MappedMultiFields {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        self.inner.iterator()
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        Ok(self.inner.multi_terms(field)?.map(|terms| {
            Box::new(MappedMultiTerms::new(
                field,
                Arc::clone(&self.merge_state),
                terms,
            )) as Box<dyn Terms>
        }))
    }

    fn size(&self) -> i32 {
        self.inner.size()
    }
}

/// The [`Terms`] of one field, merged across the source segments and mapped
/// into the merged segment's doc-ID space.
///
/// Equivalent to `MappedMultiFields.MappedMultiTerms`.
///
/// **Deliberate divergence**: Java throws `UnsupportedOperationException` from
/// [`Terms::size`], [`Terms::sum_total_term_freq`], [`Terms::sum_doc_freq`] and
/// [`Terms::doc_count`], so that a codec accidentally consulting them during a
/// merge fails loudly rather than writing statistics that ignore deletions.
/// Rucene declares those four methods infallible, so they return `-1` instead —
/// the value Lucene itself documents on [`Terms::size`] for "this measure is not
/// available", and one every codec must already tolerate. The intent is
/// unchanged: the numbers are not available here and must not be used.
struct MappedMultiTerms {
    field: String,
    merge_state: Arc<MergeState>,
    /// The unmapped merged view. Held as the shared handle `MultiFields`
    /// memoises per field, so wrapping does not force a rebuild.
    inner: Arc<MultiTerms>,
}

impl MappedMultiTerms {
    fn new(field: &str, merge_state: Arc<MergeState>, inner: Arc<MultiTerms>) -> Self {
        Self {
            field: field.to_string(),
            merge_state,
            inner,
        }
    }

    /// Wraps a merged terms enum, or returns the empty enum when the merge
    /// produced no terms at all (LUCENE-6826).
    fn wrap(&self, merged: Option<MultiTermsEnum>) -> Box<dyn TermsEnum> {
        match merged {
            None => Box::new(EmptyTermsEnum::new()) as Box<dyn TermsEnum>,
            Some(enumerator) => Box::new(MappedMultiTermsEnum::new(
                &self.field,
                Arc::clone(&self.merge_state),
                enumerator,
            )) as Box<dyn TermsEnum>,
        }
    }
}

impl Terms for MappedMultiTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(self.wrap(self.inner.multi_iterator()?))
    }

    fn intersect(
        &self,
        compiled: &CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<Box<dyn TermsEnum>> {
        // **Deliberate divergence**: Java inherits `intersect` from
        // `FilterTerms`, which delegates straight to the wrapped `MultiTerms`
        // and therefore returns postings in the *composite* doc-ID space —
        // unmapped, and wrong for a merge. Wrapping the filtered enum the same
        // way `iterator()` does keeps every entry point of this type in the
        // merged doc-ID space.
        Ok(self.wrap(self.inner.multi_intersect(compiled, start_term)?))
    }

    fn size(&self) -> i64 {
        -1
    }

    fn sum_total_term_freq(&self) -> i64 {
        -1
    }

    fn sum_doc_freq(&self) -> i64 {
        -1
    }

    fn doc_count(&self) -> i32 {
        -1
    }

    fn has_freqs(&self) -> bool {
        self.inner.has_freqs()
    }

    fn has_offsets(&self) -> bool {
        self.inner.has_offsets()
    }

    fn has_positions(&self) -> bool {
        self.inner.has_positions()
    }

    fn has_payloads(&self) -> bool {
        self.inner.has_payloads()
    }

    fn min(&self) -> Result<Option<BytesRef>> {
        self.inner.min()
    }

    fn max(&self) -> Result<Option<BytesRef>> {
        self.inner.max()
    }
}

/// The merged [`TermsEnum`] of one field, handing out postings already mapped
/// into the merged segment's doc-ID space.
///
/// Equivalent to `MappedMultiFields.MappedMultiTermsEnum`.
struct MappedMultiTermsEnum {
    field: String,
    merge_state: Arc<MergeState>,
    inner: MultiTermsEnum,
}

impl MappedMultiTermsEnum {
    fn new(field: &str, merge_state: Arc<MergeState>, inner: MultiTermsEnum) -> Self {
        Self {
            field: field.to_string(),
            merge_state,
            inner,
        }
    }

    /// Builds the mapped postings for the current term.
    fn mapped_postings(&mut self, flags: i32) -> Result<Box<dyn PostingsEnum>> {
        let merged = self.inner.multi_postings(flags)?;
        Ok(Box::new(MappingMultiPostingsEnum::new(
            &self.field,
            Arc::clone(&self.merge_state),
            merged,
        )?))
    }

    fn unsupported(&self, what: &str) -> LuceneError {
        LuceneError::UnsupportedOperation(format!(
            "{what} is not available while merging field '{}': it would ignore deleted \
             documents; derive it from the postings instead",
            self.field
        ))
    }
}

impl TermsEnum for MappedMultiTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        self.inner.attributes()
    }

    fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
        self.inner.seek_exact(text)
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        self.inner.seek_ceil(text)
    }

    fn seek_ord(&mut self, ord: i64) -> Result<()> {
        self.inner.seek_ord(ord)
    }

    fn seek_term_state(&mut self, text: &BytesRef, state: &dyn TermState) -> Result<()> {
        self.inner.seek_term_state(text, state)
    }

    fn term(&self) -> Result<BytesRef> {
        self.inner.term()
    }

    fn ord(&self) -> Result<i64> {
        self.inner.ord()
    }

    fn doc_freq(&self) -> Result<i32> {
        Err(self.unsupported("doc_freq"))
    }

    fn total_term_freq(&self) -> Result<i64> {
        Err(self.unsupported("total_term_freq"))
    }

    fn postings(
        &mut self,
        _reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        // **Deliberate divergence**: Java reuses the previous
        // `MappingMultiPostingsEnum` when the caller hands one back for the
        // same field, saving an allocation per term. Rucene's `reuse` is an
        // opaque `Box<dyn PostingsEnum>` that cannot be downcast, and
        // `MultiTermsEnum` does not thread reuse to its own sub-enums either,
        // so a fresh instance is built per term. Callers that want the reuse
        // can keep a `MappingMultiPostingsEnum` themselves and call
        // `MappingMultiPostingsEnum::reset` with
        // `MultiTermsEnum::multi_postings`.
        self.mapped_postings(flags)
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        // **Deliberate divergence**: Java inherits `impacts` from
        // `FilterTermsEnum`, which delegates to the wrapped `MultiTermsEnum`
        // and so reports *unmapped* doc IDs. Wrapping the mapped postings in a
        // `SlowImpactsEnum` — which is exactly what `MultiTermsEnum::impacts`
        // does with its own postings — keeps the doc-ID space consistent.
        Ok(Box::new(SlowImpactsEnum::new(self.mapped_postings(flags)?)))
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        self.inner.term_state()
    }

    fn prefer_seek_exact(&self) -> bool {
        self.inner.prefer_seek_exact()
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        self.inner.next()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::{BTreeMap, HashMap};

    use crate::index::merge::{deletion_doc_map, identity_doc_map, DocMap};
    use crate::index::{FieldInfos, ReaderSlice, SegmentInfo};
    use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
    use crate::store::{Directory, RamDirectory};
    use crate::util::automaton::Automaton;
    use crate::util::{Bits, StringHelper, Version};

    // -----------------------------------------------------------------------
    // Fixtures: an in-memory Fields / Terms / TermsEnum / PostingsEnum
    // -----------------------------------------------------------------------

    /// Postings over an explicit list of local doc IDs.
    struct VecPostings {
        docs: Vec<i32>,
        pos: i32,
    }

    impl VecPostings {
        fn boxed(docs: &[i32]) -> Box<dyn PostingsEnum> {
            Box::new(Self {
                docs: docs.to_vec(),
                pos: -1,
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
            Ok(1)
        }
        fn next_position(&mut self) -> Result<i32> {
            Ok(-1)
        }
        fn start_offset(&self) -> i32 {
            -1
        }
        fn end_offset(&self) -> i32 {
            -1
        }
        fn get_payload(&self) -> Result<Option<&[u8]>> {
            Ok(None)
        }
    }

    /// A term with its statistics and its local postings list.
    type Entry = (BytesRef, i32, i64, Vec<i32>);

    struct VecTermsEnum {
        entries: Vec<Entry>,
        pos: i32,
        atts: AttributeSource,
    }

    impl VecTermsEnum {
        fn new(entries: Vec<Entry>) -> Self {
            Self {
                entries,
                pos: -1,
                atts: AttributeSource::new(),
            }
        }
    }

    impl TermsEnum for VecTermsEnum {
        fn attributes(&mut self) -> &mut AttributeSource {
            &mut self.atts
        }

        fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
            match self.entries.iter().position(|(t, _, _, _)| t >= text) {
                Some(i) => {
                    self.pos = i as i32;
                    if self.entries[i].0 == *text {
                        Ok(SeekStatus::FOUND)
                    } else {
                        Ok(SeekStatus::NOT_FOUND)
                    }
                }
                None => {
                    self.pos = self.entries.len() as i32;
                    Ok(SeekStatus::END)
                }
            }
        }

        fn seek_ord(&mut self, ord: i64) -> Result<()> {
            self.pos = ord as i32;
            Ok(())
        }

        fn term(&self) -> Result<BytesRef> {
            Ok(self.entries[self.pos as usize].0.clone())
        }

        fn ord(&self) -> Result<i64> {
            Ok(self.pos as i64)
        }

        fn doc_freq(&self) -> Result<i32> {
            Ok(self.entries[self.pos as usize].1)
        }

        fn total_term_freq(&self) -> Result<i64> {
            Ok(self.entries[self.pos as usize].2)
        }

        fn postings(
            &mut self,
            _reuse: Option<Box<dyn PostingsEnum>>,
            _flags: i32,
        ) -> Result<Box<dyn PostingsEnum>> {
            Ok(VecPostings::boxed(&self.entries[self.pos as usize].3))
        }

        fn impacts(&mut self, _flags: i32) -> Result<Box<dyn ImpactsEnum>> {
            Err(LuceneError::UnsupportedOperation("not used".to_string()))
        }

        fn next(&mut self) -> Result<Option<BytesRef>> {
            self.pos += 1;
            if (self.pos as usize) >= self.entries.len() {
                self.pos = self.entries.len() as i32;
                Ok(None)
            } else {
                Ok(Some(self.entries[self.pos as usize].0.clone()))
            }
        }
    }

    #[derive(Clone)]
    struct VecTerms {
        entries: Vec<Entry>,
        has_freqs: bool,
        has_offsets: bool,
        has_positions: bool,
        has_payloads: bool,
    }

    impl VecTerms {
        fn new(entries: Vec<Entry>) -> Self {
            Self {
                entries,
                has_freqs: true,
                has_offsets: false,
                has_positions: true,
                has_payloads: false,
            }
        }
    }

    impl Terms for VecTerms {
        fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
            Ok(Box::new(VecTermsEnum::new(self.entries.clone())))
        }

        fn intersect(
            &self,
            _compiled: &CompiledAutomaton,
            _start_term: Option<&BytesRef>,
        ) -> Result<Box<dyn TermsEnum>> {
            // The automaton is irrelevant here: the point of the test that uses
            // this is that whatever comes back is still wrapped and mapped.
            self.iterator()
        }

        fn size(&self) -> i64 {
            self.entries.len() as i64
        }
        fn sum_total_term_freq(&self) -> i64 {
            self.entries.iter().map(|(_, _, ttf, _)| *ttf).sum()
        }
        fn sum_doc_freq(&self) -> i64 {
            self.entries.iter().map(|(_, df, _, _)| *df as i64).sum()
        }
        fn doc_count(&self) -> i32 {
            self.entries
                .iter()
                .flat_map(|(_, _, _, docs)| docs.iter().copied())
                .collect::<std::collections::BTreeSet<_>>()
                .len() as i32
        }
        fn has_freqs(&self) -> bool {
            self.has_freqs
        }
        fn has_offsets(&self) -> bool {
            self.has_offsets
        }
        fn has_positions(&self) -> bool {
            self.has_positions
        }
        fn has_payloads(&self) -> bool {
            self.has_payloads
        }
    }

    struct VecFields {
        fields: BTreeMap<String, VecTerms>,
    }

    impl VecFields {
        fn boxed<I>(iter: I) -> Box<dyn Fields>
        where
            I: IntoIterator<Item = (&'static str, VecTerms)>,
        {
            Box::new(Self {
                fields: iter
                    .into_iter()
                    .map(|(name, terms)| (name.to_string(), terms))
                    .collect(),
            })
        }
    }

    impl Fields for VecFields {
        fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
            Box::new(self.fields.keys().cloned().collect::<Vec<_>>().into_iter())
        }

        fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
            Ok(self
                .fields
                .get(field)
                .map(|t| Box::new(t.clone()) as Box<dyn Terms>))
        }

        fn size(&self) -> i32 {
            self.fields.len() as i32
        }
    }

    fn term(text: &str) -> BytesRef {
        BytesRef::new(text.as_bytes().to_vec())
    }

    fn entry(text: &str, docs: &[i32]) -> Entry {
        (
            term(text),
            docs.len() as i32,
            docs.len() as i64,
            docs.to_vec(),
        )
    }

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

    fn segment_info(max_doc: i32) -> SegmentInfo {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        SegmentInfo::new_without_codec(
            dir,
            Version::LUCENE_10_5_0,
            Some(Version::LUCENE_10_5_0),
            "_merged".to_string(),
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

    fn merge_state(doc_maps: Vec<DocMap>, max_docs: Vec<i32>) -> Arc<MergeState> {
        let count = doc_maps.len();
        Arc::new(MergeState::new(
            doc_maps,
            segment_info(max_docs.iter().sum()),
            FieldInfos::empty(),
            (0..count).map(|_| None).collect(),
            (0..count).map(|_| None).collect(),
            (0..count).map(|_| None).collect(),
            (0..count).map(|_| None).collect(),
            (0..count).map(|_| FieldInfos::empty()).collect(),
            (0..count).map(|_| None).collect(),
            (0..count).map(|_| None).collect(),
            (0..count).map(|_| None).collect(),
            (0..count).map(|_| None).collect(),
            max_docs,
            false,
        ))
    }

    /// The scenario shared by most tests below.
    ///
    /// Two source segments:
    ///
    /// * segment 0 has 3 documents, of which local doc 1 is deleted, and holds
    ///   `apple` on docs 0 and 2 and `pear` on the deleted doc 1;
    /// * segment 1 has 2 documents, none deleted, and holds `apple` on doc 1
    ///   and `plum` on doc 0. It also has a `title` field.
    ///
    /// The merged segment therefore numbers segment 0's surviving documents 0
    /// and 1, and segment 1's documents 2 and 3. The composite (unmapped) view
    /// would instead number them 0, 2, 3 and 4, so any test asserting a merged
    /// doc ID also proves the mapping ran.
    fn scenario() -> (Arc<MergeState>, MappedMultiFields) {
        let seg0 = VecFields::boxed([(
            "body",
            VecTerms::new(vec![entry("apple", &[0, 2]), entry("pear", &[1])]),
        )]);
        let seg1 = VecFields::boxed([
            (
                "body",
                VecTerms::new(vec![entry("apple", &[1]), entry("plum", &[0])]),
            ),
            ("title", VecTerms::new(vec![entry("rust", &[0])])),
        ]);
        let multi = MultiFields::new(
            vec![seg0, seg1],
            vec![ReaderSlice::new(0, 3, 0), ReaderSlice::new(3, 2, 1)],
        );
        let live = Box::new(LiveBits {
            len: 3,
            deleted: vec![1],
        }) as Box<dyn Bits>;
        let state = merge_state(
            vec![deletion_doc_map(3, live, 0), identity_doc_map(2, 2)],
            vec![3, 2],
        );
        let fields = MappedMultiFields::new(Arc::clone(&state), multi);
        (state, fields)
    }

    fn drain(postings: &mut dyn PostingsEnum) -> Vec<i32> {
        let mut docs = Vec::new();
        loop {
            let doc = postings.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                return docs;
            }
            docs.push(doc);
        }
    }

    /// Positions a fresh terms enum on `text` and returns its mapped postings.
    fn postings_for(fields: &MappedMultiFields, field: &str, text: &str) -> Vec<i32> {
        let terms = fields.terms(field).unwrap().unwrap();
        let mut enumerator = terms.iterator().unwrap();
        assert!(
            enumerator.seek_exact(&term(text)).unwrap(),
            "term '{text}' must exist"
        );
        let mut postings = enumerator.postings(None, 0).unwrap();
        drain(postings.as_mut())
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[test]
    fn postings_are_mapped_into_the_merged_doc_id_space() {
        let (_state, fields) = scenario();
        assert_eq!(
            postings_for(&fields, "body", "apple"),
            vec![0, 1, 3],
            "segment 0's docs 0 and 2 become 0 and 1; segment 1's doc 1 becomes 3"
        );
    }

    #[test]
    fn the_unmapped_view_underneath_still_reports_composite_doc_ids() {
        // Guards the test above from passing by accident: the merged view this
        // type wraps numbers the same postings differently.
        let (_state, fields) = scenario();
        let terms = fields.inner().terms("body").unwrap().unwrap();
        let mut enumerator = terms.iterator().unwrap();
        assert!(enumerator.seek_exact(&term("apple")).unwrap());
        let mut postings = enumerator.postings(None, 0).unwrap();
        assert_eq!(drain(postings.as_mut()), vec![0, 2, 4]);
    }

    #[test]
    fn postings_drop_the_documents_the_merge_deletes() {
        let (_state, fields) = scenario();
        assert!(
            postings_for(&fields, "body", "pear").is_empty(),
            "'pear' only occurs on a deleted document"
        );
    }

    #[test]
    fn postings_of_a_term_held_by_a_single_segment_are_still_mapped() {
        let (_state, fields) = scenario();
        assert_eq!(postings_for(&fields, "body", "plum"), vec![2]);
    }

    #[test]
    fn term_iteration_merges_and_deduplicates_the_source_segments() {
        let (_state, fields) = scenario();
        let terms = fields.terms("body").unwrap().unwrap();
        let mut enumerator = terms.iterator().unwrap();
        let mut seen = Vec::new();
        while let Some(t) = enumerator.next().unwrap() {
            seen.push(String::from_utf8(t.slice().to_vec()).unwrap());
        }
        assert_eq!(
            seen,
            vec!["apple".to_string(), "pear".to_string(), "plum".to_string()],
            "'apple' appears in both segments but is exposed once"
        );
    }

    #[test]
    fn field_names_are_the_union_across_the_source_segments() {
        let (_state, fields) = scenario();
        let names: Vec<String> = fields.iterator().collect();
        assert_eq!(names, vec!["body".to_string(), "title".to_string()]);
        assert_eq!(
            fields.size(),
            -1,
            "the merged field count is reported unknown"
        );
    }

    #[test]
    fn terms_are_none_for_a_field_no_segment_has() {
        let (_state, fields) = scenario();
        assert!(fields.terms("missing").unwrap().is_none());
    }

    #[test]
    fn a_field_held_by_one_segment_only_is_still_available() {
        let (_state, fields) = scenario();
        assert_eq!(postings_for(&fields, "title", "rust"), vec![2]);
    }

    #[test]
    fn field_level_statistics_are_unavailable_while_merging() {
        let (_state, fields) = scenario();
        let terms = fields.terms("body").unwrap().unwrap();
        // Java throws UnsupportedOperationException from all four; Rucene's
        // infallible signatures report the documented "unknown" sentinel.
        assert_eq!(terms.size(), -1);
        assert_eq!(terms.sum_total_term_freq(), -1);
        assert_eq!(terms.sum_doc_freq(), -1);
        assert_eq!(terms.doc_count(), -1);
    }

    #[test]
    fn per_term_statistics_are_rejected_while_merging() {
        let (_state, fields) = scenario();
        let terms = fields.terms("body").unwrap().unwrap();
        let mut enumerator = terms.iterator().unwrap();
        assert!(enumerator.seek_exact(&term("apple")).unwrap());
        assert!(matches!(
            enumerator.doc_freq(),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        assert!(matches!(
            enumerator.total_term_freq(),
            Err(LuceneError::UnsupportedOperation(_))
        ));
    }

    #[test]
    fn feature_flags_are_delegated_to_the_merged_terms() {
        let (_state, fields) = scenario();
        let terms = fields.terms("body").unwrap().unwrap();
        assert!(terms.has_freqs());
        assert!(terms.has_positions());
        assert!(!terms.has_offsets());
        assert!(!terms.has_payloads());
    }

    #[test]
    fn min_and_max_are_delegated_to_the_merged_terms() {
        let (_state, fields) = scenario();
        let terms = fields.terms("body").unwrap().unwrap();
        assert_eq!(
            terms.min().unwrap().map(|t| text_of(&t)),
            Some("apple".to_string())
        );
        assert_eq!(
            terms.max().unwrap().map(|t| text_of(&t)),
            Some("plum".to_string())
        );
    }

    fn text_of(bytes: &BytesRef) -> String {
        String::from_utf8(bytes.slice().to_vec()).unwrap()
    }

    #[test]
    fn seek_ceil_and_ord_are_delegated_to_the_merged_enum() {
        let (_state, fields) = scenario();
        let terms = fields.terms("body").unwrap().unwrap();
        let mut enumerator = terms.iterator().unwrap();
        assert_eq!(
            enumerator.seek_ceil(&term("peach")).unwrap(),
            SeekStatus::NOT_FOUND
        );
        assert_eq!(text_of(&enumerator.term().unwrap()), "pear");
        assert_eq!(enumerator.seek_ceil(&term("zzz")).unwrap(), SeekStatus::END);
    }

    #[test]
    fn a_merge_with_no_terms_at_all_yields_the_empty_terms_enum() {
        // LUCENE-6826: `MultiTermsEnum.reset` collapses to `TermsEnum.EMPTY`
        // when no sub contributed a term, and the mapped view must not wrap it.
        let seg0 = VecFields::boxed([("body", VecTerms::new(vec![]))]);
        let seg1 = VecFields::boxed([("body", VecTerms::new(vec![]))]);
        let multi = MultiFields::new(
            vec![seg0, seg1],
            vec![ReaderSlice::new(0, 2, 0), ReaderSlice::new(2, 2, 1)],
        );
        let state = merge_state(
            vec![identity_doc_map(2, 0), identity_doc_map(2, 2)],
            vec![2, 2],
        );
        let fields = MappedMultiFields::new(state, multi);
        let terms = fields.terms("body").unwrap().unwrap();
        let mut enumerator = terms.iterator().unwrap();
        assert!(enumerator.next().unwrap().is_none());
    }

    #[test]
    fn impacts_report_the_mapped_doc_ids() {
        let (_state, fields) = scenario();
        let terms = fields.terms("body").unwrap().unwrap();
        let mut enumerator = terms.iterator().unwrap();
        assert!(enumerator.seek_exact(&term("apple")).unwrap());
        let mut impacts = enumerator.impacts(0).unwrap();
        let mut docs = Vec::new();
        loop {
            let doc = impacts.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                break;
            }
            docs.push(doc);
        }
        assert_eq!(docs, vec![0, 1, 3]);
    }

    #[test]
    fn impacts_expose_a_single_trivial_level() {
        let (_state, fields) = scenario();
        let terms = fields.terms("body").unwrap().unwrap();
        let mut enumerator = terms.iterator().unwrap();
        assert!(enumerator.seek_exact(&term("apple")).unwrap());
        let mut impacts_enum = enumerator.impacts(0).unwrap();
        let impacts = impacts_enum.get_impacts().unwrap();
        assert_eq!(impacts.num_levels(), 1);
    }

    #[test]
    fn intersect_also_returns_mapped_postings() {
        let (_state, fields) = scenario();
        let automaton = Automaton::new();
        let compiled = CompiledAutomaton::new(automaton, true, false, true).unwrap();
        let terms = fields.terms("body").unwrap().unwrap();
        let mut enumerator = terms.intersect(&compiled, None).unwrap();
        assert!(enumerator.seek_exact(&term("apple")).unwrap());
        let mut postings = enumerator.postings(None, 0).unwrap();
        assert_eq!(
            drain(postings.as_mut()),
            vec![0, 1, 3],
            "Java's inherited FilterTerms.intersect would leak unmapped doc IDs here"
        );
    }

    #[test]
    fn every_call_hands_out_an_independent_postings_enum() {
        let (_state, fields) = scenario();
        let terms = fields.terms("body").unwrap().unwrap();
        let mut enumerator = terms.iterator().unwrap();
        assert!(enumerator.seek_exact(&term("apple")).unwrap());
        let mut first = enumerator.postings(None, 0).unwrap();
        assert_eq!(first.next_doc().unwrap(), 0);
        // Rucene never reuses the enum handed back as `reuse`, so the second
        // call starts from the beginning rather than resuming the first.
        let mut second = enumerator.postings(Some(first), 0).unwrap();
        assert_eq!(drain(second.as_mut()), vec![0, 1, 3]);
    }

    #[test]
    fn a_failed_seek_exact_does_not_drop_a_segment_from_the_mapped_enum() {
        // Regression guard for the LUCENE-2130 seek optimisation, reached
        // through `MappedMultiTermsEnum`, which delegates `seek_exact` straight
        // to the underlying `MultiTermsEnum`. Segment 0 has no `pear`, so its
        // exact seek fails and leaves it unpositioned; the following
        // `seek_exact("plum")` must still re-seek it.
        let seg0 = VecFields::boxed([(
            "body",
            VecTerms::new(vec![entry("apple", &[0]), entry("plum", &[2])]),
        )]);
        let seg1 = VecFields::boxed([(
            "body",
            VecTerms::new(vec![entry("pear", &[0]), entry("plum", &[1])]),
        )]);
        let multi = MultiFields::new(
            vec![seg0, seg1],
            vec![ReaderSlice::new(0, 3, 0), ReaderSlice::new(3, 2, 1)],
        );
        let live = Box::new(LiveBits {
            len: 3,
            deleted: vec![1],
        }) as Box<dyn Bits>;
        let state = merge_state(
            vec![deletion_doc_map(3, live, 0), identity_doc_map(2, 2)],
            vec![3, 2],
        );
        let fields = MappedMultiFields::new(Arc::clone(&state), multi);

        let terms = fields.terms("body").unwrap().unwrap();
        let mut enumerator = terms.iterator().unwrap();
        assert!(enumerator.seek_exact(&term("pear")).unwrap());
        assert!(enumerator.seek_exact(&term("plum")).unwrap());
        let mut postings = enumerator.postings(None, 0).unwrap();
        assert_eq!(
            drain(postings.as_mut()),
            vec![1, 3],
            "segment 0 was dropped by the seek optimisation"
        );
    }
}
