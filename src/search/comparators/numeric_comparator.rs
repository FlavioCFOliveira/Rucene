//! Numeric sorting with skipping, ported from
//! `org.apache.lucene.search.comparators.NumericComparator` and its two
//! competitive-iterator builders.

#![deny(unsafe_code)]

use std::rc::Rc;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::point_values::is_estimated_point_count_greater_than_or_equal_to;
use crate::index::{
    IntersectVisitor, LeafReader, LeafReaderContext, NumericDocValues, PointTree, PointValues,
    Relation,
};
use crate::search::comparators::updateable_doc_id_set_iterator::UpdateableDocIdSetIterator;
use crate::search::doc_id_set_iterator::{all, DocIdSetIterator, NO_MORE_DOCS};
use crate::search::doc_values_access::get_numeric;
use crate::search::doc_values_iteration::numeric_as_iterator;
use crate::search::field_comparator::java_long_compare;
use crate::search::pruning::Pruning;
use crate::search::skip_block_range_iterator::SkipBlockRangeIterator;
use crate::util::{DocIdSetBuilder, IntsRef, NumericUtils};

/// The lower bound of the sampling interval; both bounds are powers of two.
///
/// Equivalent to `NumericComparator.MIN_SKIP_INTERVAL`.
const MIN_SKIP_INTERVAL: i32 = 32;

/// The upper bound of the sampling interval.
///
/// Equivalent to `NumericComparator.MAX_SKIP_INTERVAL`.
const MAX_SKIP_INTERVAL: i32 = 8192;

/// How the packed bytes of the field's points decode into the comparable
/// `long` the competitive iterator ranges over.
///
/// **Divergence from Lucene 10.5.0.** Java declares `sortableBytesToLong` as an
/// abstract method that each subclass overrides with a one-line call into
/// `NumericUtils`. Rust models the same two choices as data, so that the shared
/// state can decode without a virtual call back into the concrete comparator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortableBytes {
    /// Four bytes decoded with `NumericUtils.sortableBytesToInt`, used by
    /// [`IntComparator`](crate::search::comparators::IntComparator) and
    /// [`FloatComparator`](crate::search::comparators::FloatComparator).
    Int,
    /// Eight bytes decoded with `NumericUtils.sortableBytesToLong`, used by
    /// [`LongComparator`](crate::search::comparators::LongComparator) and
    /// [`DoubleComparator`](crate::search::comparators::DoubleComparator).
    Long,
}

impl SortableBytes {
    /// The number of bytes used to encode one value.
    ///
    /// Equivalent to the `bytesCount` constructor argument, which is
    /// `Integer.BYTES` or `Long.BYTES`.
    pub fn bytes_count(self) -> i32 {
        match self {
            SortableBytes::Int => 4,
            SortableBytes::Long => 8,
        }
    }

    /// Decodes sortable bytes into a `long`, consistently with the codec the
    /// field's [`PointValues`] use.
    ///
    /// Equivalent to `NumericComparator.sortableBytesToLong(byte[])`.
    pub fn decode(self, bytes: &[u8]) -> i64 {
        match self {
            SortableBytes::Int => i64::from(NumericUtils::sortable_bytes_to_int(bytes, 0)),
            SortableBytes::Long => NumericUtils::sortable_bytes_to_long(bytes, 0),
        }
    }
}

/// How a numeric comparator obtains the doc values of a segment.
///
/// Equivalent to the `protected NumericDocValues
/// NumericComparator.NumericLeafComparator.getNumericDocValues(LeafReaderContext, String)`
/// hook, which defaults to `DocValues.getNumeric(context.reader(), field)` and
/// which `SortedNumericSortField` overrides to install a
/// [`SortedNumericSelector`](crate::search::SortedNumericSelector) view. As
/// Lucene warns, an override should normally also disable skipping, because the
/// competitive iterator builds its ranges from the points index and assumes
/// that the values in doc values and points agree.
///
/// **Divergence from Lucene 10.5.0.** Java's hook receives the whole
/// [`LeafReaderContext`]; this port passes the leaf reader it would have read
/// from it, because a context cannot be stored beyond the call that produced
/// it, and the competitive iterator re-derives the doc values after the call
/// has returned.
pub type NumericDocValuesSource =
    Rc<dyn Fn(&dyn LeafReader, &str) -> Result<Box<dyn NumericDocValues>>>;

/// The per-segment state of a numeric comparator.
///
/// Equivalent to the fields of the abstract inner class
/// `NumericComparator.NumericLeafComparator`.
pub struct NumericLeafState {
    reader: Arc<dyn LeafReader>,
    max_doc: i32,
    /// The doc values of the sort field in this segment.
    ///
    /// Equivalent to the `protected final NumericDocValues docValues` field.
    doc_values: Box<dyn NumericDocValues>,
    competitive: Option<CompetitiveDisiBuilder>,
}

impl std::fmt::Debug for NumericLeafState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NumericLeafState")
            .field("max_doc", &self.max_doc)
            .field("has_competitive_builder", &self.competitive.is_some())
            .finish_non_exhaustive()
    }
}

/// The state shared by every competitive-iterator builder.
///
/// Equivalent to the fields of the abstract inner class
/// `NumericComparator.CompetitiveDISIBuilder`.
struct CompetitiveCommon {
    max_doc: i32,
    /// Whether the top value was set before this leaf was entered.
    ///
    /// Equivalent to the `final boolean leafTopSet = topValueSet` field, which
    /// Java captures once because `setTopValue` is called before any leaf.
    leaf_top_set: bool,
    competitive_iterator: UpdateableDocIdSetIterator,
    min_value_as_long: i64,
    max_value_as_long: i64,
    max_doc_visited: i32,
    update_counter: i32,
    current_skip_interval: i32,
}

/// The points-backed and doc-values-skipper-backed competitive iterators.
///
/// Equivalent to the private inner classes
/// `NumericComparator.PointsCompetitiveDISIBuilder` and
/// `NumericComparator.DVSkipperCompetitiveDISIBuilder`, whose only shared
/// mutable state lives in [`CompetitiveCommon`].
enum CompetitiveKind {
    Points {
        point_values: Box<dyn PointValues>,
        /// Lazily constructed to avoid the overhead when it is not used.
        point_tree: Option<Box<dyn PointTree>>,
        iterator_cost: i64,
        /// Helps to be conservative about increasing the sampling interval.
        try_update_fail_count: i32,
    },
    Skipper {
        doc_count: i32,
    },
}

struct CompetitiveDisiBuilder {
    common: CompetitiveCommon,
    kind: CompetitiveKind,
}

/// Abstract numeric comparator for comparing numeric values.
///
/// Equivalent to the state and the `final` logic of the abstract class
/// `org.apache.lucene.search.comparators.NumericComparator<T extends Number>`,
/// which provides a skipping functionality: an iterator that can skip over
/// non-competitive documents.
///
/// The `field` given to the constructor names the field whose doc values and
/// points the default implementations read.
///
/// **Divergence from Lucene 10.5.0.** Java expresses the split between the
/// shared skipping machinery and the per-type value handling as an abstract
/// class with four subclasses, whose inner leaf comparators call back into the
/// outer instance. Rust has no implementation inheritance, so this type holds
/// the shared state and the four concrete comparators —
/// [`LongComparator`](crate::search::comparators::LongComparator),
/// [`IntComparator`](crate::search::comparators::IntComparator),
/// [`DoubleComparator`](crate::search::comparators::DoubleComparator) and
/// [`FloatComparator`](crate::search::comparators::FloatComparator) — embed
/// one, passing their `bottom` and `topValue` as comparable `long`s to the
/// methods that Java would have obtained through
/// `bottomAsComparableLong()`/`topAsComparableLong()`. Those two methods are
/// pure reads of the subclass's fields, so the values are the same at the same
/// points.
pub struct NumericComparator {
    /// The missing value encoded as a comparable `long`.
    ///
    /// Equivalent to the `private final long missingValueAsLong` field, which
    /// Java fills from the abstract `missingValueAsComparableLong()`.
    missing_value_as_long: i64,
    /// The name of the sort field.
    ///
    /// Equivalent to the `protected final String field` field.
    field: String,
    /// Whether the sort on this field is reversed.
    ///
    /// Equivalent to the `protected final boolean reverse` field.
    reverse: bool,
    sortable_bytes: SortableBytes,
    top_value_set: bool,
    single_sort: bool,
    hits_threshold_reached: bool,
    queue_full: bool,
    pruning: Pruning,
    doc_values_source: Option<NumericDocValuesSource>,
    leaf: Option<NumericLeafState>,
}

impl std::fmt::Debug for NumericComparator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NumericComparator")
            .field("field", &self.field)
            .field("reverse", &self.reverse)
            .field("sortable_bytes", &self.sortable_bytes)
            .field("pruning", &self.pruning)
            .finish_non_exhaustive()
    }
}

impl NumericComparator {
    /// Creates the shared half of a numeric comparator.
    ///
    /// Equivalent to
    /// `NumericComparator(String, T, boolean, Pruning, int)`.
    pub fn new(
        field: impl Into<String>,
        missing_value_as_long: i64,
        reverse: bool,
        pruning: Pruning,
        sortable_bytes: SortableBytes,
    ) -> Self {
        Self {
            missing_value_as_long,
            field: field.into(),
            reverse,
            sortable_bytes,
            top_value_set: false,
            single_sort: false,
            hits_threshold_reached: false,
            queue_full: false,
            pruning,
            doc_values_source: None,
            leaf: None,
        }
    }

    /// Installs a replacement for the default doc-values lookup.
    ///
    /// Equivalent to overriding
    /// `NumericLeafComparator.getNumericDocValues(LeafReaderContext, String)`;
    /// see [`NumericDocValuesSource`].
    pub fn set_numeric_doc_values_source(&mut self, source: NumericDocValuesSource) {
        self.doc_values_source = Some(source);
    }

    /// Opens the doc values of `reader` through the installed source, or
    /// through `DocValues.getNumeric` when none is installed.
    fn open_doc_values(&self, reader: &dyn LeafReader) -> Result<Box<dyn NumericDocValues>> {
        match self.doc_values_source.as_ref() {
            Some(source) => source(reader, &self.field),
            None => get_numeric(reader, &self.field),
        }
    }

    /// The name of the sort field.
    ///
    /// Equivalent to reading the `protected final String field` field.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Whether the sort on this field is reversed.
    ///
    /// Equivalent to reading the `protected final boolean reverse` field.
    pub fn reverse(&self) -> bool {
        self.reverse
    }

    /// Whether a top value was set on this comparator.
    ///
    /// Equivalent to reading the `protected boolean topValueSet` field.
    pub fn top_value_set(&self) -> bool {
        self.top_value_set
    }

    /// Records that a top value was set.
    ///
    /// Equivalent to the body of `NumericComparator.setTopValue(T)`, which the
    /// concrete comparators call before storing the value itself.
    pub fn set_top_value(&mut self) {
        self.top_value_set = true;
    }

    /// Informs this comparator that the sort is done on a single field.
    ///
    /// Equivalent to `NumericComparator.setSingleSort()`.
    pub fn set_single_sort(&mut self) {
        self.single_sort = true;
    }

    /// Disables skipping.
    ///
    /// Equivalent to `NumericComparator.disableSkipping()`, which sets the
    /// pruning mode to [`Pruning::NONE`].
    pub fn disable_skipping(&mut self) {
        self.pruning = Pruning::NONE;
    }

    /// Returns the doc values of the segment currently being collected.
    ///
    /// Equivalent to reading `NumericLeafComparator.docValues`; it is `None`
    /// before the first call to [`set_next_leaf`](Self::set_next_leaf).
    pub fn doc_values(&mut self) -> Option<&mut dyn NumericDocValues> {
        self.leaf
            .as_mut()
            .map(|leaf| leaf.doc_values.as_mut() as &mut dyn NumericDocValues)
    }

    /// Prepares this comparator to collect `context`, opening the segment's
    /// doc values and, when pruning is enabled, the competitive-iterator
    /// builder.
    ///
    /// Equivalent to `new NumericLeafComparator(LeafReaderContext)` plus
    /// `buildCompetitiveDISIBuilder()`.
    ///
    /// `top_as_comparable_long` is the value Java's
    /// `NumericLeafComparator.topAsComparableLong()` would return; it is only
    /// read when a top value was set.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while opening the segment's values, and
    /// returns [`LuceneError::IllegalState`] or
    /// [`LuceneError::IllegalArgument`] for the field-info inconsistencies
    /// Java rejects when building the points-backed iterator.
    pub fn set_next_leaf(
        &mut self,
        context: &LeafReaderContext,
        top_as_comparable_long: i64,
    ) -> Result<()> {
        let reader = context.leaf_reader();
        let max_doc = reader.max_doc();
        let doc_values = self.open_doc_values(reader.as_ref())?;
        self.leaf = Some(NumericLeafState {
            reader: Arc::clone(&reader),
            max_doc,
            doc_values,
            competitive: None,
        });
        let competitive = self.build_competitive_disi_builder(top_as_comparable_long)?;
        if let Some(leaf) = self.leaf.as_mut() {
            leaf.competitive = competitive;
        }
        Ok(())
    }

    /// Equivalent to `NumericLeafComparator.buildCompetitiveDISIBuilder()`.
    fn build_competitive_disi_builder(
        &mut self,
        top_as_comparable_long: i64,
    ) -> Result<Option<CompetitiveDisiBuilder>> {
        if self.pruning == Pruning::NONE {
            return Ok(None);
        }
        let (reader, max_doc) = match self.leaf.as_ref() {
            Some(leaf) => (Arc::clone(&leaf.reader), leaf.max_doc),
            None => return Ok(None),
        };

        if let Some(point_values) = reader.get_point_values(&self.field)? {
            let field_infos = reader.get_field_infos();
            let info = field_infos.field_info(&self.field);
            match info {
                None => {
                    return Err(LuceneError::IllegalState(format!(
                        "Field {} doesn't index points according to FieldInfos yet returns non-null PointValues",
                        self.field
                    )));
                }
                Some(info) if info.get_point_dimension_count() == 0 => {
                    return Err(LuceneError::IllegalState(format!(
                        "Field {} doesn't index points according to FieldInfos yet returns non-null PointValues",
                        self.field
                    )));
                }
                Some(info) if info.get_point_dimension_count() > 1 => {
                    return Err(LuceneError::IllegalArgument(format!(
                        "Field {} is indexed with multiple dimensions, sorting is not supported",
                        self.field
                    )));
                }
                Some(info) if info.get_point_num_bytes() != self.sortable_bytes.bytes_count() => {
                    return Err(LuceneError::IllegalArgument(format!(
                        "Field {} is indexed with {} bytes per dimension, but {:?} expected {}",
                        self.field,
                        info.get_point_num_bytes(),
                        self,
                        self.sortable_bytes.bytes_count()
                    )));
                }
                Some(_) => {}
            }
            let mut builder = CompetitiveDisiBuilder {
                common: self.new_competitive_common(max_doc)?,
                kind: CompetitiveKind::Points {
                    point_values,
                    point_tree: None,
                    iterator_cost: -1,
                    try_update_fail_count: 0,
                },
            };
            if builder.common.leaf_top_set {
                self.encode_top_into(&mut builder.common, top_as_comparable_long);
            }
            return Ok(Some(builder));
        }

        if let Some(skipper) = reader.get_doc_values_skipper(&self.field)? {
            let doc_count = skipper.global_doc_count();
            // The skipper is re-opened whenever a new range is materialised, so
            // only its document count is kept here.
            drop(skipper);
            let mut builder = CompetitiveDisiBuilder {
                common: self.new_competitive_common(max_doc)?,
                kind: CompetitiveKind::Skipper { doc_count },
            };
            if builder.common.leaf_top_set {
                self.encode_top_into(&mut builder.common, top_as_comparable_long);
            }
            return Ok(Some(builder));
        }

        Ok(None)
    }

    /// Equivalent to the `CompetitiveDISIBuilder(NumericLeafComparator)`
    /// constructor, minus the `encodeTop()` call the caller makes.
    fn new_competitive_common(&self, max_doc: i32) -> Result<CompetitiveCommon> {
        let competitive_iterator = UpdateableDocIdSetIterator::new();
        competitive_iterator.update(Box::new(all(max_doc)?));
        Ok(CompetitiveCommon {
            max_doc,
            leaf_top_set: self.top_value_set,
            competitive_iterator,
            min_value_as_long: i64::MIN,
            max_value_as_long: i64::MAX,
            max_doc_visited: -1,
            update_counter: 0,
            current_skip_interval: MIN_SKIP_INTERVAL,
        })
    }

    /// Equivalent to `CompetitiveDISIBuilder.encodeTop()`.
    fn encode_top_into(&self, common: &mut CompetitiveCommon, top_as_comparable_long: i64) {
        encode_top(
            common,
            self.reverse,
            self.pruning,
            self.single_sort,
            self.queue_full,
            top_as_comparable_long,
        );
    }
}

/// Equivalent to `CompetitiveDISIBuilder.encodeTop()`.
///
/// If [`Pruning::GREATER_THAN_OR_EQUAL_TO`] is in force the bound can be
/// tightened by one: for an ascending sort with a top value of 3, the range
/// becomes `[4, MAX_VALUE]`.
fn encode_top(
    common: &mut CompetitiveCommon,
    reverse: bool,
    pruning: Pruning,
    single_sort: bool,
    queue_full: bool,
    top_as_comparable_long: i64,
) {
    {
        if !reverse {
            common.min_value_as_long = top_as_comparable_long;
            if single_sort
                && pruning == Pruning::GREATER_THAN_OR_EQUAL_TO
                && queue_full
                && common.min_value_as_long != i64::MAX
            {
                common.min_value_as_long += 1;
            }
        } else {
            common.max_value_as_long = top_as_comparable_long;
            if single_sort
                && pruning == Pruning::GREATER_THAN_OR_EQUAL_TO
                && queue_full
                && common.max_value_as_long != i64::MIN
            {
                common.max_value_as_long -= 1;
            }
        }
    }
}

/// Equivalent to `CompetitiveDISIBuilder.encodeBottom()`.
///
/// If [`Pruning::GREATER_THAN_OR_EQUAL_TO`] is in force the bound can be
/// tightened by one: for an ascending sort with a bottom value of 5, the range
/// becomes `[MIN_VALUE, 4]`.
fn encode_bottom(
    common: &mut CompetitiveCommon,
    reverse: bool,
    pruning: Pruning,
    bottom_as_comparable_long: i64,
) {
    if !reverse {
        common.max_value_as_long = bottom_as_comparable_long;
        if pruning == Pruning::GREATER_THAN_OR_EQUAL_TO && common.max_value_as_long != i64::MIN {
            common.max_value_as_long -= 1;
        }
    } else {
        common.min_value_as_long = bottom_as_comparable_long;
        if pruning == Pruning::GREATER_THAN_OR_EQUAL_TO && common.min_value_as_long != i64::MAX {
            common.min_value_as_long += 1;
        }
    }
}

impl NumericComparator {
    /// Equivalent to `CompetitiveDISIBuilder.isMissingValueCompetitive()`.
    fn is_missing_value_competitive(
        &self,
        common: &CompetitiveCommon,
        bottom_as_comparable_long: i64,
        top_as_comparable_long: i64,
    ) -> bool {
        // If the queue is full, compare with bottom first; if competitive, then
        // check whether we can compare with the top value.
        if self.queue_full {
            let result = java_long_compare(self.missing_value_as_long, bottom_as_comparable_long);
            // In a reverse (descending) sort the missing value is competitive
            // when it is greater than or equal to bottom; in an ascending sort
            // it is competitive when it is smaller than or equal to bottom.
            let competitive = if self.reverse {
                if self.pruning == Pruning::GREATER_THAN_OR_EQUAL_TO {
                    result > 0
                } else {
                    result >= 0
                }
            } else if self.pruning == Pruning::GREATER_THAN_OR_EQUAL_TO {
                result < 0
            } else {
                result <= 0
            };
            if !competitive {
                return false;
            }
        }

        if common.leaf_top_set {
            let result = java_long_compare(self.missing_value_as_long, top_as_comparable_long);
            // In a reverse (descending) sort the missing value is competitive
            // when it is smaller than or equal to the top value; in an
            // ascending sort when it is greater than or equal to it.
            return if self.reverse {
                result <= 0
            } else {
                result >= 0
            };
        }

        // Competitive by default.
        true
    }

    /// Records that the queue is full and refreshes the competitive iterator.
    ///
    /// Equivalent to `NumericLeafComparator.setBottom(int)`, whose per-type
    /// half — copying the slot's value into `bottom` — the concrete comparators
    /// perform before calling this.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while rebuilding the iterator.
    pub fn set_bottom(
        &mut self,
        bottom_as_comparable_long: i64,
        top_as_comparable_long: i64,
    ) -> Result<()> {
        // Setting bottom means that we have collected enough hits.
        self.queue_full = true;
        self.update_competitive_iterator(bottom_as_comparable_long, top_as_comparable_long)
    }

    /// Records the highest document visited so far.
    ///
    /// Equivalent to `NumericLeafComparator.copy(int, int)`, whose per-type
    /// half — copying the document's value into the slot — the concrete
    /// comparators perform before calling this.
    pub fn copy(&mut self, doc: i32) {
        if let Some(common) = self.competitive_common_mut() {
            common.max_doc_visited = doc;
        }
    }

    /// Reacts to the collector installing a scorer.
    ///
    /// Equivalent to `NumericLeafComparator.setScorer(Scorable)`, which
    /// forwards to the competitive-iterator builder.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's points-backed builder seeds
    /// `iteratorCost` with `((Scorer) scorer).iterator().cost()` when the
    /// scorable happens to be a [`Scorer`](crate::search::Scorer), and with
    /// `maxDoc` otherwise. Rust cannot test a `&mut dyn Scorable` for a second
    /// trait at this crate's minimum supported Rust version, and
    /// [`Scorable`](crate::search::Scorable) exposes no cost, so this port
    /// always uses Java's own fallback, `maxDoc`. `iteratorCost` only tunes how
    /// eagerly the competitive iterator is rebuilt and how selective a new
    /// range must be to be worth materialising; it never takes part in deciding
    /// whether a document is competitive, so the collected hits and their order
    /// are unchanged.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while rebuilding the iterator.
    pub fn set_scorer(
        &mut self,
        bottom_as_comparable_long: i64,
        top_as_comparable_long: i64,
    ) -> Result<()> {
        let should_update = match self
            .leaf
            .as_mut()
            .and_then(|leaf| leaf.competitive.as_mut())
        {
            None => false,
            Some(builder) => match &mut builder.kind {
                CompetitiveKind::Points { iterator_cost, .. } => {
                    if *iterator_cost == -1 {
                        *iterator_cost = i64::from(builder.common.max_doc);
                        true
                    } else {
                        false
                    }
                }
                CompetitiveKind::Skipper { .. } => true,
            },
        };
        if should_update {
            // Update the iterator when we have a new segment.
            self.update_competitive_iterator(bottom_as_comparable_long, top_as_comparable_long)?;
        }
        Ok(())
    }

    /// Records that the hits threshold was reached and refreshes the
    /// competitive iterator.
    ///
    /// Equivalent to `NumericLeafComparator.setHitsThresholdReached()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while rebuilding the iterator.
    pub fn set_hits_threshold_reached(
        &mut self,
        bottom_as_comparable_long: i64,
        top_as_comparable_long: i64,
    ) -> Result<()> {
        self.hits_threshold_reached = true;
        self.update_competitive_iterator(bottom_as_comparable_long, top_as_comparable_long)
    }

    /// Returns the competitive iterator of the segment being collected.
    ///
    /// Equivalent to `NumericLeafComparator.competitiveIterator()`.
    pub fn competitive_iterator(&self) -> Option<Box<dyn DocIdSetIterator>> {
        self.leaf
            .as_ref()
            .and_then(|leaf| leaf.competitive.as_ref())
            .map(|builder| {
                Box::new(builder.common.competitive_iterator.clone()) as Box<dyn DocIdSetIterator>
            })
    }

    fn competitive_common_mut(&mut self) -> Option<&mut CompetitiveCommon> {
        self.leaf
            .as_mut()
            .and_then(|leaf| leaf.competitive.as_mut())
            .map(|builder| &mut builder.common)
    }

    /// Equivalent to the `final CompetitiveDISIBuilder.updateCompetitiveIterator()`.
    fn update_competitive_iterator(
        &mut self,
        bottom_as_comparable_long: i64,
        top_as_comparable_long: i64,
    ) -> Result<()> {
        if self.leaf.as_ref().map_or(true, |l| l.competitive.is_none()) {
            return Ok(());
        }
        if !self.hits_threshold_reached {
            return Ok(());
        }
        let (leaf_top_set, has_missing_docs) = {
            let leaf = self
                .leaf
                .as_ref()
                .expect("INVARIANT: the leaf was just observed to be present");
            let builder = leaf
                .competitive
                .as_ref()
                .expect("INVARIANT: the builder was just observed to be present");
            let has_missing = match &builder.kind {
                CompetitiveKind::Points { point_values, .. } => {
                    point_values.doc_count() != builder.common.max_doc
                }
                CompetitiveKind::Skipper { doc_count, .. } => *doc_count != builder.common.max_doc,
            };
            (builder.common.leaf_top_set, has_missing)
        };
        if !leaf_top_set && !self.queue_full {
            return Ok(());
        }

        // If some documents have missing points, check that missing values
        // prohibit the optimization.
        if has_missing_docs {
            let competitive = {
                let common = &self
                    .leaf
                    .as_ref()
                    .expect("INVARIANT: the leaf was just observed to be present")
                    .competitive
                    .as_ref()
                    .expect("INVARIANT: the builder was just observed to be present")
                    .common;
                self.is_missing_value_competitive(
                    common,
                    bottom_as_comparable_long,
                    top_as_comparable_long,
                )
            };
            if competitive {
                return Ok(());
            }
        }

        {
            let common = self
                .competitive_common_mut()
                .expect("INVARIANT: the builder was just observed to be present");
            common.update_counter += 1;
            // Start sampling if we get called too much.
            if common.update_counter > 256
                && (common.update_counter & (common.current_skip_interval - 1))
                    != common.current_skip_interval - 1
            {
                return Ok(());
            }
        }

        if self.queue_full {
            let reverse = self.reverse;
            let pruning = self.pruning;
            let common = self
                .competitive_common_mut()
                .expect("INVARIANT: the builder was just observed to be present");
            encode_bottom(common, reverse, pruning, bottom_as_comparable_long);
        }

        self.do_update_competitive_iterator()
    }

    /// Equivalent to the two `doUpdateCompetitiveIterator()` implementations.
    fn do_update_competitive_iterator(&mut self) -> Result<()> {
        let field = self.field.clone();
        let sortable_bytes = self.sortable_bytes;
        let doc_values_source = self.doc_values_source.clone();
        let reader = match self.leaf.as_ref() {
            Some(leaf) => Arc::clone(&leaf.reader),
            None => return Ok(()),
        };
        let Some(builder) = self
            .leaf
            .as_mut()
            .and_then(|leaf| leaf.competitive.as_mut())
        else {
            return Ok(());
        };

        match &mut builder.kind {
            CompetitiveKind::Skipper { .. } => {
                let Some(skipper) = reader.get_doc_values_skipper(&field)? else {
                    return Ok(());
                };
                let iterator = SkipBlockRangeIterator::new(
                    skipper,
                    builder.common.min_value_as_long,
                    builder.common.max_value_as_long,
                );
                builder
                    .common
                    .competitive_iterator
                    .update(Box::new(iterator));
                Ok(())
            }
            CompetitiveKind::Points {
                point_values,
                point_tree,
                iterator_cost,
                try_update_fail_count,
            } => {
                let max_doc = builder.common.max_doc;
                let mut result = DocIdSetBuilder::new(max_doc);
                let threshold = ((*iterator_cost as u64) >> 3) as i64;

                let reached = {
                    let mut visitor = CompetitiveVisitor {
                        result: &mut result,
                        max_doc_visited: builder.common.max_doc_visited,
                        min_value_as_long: builder.common.min_value_as_long,
                        max_value_as_long: builder.common.max_value_as_long,
                        sortable_bytes,
                    };
                    if point_tree.is_none() {
                        *point_tree = Some(point_values.point_tree()?);
                    }
                    let tree = point_tree
                        .as_mut()
                        .expect("INVARIANT: the point tree was just built");
                    is_estimated_point_count_greater_than_or_equal_to(
                        &mut visitor,
                        tree.as_mut(),
                        threshold,
                    )?
                };

                if reached {
                    // The new range is not selective enough to be worth
                    // materializing: it does not reduce the number of docs at
                    // least 8x.
                    let doc_count = point_values.doc_count();
                    let success = false;
                    update_skip_interval(&mut builder.common, try_update_fail_count, success);
                    if i64::from(doc_count) < *iterator_cost {
                        // Use the set of docs with values to help drive
                        // iteration.
                        let doc_values = match doc_values_source.as_ref() {
                            Some(source) => source(reader.as_ref(), &field)?,
                            None => get_numeric(reader.as_ref(), &field)?,
                        };
                        builder
                            .common
                            .competitive_iterator
                            .update(numeric_as_iterator(doc_values));
                        *iterator_cost = i64::from(doc_count);
                    }
                    return Ok(());
                }

                {
                    let mut visitor = CompetitiveVisitor {
                        result: &mut result,
                        max_doc_visited: builder.common.max_doc_visited,
                        min_value_as_long: builder.common.min_value_as_long,
                        max_value_as_long: builder.common.max_value_as_long,
                        sortable_bytes,
                    };
                    point_values.intersect(&mut visitor)?;
                }
                let new_iterator = result.build()?.iterator()?;
                *iterator_cost = new_iterator.cost();
                builder.common.competitive_iterator.update(new_iterator);
                update_skip_interval(&mut builder.common, try_update_fail_count, true);
                Ok(())
            }
        }
    }
}

/// Equivalent to `PointsCompetitiveDISIBuilder.updateSkipInterval(boolean)`.
fn update_skip_interval(
    common: &mut CompetitiveCommon,
    try_update_fail_count: &mut i32,
    success: bool,
) {
    if common.update_counter > 256 {
        if success {
            common.current_skip_interval =
                (common.current_skip_interval / 2).max(MIN_SKIP_INTERVAL);
            *try_update_fail_count = 0;
        } else if *try_update_fail_count >= 3 {
            common.current_skip_interval =
                (common.current_skip_interval * 2).min(MAX_SKIP_INTERVAL);
            *try_update_fail_count = 0;
        } else {
            *try_update_fail_count += 1;
        }
    }
}

/// The visitor that collects the documents whose point value lies inside the
/// competitive range.
///
/// Equivalent to the anonymous `PointValues.IntersectVisitor` that
/// `PointsCompetitiveDISIBuilder.doUpdateCompetitiveIterator()` builds.
///
/// **Divergence from Lucene 10.5.0.** Java keeps the `BulkAdder` returned by
/// `grow(int)` in a field and reuses it across `visit` calls. This crate's
/// [`DocIdSetBuilder::grow`] returns an adder that borrows the builder, so it
/// cannot be stored beside it; each `visit` therefore re-derives the adder with
/// `grow(0)`, which reserves nothing — the capacity was already reserved by the
/// preceding `grow(count)` — and appends exactly as Java's adder does.
struct CompetitiveVisitor<'a> {
    result: &'a mut DocIdSetBuilder,
    max_doc_visited: i32,
    min_value_as_long: i64,
    max_value_as_long: i64,
    sortable_bytes: SortableBytes,
}

impl CompetitiveVisitor<'_> {
    /// The body of Java's `adder.add(int)`.
    fn add(&mut self, doc_id: i32) {
        self.result.grow(0).add(doc_id);
    }
}

impl IntersectVisitor for CompetitiveVisitor<'_> {
    fn grow(&mut self, count: i32) {
        self.result.grow(count);
    }

    fn visit(&mut self, doc_id: i32) -> Result<()> {
        if doc_id <= self.max_doc_visited {
            // Already visited or skipped.
            return Ok(());
        }
        self.add(doc_id);
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        if doc_id <= self.max_doc_visited {
            // Already visited or skipped.
            return Ok(());
        }
        let value = self.sortable_bytes.decode(packed_value);
        if value >= self.min_value_as_long && value <= self.max_value_as_long {
            // The doc is competitive.
            self.add(doc_id);
        }
        Ok(())
    }

    fn visit_iterator(&mut self, iterator: &mut dyn DocIdSetIterator) -> Result<()> {
        if iterator.advance(self.max_doc_visited + 1)? != NO_MORE_DOCS {
            let doc = iterator.doc_id();
            self.add(doc);
            self.result.grow(0).add_iterator(iterator)?;
        }
        Ok(())
    }

    fn visit_ints_ref(&mut self, ints_ref: &IntsRef) -> Result<()> {
        let lower_bound = self.max_doc_visited + 1;
        self.result.grow(0).add_ints_from(ints_ref, lower_bound);
        Ok(())
    }

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Relation {
        let min = self.sortable_bytes.decode(min_packed_value);
        let max = self.sortable_bytes.decode(max_packed_value);

        if min > self.max_value_as_long || max < self.min_value_as_long {
            // 1. When the comparison is 0 and pruning is
            //    GREATER_THAN_OR_EQUAL_TO: if the sort is ascending then
            //    maxValueAsLong is the value just below bottom, so it is
            //    competitive.
            // 2. When the comparison is 0 and pruning is GREATER_THAN:
            //    maxValueAsLong equals bottom, but there are several
            //    comparators, so it could be competitive.
            return Relation::CellOutsideQuery;
        }

        if min < self.min_value_as_long || max > self.max_value_as_long {
            return Relation::CellCrossesQuery;
        }
        Relation::CellInsideQuery
    }
}
