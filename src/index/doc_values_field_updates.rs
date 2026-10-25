//! In-memory doc-values field updates for a single segment.
//!
//! Holds per-document updates to a single DocValues field, buffered in RAM
//! before they are written to a generation-named file.  Each update is either
//! a numeric (`i64`) value, a binary (`BytesRef`) value, or a *reset* that
//! clears the field for a document.
//!
//! Equivalent to `org.apache.lucene.index.DocValuesFieldUpdates`.

#![deny(unsafe_code)]

use std::fmt;

use crate::error::{LuceneError, Result};
use crate::index::DocValuesType;
use crate::search::doc_id_set_iterator::NO_MORE_DOCS;
use crate::search::DocIdSetIterator;
use crate::util::bit_set::BitSet;
use crate::util::bit_sets::SparseFixedBitSet;
use crate::util::{Accountable, BytesRef, RamUsageEstimator};

/// Page size for the internal doc-ID storage, matching Lucene's
/// `DocValuesFieldUpdates.PAGE_SIZE`.
const PAGE_SIZE: usize = 1024;

/// Mask stored in the low bit of each packed doc entry: `1` = has value,
/// `0` = reset (no value).
const HAS_VALUE_MASK: u64 = 1;
const HAS_NO_VALUE_MASK: u64 = 0;
const SHIFT: u32 = 1;

/// A single update value: either a long, a binary blob, or a reset.
#[derive(Clone, Debug)]
enum UpdateValue {
    /// Numeric update.
    Long(i64),
    /// Binary update.
    Binary(Vec<u8>),
    /// Reset (clear the field for this doc).
    Reset,
}

/// Holds updates of a single DocValues field for a set of documents within one
/// segment.
///
/// Equivalent to `org.apache.lucene.index.DocValuesFieldUpdates`.
pub struct DocValuesFieldUpdates {
    /// Field name being updated.
    pub field: String,
    /// DocValues type of the field.
    pub dv_type: DocValuesType,
    /// Generation of this update packet.
    pub del_gen: i64,
    /// Maximum document ID for this segment.
    max_doc: i32,

    // Packed doc entries: (doc_id << SHIFT) | has_value_mask
    docs: Vec<u64>,
    /// Per-entry values, indexed by position (0..size).
    values: Vec<UpdateValue>,
    /// Number of updates currently buffered.
    size: usize,
    /// Whether `finish()` has been called.
    finished: bool,
    /// The subclass state when this instance is
    /// `NumericDocValuesFieldUpdates.SingleValueNumericDocValuesFieldUpdates`;
    /// see [`new_single_value_numeric`](Self::new_single_value_numeric).
    single_value: Option<SingleValueNumericDocValuesFieldUpdates>,
}

/// The updates of a numeric doc-values field that write one value to every
/// matched document.
///
/// Equivalent to
/// `org.apache.lucene.index.NumericDocValuesFieldUpdates.SingleValueNumericDocValuesFieldUpdates`
/// (`NumericDocValuesFieldUpdates.java:136-235`), which
/// `FrozenBufferedUpdates.applyDocValuesUpdates` builds instead of the general
/// packed form when the buffered updates of a field carry a single value
/// (`FrozenBufferedUpdates.java:284-298`): the value is kept once, and the
/// documents in a sparse bitset, with a second bitset marking the documents
/// whose last update was a reset.
///
/// **Divergence from Lucene 10.5.0.** Java's class extends
/// `DocValuesFieldUpdates`; this port's [`DocValuesFieldUpdates`] is one
/// concrete type — it already stands for both `NumericDocValuesFieldUpdates`
/// and `BinaryDocValuesFieldUpdates` — so the subclass's own state is this
/// struct, which a [`DocValuesFieldUpdates`] holds and dispatches to, exactly
/// where Java's overrides take over. Build one with
/// [`DocValuesFieldUpdates::new_single_value_numeric`].
#[derive(Clone, Debug)]
pub struct SingleValueNumericDocValuesFieldUpdates {
    value: i64,
    bit_set: SparseFixedBitSet,
    has_no_value: Option<SparseFixedBitSet>,
    has_at_least_one_value: bool,
}

impl SingleValueNumericDocValuesFieldUpdates {
    /// Equivalent to the subclass part of
    /// `SingleValueNumericDocValuesFieldUpdates(long, String, int, long)`.
    fn new(max_doc: i32, value: i64) -> Self {
        Self {
            value,
            bit_set: SparseFixedBitSet::new(max_doc.max(0) as usize),
            has_no_value: None,
            has_at_least_one_value: false,
        }
    }

    /// Returns the value every updated document receives.
    ///
    /// Equivalent to the package-private `longValue()`, which Java keeps for
    /// testing.
    pub fn long_value(&self) -> i64 {
        self.value
    }

    /// Equivalent to `SingleValueNumericDocValuesFieldUpdates.add(int, long)`.
    fn add(&mut self, doc: i32, value: i64) {
        debug_assert_eq!(self.value, value);
        self.bit_set.set(doc as usize);
        self.has_at_least_one_value = true;
        if let Some(has_no_value) = self.has_no_value.as_mut() {
            has_no_value.clear(doc as usize);
        }
    }

    /// Equivalent to `SingleValueNumericDocValuesFieldUpdates.reset(int)`.
    fn reset(&mut self, doc: i32, max_doc: i32) {
        self.bit_set.set(doc as usize);
        self.has_at_least_one_value = true;
        self.has_no_value
            .get_or_insert_with(|| SparseFixedBitSet::new(max_doc.max(0) as usize))
            .set(doc as usize);
    }

    /// The part of `ramBytesUsed()` the subclass adds to the base class's:
    /// `bitSet.ramBytesUsed() + (hasNoValue == null ? 0 : hasNoValue.ramBytesUsed())`.
    fn ram_bytes_used(&self) -> i64 {
        self.bit_set.ram_bytes_used()
            + self
                .has_no_value
                .as_ref()
                .map_or(0, Accountable::ram_bytes_used)
    }
}

impl fmt::Debug for DocValuesFieldUpdates {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocValuesFieldUpdates")
            .field("field", &self.field)
            .field("type", &self.dv_type)
            .field("del_gen", &self.del_gen)
            .field("size", &self.size)
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

impl DocValuesFieldUpdates {
    /// Creates a new, empty update buffer.
    ///
    /// Equivalent to `DocValuesFieldUpdates(maxDoc, delGen, field, type)`.
    pub fn new(
        max_doc: i32,
        del_gen: i64,
        field: impl Into<String>,
        dv_type: DocValuesType,
    ) -> Self {
        Self {
            field: field.into(),
            dv_type,
            del_gen,
            max_doc,
            docs: Vec::with_capacity(PAGE_SIZE),
            values: Vec::with_capacity(PAGE_SIZE),
            size: 0,
            finished: false,
            single_value: None,
        }
    }

    /// Creates the updates of a numeric field that write `value` to every
    /// matched document.
    ///
    /// Equivalent to `new SingleValueNumericDocValuesFieldUpdates(long delGen,
    /// String field, int maxDoc, long value)`; see
    /// [`SingleValueNumericDocValuesFieldUpdates`].
    pub fn new_single_value_numeric(
        del_gen: i64,
        field: impl Into<String>,
        max_doc: i32,
        value: i64,
    ) -> Self {
        let mut updates = Self::new(max_doc, del_gen, field, DocValuesType::NUMERIC);
        updates.single_value = Some(SingleValueNumericDocValuesFieldUpdates::new(max_doc, value));
        updates
    }

    /// Returns the single-value state when this instance is a
    /// [`SingleValueNumericDocValuesFieldUpdates`].
    pub fn as_single_value_numeric(&self) -> Option<&SingleValueNumericDocValuesFieldUpdates> {
        self.single_value.as_ref()
    }

    /// Returns `true` if `finish()` has been called.
    pub fn get_finished(&self) -> bool {
        self.finished
    }

    /// Returns `true` if this instance contains any updates.
    ///
    /// Equivalent to `DocValuesFieldUpdates.any()`, and to the
    /// `SingleValueNumericDocValuesFieldUpdates` override, which also answers
    /// `true` once a document was added to its bitset.
    pub fn any(&self) -> bool {
        self.size > 0
            || self
                .single_value
                .as_ref()
                .is_some_and(|single| single.has_at_least_one_value)
    }

    /// Returns the number of buffered updates.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Adds a numeric update for `doc`.
    ///
    /// # Panics
    ///
    /// Panics if this buffer is already finished or `doc >= max_doc`.
    pub fn add_long(&mut self, doc: i32, value: i64) {
        // `SingleValueNumericDocValuesFieldUpdates.add(int, long)` neither
        // checks `finished` nor records an entry: it only sets the document's
        // bit.
        if let Some(single) = self.single_value.as_mut() {
            assert!(doc < self.max_doc, "doc {doc} >= maxDoc {}", self.max_doc);
            single.add(doc, value);
            return;
        }
        assert!(!self.finished, "already finished");
        assert!(doc < self.max_doc, "doc {doc} >= maxDoc {}", self.max_doc);
        let idx = self.add_internal(doc, HAS_VALUE_MASK);
        self.values[idx] = UpdateValue::Long(value);
    }

    /// Adds a binary update for `doc`.
    ///
    /// # Panics
    ///
    /// Panics if this buffer is already finished or `doc >= max_doc`.
    pub fn add_binary(&mut self, doc: i32, value: &BytesRef) {
        // `SingleValueNumericDocValuesFieldUpdates.add(int, BytesRef)` throws
        // `UnsupportedOperationException`.
        assert!(
            self.single_value.is_none(),
            "a single-value numeric update cannot take a binary value"
        );
        assert!(!self.finished, "already finished");
        assert!(doc < self.max_doc, "doc {doc} >= maxDoc {}", self.max_doc);
        let idx = self.add_internal(doc, HAS_VALUE_MASK);
        self.values[idx] = UpdateValue::Binary(value.slice().to_vec());
    }

    /// Resets (clears) the value for `doc`.
    ///
    /// Equivalent to `DocValuesFieldUpdates.reset(int)`.
    ///
    /// # Panics
    ///
    /// Panics if this buffer is already finished or `doc >= max_doc`.
    pub fn reset(&mut self, doc: i32) {
        // `SingleValueNumericDocValuesFieldUpdates.reset(int)` marks the
        // document in both bitsets instead of recording an entry.
        if let Some(single) = self.single_value.as_mut() {
            assert!(doc < self.max_doc, "doc {doc} >= maxDoc {}", self.max_doc);
            single.reset(doc, self.max_doc);
            return;
        }
        assert!(!self.finished, "already finished");
        assert!(doc < self.max_doc, "doc {doc} >= maxDoc {}", self.max_doc);
        let idx = self.add_internal(doc, HAS_NO_VALUE_MASK);
        self.values[idx] = UpdateValue::Reset;
    }

    fn add_internal(&mut self, doc: i32, has_value_mask: u64) -> usize {
        // `DocValuesFieldUpdates.addInternal` grows the structures only when the
        // buffer is full, and grows them through `grow(size + 1)`.
        if self.size == self.docs.len() {
            self.grow(self.size + 1);
        }
        let packed = ((doc as u64) << SHIFT) | has_value_mask;
        self.docs[self.size] = packed;
        let idx = self.size;
        self.size += 1;
        idx
    }

    /// Makes room for at least `size` entries.
    ///
    /// Equivalent to `DocValuesFieldUpdates.grow(int)` together with the
    /// `BinaryDocValuesFieldUpdates.grow(int)` and
    /// `NumericDocValuesFieldUpdates.grow(int)` overrides: Java splits the
    /// entries across a `PagedMutable docs` on the base class and the value
    /// arrays on each subclass, and every override grows its own arrays after
    /// calling `super.grow(size)`. This port keeps one buffer per concern in a
    /// single type, so the one method grows both.
    ///
    /// The growth policy is `AbstractPagedMutable.grow(long)`'s: nothing happens
    /// when the buffer is already long enough, and otherwise it becomes
    /// `size + max(size >> 3, 3)` entries. Charging that shape matters because
    /// [`ram_bytes_used`](Accountable::ram_bytes_used) counts the buffer, and
    /// that figure drives when buffered updates are written to disk.
    fn grow(&mut self, size: usize) {
        if size <= self.docs.len() {
            return;
        }
        let extra = std::cmp::max(size >> 3, 3);
        self.resize(size + extra);
    }

    /// Exchanges the entries at `i` and `j`.
    ///
    /// Equivalent to `DocValuesFieldUpdates.swap(int, int)` together with the
    /// `BinaryDocValuesFieldUpdates` and `NumericDocValuesFieldUpdates`
    /// overrides, which swap their own value columns after calling
    /// `super.swap(i, j)`. One method covers both columns here for the same
    /// reason [`grow`](Self::grow) does.
    fn swap(&mut self, i: usize, j: usize) {
        self.docs.swap(i, j);
        self.values.swap(i, j);
    }

    /// Freezes internal data structures and sorts updates by docID for
    /// efficient iteration.
    ///
    /// Equivalent to `DocValuesFieldUpdates.finish()`.
    ///
    /// # Errors
    ///
    /// Returns `IllegalState` if called twice.
    pub fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Err(LuceneError::IllegalState(
                "DocValuesFieldUpdates already finished".to_string(),
            ));
        }
        self.finished = true;
        // shrink wrap
        if self.size < self.docs.len() {
            self.resize(self.size);
        }
        if self.size > 1 {
            self.sort();
        }
        Ok(())
    }

    /// Resizes the packed doc entries and their values to hold exactly `size`
    /// updates.
    ///
    /// Equivalent to `DocValuesFieldUpdates.resize(int)` together with the
    /// `BinaryDocValuesFieldUpdates.resize(int)` and
    /// `NumericDocValuesFieldUpdates.resize(int)` overrides: Java splits the
    /// entries across a `PagedMutable docs` on the base class and the value
    /// arrays on each subclass, and every override resizes its own arrays after
    /// calling `super.resize(size)`. This port keeps one buffer per concern in a
    /// single type, so the one method resizes both. Growing leaves the new
    /// entries zeroed, as `PagedMutable.resize` does.
    fn resize(&mut self, size: usize) {
        self.docs.resize(size, 0);
        self.values.resize(size, UpdateValue::Reset);
    }

    /// Checks that [`finish`](Self::finish) has already run.
    ///
    /// Equivalent to `DocValuesFieldUpdates.ensureFinished()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the buffer is not finished.
    pub fn ensure_finished(&self) -> Result<()> {
        if !self.finished {
            return Err(LuceneError::IllegalState("call finish first".to_string()));
        }
        Ok(())
    }

    /// Stable sort by docID (ascending), preserving insertion order for ties
    /// so the last update to a given docID wins during iteration.
    fn sort(&mut self) {
        // Java sorts with an `IntroSorter` over an `ords` array that records each
        // entry's original position, so that ties on docID keep insertion order
        // and the last update to a docID wins; its `swap` moves the ords and the
        // entries together. This port computes the same permutation with a
        // stable sort and then applies it in place through
        // [`swap`](Self::swap), which is where Java's `swap` override does its
        // work.
        let mut ords: Vec<usize> = (0..self.size).collect();
        ords.sort_by_key(|&i| self.docs[i] >> SHIFT);

        // `ords[new] = old`; invert it into "the entry at `old` belongs at
        // `new`", which is the form the in-place swap loop consumes.
        let mut destination = vec![0usize; self.size];
        for (new_pos, &old_pos) in ords.iter().enumerate() {
            destination[old_pos] = new_pos;
        }
        for i in 0..self.size {
            while destination[i] != i {
                let j = destination[i];
                self.swap(i, j);
                destination.swap(i, j);
            }
        }
    }

    /// Returns an iterator over the updated documents and their values.
    ///
    /// Documents are returned in increasing docID order.  When the same docID
    /// was updated multiple times, only the last update is returned.
    ///
    /// Equivalent to `DocValuesFieldUpdates.iterator()`.
    pub fn iterator(&self) -> DocValuesFieldUpdatesIterator<'_> {
        // `SingleValueNumericDocValuesFieldUpdates.iterator()` iterates its
        // bitset with a `BitSetIterator(bitSet, maxDoc)` and, unlike the base
        // class, does not call `ensureFinished()`.
        if let Some(single) = self.single_value.as_ref() {
            return DocValuesFieldUpdatesIterator::single_value(single, self.max_doc, self.del_gen);
        }
        assert!(self.ensure_finished().is_ok(), "call finish() first");
        DocValuesFieldUpdatesIterator::new(self.size, &self.docs, &self.values, self.del_gen)
    }

    /// Creates a deep copy of this update buffer, suitable for carrying into
    /// a merge.
    ///
    /// Equivalent to `DocValuesFieldUpdates.clone()`.
    pub fn clone_for_merge(&self) -> Self {
        Self {
            field: self.field.clone(),
            dv_type: self.dv_type,
            del_gen: self.del_gen,
            max_doc: self.max_doc,
            docs: self.docs.clone(),
            values: self.values.clone(),
            size: self.size,
            finished: true,
            single_value: self.single_value.clone(),
        }
    }
}

impl Accountable for DocValuesFieldUpdates {
    fn ram_bytes_used(&self) -> i64 {
        // Rough estimate: docs vec (u64 per entry) + values vec (header + per-entry).
        let docs_size = (self.docs.capacity() * std::mem::size_of::<u64>()) as i64;
        let values_header = RamUsageEstimator::NUM_BYTES_OBJECT_HEADER;
        let values_per_entry = std::mem::size_of::<UpdateValue>() as i64;
        let values_size = values_header + (self.values.capacity() as i64 * values_per_entry);
        // Add binary content sizes.
        let binary_size: i64 = self
            .values
            .iter()
            .map(|v| match v {
                UpdateValue::Binary(b) => b.len() as i64,
                _ => 0,
            })
            .sum();
        // Java's tail is
        // `NUM_BYTES_OBJECT_HEADER + 2 * Integer.BYTES + 2 + Long.BYTES +
        // NUM_BYTES_OBJECT_REF` (`DocValuesFieldUpdates.java:416-423`); it is
        // written out literally here so the constant does not drift with Rust's
        // own type sizes.
        // `SingleValueNumericDocValuesFieldUpdates.ramBytesUsed()` adds its
        // bitsets to the base class's figure.
        let single_size = self
            .single_value
            .as_ref()
            .map_or(0, SingleValueNumericDocValuesFieldUpdates::ram_bytes_used);
        docs_size
            + values_size
            + binary_size
            + single_size
            + RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
            + 2 * 4
            + 2
            + 8
            + RamUsageEstimator::NUM_BYTES_OBJECT_REF
    }
}

/// An iterator over documents and their updated values.
///
/// Only documents with updates are returned, in increasing docID order.  When
/// the same docID appears multiple times, only the final update is yielded.
///
/// Equivalent to `DocValuesFieldUpdates.Iterator`.
pub struct DocValuesFieldUpdatesIterator<'a> {
    /// The bitsets and value of a
    /// [`SingleValueNumericDocValuesFieldUpdates`], whose iterator is the
    /// anonymous `DocValuesFieldUpdates.Iterator` over a
    /// `BitSetIterator(bitSet, maxDoc)` that it returns.
    single: Option<(&'a SingleValueNumericDocValuesFieldUpdates, i32)>,
    size: usize,
    docs: &'a [u64],
    values: &'a [UpdateValue],
    del_gen: i64,
    idx: usize,
    doc: i32,
    has_value: bool,
    // Cached current value (set by next_doc).
    current_long: i64,
    current_binary: BytesRef,
    current_is_binary: bool,
}

impl<'a> DocValuesFieldUpdatesIterator<'a> {
    fn new(size: usize, docs: &'a [u64], values: &'a [UpdateValue], del_gen: i64) -> Self {
        Self {
            single: None,
            size,
            docs,
            values,
            del_gen,
            idx: 0,
            doc: -1,
            has_value: false,
            current_long: 0,
            current_binary: BytesRef::default(),
            current_is_binary: false,
        }
    }

    /// The iterator of a [`SingleValueNumericDocValuesFieldUpdates`]: its
    /// set bits below `max_doc`, each with the one value, and a value only
    /// where the document's last update was not a reset.
    fn single_value(
        single: &'a SingleValueNumericDocValuesFieldUpdates,
        max_doc: i32,
        del_gen: i64,
    ) -> Self {
        let mut iterator = Self::new(0, &[], &[], del_gen);
        iterator.single = Some((single, max_doc));
        iterator.current_long = single.value;
        iterator
    }

    /// Returns the delGen of this packet.
    pub fn del_gen(&self) -> i64 {
        self.del_gen
    }

    /// Returns `true` if the current document has a value (not a reset).
    pub fn has_value(&self) -> bool {
        self.has_value
    }

    /// Returns the long value for the current document.
    ///
    /// # Panics
    ///
    /// Panics if this iterator is not a long iterator or the current doc has
    /// no value.
    pub fn long_value(&self) -> i64 {
        // The single-value iterator's `longValue()` returns the value whatever
        // `hasValue()` says.
        if let Some((single, _)) = self.single {
            return single.value;
        }
        assert!(self.has_value, "no value for current doc");
        assert!(!self.current_is_binary, "not a numeric iterator");
        self.current_long
    }

    /// Returns the binary value for the current document.
    ///
    /// # Panics
    ///
    /// Panics if this iterator is not a binary iterator or the current doc has
    /// no value.
    pub fn binary_value(&self) -> BytesRef {
        assert!(self.has_value, "no value for current doc");
        assert!(self.current_is_binary, "not a binary iterator");
        self.current_binary.clone()
    }
}

impl<'a> DocIdSetIterator for DocValuesFieldUpdatesIterator<'a> {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        if let Some((single, max_doc)) = self.single {
            // `BitSetIterator.nextDoc()`, which is `advance(doc + 1)`.
            let target = self.doc + 1;
            self.doc = if target >= max_doc {
                NO_MORE_DOCS
            } else {
                BitSet::next_set_bit(&single.bit_set, target)
            };
            // The anonymous iterator's `hasValue()`.
            self.has_value = self.doc != NO_MORE_DOCS
                && single
                    .has_no_value
                    .as_ref()
                    .map_or(true, |has_no_value| !has_no_value.get(self.doc as usize));
            return Ok(self.doc);
        }
        if self.idx >= self.size {
            self.doc = NO_MORE_DOCS;
            return Ok(NO_MORE_DOCS);
        }
        let mut long_doc = self.docs[self.idx];
        self.idx += 1;
        // Scan forward to the last update for this doc.
        while self.idx < self.size {
            let next = self.docs[self.idx];
            if (long_doc >> SHIFT) != (next >> SHIFT) {
                break;
            }
            long_doc = next;
            self.idx += 1;
        }
        self.has_value = (long_doc & HAS_VALUE_MASK) > 0;
        if self.has_value {
            let value_idx = self.idx - 1;
            match &self.values[value_idx] {
                UpdateValue::Long(v) => {
                    self.current_long = *v;
                    self.current_is_binary = false;
                }
                UpdateValue::Binary(b) => {
                    self.current_binary = BytesRef::new(b.clone());
                    self.current_is_binary = true;
                }
                UpdateValue::Reset => {
                    // Should not happen: has_value is true but value is Reset.
                    // This means the packed entry had HAS_VALUE_MASK but the
                    // value slot was Reset.  Treat as no value.
                    self.has_value = false;
                }
            }
        }
        self.doc = (long_doc >> SHIFT) as i32;
        Ok(self.doc)
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "advance not supported for DocValuesFieldUpdatesIterator".to_string(),
        ))
    }

    /// The packed iterator's cost is its entry count; the single-value one is
    /// a `BitSetIterator(bitSet, maxDoc)`, whose cost is `maxDoc`.
    fn cost(&self) -> i64 {
        match self.single {
            Some((_, max_doc)) => i64::from(max_doc),
            None => self.size as i64,
        }
    }
}

/// Merge-sorts multiple iterators, one per delGen, favoring the largest delGen
/// that has updates for a given docID.
///
/// Equivalent to `DocValuesFieldUpdates.mergedIterator(Iterator[])`.
pub fn merged_iterator(subs: Vec<DocValuesFieldUpdatesIterator<'_>>) -> MergedIterator<'_> {
    if subs.is_empty() {
        return MergedIterator::empty();
    }
    if subs.len() == 1 {
        return MergedIterator::single(subs.into_iter().next().expect("subs non-empty"));
    }
    MergedIterator::merge(subs)
}

/// A merged iterator over multiple `DocValuesFieldUpdatesIterator`s.
///
/// Equivalent to the anonymous `Iterator` created by
/// `DocValuesFieldUpdates.mergedIterator`.
pub struct MergedIterator<'a> {
    // Each sub-iterator with its current docID.
    subs: Vec<MergedSub<'a>>,
    doc: i32,
    // Index into `subs` for the current top.
    top_idx: usize,
}

struct MergedSub<'a> {
    it: DocValuesFieldUpdatesIterator<'a>,
    doc: i32,
}

impl<'a> MergedIterator<'a> {
    fn empty() -> Self {
        Self {
            subs: Vec::new(),
            doc: NO_MORE_DOCS,
            top_idx: 0,
        }
    }

    fn single(mut it: DocValuesFieldUpdatesIterator<'a>) -> Self {
        // Pre-advance to the first doc, consistent with `merge`.
        let doc = it.next_doc().unwrap_or(NO_MORE_DOCS);
        if doc == NO_MORE_DOCS {
            return Self::empty();
        }
        Self {
            subs: vec![MergedSub { it, doc }],
            doc: -1,
            top_idx: 0,
        }
    }

    fn merge(subs: Vec<DocValuesFieldUpdatesIterator<'a>>) -> Self {
        let mut merged_subs: Vec<MergedSub<'a>> = Vec::with_capacity(subs.len());
        for mut s in subs.into_iter() {
            // Pre-advance each sub.
            let doc = s.next_doc().unwrap_or(NO_MORE_DOCS);
            if doc != NO_MORE_DOCS {
                merged_subs.push(MergedSub { it: s, doc });
            }
        }
        Self {
            subs: merged_subs,
            doc: -1,
            top_idx: 0,
        }
    }

    /// Returns the delGen of the current top sub-iterator.
    pub fn del_gen(&self) -> i64 {
        if self.top_idx < self.subs.len() {
            self.subs[self.top_idx].it.del_gen()
        } else {
            -1
        }
    }

    /// Returns `true` if the current document has a value.
    pub fn has_value(&self) -> bool {
        if self.top_idx < self.subs.len() {
            self.subs[self.top_idx].it.has_value()
        } else {
            false
        }
    }

    /// Returns the long value of the current top sub-iterator.
    pub fn long_value(&self) -> i64 {
        self.subs[self.top_idx].it.long_value()
    }

    /// Returns the binary value of the current top sub-iterator.
    pub fn binary_value(&self) -> BytesRef {
        self.subs[self.top_idx].it.binary_value()
    }
}

impl<'a> DocIdSetIterator for MergedIterator<'a> {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        loop {
            if self.subs.is_empty() {
                self.doc = NO_MORE_DOCS;
                return Ok(NO_MORE_DOCS);
            }
            // Find the minimum docID, breaking ties by largest delGen.
            let mut min_idx = 0;
            let mut min_doc = self.subs[0].doc;
            for i in 1..self.subs.len() {
                if self.subs[i].doc < min_doc
                    || (self.subs[i].doc == min_doc
                        && self.subs[i].it.del_gen() > self.subs[min_idx].it.del_gen())
                {
                    min_idx = i;
                    min_doc = self.subs[i].doc;
                }
            }
            if min_doc == NO_MORE_DOCS {
                self.doc = NO_MORE_DOCS;
                return Ok(NO_MORE_DOCS);
            }
            if min_doc != self.doc {
                self.doc = min_doc;
                self.top_idx = min_idx;
                // Advance all NON-TOP subs at this docID past it, so the top
                // sub remains positioned at the current doc for value reads.
                let current = self.doc;
                let mut i = 0;
                while i < self.subs.len() {
                    if i != min_idx && self.subs[i].doc == current {
                        let next = self.subs[i].it.next_doc().unwrap_or(NO_MORE_DOCS);
                        self.subs[i].doc = next;
                        if next == NO_MORE_DOCS {
                            // `remove`, not `swap_remove`: the subs after `i`
                            // shift down by one, which is what keeps `min_idx`
                            // naming the top sub. `swap_remove` would move the
                            // last sub into `i` and leave `min_idx` pointing at
                            // the wrong one, so the value read for this
                            // document would come from a lower generation.
                            self.subs.remove(i);
                            if min_idx > i {
                                min_idx -= 1;
                            }
                            continue;
                        }
                    }
                    i += 1;
                }
                self.top_idx = min_idx;
                return Ok(self.doc);
            }
            // min_doc == self.doc: advance the top sub past the current doc.
            let next = self.subs[min_idx].it.next_doc().unwrap_or(NO_MORE_DOCS);
            self.subs[min_idx].doc = next;
            if next == NO_MORE_DOCS {
                self.subs.swap_remove(min_idx);
            }
        }
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "advance not supported for MergedIterator".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        self.subs.iter().map(|s| s.it.cost()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_updates_finish_and_iterate() {
        let mut updates = DocValuesFieldUpdates::new(100, 5, "count", DocValuesType::NUMERIC);
        updates.add_long(3, 30);
        updates.add_long(1, 10);
        updates.add_long(3, 31); // overwrite doc 3
        updates.finish().unwrap();

        let mut it = updates.iterator();
        assert_eq!(it.next_doc().unwrap(), 1);
        assert!(it.has_value());
        assert_eq!(it.long_value(), 10);

        assert_eq!(it.next_doc().unwrap(), 3);
        assert!(it.has_value());
        assert_eq!(it.long_value(), 31); // last update wins

        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn binary_updates_finish_and_iterate() {
        let mut updates = DocValuesFieldUpdates::new(100, 2, "label", DocValuesType::BINARY);
        updates.add_binary(0, &BytesRef::new(b"hello".to_vec()));
        updates.add_binary(2, &BytesRef::new(b"world".to_vec()));
        updates.finish().unwrap();

        let mut it = updates.iterator();
        assert_eq!(it.next_doc().unwrap(), 0);
        assert_eq!(it.binary_value().slice(), b"hello");
        assert_eq!(it.next_doc().unwrap(), 2);
        assert_eq!(it.binary_value().slice(), b"world");
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn reset_clears_value() {
        let mut updates = DocValuesFieldUpdates::new(100, 1, "count", DocValuesType::NUMERIC);
        updates.add_long(5, 50);
        updates.reset(5); // reset after add — last wins
        updates.finish().unwrap();

        let mut it = updates.iterator();
        assert_eq!(it.next_doc().unwrap(), 5);
        assert!(!it.has_value());
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn finish_twice_errors() {
        let mut updates = DocValuesFieldUpdates::new(100, 1, "f", DocValuesType::NUMERIC);
        updates.add_long(0, 1);
        updates.finish().unwrap();
        assert!(updates.finish().is_err());
    }

    #[test]
    fn any_and_size() {
        let mut updates = DocValuesFieldUpdates::new(100, 1, "f", DocValuesType::NUMERIC);
        assert!(!updates.any());
        assert_eq!(updates.size(), 0);
        updates.add_long(0, 1);
        updates.add_long(1, 2);
        assert!(updates.any());
        assert_eq!(updates.size(), 2);
    }

    #[test]
    fn merged_iterator_favors_largest_del_gen() {
        let mut u1 = DocValuesFieldUpdates::new(100, 1, "f", DocValuesType::NUMERIC);
        u1.add_long(0, 100);
        u1.add_long(2, 200);
        u1.finish().unwrap();

        let mut u2 = DocValuesFieldUpdates::new(100, 5, "f", DocValuesType::NUMERIC);
        u2.add_long(0, 999); // larger delGen, should win for doc 0
        u2.add_long(1, 111);
        u2.finish().unwrap();

        let i1 = u1.iterator();
        let i2 = u2.iterator();
        let mut merged = merged_iterator(vec![i1, i2]);

        assert_eq!(merged.next_doc().unwrap(), 0);
        assert!(merged.has_value());
        assert_eq!(merged.long_value(), 999); // delGen 5 wins

        assert_eq!(merged.next_doc().unwrap(), 1);
        assert_eq!(merged.long_value(), 111);

        assert_eq!(merged.next_doc().unwrap(), 2);
        assert_eq!(merged.long_value(), 200);

        assert_eq!(merged.next_doc().unwrap(), NO_MORE_DOCS);
    }

    /// Five generations, in no particular order, update the same two
    /// documents; the subs run out together on the last one. The highest
    /// generation must win for both, as Java's priority queue picks it
    /// (`DocValuesFieldUpdates.java:162-178`), however the exhausted subs are
    /// removed.
    #[test]
    fn merged_iterator_keeps_the_highest_generation_when_subs_run_out_together() {
        let gens = [6i64, 4, 7, 3, 5];
        let updates: Vec<DocValuesFieldUpdates> = gens
            .iter()
            .map(|&gen| {
                let mut u = DocValuesFieldUpdates::new(2, gen, "f", DocValuesType::NUMERIC);
                u.add_long(0, gen * 10);
                u.add_long(1, gen * 10 + 1);
                u.finish().unwrap();
                u
            })
            .collect();
        let mut merged = merged_iterator(updates.iter().map(|u| u.iterator()).collect());
        assert_eq!(merged.next_doc().unwrap(), 0);
        assert_eq!((merged.del_gen(), merged.long_value()), (7, 70));
        assert_eq!(merged.next_doc().unwrap(), 1);
        assert_eq!((merged.del_gen(), merged.long_value()), (7, 71));
        assert_eq!(merged.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn merged_iterator_single_sub() {
        let mut u = DocValuesFieldUpdates::new(100, 1, "f", DocValuesType::NUMERIC);
        u.add_long(3, 30);
        u.add_long(7, 70);
        u.finish().unwrap();

        let mut merged = merged_iterator(vec![u.iterator()]);
        assert_eq!(merged.next_doc().unwrap(), 3);
        assert_eq!(merged.long_value(), 30);
        assert_eq!(merged.next_doc().unwrap(), 7);
        assert_eq!(merged.long_value(), 70);
        assert_eq!(merged.next_doc().unwrap(), NO_MORE_DOCS);
    }

    /// `grow` follows `AbstractPagedMutable.grow(long)`: a buffer that is
    /// already long enough is left alone, and otherwise it becomes
    /// `size + max(size >> 3, 3)` entries. The port previously added a fixed
    /// 1024 entries at a time, which charged RAM in the wrong steps.
    #[test]
    fn grow_follows_lucenes_paged_mutable_growth() {
        let mut updates = DocValuesFieldUpdates::new(1_000, -1, "f", DocValuesType::NUMERIC);
        // Start from an empty buffer so the policy is visible.
        updates.docs.clear();
        updates.values.clear();

        updates.grow(1);
        assert_eq!(updates.docs.len(), 4, "1 + max(0, 3)");
        assert_eq!(updates.values.len(), updates.docs.len());

        updates.grow(4);
        assert_eq!(updates.docs.len(), 4, "already long enough");

        updates.grow(5);
        assert_eq!(updates.docs.len(), 8, "5 + max(0, 3)");

        updates.grow(64);
        assert_eq!(updates.docs.len(), 72, "64 + max(8, 3)");
    }

    /// `swap` exchanges the doc entry and its value together, which is what
    /// Java's base `swap` plus the numeric and binary overrides do between them.
    #[test]
    fn swap_moves_the_doc_entry_and_its_value_together() {
        let mut updates = DocValuesFieldUpdates::new(10, -1, "f", DocValuesType::NUMERIC);
        updates.add_long(1, 100);
        updates.add_long(2, 200);
        updates.swap(0, 1);
        assert_eq!(updates.docs[0] >> SHIFT, 2);
        assert_eq!(updates.docs[1] >> SHIFT, 1);
        assert!(matches!(updates.values[0], UpdateValue::Long(200)));
        assert!(matches!(updates.values[1], UpdateValue::Long(100)));
    }

    /// The in-place sort must still be stable: with several updates to the same
    /// docID the *last* one has to win, which is what `finish` promises and what
    /// the iterator relies on.
    #[test]
    fn sorting_in_place_keeps_the_last_update_to_a_doc() {
        let mut updates = DocValuesFieldUpdates::new(10, -1, "f", DocValuesType::NUMERIC);
        for (doc, value) in [(5, 1), (1, 2), (5, 3), (3, 4), (1, 5), (5, 6)] {
            updates.add_long(doc, value);
        }
        updates.finish().unwrap();
        let mut it = updates.iterator();
        let mut seen = Vec::new();
        loop {
            let doc = it.next_doc().unwrap();
            if doc == crate::search::NO_MORE_DOCS {
                break;
            }
            seen.push((doc, it.long_value()));
        }
        assert_eq!(seen, vec![(1, 5), (3, 4), (5, 6)]);
    }

    #[test]
    fn ram_bytes_used_is_positive() {
        let mut u = DocValuesFieldUpdates::new(100, 1, "f", DocValuesType::NUMERIC);
        u.add_long(0, 1);
        u.finish().unwrap();
        assert!(u.ram_bytes_used() > 0);
    }
}
