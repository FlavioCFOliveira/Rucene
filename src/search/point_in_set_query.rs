//! Multi-dimensional set queries, ported from
//! `org.apache.lucene.search.PointInSetQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::cell::RefCell;
use std::fmt::Debug;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::{
    IntersectVisitor, LeafReaderContext, PointValues, PrefixCodedTerms, PrefixCodedTermsBuilder,
    Relation, MAX_INDEX_DIMENSIONS, MAX_NUM_BYTES,
};
use crate::search::constant_score_scorer::ConstantScoreScorer;
use crate::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::index_searcher::IndexSearcher;
use crate::search::point_range_query::compare_unsigned;
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::weight::Weight;
use crate::util::{Accountable, BytesRef, BytesRefIterator, DocIdSetBuilder};

/// Renders a packed point value.
///
/// Equivalent to the `protected abstract String
/// PointInSetQuery.toString(byte[] value)` hook, which every concrete set query
/// — `IntPoint.newSetQuery` and its siblings — implements with an anonymous
/// subclass.
///
/// **Divergence from Lucene 10.5.0.** As for
/// [`DimensionFormatter`](crate::search::DimensionFormatter), the anonymous
/// subclass also gives each family its own class, which `Query.sameClassAs`
/// distinguishes. This port compares formatter types through
/// [`as_any`](Self::as_any) instead, which is the same identity.
pub trait PointFormatter: Debug + Send + Sync {
    /// Renders the packed bytes of one point.
    ///
    /// Equivalent to `PointInSetQuery.toString(byte[])`.
    fn format(&self, value: &[u8]) -> String;

    /// Returns this formatter as [`Any`], so that two set queries can be told
    /// apart by the family they belong to.
    ///
    /// Every implementation writes `self`.
    fn as_any(&self) -> &dyn Any;
}

/// Abstract query class to find all documents whose single or multi-dimensional
/// point values, previously indexed with e.g. `IntPoint`, is contained in the
/// specified set.
///
/// Equivalent to `org.apache.lucene.search.PointInSetQuery`. This is for
/// subclasses and works on the underlying binary encoding: to create range
/// queries for numeric or binary values such as
/// [`IntPoint`](crate::document), refer to the factory methods on those
/// classes.
///
/// Java leaves the class abstract with a single abstract method; this port
/// takes that method as the [`PointFormatter`] supplied at construction.
#[derive(Debug)]
pub struct PointInSetQuery {
    /// A little bit overkill, since all of these "terms" are always in the same
    /// field.
    sorted_packed_points: Arc<PrefixCodedTerms>,
    sorted_packed_points_hash_code: u64,
    field: String,
    num_dims: i32,
    bytes_per_dim: usize,
    /// Cached, as Java's `ramBytesUsed` field is.
    ram_bytes_used: i64,
    lower_point: Option<Vec<u8>>,
    upper_point: Option<Vec<u8>>,
    formatter: Arc<dyn PointFormatter>,
}

impl PointInSetQuery {
    /// The shallow size of the query, which the RAM estimate starts from.
    ///
    /// Equivalent to `PointInSetQuery.BASE_RAM_BYTES`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java measures the instance with
    /// `RamUsageEstimator.shallowSizeOfInstance`; Rust has no such reflection,
    /// so the constant is the size of this struct. Only the estimate reported
    /// by [`Accountable::ram_bytes_used`] differs.
    pub const BASE_RAM_BYTES: i64 = std::mem::size_of::<PointInSetQuery>() as i64;

    /// Expert: creates a query matching any of the given packed points.
    ///
    /// Equivalent to the protected
    /// `PointInSetQuery(String, int, int, Stream)` constructor. The points must
    /// arrive in ascending unsigned byte order; duplicates are dropped.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with the message text Java
    /// produces — when `bytes_per_dim` or `num_dims` is out of range, when a
    /// point has the wrong length, or when the points are out of order; and
    /// propagates any error the iterator raises.
    pub fn new(
        field: &str,
        num_dims: i32,
        bytes_per_dim: i32,
        packed_points: &mut dyn BytesRefIterator,
        formatter: Arc<dyn PointFormatter>,
    ) -> Result<Self> {
        if bytes_per_dim < 1 || bytes_per_dim > MAX_NUM_BYTES {
            return Err(LuceneError::IllegalArgument(format!(
                "bytesPerDim must be > 0 and <= {MAX_NUM_BYTES}; got {bytes_per_dim}"
            )));
        }
        if num_dims < 1 || num_dims > MAX_INDEX_DIMENSIONS {
            return Err(LuceneError::IllegalArgument(format!(
                "numDims must be > 0 and <= {MAX_INDEX_DIMENSIONS}; got {num_dims}"
            )));
        }
        let bytes_per_dim = bytes_per_dim as usize;
        let point_len = (num_dims as usize) * bytes_per_dim;

        // In the 1D case this works well — the more points, the more common
        // prefixes they typically share — and in the multi-dimensional case,
        // where only the first dimension's prefix bytes are looked at, it can at
        // worst not hurt.
        let mut builder = PrefixCodedTermsBuilder::new();
        let mut previous: Option<BytesRef> = None;
        let mut lower_point: Option<Vec<u8>> = None;
        let mut content_bytes: i64 = 0;
        let mut hasher_input: Vec<u8> = Vec::new();
        while let Some(current) = packed_points.next()? {
            if current.length != point_len {
                return Err(LuceneError::IllegalArgument(format!(
                    "packed point length should be {} but got {}; field=\"{}\" numDims={} bytesPerDim={}",
                    point_len, current.length, field, num_dims, bytes_per_dim
                )));
            }
            match previous.as_ref() {
                None => {
                    lower_point = Some(current.slice().to_vec());
                }
                Some(previous) => match previous.cmp(&current) {
                    // Deduplicate.
                    std::cmp::Ordering::Equal => continue,
                    std::cmp::Ordering::Greater => {
                        return Err(LuceneError::IllegalArgument(format!(
                            "values are out of order: saw {previous} before {current}"
                        )))
                    }
                    std::cmp::Ordering::Less => {}
                },
            }
            builder.add_bytes(field, &current)?;
            content_bytes += current.length as i64;
            hasher_input.extend_from_slice(current.slice());
            previous = Some(BytesRef::deep_copy_of(&current));
        }
        let upper_point = previous.as_ref().map(|max| max.slice().to_vec());
        let sorted_packed_points = Arc::new(builder.finish());
        let sorted_packed_points_hash_code = hash_bytes(&hasher_input);

        Ok(Self {
            sorted_packed_points,
            sorted_packed_points_hash_code,
            ram_bytes_used: Self::BASE_RAM_BYTES + field.len() as i64 + content_bytes,
            field: field.to_string(),
            num_dims,
            bytes_per_dim,
            lower_point,
            upper_point,
            formatter,
        })
    }

    /// Returns the packed points this query matches, in sorted order.
    ///
    /// Equivalent to `PointInSetQuery.getPackedPoints()`, which returns a lazy
    /// collection over the encoded terms.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while decoding the encoded points.
    pub fn get_packed_points(&self) -> Result<Vec<Vec<u8>>> {
        let mut points = Vec::with_capacity(self.sorted_packed_points.size() as usize);
        let mut iterator = self.sorted_packed_points.iterator();
        while let Some(point) = iterator.next()? {
            points.push(point.slice().to_vec());
        }
        Ok(points)
    }

    /// Returns the field name.
    ///
    /// Equivalent to `PointInSetQuery.getField()`.
    pub fn get_field(&self) -> &str {
        &self.field
    }

    /// Returns the number of dimensions.
    ///
    /// Equivalent to `PointInSetQuery.getNumDims()`.
    pub fn get_num_dims(&self) -> i32 {
        self.num_dims
    }

    /// Returns the number of bytes per dimension.
    ///
    /// Equivalent to `PointInSetQuery.getBytesPerDim()`.
    pub fn get_bytes_per_dim(&self) -> i32 {
        self.bytes_per_dim as i32
    }
}

/// Hashes a byte sequence, standing in for `Arrays.hashCode(byte[])` over the
/// encoded points.
fn hash_bytes(bytes: &[u8]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

impl Accountable for PointInSetQuery {
    /// Equivalent to `PointInSetQuery.ramBytesUsed()`.
    fn ram_bytes_used(&self) -> i64 {
        self.ram_bytes_used
    }
}

/// Merges the sorted query points against the indexed values in one pass.
///
/// Equivalent to the private inner class `PointInSetQuery.MergePointVisitor`,
/// used for the one-dimensional case.
///
/// **Divergence from Lucene 10.5.0.** Java keeps the `BulkAdder` returned by
/// `grow(int)` in a field; this crate's [`DocIdSetBuilder::grow`] returns an
/// adder that borrows the builder, so each `visit` re-derives it with
/// `grow(0)`, which reserves nothing and appends exactly as Java's adder does.
struct MergePointVisitor<'a> {
    result: &'a mut DocIdSetBuilder,
    /// The position in the sorted query points.
    ///
    /// **Divergence from Lucene 10.5.0.** Java advances `nextQueryPoint` from
    /// `compare`, which this crate's [`IntersectVisitor::compare`] declares on
    /// `&self`; the cursor therefore lives behind a [`RefCell`], which is the
    /// Rust expression of the same mutation. Nothing re-enters the visitor, so
    /// the borrow is never contended.
    cursor: RefCell<MergeCursor>,
    bytes_per_dim: usize,
}

/// The query-point cursor a [`MergePointVisitor`] walks.
struct MergeCursor {
    iterator: crate::index::PrefixCodedTermsIterator,
    next_query_point: Option<BytesRef>,
}

impl<'a> MergePointVisitor<'a> {
    /// Equivalent to
    /// `new MergePointVisitor(PrefixCodedTerms, DocIdSetBuilder)`.
    fn new(
        sorted_packed_points: &PrefixCodedTerms,
        result: &'a mut DocIdSetBuilder,
        bytes_per_dim: usize,
    ) -> Result<Self> {
        let mut iterator = sorted_packed_points.iterator();
        let next_query_point = iterator.next()?;
        Ok(Self {
            result,
            cursor: RefCell::new(MergeCursor {
                iterator,
                next_query_point,
            }),
            bytes_per_dim,
        })
    }

    /// Equivalent to the private `MergePointVisitor.matches(byte[])`.
    fn matches(&self, packed_value: &[u8]) -> Result<bool> {
        let mut cursor = self.cursor.borrow_mut();
        loop {
            let Some(next) = cursor.next_query_point.as_ref() else {
                break;
            };
            let cmp = compare_unsigned(
                &next.bytes,
                next.offset,
                packed_value,
                0,
                self.bytes_per_dim,
            );
            if cmp == 0 {
                return Ok(true);
            } else if cmp < 0 {
                // The query point is before the index point, so move to the next
                // query point.
                cursor.next_query_point = cursor.iterator.next()?;
            } else {
                // The query point is after the index point, so do not collect
                // and return.
                break;
            }
        }
        Ok(false)
    }
}

impl IntersectVisitor for MergePointVisitor<'_> {
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

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        if self.matches(packed_value)? {
            self.visit(doc_id)?;
        }
        Ok(())
    }

    fn visit_iterator_with_value(
        &mut self,
        iterator: &mut dyn DocIdSetIterator,
        packed_value: &[u8],
    ) -> Result<()> {
        if self.matches(packed_value)? {
            self.result.grow(0).add_iterator(iterator)?;
        }
        Ok(())
    }

    /// Equivalent to `MergePointVisitor.compare(byte[], byte[])`.
    ///
    /// A decoding error while advancing the cursor — which cannot happen for a
    /// well-formed in-memory encoding — is reported as
    /// [`Relation::CellCrossesQuery`], the conservative answer that forces the
    /// cell to be evaluated document by document and surfaces the error from
    /// [`matches`](Self::matches).
    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Relation {
        let mut cursor = self.cursor.borrow_mut();
        loop {
            let Some(next) = cursor.next_query_point.as_ref() else {
                break;
            };
            let cmp_min = compare_unsigned(
                &next.bytes,
                next.offset,
                min_packed_value,
                0,
                self.bytes_per_dim,
            );
            if cmp_min < 0 {
                // The query point is before the start of this cell.
                cursor.next_query_point = match cursor.iterator.next() {
                    Ok(point) => point,
                    Err(_) => return Relation::CellCrossesQuery,
                };
                continue;
            }
            let cmp_max = compare_unsigned(
                &next.bytes,
                next.offset,
                max_packed_value,
                0,
                self.bytes_per_dim,
            );
            if cmp_max > 0 {
                // The query point is after the end of this cell.
                return Relation::CellOutsideQuery;
            }
            if cmp_min == 0 && cmp_max == 0 {
                // This is only reached on a cell whose minimum and maximum
                // values are exactly equal to the point, which happens easily
                // when many (more than 512) docs share this one value.
                return Relation::CellInsideQuery;
            }
            return Relation::CellCrossesQuery;
        }
        // All the points in the query were exhausted.
        Relation::CellOutsideQuery
    }
}

/// Intersects the tree against one query point.
///
/// Equivalent to the private inner class `PointInSetQuery.SinglePointVisitor`,
/// used for the multi-dimensional case.
struct SinglePointVisitor<'a> {
    result: &'a mut DocIdSetBuilder,
    point_bytes: Vec<u8>,
    num_dims: i32,
    bytes_per_dim: usize,
}

impl<'a> SinglePointVisitor<'a> {
    /// Equivalent to `new SinglePointVisitor(DocIdSetBuilder)`.
    fn new(result: &'a mut DocIdSetBuilder, num_dims: i32, bytes_per_dim: usize) -> Self {
        Self {
            result,
            point_bytes: vec![0; (num_dims as usize) * bytes_per_dim],
            num_dims,
            bytes_per_dim,
        }
    }

    /// Equivalent to `SinglePointVisitor.setPoint(BytesRef)`.
    fn set_point(&mut self, point: &BytesRef) {
        // The length was verified up front in the query's constructor.
        debug_assert_eq!(point.length, self.point_bytes.len());
        self.point_bytes.copy_from_slice(point.slice());
    }
}

impl IntersectVisitor for SinglePointVisitor<'_> {
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

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        debug_assert_eq!(packed_value.len(), self.point_bytes.len());
        if packed_value == self.point_bytes.as_slice() {
            // The point for this doc matches the point being queried on.
            self.visit(doc_id)?;
        }
        Ok(())
    }

    fn visit_iterator_with_value(
        &mut self,
        iterator: &mut dyn DocIdSetIterator,
        packed_value: &[u8],
    ) -> Result<()> {
        debug_assert_eq!(packed_value.len(), self.point_bytes.len());
        if packed_value == self.point_bytes.as_slice() {
            // The point for this set of docs matches the point being queried on.
            self.result.grow(0).add_iterator(iterator)?;
        }
        Ok(())
    }

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Relation {
        let mut crosses = false;
        for dim in 0..self.num_dims as usize {
            let offset = dim * self.bytes_per_dim;
            let cmp_min = compare_unsigned(
                min_packed_value,
                offset,
                &self.point_bytes,
                offset,
                self.bytes_per_dim,
            );
            if cmp_min > 0 {
                return Relation::CellOutsideQuery;
            }
            let cmp_max = compare_unsigned(
                max_packed_value,
                offset,
                &self.point_bytes,
                offset,
                self.bytes_per_dim,
            );
            if cmp_max < 0 {
                return Relation::CellOutsideQuery;
            }
            if cmp_min != 0 || cmp_max != 0 {
                crosses = true;
            }
        }
        if crosses {
            Relation::CellCrossesQuery
        } else {
            // This is only reached on a cell whose minimum and maximum values
            // are exactly equal to the point, which happens easily when many
            // docs share this one value.
            Relation::CellInsideQuery
        }
    }
}

/// The scorer supplier of a [`PointInSetQuery`].
///
/// Equivalent to the two anonymous `ScorerSupplier`s that the weight returns —
/// the merge-sort one for a single dimension and the per-point one otherwise.
struct PointInSetScorerSupplier {
    query: Arc<PointInSetQuery>,
    values: Box<dyn PointValues>,
    max_doc: i32,
    score: f32,
    score_mode: ScoreMode,
    /// Calculated lazily, only once.
    cost: Option<i64>,
}

impl std::fmt::Debug for PointInSetScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PointInSetScorerSupplier")
            .field("max_doc", &self.max_doc)
            .field("cost", &self.cost)
            .finish_non_exhaustive()
    }
}

impl PointInSetScorerSupplier {
    /// Builds the matching doc-ID set, which both `get` and `cost` walk.
    fn build(&self, estimate_only: bool) -> Result<(DocIdSetBuilder, i64)> {
        let mut result = DocIdSetBuilder::from_point_values(self.max_doc, self.values.as_ref());
        let mut cost = 0i64;
        if self.query.num_dims == 1 {
            // The common case is optimized, effectively doing a merge sort of
            // the indexed values against the queried set.
            let mut visitor = MergePointVisitor::new(
                &self.query.sorted_packed_points,
                &mut result,
                self.query.bytes_per_dim,
            )?;
            if estimate_only {
                cost = self.values.estimate_doc_count(&mut visitor)?;
            } else {
                self.values.intersect(&mut visitor)?;
            }
        } else {
            // This is a naive implementation: for each point the k-d tree is
            // re-walked to intersect. A similar optimization to the 1D case
            // would mean building a query-time k-d tree so that it could be
            // intersected efficiently against the index, which is tricky.
            let mut visitor =
                SinglePointVisitor::new(&mut result, self.query.num_dims, self.query.bytes_per_dim);
            let mut iterator = self.query.sorted_packed_points.iterator();
            while let Some(point) = iterator.next()? {
                visitor.set_point(&point);
                if estimate_only {
                    cost += self.values.estimate_doc_count(&mut visitor)?;
                } else {
                    self.values.intersect(&mut visitor)?;
                }
            }
        }
        Ok((result, cost))
    }
}

impl ScorerSupplier for PointInSetScorerSupplier {
    fn get(&mut self, _lead_cost: i64) -> Result<Box<dyn Scorer>> {
        let (result, _) = self.build(false)?;
        let iterator = result.build()?.iterator()?;
        Ok(Box::new(ConstantScoreScorer::from_iterator(
            self.score,
            self.score_mode,
            iterator,
        )))
    }

    fn cost(&self) -> i64 {
        match self.cost {
            Some(cost) => cost,
            // Computing the cost may be expensive, so it is only done if
            // necessary.
            //
            // **Divergence from Lucene 10.5.0.** Java memoises the estimate in a
            // mutable field; `ScorerSupplier::cost` takes `&self` here, so the
            // estimate is recomputed when it was not primed by
            // [`prime_cost`](Self::prime_cost). The value is the same either
            // way.
            None => self.build(true).map(|(_, cost)| cost).unwrap_or(0),
        }
    }
}

impl PointInSetScorerSupplier {
    /// Computes and memoises the cost estimate.
    ///
    /// Equivalent to the first call of Java's lazily-initialised `cost()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while estimating.
    fn prime_cost(&mut self) -> Result<()> {
        if self.cost.is_none() {
            let (_, cost) = self.build(true)?;
            debug_assert!(cost >= 0);
            self.cost = Some(cost);
        }
        Ok(())
    }
}

/// The weight of a [`PointInSetQuery`].
///
/// Equivalent to the anonymous `ConstantScoreWeight` that
/// `PointInSetQuery.createWeight` returns.
#[derive(Debug)]
struct PointInSetWeight {
    query: Arc<PointInSetQuery>,
    score: f32,
    score_mode: ScoreMode,
}

impl ConstantScoreWeightImpl for PointInSetWeight {
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let reader = context.leaf_reader();
        let Some(values) = reader.get_point_values(&self.query.field)? else {
            // No docs in this segment or field indexed any points.
            return Ok(None);
        };
        if values.num_index_dimensions()? != self.query.num_dims {
            return Err(LuceneError::IllegalArgument(format!(
                "field=\"{}\" was indexed with numIndexDims={} but this query has numIndexDims={}",
                self.query.field,
                values.num_index_dimensions()?,
                self.query.num_dims
            )));
        }
        if values.bytes_per_dimension()? != self.query.bytes_per_dim as i32 {
            return Err(LuceneError::IllegalArgument(format!(
                "field=\"{}\" was indexed with bytesPerDim={} but this query has bytesPerDim={}",
                self.query.field,
                values.bytes_per_dimension()?,
                self.query.bytes_per_dim
            )));
        }
        if values.doc_count() == 0 {
            return Ok(None);
        }
        if let (Some(lower_point), Some(upper_point)) =
            (&self.query.lower_point, &self.query.upper_point)
        {
            let field_packed_lower = values.min_packed_value()?;
            let field_packed_upper = values.max_packed_value()?;
            let (Some(field_packed_lower), Some(field_packed_upper)) =
                (field_packed_lower, field_packed_upper)
            else {
                return Ok(None);
            };
            for i in 0..self.query.num_dims as usize {
                let offset = i * self.query.bytes_per_dim;
                if compare_unsigned(
                    lower_point,
                    offset,
                    &field_packed_upper,
                    offset,
                    self.query.bytes_per_dim,
                ) > 0
                    || compare_unsigned(
                        upper_point,
                        offset,
                        &field_packed_lower,
                        offset,
                        self.query.bytes_per_dim,
                    ) < 0
                {
                    return Ok(None);
                }
            }
        }

        let mut supplier = PointInSetScorerSupplier {
            query: Arc::clone(&self.query),
            values,
            max_doc: reader.max_doc(),
            score: self.score,
            score_mode: self.score_mode,
            cost: None,
        };
        supplier.prime_cost()?;
        Ok(Some(Box::new(supplier)))
    }

    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

impl Query for PointInSetQuery {
    fn to_query_string(&self, field: &str) -> String {
        let mut sb = String::new();
        if self.field != field {
            sb.push_str(&self.field);
            sb.push(':');
        }
        sb.push('{');
        let mut iterator = self.sorted_packed_points.iterator();
        let mut first = true;
        while let Ok(Some(point)) = iterator.next() {
            if !first {
                sb.push(' ');
            }
            first = false;
            sb.push_str(&self.formatter.format(point.slice()));
        }
        sb.push('}');
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
            sorted_packed_points: Arc::clone(&self.sorted_packed_points),
            sorted_packed_points_hash_code: self.sorted_packed_points_hash_code,
            field: self.field.clone(),
            num_dims: self.num_dims,
            bytes_per_dim: self.bytes_per_dim,
            ram_bytes_used: self.ram_bytes_used,
            lower_point: self.lower_point.clone(),
            upper_point: self.upper_point.clone(),
            formatter: Arc::clone(&self.formatter),
        });
        let inner = PointInSetWeight {
            query: Arc::clone(&query),
            score: boost,
            score_mode,
        };
        Ok(Arc::new(ConstantScoreWeight::new(query, boost, inner)))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        let Some(other) = other.as_any().downcast_ref::<PointInSetQuery>() else {
            return false;
        };
        if self.formatter.as_any().type_id() != other.formatter.as_any().type_id()
            || self.field != other.field
            || self.num_dims != other.num_dims
            || self.bytes_per_dim != other.bytes_per_dim
            || self.sorted_packed_points_hash_code != other.sorted_packed_points_hash_code
            || self.sorted_packed_points.size() != other.sorted_packed_points.size()
        {
            return false;
        }
        // Java compares the two `PrefixCodedTerms` byte-wise; this crate's
        // encoding does not expose its bytes, so the decoded points are
        // compared instead, which is the same comparison.
        let mut a = self.sorted_packed_points.iterator();
        let mut b = other.sorted_packed_points.iterator();
        loop {
            match (a.next(), b.next()) {
                (Ok(None), Ok(None)) => return true,
                (Ok(Some(x)), Ok(Some(y))) if x == y => continue,
                _ => return false,
            }
        }
    }

    fn query_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.formatter.as_any().type_id().hash(&mut hasher);
        self.field.hash(&mut hasher);
        self.sorted_packed_points_hash_code.hash(&mut hasher);
        self.num_dims.hash(&mut hasher);
        self.bytes_per_dim.hash(&mut hasher);
        hasher.finish()
    }
}
