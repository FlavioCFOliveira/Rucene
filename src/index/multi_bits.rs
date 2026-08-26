//! `MultiBits` and `BitsSlice` ported from `org.apache.lucene.index`.
//!
//! Both types present a [`Bits`] view whose coordinate system is the composite
//! reader's *global* doc-ID space, translating each lookup into the local
//! doc-ID space of the sub-reader that owns it:
//!
//! * [`MultiBits`] concatenates one [`Bits`] per sub-reader, resolving the
//!   owner with a binary search over the doc-base table;
//! * [`BitsSlice`] does the opposite — it narrows a global [`Bits`] down to the
//!   window covered by a single [`ReaderSlice`].
//!
//! Both are deliberately slow paths. Per-lookup owner resolution costs a binary
//! search, so code that can iterate the leaves and work in local doc-ID space
//! should do that instead; these exist for the cases where a single flat view
//! is the only option.

#![deny(unsafe_code)]

use std::{
    fmt::{Debug, Display, Formatter},
    sync::Arc,
};

use crate::error::{LuceneError, Result};
use crate::index::index_reader::IndexReader;
use crate::index::multi_reader::{reader_util, ReaderSlice};
use crate::util::Bits;

// ---------------------------------------------------------------------------
// BitsSlice
// ---------------------------------------------------------------------------

/// Exposes the window of an existing [`Bits`] covered by a [`ReaderSlice`] as a
/// [`Bits`] in its own right.
///
/// Equivalent to `org.apache.lucene.index.BitsSlice`. Index `0` of the slice is
/// index `slice.start` of the parent, and the slice's length is
/// `slice.length` — the parent's own length is never consulted.
#[derive(Debug)]
pub struct BitsSlice {
    parent: Box<dyn Bits>,
    /// First parent index covered by this slice (inclusive).
    start: usize,
    /// Number of indices covered; the parent range is `start..start + length`.
    length: usize,
}

impl BitsSlice {
    /// Narrows `parent` to the window described by `slice`.
    ///
    /// Equivalent to `BitsSlice(Bits, ReaderSlice)`.
    ///
    /// **Deliberate divergence**: Java only checks `length >= 0`, and does so
    /// with an `assert`, which production builds disable — a negative length
    /// then silently yields a slice that rejects every lookup. Both bounds are
    /// checked here, and the check is reported as an error rather than
    /// skipped, because [`Bits::get`] has no way to signal the problem later.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `slice.start` or
    /// `slice.length` is negative.
    pub fn new(parent: Box<dyn Bits>, slice: ReaderSlice) -> Result<Self> {
        let start = usize::try_from(slice.start).map_err(|_| {
            LuceneError::IllegalArgument(format!("slice start must be >= 0; got {}", slice.start))
        })?;
        let length = usize::try_from(slice.length).map_err(|_| {
            LuceneError::IllegalArgument(format!("slice length must be >= 0; got {}", slice.length))
        })?;
        Ok(Self {
            parent,
            start,
            length,
        })
    }

    /// Returns the first parent index covered by this slice.
    pub fn start(&self) -> usize {
        self.start
    }
}

impl Bits for BitsSlice {
    /// Returns the parent's bit at `slice.start + index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is outside `0..length()`, mirroring the
    /// `IndexOutOfBoundsException` that Java's `Objects.checkIndex` raises.
    /// Lookups inside the slice may still panic if the slice reaches past the
    /// end of the parent, which is a bug in the [`ReaderSlice`] handed to
    /// [`BitsSlice::new`], not in the caller of `get`.
    fn get(&self, index: usize) -> bool {
        assert!(
            index < self.length,
            "index {index} out of bounds for slice of length {}",
            self.length
        );
        self.parent.get(index + self.start)
    }

    fn length(&self) -> usize {
        self.length
    }
}

// ---------------------------------------------------------------------------
// MultiBits
// ---------------------------------------------------------------------------

/// Concatenates one [`Bits`] per sub-reader into a single [`Bits`] over the
/// composite reader's global doc-ID space.
///
/// Equivalent to `org.apache.lucene.index.MultiBits`.
///
/// **NOTE**: this is very costly. Every lookup runs a binary search to locate
/// the owning sub-reader; iterating the leaves and reading each sub-reader's
/// own [`Bits`] is dramatically faster.
///
/// A sub-reader may contribute no [`Bits`] at all (a segment with no deletions
/// has no live-docs bitset). Those positions answer with the instance's default
/// value rather than being skipped, which is what keeps the global doc-ID
/// numbering intact.
///
/// Like Java, the only way to obtain one is [`MultiBits::get_live_docs`]; there
/// is no public constructor for concatenating arbitrary [`Bits`].
pub struct MultiBits {
    /// One entry per sub-reader; `None` where the sub-reader has no bitset.
    subs: Vec<Option<Box<dyn Bits>>>,
    /// Doc bases, with a trailing sentinel: `starts.len() == subs.len() + 1`
    /// and `starts[subs.len()]` is the composite reader's `maxDoc`.
    starts: Vec<i32>,
    /// Answer used for sub-readers that contributed no bitset.
    default_value: bool,
}

impl MultiBits {
    /// Builds a concatenated view. `starts` must hold one doc base per sub plus
    /// a trailing `maxDoc` sentinel.
    ///
    /// Equivalent to the private `MultiBits(Bits[], int[], boolean)`.
    fn new(subs: Vec<Option<Box<dyn Bits>>>, starts: Vec<i32>, default_value: bool) -> Self {
        debug_assert_eq!(
            starts.len(),
            subs.len() + 1,
            "starts must carry one entry per sub plus the maxDoc sentinel"
        );
        Self {
            subs,
            starts,
            default_value,
        }
    }

    /// Returns a single [`Bits`] over `reader`'s live documents, merging the
    /// per-leaf live-docs bitsets on the fly, or `None` when the reader has no
    /// deletions.
    ///
    /// Equivalent to `MultiBits.getLiveDocs(IndexReader)`.
    ///
    /// **NOTE**: this is a very slow way to reach live docs — each lookup costs
    /// a binary search. Prefer iterating the leaves and reading each one's own
    /// live docs.
    ///
    /// A reader with a single leaf yields that leaf's own live-docs instance
    /// with no wrapper, matching Lucene.
    pub fn get_live_docs(reader: &Arc<dyn IndexReader>) -> Option<Box<dyn Bits>> {
        if !reader.has_deletions() {
            return None;
        }

        let leaves = Arc::clone(reader).leaves();
        match leaves.len() {
            // Java asserts that a reader with deletions has at least one leaf,
            // then builds a MultiBits whose every lookup would fail. Reporting
            // "no live-docs view" is the truthful answer for a state that
            // cannot arise: deletions imply `max_doc > num_docs`, which implies
            // at least one leaf.
            0 => None,
            1 => leaves[0].leaf_reader().get_live_docs(),
            size => {
                let mut subs: Vec<Option<Box<dyn Bits>>> = Vec::with_capacity(size);
                let mut starts: Vec<i32> = Vec::with_capacity(size + 1);
                for ctx in &leaves {
                    // Record every leaf, including those with no live docs, so
                    // that the doc bases stay aligned with the global numbering.
                    subs.push(ctx.leaf_reader().get_live_docs());
                    starts.push(ctx.doc_base());
                }
                starts.push(reader.max_doc());
                Some(Box::new(Self::new(subs, starts, true)))
            }
        }
    }

    /// Returns the value reported for sub-readers that contributed no bitset.
    pub fn default_value(&self) -> bool {
        self.default_value
    }

    /// Returns the number of concatenated sub-readers.
    pub fn num_subs(&self) -> usize {
        self.subs.len()
    }
}

impl Bits for MultiBits {
    /// Resolves the sub-reader owning the global doc ID `doc` and returns its
    /// bit, or [`default_value`](Self::default_value) when that sub-reader
    /// contributed no bitset.
    ///
    /// # Panics
    ///
    /// Panics if `doc` is outside `0..length()`. Java raises
    /// `ArrayIndexOutOfBoundsException` from the same situation, having walked
    /// off the end of its `subs` array; the bound is checked up front here so
    /// the failure names the real problem.
    fn get(&self, doc: usize) -> bool {
        let length = self.length();
        assert!(
            doc < length,
            "doc {doc} out of bounds for reader with maxDoc {length}"
        );

        // `doc < length` and `length` came from an `i32`, so this is lossless.
        let reader = reader_util::sub_index(doc as i32, &self.starts);
        let sub = self
            .subs
            .get(reader)
            .expect("INVARIANT: doc < length() keeps sub_index() inside subs");
        match sub {
            None => self.default_value,
            Some(bits) => {
                let start = self.starts[reader] as usize;
                debug_assert!(
                    doc - start < (self.starts[reader + 1] - self.starts[reader]) as usize,
                    "doc {doc} escapes sub-reader {reader}"
                );
                bits.get(doc - start)
            }
        }
    }

    fn length(&self) -> usize {
        // The trailing sentinel holds the composite reader's maxDoc.
        self.starts
            .last()
            .copied()
            .unwrap_or(0)
            .try_into()
            .unwrap_or(0)
    }
}

impl Display for MultiBits {
    /// Renders the layout in the exact format of Java's `MultiBits.toString()`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} subs: ", self.subs.len())?;
        for (i, sub) in self.subs.iter().enumerate() {
            if i != 0 {
                write!(f, "; ")?;
            }
            match sub {
                None => write!(f, "s={} l=null", self.starts[i])?,
                Some(bits) => write!(f, "s={} l={} b={:?}", self.starts[i], bits.length(), bits)?,
            }
        }
        write!(f, " end={}", self.starts[self.subs.len()])
    }
}

impl Debug for MultiBits {
    /// Forwards to [`Display`], which reproduces Java's `toString()`.
    /// [`Bits`] requires `Debug`, and Java's only rendering of a `MultiBits` is
    /// that `toString()`, so both spellings agree here rather than offering two
    /// different views of the same layout.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    use crate::codecs::stub::StoredFieldVisitor;
    use crate::document::Document;
    use crate::index::index_reader::{CacheHelper, IndexReaderCore, StoredFields};
    use crate::index::leaf_reader::{LeafMetaData, LeafReader, TermVectors};
    use crate::index::multi_reader::MultiReader;
    use crate::index::{
        BinaryDocValues, ByteVectorValues, DocValuesSkipper, EmptyFields, FieldInfos, Fields,
        FloatVectorValues, NumericDocValues, PointValues, SortedDocValues, SortedNumericDocValues,
        SortedSetDocValues, Terms,
    };
    use crate::search::knn::KnnCollector;
    use crate::search::AcceptDocs;
    use crate::util::FixedBitSet;

    // -----------------------------------------------------------------------
    // Fixtures
    // -----------------------------------------------------------------------

    /// A [`Bits`] whose set positions are given explicitly, so tests can assert
    /// on exactly which local index was consulted.
    #[derive(Debug)]
    struct ListBits {
        set: Vec<bool>,
    }

    impl ListBits {
        fn boxed(set: &[bool]) -> Box<dyn Bits> {
            Box::new(Self { set: set.to_vec() })
        }
    }

    impl Bits for ListBits {
        fn get(&self, index: usize) -> bool {
            self.set[index]
        }

        fn length(&self) -> usize {
            self.set.len()
        }
    }

    fn live_docs(max_doc: usize, deleted: &[usize]) -> Box<dyn Bits> {
        let mut bits = FixedBitSet::new(max_doc);
        for doc in 0..max_doc {
            bits.set(doc);
        }
        for &doc in deleted {
            bits.clear(doc);
        }
        Box::new(bits)
    }

    #[derive(Debug)]
    struct StubTermVectors;
    impl TermVectors for StubTermVectors {
        fn get(&self, _doc: i32) -> Result<Option<Box<dyn Fields>>> {
            Ok(Some(Box::new(EmptyFields)))
        }
    }

    #[derive(Debug)]
    struct StubStoredFields;
    impl StoredFields for StubStoredFields {
        fn document_with_visitor(
            &self,
            _doc_id: i32,
            _visitor: &mut dyn StoredFieldVisitor,
        ) -> Result<()> {
            Ok(())
        }

        fn document(&self, _doc_id: i32) -> Result<Document> {
            Ok(Document::new())
        }

        fn document_fields(
            &self,
            _doc_id: i32,
            _fields_to_load: &HashSet<String>,
        ) -> Result<Document> {
            Ok(Document::new())
        }
    }

    /// Leaf reader carrying a configurable live-docs bitset, which is all
    /// [`MultiBits::get_live_docs`] reads.
    #[derive(Debug)]
    struct StubLeaf {
        core: IndexReaderCore,
        max_doc: i32,
        /// Docs deleted in this leaf; empty means "no live-docs bitset".
        deleted: Vec<usize>,
    }

    impl StubLeaf {
        fn arc(max_doc: i32, deleted: &[usize]) -> Arc<dyn IndexReader> {
            Arc::new(Self {
                core: IndexReaderCore::new(),
                max_doc,
                deleted: deleted.to_vec(),
            }) as Arc<dyn IndexReader>
        }
    }

    impl LeafReader for StubLeaf {
        fn core(&self) -> &IndexReaderCore {
            &self.core
        }

        fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
            Ok(Box::new(StubTermVectors))
        }

        fn num_docs(&self) -> i32 {
            self.max_doc - self.deleted.len() as i32
        }

        fn max_doc(&self) -> i32 {
            self.max_doc
        }

        fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
            Ok(Box::new(StubStoredFields))
        }

        fn do_close(&self) -> Result<()> {
            Ok(())
        }

        fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
            None
        }

        fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
            None
        }

        fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
            Ok(None)
        }

        fn get_numeric_doc_values(&self, _f: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
            Ok(None)
        }

        fn get_binary_doc_values(&self, _f: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
            Ok(None)
        }

        fn get_sorted_doc_values(&self, _f: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
            Ok(None)
        }

        fn get_sorted_numeric_doc_values(
            &self,
            _f: &str,
        ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
            Ok(None)
        }

        fn get_sorted_set_doc_values(
            &self,
            _f: &str,
        ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
            Ok(None)
        }

        fn get_norm_values(&self, _f: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
            Ok(None)
        }

        fn get_doc_values_skipper(&self, _f: &str) -> Result<Option<Box<dyn DocValuesSkipper>>> {
            Ok(None)
        }

        fn get_float_vector_values(&self, _f: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
            Ok(None)
        }

        fn get_byte_vector_values(&self, _f: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
            Ok(None)
        }

        fn search_nearest_vectors(
            &self,
            _field: &str,
            _target: &[f32],
            _collector: &mut dyn KnnCollector,
            _accept_docs: &mut dyn AcceptDocs,
        ) -> Result<()> {
            Ok(())
        }

        fn search_nearest_vectors_byte(
            &self,
            _field: &str,
            _target: &[u8],
            _collector: &mut dyn KnnCollector,
            _accept_docs: &mut dyn AcceptDocs,
        ) -> Result<()> {
            Ok(())
        }

        fn get_field_infos(&self) -> FieldInfos {
            FieldInfos::empty()
        }

        fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
            if self.deleted.is_empty() {
                None
            } else {
                Some(live_docs(self.max_doc as usize, &self.deleted))
            }
        }

        fn get_point_values(&self, _f: &str) -> Result<Option<Box<dyn PointValues>>> {
            Ok(None)
        }

        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }

        fn get_meta_data(&self) -> LeafMetaData {
            LeafMetaData::new(10, None, None, false).unwrap()
        }
    }

    fn composite(leaves: Vec<Arc<dyn IndexReader>>) -> Arc<dyn IndexReader> {
        Arc::new(MultiReader::new(leaves, false).unwrap()) as Arc<dyn IndexReader>
    }

    // -----------------------------------------------------------------------
    // BitsSlice
    // -----------------------------------------------------------------------

    #[test]
    fn bits_slice_translates_to_parent_coordinates() {
        // Parent covers 6 positions; the slice exposes positions 2..5.
        let parent = ListBits::boxed(&[false, false, true, false, true, false]);
        let slice = BitsSlice::new(parent, ReaderSlice::new(2, 3, 0)).unwrap();

        assert_eq!(slice.length(), 3);
        assert_eq!(slice.start(), 2);
        assert!(slice.get(0), "slice[0] is parent[2]");
        assert!(!slice.get(1), "slice[1] is parent[3]");
        assert!(slice.get(2), "slice[2] is parent[4]");
    }

    #[test]
    fn bits_slice_at_offset_zero_mirrors_the_parent_prefix() {
        let parent = ListBits::boxed(&[true, false, true]);
        let slice = BitsSlice::new(parent, ReaderSlice::new(0, 2, 0)).unwrap();
        assert_eq!(slice.length(), 2);
        assert!(slice.get(0));
        assert!(!slice.get(1));
    }

    #[test]
    fn bits_slice_length_is_the_slice_length_not_the_parent_length() {
        // The slice deliberately stops short of the parent's end.
        let parent = ListBits::boxed(&[true; 10]);
        let slice = BitsSlice::new(parent, ReaderSlice::new(1, 4, 0)).unwrap();
        assert_eq!(slice.length(), 4);
    }

    #[test]
    fn bits_slice_of_zero_length_accepts_no_lookup() {
        let parent = ListBits::boxed(&[true, true]);
        let slice = BitsSlice::new(parent, ReaderSlice::new(1, 0, 0)).unwrap();
        assert_eq!(slice.length(), 0);
    }

    #[test]
    #[should_panic(expected = "out of bounds for slice of length 3")]
    fn bits_slice_rejects_index_at_the_length() {
        let parent = ListBits::boxed(&[true; 10]);
        let slice = BitsSlice::new(parent, ReaderSlice::new(2, 3, 0)).unwrap();
        // Position 3 is inside the parent but outside the slice: Java's
        // Objects.checkIndex rejects it, and so must this port.
        let _ = slice.get(3);
    }

    #[test]
    #[should_panic(expected = "out of bounds for slice of length 0")]
    fn bits_slice_of_zero_length_rejects_index_zero() {
        let parent = ListBits::boxed(&[true, true]);
        let slice = BitsSlice::new(parent, ReaderSlice::new(1, 0, 0)).unwrap();
        let _ = slice.get(0);
    }

    #[test]
    fn bits_slice_rejects_negative_bounds() {
        let negative_length = BitsSlice::new(ListBits::boxed(&[true]), ReaderSlice::new(0, -1, 0));
        assert!(matches!(
            negative_length,
            Err(LuceneError::IllegalArgument(_))
        ));

        let negative_start = BitsSlice::new(ListBits::boxed(&[true]), ReaderSlice::new(-1, 1, 0));
        assert!(matches!(
            negative_start,
            Err(LuceneError::IllegalArgument(_))
        ));
    }

    #[test]
    fn bits_slice_ignores_the_reader_index() {
        // reader_index carries no meaning for a BitsSlice; only start/length do.
        let parent = ListBits::boxed(&[false, true]);
        let slice = BitsSlice::new(parent, ReaderSlice::new(1, 1, 42)).unwrap();
        assert!(slice.get(0));
    }

    // -----------------------------------------------------------------------
    // MultiBits: direct construction
    // -----------------------------------------------------------------------

    /// Three subs of 3, 2 and 4 docs; maxDoc 9.
    fn three_subs() -> MultiBits {
        MultiBits::new(
            vec![
                Some(ListBits::boxed(&[true, false, true])),
                Some(ListBits::boxed(&[false, true])),
                Some(ListBits::boxed(&[true, true, false, true])),
            ],
            vec![0, 3, 5, 9],
            true,
        )
    }

    #[test]
    fn multi_bits_length_is_the_trailing_sentinel() {
        assert_eq!(three_subs().length(), 9);
        assert_eq!(three_subs().num_subs(), 3);
    }

    #[test]
    fn multi_bits_maps_every_global_doc_to_the_right_local_bit() {
        let bits = three_subs();
        let expected = [true, false, true, false, true, true, true, false, true];
        for (doc, &want) in expected.iter().enumerate() {
            assert_eq!(bits.get(doc), want, "doc {doc}");
        }
    }

    #[test]
    fn multi_bits_resolves_sub_reader_boundaries() {
        // The docs either side of each boundary must land in different subs.
        let bits = three_subs();
        assert!(bits.get(2), "last doc of sub 0");
        assert!(!bits.get(3), "first doc of sub 1");
        assert!(bits.get(4), "last doc of sub 1");
        assert!(bits.get(5), "first doc of sub 2");
        assert!(bits.get(8), "last doc of sub 2");
    }

    #[test]
    fn multi_bits_uses_the_default_value_for_subs_without_bits() {
        // Sub 1 has no bitset: every one of its docs answers with the default.
        let all_live = MultiBits::new(
            vec![
                Some(ListBits::boxed(&[true, false])),
                None,
                Some(ListBits::boxed(&[false])),
            ],
            vec![0, 2, 5, 6],
            true,
        );
        assert!(all_live.default_value());
        assert!(all_live.get(0));
        assert!(!all_live.get(1));
        assert!(all_live.get(2), "sub 1 has no bits -> default");
        assert!(all_live.get(3));
        assert!(all_live.get(4));
        assert!(!all_live.get(5));

        // The default is a field, not a constant: prove `false` is honoured too.
        let none_live = MultiBits::new(vec![None], vec![0, 2], false);
        assert!(!none_live.default_value());
        assert!(!none_live.get(0));
        assert!(!none_live.get(1));
    }

    #[test]
    fn multi_bits_handles_empty_leading_and_trailing_subs() {
        // Empty subs produce duplicate doc bases; the binary search must skip
        // past them to the sub that actually owns the doc.
        let bits = MultiBits::new(
            vec![
                Some(ListBits::boxed(&[])),      // empty, docBase 0
                Some(ListBits::boxed(&[true])),  // docBase 0
                Some(ListBits::boxed(&[])),      // empty, docBase 1
                Some(ListBits::boxed(&[false])), // docBase 1
                Some(ListBits::boxed(&[])),      // empty, docBase 2
            ],
            vec![0, 0, 1, 1, 2, 2],
            true,
        );
        assert_eq!(bits.length(), 2);
        assert!(bits.get(0), "doc 0 belongs to sub 1, not the empty sub 0");
        assert!(!bits.get(1), "doc 1 belongs to sub 3, not the empty sub 2");
    }

    #[test]
    fn multi_bits_single_sub_still_offsets_from_its_doc_base() {
        let bits = MultiBits::new(
            vec![Some(ListBits::boxed(&[false, true]))],
            vec![0, 2],
            true,
        );
        assert!(!bits.get(0));
        assert!(bits.get(1));
    }

    #[test]
    #[should_panic(expected = "doc 9 out of bounds for reader with maxDoc 9")]
    fn multi_bits_rejects_doc_at_max_doc() {
        // Java walks off the end of its subs array here; the bound is named.
        let _ = three_subs().get(9);
    }

    #[test]
    #[should_panic(expected = "out of bounds for reader with maxDoc 9")]
    fn multi_bits_rejects_doc_past_max_doc() {
        let _ = three_subs().get(100);
    }

    #[test]
    fn multi_bits_renders_javas_to_string_layout() {
        let bits = MultiBits::new(
            vec![Some(ListBits::boxed(&[true, false])), None],
            vec![0, 2, 5],
            true,
        );
        let rendered = format!("{bits}");
        assert!(
            rendered.starts_with("2 subs: s=0 l=2 b="),
            "unexpected prefix: {rendered}"
        );
        assert!(
            rendered.contains("; s=2 l=null"),
            "missing null sub: {rendered}"
        );
        assert!(rendered.ends_with(" end=5"), "missing end: {rendered}");
        // Bits requires Debug, and it must show the same layout.
        assert_eq!(format!("{bits:?}"), rendered);
    }

    // -----------------------------------------------------------------------
    // MultiBits::get_live_docs
    // -----------------------------------------------------------------------

    #[test]
    fn get_live_docs_returns_none_without_deletions() {
        let reader = composite(vec![StubLeaf::arc(3, &[]), StubLeaf::arc(4, &[])]);
        assert!(MultiBits::get_live_docs(&reader).is_none());
    }

    #[test]
    fn get_live_docs_returns_none_for_an_empty_reader() {
        let reader = composite(vec![]);
        assert!(MultiBits::get_live_docs(&reader).is_none());
    }

    #[test]
    fn get_live_docs_unwraps_a_single_leaf() {
        // One leaf: Lucene hands back that leaf's own bitset with no wrapper,
        // so its length is the leaf's maxDoc and lookups are already local.
        let leaf = StubLeaf::arc(4, &[1]);
        let bits = MultiBits::get_live_docs(&leaf).expect("leaf has deletions");
        assert_eq!(bits.length(), 4);
        assert!(bits.get(0));
        assert!(!bits.get(1));
        assert!(bits.get(2));
        assert!(bits.get(3));
    }

    #[test]
    fn get_live_docs_concatenates_leaves_at_their_doc_bases() {
        // Leaf 0: docs 0..3 with doc 1 deleted.
        // Leaf 1: docs 3..7 with no deletions at all (no bitset).
        // Leaf 2: docs 7..9 with doc 0 (global 7) deleted.
        let reader = composite(vec![
            StubLeaf::arc(3, &[1]),
            StubLeaf::arc(4, &[]),
            StubLeaf::arc(2, &[0]),
        ]);
        let bits = MultiBits::get_live_docs(&reader).expect("reader has deletions");

        assert_eq!(bits.length(), 9);
        let expected = [true, false, true, true, true, true, true, false, true];
        for (doc, &want) in expected.iter().enumerate() {
            assert_eq!(bits.get(doc), want, "global doc {doc}");
        }
    }

    #[test]
    fn get_live_docs_defaults_to_live_for_leaves_without_a_bitset() {
        // The first leaf has no bitset; had the default been `false`, its docs
        // would read as deleted.
        let reader = composite(vec![StubLeaf::arc(2, &[]), StubLeaf::arc(2, &[0])]);
        let bits = MultiBits::get_live_docs(&reader).expect("reader has deletions");
        assert!(bits.get(0), "leaf 0 has no bitset -> live by default");
        assert!(bits.get(1));
        assert!(!bits.get(2), "global doc 2 is leaf 1's deleted doc 0");
        assert!(bits.get(3));
    }

    #[test]
    fn get_live_docs_skips_empty_leaves() {
        // An empty leaf shares its doc base with the next one; the lookup must
        // still reach the leaf that owns the doc.
        let reader = composite(vec![
            StubLeaf::arc(0, &[]),
            StubLeaf::arc(3, &[2]),
            StubLeaf::arc(0, &[]),
        ]);
        let bits = MultiBits::get_live_docs(&reader).expect("reader has deletions");
        assert_eq!(bits.length(), 3);
        assert!(bits.get(0));
        assert!(bits.get(1));
        assert!(!bits.get(2));
    }

    #[test]
    fn get_live_docs_matches_per_leaf_lookup_across_the_whole_reader() {
        // The contract that matters: the concatenated view must agree with
        // reading each leaf's own bitset at its local doc ID, for every doc.
        let leaves = vec![
            StubLeaf::arc(5, &[0, 4]),
            StubLeaf::arc(3, &[]),
            StubLeaf::arc(4, &[1, 2]),
            StubLeaf::arc(0, &[]),
        ];
        let reader = composite(leaves);
        let bits = MultiBits::get_live_docs(&reader).expect("reader has deletions");

        for ctx in Arc::clone(&reader).leaves() {
            let leaf = ctx.leaf_reader();
            let leaf_bits = leaf.get_live_docs();
            let base = ctx.doc_base() as usize;
            for local in 0..leaf.max_doc() as usize {
                let want = match &leaf_bits {
                    Some(b) => b.get(local),
                    None => true,
                };
                assert_eq!(
                    bits.get(base + local),
                    want,
                    "leaf at base {base}, local doc {local}"
                );
            }
        }
    }
}
