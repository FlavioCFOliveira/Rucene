//! The two distance feature queries, ported from
//! `org.apache.lucene.document.LongDistanceFeatureQuery` and
//! `org.apache.lucene.document.LatLonPointDistanceFeatureQuery`.
//!
//! Both score a document by how close its value is to an origin —
//! `weight * pivotDistance / (pivotDistance + distance)` — and both exploit the
//! points index: once a collector raises the minimum competitive score, the
//! scorer computes the largest distance that can still be competitive and
//! replaces its iterator with the (much smaller) set of documents inside the
//! matching range, materialised straight out of the BKD tree.

use std::cell::RefCell;
use std::rc::Rc;

use crate::document::geo_fields::LatLonPoint;
use crate::error::{LuceneError, Result};
use crate::geo::encoding::{GeoEncodingUtils, GeoUtils};
use crate::geo::geometry::Rectangle;
use crate::index::point_values::{
    is_estimated_point_count_greater_than_or_equal_to, IntersectVisitor, Relation,
};
use crate::index::{DocValues, LeafReader, PointValues, SortedNumericDocValues};
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::scorable::Scorable;
use crate::search::scorer::Scorer;
use crate::search::similarities::Explanation;
use crate::util::doc_id_set::DocIdSetBuilder;
use crate::util::sloppy_math::SloppyMath;
use crate::util::{IntsRef, NumericUtils};

/// Both `MIN_SKIP_INTERVAL` and `MAX_SKIP_INTERVAL` are powers of two.
///
/// Equivalent to `LongDistanceFeatureQuery.MIN_SKIP_INTERVAL`.
const MIN_SKIP_INTERVAL: i32 = 32;

/// Equivalent to `LongDistanceFeatureQuery.MAX_SKIP_INTERVAL`.
const MAX_SKIP_INTERVAL: i32 = 8192;

// -----------------------------------------------------------------------------
// Value selection
// -----------------------------------------------------------------------------

/// Which of a document's several values a distance feature query scores on.
///
/// Equivalent to the two private `selectValue(SortedNumericDocValues)` methods,
/// one per query.
#[derive(Clone, Copy, Debug)]
enum ValueSelector {
    /// Picks the value closest to `origin`, comparing distances unsigned so an
    /// underflow does not invert the order.
    ///
    /// Equivalent to `LongDistanceFeatureQuery.selectValue`.
    Long { origin: i64 },
    /// Picks the encoded location whose haversine sort key to the origin is
    /// smallest.
    ///
    /// Equivalent to `LatLonPointDistanceFeatureQuery.selectValue`.
    LatLon { origin_lat: f64, origin_lon: f64 },
}

impl ValueSelector {
    fn select(self, multi: &mut dyn SortedNumericDocValues) -> Result<i64> {
        match self {
            Self::Long { origin } => {
                let count = multi.doc_value_count()?;
                let mut next = multi.next_value()?;
                if count == 1 || next >= origin {
                    return Ok(next);
                }
                let mut previous = next;
                for _ in 1..count {
                    next = multi.next_value()?;
                    if next >= origin {
                        // An unsigned comparison, because of underflows.
                        return Ok(
                            if (origin.wrapping_sub(previous) as u64)
                                < (next.wrapping_sub(origin) as u64)
                            {
                                previous
                            } else {
                                next
                            },
                        );
                    }
                    previous = next;
                }
                debug_assert!(next < origin);
                Ok(next)
            }
            Self::LatLon {
                origin_lat,
                origin_lon,
            } => {
                let count = multi.doc_value_count()?;
                let mut value = multi.next_value()?;
                if count == 1 {
                    return Ok(value);
                }
                // Compute the exact sort key, avoiding any `asin()` computation.
                let mut distance = distance_key_from_encoded(origin_lat, origin_lon, value);
                for _ in 1..count {
                    let next_value = multi.next_value()?;
                    let next_distance =
                        distance_key_from_encoded(origin_lat, origin_lon, next_value);
                    if next_distance < distance {
                        distance = next_distance;
                        value = next_value;
                    }
                }
                Ok(value)
            }
        }
    }
}

/// Equivalent to the private
/// `LatLonPointDistanceFeatureQuery.getDistanceKeyFromEncoded(long)`.
fn distance_key_from_encoded(origin_lat: f64, origin_lon: f64, encoded: i64) -> f64 {
    let latitude_bits = (encoded >> 32) as i32;
    let longitude_bits = (encoded & 0xFFFF_FFFF) as i32;
    let lat = GeoEncodingUtils::decode_latitude(latitude_bits);
    let lon = GeoEncodingUtils::decode_longitude(longitude_bits);
    SloppyMath::haversin_sort_key(origin_lat, origin_lon, lat, lon)
}

/// Equivalent to the private
/// `LatLonPointDistanceFeatureQuery.getDistanceFromEncoded(long)`.
fn distance_from_encoded(origin_lat: f64, origin_lon: f64, encoded: i64) -> f64 {
    SloppyMath::haversin_meters_from_sort_key(distance_key_from_encoded(
        origin_lat, origin_lon, encoded,
    ))
}

/// Presents a multi-valued field as one value per document.
///
/// Equivalent to the anonymous `NumericDocValues` that both queries'
/// `selectValues(SortedNumericDocValues)` returns.
///
/// **Divergence from Lucene 10.5.0.** Java first tries
/// `DocValues.unwrapSingleton(multiDocValues)` and uses the underlying
/// single-valued iterator when there is one. That accessor hands out a borrow
/// in this port, which cannot be owned by the scorer, so the wrapper is always
/// used. It is behaviourally identical: with a single value per document,
/// `selectValue` returns exactly that value.
struct SelectedNumericDocValues {
    multi: Box<dyn SortedNumericDocValues>,
    selector: ValueSelector,
    value: i64,
}

impl SelectedNumericDocValues {
    fn new(multi: Box<dyn SortedNumericDocValues>, selector: ValueSelector) -> Self {
        Self {
            multi,
            selector,
            value: 0,
        }
    }

    /// Equivalent to the wrapper's `longValue()`.
    fn long_value(&self) -> i64 {
        self.value
    }

    /// Equivalent to the wrapper's `advanceExact(int)`.
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        if self.multi.advance_exact(target)? {
            self.value = self.selector.select(self.multi.as_mut())?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Equivalent to the wrapper's `docID()`. The scorer tracks the current
    /// document in [`DistanceIterator`] instead — exactly as Java's scorer
    /// tracks it in its own `doc` field — so nothing in this port calls it; it
    /// is kept because it is part of the `NumericDocValues` contract the
    /// wrapper implements.
    #[allow(dead_code)]
    fn doc_id(&self) -> i32 {
        self.multi.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.multi.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.multi.advance(target)
    }

    fn cost(&self) -> i64 {
        self.multi.cost()
    }
}

// -----------------------------------------------------------------------------
// The scorer's iterator
// -----------------------------------------------------------------------------

/// Where a distance scorer currently draws its documents from.
///
/// Equivalent to the `DocIdSetIterator it` field the scorer swaps: it starts as
/// the field's doc values, so every document with a value is iterated, and is
/// replaced by a materialised set once the minimum competitive score makes the
/// range selective enough.
enum IteratorSource {
    /// The field's doc values.
    DocValues,
    /// The documents inside the current competitive range.
    Materialized(Box<dyn DocIdSetIterator>),
    /// Nothing can be competitive any more.
    Empty,
}

/// The iterator view a distance scorer hands out.
///
/// Equivalent to the anonymous `DocIdSetIterator` `DistanceScorer.iterator()`
/// returns, whose indirection lets the scorer swap its source while the
/// consumer keeps the same object.
struct DistanceIterator {
    doc: i32,
    source: IteratorSource,
    values: Rc<RefCell<SelectedNumericDocValues>>,
}

impl DocIdSetIterator for DistanceIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.doc = match &mut self.source {
            IteratorSource::DocValues => self.values.borrow_mut().next_doc()?,
            IteratorSource::Materialized(it) => it.next_doc()?,
            IteratorSource::Empty => NO_MORE_DOCS,
        };
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc = match &mut self.source {
            IteratorSource::DocValues => self.values.borrow_mut().advance(target)?,
            IteratorSource::Materialized(it) => it.advance(target)?,
            IteratorSource::Empty => NO_MORE_DOCS,
        };
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        match &self.source {
            IteratorSource::DocValues => self.values.borrow().cost(),
            IteratorSource::Materialized(it) => it.cost(),
            IteratorSource::Empty => 0,
        }
    }
}

// -----------------------------------------------------------------------------
// LongDistanceFeatureQuery
// -----------------------------------------------------------------------------

/// Scores a document by how close its `long` value is to an origin.
///
/// Equivalent to `org.apache.lucene.document.LongDistanceFeatureQuery`, which
/// [`LongField::new_distance_feature_query`](crate::document::LongField) and
/// its siblings build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LongDistanceFeatureQuery {
    field: String,
    origin: i64,
    pivot_distance: i64,
}

impl LongDistanceFeatureQuery {
    /// Creates the query.
    ///
    /// Equivalent to `LongDistanceFeatureQuery(String, long, long)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `pivot_distance` is not
    /// positive.
    pub fn new(field: impl Into<String>, origin: i64, pivot_distance: i64) -> Result<Self> {
        if pivot_distance <= 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "pivotDistance must be > 0, got {pivot_distance}"
            )));
        }
        Ok(Self {
            field: field.into(),
            origin,
            pivot_distance,
        })
    }

    /// Returns the field the query reads.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns the origin distances are measured from.
    pub fn origin(&self) -> i64 {
        self.origin
    }

    /// Returns the distance at which the score halves.
    pub fn pivot_distance(&self) -> i64 {
        self.pivot_distance
    }

    /// Explains the score of one document.
    ///
    /// Equivalent to the `Weight.explain(LeafReaderContext, int)` of the weight
    /// `LongDistanceFeatureQuery.createWeight` builds.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the doc values raises.
    pub fn explain(&self, reader: &dyn LeafReader, doc: i32, boost: f32) -> Result<Explanation> {
        let mut multi = match reader.get_sorted_numeric_doc_values(&self.field)? {
            Some(values) => values,
            None => Box::new(DocValues::empty_sorted_numeric()),
        };
        if !multi.advance_exact(doc)? {
            return Ok(Explanation::no_match(
                format!(
                    "Document {doc} doesn't have a value for field {}",
                    self.field
                ),
                Vec::new(),
            ));
        }
        let selector = ValueSelector::Long {
            origin: self.origin,
        };
        let value = selector.select(multi.as_mut())?;
        let distance = long_distance(value, self.origin);
        let score = (f64::from(boost)
            * (self.pivot_distance as f64 / (self.pivot_distance as f64 + distance as f64)))
            as f32;
        Ok(Explanation::matched(
            score,
            "Distance score, computed as weight * pivotDistance / (pivotDistance + abs(value - \
             origin)) from:",
            vec![
                Explanation::matched(boost, "weight", Vec::new()),
                Explanation::matched(self.pivot_distance, "pivotDistance", Vec::new()),
                Explanation::matched(self.origin, "origin", Vec::new()),
                Explanation::matched(value, "current value", Vec::new()),
            ],
        ))
    }

    /// Returns the scorer over one leaf, or `None` when the leaf holds no
    /// points for the field.
    ///
    /// Equivalent to the `ScorerSupplier.get(long)` of the weight
    /// `LongDistanceFeatureQuery.createWeight` builds.
    ///
    /// # Errors
    ///
    /// Propagates whatever the reader raises.
    pub fn scorer(
        &self,
        reader: &dyn LeafReader,
        lead_cost: i64,
        boost: f32,
    ) -> Result<Option<LongDistanceScorer>> {
        let Some(point_values) = reader.get_point_values(&self.field)? else {
            // No data on this segment.
            return Ok(None);
        };
        let multi = match reader.get_sorted_numeric_doc_values(&self.field)? {
            Some(values) => values,
            None => Box::new(DocValues::empty_sorted_numeric()),
        };
        let values = Rc::new(RefCell::new(SelectedNumericDocValues::new(
            multi,
            ValueSelector::Long {
                origin: self.origin,
            },
        )));
        Ok(Some(LongDistanceScorer {
            max_doc: reader.max_doc(),
            lead_cost,
            boost,
            origin: self.origin,
            pivot_distance: self.pivot_distance,
            point_values,
            values: Rc::clone(&values),
            iter: DistanceIterator {
                doc: -1,
                // Initially use doc values, so every document that has a value
                // for this field is iterated.
                source: IteratorSource::DocValues,
                values,
            },
            max_distance: i64::MAX,
            current_skip_interval: MIN_SKIP_INTERVAL,
            try_update_fail_count: 0,
            set_min_competitive_score_counter: 0,
        }))
    }

    /// Returns the cost of this query over one leaf.
    ///
    /// Equivalent to the `ScorerSupplier.cost()` Java answers with
    /// `docValues.cost()`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the reader raises.
    pub fn cost(&self, reader: &dyn LeafReader) -> Result<i64> {
        Ok(match reader.get_sorted_numeric_doc_values(&self.field)? {
            Some(values) => values.cost(),
            None => DocValues::empty_sorted_numeric().cost(),
        })
    }

    /// Prints this query.
    ///
    /// Equivalent to `LongDistanceFeatureQuery.toString(String)`, which — as in
    /// Java — prints the *argument* rather than this query's own field.
    pub fn to_query_string(&self, field: &str) -> String {
        format!(
            "LongDistanceFeatureQuery(field={field},origin={},pivotDistance={})",
            self.origin, self.pivot_distance
        )
    }
}

/// The unsigned distance between a value and the origin.
///
/// Equivalent to the `Math.max(v, origin) - Math.min(v, origin)` of
/// `LongDistanceFeatureQuery`, including its underflow guard: a distance that
/// does not fit in a `long` is treated as `Long.MAX_VALUE`.
fn long_distance(value: i64, origin: i64) -> i64 {
    let distance = value.max(origin).wrapping_sub(value.min(origin));
    if distance < 0 {
        i64::MAX
    } else {
        distance
    }
}

/// The scorer of a [`LongDistanceFeatureQuery`].
///
/// Equivalent to the private inner class
/// `LongDistanceFeatureQuery.DistanceScorer`.
pub struct LongDistanceScorer {
    max_doc: i32,
    lead_cost: i64,
    boost: f32,
    origin: i64,
    pivot_distance: i64,
    point_values: Box<dyn PointValues>,
    values: Rc<RefCell<SelectedNumericDocValues>>,
    iter: DistanceIterator,
    max_distance: i64,
    current_skip_interval: i32,
    /// Helps to be conservative about increasing the sampling interval.
    try_update_fail_count: i32,
    set_min_competitive_score_counter: i32,
}

impl std::fmt::Debug for LongDistanceScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LongDistanceScorer")
            .field("origin", &self.origin)
            .field("pivot_distance", &self.pivot_distance)
            .field("max_distance", &self.max_distance)
            .finish_non_exhaustive()
    }
}

impl LongDistanceScorer {
    /// Equivalent to the private `DistanceScorer.score(long)`.
    fn score_distance(&self, distance: i64) -> f32 {
        (f64::from(self.boost)
            * (self.pivot_distance as f64 / (self.pivot_distance as f64 + distance as f64)))
            as f32
    }

    /// Binary-searches the maximum distance that still scores at least
    /// `min_score`.
    ///
    /// Equivalent to the private `DistanceScorer.computeMaxDistance(float, long)`.
    /// Inverting the score computation directly is very hard because of the
    /// rounding errors, hence the search.
    fn compute_max_distance(&self, min_score: f32, previous_max_distance: i64) -> i64 {
        debug_assert!(self.score_distance(0) >= min_score);
        if self.score_distance(previous_max_distance) >= min_score {
            // `min_score` did not decrease enough to require an update.
            return previous_max_distance;
        }
        let mut min = 0i64;
        let mut max = previous_max_distance;
        // Invariant: score(min) >= min_score && score(max) < min_score.
        while max - min > 1 {
            let mid = (((min as u64) + (max as u64)) >> 1) as i64;
            if self.score_distance(mid) >= min_score {
                min = mid;
            } else {
                max = mid;
            }
        }
        min
    }

    /// Equivalent to the private `DistanceScorer.updateSkipInterval(boolean)`.
    fn update_skip_interval(&mut self, success: bool) {
        if self.set_min_competitive_score_counter > 256 {
            if success {
                self.current_skip_interval =
                    (self.current_skip_interval / 2).max(MIN_SKIP_INTERVAL);
                self.try_update_fail_count = 0;
            } else if self.try_update_fail_count >= 3 {
                self.current_skip_interval =
                    (self.current_skip_interval * 2).min(MAX_SKIP_INTERVAL);
                self.try_update_fail_count = 0;
            } else {
                self.try_update_fail_count += 1;
            }
        }
    }
}

impl Scorable for LongDistanceScorer {
    fn score(&mut self) -> Result<f32> {
        let doc = self.iter.doc;
        if !self.values.borrow_mut().advance_exact(doc)? {
            return Ok(0.0);
        }
        let v = self.values.borrow().long_value();
        // The distance is unsigned; one that overflows is treated as
        // `Long.MAX_VALUE`.
        Ok(self.score_distance(long_distance(v, self.origin)))
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        if min_score > self.boost {
            self.iter.source = IteratorSource::Empty;
            return Ok(());
        }

        // Start sampling once called too often.
        self.set_min_competitive_score_counter += 1;
        if self.set_min_competitive_score_counter > 256
            && (self.set_min_competitive_score_counter & (self.current_skip_interval - 1))
                != self.current_skip_interval - 1
        {
            return Ok(());
        }

        let previous_max_distance = self.max_distance;
        self.max_distance = self.compute_max_distance(min_score, self.max_distance);
        if self.max_distance == previous_max_distance {
            // Nothing to update.
            return Ok(());
        }
        let mut min_value = self.origin.wrapping_sub(self.max_distance);
        if min_value > self.origin {
            // Underflow.
            min_value = i64::MIN;
        }
        let mut max_value = self.origin.wrapping_add(self.max_distance);
        if max_value < self.origin {
            // Overflow.
            max_value = i64::MAX;
        }

        let mut result = DocIdSetBuilder::new(self.max_doc);
        let doc = self.iter.doc;
        let current_query_cost = self.lead_cost.min(self.iter.cost());
        // The right factor compared with the current iterator is a guess; eight
        // is what Lucene uses.
        let threshold = ((current_query_cost as u64) >> 3) as i64;
        {
            let mut visitor = LongRangeVisitor {
                result: &mut result,
                doc,
                min: min_value,
                max: max_value,
            };
            let mut tree = self.point_values.point_tree()?;
            if is_estimated_point_count_greater_than_or_equal_to(
                &mut visitor,
                tree.as_mut(),
                threshold,
            )? {
                // The new range is not selective enough to be worth
                // materialising.
                drop(tree);
                self.update_skip_interval(false);
                return Ok(());
            }
            drop(tree);
            self.point_values.intersect(&mut visitor)?;
        }
        self.iter.source = IteratorSource::Materialized(result.build()?.iterator()?);
        self.update_skip_interval(true);
        Ok(())
    }
}

impl Scorer for LongDistanceScorer {
    fn as_scorable(&mut self) -> &mut dyn Scorable {
        self
    }

    fn doc_id(&self) -> i32 {
        self.iter.doc
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.iter
    }

    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(self.boost)
    }
}

/// Collects the documents whose `long` value falls in the competitive range.
///
/// Equivalent to the anonymous `IntersectVisitor` of
/// `LongDistanceFeatureQuery.DistanceScorer.setMinCompetitiveScore`.
struct LongRangeVisitor<'a> {
    result: &'a mut DocIdSetBuilder,
    doc: i32,
    min: i64,
    max: i64,
}

impl IntersectVisitor for LongRangeVisitor<'_> {
    fn grow(&mut self, count: i32) {
        self.result.grow(count);
    }

    fn visit(&mut self, doc_id: i32) -> Result<()> {
        if doc_id <= self.doc {
            // Already visited or skipped.
            return Ok(());
        }
        self.result.grow(0).add(doc_id);
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        if doc_id <= self.doc {
            // Already visited or skipped.
            return Ok(());
        }
        let doc_value = NumericUtils::sortable_bytes_to_long(packed_value, 0);
        if doc_value < self.min || doc_value > self.max {
            // The document's value is out of range in this dimension.
            return Ok(());
        }
        // The document is in bounds.
        self.result.grow(0).add(doc_id);
        Ok(())
    }

    fn visit_iterator(&mut self, iterator: &mut dyn DocIdSetIterator) -> Result<()> {
        loop {
            let doc_id = iterator.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                return Ok(());
            }
            self.visit(doc_id)?;
        }
    }

    fn visit_ints_ref(&mut self, ints_ref: &IntsRef) -> Result<()> {
        for doc_id in ints_ref.slice().iter().copied() {
            self.visit(doc_id)?;
        }
        Ok(())
    }

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Relation {
        let min_doc_value = NumericUtils::sortable_bytes_to_long(min_packed_value, 0);
        let max_doc_value = NumericUtils::sortable_bytes_to_long(max_packed_value, 0);
        if min_doc_value > self.max || max_doc_value < self.min {
            return Relation::CellOutsideQuery;
        }
        if min_doc_value < self.min || max_doc_value > self.max {
            return Relation::CellCrossesQuery;
        }
        Relation::CellInsideQuery
    }
}

// -----------------------------------------------------------------------------
// LatLonPointDistanceFeatureQuery
// -----------------------------------------------------------------------------

/// Scores a document by how close its location is to an origin.
///
/// Equivalent to
/// `org.apache.lucene.document.LatLonPointDistanceFeatureQuery`, which
/// `LatLonPoint.newDistanceFeatureQuery` builds.
#[derive(Clone, Debug)]
pub struct LatLonPointDistanceFeatureQuery {
    field: String,
    origin_lat: f64,
    origin_lon: f64,
    pivot_distance: f64,
}

impl LatLonPointDistanceFeatureQuery {
    /// Creates the query.
    ///
    /// Equivalent to
    /// `LatLonPointDistanceFeatureQuery(String, double, double, double)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for an invalid latitude or
    /// longitude, and when `pivot_distance` is not positive.
    pub fn new(
        field: impl Into<String>,
        origin_lat: f64,
        origin_lon: f64,
        pivot_distance: f64,
    ) -> Result<Self> {
        GeoUtils::check_latitude(origin_lat)?;
        GeoUtils::check_longitude(origin_lon)?;
        if pivot_distance <= 0.0 {
            return Err(LuceneError::IllegalArgument(format!(
                "pivotDistance must be > 0, got {pivot_distance}"
            )));
        }
        Ok(Self {
            field: field.into(),
            origin_lat,
            origin_lon,
            pivot_distance,
        })
    }

    /// Returns the field the query reads.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns the origin latitude.
    pub fn origin_lat(&self) -> f64 {
        self.origin_lat
    }

    /// Returns the origin longitude.
    pub fn origin_lon(&self) -> f64 {
        self.origin_lon
    }

    /// Returns the distance at which the score halves.
    pub fn pivot_distance(&self) -> f64 {
        self.pivot_distance
    }

    /// Explains the score of one document.
    ///
    /// Equivalent to the `Weight.explain(LeafReaderContext, int)` of the weight
    /// `LatLonPointDistanceFeatureQuery.createWeight` builds.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the doc values raises.
    pub fn explain(&self, reader: &dyn LeafReader, doc: i32, boost: f32) -> Result<Explanation> {
        let mut multi = match reader.get_sorted_numeric_doc_values(&self.field)? {
            Some(values) => values,
            None => Box::new(DocValues::empty_sorted_numeric()),
        };
        if !multi.advance_exact(doc)? {
            return Ok(Explanation::no_match(
                format!(
                    "Document {doc} doesn't have a value for field {}",
                    self.field
                ),
                Vec::new(),
            ));
        }
        let selector = ValueSelector::LatLon {
            origin_lat: self.origin_lat,
            origin_lon: self.origin_lon,
        };
        let encoded = selector.select(multi.as_mut())?;
        let latitude_bits = (encoded >> 32) as i32;
        let longitude_bits = (encoded & 0xFFFF_FFFF) as i32;
        let lat = GeoEncodingUtils::decode_latitude(latitude_bits);
        let lon = GeoEncodingUtils::decode_longitude(longitude_bits);
        let distance = SloppyMath::haversin_meters(self.origin_lat, self.origin_lon, lat, lon);
        let score =
            (f64::from(boost) * (self.pivot_distance / (self.pivot_distance + distance))) as f32;
        Ok(Explanation::matched(
            score,
            "Distance score, computed as weight * pivotDistance / (pivotDistance + abs(distance)) \
             from:",
            vec![
                Explanation::matched(boost, "weight", Vec::new()),
                Explanation::matched(self.pivot_distance, "pivotDistance", Vec::new()),
                Explanation::matched(self.origin_lat, "originLat", Vec::new()),
                Explanation::matched(self.origin_lon, "originLon", Vec::new()),
                Explanation::matched(lat, "current lat", Vec::new()),
                Explanation::matched(lon, "current lon", Vec::new()),
                Explanation::matched(distance, "distance", Vec::new()),
            ],
        ))
    }

    /// Returns the scorer over one leaf, or `None` when the leaf holds no
    /// points for the field.
    ///
    /// Equivalent to the `ScorerSupplier.get(long)` of the weight
    /// `LatLonPointDistanceFeatureQuery.createWeight` builds.
    ///
    /// # Errors
    ///
    /// Propagates whatever the reader raises.
    pub fn scorer(
        &self,
        reader: &dyn LeafReader,
        lead_cost: i64,
        boost: f32,
    ) -> Result<Option<LatLonPointDistanceScorer>> {
        let Some(point_values) = reader.get_point_values(&self.field)? else {
            // No data on this segment.
            return Ok(None);
        };
        let multi = match reader.get_sorted_numeric_doc_values(&self.field)? {
            Some(values) => values,
            None => Box::new(DocValues::empty_sorted_numeric()),
        };
        let values = Rc::new(RefCell::new(SelectedNumericDocValues::new(
            multi,
            ValueSelector::LatLon {
                origin_lat: self.origin_lat,
                origin_lon: self.origin_lon,
            },
        )));
        Ok(Some(LatLonPointDistanceScorer {
            max_doc: reader.max_doc(),
            lead_cost,
            boost,
            origin_lat: self.origin_lat,
            origin_lon: self.origin_lon,
            pivot_distance: self.pivot_distance,
            point_values,
            values: Rc::clone(&values),
            iter: DistanceIterator {
                doc: -1,
                source: IteratorSource::DocValues,
                values,
            },
            max_distance: GeoUtils::EARTH_MEAN_RADIUS_METERS * std::f64::consts::PI,
            set_min_competitive_score_counter: 0,
        }))
    }

    /// Returns the cost of this query over one leaf.
    ///
    /// Equivalent to the `ScorerSupplier.cost()` Java answers with
    /// `docValues.cost()`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the reader raises.
    pub fn cost(&self, reader: &dyn LeafReader) -> Result<i64> {
        Ok(match reader.get_sorted_numeric_doc_values(&self.field)? {
            Some(values) => values.cost(),
            None => DocValues::empty_sorted_numeric().cost(),
        })
    }

    /// Prints this query.
    ///
    /// Equivalent to `LatLonPointDistanceFeatureQuery.toString(String)`, which
    /// — as in Java — prints the *argument* rather than this query's own field.
    pub fn to_query_string(&self, field: &str) -> String {
        format!(
            "LatLonPointDistanceFeatureQuery(field={field},originLat={},originLon={},\
             pivotDistance={})",
            self.origin_lat, self.origin_lon, self.pivot_distance
        )
    }
}

impl PartialEq for LatLonPointDistanceFeatureQuery {
    /// Equivalent to the private
    /// `LatLonPointDistanceFeatureQuery.equalsTo`.
    fn eq(&self, other: &Self) -> bool {
        self.field == other.field
            && self.origin_lon == other.origin_lon
            && self.origin_lat == other.origin_lat
            && self.pivot_distance == other.pivot_distance
    }
}

/// The scorer of a [`LatLonPointDistanceFeatureQuery`].
///
/// Equivalent to the private inner class
/// `LatLonPointDistanceFeatureQuery.DistanceScorer`.
pub struct LatLonPointDistanceScorer {
    max_doc: i32,
    lead_cost: i64,
    boost: f32,
    origin_lat: f64,
    origin_lon: f64,
    pivot_distance: f64,
    point_values: Box<dyn PointValues>,
    values: Rc<RefCell<SelectedNumericDocValues>>,
    iter: DistanceIterator,
    max_distance: f64,
    set_min_competitive_score_counter: i32,
}

impl std::fmt::Debug for LatLonPointDistanceScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LatLonPointDistanceScorer")
            .field("origin_lat", &self.origin_lat)
            .field("origin_lon", &self.origin_lon)
            .field("pivot_distance", &self.pivot_distance)
            .field("max_distance", &self.max_distance)
            .finish_non_exhaustive()
    }
}

impl LatLonPointDistanceScorer {
    /// Equivalent to the private `DistanceScorer.score(double)`.
    fn score_distance(&self, distance: f64) -> f32 {
        (f64::from(self.boost) * (self.pivot_distance / (self.pivot_distance + distance))) as f32
    }

    /// Binary-searches the maximum distance that still scores at least
    /// `min_score`, down to a limit of one metre.
    ///
    /// Equivalent to the private
    /// `DistanceScorer.computeMaxDistance(float, double)`.
    fn compute_max_distance(&self, min_score: f32, previous_max_distance: f64) -> f64 {
        debug_assert!(self.score_distance(0.0) >= min_score);
        if self.score_distance(previous_max_distance) >= min_score {
            // `min_score` did not decrease enough to require an update.
            return previous_max_distance;
        }
        let mut min = 0.0f64;
        let mut max = previous_max_distance;
        // Invariant: score(min) >= min_score && score(max) < min_score.
        while max - min > 1.0 {
            let mid = (min + max) / 2.0;
            if self.score_distance(mid) >= min_score {
                min = mid;
            } else {
                max = mid;
            }
        }
        min
    }
}

impl Scorable for LatLonPointDistanceScorer {
    fn score(&mut self) -> Result<f32> {
        let doc = self.iter.doc;
        if !self.values.borrow_mut().advance_exact(doc)? {
            return Ok(0.0);
        }
        let encoded = self.values.borrow().long_value();
        Ok(self.score_distance(distance_from_encoded(
            self.origin_lat,
            self.origin_lon,
            encoded,
        )))
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        if min_score > self.boost {
            self.iter.source = IteratorSource::Empty;
            return Ok(());
        }

        self.set_min_competitive_score_counter += 1;
        // The calls are sampled, because recomputing the iterator is expensive.
        if self.set_min_competitive_score_counter > 256
            && (self.set_min_competitive_score_counter & 0x1F) != 0x1F
        {
            return Ok(());
        }

        let previous_max_distance = self.max_distance;
        self.max_distance = self.compute_max_distance(min_score, self.max_distance);
        if self.max_distance == previous_max_distance {
            // Nothing to update.
            return Ok(());
        }

        // A distance query would be ideal but is too expensive, so a box query
        // approximates it and performs better.
        let box_ =
            Rectangle::from_point_distance(self.origin_lat, self.origin_lon, self.max_distance)?;
        let visitor_bounds = LatLonRangeBounds {
            min_lat: GeoEncodingUtils::encode_latitude(box_.min_lat())?,
            max_lat: GeoEncodingUtils::encode_latitude(box_.max_lat())?,
            min_lon: GeoEncodingUtils::encode_longitude(box_.min_lon())?,
            max_lon: GeoEncodingUtils::encode_longitude(box_.max_lon())?,
            cross_date_line: box_.crosses_dateline(),
        };

        let mut result = DocIdSetBuilder::new(self.max_doc);
        let doc = self.iter.doc;
        let current_query_cost = self.lead_cost.min(self.iter.cost());
        let threshold = ((current_query_cost as u64) >> 3) as i64;
        {
            let mut visitor = LatLonRangeVisitor {
                result: &mut result,
                doc,
                bounds: visitor_bounds,
            };
            let mut tree = self.point_values.point_tree()?;
            if is_estimated_point_count_greater_than_or_equal_to(
                &mut visitor,
                tree.as_mut(),
                threshold,
            )? {
                // The new range is not selective enough to be worth
                // materialising.
                return Ok(());
            }
            drop(tree);
            self.point_values.intersect(&mut visitor)?;
        }
        self.iter.source = IteratorSource::Materialized(result.build()?.iterator()?);
        Ok(())
    }
}

impl Scorer for LatLonPointDistanceScorer {
    fn as_scorable(&mut self) -> &mut dyn Scorable {
        self
    }

    fn doc_id(&self) -> i32 {
        self.iter.doc
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.iter
    }

    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(self.boost)
    }
}

/// The encoded bounding box the competitive range approximates to.
#[derive(Clone, Copy, Debug)]
struct LatLonRangeBounds {
    min_lat: i32,
    max_lat: i32,
    min_lon: i32,
    max_lon: i32,
    cross_date_line: bool,
}

/// Collects the documents whose location falls in the competitive box.
///
/// Equivalent to the anonymous `IntersectVisitor` of
/// `LatLonPointDistanceFeatureQuery.DistanceScorer.setMinCompetitiveScore`.
struct LatLonRangeVisitor<'a> {
    result: &'a mut DocIdSetBuilder,
    doc: i32,
    bounds: LatLonRangeBounds,
}

impl IntersectVisitor for LatLonRangeVisitor<'_> {
    fn grow(&mut self, count: i32) {
        self.result.grow(count);
    }

    fn visit(&mut self, doc_id: i32) -> Result<()> {
        if doc_id <= self.doc {
            // Already visited or skipped.
            return Ok(());
        }
        self.result.grow(0).add(doc_id);
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        if doc_id <= self.doc {
            // Already visited or skipped.
            return Ok(());
        }
        let lat = NumericUtils::sortable_bytes_to_int(packed_value, 0);
        if lat > self.bounds.max_lat || lat < self.bounds.min_lat {
            // Latitude out of range.
            return Ok(());
        }
        let lon = NumericUtils::sortable_bytes_to_int(packed_value, LatLonPoint::BYTES);
        if self.bounds.cross_date_line {
            if lon < self.bounds.min_lon && lon > self.bounds.max_lon {
                // Longitude out of range.
                return Ok(());
            }
        } else if lon > self.bounds.max_lon || lon < self.bounds.min_lon {
            // Longitude out of range.
            return Ok(());
        }
        self.result.grow(0).add(doc_id);
        Ok(())
    }

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Relation {
        let lat_lower_bound = NumericUtils::sortable_bytes_to_int(min_packed_value, 0);
        let lat_upper_bound = NumericUtils::sortable_bytes_to_int(max_packed_value, 0);
        if lat_lower_bound > self.bounds.max_lat || lat_upper_bound < self.bounds.min_lat {
            return Relation::CellOutsideQuery;
        }
        let mut crosses =
            lat_lower_bound < self.bounds.min_lat || lat_upper_bound > self.bounds.max_lat;
        let lon_lower_bound =
            NumericUtils::sortable_bytes_to_int(min_packed_value, LatLonPoint::BYTES);
        let lon_upper_bound =
            NumericUtils::sortable_bytes_to_int(max_packed_value, LatLonPoint::BYTES);
        if self.bounds.cross_date_line {
            if lon_lower_bound > self.bounds.max_lon && lon_upper_bound < self.bounds.min_lon {
                return Relation::CellOutsideQuery;
            }
            crosses |=
                lon_lower_bound < self.bounds.max_lon || lon_upper_bound > self.bounds.min_lon;
        } else {
            if lon_lower_bound > self.bounds.max_lon || lon_upper_bound < self.bounds.min_lon {
                return Relation::CellOutsideQuery;
            }
            crosses |=
                lon_lower_bound < self.bounds.min_lon || lon_upper_bound > self.bounds.max_lon;
        }
        if crosses {
            Relation::CellCrossesQuery
        } else {
            Relation::CellInsideQuery
        }
    }
}
