//! Multi-dimensional range queries, ported from
//! `org.apache.lucene.search.PointRangeQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::cmp::Ordering;
use std::fmt::Debug;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::point_values::{max_packed_value, min_packed_value};
use crate::index::{
    DocValuesType, IndexReaderContext, IntersectVisitor, LeafReaderContext, PointTree, PointValues,
    Relation,
};
use crate::search::constant_score_scorer_supplier::{
    ConstantScoreIteratorSupplier, ConstantScoreScorerSupplier,
};
use crate::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::field_exists_query::FieldExistsQuery;
use crate::search::index_searcher::IndexSearcher;
use crate::search::match_all_docs_query::MatchAllDocsQuery;
use crate::search::match_no_docs_query::MatchNoDocsQuery;
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::two_phase_iterator::ScorerIterator;
use crate::search::weight::Weight;
use crate::util::{BitSet, BitSetIterator, DocIdSetBuilder, FixedBitSet, IntsRef};

/// Compares the `len` bytes of `a` starting at `a_offset` with the `len` bytes
/// of `b` starting at `b_offset`, treating each byte as unsigned.
///
/// Equivalent to the `ArrayUtil.ByteArrayComparator` that
/// `ArrayUtil.getUnsignedComparator(int)` returns. Rust's `u8` ordering is
/// already unsigned, so the slice comparison is the comparator.
pub(crate) fn compare_unsigned(
    a: &[u8],
    a_offset: usize,
    b: &[u8],
    b_offset: usize,
    len: usize,
) -> i32 {
    match a[a_offset..a_offset + len].cmp(&b[b_offset..b_offset + len]) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

/// Renders one dimension of a packed point value.
///
/// Equivalent to the `protected abstract String
/// PointRangeQuery.toString(int dimension, byte[] value)` hook, which every
/// concrete range query — `IntPoint.newRangeQuery` and its siblings —
/// implements with an anonymous subclass.
///
/// **Divergence from Lucene 10.5.0.** Java's anonymous subclasses also give
/// each family of range queries its own class, which `Query.sameClassAs` uses
/// to keep an `IntPoint` range query distinct from a `LongPoint` one with the
/// same packed bytes. Rust has one concrete
/// [`PointRangeQuery`] type, so that distinction moves onto the formatter:
/// [`PointRangeQuery::query_eq`] compares formatter types through
/// [`as_any`](Self::as_any), which is the same identity Java compares.
pub trait DimensionFormatter: Debug + Send + Sync {
    /// Renders `value`, the packed bytes of dimension `dimension`.
    ///
    /// Equivalent to `PointRangeQuery.toString(int, byte[])`.
    fn format(&self, dimension: i32, value: &[u8]) -> String;

    /// Returns this formatter as [`Any`], so that two range queries can be told
    /// apart by the family they belong to.
    ///
    /// Every implementation writes `self`.
    fn as_any(&self) -> &dyn Any;
}

/// Abstract class for range queries against single or multi-dimensional points.
///
/// Equivalent to `org.apache.lucene.search.PointRangeQuery`. This is for
/// subclasses and works on the underlying binary encoding: to create range
/// queries for numeric or binary values such as
/// [`IntPoint`](crate::document), refer to the factory methods on those
/// classes — for example `IntPoint::new_range_query` for fields indexed with
/// `IntPoint`.
///
/// Java leaves the class abstract with a single abstract method; this port
/// takes that method as the [`DimensionFormatter`] supplied at construction.
#[derive(Debug)]
pub struct PointRangeQuery {
    field: String,
    num_dims: i32,
    bytes_per_dim: usize,
    lower_point: Vec<u8>,
    upper_point: Vec<u8>,
    formatter: Arc<dyn DimensionFormatter>,
}

impl PointRangeQuery {
    /// Expert: creates a new multidimensional range query for `n`-dimensional
    /// byte values.
    ///
    /// Equivalent to the protected
    /// `PointRangeQuery(String, byte[], byte[], int)` constructor.
    ///
    /// * `field` — the field name;
    /// * `lower_point` — the packed lower point, inclusive;
    /// * `upper_point` — the packed upper point, inclusive;
    /// * `num_dims` — the number of dimensions;
    /// * `formatter` — renders one dimension for
    ///   [`Query::to_query_string`].
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with the message text Java
    /// produces — when `num_dims` is not positive, when `lower_point` is empty
    /// or is not a fixed multiple of `num_dims`, or when the two points have
    /// different lengths. Java's null checks are unnecessary here.
    pub fn new(
        field: &str,
        lower_point: Vec<u8>,
        upper_point: Vec<u8>,
        num_dims: i32,
        formatter: Arc<dyn DimensionFormatter>,
    ) -> Result<Self> {
        if num_dims <= 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numDims must be positive, got {num_dims}"
            )));
        }
        if lower_point.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "lowerPoint has length of zero".to_string(),
            ));
        }
        if lower_point.len() % (num_dims as usize) != 0 {
            return Err(LuceneError::IllegalArgument(
                "lowerPoint is not a fixed multiple of numDims".to_string(),
            ));
        }
        if lower_point.len() != upper_point.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "lowerPoint has length={} but upperPoint has different length={}",
                lower_point.len(),
                upper_point.len()
            )));
        }
        let bytes_per_dim = lower_point.len() / (num_dims as usize);
        Ok(Self {
            field: field.to_string(),
            num_dims,
            bytes_per_dim,
            lower_point,
            upper_point,
            formatter,
        })
    }

    /// Returns the field name.
    ///
    /// Equivalent to `PointRangeQuery.getField()`.
    pub fn get_field(&self) -> &str {
        &self.field
    }

    /// Returns the number of dimensions.
    ///
    /// Equivalent to `PointRangeQuery.getNumDims()`.
    pub fn get_num_dims(&self) -> i32 {
        self.num_dims
    }

    /// Returns the number of bytes per dimension.
    ///
    /// Equivalent to `PointRangeQuery.getBytesPerDim()`.
    pub fn get_bytes_per_dim(&self) -> i32 {
        self.bytes_per_dim as i32
    }

    /// Returns the packed lower point.
    ///
    /// Equivalent to `PointRangeQuery.getLowerPoint()`, which clones.
    pub fn get_lower_point(&self) -> Vec<u8> {
        self.lower_point.clone()
    }

    /// Returns the packed upper point.
    ///
    /// Equivalent to `PointRangeQuery.getUpperPoint()`, which clones.
    pub fn get_upper_point(&self) -> Vec<u8> {
        self.upper_point.clone()
    }

    /// Whether `packed_value` falls inside the range in every dimension.
    ///
    /// Equivalent to the private `matches(byte[])` of the weight.
    fn matches(&self, packed_value: &[u8]) -> bool {
        let mut offset = 0;
        for _ in 0..self.num_dims {
            if compare_unsigned(
                packed_value,
                offset,
                &self.lower_point,
                offset,
                self.bytes_per_dim,
            ) < 0
            {
                // The doc's value is too low in this dimension.
                return false;
            }
            if compare_unsigned(
                packed_value,
                offset,
                &self.upper_point,
                offset,
                self.bytes_per_dim,
            ) > 0
            {
                // The doc's value is too high in this dimension.
                return false;
            }
            offset += self.bytes_per_dim;
        }
        true
    }

    /// How a BKD cell relates to this range.
    ///
    /// Equivalent to the private `PointRangeQuery.relate(byte[], byte[])`.
    fn relate(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Relation {
        let mut crosses = false;
        let mut offset = 0;
        for _ in 0..self.num_dims {
            if compare_unsigned(
                min_packed_value,
                offset,
                &self.upper_point,
                offset,
                self.bytes_per_dim,
            ) > 0
                || compare_unsigned(
                    max_packed_value,
                    offset,
                    &self.lower_point,
                    offset,
                    self.bytes_per_dim,
                ) < 0
            {
                return Relation::CellOutsideQuery;
            }
            // Evaluate `crosses` only while it is false; the loop still has to
            // visit every dimension to make sure none of them is completely
            // outside.
            if !crosses {
                crosses = compare_unsigned(
                    min_packed_value,
                    offset,
                    &self.lower_point,
                    offset,
                    self.bytes_per_dim,
                ) < 0
                    || compare_unsigned(
                        max_packed_value,
                        offset,
                        &self.upper_point,
                        offset,
                        self.bytes_per_dim,
                    ) > 0;
            }
            offset += self.bytes_per_dim;
        }
        if crosses {
            Relation::CellCrossesQuery
        } else {
            Relation::CellInsideQuery
        }
    }

    /// Checks that the field's points agree with this query's shape.
    ///
    /// Equivalent to the private `checkValidPointValues(PointValues)`, which
    /// returns `false` when no document in the segment indexed any point.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with the message text Java
    /// produces — when the field was indexed with a different number of index
    /// dimensions or a different number of bytes per dimension.
    fn check_valid_point_values(&self, values: Option<&dyn PointValues>) -> Result<bool> {
        let Some(values) = values else {
            // No docs in this segment or field indexed any points.
            return Ok(false);
        };
        if values.num_index_dimensions()? != self.num_dims {
            return Err(LuceneError::IllegalArgument(format!(
                "field=\"{}\" was indexed with numIndexDimensions={} but this query has numDims={}",
                self.field,
                values.num_index_dimensions()?,
                self.num_dims
            )));
        }
        if self.bytes_per_dim as i32 != values.bytes_per_dimension()? {
            return Err(LuceneError::IllegalArgument(format!(
                "field=\"{}\" was indexed with bytesPerDim={} but this query has bytesPerDim={}",
                self.field,
                values.bytes_per_dimension()?,
                self.bytes_per_dim
            )));
        }
        Ok(true)
    }

    /// Equivalent to the private `canRewriteToMatchAllQuery(IndexReader)`.
    fn can_rewrite_to_match_all_query(&self, searcher: &IndexSearcher) -> Result<bool> {
        for context in Arc::clone(searcher.get_index_reader()).leaves() {
            let leaf = context.leaf_reader();
            match leaf.get_point_values(&self.field)? {
                None => return Ok(false),
                Some(values) => {
                    if values.doc_count() != leaf.max_doc() {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }

    /// Equivalent to the private `canRewriteToFieldExistsQuery(IndexReader)`.
    fn can_rewrite_to_field_exists_query(&self, searcher: &IndexSearcher) -> bool {
        for context in Arc::clone(searcher.get_index_reader()).leaves() {
            let field_infos = context.leaf_reader().get_field_infos();
            if let Some(info) = field_infos.field_info(&self.field) {
                if info.get_doc_values_type() == DocValuesType::NONE
                    && !info.has_norms()
                    && info.get_vector_dimension() == 0
                {
                    // A FieldExistsQuery cannot be used on this segment.
                    return false;
                }
            }
        }
        true
    }

    /// Counts the documents whose point falls in the range, walking the tree.
    ///
    /// Equivalent to the two private `pointCount` helpers of the weight, fused:
    /// the outer one builds a visitor that tallies matching leaf values, and the
    /// inner one recurses through the tree.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while walking the tree.
    fn point_count(&self, point_tree: &mut dyn PointTree) -> Result<i64> {
        let mut visitor = CountingVisitor {
            query: self,
            matching_node_count: 0,
        };
        Self::point_count_recurse(&mut visitor, point_tree)?;
        Ok(visitor.matching_node_count)
    }

    /// Equivalent to the private
    /// `pointCount(IntersectVisitor, PointTree, long[])`.
    fn point_count_recurse(
        visitor: &mut CountingVisitor<'_>,
        point_tree: &mut dyn PointTree,
    ) -> Result<()> {
        let relation = {
            let min = point_tree.min_packed_value().to_vec();
            let max = point_tree.max_packed_value().to_vec();
            visitor.compare(&min, &max)
        };
        match relation {
            // This cell is fully outside the query shape: count none of its
            // nodes.
            Relation::CellOutsideQuery => Ok(()),
            // This cell is fully inside the query shape: count the whole node.
            Relation::CellInsideQuery => {
                visitor.matching_node_count += point_tree.size();
                Ok(())
            }
            // The cell crosses the shape boundary, or fully contains the query,
            // so fall through to full counting.
            Relation::CellCrossesQuery => {
                if point_tree.move_to_child()? {
                    loop {
                        Self::point_count_recurse(visitor, point_tree)?;
                        if !point_tree.move_to_sibling()? {
                            break;
                        }
                    }
                    point_tree.move_to_parent()?;
                } else {
                    // A leaf node was reached; the visitor tallies its matching
                    // values.
                    point_tree.visit_doc_values(visitor)?;
                }
                Ok(())
            }
        }
    }
}

/// The visitor that tallies matching values while counting.
///
/// Equivalent to the anonymous `IntersectVisitor` the private
/// `pointCount(PointTree, BiFunction, Predicate)` builds.
struct CountingVisitor<'a> {
    query: &'a PointRangeQuery,
    matching_node_count: i64,
}

impl IntersectVisitor for CountingVisitor<'_> {
    /// This branch is unreachable: the visitor is only ever handed leaf values.
    fn visit(&mut self, doc_id: i32) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(format!(
            "This IntersectVisitor does not perform any actions on a docID={doc_id} node being visited"
        )))
    }

    fn visit_with_value(&mut self, _doc_id: i32, packed_value: &[u8]) -> Result<()> {
        if self.query.matches(packed_value) {
            self.matching_node_count += 1;
        }
        Ok(())
    }

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Relation {
        self.query.relate(min_packed_value, max_packed_value)
    }
}

/// The visitor that gathers matching documents.
///
/// Equivalent to the anonymous `IntersectVisitor` that the weight's
/// `getIntersectVisitor(DocIdSetBuilder)` returns.
///
/// **Divergence from Lucene 10.5.0.** Java keeps the `BulkAdder` returned by
/// `grow(int)` in a field; this crate's [`DocIdSetBuilder::grow`] returns an
/// adder that borrows the builder, so each `visit` re-derives it with
/// `grow(0)`, which reserves nothing and appends exactly as Java's adder does.
struct MatchingVisitor<'a> {
    query: &'a PointRangeQuery,
    result: &'a mut DocIdSetBuilder,
}

impl IntersectVisitor for MatchingVisitor<'_> {
    fn grow(&mut self, count: i32) {
        self.result.grow(count);
    }

    fn visit(&mut self, doc_id: i32) -> Result<()> {
        self.result.grow(0).add(doc_id);
        Ok(())
    }

    fn visit_iterator(&mut self, iterator: &mut dyn DocIdSetIterator) -> Result<()> {
        self.result.grow(0).add_iterator(iterator)
    }

    fn visit_ints_ref(&mut self, ints_ref: &IntsRef) -> Result<()> {
        self.result.grow(0).add_ints(ints_ref);
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        if self.query.matches(packed_value) {
            self.visit(doc_id)?;
        }
        Ok(())
    }

    fn visit_iterator_with_value(
        &mut self,
        iterator: &mut dyn DocIdSetIterator,
        packed_value: &[u8],
    ) -> Result<()> {
        if self.query.matches(packed_value) {
            self.result.grow(0).add_iterator(iterator)?;
        }
        Ok(())
    }

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Relation {
        self.query.relate(min_packed_value, max_packed_value)
    }
}

/// The visitor that gathers the documents that do **not** match.
///
/// Equivalent to the anonymous `IntersectVisitor` that the weight's
/// `getInverseIntersectVisitor(FixedBitSet, long[])` returns.
struct InverseVisitor<'a> {
    query: &'a PointRangeQuery,
    result: &'a mut FixedBitSet,
    cost: i64,
}

impl IntersectVisitor for InverseVisitor<'_> {
    fn visit(&mut self, doc_id: i32) -> Result<()> {
        BitSet::set(self.result, doc_id as usize);
        self.cost += 1;
        Ok(())
    }

    fn visit_iterator(&mut self, iterator: &mut dyn DocIdSetIterator) -> Result<()> {
        let cost = iterator.cost();
        BitSet::or(self.result, iterator)?;
        self.cost += cost;
        Ok(())
    }

    fn visit_ints_ref(&mut self, ints_ref: &IntsRef) -> Result<()> {
        for i in ints_ref.offset..ints_ref.offset + ints_ref.length {
            BitSet::set(self.result, ints_ref.ints[i] as usize);
        }
        self.cost += ints_ref.length as i64;
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        if !self.query.matches(packed_value) {
            self.visit(doc_id)?;
        }
        Ok(())
    }

    fn visit_iterator_with_value(
        &mut self,
        iterator: &mut dyn DocIdSetIterator,
        packed_value: &[u8],
    ) -> Result<()> {
        if !self.query.matches(packed_value) {
            self.visit_iterator(iterator)?;
        }
        Ok(())
    }

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Relation {
        match self.query.relate(min_packed_value, max_packed_value) {
            // All points match, so skip this subtree.
            Relation::CellInsideQuery => Relation::CellOutsideQuery,
            // None of the points match, so clear all documents.
            Relation::CellOutsideQuery => Relation::CellInsideQuery,
            relation => relation,
        }
    }
}

/// The iteration half of the scorer supplier a [`PointRangeQuery`] builds.
///
/// Equivalent to the anonymous `ConstantScoreScorerSupplier` the weight
/// returns, whose `iterator(long)` and `cost()` it overrides.
struct PointRangeIteratorSupplier {
    query: Arc<PointRangeQuery>,
    values: Box<dyn PointValues>,
    max_doc: i32,
    cost: i64,
}

impl std::fmt::Debug for PointRangeIteratorSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PointRangeIteratorSupplier")
            .field("query", &self.query)
            .field("max_doc", &self.max_doc)
            .field("cost", &self.cost)
            .finish_non_exhaustive()
    }
}

impl ConstantScoreIteratorSupplier for PointRangeIteratorSupplier {
    fn iterator(&mut self, _lead_cost: i64) -> Result<ScorerIterator> {
        if self.values.doc_count() == self.max_doc
            && i64::from(self.values.doc_count()) == self.values.size()
            && self.cost() > i64::from(self.max_doc / 2)
        {
            // If all docs have exactly one value and the cost is greater than
            // half the leaf size, things may be faster by computing the set of
            // documents that do NOT match the range.
            let mut result = FixedBitSet::new(self.max_doc as usize);
            let cost = {
                let mut visitor = InverseVisitor {
                    query: self.query.as_ref(),
                    result: &mut result,
                    cost: 0,
                };
                self.values.intersect(&mut visitor)?;
                visitor.cost
            };
            // Flip the bit set and the cost.
            let result = flip_all(&result, self.max_doc as usize);
            let cost = (i64::from(self.max_doc) - cost).max(0);
            let iterator = BitSetIterator::new(Arc::new(result), cost)?;
            return Ok(ScorerIterator::Simple(Box::new(iterator)));
        }

        let mut result = DocIdSetBuilder::from_point_values(self.max_doc, self.values.as_ref());
        {
            let mut visitor = MatchingVisitor {
                query: self.query.as_ref(),
                result: &mut result,
            };
            self.values.intersect(&mut visitor)?;
        }
        Ok(ScorerIterator::Simple(result.build()?.iterator()?))
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

impl PointRangeIteratorSupplier {
    /// Computes the cost once, which Java defers to the first `cost()` call.
    ///
    /// **Divergence from Lucene 10.5.0.** Java caches the estimate in a
    /// mutable field read from `cost()`, which takes no `&mut self` here; the
    /// estimate is therefore computed when the supplier is built. It is the
    /// same estimate, from the same visitor, and a supplier is only built when
    /// its cost is about to be asked for.
    fn new(
        query: Arc<PointRangeQuery>,
        values: Box<dyn PointValues>,
        max_doc: i32,
    ) -> Result<Self> {
        let cost = {
            let mut builder = DocIdSetBuilder::from_point_values(max_doc, values.as_ref());
            let mut visitor = MatchingVisitor {
                query: query.as_ref(),
                result: &mut builder,
            };
            values.estimate_doc_count(&mut visitor)?
        };
        debug_assert!(cost >= 0);
        Ok(Self {
            query,
            values,
            max_doc,
            cost,
        })
    }
}

/// Returns the bit set with every bit in `[0, num_bits)` flipped.
///
/// Equivalent to `FixedBitSet.flip(0, maxDoc)`, which this crate's
/// [`FixedBitSet`] does not expose; the words are complemented and the bits
/// past `num_bits` are cleared so that the cardinality stays exact.
fn flip_all(set: &FixedBitSet, num_bits: usize) -> FixedBitSet {
    let mut words: Vec<u64> = set.get_bits().iter().map(|word| !word).collect();
    let remainder = num_bits & 63;
    if remainder != 0 {
        if let Some(last) = words.last_mut() {
            *last &= (1u64 << remainder) - 1;
        }
    }
    FixedBitSet::from_bits(words, num_bits)
}

/// The weight of a [`PointRangeQuery`].
///
/// Equivalent to the anonymous `ConstantScoreWeight` that
/// `PointRangeQuery.createWeight` returns.
#[derive(Debug)]
struct PointRangeWeight {
    query: Arc<PointRangeQuery>,
    score: f32,
    score_mode: ScoreMode,
}

impl ConstantScoreWeightImpl for PointRangeWeight {
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let reader = context.leaf_reader();
        let values = reader.get_point_values(self.query.get_field())?;
        if !self.query.check_valid_point_values(values.as_deref())? {
            return Ok(None);
        }
        let values = values.expect("INVARIANT: checkValidPointValues rejected the absent case");
        if values.doc_count() == 0 {
            return Ok(None);
        }

        let field_packed_lower = values.min_packed_value()?;
        let field_packed_upper = values.max_packed_value()?;
        let (Some(field_packed_lower), Some(field_packed_upper)) =
            (field_packed_lower, field_packed_upper)
        else {
            return Ok(None);
        };
        let bytes_per_dim = self.query.bytes_per_dim;
        for i in 0..self.query.num_dims as usize {
            let offset = i * bytes_per_dim;
            if compare_unsigned(
                &self.query.lower_point,
                offset,
                &field_packed_upper,
                offset,
                bytes_per_dim,
            ) > 0
                || compare_unsigned(
                    &self.query.upper_point,
                    offset,
                    &field_packed_lower,
                    offset,
                    bytes_per_dim,
                ) < 0
            {
                // Returning None here helps make sure that, when this query is a
                // required clause of a boolean query, the other required clauses
                // are not asked for a scorer — an expensive operation for some
                // queries.
                return Ok(None);
            }
        }

        let all_docs_match = if values.doc_count() == reader.max_doc() {
            let mut all = true;
            for i in 0..self.query.num_dims as usize {
                let offset = i * bytes_per_dim;
                if compare_unsigned(
                    &self.query.lower_point,
                    offset,
                    &field_packed_lower,
                    offset,
                    bytes_per_dim,
                ) > 0
                    || compare_unsigned(
                        &self.query.upper_point,
                        offset,
                        &field_packed_upper,
                        offset,
                        bytes_per_dim,
                    ) < 0
                {
                    all = false;
                    break;
                }
            }
            all
        } else {
            false
        };

        if all_docs_match {
            // All docs have a value and all points are within bounds, so
            // everything matches.
            Ok(Some(Box::new(ConstantScoreScorerSupplier::match_all(
                self.score,
                self.score_mode,
                reader.max_doc(),
            )?)))
        } else {
            let inner =
                PointRangeIteratorSupplier::new(Arc::clone(&self.query), values, reader.max_doc())?;
            Ok(Some(Box::new(ConstantScoreScorerSupplier::new(
                self.score,
                self.score_mode,
                reader.max_doc(),
                inner,
            ))))
        }
    }

    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        let reader = context.leaf_reader();
        let values = reader.get_point_values(self.query.get_field())?;
        if !self.query.check_valid_point_values(values.as_deref())? {
            return Ok(0);
        }
        let values = values.expect("INVARIANT: checkValidPointValues rejected the absent case");
        if !context.reader().has_deletions() {
            let min = values.min_packed_value()?;
            let max = values.max_packed_value()?;
            if let (Some(min), Some(max)) = (min, max) {
                if self.query.relate(&min, &max) == Relation::CellInsideQuery {
                    return Ok(values.doc_count());
                }
            }
            // Only 1D: there is a guarantee that this runs fast, because there
            // are at most two crossing leaves. `docCount == size` means the
            // field is single-valued, so counting the points in the leaf nodes
            // counts documents.
            if self.query.num_dims == 1 && i64::from(values.doc_count()) == values.size() {
                let mut tree = values.point_tree()?;
                return Ok(self.query.point_count(tree.as_mut())? as i32);
            }
        }
        Ok(-1)
    }
}

impl Query for PointRangeQuery {
    fn to_query_string(&self, field: &str) -> String {
        let mut sb = String::new();
        if self.field != field {
            sb.push_str(&self.field);
            sb.push(':');
        }
        // Print ourselves as "range per dimension".
        for i in 0..self.num_dims {
            if i > 0 {
                sb.push(',');
            }
            let start_offset = self.bytes_per_dim * (i as usize);
            let end_offset = start_offset + self.bytes_per_dim;
            sb.push('[');
            sb.push_str(
                &self
                    .formatter
                    .format(i, &self.lower_point[start_offset..end_offset]),
            );
            sb.push_str(" TO ");
            sb.push_str(
                &self
                    .formatter
                    .format(i, &self.upper_point[start_offset..end_offset]),
            );
            sb.push(']');
        }
        sb
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(&self.field) {
            visitor.visit_leaf(self);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        // A RandomAccessWeight is no good here: approximating with "match all
        // docs" is wasteful for an inverted structure, which should be used in
        // the first pass.
        let query = Arc::new(Self {
            field: self.field.clone(),
            num_dims: self.num_dims,
            bytes_per_dim: self.bytes_per_dim,
            lower_point: self.lower_point.clone(),
            upper_point: self.upper_point.clone(),
            formatter: Arc::clone(&self.formatter),
        });
        let inner = PointRangeWeight {
            query: Arc::clone(&query),
            score: boost,
            score_mode,
        };
        Ok(Arc::new(ConstantScoreWeight::new(query, boost, inner)))
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        let reader = Arc::clone(searcher.get_index_reader());
        for leaf in Arc::clone(&reader).leaves() {
            let values = leaf.leaf_reader().get_point_values(&self.field)?;
            self.check_valid_point_values(values.as_deref())?;
        }
        // Fetch the global min/max packed values across all segments.
        let global_min_packed = min_packed_value(Arc::clone(&reader), &self.field)?;
        let global_max_packed = max_packed_value(Arc::clone(&reader), &self.field)?;
        let (Some(global_min_packed), Some(global_max_packed)) =
            (global_min_packed, global_max_packed)
        else {
            return Ok(Some(Arc::new(MatchNoDocsQuery::instance())));
        };
        match self.relate(&global_min_packed, &global_max_packed) {
            Relation::CellInsideQuery => {
                if self.can_rewrite_to_match_all_query(searcher)? {
                    Ok(Some(Arc::new(MatchAllDocsQuery::instance())))
                } else if self.can_rewrite_to_field_exists_query(searcher) {
                    Ok(Some(Arc::new(FieldExistsQuery::new(&self.field))))
                } else {
                    Ok(None)
                }
            }
            Relation::CellOutsideQuery => Ok(Some(Arc::new(MatchNoDocsQuery::instance()))),
            Relation::CellCrossesQuery => Ok(None),
        }
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        let Some(other) = other.as_any().downcast_ref::<PointRangeQuery>() else {
            return false;
        };
        self.formatter.as_any().type_id() == other.formatter.as_any().type_id()
            && self.field == other.field
            && self.num_dims == other.num_dims
            && self.bytes_per_dim == other.bytes_per_dim
            && self.lower_point == other.lower_point
            && self.upper_point == other.upper_point
    }

    fn query_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.formatter.as_any().type_id().hash(&mut hasher);
        self.field.hash(&mut hasher);
        self.lower_point.hash(&mut hasher);
        self.upper_point.hash(&mut hasher);
        self.num_dims.hash(&mut hasher);
        self.bytes_per_dim.hash(&mut hasher);
        hasher.finish()
    }
}
