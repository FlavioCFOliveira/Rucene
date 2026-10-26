//! Doc-values range and set queries, ported from `org.apache.lucene.document`.
//!
//! These answer a range or a set membership by scanning doc values, which beats
//! a points lookup when the field is already being read for another reason, or
//! when no points index exists.

use std::any::Any;
use std::fmt::Debug;
use std::sync::Arc;

use crate::document::range_bulk_scorer::RangeBulkScorer;
use crate::error::Result;
use crate::index::doc_values::DocValues;
use crate::index::doc_values::DocValuesSkipper;
use crate::index::doc_values::{SortedNumericDocValues, SortedSetDocValues};
use crate::index::leaf_reader::LeafReader;
use crate::index::reader_context::LeafReaderContext;
use crate::search::bulk_scorer::BulkScorer;
use crate::search::collector::LeafCollector;
use crate::search::constant_score_scorer::ConstantScoreScorer;
use crate::search::constant_score_scorer_supplier::ConstantScoreScorerSupplier;
use crate::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::doc_id_set_iterator::NO_MORE_DOCS;
use crate::search::doc_values_range_iterator::DocValuesRangeIterator;
use crate::search::field_exists_query::FieldExistsQuery;
use crate::search::index_searcher::IndexSearcher;
use crate::search::match_all_docs_query::MatchAllDocsQuery;
use crate::search::match_no_docs_query::MatchNoDocsQuery;
use crate::search::numeric_field_stats::NumericFieldStats;
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::two_phase_iterator::{ScorerIterator, TwoPhaseIterator};
use crate::search::weight::Weight;
use crate::util::packed::PackedInts;
use crate::util::Bits;
use crate::util::BytesRef;
use crate::util::{Accountable, RamUsageEstimator};

/// Sentinel standing for a value the table cannot hold, which is also the
/// smallest long.
const MISSING: i64 = i64::MIN;

/// An open-addressed set of longs, built once from a sorted array.
///
/// Equivalent to `org.apache.lucene.document.DocValuesLongHashSet`, which a set
/// query uses to test membership without a per-document allocation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DocValuesLongHashSet {
    table: Vec<i64>,
    mask: usize,
    /// Whether `i64::MIN` itself is a member, which the table cannot store.
    has_missing_value: bool,
    size: usize,
    min_value: i64,
    max_value: i64,
}

impl DocValuesLongHashSet {
    /// Builds the set from `values`, which must be sorted ascending.
    ///
    /// Equivalent to the `DocValuesLongHashSet(long[])` constructor, which
    /// sizes the table as `1 << PackedInts.bitsRequired(values.length * 3 / 2)`.
    ///
    /// That is deliberately *not* "round 1.5x up to a power of two": for a
    /// `wanted` that is already a power of two, `bitsRequired` still needs one
    /// more bit, so the table lands on the next power of two **strictly above**
    /// it. Lucene asserts `tableSize >= values.length * 3 / 2` and relies on the
    /// strictness: it is what guarantees the table always keeps a free slot, and
    /// so what makes the linear probe in [`add`](Self::add) terminate. Rounding
    /// with `next_power_of_two` instead would size a one-element set at a
    /// single slot, filling the table completely and leaving `add` unable to
    /// finish — besides reporting a different `ram_bytes_used` than Java for
    /// every input whose `3n/2` is a power of two.
    ///
    /// Java computes the intermediate with `Math.toIntExact`, which would throw
    /// on a slice of more than about 1.4 billion values; a `usize` count cannot
    /// overflow the `i64` used here on any target this crate supports.
    pub fn new(values: &[i64]) -> Self {
        let wanted = values.len() as i64 * 3 / 2;
        let table_size = 1usize
            << PackedInts::bits_required(wanted)
                .expect("INVARIANT: a length times 3/2 is never negative");
        let mut set = Self {
            table: vec![MISSING; table_size],
            mask: table_size - 1,
            has_missing_value: false,
            size: 0,
            min_value: i64::MAX,
            max_value: i64::MIN,
        };
        for &value in values {
            if value == MISSING {
                if !set.has_missing_value {
                    set.size += 1;
                }
                set.has_missing_value = true;
            } else if set.add(value) {
                set.size += 1;
            }
        }
        if !values.is_empty() {
            set.min_value = values[0];
            set.max_value = values[values.len() - 1];
        }
        set
    }

    /// Inserts `value` into the table, returning whether it was new.
    ///
    /// Equivalent to the private `DocValuesLongHashSet.add(long)`, which the
    /// constructor calls once per input value. Like Java's, it assumes `value`
    /// is not the [`MISSING`] sentinel, which the caller filters out.
    ///
    /// The linear probe is unbounded, exactly as Java's is. It terminates
    /// because [`new`](Self::new) sizes the table strictly above `3n/2`, so a
    /// free slot always remains after the constructor has inserted every value.
    /// Calling this on a table with no free slot would never return — which is
    /// why the sizing must stay faithful to Lucene's.
    fn add(&mut self, value: i64) -> bool {
        let mut slot = (hash_long(value) as usize) & self.mask;
        loop {
            if self.table[slot] == MISSING {
                self.table[slot] = value;
                return true;
            }
            if self.table[slot] == value {
                return false;
            }
            slot = (slot + 1) & self.mask;
        }
    }

    /// Returns whether `value` is a member.
    ///
    /// Equivalent to `DocValuesLongHashSet.contains(long)`.
    pub fn contains(&self, value: i64) -> bool {
        if value == MISSING {
            return self.has_missing_value;
        }
        if value < self.min_value || value > self.max_value {
            return false;
        }
        let mut slot = (hash_long(value) as usize) & self.mask;
        loop {
            match self.table[slot] {
                MISSING => return false,
                v if v == value => return true,
                _ => slot = (slot + 1) & self.mask,
            }
        }
    }

    /// Returns how many values the set holds.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Returns the smallest member.
    pub fn min_value(&self) -> i64 {
        self.min_value
    }

    /// Returns the largest member.
    pub fn max_value(&self) -> i64 {
        self.max_value
    }

    /// Iterates the members in table order.
    ///
    /// Equivalent to `DocValuesLongHashSet.stream()`, which filters the
    /// sentinel out of the table and, when the sentinel is itself a member,
    /// prepends it. The order is therefore the hash table's layout, not the
    /// insertion order and not sorted — and it is exactly the order
    /// [`Display`](std::fmt::Display) prints.
    pub fn stream(&self) -> impl Iterator<Item = i64> + '_ {
        self.has_missing_value
            .then_some(MISSING)
            .into_iter()
            .chain(self.table.iter().copied().filter(|v| *v != MISSING))
    }
}

impl std::fmt::Display for DocValuesLongHashSet {
    /// Equivalent to `DocValuesLongHashSet.toString()`, which joins
    /// [`stream`](Self::stream) with `", "` between `[` and `]`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[")?;
        for (i, value) in self.stream().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{value}")?;
        }
        f.write_str("]")
    }
}

impl Accountable for DocValuesLongHashSet {
    /// Returns the heap the set occupies: the object itself plus its table.
    ///
    /// Equivalent to `DocValuesLongHashSet.ramBytesUsed()`, which charges
    /// `BASE_RAM_BYTES` — the shallow size of the instance — plus
    /// `RamUsageEstimator.sizeOfObject(table)`.
    fn ram_bytes_used(&self) -> i64 {
        RamUsageEstimator::shallow_size_of_instance::<Self>()
            + RamUsageEstimator::size_of_long(&self.table)
    }
}

/// Hashes a long the way `Long.hashCode` does, so the probe sequence matches
/// Java's.
fn hash_long(value: i64) -> i32 {
    ((value ^ (value >> 32)) & 0xFFFF_FFFF) as i32
}

/// A range query over a numeric doc-values field.
///
/// Equivalent to
/// `org.apache.lucene.document.SortedNumericDocValuesRangeQuery`: a `Query`
/// whose scorer supplier reads the segment's doc values, using the doc-values
/// skipper when the field has one.
#[derive(Clone, Debug)]
pub struct SortedNumericDocValuesRangeQuery {
    field: String,
    lower_value: i64,
    upper_value: i64,
}

impl SortedNumericDocValuesRangeQuery {
    /// Creates the query over `[lower_value, upper_value]`, inclusive.
    pub fn new(field: impl Into<String>, lower_value: i64, upper_value: i64) -> Self {
        Self {
            field: field.into(),
            lower_value,
            upper_value,
        }
    }

    /// Returns the field the query reads.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns the lower bound.
    pub fn lower_value(&self) -> i64 {
        self.lower_value
    }

    /// Returns the upper bound.
    pub fn upper_value(&self) -> i64 {
        self.upper_value
    }

    /// Returns whether the query matches nothing, which an inverted range does.
    pub fn matches_no_docs(&self) -> bool {
        self.lower_value > self.upper_value
    }

    /// Returns whether `value` falls in the range.
    pub fn matches(&self, value: i64) -> bool {
        value >= self.lower_value && value <= self.upper_value
    }
}

/// A set-membership query over a numeric doc-values field.
///
/// Equivalent to `org.apache.lucene.document.SortedNumericDocValuesSetQuery`.
/// The same divergence as the range query above applies.
#[derive(Clone, Debug)]
pub struct SortedNumericDocValuesSetQuery {
    field: String,
    values: DocValuesLongHashSet,
}

impl SortedNumericDocValuesSetQuery {
    /// Creates the query from `values`, which must be sorted ascending.
    pub fn new(field: impl Into<String>, values: &[i64]) -> Self {
        Self {
            field: field.into(),
            values: DocValuesLongHashSet::new(values),
        }
    }

    /// Returns the field the query reads.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns whether `value` is in the set.
    pub fn matches(&self, value: i64) -> bool {
        self.values.contains(value)
    }

    /// Returns the set the query tests against.
    pub fn values(&self) -> &DocValuesLongHashSet {
        &self.values
    }
}

impl Accountable for SortedNumericDocValuesSetQuery {
    /// Returns the heap the query occupies: the object itself, its field name
    /// and the set of values it tests against.
    ///
    /// Equivalent to `SortedNumericDocValuesSetQuery.ramBytesUsed()`, which
    /// charges `BASE_RAM_BYTES` plus `sizeOfObject(field)` plus
    /// `sizeOfObject(numbers)`. Java reaches the set's own `ramBytesUsed`
    /// through the `Accountable` branch of `sizeOfObject`; the call here is
    /// direct, and is the same number.
    fn ram_bytes_used(&self) -> i64 {
        RamUsageEstimator::shallow_size_of_instance::<Self>()
            + RamUsageEstimator::size_of_string(&self.field)
            + self.values.ram_bytes_used()
    }
}

/// A range query over a sorted-set doc-values field, comparing terms as bytes.
///
/// Equivalent to `org.apache.lucene.document.SortedSetDocValuesRangeQuery`.
/// The same divergence as the numeric range query above applies.
#[derive(Clone, Debug)]
pub struct SortedSetDocValuesRangeQuery {
    field: String,
    lower_value: Option<BytesRef>,
    upper_value: Option<BytesRef>,
    lower_inclusive: bool,
    upper_inclusive: bool,
}

impl SortedSetDocValuesRangeQuery {
    /// Creates the query. A `None` bound is open on that side.
    pub fn new(
        field: impl Into<String>,
        lower_value: Option<BytesRef>,
        upper_value: Option<BytesRef>,
        lower_inclusive: bool,
        upper_inclusive: bool,
    ) -> Self {
        Self {
            field: field.into(),
            // An exclusive open bound is the same as an inclusive one, as Java
            // normalises in its constructor.
            lower_inclusive: lower_inclusive || lower_value.is_none(),
            upper_inclusive: upper_inclusive || upper_value.is_none(),
            lower_value,
            upper_value,
        }
    }

    /// Returns the field the query reads.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns whether `term` falls in the range.
    pub fn matches(&self, term: &BytesRef) -> bool {
        if let Some(lower) = &self.lower_value {
            let cmp = term.slice().cmp(lower.slice());
            let ok = if self.lower_inclusive {
                cmp != std::cmp::Ordering::Less
            } else {
                cmp == std::cmp::Ordering::Greater
            };
            if !ok {
                return false;
            }
        }
        if let Some(upper) = &self.upper_value {
            let cmp = term.slice().cmp(upper.slice());
            let ok = if self.upper_inclusive {
                cmp != std::cmp::Ordering::Greater
            } else {
                cmp == std::cmp::Ordering::Less
            };
            if !ok {
                return false;
            }
        }
        true
    }
}

/// Decides whether a doc-values range query can use the field's skipper, and
/// what it can skip.
///
/// Equivalent to `org.apache.lucene.document.SortedSkipperScorerSupplier`,
/// which reads the per-block minimum and maximum a doc-values skipper records
/// and drops a block whose whole range falls outside the query's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipperRelation {
    /// Every document of the block matches, so no value need be read.
    AllMatch,
    /// No document of the block matches, so the block can be skipped whole.
    NoneMatch,
    /// Some may match, so the block must be scanned.
    SomeMatch,
}

/// Chooses how a block relates to a query range from the block's own bounds.
///
/// Equivalent to the block-level decision inside `SortedSkipperScorerSupplier`.
pub fn skipper_relation(
    block_min: i64,
    block_max: i64,
    query_min: i64,
    query_max: i64,
) -> SkipperRelation {
    if block_min > query_max || block_max < query_min {
        SkipperRelation::NoneMatch
    } else if block_min >= query_min && block_max <= query_max {
        SkipperRelation::AllMatch
    } else {
        SkipperRelation::SomeMatch
    }
}

/// Returns the sorted-set ordinals a range spans, given the field's term
/// dictionary bounds.
///
/// Equivalent to the ordinal lookup `SortedSetDocValuesRangeQuery` performs
/// before it scans: a range that spans no ordinal matches nothing.
pub fn ordinal_range(min_ord: i64, max_ord: i64) -> Option<(i64, i64)> {
    if min_ord > max_ord {
        None
    } else {
        Some((min_ord, max_ord))
    }
}

// ---------------------------------------------------------------------------
// The queries themselves
// ---------------------------------------------------------------------------

/// The weight of a doc-values query, built over the query's own scorer
/// supplier.
///
/// Equivalent to the anonymous `ConstantScoreWeight` each of the three queries
/// below creates in `createWeight`. Rust has no anonymous classes, so the three
/// share one weight body parameterised by a [`DocValuesScorerSource`].
#[derive(Debug)]
struct DocValuesWeightImpl<S: DocValuesScorerSource> {
    source: S,
    score: f32,
    score_mode: ScoreMode,
}

/// What a doc-values query contributes to its weight: the field it reads and
/// the iterator it wants over a leaf.
trait DocValuesScorerSource: Debug + Send + Sync + 'static {
    /// The field the query reads, which decides cacheability.
    fn field(&self) -> &str;

    /// Returns the iterator over the leaf's matching documents, or `None` when
    /// nothing in the leaf can match.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while opening the doc values.
    fn iterator(&self, reader: &dyn LeafReader) -> Result<Option<ScorerIterator>>;

    /// Counts the matching documents without iterating, or returns `-1`.
    ///
    /// Equivalent to the `count(LeafReaderContext)` override, which only
    /// `SortedNumericDocValuesRangeQuery` has.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the skipper.
    fn count(&self, _reader: &dyn LeafReader) -> Result<i32> {
        Ok(-1)
    }

    /// Returns the leaf's scorer supplier, or `None` when nothing in the leaf
    /// can match.
    ///
    /// The default wraps [`iterator`](Self::iterator) in a
    /// [`ConstantScoreScorerSupplier`]; the two range queries override it to
    /// hand out a [`SortedSkipperScorerSupplier`] when the leaf is sorted by
    /// their field, as their `scorerSupplier` does in Java.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while opening the doc values.
    fn scorer_supplier(
        &self,
        reader: &dyn LeafReader,
        score: f32,
        score_mode: ScoreMode,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let max_doc = reader.max_doc();
        Ok(self.iterator(reader)?.map(|iterator| {
            Box::new(ConstantScoreScorerSupplier::from_iterator(
                iterator, score, score_mode, max_doc,
            )) as Box<dyn ScorerSupplier>
        }))
    }
}

impl<S: DocValuesScorerSource> ConstantScoreWeightImpl for DocValuesWeightImpl<S> {
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let reader = context.leaf_reader();
        if reader
            .get_field_infos()
            .field_info(self.source.field())
            .is_none()
        {
            return Ok(None);
        }
        self.source
            .scorer_supplier(reader.as_ref(), self.score, self.score_mode)
    }

    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        DocValues::is_cacheable(ctx, &[self.source.field()])
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        let reader = context.leaf_reader();
        let max_doc = reader.max_doc();
        let cnt = self.source.count(reader.as_ref())?;
        if cnt == max_doc {
            // Return the reader's numDocs, which accounts for deleted documents
            // as well.
            return Ok(reader.num_docs());
        }
        Ok(cnt)
    }
}

/// Counts the documents of a leaf whose values all fall inside `[lower, upper]`,
/// ignoring deletes, or returns `-1` when the count cannot be had cheaply.
///
/// Equivalent to the private
/// `SortedNumericDocValuesRangeQuery…docCountIgnoringDeletes(LeafReaderContext)`.
fn doc_count_ignoring_deletes(
    reader: &dyn LeafReader,
    field: &str,
    lower_value: i64,
    upper_value: i64,
) -> Result<i32> {
    if let Some(skipper) = reader.get_doc_values_skipper(field)? {
        if skipper.global_min_value() > upper_value || skipper.global_max_value() < lower_value {
            return Ok(0);
        }
        if skipper.global_doc_count() == reader.max_doc()
            && skipper.global_min_value() >= lower_value
            && skipper.global_max_value() <= upper_value
        {
            return Ok(reader.max_doc());
        }
    }
    Ok(-1)
}

/// The scorer source of [`SortedNumericDocValuesRangeQuery`].
#[derive(Clone, Debug)]
struct SortedNumericRangeSource {
    field: String,
    lower_value: i64,
    upper_value: i64,
}

impl DocValuesScorerSource for SortedNumericRangeSource {
    fn field(&self) -> &str {
        &self.field
    }

    fn iterator(&self, reader: &dyn LeafReader) -> Result<Option<ScorerIterator>> {
        let count =
            doc_count_ignoring_deletes(reader, &self.field, self.lower_value, self.upper_value)?;
        self.iterator_with_count(reader, count)
    }

    fn count(&self, reader: &dyn LeafReader) -> Result<i32> {
        doc_count_ignoring_deletes(reader, &self.field, self.lower_value, self.upper_value)
    }

    /// Equivalent to `SortedNumericDocValuesRangeQuery…scorerSupplier`
    /// (`SortedNumericDocValuesRangeQuery.java:123-160`): after the two
    /// counting short cuts, a single-valued field whose leaf is sorted by it
    /// first, densely, is served by a [`SortedSkipperScorerSupplier`].
    fn scorer_supplier(
        &self,
        reader: &dyn LeafReader,
        score: f32,
        score_mode: ScoreMode,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let max_doc = reader.max_doc();
        let count =
            doc_count_ignoring_deletes(reader, &self.field, self.lower_value, self.upper_value)?;
        if count != 0 && count != max_doc {
            let values = DocValues::get_sorted_numeric(reader, &self.field)?;
            if let Some(singleton) = DocValues::unwrap_singleton_numeric(values) {
                if let Some(skipper) = reader.get_doc_values_skipper(&self.field)? {
                    if let Some(primary_sort_field) =
                        dense_primary_sort(reader, skipper.as_ref(), &self.field)
                    {
                        return Ok(Some(Box::new(SortedSkipperScorerSupplier::new(
                            skipper,
                            primary_sort_field.reverse(),
                            score,
                            score_mode,
                            Box::new(NumericSkipperRange {
                                lower_value: self.lower_value,
                                upper_value: self.upper_value,
                                singleton,
                            }),
                        ))));
                    }
                }
            }
        }
        Ok(self.iterator_with_count(reader, count)?.map(|iterator| {
            Box::new(ConstantScoreScorerSupplier::from_iterator(
                iterator, score, score_mode, max_doc,
            )) as Box<dyn ScorerSupplier>
        }))
    }
}

impl SortedNumericRangeSource {
    /// The iterator of the leaf once its document count is known.
    fn iterator_with_count(
        &self,
        reader: &dyn LeafReader,
        count: i32,
    ) -> Result<Option<ScorerIterator>> {
        let max_doc = reader.max_doc();
        if count == 0 {
            return Ok(None);
        }
        if count == max_doc {
            return Ok(Some(ScorerIterator::Simple(Box::new(
                crate::search::doc_id_set_iterator::all(max_doc)?,
            ))));
        }

        let values = DocValues::get_sorted_numeric(reader, &self.field)?;
        let skipper = reader.get_doc_values_skipper(&self.field)?;
        // A single two-phase iterator covers every density: its approximation
        // rides the skipper, so it never over-scans.
        match DocValues::unwrap_singleton_numeric(values) {
            Some(singleton) => Ok(Some(ScorerIterator::TwoPhase(Box::new(
                DocValuesRangeIterator::for_range(
                    singleton,
                    skipper,
                    self.lower_value,
                    self.upper_value,
                ),
            )))),
            None => {
                // `unwrap_singleton_numeric` consumed the values, so they are
                // reopened for the multi-valued path; Java can simply keep the
                // reference it tested.
                let values = DocValues::get_sorted_numeric(reader, &self.field)?;
                Ok(Some(ScorerIterator::TwoPhase(Box::new(
                    DocValuesRangeIterator::for_sorted_numeric_range(
                        values,
                        skipper,
                        self.lower_value,
                        self.upper_value,
                    ),
                ))))
            }
        }
    }
}

/// The [`SkipperRangeSource`] of [`SortedNumericDocValuesRangeQuery`].
///
/// Equivalent to the anonymous `SortedSkipperScorerSupplier` subclass of
/// `getScorerSupplierFromDensePrimarySort(NumericDocValues, DocValuesSkipper,
/// SortField)` (`SortedNumericDocValuesRangeQuery.java:193-219`).
struct NumericSkipperRange {
    lower_value: i64,
    upper_value: i64,
    singleton: Box<dyn crate::index::doc_values::NumericDocValues>,
}

impl Debug for NumericSkipperRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NumericSkipperRange")
            .field("lower_value", &self.lower_value)
            .field("upper_value", &self.upper_value)
            .finish_non_exhaustive()
    }
}

impl SkipperRangeSource for NumericSkipperRange {
    fn lower_value(&mut self) -> Result<i64> {
        Ok(self.lower_value)
    }

    fn upper_value(&mut self) -> Result<i64> {
        Ok(self.upper_value)
    }

    fn next_doc(&mut self, start_doc_id: i32, predicate: &dyn Fn(i64) -> bool) -> Result<i32> {
        let mut doc = self.singleton.doc_id();
        if start_doc_id > doc {
            doc = self.singleton.advance(start_doc_id)?;
        }
        while doc < NO_MORE_DOCS {
            if predicate(self.singleton.long_value()?) {
                break;
            }
            doc = self.singleton.next_doc()?;
        }
        Ok(doc)
    }
}

/// The [`SkipperRangeSource`] of [`SortedSetDocValuesRangeQuery`].
///
/// Equivalent to the anonymous `SortedSkipperScorerSupplier` subclass of
/// `getScorerSupplierFromDensePrimarySort(SortedDocValues, SortedSetDocValues,
/// DocValuesSkipper, SortField)` (`SortedSetDocValuesRangeQuery.java:198-234`):
/// the ordinal bounds are looked up once, on first use.
struct OrdinalSkipperRange {
    range: SortedSetRangeSource,
    singleton: Box<dyn crate::index::doc_values::SortedDocValues>,
    values: Box<dyn SortedSetDocValues>,
    min_ord: i64,
    max_ord: i64,
}

impl Debug for OrdinalSkipperRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrdinalSkipperRange")
            .field("min_ord", &self.min_ord)
            .field("max_ord", &self.max_ord)
            .finish_non_exhaustive()
    }
}

impl SkipperRangeSource for OrdinalSkipperRange {
    fn lower_value(&mut self) -> Result<i64> {
        if self.min_ord == -1 {
            self.min_ord = self.range.min_ord(self.values.as_ref())?;
        }
        Ok(self.min_ord)
    }

    fn upper_value(&mut self) -> Result<i64> {
        if self.max_ord == -1 {
            self.max_ord = self.range.max_ord(self.values.as_ref())?;
        }
        Ok(self.max_ord)
    }

    fn next_doc(&mut self, start_doc_id: i32, predicate: &dyn Fn(i64) -> bool) -> Result<i32> {
        let mut doc = self.singleton.doc_id();
        if start_doc_id > doc {
            doc = self.singleton.advance(start_doc_id)?;
        }
        while doc < NO_MORE_DOCS {
            if predicate(i64::from(self.singleton.ord_value()?)) {
                break;
            }
            doc = self.singleton.next_doc()?;
        }
        Ok(doc)
    }
}

/// Returns the leaf's primary sort field when it orders by `field` and the
/// skipper covers every document.
///
/// Equivalent to the private `densePrimarySort(LeafReader, DocValuesSkipper)`
/// that both doc-values range queries declare: the skipper-driven interval is
/// only valid when the documents are already ordered by this very field.
fn dense_primary_sort(
    reader: &dyn LeafReader,
    skipper: &dyn DocValuesSkipper,
    field: &str,
) -> Option<crate::search::SortField> {
    if skipper.global_doc_count() != reader.max_doc() {
        return None;
    }
    let meta_data = reader.get_meta_data();
    let index_sort = meta_data.sort()?;
    let first = index_sort.fields().first()?;
    if first.field() != Some(field) {
        return None;
    }
    Some(first.clone())
}

impl SortedNumericDocValuesRangeQuery {
    /// The scorer source this query hands to its weight.
    fn source(&self) -> SortedNumericRangeSource {
        SortedNumericRangeSource {
            field: self.field.clone(),
            lower_value: self.lower_value,
            upper_value: self.upper_value,
        }
    }
}

impl Query for SortedNumericDocValuesRangeQuery {
    fn to_query_string(&self, field: &str) -> String {
        let mut b = String::new();
        if self.field != field {
            b.push_str(&self.field);
            b.push(':');
        }
        b.push('[');
        b.push_str(&self.lower_value.to_string());
        b.push_str(" TO ");
        b.push_str(&self.upper_value.to_string());
        b.push(']');
        b
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(&self.field) {
            visitor.visit_leaf(self);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        if self.lower_value == i64::MIN && self.upper_value == i64::MAX {
            return Ok(Some(Arc::new(FieldExistsQuery::new(&self.field))));
        }
        if self.lower_value > self.upper_value {
            return Ok(Some(Arc::new(MatchNoDocsQuery::new(""))));
        }
        let reader = Arc::clone(searcher.get_index_reader());
        if let Some(stats) = NumericFieldStats::get_stats(Arc::clone(&reader), &self.field)? {
            if self.lower_value > stats.max || self.upper_value < stats.min {
                return Ok(Some(Arc::new(MatchNoDocsQuery::new(""))));
            }
            if self.lower_value <= stats.min
                && self.upper_value >= stats.max
                && stats.doc_count == reader.max_doc()
            {
                return Ok(Some(Arc::new(MatchAllDocsQuery)));
            }
        }
        Ok(None)
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        match other.as_any().downcast_ref::<Self>() {
            Some(other) => {
                self.field == other.field
                    && self.lower_value == other.lower_value
                    && self.upper_value == other.upper_value
            }
            None => false,
        }
    }

    fn query_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.class_hash().hash(&mut hasher);
        self.field.hash(&mut hasher);
        self.lower_value.hash(&mut hasher);
        self.upper_value.hash(&mut hasher);
        hasher.finish()
    }

    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(ConstantScoreWeight::new(
            Arc::new(self.clone()),
            boost,
            DocValuesWeightImpl {
                source: self.source(),
                score: boost,
                score_mode,
            },
        )))
    }
}

/// The scorer source of [`SortedNumericDocValuesSetQuery`].
#[derive(Clone, Debug)]
struct SortedNumericSetSource {
    field: String,
    numbers: DocValuesLongHashSet,
}

/// Confirms that one of a document's numeric values is in the set.
///
/// Equivalent to the anonymous `TwoPhaseIterator` of
/// `SortedNumericDocValuesSetQuery.createWeight`.
struct SortedNumericSetIterator {
    values: Box<dyn SortedNumericDocValues>,
    numbers: DocValuesLongHashSet,
    /// Whether the values are single-valued, in which case one comparison
    /// suffices.
    singleton: bool,
}

impl Debug for SortedNumericSetIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortedNumericSetIterator")
            .field("numbers", &self.numbers)
            .field("singleton", &self.singleton)
            .finish_non_exhaustive()
    }
}

impl TwoPhaseIterator for SortedNumericSetIterator {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        self.values.as_mut()
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        self.values.as_ref()
    }

    fn matches(&mut self) -> Result<bool> {
        let count = self.values.doc_value_count()?;
        for _ in 0..count {
            let value = self.values.next_value()?;
            if value < self.numbers.min_value() {
                continue;
            } else if value > self.numbers.max_value() {
                // Values are sorted, so nothing later can match.
                return Ok(false);
            } else if self.numbers.contains(value) {
                return Ok(true);
            }
            if self.singleton {
                break;
            }
        }
        Ok(false)
    }

    fn match_cost(&self) -> f32 {
        // 2 comparisons, plus a possible lookup in the set.
        5.0
    }
}

impl DocValuesScorerSource for SortedNumericSetSource {
    fn field(&self) -> &str {
        &self.field
    }

    fn iterator(&self, reader: &dyn LeafReader) -> Result<Option<ScorerIterator>> {
        let values = DocValues::get_sorted_numeric(reader, &self.field)?;
        // Java tests for a singleton so it can spend one comparison instead of
        // walking the value list; the walk over a single value is the same work,
        // so the flag records which case this is rather than splitting the
        // iterator in two.
        let singleton = values.into_singleton().is_some();
        let values = DocValues::get_sorted_numeric(reader, &self.field)?;
        Ok(Some(ScorerIterator::TwoPhase(Box::new(
            SortedNumericSetIterator {
                values,
                numbers: self.numbers.clone(),
                singleton,
            },
        ))))
    }
}

impl Query for SortedNumericDocValuesSetQuery {
    fn to_query_string(&self, _default_field: &str) -> String {
        format!("{}: {:?}", self.field, self.values)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(&self.field) {
            visitor.visit_leaf(self);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, _searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        if self.values.size() == 0 {
            return Ok(Some(Arc::new(MatchNoDocsQuery::new(""))));
        }
        Ok(None)
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        match other.as_any().downcast_ref::<Self>() {
            Some(other) => self.field == other.field && self.values == other.values,
            None => false,
        }
    }

    fn query_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.class_hash().hash(&mut hasher);
        self.field.hash(&mut hasher);
        self.values.hash(&mut hasher);
        hasher.finish()
    }

    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(ConstantScoreWeight::new(
            Arc::new(self.clone()),
            boost,
            DocValuesWeightImpl {
                source: SortedNumericSetSource {
                    field: self.field.clone(),
                    numbers: self.values.clone(),
                },
                score: boost,
                score_mode,
            },
        )))
    }
}

/// The scorer source of [`SortedSetDocValuesRangeQuery`].
#[derive(Clone, Debug)]
struct SortedSetRangeSource {
    field: String,
    lower_value: Option<BytesRef>,
    upper_value: Option<BytesRef>,
    lower_inclusive: bool,
    upper_inclusive: bool,
}

impl SortedSetRangeSource {
    /// The lowest ordinal the range covers.
    ///
    /// Equivalent to the private
    /// `SortedSetDocValuesRangeQuery.minOrd(SortedSetDocValues)`.
    fn min_ord(&self, values: &dyn SortedSetDocValues) -> Result<i64> {
        let Some(lower) = self.lower_value.as_ref() else {
            return Ok(0);
        };
        let ord = values.lookup_term(lower)?;
        Ok(if ord < 0 {
            -1 - ord
        } else if self.lower_inclusive {
            ord
        } else {
            ord + 1
        })
    }

    /// The highest ordinal the range covers.
    ///
    /// Equivalent to the private
    /// `SortedSetDocValuesRangeQuery.maxOrd(SortedSetDocValues)`.
    fn max_ord(&self, values: &dyn SortedSetDocValues) -> Result<i64> {
        let Some(upper) = self.upper_value.as_ref() else {
            return Ok(values.get_value_count()? - 1);
        };
        let ord = values.lookup_term(upper)?;
        Ok(if ord < 0 {
            -2 - ord
        } else if self.upper_inclusive {
            ord
        } else {
            ord - 1
        })
    }
}

impl DocValuesScorerSource for SortedSetRangeSource {
    fn field(&self) -> &str {
        &self.field
    }

    fn iterator(&self, reader: &dyn LeafReader) -> Result<Option<ScorerIterator>> {
        let skipper = reader.get_doc_values_skipper(&self.field)?;
        let values = DocValues::get_sorted_set(reader, &self.field)?;
        let min_ord = self.min_ord(values.as_ref())?;
        let max_ord = self.max_ord(values.as_ref())?;

        // No term matched in this segment.
        if min_ord > max_ord {
            return Ok(None);
        }
        if let Some(skipper) = skipper.as_ref() {
            if min_ord > skipper.global_max_value() || max_ord < skipper.global_min_value() {
                return Ok(None);
            }
            // Every term matched in this segment.
            if skipper.global_doc_count() == reader.max_doc()
                && skipper.global_min_value() >= min_ord
                && skipper.global_max_value() <= max_ord
            {
                return Ok(Some(ScorerIterator::Simple(Box::new(
                    crate::search::doc_id_set_iterator::all(skipper.global_doc_count())?,
                ))));
            }
        }

        // A single two-phase iterator covers every density: its approximation
        // rides the skipper, so it never over-scans.
        match DocValues::unwrap_singleton_sorted(values) {
            Some(singleton) => Ok(Some(ScorerIterator::TwoPhase(Box::new(
                DocValuesRangeIterator::for_ordinal_range(singleton, skipper, min_ord, max_ord),
            )))),
            None => {
                // The singleton test consumed the values, so they are reopened
                // for the multi-valued path; Java keeps the reference it tested.
                let values = DocValues::get_sorted_set(reader, &self.field)?;
                Ok(Some(ScorerIterator::TwoPhase(Box::new(
                    DocValuesRangeIterator::for_sorted_set_ordinal_range(
                        values, skipper, min_ord, max_ord,
                    ),
                ))))
            }
        }
    }

    /// Equivalent to `SortedSetDocValuesRangeQuery…scorerSupplier`
    /// (`SortedSetDocValuesRangeQuery.java:116-130`): a single-valued field
    /// whose leaf is sorted by it first, densely, is served by a
    /// [`SortedSkipperScorerSupplier`] before any ordinal is looked up.
    fn scorer_supplier(
        &self,
        reader: &dyn LeafReader,
        score: f32,
        score_mode: ScoreMode,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        if let Some(skipper) = reader.get_doc_values_skipper(&self.field)? {
            let values = DocValues::get_sorted_set(reader, &self.field)?;
            if let Some(singleton) = DocValues::unwrap_singleton_sorted(values) {
                if let Some(primary_sort_field) =
                    dense_primary_sort(reader, skipper.as_ref(), &self.field)
                {
                    // `unwrap_singleton_sorted` consumed the values, so they
                    // are reopened for the ordinal lookups; Java keeps the
                    // reference it tested.
                    let values = DocValues::get_sorted_set(reader, &self.field)?;
                    return Ok(Some(Box::new(SortedSkipperScorerSupplier::new(
                        skipper,
                        primary_sort_field.reverse(),
                        score,
                        score_mode,
                        Box::new(OrdinalSkipperRange {
                            range: self.clone(),
                            singleton,
                            values,
                            min_ord: -1,
                            max_ord: -1,
                        }),
                    ))));
                }
            }
        }
        let max_doc = reader.max_doc();
        Ok(self.iterator(reader)?.map(|iterator| {
            Box::new(ConstantScoreScorerSupplier::from_iterator(
                iterator, score, score_mode, max_doc,
            )) as Box<dyn ScorerSupplier>
        }))
    }

    fn count(&self, reader: &dyn LeafReader) -> Result<i32> {
        if reader.get_field_infos().field_info(&self.field).is_none() {
            return Ok(0);
        }
        let values = DocValues::get_sorted_set(reader, &self.field)?;
        let min_ord = self.min_ord(values.as_ref())?;
        let max_ord = self.max_ord(values.as_ref())?;
        if min_ord > max_ord {
            return Ok(0);
        }
        if let Some(skipper) = reader.get_doc_values_skipper(&self.field)? {
            if min_ord > skipper.global_max_value() || max_ord < skipper.global_min_value() {
                return Ok(0);
            }
            if skipper.global_doc_count() == reader.max_doc()
                && skipper.global_min_value() >= min_ord
                && skipper.global_max_value() <= max_ord
            {
                return Ok(reader.max_doc());
            }
        }
        Ok(-1)
    }
}

impl Query for SortedSetDocValuesRangeQuery {
    fn to_query_string(&self, field: &str) -> String {
        let mut b = String::new();
        if self.field != field {
            b.push_str(&self.field);
            b.push(':');
        }
        b.push(if self.lower_inclusive { '[' } else { '{' });
        match self.lower_value.as_ref() {
            None => b.push('*'),
            Some(v) => b.push_str(&v.to_string()),
        }
        b.push_str(" TO ");
        match self.upper_value.as_ref() {
            None => b.push('*'),
            Some(v) => b.push_str(&v.to_string()),
        }
        b.push(if self.upper_inclusive { ']' } else { '}' });
        b
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(&self.field) {
            visitor.visit_leaf(self);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        match other.as_any().downcast_ref::<Self>() {
            Some(other) => {
                self.field == other.field
                    && self.lower_value == other.lower_value
                    && self.upper_value == other.upper_value
                    && self.lower_inclusive == other.lower_inclusive
                    && self.upper_inclusive == other.upper_inclusive
            }
            None => false,
        }
    }

    fn query_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.class_hash().hash(&mut hasher);
        self.field.hash(&mut hasher);
        self.lower_value
            .as_ref()
            .map(|v| v.slice().to_vec())
            .hash(&mut hasher);
        self.upper_value
            .as_ref()
            .map(|v| v.slice().to_vec())
            .hash(&mut hasher);
        self.lower_inclusive.hash(&mut hasher);
        self.upper_inclusive.hash(&mut hasher);
        hasher.finish()
    }

    fn rewrite(&self, _searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        if self.lower_value.is_none() && self.upper_value.is_none() {
            return Ok(Some(Arc::new(FieldExistsQuery::new(&self.field))));
        }
        Ok(None)
    }

    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(ConstantScoreWeight::new(
            Arc::new(self.clone()),
            boost,
            DocValuesWeightImpl {
                source: SortedSetRangeSource {
                    field: self.field.clone(),
                    lower_value: self.lower_value.clone(),
                    upper_value: self.upper_value.clone(),
                    lower_inclusive: self.lower_inclusive,
                    upper_inclusive: self.upper_inclusive,
                },
                score: boost,
                score_mode,
            },
        )))
    }
}

/// The document range a [`SortedSkipperScorerSupplier`] narrows the query to.
///
/// Equivalent to the private record
/// `SortedSkipperScorerSupplier.DocIdRange`.
#[derive(Clone, Copy, Debug)]
struct DocIdRange {
    min_doc_id: i32,
    max_doc_id: i32,
}

/// What a [`SortedSkipperScorerSupplier`] needs from the query it serves: the
/// range's bounds, and how to scan forward for the first document satisfying a
/// predicate.
///
/// Equivalent to the three abstract methods of
/// `SortedSkipperScorerSupplier` — `getLowerValue`, `getUpperValue` and
/// `nextDoc` — which each query overrides in an anonymous subclass.
trait SkipperRangeSource: Debug {
    /// The lower bound of the query range, in the skipper's coordinate space.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the values raises.
    fn lower_value(&mut self) -> Result<i64>;

    /// The upper bound of the query range, in the skipper's coordinate space.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the values raises.
    fn upper_value(&mut self) -> Result<i64>;

    /// Advances from `start_doc_id` to the first document whose value makes
    /// `predicate` true, or [`NO_MORE_DOCS`] when there is none.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the values raises.
    fn next_doc(&mut self, start_doc_id: i32, predicate: &dyn Fn(i64) -> bool) -> Result<i32>;
}

/// A constant-score supplier that narrows a range query to one document
/// interval, using the field's skip index when the leaf's primary sort is on
/// that same field.
///
/// Equivalent to `org.apache.lucene.document.SortedSkipperScorerSupplier`.
/// The skipper yields coarse bounds containing every matching document; where
/// a boundary interval is not exact for the range, the supplier scans forward
/// from it to the first document that really satisfies the comparison, so the
/// interval it hands out is tight.
struct SortedSkipperScorerSupplier {
    skipper: Box<dyn DocValuesSkipper>,
    reverse: bool,
    score_mode: ScoreMode,
    score: f32,
    source: Box<dyn SkipperRangeSource>,
    skipper_min_doc_id: i32,
    skipper_max_doc_id: i32,
    skipper_min_doc_id_exact: bool,
    skipper_max_doc_id_exact: bool,
}

impl Debug for SortedSkipperScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SortedSkipperScorerSupplier")
            .field("reverse", &self.reverse)
            .finish_non_exhaustive()
    }
}

impl SortedSkipperScorerSupplier {
    /// Creates the supplier over `skipper`, whose field is the one the leaf's
    /// primary sort orders by.
    ///
    /// Equivalent to `SortedSkipperScorerSupplier(DocValuesSkipper, SortField,
    /// float, ScoreMode)`; `reverse` is the sort field's own, which decides how
    /// the range endpoints map onto the skipper's values.
    fn new(
        skipper: Box<dyn DocValuesSkipper>,
        reverse: bool,
        score: f32,
        score_mode: ScoreMode,
        source: Box<dyn SkipperRangeSource>,
    ) -> Self {
        Self {
            skipper,
            reverse,
            score_mode,
            score,
            source,
            skipper_min_doc_id: -1,
            skipper_max_doc_id: -1,
            skipper_min_doc_id_exact: false,
            skipper_max_doc_id_exact: false,
        }
    }

    /// Equivalent to the private `SortedSkipperScorerSupplier.range()`.
    fn range(&mut self) -> Result<DocIdRange> {
        if self.skipper_min_doc_id == -1 {
            self.compute_skipper_doc_ids()?;
        }
        let min_ord = self.source.lower_value()?;
        let max_ord = self.source.upper_value()?;
        let (min_doc_id, max_doc_id) = if self.reverse {
            (
                if self.skipper_min_doc_id_exact {
                    self.skipper_min_doc_id
                } else {
                    self.source
                        .next_doc(self.skipper_min_doc_id, &|l| l <= max_ord)?
                },
                if self.skipper_max_doc_id_exact {
                    self.skipper_max_doc_id
                } else {
                    self.source
                        .next_doc(self.skipper_max_doc_id, &|l| l < min_ord)?
                },
            )
        } else {
            (
                if self.skipper_min_doc_id_exact {
                    self.skipper_min_doc_id
                } else {
                    self.source
                        .next_doc(self.skipper_min_doc_id, &|l| l >= min_ord)?
                },
                if self.skipper_max_doc_id_exact {
                    self.skipper_max_doc_id
                } else {
                    self.source
                        .next_doc(self.skipper_max_doc_id, &|l| l > max_ord)?
                },
            )
        };
        Ok(DocIdRange {
            min_doc_id,
            max_doc_id,
        })
    }

    /// Equivalent to the private
    /// `SortedSkipperScorerSupplier.computeSkipperDocIds()`.
    fn compute_skipper_doc_ids(&mut self) -> Result<()> {
        let min_ord = self.source.lower_value()?;
        let max_ord = self.source.upper_value()?;
        if min_ord > max_ord
            || min_ord > self.skipper.global_max_value()
            || max_ord < self.skipper.global_min_value()
        {
            self.skipper_min_doc_id = NO_MORE_DOCS;
            self.skipper_max_doc_id = NO_MORE_DOCS;
            self.skipper_min_doc_id_exact = true;
            self.skipper_max_doc_id_exact = true;
            return Ok(());
        }
        if self.skipper.global_min_value() >= min_ord && self.skipper.global_max_value() <= max_ord
        {
            self.skipper_min_doc_id = 0;
            self.skipper_max_doc_id = self.skipper.global_doc_count();
            self.skipper_min_doc_id_exact = true;
            self.skipper_max_doc_id_exact = true;
            return Ok(());
        }
        if self.reverse {
            if self.skipper.global_max_value() <= max_ord {
                self.skipper_min_doc_id = 0;
                self.skipper_min_doc_id_exact = true;
            } else {
                self.skipper.advance_range(i64::MIN, max_ord)?;
                self.skipper_min_doc_id = self.skipper.min_doc_id(0);
                self.skipper_min_doc_id_exact = self.skipper.max_value(0) == max_ord;
            }
            if self.skipper.global_min_value() >= min_ord {
                self.skipper_max_doc_id = self.skipper.global_doc_count();
                self.skipper_max_doc_id_exact = true;
            } else {
                self.skipper.advance_range(i64::MIN, min_ord - 1)?;
                self.skipper_max_doc_id = self.skipper.min_doc_id(0);
                self.skipper_max_doc_id_exact = self.skipper.max_value(0) < min_ord;
            }
        } else {
            if self.skipper.global_min_value() >= min_ord {
                self.skipper_min_doc_id = 0;
                self.skipper_min_doc_id_exact = true;
            } else {
                self.skipper.advance_range(min_ord, i64::MAX)?;
                self.skipper_min_doc_id = self.skipper.min_doc_id(0);
                self.skipper_min_doc_id_exact = self.skipper.min_value(0) == min_ord;
            }
            if self.skipper.global_max_value() <= max_ord {
                self.skipper_max_doc_id = self.skipper.global_doc_count();
                self.skipper_max_doc_id_exact = true;
            } else {
                self.skipper.advance_range(max_ord + 1, i64::MAX)?;
                self.skipper_max_doc_id = self.skipper.min_doc_id(0);
                self.skipper_max_doc_id_exact = self.skipper.min_value(0) > max_ord;
            }
        }
        Ok(())
    }

    /// Returns the bulk scorer used when the document range is empty.
    ///
    /// Equivalent to the private static
    /// `SortedSkipperScorerSupplier.emptyBulkScorer()`, whose anonymous
    /// `BulkScorer` scores nothing and costs nothing.
    fn empty_bulk_scorer() -> Box<dyn BulkScorer> {
        Box::new(EmptyBulkScorer)
    }
}

/// The bulk scorer a supplier hands out for an empty range.
///
/// Equivalent to the private static
/// `SortedSkipperScorerSupplier.emptyBulkScorer()`.
#[derive(Debug)]
struct EmptyBulkScorer;

impl BulkScorer for EmptyBulkScorer {
    fn score(
        &mut self,
        _collector: &mut dyn LeafCollector,
        _accept_docs: Option<&dyn Bits>,
        _min: i32,
        _max: i32,
    ) -> crate::search::collection_terminated_exception::CollectionResult<i32> {
        Ok(NO_MORE_DOCS)
    }

    fn cost(&self) -> i64 {
        0
    }
}

impl ScorerSupplier for SortedSkipperScorerSupplier {
    fn get(&mut self, _lead_cost: i64) -> Result<Box<dyn Scorer>> {
        let range = self.range()?;
        let iterator: Box<dyn DocIdSetIterator> = if range.min_doc_id == range.max_doc_id {
            Box::new(crate::search::doc_id_set_iterator::empty())
        } else {
            Box::new(crate::search::doc_id_set_iterator::range(
                range.min_doc_id,
                range.max_doc_id,
            )?)
        };
        Ok(Box::new(ConstantScoreScorer::from_iterator(
            self.score,
            self.score_mode,
            iterator,
        )))
    }

    fn bulk_scorer(&mut self) -> Result<Box<dyn BulkScorer>> {
        let range = self.range()?;
        if range.min_doc_id == range.max_doc_id {
            return Ok(Self::empty_bulk_scorer());
        }
        let iterator = Box::new(crate::search::doc_id_set_iterator::range(
            range.min_doc_id,
            range.max_doc_id,
        )?);
        Ok(Box::new(RangeBulkScorer::new(
            iterator,
            self.score,
            range.min_doc_id,
            range.max_doc_id,
        )?))
    }

    fn cost(&self) -> i64 {
        // Java computes the skipper bounds lazily here; this port cannot,
        // because `cost` takes `&self` while `computeSkipperDocIds` mutates.
        // The supplier's users — `get` and `bulk_scorer` — compute them first,
        // and until they have, the whole field is the honest upper bound.
        if self.skipper_min_doc_id == -1 {
            return i64::from(self.skipper.global_doc_count());
        }
        if self.skipper_max_doc_id_exact {
            return i64::from(self.skipper_max_doc_id - self.skipper_min_doc_id);
        }
        i64::from(self.skipper_max_doc_id - self.skipper_min_doc_id + self.skipper.doc_count(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_values_long_hash_set_add_reports_whether_a_value_was_new() {
        // `add` is the private insert Java's constructor calls once per input
        // value; it returns false for a repeat so the constructor does not
        // count it twice. The set is built from four values so that the table
        // (eight slots) keeps free slots for the inserts below, which is the
        // condition under which Lucene calls `add` at all.
        let mut set = DocValuesLongHashSet::new(&[1, 2, 3, 4]);
        assert!(!set.add(1));
        assert!(set.add(7));
        assert!(!set.add(7));
        assert!(set.add(8));
    }

    #[test]
    fn doc_values_long_hash_set_sizes_its_table_as_java_does() {
        // Java: `1 << PackedInts.bitsRequired(values.length * 3 / 2)`, which is
        // the next power of two *strictly above* `3n/2` — never `3n/2` itself.
        // The expected lengths were measured by running PackedInts.bitsRequired
        // from lucene-core-10.5.0.jar for each n.
        let expected = [
            (0usize, 2usize),
            (1, 2),
            (2, 4),
            (3, 8),
            (4, 8),
            (5, 8),
            (6, 16),
            (7, 16),
            (8, 16),
            (9, 16),
            (10, 16),
            (11, 32),
            (12, 32),
        ];
        for (n, table_len) in expected {
            let values: Vec<i64> = (0..n as i64).collect();
            let set = DocValuesLongHashSet::new(&values);
            assert_eq!(
                set.table.len(),
                table_len,
                "table length for {n} values must match Lucene's"
            );
            // A free slot always remains, which is what lets `add` terminate.
            assert!(
                set.table.iter().any(|&slot| slot == MISSING),
                "the table for {n} values must keep a free slot"
            );
        }
    }

    #[test]
    fn doc_values_long_hash_set_counts_distinct_values_only() {
        let set = DocValuesLongHashSet::new(&[1, 1, 2, 2, 2, 5]);
        assert_eq!(set.size(), 3);
        assert_eq!(set.min_value(), 1);
        assert_eq!(set.max_value(), 5);
        for value in [1, 2, 5] {
            assert!(set.contains(value));
        }
        for value in [0, 3, 4, 6] {
            assert!(!set.contains(value));
        }
    }

    #[test]
    fn doc_values_long_hash_set_ram_bytes_used_charges_the_object_and_its_table() {
        // Java charges `BASE_RAM_BYTES` — the shallow size of the instance —
        // plus `RamUsageEstimator.sizeOfObject(table)`. The two terms are
        // asserted separately so that a change to either shows up here.
        let set = DocValuesLongHashSet::new(&[1, 2, 3, 4, 5]);
        let expected = RamUsageEstimator::shallow_size_of_instance::<DocValuesLongHashSet>()
            + RamUsageEstimator::size_of_long(&set.table);
        assert_eq!(set.ram_bytes_used(), expected);
        // A larger set costs more, because its table is longer.
        let bigger = DocValuesLongHashSet::new(&(0..1000).collect::<Vec<i64>>());
        assert!(bigger.ram_bytes_used() > set.ram_bytes_used());
    }

    #[test]
    fn sorted_numeric_set_query_ram_bytes_used_charges_the_field_and_the_values() {
        // Java charges `BASE_RAM_BYTES + sizeOfObject(field) + sizeOfObject(numbers)`,
        // and reaches the set's own `ramBytesUsed` through the `Accountable`
        // branch of `sizeOfObject`.
        let query = SortedNumericDocValuesSetQuery::new("field", &[1, 2, 3]);
        let expected =
            RamUsageEstimator::shallow_size_of_instance::<SortedNumericDocValuesSetQuery>()
                + RamUsageEstimator::size_of_string("field")
                + query.values().ram_bytes_used();
        assert_eq!(query.ram_bytes_used(), expected);
        // A longer field name costs more.
        let longer = SortedNumericDocValuesSetQuery::new("a_much_longer_field_name", &[1, 2, 3]);
        assert!(longer.ram_bytes_used() > query.ram_bytes_used());
    }
}
