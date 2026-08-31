//! Index-sort-aware numeric range queries, ported from
//! `org.apache.lucene.search.IndexSortSortedNumericDocValuesRangeQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::{
    DocValuesType, IndexReaderContext, IntersectVisitor, LeafReaderContext, NumericDocValues,
    PointTree, PointValues, Relation,
};
use crate::search::abstract_doc_id_set_iterator::AbstractDocIdSetIterator;
use crate::search::constant_score_scorer_supplier::ConstantScoreScorerSupplier;
use crate::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use crate::search::doc_id_set_iterator::{all, empty, range, DocIdSetIterator, NO_MORE_DOCS};
use crate::search::doc_values_iteration::numeric_as_iterator;
use crate::search::field_comparator::SortValue;
use crate::search::field_exists_query::FieldExistsQuery;
use crate::search::index_searcher::IndexSearcher;
use crate::search::match_all_docs_query::MatchAllDocsQuery;
use crate::search::numeric_doc_values_range_query::NumericDocValuesRangeQuery;
use crate::search::point_range_query::compare_unsigned;
use crate::search::pruning::Pruning;
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::scorable::SimpleScorable;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::sort::{MissingValue, SortField, SortFieldKind, SortFieldType};
use crate::search::two_phase_iterator::ScorerIterator;
use crate::search::weight::Weight;
use crate::util::NumericUtils;

/// A range query over a numeric doc-values field that exploits the index sort
/// when the field is the primary sort key.
///
/// Equivalent to
/// `org.apache.lucene.search.IndexSortSortedNumericDocValuesRangeQuery`, a
/// subclass of `NumericDocValuesRangeQuery`. Rust has no implementation
/// inheritance, so the base class's state is the embedded
/// [`NumericDocValuesRangeQuery`].
///
/// The query first tries to use the BKD tree or the index sort to turn the
/// range into a contiguous run of doc IDs; when neither applies it delegates to
/// the fallback query, which must match the same documents.
#[derive(Debug)]
pub struct IndexSortSortedNumericDocValuesRangeQuery {
    base: NumericDocValuesRangeQuery,
    fallback_query: Arc<dyn Query>,
}

/// An iterator over the matching documents together with their count, or `-1`
/// when the count is not known.
///
/// Equivalent to the private record
/// `IndexSortSortedNumericDocValuesRangeQuery.IteratorAndCount`.
struct IteratorAndCount {
    it: Box<dyn DocIdSetIterator>,
    count: i32,
}

impl IteratorAndCount {
    /// Equivalent to `IteratorAndCount.empty()`.
    fn empty() -> Self {
        Self {
            it: Box::new(empty()),
            count: 0,
        }
    }

    /// Equivalent to `IteratorAndCount.all(int)`.
    fn all(max_doc: i32) -> Result<Self> {
        Ok(Self {
            it: Box::new(all(max_doc)?),
            count: max_doc,
        })
    }

    /// Equivalent to `IteratorAndCount.denseRange(int, int)`.
    fn dense_range(min_doc: i32, max_doc: i32) -> Result<Self> {
        Ok(Self {
            it: Box::new(range(min_doc, max_doc)?),
            count: max_doc - min_doc,
        })
    }

    /// Equivalent to `IteratorAndCount.sparseRange(int, int, DocIdSetIterator)`.
    fn sparse_range(min_doc: i32, max_doc: i32, delegate: Box<dyn DocIdSetIterator>) -> Self {
        Self {
            it: Box::new(BoundedDocIdSetIterator::new(min_doc, max_doc, delegate)),
            count: -1,
        }
    }
}

/// Restricts a delegate iterator to a half-open doc-ID range.
///
/// Equivalent to the private static class
/// `IndexSortSortedNumericDocValuesRangeQuery.BoundedDocIdSetIterator`.
struct BoundedDocIdSetIterator {
    base: AbstractDocIdSetIterator,
    first_doc: i32,
    last_doc: i32,
    delegate: Box<dyn DocIdSetIterator>,
}

impl BoundedDocIdSetIterator {
    fn new(first_doc: i32, last_doc: i32, delegate: Box<dyn DocIdSetIterator>) -> Self {
        Self {
            base: AbstractDocIdSetIterator::new(),
            first_doc,
            last_doc,
            delegate,
        }
    }
}

impl DocIdSetIterator for BoundedDocIdSetIterator {
    fn doc_id(&self) -> i32 {
        self.base.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.base.doc + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let target = target.max(self.first_doc);
        let result = self.delegate.advance(target)?;
        let doc = if result < self.last_doc {
            result
        } else {
            NO_MORE_DOCS
        };
        Ok(self.base.set(doc))
    }

    fn cost(&self) -> i64 {
        self.delegate
            .cost()
            .min(i64::from(self.last_doc - self.first_doc))
    }
}

/// A packed value and the doc it was found on.
///
/// Equivalent to the private static class
/// `IndexSortSortedNumericDocValuesRangeQuery.ValueAndDoc`.
#[derive(Default)]
struct ValueAndDoc {
    value: Option<Vec<u8>>,
    doc_id: i32,
    done: bool,
}

/// The visitor that finds the first value at or past a bound.
///
/// Equivalent to the anonymous `IntersectVisitor` of the private static
/// `findNextValue`.
struct FindNextValueVisitor<'a> {
    state: &'a mut ValueAndDoc,
    value: &'a [u8],
    allow_equal: bool,
    last_doc: bool,
    bytes_per_dim: usize,
}

impl IntersectVisitor for FindNextValueVisitor<'_> {
    fn visit(&mut self, _doc_id: i32) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "this visitor is only handed leaf values".to_string(),
        ))
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        match self.state.value.as_ref() {
            None => {
                let cmp = compare_unsigned(packed_value, 0, self.value, 0, self.bytes_per_dim);
                if cmp > 0 || (cmp == 0 && self.allow_equal) {
                    self.state.value = Some(packed_value.to_vec());
                    self.state.doc_id = doc_id;
                }
            }
            Some(current) => {
                if self.last_doc && !self.state.done {
                    let cmp = compare_unsigned(packed_value, 0, current, 0, self.bytes_per_dim);
                    debug_assert!(cmp >= 0);
                    if cmp > 0 {
                        self.state.done = true;
                    } else {
                        self.state.doc_id = doc_id;
                    }
                }
            }
        }
        Ok(())
    }

    fn compare(&self, _min_packed_value: &[u8], _max_packed_value: &[u8]) -> Relation {
        Relation::CellCrossesQuery
    }
}

/// The visitor that finds the last doc holding a given value.
///
/// Equivalent to the anonymous `IntersectVisitor` of the private static
/// `lastDoc`.
struct LastDocVisitor<'a> {
    last_doc: &'a mut i32,
    value: &'a [u8],
    bytes_per_dim: usize,
}

impl IntersectVisitor for LastDocVisitor<'_> {
    fn visit(&mut self, _doc_id: i32) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "this visitor is only handed leaf values".to_string(),
        ))
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        if compare_unsigned(self.value, 0, packed_value, 0, self.bytes_per_dim) == 0 {
            *self.last_doc = doc_id;
        }
        Ok(())
    }

    fn compare(&self, _min_packed_value: &[u8], _max_packed_value: &[u8]) -> Relation {
        Relation::CellCrossesQuery
    }
}

/// Equivalent to the private static
/// `findNextValue(PointTree, byte[], boolean, ByteArrayComparator, boolean)`.
fn find_next_value(
    point_tree: &mut dyn PointTree,
    value: &[u8],
    allow_equal: bool,
    bytes_per_dim: usize,
    last_doc: bool,
) -> Result<Option<ValueAndDoc>> {
    let cmp = compare_unsigned(point_tree.max_packed_value(), 0, value, 0, bytes_per_dim);
    if cmp < 0 || (cmp == 0 && !allow_equal) {
        return Ok(None);
    }
    if !point_tree.move_to_child()? {
        let mut state = ValueAndDoc::default();
        {
            let mut visitor = FindNextValueVisitor {
                state: &mut state,
                value,
                allow_equal,
                last_doc,
                bytes_per_dim,
            };
            point_tree.visit_doc_values(&mut visitor)?;
        }
        return Ok(if state.value.is_some() {
            Some(state)
        } else {
            None
        });
    }
    // Recurse.
    loop {
        if let Some(found) =
            find_next_value(point_tree, value, allow_equal, bytes_per_dim, last_doc)?
        {
            return Ok(Some(found));
        }
        if !point_tree.move_to_sibling()? {
            break;
        }
    }
    let moved = point_tree.move_to_parent()?;
    debug_assert!(moved);
    Ok(None)
}

/// Equivalent to the private static
/// `nextDoc(PointTree, byte[], boolean, ByteArrayComparator, boolean)`.
fn next_doc(
    point_tree: &mut dyn PointTree,
    value: &[u8],
    allow_equal: bool,
    bytes_per_dim: usize,
    last_doc: bool,
) -> Result<i32> {
    let Some(found) = find_next_value(point_tree, value, allow_equal, bytes_per_dim, last_doc)?
    else {
        return Ok(-1);
    };
    if !last_doc || found.done {
        return Ok(found.doc_id);
    }
    // The next value was found; now the last doc ID holding it is needed.
    let found_value = found
        .value
        .as_ref()
        .expect("INVARIANT: findNextValue only returns a state with a value");
    let doc = last_doc_of(point_tree, found_value, bytes_per_dim)?;
    if doc == -1 {
        // `found.doc_id` was actually the last doc ID.
        Ok(found.doc_id)
    } else {
        Ok(doc)
    }
}

/// Equivalent to the private static
/// `lastDoc(PointTree, byte[], ByteArrayComparator)`, which effectively runs a
/// binary search, made verbose by the point-tree API not allowing a move back
/// to a previous sibling.
fn last_doc_of(point_tree: &mut dyn PointTree, value: &[u8], bytes_per_dim: usize) -> Result<i32> {
    // A stack of nodes that may contain the value, used to search for the last
    // leaf node holding it.
    let mut stack: Vec<Box<dyn PointTree>> = Vec::new();
    'outer: loop {
        // Move to the next node.
        while !point_tree.move_to_sibling()? {
            if !point_tree.move_to_parent()? {
                // There is no next node.
                break 'outer;
            }
        }
        if compare_unsigned(point_tree.min_packed_value(), 0, value, 0, bytes_per_dim) > 0 {
            // This node does not hold the value, so the next nodes cannot
            // either.
            break;
        }
        stack.push(point_tree.clone_tree());
    }
    while let Some(mut next) = stack.pop() {
        if !next.move_to_child()? {
            let mut last_doc = -1;
            {
                let mut visitor = LastDocVisitor {
                    last_doc: &mut last_doc,
                    value,
                    bytes_per_dim,
                };
                next.visit_doc_values(&mut visitor)?;
            }
            if last_doc != -1 {
                return Ok(last_doc);
            }
        } else {
            loop {
                if compare_unsigned(next.min_packed_value(), 0, value, 0, bytes_per_dim) > 0 {
                    // This node does not hold the value, so the next nodes
                    // cannot either.
                    break;
                }
                stack.push(next.clone_tree());
                if !next.move_to_sibling()? {
                    break;
                }
            }
        }
    }
    Ok(-1)
}

/// Reads the numeric type of the primary index-sort field.
///
/// Equivalent to the private static
/// `getSortFieldType(SortField)`, which expects a `SortedNumericSortField`.
fn get_sort_field_type(sort_field: &SortField) -> SortFieldType {
    match sort_field.kind() {
        SortFieldKind::SortedNumeric { numeric_type, .. } => *numeric_type,
        _ => sort_field.field_type(),
    }
}

/// The numeric value of a sort field's missing value, or `0` when it has none.
///
/// Equivalent to `missingValue == null ? 0L : ((Number) missingValue).longValue()`.
fn missing_long_value(sort_field: &SortField) -> i64 {
    match sort_field.missing_value() {
        None => 0,
        Some(MissingValue::Int(value)) => i64::from(value),
        Some(MissingValue::Long(value)) => value,
        Some(MissingValue::Float(value)) => value as i64,
        Some(MissingValue::Double(value)) => value as i64,
        // The two string sentinels are not `Number`s; Java would raise a
        // `ClassCastException`, which cannot arise for the INT and LONG sorts
        // this optimization is restricted to.
        Some(_) => 0,
    }
}

impl IndexSortSortedNumericDocValuesRangeQuery {
    /// Creates a new query, both bounds inclusive.
    ///
    /// Equivalent to
    /// `new IndexSortSortedNumericDocValuesRangeQuery(String, long, long, Query)`.
    ///
    /// * `field` — the field name;
    /// * `lower_value` — the lower bound, inclusive;
    /// * `upper_value` — the upper bound, inclusive;
    /// * `fallback_query` — the query to fall back to when the optimization
    ///   cannot be applied; it must match the same documents.
    pub fn new(
        field: &str,
        lower_value: i64,
        upper_value: i64,
        fallback_query: Arc<dyn Query>,
    ) -> Self {
        Self {
            base: NumericDocValuesRangeQuery::new(field, lower_value, upper_value),
            fallback_query,
        }
    }

    /// Returns the fallback query.
    ///
    /// Equivalent to
    /// `IndexSortSortedNumericDocValuesRangeQuery.getFallbackQuery()`.
    pub fn get_fallback_query(&self) -> &Arc<dyn Query> {
        &self.fallback_query
    }

    /// Returns the field this query ranges over.
    ///
    /// Equivalent to the inherited `NumericDocValuesRangeQuery.getField()`.
    pub fn get_field(&self) -> &str {
        self.base.get_field()
    }

    /// Returns the inclusive lower bound.
    ///
    /// Equivalent to the inherited `NumericDocValuesRangeQuery.lowerValue()`.
    pub fn lower_value(&self) -> i64 {
        self.base.lower_value()
    }

    /// Returns the inclusive upper bound.
    ///
    /// Equivalent to the inherited `NumericDocValuesRangeQuery.upperValue()`.
    pub fn upper_value(&self) -> i64 {
        self.base.upper_value()
    }

    /// Equivalent to the private `matchNone(PointValues, byte[], byte[])`.
    fn match_none(
        points: &dyn PointValues,
        query_lower_point: &[u8],
        query_upper_point: &[u8],
    ) -> Result<bool> {
        let bytes_per_dim = points.bytes_per_dimension()? as usize;
        let (Some(min), Some(max)) = (points.min_packed_value()?, points.max_packed_value()?)
        else {
            return Ok(true);
        };
        Ok(
            compare_unsigned(&min, 0, query_upper_point, 0, bytes_per_dim) > 0
                || compare_unsigned(&max, 0, query_lower_point, 0, bytes_per_dim) < 0,
        )
    }

    /// Equivalent to the private `matchAll(PointValues, byte[], byte[])`.
    fn match_all(
        points: &dyn PointValues,
        query_lower_point: &[u8],
        query_upper_point: &[u8],
    ) -> Result<bool> {
        let bytes_per_dim = points.bytes_per_dimension()? as usize;
        let (Some(min), Some(max)) = (points.min_packed_value()?, points.max_packed_value()?)
        else {
            return Ok(false);
        };
        Ok(
            compare_unsigned(&min, 0, query_lower_point, 0, bytes_per_dim) >= 0
                && compare_unsigned(&max, 0, query_upper_point, 0, bytes_per_dim) <= 0,
        )
    }

    /// Returns the single-valued numeric doc values of the field, when the
    /// field is single-valued.
    ///
    /// **Divergence from Lucene 10.5.0.** Java calls
    /// `DocValues.unwrapSingleton(DocValues.getSortedNumeric(reader, field))`,
    /// which returns the wrapped numeric values whenever the sorted-numeric
    /// view is a `SingletonSortedNumericDocValues`.
    /// [`DocValues::unwrap_singleton_numeric`](crate::index::DocValues::unwrap_singleton_numeric)
    /// cannot recover the wrapped values from a trait object and always returns
    /// `None`, so the singleton case is instead recognised from the field's
    /// doc-values type: a `NUMERIC` field is exactly the case in which
    /// `DocValues.getSortedNumeric` wraps `reader.getNumericDocValues(field)`.
    /// A `SORTED_NUMERIC` field that a codec happens to expose as a singleton
    /// is therefore not recognised, so the optimization applies less often and
    /// the query falls back; the fallback matches the same documents, so the
    /// hits are unchanged.
    fn single_valued_numeric(
        context: &LeafReaderContext,
        field: &str,
    ) -> Result<Option<Box<dyn NumericDocValues>>> {
        let reader = context.leaf_reader();
        let field_infos = reader.get_field_infos();
        let Some(info) = field_infos.field_info(field) else {
            return Ok(None);
        };
        if info.get_doc_values_type() != DocValuesType::NUMERIC {
            return Ok(None);
        }
        reader.get_numeric_doc_values(field)
    }

    /// Equivalent to the private
    /// `getDocIdSetIteratorOrNullFromBkd(LeafReaderContext, DocIdSetIterator)`.
    fn doc_id_set_iterator_from_bkd(
        &self,
        context: &LeafReaderContext,
        delegate: Box<dyn DocIdSetIterator>,
    ) -> Result<Option<IteratorAndCount>> {
        let reader = context.leaf_reader();
        let meta = reader.get_meta_data();
        let Some(index_sort) = meta.sort() else {
            return Ok(None);
        };
        let Some(first) = index_sort.fields().first() else {
            return Ok(None);
        };
        if first.field() != Some(self.get_field()) {
            return Ok(None);
        }
        let reverse = first.reverse();
        let Some(points) = reader.get_point_values(self.get_field())? else {
            return Ok(None);
        };
        if points.num_dimensions()? != 1 {
            return Ok(None);
        }
        let bytes_per_dim = points.bytes_per_dimension()?;
        if bytes_per_dim != 8 && bytes_per_dim != 4 {
            return Ok(None);
        }
        if points.size() != i64::from(points.doc_count()) {
            return Ok(None);
        }
        debug_assert!(self.lower_value() <= self.upper_value());

        // Equivalent to `IntPoint.pack(int)` and `LongPoint.pack(long)`, which
        // encode one value with `NumericUtils`.
        let (query_lower_point, query_upper_point) = if bytes_per_dim == 4 {
            let mut lower = vec![0u8; 4];
            let mut upper = vec![0u8; 4];
            NumericUtils::int_to_sortable_bytes(self.lower_value() as i32, &mut lower, 0);
            NumericUtils::int_to_sortable_bytes(self.upper_value() as i32, &mut upper, 0);
            (lower, upper)
        } else {
            let mut lower = vec![0u8; 8];
            let mut upper = vec![0u8; 8];
            NumericUtils::long_to_sortable_bytes(self.lower_value(), &mut lower, 0);
            NumericUtils::long_to_sortable_bytes(self.upper_value(), &mut upper, 0);
            (lower, upper)
        };

        if Self::match_none(points.as_ref(), &query_lower_point, &query_upper_point)? {
            return Ok(Some(IteratorAndCount::empty()));
        }
        if Self::match_all(points.as_ref(), &query_lower_point, &query_upper_point)? {
            let max_doc = reader.max_doc();
            return Ok(Some(if points.doc_count() == max_doc {
                IteratorAndCount::all(max_doc)?
            } else {
                IteratorAndCount::sparse_range(0, max_doc, delegate)
            }));
        }

        let bytes_per_dim = bytes_per_dim as usize;
        let min_doc_id;
        let max_doc_id;
        if reverse {
            let mut tree = points.point_tree()?;
            min_doc_id = next_doc(
                tree.as_mut(),
                &query_upper_point,
                false,
                bytes_per_dim,
                true,
            )? + 1;
        } else {
            let mut tree = points.point_tree()?;
            min_doc_id = next_doc(
                tree.as_mut(),
                &query_lower_point,
                true,
                bytes_per_dim,
                false,
            )?;
            if min_doc_id == -1 {
                // No matches.
                return Ok(Some(IteratorAndCount::empty()));
            }
        }
        if reverse {
            let mut tree = points.point_tree()?;
            max_doc_id =
                next_doc(tree.as_mut(), &query_lower_point, true, bytes_per_dim, true)? + 1;
            if max_doc_id == 0 {
                // No matches.
                return Ok(Some(IteratorAndCount::empty()));
            }
        } else {
            let mut tree = points.point_tree()?;
            let found = next_doc(
                tree.as_mut(),
                &query_upper_point,
                false,
                bytes_per_dim,
                false,
            )?;
            max_doc_id = if found == -1 { reader.max_doc() } else { found };
        }
        if min_doc_id == max_doc_id {
            return Ok(Some(IteratorAndCount::empty()));
        }
        Ok(Some(if points.doc_count() == reader.max_doc() {
            IteratorAndCount::dense_range(min_doc_id, max_doc_id)?
        } else {
            IteratorAndCount::sparse_range(min_doc_id, max_doc_id, delegate)
        }))
    }

    /// Equivalent to the private
    /// `getDocIdSetIteratorOrNull(LeafReaderContext)`.
    fn doc_id_set_iterator_or_none(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<IteratorAndCount>> {
        if self.lower_value() > self.upper_value() {
            return Ok(Some(IteratorAndCount::empty()));
        }
        let Some(numeric_values) = Self::single_valued_numeric(context, self.get_field())? else {
            return Ok(None);
        };
        if let Some(it_and_count) =
            self.doc_id_set_iterator_from_bkd(context, numeric_as_iterator(numeric_values))?
        {
            return Ok(Some(it_and_count));
        }
        let reader = context.leaf_reader();
        let meta = reader.get_meta_data();
        let Some(index_sort) = meta.sort() else {
            return Ok(None);
        };
        let Some(sort_field) = index_sort.fields().first() else {
            return Ok(None);
        };
        if sort_field.field() != Some(self.get_field()) {
            return Ok(None);
        }
        let sort_field_type = get_sort_field_type(sort_field);
        // The index sort optimization is only supported for INT and LONG.
        if sort_field_type != SortFieldType::Int && sort_field_type != SortFieldType::Long {
            return Ok(None);
        }
        let Some(numeric_values) = Self::single_valued_numeric(context, self.get_field())? else {
            return Ok(None);
        };
        self.doc_id_set_iterator(
            sort_field,
            sort_field_type,
            context,
            numeric_as_iterator(numeric_values),
        )
        .map(Some)
    }

    /// Equivalent to the private
    /// `getDocIdSetIterator(SortField, SortField.Type, LeafReaderContext, DocIdSetIterator)`.
    fn doc_id_set_iterator(
        &self,
        sort_field: &SortField,
        sort_field_type: SortFieldType,
        context: &LeafReaderContext,
        delegate: Box<dyn DocIdSetIterator>,
    ) -> Result<IteratorAndCount> {
        let lower = if sort_field.reverse() {
            self.upper_value()
        } else {
            self.lower_value()
        };
        let upper = if sort_field.reverse() {
            self.lower_value()
        } else {
            self.upper_value()
        };
        let max_doc = context.leaf_reader().max_doc();
        let mut scorable = SimpleScorable::new();

        // A binary search finds the first document whose value is `>= lower`.
        let mut comparator = load_comparator(sort_field, sort_field_type, lower, context)?;
        let mut low = 0;
        let mut high = max_doc - 1;
        while low <= high {
            let mid = ((low as u32 + high as u32) >> 1) as i32;
            if comparator.compare(mid, &mut scorable)? <= 0 {
                high = mid - 1;
                comparator = load_comparator(sort_field, sort_field_type, lower, context)?;
            } else {
                low = mid + 1;
            }
        }
        let first_doc_id_inclusive = high + 1;

        // A binary search finds the first document whose value is `> upper`.
        // Since `upper >= lower`, the lower bound of this search starts at the
        // result of the previous one.
        let mut comparator = load_comparator(sort_field, sort_field_type, upper, context)?;
        low = first_doc_id_inclusive;
        high = max_doc - 1;
        while low <= high {
            let mid = ((low as u32 + high as u32) >> 1) as i32;
            if comparator.compare(mid, &mut scorable)? < 0 {
                high = mid - 1;
                comparator = load_comparator(sort_field, sort_field_type, upper, context)?;
            } else {
                low = mid + 1;
            }
        }
        let last_doc_id_exclusive = high + 1;
        if first_doc_id_inclusive == last_doc_id_exclusive {
            return Ok(IteratorAndCount::empty());
        }

        let reader = context.leaf_reader();
        let point_values = reader.get_point_values(self.get_field())?;
        let missing_long_value = missing_long_value(sort_field);
        // Either all documents have doc values, or the missing value falls
        // outside the range.
        if point_values
            .map(|values| values.doc_count() == reader.max_doc())
            .unwrap_or(false)
            || missing_long_value < self.lower_value()
            || missing_long_value > self.upper_value()
        {
            IteratorAndCount::dense_range(first_doc_id_inclusive, last_doc_id_exclusive)
        } else {
            Ok(IteratorAndCount::sparse_range(
                first_doc_id_inclusive,
                last_doc_id_exclusive,
                delegate,
            ))
        }
    }
}

/// A comparison of one document against a top value.
///
/// Equivalent to the private functional interface
/// `IndexSortSortedNumericDocValuesRangeQuery.ValueComparator`.
struct ValueComparator {
    comparator: Box<dyn crate::search::field_comparator::FieldComparator>,
    direction: i32,
}

impl ValueComparator {
    /// Equivalent to `ValueComparator.compare(int)`.
    fn compare(&mut self, doc: i32, scorable: &mut SimpleScorable) -> Result<i32> {
        let value = self.comparator.compare_top(doc, scorable)?;
        Ok(self.direction * value)
    }
}

/// Equivalent to the private static
/// `loadComparator(SortField, SortField.Type, long, LeafReaderContext)`.
fn load_comparator(
    sort_field: &SortField,
    field_type: SortFieldType,
    top_value: i64,
    context: &LeafReaderContext,
) -> Result<ValueComparator> {
    let mut comparator = sort_field.get_comparator(1, Pruning::NONE)?;
    if field_type == SortFieldType::Int {
        comparator.set_top_value(SortValue::Int(top_value as i32));
    } else {
        // Only INT and LONG are supported, so LONG is assumed for every other
        // case.
        comparator.set_top_value(SortValue::Long(top_value));
    }
    comparator.get_leaf_comparator(context)?;
    Ok(ValueComparator {
        comparator,
        direction: if sort_field.reverse() { -1 } else { 1 },
    })
}

/// The weight of an [`IndexSortSortedNumericDocValuesRangeQuery`].
///
/// Equivalent to the anonymous `ConstantScoreWeight` the query returns.
#[derive(Debug)]
struct IndexSortRangeWeight {
    query: Arc<IndexSortSortedNumericDocValuesRangeQuery>,
    fallback_weight: Arc<dyn Weight>,
    score: f32,
    score_mode: ScoreMode,
}

impl ConstantScoreWeightImpl for IndexSortRangeWeight {
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        if let Some(it_and_count) = self.query.doc_id_set_iterator_or_none(context)? {
            return Ok(Some(Box::new(ConstantScoreScorerSupplier::from_iterator(
                ScorerIterator::Simple(it_and_count.it),
                self.score,
                self.score_mode,
                context.leaf_reader().max_doc(),
            ))));
        }
        self.fallback_weight.scorer_supplier(context)
    }

    /// Both queries always return the same values, so it is enough to check
    /// whether the fallback query is cacheable.
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.fallback_weight.is_cacheable(ctx)
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        if !context.reader().has_deletions() {
            if self.query.lower_value() > self.query.upper_value() {
                return Ok(0);
            }
            let reader = context.leaf_reader();
            let field = self.query.get_field();
            // First use the BKD optimization when possible.
            let numeric_values =
                IndexSortSortedNumericDocValuesRangeQuery::single_valued_numeric(context, field)?;
            let point_values = reader.get_point_values(field)?;
            let points_cover_all_docs = point_values
                .as_ref()
                .map(|values| values.doc_count() == reader.max_doc())
                .unwrap_or(false);
            let mut it_and_count = None;
            if points_cover_all_docs {
                if let Some(numeric_values) = numeric_values {
                    it_and_count = self.query.doc_id_set_iterator_from_bkd(
                        context,
                        numeric_as_iterator(numeric_values),
                    )?;
                }
            }
            if let Some(it_and_count) = it_and_count.as_ref() {
                if it_and_count.count != -1 {
                    return Ok(it_and_count.count);
                }
            }

            // Then use the index sort optimization when possible.
            let meta = reader.get_meta_data();
            if let Some(index_sort) = meta.sort() {
                if let Some(sort_field) = index_sort.fields().first() {
                    if sort_field.field() == Some(field) {
                        let sort_field_type = get_sort_field_type(sort_field);
                        // The index sort optimization is only supported for INT
                        // and LONG.
                        if sort_field_type == SortFieldType::Int
                            || sort_field_type == SortFieldType::Long
                        {
                            let missing = missing_long_value(sort_field);
                            // Either all documents have doc values, or the
                            // missing value falls outside the range.
                            if points_cover_all_docs
                                || missing < self.query.lower_value()
                                || missing > self.query.upper_value()
                            {
                                if let Some(numeric_values) =
                                    IndexSortSortedNumericDocValuesRangeQuery::single_valued_numeric(
                                        context, field,
                                    )?
                                {
                                    let it_and_count = self.query.doc_id_set_iterator(
                                        sort_field,
                                        sort_field_type,
                                        context,
                                        numeric_as_iterator(numeric_values),
                                    )?;
                                    if it_and_count.count != -1 {
                                        return Ok(it_and_count.count);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        self.fallback_weight.count(context)
    }
}

impl Query for IndexSortSortedNumericDocValuesRangeQuery {
    fn to_query_string(&self, field: &str) -> String {
        let mut b = String::new();
        if self.get_field() != field {
            b.push_str(self.get_field());
            b.push(':');
        }
        b.push('[');
        b.push_str(&self.lower_value().to_string());
        b.push_str(" TO ");
        b.push_str(&self.upper_value().to_string());
        b.push(']');
        b
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(self.get_field()) {
            visitor.visit_leaf(self);
            self.fallback_query.visit(visitor);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        if self.lower_value() == i64::MIN && self.upper_value() == i64::MAX {
            return Ok(Some(Arc::new(FieldExistsQuery::new(self.get_field()))));
        }
        let rewritten_fallback = self.fallback_query.rewrite(searcher)?;
        let effective = rewritten_fallback
            .clone()
            .unwrap_or_else(|| Arc::clone(&self.fallback_query));
        if effective.as_any().is::<MatchAllDocsQuery>() {
            return Ok(Some(Arc::new(MatchAllDocsQuery::instance())));
        }
        match rewritten_fallback {
            None => Ok(None),
            Some(rewritten) => Ok(Some(Arc::new(
                IndexSortSortedNumericDocValuesRangeQuery::new(
                    self.get_field(),
                    self.lower_value(),
                    self.upper_value(),
                    rewritten,
                ),
            ))),
        }
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let fallback_weight = self
            .fallback_query
            .create_weight(searcher, score_mode, boost)?;
        let query = Arc::new(Self::new(
            self.get_field(),
            self.lower_value(),
            self.upper_value(),
            Arc::clone(&self.fallback_query),
        ));
        let inner = IndexSortRangeWeight {
            query: Arc::clone(&query),
            fallback_weight,
            score: boost,
            score_mode,
        };
        Ok(Arc::new(ConstantScoreWeight::new(query, boost, inner)))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        match other
            .as_any()
            .downcast_ref::<IndexSortSortedNumericDocValuesRangeQuery>()
        {
            Some(other) => {
                self.base == other.base && self.fallback_query.query_eq(&*other.fallback_query)
            }
            None => false,
        }
    }

    fn query_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.base.hash(&mut hasher);
        self.fallback_query.query_hash().hash(&mut hasher);
        hasher.finish()
    }
}
