//! The queries over indexed points and over location doc values, ported from
//! `org.apache.lucene.document`.
//!
//! [`LatLonPointQuery`] and [`XYPointInGeometryQuery`] read the BKD tree a
//! [`LatLonPoint`] or an
//! [`XYPointField`] writes; the three
//! doc-values queries scan the packed longs a
//! [`LatLonDocValuesField`](crate::document::geo_fields::LatLonDocValuesField) or an
//! [`XYDocValuesField`](crate::document::geo_fields::XYDocValuesField) writes instead.
//!
//! # Divergence from Lucene 10.5.0: the query hierarchy
//!
//! As in [`spatial_query`](crate::document::spatial_query), these types stop
//! one step short of `Query`/`Weight`/`Scorer` and expose the
//! [`DocIdSetIterator`] a `ConstantScoreScorer` would have wrapped. The three
//! doc-values queries build Java's `TwoPhaseIterator` over the sorted-numeric
//! values, and their scorers expose it; their `matching_docs` is that
//! iterator's `TwoPhaseIterator.asDocIdSetIterator` view.

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use crate::document::geo_fields::{LatLonPoint, XYPointField};
use crate::document::shape_field::QueryRelation;
use crate::document::spatial_query::{spatial_create_weight, SpatialMatcher};
use crate::document::spatial_query::{
    LatLonGeometryValue, SpatialQuery, SpatialVisitor, XYGeometryValue,
};
use crate::error::{LuceneError, Result};
use crate::geo::component2d::WithinRelation;
use crate::geo::encoding::{
    Component2DPredicate, DistancePredicate, GeoEncodingUtils, GeoUtils, XYEncodingUtils,
};
use crate::geo::Component2D;
use crate::index::point_values::{IntersectVisitor, PointValues, Relation};
use crate::index::{LeafReader, SortedNumericDocValues};
use crate::search::{
    ConstantScoreScorer, DefaultScorerSupplier, DocIdSetIterator, Scorer, ScorerSupplier,
    TwoPhaseIterator, TwoPhaseIteratorAsDocIdSetIterator, NO_MORE_DOCS,
};
use crate::search::{IndexSearcher, Query, QueryVisitor, ScoreMode, Weight};
use crate::util::doc_id_set::DocIdSetBuilder;
use crate::util::{IntsRef, NumericUtils};

/// How many bytes one encoded coordinate occupies.
const COORDINATE_BYTES: usize = 4;

// -----------------------------------------------------------------------------
// LatLonPointQuery
// -----------------------------------------------------------------------------

/// The [`SpatialVisitor`] of a geographic point query.
///
/// Equivalent to the anonymous `SpatialVisitor` that
/// `LatLonPointQuery.getSpatialVisitor()` returns.
///
#[derive(Clone)]
pub struct LatLonPointSpatialVisitor {
    component2d: Arc<dyn Component2D>,
    /// The grid of per-sub-box relations, which answers `contains` for most
    /// encoded points without touching the geometry.
    ///
    /// Equivalent to the `GeoEncodingUtils.Component2DPredicate` Java
    /// precomputes in `LatLonPointQuery.getSpatialVisitor()`.
    predicate: Component2DPredicate,
    /// The bounding box over all geometries, which cheaply prunes a cell before
    /// the geometry is consulted.
    min_lat: i32,
    max_lat: i32,
    min_lon: i32,
    max_lon: i32,
}

impl std::fmt::Debug for LatLonPointSpatialVisitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LatLonPointSpatialVisitor")
            .field("min_lat", &self.min_lat)
            .field("max_lat", &self.max_lat)
            .field("min_lon", &self.min_lon)
            .field("max_lon", &self.max_lon)
            .finish_non_exhaustive()
    }
}

impl LatLonPointSpatialVisitor {
    /// Creates the visitor over `component2d`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the coordinate encoder raises.
    pub fn new(component2d: Arc<dyn Component2D>) -> Result<Self> {
        Ok(Self {
            min_lat: GeoEncodingUtils::encode_latitude(component2d.get_min_y())?,
            max_lat: GeoEncodingUtils::encode_latitude(component2d.get_max_y())?,
            min_lon: GeoEncodingUtils::encode_longitude(component2d.get_min_x())?,
            max_lon: GeoEncodingUtils::encode_longitude(component2d.get_max_x())?,
            predicate: GeoEncodingUtils::create_component_predicate(Arc::clone(&component2d))?,
            component2d,
        })
    }

    /// Equivalent to `Component2DPredicate.test(int, int)`, the grid lookup
    /// Java precomputes for this visitor.
    fn test(&self, lat: i32, lon: i32) -> bool {
        self.predicate.test(lat, lon)
    }
}

impl SpatialVisitor for LatLonPointSpatialVisitor {
    fn relate(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Result<Relation> {
        let lat_lower_bound = NumericUtils::sortable_bytes_to_int(min_packed_value, 0);
        let lat_upper_bound = NumericUtils::sortable_bytes_to_int(max_packed_value, 0);
        if lat_lower_bound > self.max_lat || lat_upper_bound < self.min_lat {
            // Outside of the global bounding box range.
            return Ok(Relation::CellOutsideQuery);
        }

        let lon_lower_bound =
            NumericUtils::sortable_bytes_to_int(min_packed_value, LatLonPoint::BYTES);
        let lon_upper_bound =
            NumericUtils::sortable_bytes_to_int(max_packed_value, LatLonPoint::BYTES);
        if lon_lower_bound > self.max_lon || lon_upper_bound < self.min_lon {
            // Outside of the global bounding box range.
            return Ok(Relation::CellOutsideQuery);
        }

        let cell_min_lat = GeoEncodingUtils::decode_latitude(lat_lower_bound);
        let cell_min_lon = GeoEncodingUtils::decode_longitude(lon_lower_bound);
        let cell_max_lat = GeoEncodingUtils::decode_latitude(lat_upper_bound);
        let cell_max_lon = GeoEncodingUtils::decode_longitude(lon_upper_bound);

        Ok(self
            .component2d
            .relate(cell_min_lon, cell_max_lon, cell_min_lat, cell_max_lat))
    }

    fn intersects(&self, packed_value: &[u8]) -> Result<bool> {
        Ok(self.test(
            NumericUtils::sortable_bytes_to_int(packed_value, 0),
            NumericUtils::sortable_bytes_to_int(packed_value, COORDINATE_BYTES),
        ))
    }

    fn within(&self, packed_value: &[u8]) -> Result<bool> {
        Ok(self.test(
            NumericUtils::sortable_bytes_to_int(packed_value, 0),
            NumericUtils::sortable_bytes_to_int(packed_value, COORDINATE_BYTES),
        ))
    }

    fn contains(&self, packed_value: &[u8]) -> Result<WithinRelation> {
        Ok(self.component2d.within_point(
            GeoEncodingUtils::decode_longitude(NumericUtils::sortable_bytes_to_int(
                packed_value,
                COORDINATE_BYTES,
            )),
            GeoEncodingUtils::decode_latitude(NumericUtils::sortable_bytes_to_int(packed_value, 0)),
        ))
    }
}

/// Finds every previously indexed geographic point that complies with the given
/// [`QueryRelation`] against an array of geometries.
///
/// Equivalent to `org.apache.lucene.document.LatLonPointQuery`. The field must
/// be indexed with one or more
/// [`LatLonPoint`] per document.
#[derive(Clone, Debug)]
pub struct LatLonPointQuery {
    query: SpatialQuery,
    geometries: Vec<LatLonGeometryValue>,
    visitor: LatLonPointSpatialVisitor,
}

impl LatLonPointQuery {
    /// Builds the [`Component2D`] this query relates its candidates against.
    ///
    /// Equivalent to `LatLonPointQuery.createComponent2D(LatLonGeometry[])`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) when the geometry list is
    /// empty.
    pub fn create_component_2d(
        geometries: &[LatLonGeometryValue],
    ) -> Result<std::sync::Arc<dyn Component2D>> {
        SpatialQuery::create_component_2d(geometries)
    }

    /// Creates the query.
    ///
    /// Equivalent to
    /// `LatLonPointQuery(String, QueryRelation, LatLonGeometry...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) for a `WITHIN` query over a
    /// line geometry, for a `CONTAINS` query over a non-point geometry, and for
    /// an empty geometry list.
    pub fn new(
        field: impl Into<String>,
        query_relation: QueryRelation,
        geometries: Vec<LatLonGeometryValue>,
    ) -> Result<Self> {
        validate_geometry(query_relation, &geometries)?;
        let component2d = LatLonGeometryValue::create(&geometries)?;
        Ok(Self {
            query: SpatialQuery::new(field, query_relation, Arc::clone(&component2d)),
            geometries,
            visitor: LatLonPointSpatialVisitor::new(component2d)?,
        })
    }

    /// Returns the shared query state.
    pub fn spatial_query(&self) -> &SpatialQuery {
        &self.query
    }

    /// Returns the geometries the query relates against.
    pub fn geometries(&self) -> &[LatLonGeometryValue] {
        &self.geometries
    }

    /// Returns the visitor that drives the BKD walk.
    ///
    /// Equivalent to `LatLonPointQuery.getSpatialVisitor()`.
    pub fn get_spatial_visitor(&self) -> &LatLonPointSpatialVisitor {
        &self.visitor
    }

    /// Returns the documents of `reader` this query matches.
    ///
    /// See [`SpatialQuery::matching_docs`].
    ///
    /// # Errors
    ///
    /// Propagates whatever the point tree raises.
    pub fn matching_docs(
        &self,
        reader: &dyn LeafReader,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        self.query.matching_docs(reader, &self.visitor)
    }

    /// Prints this query, omitting `field` when it is the default one.
    ///
    /// Equivalent to `LatLonPointQuery.toString(String)`.
    pub fn to_query_string(&self, field: &str) -> String {
        self.query.to_query_string(
            "LatLonPointQuery",
            field,
            &self
                .geometries
                .iter()
                .map(|g| format!("{g:?}"))
                .collect::<Vec<_>>(),
        )
    }
}

/// Rejects the geometry/relation combinations `LatLonPointQuery` does not
/// support.
///
/// Equivalent to the private static `LatLonPointQuery.validateGeometry`.
fn validate_geometry(
    query_relation: QueryRelation,
    geometries: &[LatLonGeometryValue],
) -> Result<()> {
    if query_relation == QueryRelation::Within {
        for geometry in geometries {
            if matches!(geometry, LatLonGeometryValue::Line(_)) {
                return Err(LuceneError::illegal_argument(
                    "LatLonPointQuery does not support WITHIN queries with line geometries"
                        .to_string(),
                ));
            }
        }
    }
    if query_relation == QueryRelation::Contains {
        for geometry in geometries {
            if !matches!(geometry, LatLonGeometryValue::Point(_)) {
                return Err(LuceneError::illegal_argument(
                    "LatLonPointQuery does not support CONTAINS queries with non-points geometries"
                        .to_string(),
                ));
            }
        }
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// Doc-values scanning
// -----------------------------------------------------------------------------

/// Decides whether the values of one document match.
///
/// Equivalent to the `matches()` of the `TwoPhaseIterator` each doc-values
/// query builds.
trait SortedNumericMatcher: std::fmt::Debug {
    /// Returns whether the current document matches, consuming its values.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the doc values raises.
    fn matches(&self, values: &mut dyn SortedNumericDocValues) -> Result<bool>;
}

/// The anonymous `TwoPhaseIterator` each doc-values query builds over the
/// field's sorted-numeric values: the values are the approximation, the
/// matcher is `matches()` and `match_cost` is `matchCost()`
/// (`LatLonDocValuesBoxQuery.java:121-155`, `LatLonDocValuesQuery.java:173-271`,
/// `XYDocValuesPointInGeometryQuery.java:112-132`).
struct SortedNumericTwoPhaseIterator {
    values: Box<dyn SortedNumericDocValues>,
    matcher: Box<dyn SortedNumericMatcher>,
    match_cost: f32,
}

impl TwoPhaseIterator for SortedNumericTwoPhaseIterator {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        self.values.as_mut()
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        self.values.as_ref()
    }

    fn matches(&mut self) -> Result<bool> {
        self.matcher.matches(self.values.as_mut())
    }

    fn match_cost(&self) -> f32 {
        self.match_cost
    }
}

/// `TwoPhaseIterator.asDocIdSetIterator(iterator)`, the documents a two-phase
/// iterator confirms.
fn as_doc_id_set_iterator(two_phase: SortedNumericTwoPhaseIterator) -> Box<dyn DocIdSetIterator> {
    Box::new(TwoPhaseIteratorAsDocIdSetIterator::new(Box::new(two_phase)))
}

/// The scorer supplier of the `ConstantScoreWeight` each doc-values query
/// builds: `new DefaultScorerSupplier(new ConstantScoreScorer(boost,
/// scoreMode, iterator))` (`LatLonDocValuesBoxQuery.java:156-157`,
/// `LatLonDocValuesQuery.java:161-162`,
/// `XYDocValuesPointInGeometryQuery.java:133-134`).
fn two_phase_scorer_supplier(
    two_phase: SortedNumericTwoPhaseIterator,
    score_mode: ScoreMode,
    boost: f32,
) -> Box<dyn ScorerSupplier> {
    let scorer = ConstantScoreScorer::from_two_phase(boost, score_mode, Box::new(two_phase));
    Box::new(DefaultScorerSupplier::new(Box::new(scorer)))
}

/// Splits one packed location value into its two encoded coordinates.
///
/// Equivalent to the `(int) (value >>> 32)` / `(int) (value & 0xFFFFFFFF)` pair
/// every location doc-values query performs.
fn split_packed(value: i64) -> (i32, i32) {
    (((value as u64) >> 32) as i32, (value & 0xFFFF_FFFF) as i32)
}

// -----------------------------------------------------------------------------
// LatLonDocValuesQuery
// -----------------------------------------------------------------------------

/// The per-document test of a [`LatLonDocValuesQuery`].
#[derive(Debug)]
struct LatLonDocValuesMatcher {
    query_relation: QueryRelation,
    component2d: Arc<dyn Component2D>,
    /// One component per geometry, which only the `CONTAINS` relation needs.
    contains_components: Vec<Arc<dyn Component2D>>,
}

impl LatLonDocValuesMatcher {
    /// The `Component2DPredicate.test(int, int)` Java precomputes; see the
    /// divergence note on [`LatLonPointSpatialVisitor`].
    fn test(&self, lat: i32, lon: i32) -> bool {
        self.component2d.contains(
            GeoEncodingUtils::decode_longitude(lon),
            GeoEncodingUtils::decode_latitude(lat),
        )
    }

    /// Whether no value of the document falls inside the query geometry.
    ///
    /// Equivalent to the `matches()` of the two-phase iterator the private
    /// `LatLonDocValuesQuery.disjoint(SortedNumericDocValues, Component2DPredicate)`
    /// returns.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the doc values raises.
    fn disjoint(&self, values: &mut dyn SortedNumericDocValues, count: i32) -> Result<bool> {
        for _ in 0..count {
            let (lat, lon) = split_packed(values.next_value()?);
            if self.test(lat, lon) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

impl SortedNumericMatcher for LatLonDocValuesMatcher {
    fn matches(&self, values: &mut dyn SortedNumericDocValues) -> Result<bool> {
        let count = values.doc_value_count()?;
        match self.query_relation {
            // `LatLonDocValuesQuery.intersects(...)`.
            QueryRelation::Intersects => {
                for _ in 0..count {
                    let (lat, lon) = split_packed(values.next_value()?);
                    if self.test(lat, lon) {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            // `LatLonDocValuesQuery.within(...)`.
            QueryRelation::Within => {
                for _ in 0..count {
                    let (lat, lon) = split_packed(values.next_value()?);
                    if !self.test(lat, lon) {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            // `LatLonDocValuesQuery.disjoint(...)`.
            QueryRelation::Disjoint => self.disjoint(values, count),
            // `LatLonDocValuesQuery.contains(...)`.
            QueryRelation::Contains => {
                let mut answer = WithinRelation::Disjoint;
                for _ in 0..count {
                    let (lat, lon) = split_packed(values.next_value()?);
                    let lat = GeoEncodingUtils::decode_latitude(lat);
                    let lon = GeoEncodingUtils::decode_longitude(lon);
                    for component2d in &self.contains_components {
                        let relation = component2d.within_point(lon, lat);
                        if relation == WithinRelation::NotWithin {
                            return Ok(false);
                        } else if relation != WithinRelation::Disjoint {
                            answer = relation;
                        }
                    }
                }
                Ok(answer == WithinRelation::Candidate)
            }
        }
    }
}

/// Finds every previously indexed geographic point that complies with the given
/// [`QueryRelation`] against an array of geometries, by scanning doc values.
///
/// Equivalent to `org.apache.lucene.document.LatLonDocValuesQuery`. The field
/// must be indexed with a
/// [`LatLonDocValuesField`](crate::document::geo_fields::LatLonDocValuesField) per
/// document.
#[derive(Clone, Debug)]
pub struct LatLonDocValuesQuery {
    field: String,
    geometries: Vec<LatLonGeometryValue>,
    query_relation: QueryRelation,
    component2d: Arc<dyn Component2D>,
}

impl LatLonDocValuesQuery {
    /// Creates the query.
    ///
    /// Equivalent to
    /// `LatLonDocValuesQuery(String, QueryRelation, LatLonGeometry...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) for a `WITHIN` query over a
    /// line geometry, for a `CONTAINS` query over a non-point geometry, and for
    /// an empty geometry list.
    pub fn new(
        field: impl Into<String>,
        query_relation: QueryRelation,
        geometries: Vec<LatLonGeometryValue>,
    ) -> Result<Self> {
        if query_relation == QueryRelation::Within {
            for geometry in &geometries {
                if matches!(geometry, LatLonGeometryValue::Line(_)) {
                    return Err(LuceneError::illegal_argument(
                        "LatLonDocValuesPointQuery does not support WITHIN queries with line \
                         geometries"
                            .to_string(),
                    ));
                }
            }
        }
        if query_relation == QueryRelation::Contains {
            for geometry in &geometries {
                if !matches!(geometry, LatLonGeometryValue::Point(_)) {
                    return Err(LuceneError::illegal_argument(
                        "LatLonDocValuesPointQuery does not support CONTAINS queries with \
                         non-points geometries"
                            .to_string(),
                    ));
                }
            }
        }
        let component2d = LatLonGeometryValue::create(&geometries)?;
        Ok(Self {
            field: field.into(),
            geometries,
            query_relation,
            component2d,
        })
    }

    /// Returns the field the query reads.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns the relation the query applies.
    pub fn query_relation(&self) -> QueryRelation {
        self.query_relation
    }

    /// Returns the geometries the query relates against.
    pub fn geometries(&self) -> &[LatLonGeometryValue] {
        &self.geometries
    }

    /// Returns the cost of one match.
    ///
    /// Equivalent to the `matchCost()` of every `TwoPhaseIterator`
    /// `LatLonDocValuesQuery` builds.
    pub fn match_cost(&self) -> f32 {
        1000.0
    }

    /// Builds the two-phase iterator over `values` for the query relation.
    ///
    /// Equivalent to the `switch (queryRelation)` of the `scorerSupplier`
    /// `LatLonDocValuesQuery.createWeight` builds
    /// (`LatLonDocValuesQuery.java:143-160`), over the private `intersects`,
    /// `within`, `disjoint` and `contains` factories.
    ///
    /// # Errors
    ///
    /// Propagates whatever building the `CONTAINS` components raises.
    fn two_phase_iterator(
        &self,
        values: Box<dyn SortedNumericDocValues>,
    ) -> Result<SortedNumericTwoPhaseIterator> {
        let mut contains_components = Vec::new();
        if self.query_relation == QueryRelation::Contains {
            for geometry in &self.geometries {
                contains_components.push(geometry.to_component2d()?);
            }
        }
        Ok(SortedNumericTwoPhaseIterator {
            values,
            matcher: Box::new(LatLonDocValuesMatcher {
                query_relation: self.query_relation,
                component2d: Arc::clone(&self.component2d),
                contains_components,
            }),
            match_cost: self.match_cost(),
        })
    }

    /// Returns the documents of `reader` this query matches: the
    /// `TwoPhaseIterator.asDocIdSetIterator` view of the two-phase iterator
    /// the query's scorer exposes.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the doc values raises.
    pub fn matching_docs(
        &self,
        reader: &dyn LeafReader,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        let Some(values) = reader.get_sorted_numeric_doc_values(&self.field)? else {
            return Ok(None);
        };
        Ok(Some(as_doc_id_set_iterator(
            self.two_phase_iterator(values)?,
        )))
    }

    /// Equivalent to `scorerSupplier(LeafReaderContext)` of the
    /// `ConstantScoreWeight` `LatLonDocValuesQuery.createWeight` builds
    /// (`LatLonDocValuesQuery.java:137-163`): `None` when the leaf has no
    /// values for the field.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the doc values raises.
    fn scorer_supplier(
        &self,
        reader: &dyn LeafReader,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let Some(values) = reader.get_sorted_numeric_doc_values(&self.field)? else {
            return Ok(None);
        };
        Ok(Some(two_phase_scorer_supplier(
            self.two_phase_iterator(values)?,
            score_mode,
            boost,
        )))
    }

    /// Prints this query, omitting `field` when it is the default one.
    ///
    /// Equivalent to `LatLonDocValuesQuery.toString(String)`.
    pub fn to_query_string(&self, field: &str) -> String {
        let mut sb = String::new();
        if self.field != field {
            sb.push_str(&self.field);
            sb.push(':');
        }
        sb.push_str(&format!("{:?}", self.query_relation));
        sb.push(':');
        sb.push_str("geometries(");
        sb.push_str(&format!("{:?}", self.geometries));
        sb.push(')');
        sb
    }
}

impl PartialEq for LatLonDocValuesQuery {
    /// Equivalent to `LatLonDocValuesQuery.equals(Object)`.
    fn eq(&self, other: &Self) -> bool {
        self.field == other.field
            && self.query_relation == other.query_relation
            && self.geometries == other.geometries
    }
}

// -----------------------------------------------------------------------------
// LatLonDocValuesBoxQuery
// -----------------------------------------------------------------------------

/// The per-document test of a [`LatLonDocValuesBoxQuery`].
#[derive(Clone, Copy, Debug)]
struct LatLonBoxMatcher {
    min_latitude: i32,
    max_latitude: i32,
    min_longitude: i32,
    max_longitude: i32,
    crosses_dateline: bool,
}

impl SortedNumericMatcher for LatLonBoxMatcher {
    fn matches(&self, values: &mut dyn SortedNumericDocValues) -> Result<bool> {
        let count = values.doc_value_count()?;
        for _ in 0..count {
            let (lat, lon) = split_packed(values.next_value()?);
            if lat < self.min_latitude || lat > self.max_latitude {
                // Not within the latitude range.
                continue;
            }
            if self.crosses_dateline {
                if lon > self.max_longitude && lon < self.min_longitude {
                    // Not within the longitude range.
                    continue;
                }
            } else if lon < self.min_longitude || lon > self.max_longitude {
                // Not within the longitude range.
                continue;
            }
            return Ok(true);
        }
        Ok(false)
    }
}

/// Finds every previously indexed geographic point inside a bounding box, by
/// scanning doc values.
///
/// Equivalent to `org.apache.lucene.document.LatLonDocValuesBoxQuery`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LatLonDocValuesBoxQuery {
    field: String,
    matcher_min_latitude: i32,
    matcher_max_latitude: i32,
    matcher_min_longitude: i32,
    matcher_max_longitude: i32,
    crosses_dateline: bool,
}

impl LatLonDocValuesBoxQuery {
    /// Creates the query.
    ///
    /// Equivalent to
    /// `LatLonDocValuesBoxQuery(String, double, double, double, double)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) for a coordinate outside its
    /// valid range.
    pub fn new(
        field: impl Into<String>,
        min_latitude: f64,
        max_latitude: f64,
        min_longitude: f64,
        max_longitude: f64,
    ) -> Result<Self> {
        GeoUtils::check_latitude(min_latitude)?;
        GeoUtils::check_latitude(max_latitude)?;
        GeoUtils::check_longitude(min_longitude)?;
        GeoUtils::check_longitude(max_longitude)?;
        Ok(Self {
            field: field.into(),
            // Compute this before rounding.
            crosses_dateline: min_longitude > max_longitude,
            matcher_min_latitude: GeoEncodingUtils::encode_latitude_ceil(min_latitude)?,
            matcher_max_latitude: GeoEncodingUtils::encode_latitude(max_latitude)?,
            matcher_min_longitude: GeoEncodingUtils::encode_longitude_ceil(min_longitude)?,
            matcher_max_longitude: GeoEncodingUtils::encode_longitude(max_longitude)?,
        })
    }

    /// Returns the field the query reads.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns whether the box crosses the dateline.
    pub fn crosses_dateline(&self) -> bool {
        self.crosses_dateline
    }

    /// Returns the cost of one match, in comparisons.
    ///
    /// Equivalent to the `matchCost()` of the `TwoPhaseIterator`
    /// `LatLonDocValuesBoxQuery` builds: five comparisons.
    pub fn match_cost(&self) -> f32 {
        5.0
    }

    /// Builds the anonymous two-phase iterator of the `scorerSupplier`
    /// `LatLonDocValuesBoxQuery.createWeight` builds
    /// (`LatLonDocValuesBoxQuery.java:121-155`).
    fn two_phase_iterator(
        &self,
        values: Box<dyn SortedNumericDocValues>,
    ) -> SortedNumericTwoPhaseIterator {
        SortedNumericTwoPhaseIterator {
            values,
            matcher: Box::new(LatLonBoxMatcher {
                min_latitude: self.matcher_min_latitude,
                max_latitude: self.matcher_max_latitude,
                min_longitude: self.matcher_min_longitude,
                max_longitude: self.matcher_max_longitude,
                crosses_dateline: self.crosses_dateline,
            }),
            match_cost: self.match_cost(),
        }
    }

    /// Returns the documents of `reader` this query matches: the
    /// `TwoPhaseIterator.asDocIdSetIterator` view of the two-phase iterator
    /// the query's scorer exposes.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the doc values raises.
    pub fn matching_docs(
        &self,
        reader: &dyn LeafReader,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        let Some(values) = reader.get_sorted_numeric_doc_values(&self.field)? else {
            return Ok(None);
        };
        Ok(Some(as_doc_id_set_iterator(
            self.two_phase_iterator(values),
        )))
    }

    /// Equivalent to `scorerSupplier(LeafReaderContext)` of the
    /// `ConstantScoreWeight` `LatLonDocValuesBoxQuery.createWeight` builds
    /// (`LatLonDocValuesBoxQuery.java:114-158`): `None` when the leaf has no
    /// values for the field.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the doc values raises.
    fn scorer_supplier(
        &self,
        reader: &dyn LeafReader,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let Some(values) = reader.get_sorted_numeric_doc_values(&self.field)? else {
            return Ok(None);
        };
        Ok(Some(two_phase_scorer_supplier(
            self.two_phase_iterator(values),
            score_mode,
            boost,
        )))
    }

    /// Prints this query, omitting `field` when it is the default one.
    ///
    /// Equivalent to `LatLonDocValuesBoxQuery.toString(String)`.
    pub fn to_query_string(&self, field: &str) -> String {
        let mut sb = String::new();
        if self.field != field {
            sb.push_str(&self.field);
            sb.push(':');
        }
        sb.push_str(&format!(
            "box(minLat={}, maxLat={}, minLon={}, maxLon={})",
            GeoEncodingUtils::decode_latitude(self.matcher_min_latitude),
            GeoEncodingUtils::decode_latitude(self.matcher_max_latitude),
            GeoEncodingUtils::decode_longitude(self.matcher_min_longitude),
            GeoEncodingUtils::decode_longitude(self.matcher_max_longitude),
        ));
        sb
    }
}

// -----------------------------------------------------------------------------
// XYDocValuesPointInGeometryQuery
// -----------------------------------------------------------------------------

/// The per-document test of an [`XYDocValuesPointInGeometryQuery`].
#[derive(Debug)]
struct XYDocValuesMatcher {
    component2d: Arc<dyn Component2D>,
}

impl SortedNumericMatcher for XYDocValuesMatcher {
    fn matches(&self, values: &mut dyn SortedNumericDocValues) -> Result<bool> {
        let count = values.doc_value_count()?;
        for _ in 0..count {
            let (x, y) = split_packed(values.next_value()?);
            let x = f64::from(XYEncodingUtils::decode(x));
            let y = f64::from(XYEncodingUtils::decode(y));
            if self.component2d.contains(x, y) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Finds every previously indexed cartesian point inside the provided
/// geometries, by scanning doc values.
///
/// Equivalent to `org.apache.lucene.document.XYDocValuesPointInGeometryQuery`.
/// The field must be indexed with an
/// [`XYDocValuesField`](crate::document::geo_fields::XYDocValuesField) per document.
#[derive(Clone, Debug)]
pub struct XYDocValuesPointInGeometryQuery {
    field: String,
    geometries: Vec<XYGeometryValue>,
    component2d: Arc<dyn Component2D>,
}

impl XYDocValuesPointInGeometryQuery {
    /// Creates the query.
    ///
    /// Equivalent to
    /// `XYDocValuesPointInGeometryQuery(String, XYGeometry...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) for an empty geometry list.
    pub fn new(field: impl Into<String>, geometries: Vec<XYGeometryValue>) -> Result<Self> {
        if geometries.is_empty() {
            return Err(LuceneError::illegal_argument(
                "geometries must not be empty".to_string(),
            ));
        }
        let component2d = XYGeometryValue::create(&geometries)?;
        Ok(Self {
            field: field.into(),
            geometries,
            component2d,
        })
    }

    /// Returns the field the query reads.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns the geometries the query relates against.
    pub fn geometries(&self) -> &[XYGeometryValue] {
        &self.geometries
    }

    /// Returns the cost of one match.
    ///
    /// Equivalent to the `matchCost()` of the `TwoPhaseIterator` this query
    /// builds.
    pub fn match_cost(&self) -> f32 {
        1000.0
    }

    /// Builds the anonymous two-phase iterator of the `scorerSupplier`
    /// `XYDocValuesPointInGeometryQuery.createWeight` builds
    /// (`XYDocValuesPointInGeometryQuery.java:112-132`).
    fn two_phase_iterator(
        &self,
        values: Box<dyn SortedNumericDocValues>,
    ) -> SortedNumericTwoPhaseIterator {
        SortedNumericTwoPhaseIterator {
            values,
            matcher: Box::new(XYDocValuesMatcher {
                component2d: Arc::clone(&self.component2d),
            }),
            match_cost: self.match_cost(),
        }
    }

    /// Returns the documents of `reader` this query matches: the
    /// `TwoPhaseIterator.asDocIdSetIterator` view of the two-phase iterator
    /// the query's scorer exposes.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the doc values raises.
    pub fn matching_docs(
        &self,
        reader: &dyn LeafReader,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        let Some(values) = reader.get_sorted_numeric_doc_values(&self.field)? else {
            return Ok(None);
        };
        Ok(Some(as_doc_id_set_iterator(
            self.two_phase_iterator(values),
        )))
    }

    /// Equivalent to `scorerSupplier(LeafReaderContext)` of the
    /// `ConstantScoreWeight` `XYDocValuesPointInGeometryQuery.createWeight`
    /// builds (`XYDocValuesPointInGeometryQuery.java:105-135`): `None` when
    /// the leaf has no values for the field.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the doc values raises.
    fn scorer_supplier(
        &self,
        reader: &dyn LeafReader,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let Some(values) = reader.get_sorted_numeric_doc_values(&self.field)? else {
            return Ok(None);
        };
        Ok(Some(two_phase_scorer_supplier(
            self.two_phase_iterator(values),
            score_mode,
            boost,
        )))
    }

    /// Prints this query, omitting `field` when it is the default one.
    ///
    /// Equivalent to `XYDocValuesPointInGeometryQuery.toString(String)`.
    pub fn to_query_string(&self, field: &str) -> String {
        let mut sb = String::new();
        if self.field != field {
            sb.push_str(&self.field);
            sb.push(':');
        }
        sb.push_str("geometries(");
        sb.push_str(&format!("{:?}", self.geometries));
        sb.push(')');
        sb
    }
}

impl PartialEq for XYDocValuesPointInGeometryQuery {
    /// Equivalent to `XYDocValuesPointInGeometryQuery.equals(Object)`.
    fn eq(&self, other: &Self) -> bool {
        self.field == other.field && self.geometries == other.geometries
    }
}

// -----------------------------------------------------------------------------
// XYPointInGeometryQuery
// -----------------------------------------------------------------------------

/// Collects the indexed cartesian points that fall inside the query geometry.
///
/// Equivalent to the visitor
/// `XYPointInGeometryQuery.getIntersectVisitor(DocIdSetBuilder, Component2D)`
/// returns. The same `BulkAdder` divergence as
/// [`spatial_query`](crate::document::spatial_query) applies: `grow(0)`
/// re-obtains an adder without reserving anything.
struct XYPointIntersectVisitor<'a> {
    result: &'a mut DocIdSetBuilder,
    tree: &'a dyn Component2D,
}

impl XYPointIntersectVisitor<'_> {
    fn contains_packed(&self, packed_value: &[u8]) -> bool {
        let x = f64::from(XYEncodingUtils::decode_bytes(packed_value, 0));
        let y = f64::from(XYEncodingUtils::decode_bytes(
            packed_value,
            COORDINATE_BYTES,
        ));
        self.tree.contains(x, y)
    }
}

impl IntersectVisitor for XYPointIntersectVisitor<'_> {
    /// `XYPointInGeometryQuery$1` declares no `toString()`, so `java.lang.Object.toString()` applies
    /// ([`java_object_to_string_stand_in`](crate::index::index_reader::java_object_to_string_stand_in)).
    fn java_to_string(&self) -> String {
        crate::index::index_reader::java_object_to_string_stand_in(
            "org.apache.lucene.document.XYPointInGeometryQuery$1",
        )
    }
    fn grow(&mut self, count: i32) -> Result<()> {
        self.result.grow(count);
        Ok(())
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
        if self.contains_packed(packed_value) {
            self.visit(doc_id)?;
        }
        Ok(())
    }

    fn visit_iterator_with_value(
        &mut self,
        iterator: &mut dyn DocIdSetIterator,
        packed_value: &[u8],
    ) -> Result<()> {
        if self.contains_packed(packed_value) {
            self.result.grow(0).add_iterator(iterator)?;
        }
        Ok(())
    }

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Result<Relation> {
        let cell_min_x = f64::from(XYEncodingUtils::decode_bytes(min_packed_value, 0));
        let cell_min_y = f64::from(XYEncodingUtils::decode_bytes(
            min_packed_value,
            COORDINATE_BYTES,
        ));
        let cell_max_x = f64::from(XYEncodingUtils::decode_bytes(max_packed_value, 0));
        let cell_max_y = f64::from(XYEncodingUtils::decode_bytes(
            max_packed_value,
            COORDINATE_BYTES,
        ));
        Ok(self
            .tree
            .relate(cell_min_x, cell_max_x, cell_min_y, cell_max_y))
    }
}

/// Finds every previously indexed point that falls within the specified
/// cartesian geometries.
///
/// Equivalent to `org.apache.lucene.document.XYPointInGeometryQuery`. The field
/// must be indexed with an [`XYPointField`] per
/// document.
#[derive(Clone, Debug)]
pub struct XYPointInGeometryQuery {
    field: String,
    xy_geometries: Vec<XYGeometryValue>,
    tree: Arc<dyn Component2D>,
}

impl XYPointInGeometryQuery {
    /// Creates the query.
    ///
    /// Equivalent to `XYPointInGeometryQuery(String, XYGeometry...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) for an empty geometry list.
    pub fn new(field: impl Into<String>, xy_geometries: Vec<XYGeometryValue>) -> Result<Self> {
        if xy_geometries.is_empty() {
            return Err(LuceneError::illegal_argument(
                "geometries must not be empty".to_string(),
            ));
        }
        let tree = XYGeometryValue::create(&xy_geometries)?;
        Ok(Self {
            field: field.into(),
            xy_geometries,
            tree,
        })
    }

    /// Returns the query field.
    ///
    /// Equivalent to `XYPointInGeometryQuery.getField()`.
    pub fn get_field(&self) -> &str {
        &self.field
    }

    /// Returns a copy of the internal geometry list.
    ///
    /// Equivalent to `XYPointInGeometryQuery.getGeometries()`.
    pub fn get_geometries(&self) -> Vec<XYGeometryValue> {
        self.xy_geometries.clone()
    }

    /// Returns the documents of `reader` this query matches: the iterator the
    /// scorer of [`scorer_supplier`](Self::scorer_supplier)'s `get` wraps.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) when the field was indexed with
    /// an incompatible point type, and propagates whatever the point tree
    /// raises.
    pub fn matching_docs(
        &self,
        reader: &dyn LeafReader,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        match self.scorer_supplier(reader, 0.0, ScoreMode::COMPLETE_NO_SCORES)? {
            Some(mut supplier) => Ok(Some(supplier.iterator()?)),
            None => Ok(None),
        }
    }

    /// Equivalent to `scorerSupplier(LeafReaderContext)` of the
    /// `ConstantScoreWeight` `XYPointInGeometryQuery.createWeight` builds
    /// (`XYPointInGeometryQuery.java:136-176`): `None` when the leaf has no
    /// points or no field info for the field; otherwise the supplier, which
    /// intersects the tree only in `get` and estimates its cost lazily.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) when the field was indexed with
    /// an incompatible point type, and propagates whatever opening the points
    /// raises.
    fn scorer_supplier(
        &self,
        reader: &dyn LeafReader,
        score: f32,
        score_mode: ScoreMode,
    ) -> Result<Option<XYPointInGeometryScorerSupplier>> {
        let Some(values) = reader.get_point_values(&self.field)? else {
            // No docs in this segment had any points fields
            return Ok(None);
        };
        let field_infos = reader.get_field_infos();
        let Some(field_info) = field_infos.field_info(&self.field) else {
            // No docs in this segment indexed this field at all
            return Ok(None);
        };
        XYPointField::check_compatible(field_info)?;

        let max_doc = reader.max_doc();
        let result = DocIdSetBuilder::from_point_values(max_doc, values.as_ref())?;
        Ok(Some(XYPointInGeometryScorerSupplier {
            tree: Arc::clone(&self.tree),
            values,
            max_doc,
            result: RefCell::new(Some(result)),
            cost: Cell::new(-1),
            score,
            score_mode,
        }))
    }

    /// Builds the visitor that collects the points falling inside `tree`.
    ///
    /// Equivalent to the private
    /// `XYPointInGeometryQuery.getIntersectVisitor(DocIdSetBuilder, Component2D)`.
    fn get_intersect_visitor<'a>(
        result: &'a mut DocIdSetBuilder,
        tree: &'a dyn Component2D,
    ) -> XYPointIntersectVisitor<'a> {
        XYPointIntersectVisitor { result, tree }
    }

    /// Returns the estimated number of documents this query matches in
    /// `reader`.
    ///
    /// Equivalent to the `ScorerSupplier.cost()` Java computes lazily.
    ///
    /// # Errors
    ///
    /// Propagates whatever the point tree raises.
    pub fn cost(&self, reader: &dyn LeafReader) -> Result<i64> {
        let Some(values) = reader.get_point_values(&self.field)? else {
            return Ok(0);
        };
        let mut result = DocIdSetBuilder::from_point_values(reader.max_doc(), values.as_ref())?;
        let mut visitor = Self::get_intersect_visitor(&mut result, self.tree.as_ref());
        let cost = values.estimate_doc_count(&mut visitor)?;
        debug_assert!(cost >= 0);
        Ok(cost)
    }

    /// Prints this query, omitting `field` when it is the default one.
    ///
    /// Equivalent to `XYPointInGeometryQuery.toString(String)`.
    pub fn to_query_string(&self, field: &str) -> String {
        let mut sb = String::from("XYPointInGeometryQuery:");
        if self.field != field {
            sb.push_str(" field=");
            sb.push_str(&self.field);
            sb.push(':');
        }
        sb.push_str(&format!("{:?}", self.xy_geometries));
        sb
    }
}

/// The anonymous `ScorerSupplier` of the weight
/// `XYPointInGeometryQuery.createWeight` builds
/// (`XYPointInGeometryQuery.java:151-175`).
///
/// The tree is intersected only in [`get`](ScorerSupplier::get); `cost()`
/// estimates with the same visitor, without intersecting, and memoises the
/// estimate in `cost` (`-1` until computed), as Java's `long cost = -1` field.
/// The visitor's `DocIdSetBuilder result` sits in a `RefCell` because `cost()`
/// takes `&self` and the visitor holds the builder by `&mut`; the estimate
/// never adds to it.
struct XYPointInGeometryScorerSupplier {
    tree: Arc<dyn Component2D>,
    values: Box<dyn PointValues>,
    max_doc: i32,
    /// `final DocIdSetBuilder result = new DocIdSetBuilder(reader.maxDoc(),
    /// values)`; `None` once `get` has built it.
    result: RefCell<Option<DocIdSetBuilder>>,
    cost: Cell<i64>,
    score: f32,
    score_mode: ScoreMode,
}

impl std::fmt::Debug for XYPointInGeometryScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XYPointInGeometryScorerSupplier")
            .field("max_doc", &self.max_doc)
            .field("cost", &self.cost.get())
            .finish_non_exhaustive()
    }
}

impl XYPointInGeometryScorerSupplier {
    /// `values.estimateDocCount(visitor)`, with the visitor over `result`.
    fn estimate_doc_count(&self) -> Result<i64> {
        let mut result = self.result.borrow_mut();
        let mut built;
        let result = match result.as_mut() {
            Some(result) => result,
            // `get` already consumed the builder; the estimate only calls
            // `compare`, so a fresh builder stands for it.
            None => {
                built = DocIdSetBuilder::from_point_values(self.max_doc, self.values.as_ref())?;
                &mut built
            }
        };
        let mut visitor = XYPointInGeometryQuery::get_intersect_visitor(result, self.tree.as_ref());
        self.values.estimate_doc_count(&mut visitor)
    }

    /// The iterator `get(long)` wraps in its `ConstantScoreScorer`
    /// (`XYPointInGeometryQuery.java:157-161`).
    ///
    /// # Errors
    ///
    /// Propagates whatever the point tree raises; a second call is refused
    /// with [`LuceneErrorKind::IllegalState`](crate::error::LuceneErrorKind::IllegalState),
    /// as `ScorerSupplier.get` may be called at most once.
    fn iterator(&mut self) -> Result<Box<dyn DocIdSetIterator>> {
        let mut result = self.result.get_mut().take().ok_or_else(|| {
            LuceneError::illegal_state(
                "ScorerSupplier.get(long) must be called at most once".to_string(),
            )
        })?;
        {
            let mut visitor =
                XYPointInGeometryQuery::get_intersect_visitor(&mut result, self.tree.as_ref());
            self.values.intersect(&mut visitor)?;
        }
        result.build()?.iterator()
    }
}

impl ScorerSupplier for XYPointInGeometryScorerSupplier {
    fn get(&mut self, _lead_cost: i64) -> Result<Box<dyn Scorer>> {
        let iterator = self.iterator()?;
        Ok(Box::new(ConstantScoreScorer::from_iterator(
            self.score,
            self.score_mode,
            iterator,
        )))
    }

    /// `if (cost == -1) cost = values.estimateDocCount(visitor)`
    /// (`XYPointInGeometryQuery.java:163-172`).
    ///
    /// # Panics
    ///
    /// Java's `estimateDocCount` rethrows an `IOException` as the unchecked
    /// `UncheckedIOException`, which `cost()` lets propagate; this port
    /// raises the error the estimate returns as a panic payload.
    fn cost(&self) -> i64 {
        if self.cost.get() == -1 {
            // Computing the cost may be expensive, so only do it if necessary
            let cost = self
                .estimate_doc_count()
                .unwrap_or_else(|e| std::panic::panic_any(e));
            debug_assert!(cost >= 0);
            self.cost.set(cost);
        }
        self.cost.get()
    }
}

impl PartialEq for XYPointInGeometryQuery {
    /// Equivalent to the private `XYPointInGeometryQuery.equalsTo`.
    fn eq(&self, other: &Self) -> bool {
        self.field == other.field && self.xy_geometries == other.xy_geometries
    }
}

impl SpatialMatcher for XYPointInGeometryQuery {
    fn spatial_field(&self) -> &str {
        &self.field
    }

    fn spatial_matching_docs(
        &self,
        reader: &dyn LeafReader,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        self.matching_docs(reader)
    }

    fn spatial_cost(&self, reader: &dyn LeafReader) -> Result<i64> {
        self.cost(reader)
    }

    /// The query's own `ConstantScoreWeight.scorerSupplier`, whose supplier
    /// intersects lazily. Its `score()` is the boost `createWeight` received,
    /// the constant score of the enclosing weight.
    fn spatial_scorer_supplier_override(
        &self,
        reader: &dyn LeafReader,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Option<Option<Box<dyn ScorerSupplier>>>> {
        Ok(Some(self.scorer_supplier(reader, boost, score_mode)?.map(
            |supplier| Box::new(supplier) as Box<dyn ScorerSupplier>,
        )))
    }
}

impl Query for XYPointInGeometryQuery {
    /// Equivalent to `XYPointInGeometryQuery.toString(String)`.
    fn to_query_string(&self, field: &str) -> String {
        XYPointInGeometryQuery::to_query_string(self, field)
    }

    /// Equivalent to `XYPointInGeometryQuery.visit(QueryVisitor)`.
    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(&self.field) {
            visitor.visit_leaf(self);
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// Equivalent to
    /// `XYPointInGeometryQuery.createWeight(IndexSearcher, ScoreMode, float)`,
    /// whose `ConstantScoreWeight` walks the BKD tree with the geometry
    /// visitor.
    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let matcher = Arc::new(self.clone());
        let query: Arc<dyn Query> = Arc::new(self.clone());
        spatial_create_weight(matcher, query, score_mode, boost)
    }

    /// Equivalent to `XYPointInGeometryQuery.equals(Object)`.
    fn query_eq(&self, other: &dyn Query) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| self == other)
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.XYPointInGeometryQuery"
    }

    /// Equivalent to `XYPointInGeometryQuery.hashCode()`: `classHash()`,
    /// `field.hashCode()` and `Arrays.hashCode(xyGeometries)`.
    fn query_hash(&self) -> i32 {
        let mut result = self.class_hash();
        result = 31i32.wrapping_mul(result).wrapping_add(
            crate::util::java_collections::java_string_hash_code(&self.field),
        );
        31i32.wrapping_mul(result).wrapping_add(
            crate::document::spatial_query::xy_geometries_hash_code(&self.xy_geometries),
        )
    }
}

// -----------------------------------------------------------------------------
// LatLonPointDistanceQuery
// -----------------------------------------------------------------------------

/// The bounding boxes and the distance predicate a
/// [`LatLonPointDistanceQuery`] tests against.
///
/// Equivalent to the state the anonymous `ConstantScoreWeight` of
/// `LatLonPointDistanceQuery.createWeight` captures.
#[derive(Clone, Debug)]
struct LatLonDistanceBounds {
    latitude: f64,
    longitude: f64,
    /// The grid of per-sub-box relations, which answers most encoded points
    /// without a haversine call.
    ///
    /// Equivalent to the `GeoEncodingUtils.DistancePredicate` Java precomputes
    /// in `LatLonPointDistanceQuery.createWeight`.
    predicate: DistancePredicate,
    min_lat: i32,
    max_lat: i32,
    min_lon: i32,
    max_lon: i32,
    /// A second longitude range, for the cross-dateline case.
    min_lon2: i32,
    /// The exact sort key, which avoids any `asin()` computation.
    sort_key: f64,
    axis_lat: f64,
}

impl LatLonDistanceBounds {
    /// Equivalent to the bounding-box setup at the top of
    /// `LatLonPointDistanceQuery.createWeight`.
    fn new(latitude: f64, longitude: f64, radius_meters: f64) -> Result<Self> {
        let box_ = crate::geo::geometry::Rectangle::from_point_distance(
            latitude,
            longitude,
            radius_meters,
        )?;
        let min_lat = GeoEncodingUtils::encode_latitude(box_.min_lat())?;
        let max_lat = GeoEncodingUtils::encode_latitude(box_.max_lat())?;
        let (min_lon, max_lon, min_lon2) = if box_.crosses_dateline() {
            // Crosses the dateline: split into two boxes.
            (
                i32::MIN,
                GeoEncodingUtils::encode_longitude(box_.max_lon())?,
                GeoEncodingUtils::encode_longitude(box_.min_lon())?,
            )
        } else {
            (
                GeoEncodingUtils::encode_longitude(box_.min_lon())?,
                GeoEncodingUtils::encode_longitude(box_.max_lon())?,
                // Disable box two.
                i32::MAX,
            )
        };
        Ok(Self {
            latitude,
            longitude,
            min_lat,
            max_lat,
            min_lon,
            max_lon,
            min_lon2,
            sort_key: GeoUtils::distance_query_sort_key(radius_meters),
            axis_lat: crate::geo::geometry::Rectangle::axis_lat(latitude, radius_meters),
            predicate: GeoEncodingUtils::create_distance_predicate(
                latitude,
                longitude,
                radius_meters,
            )?,
        })
    }

    /// Equivalent to `DistancePredicate.test(int, int)`, the grid lookup Java
    /// precomputes for this weight.
    fn test(&self, lat: i32, lon: i32) -> bool {
        self.predicate.test(lat, lon)
    }

    /// Equivalent to the private `matches(byte[])` of the weight.
    fn matches(&self, packed_value: &[u8]) -> bool {
        let lat = NumericUtils::sortable_bytes_to_int(packed_value, 0);
        // Bounding-box check.
        if lat > self.max_lat || lat < self.min_lat {
            // Latitude out of the bounding-box range.
            return false;
        }
        let lon = NumericUtils::sortable_bytes_to_int(packed_value, COORDINATE_BYTES);
        if (lon > self.max_lon || lon < self.min_lon) && lon < self.min_lon2 {
            // Longitude out of the bounding-box range.
            return false;
        }
        self.test(lat, lon)
    }

    /// Equivalent to the private `relate(byte[], byte[])` of the weight.
    ///
    /// The algorithm builds one bounding box, or two when the circle crosses
    /// the dateline, and then:
    ///
    /// 1. checks the bounding box(es) first, bailing out when the subtree falls
    ///    entirely outside them;
    /// 2. checks whether the subtree is disjoint — it may cross the bounding box
    ///    yet miss the circle;
    /// 3. checks whether the subtree is fully contained, which cannot work for a
    ///    subtree enormous along the x axis, wrapping half way around the world;
    /// 4. otherwise recurses naively, for a subtree crossing the circle's edge.
    fn relate(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Result<Relation> {
        let lat_lower_bound = NumericUtils::sortable_bytes_to_int(min_packed_value, 0);
        let lat_upper_bound = NumericUtils::sortable_bytes_to_int(max_packed_value, 0);
        if lat_lower_bound > self.max_lat || lat_upper_bound < self.min_lat {
            // Latitude out of the bounding-box range.
            return Ok(Relation::CellOutsideQuery);
        }

        let lon_lower_bound =
            NumericUtils::sortable_bytes_to_int(min_packed_value, LatLonPoint::BYTES);
        let lon_upper_bound =
            NumericUtils::sortable_bytes_to_int(max_packed_value, LatLonPoint::BYTES);
        if (lon_lower_bound > self.max_lon || lon_upper_bound < self.min_lon)
            && lon_upper_bound < self.min_lon2
        {
            // Longitude out of the bounding-box range.
            return Ok(Relation::CellOutsideQuery);
        }

        let lat_min = GeoEncodingUtils::decode_latitude(lat_lower_bound);
        let lon_min = GeoEncodingUtils::decode_longitude(lon_lower_bound);
        let lat_max = GeoEncodingUtils::decode_latitude(lat_upper_bound);
        let lon_max = GeoEncodingUtils::decode_longitude(lon_upper_bound);

        GeoUtils::relate(
            lat_min,
            lat_max,
            lon_min,
            lon_max,
            self.latitude,
            self.longitude,
            self.sort_key,
            self.axis_lat,
        )
    }
}

/// Collects the indexed points that fall inside the query circle.
///
/// Equivalent to the visitor
/// `LatLonPointDistanceQuery.getIntersectVisitor(DocIdSetBuilder)` returns.
struct DistanceIntersectVisitor<'a> {
    bounds: LatLonDistanceBounds,
    result: &'a mut DocIdSetBuilder,
}

impl IntersectVisitor for DistanceIntersectVisitor<'_> {
    /// `LatLonPointDistanceQuery$1$2` declares no `toString()`, so `java.lang.Object.toString()` applies
    /// ([`java_object_to_string_stand_in`](crate::index::index_reader::java_object_to_string_stand_in)).
    fn java_to_string(&self) -> String {
        crate::index::index_reader::java_object_to_string_stand_in(
            "org.apache.lucene.document.LatLonPointDistanceQuery$1$2",
        )
    }
    fn grow(&mut self, count: i32) -> Result<()> {
        self.result.grow(count);
        Ok(())
    }

    fn visit(&mut self, doc_id: i32) -> Result<()> {
        self.result.grow(0).add(doc_id);
        Ok(())
    }

    fn visit_ints_ref(&mut self, ints_ref: &IntsRef) -> Result<()> {
        self.result.grow(0).add_ints(ints_ref);
        Ok(())
    }

    fn visit_iterator(&mut self, iterator: &mut dyn DocIdSetIterator) -> Result<()> {
        self.result.grow(0).add_iterator(iterator)
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        if self.bounds.matches(packed_value) {
            self.visit(doc_id)?;
        }
        Ok(())
    }

    fn visit_iterator_with_value(
        &mut self,
        iterator: &mut dyn DocIdSetIterator,
        packed_value: &[u8],
    ) -> Result<()> {
        if self.bounds.matches(packed_value) {
            self.result.grow(0).add_iterator(iterator)?;
        }
        Ok(())
    }

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Result<Relation> {
        Ok(self
            .bounds
            .relate(min_packed_value, max_packed_value)
            .unwrap_or(Relation::CellCrossesQuery))
    }
}

/// Clears the documents that do *not* fall inside the query circle.
///
/// Equivalent to the visitor
/// `LatLonPointDistanceQuery.getInverseIntersectVisitor(FixedBitSet, long[])`
/// returns.
struct InverseDistanceIntersectVisitor<'a> {
    bounds: LatLonDistanceBounds,
    result: &'a mut crate::util::FixedBitSet,
    cost: &'a mut i64,
}

impl IntersectVisitor for InverseDistanceIntersectVisitor<'_> {
    /// `LatLonPointDistanceQuery$1$3` declares no `toString()`, so `java.lang.Object.toString()` applies
    /// ([`java_object_to_string_stand_in`](crate::index::index_reader::java_object_to_string_stand_in)).
    fn java_to_string(&self) -> String {
        crate::index::index_reader::java_object_to_string_stand_in(
            "org.apache.lucene.document.LatLonPointDistanceQuery$1$3",
        )
    }
    fn visit(&mut self, doc_id: i32) -> Result<()> {
        self.result.clear(doc_id as usize);
        *self.cost -= 1;
        Ok(())
    }

    fn visit_ints_ref(&mut self, ints_ref: &IntsRef) -> Result<()> {
        for doc_id in ints_ref.slice().iter().copied() {
            self.result.clear(doc_id as usize);
        }
        *self.cost = 0.max(*self.cost - ints_ref.length as i64);
        Ok(())
    }

    fn visit_iterator(&mut self, iterator: &mut dyn DocIdSetIterator) -> Result<()> {
        let cost = iterator.cost();
        loop {
            let doc = iterator.next_doc()?;
            if doc == NO_MORE_DOCS {
                break;
            }
            self.result.clear(doc as usize);
        }
        *self.cost = 0.max(*self.cost - cost);
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        if !self.bounds.matches(packed_value) {
            self.visit(doc_id)?;
        }
        Ok(())
    }

    fn visit_iterator_with_value(
        &mut self,
        iterator: &mut dyn DocIdSetIterator,
        packed_value: &[u8],
    ) -> Result<()> {
        if !self.bounds.matches(packed_value) {
            self.visit_iterator(iterator)?;
        }
        Ok(())
    }

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Result<Relation> {
        Ok(
            match self
                .bounds
                .relate(min_packed_value, max_packed_value)
                .unwrap_or(Relation::CellCrossesQuery)
            {
                // All points match, so skip this subtree.
                Relation::CellInsideQuery => Relation::CellOutsideQuery,
                // No point matches, so clear every document.
                Relation::CellOutsideQuery => Relation::CellInsideQuery,
                Relation::CellCrossesQuery => Relation::CellCrossesQuery,
            },
        )
    }
}

/// Finds every previously indexed geographic point within a radius of an
/// origin.
///
/// Equivalent to `org.apache.lucene.document.LatLonPointDistanceQuery`, which
/// [`LatLonPoint::new_distance_query`](crate::document::geo_fields::LatLonPoint) builds.
#[derive(Clone, Debug)]
pub struct LatLonPointDistanceQuery {
    field: String,
    latitude: f64,
    longitude: f64,
    radius_meters: f64,
    bounds: LatLonDistanceBounds,
}

impl LatLonPointDistanceQuery {
    /// Creates the query.
    ///
    /// Equivalent to
    /// `LatLonPointDistanceQuery(String, double, double, double)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) for a non-finite or negative
    /// radius, and for an invalid latitude or longitude.
    pub fn new(
        field: impl Into<String>,
        latitude: f64,
        longitude: f64,
        radius_meters: f64,
    ) -> Result<Self> {
        if !radius_meters.is_finite() || radius_meters < 0.0 {
            return Err(LuceneError::illegal_argument(format!(
                "radiusMeters: '{radius_meters}' is invalid"
            )));
        }
        GeoUtils::check_latitude(latitude)?;
        GeoUtils::check_longitude(longitude)?;
        Ok(Self {
            field: field.into(),
            latitude,
            longitude,
            radius_meters,
            bounds: LatLonDistanceBounds::new(latitude, longitude, radius_meters)?,
        })
    }

    /// Returns the query field.
    ///
    /// Equivalent to `LatLonPointDistanceQuery.getField()`.
    pub fn get_field(&self) -> &str {
        &self.field
    }

    /// Returns the origin latitude.
    ///
    /// Equivalent to `LatLonPointDistanceQuery.getLatitude()`.
    pub fn get_latitude(&self) -> f64 {
        self.latitude
    }

    /// Returns the origin longitude.
    ///
    /// Equivalent to `LatLonPointDistanceQuery.getLongitude()`.
    pub fn get_longitude(&self) -> f64 {
        self.longitude
    }

    /// Returns the radius in metres.
    ///
    /// Equivalent to `LatLonPointDistanceQuery.getRadiusMeters()`.
    pub fn get_radius_meters(&self) -> f64 {
        self.radius_meters
    }

    /// Returns the documents of `reader` this query matches: the iterator the
    /// scorer of [`scorer_supplier`](Self::scorer_supplier)'s `get` wraps.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) when the field was indexed with
    /// an incompatible point type, and propagates whatever the point tree
    /// raises.
    pub fn matching_docs(
        &self,
        reader: &dyn LeafReader,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        match self.scorer_supplier(reader, 0.0, ScoreMode::COMPLETE_NO_SCORES)? {
            Some(mut supplier) => Ok(Some(supplier.iterator()?)),
            None => Ok(None),
        }
    }

    /// Equivalent to `scorerSupplier(LeafReaderContext)` of the
    /// `ConstantScoreWeight` `LatLonPointDistanceQuery.createWeight` builds
    /// (`LatLonPointDistanceQuery.java:122-173`): `None` when the leaf has no
    /// points or no field info for the field; otherwise the supplier, which
    /// intersects the tree only in `get` and estimates its cost lazily.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) when the field was indexed with
    /// an incompatible point type, and propagates whatever opening the points
    /// raises.
    fn scorer_supplier(
        &self,
        reader: &dyn LeafReader,
        score: f32,
        score_mode: ScoreMode,
    ) -> Result<Option<LatLonPointDistanceScorerSupplier>> {
        let Some(values) = reader.get_point_values(&self.field)? else {
            // No docs in this segment had any points fields
            return Ok(None);
        };
        let field_infos = reader.get_field_infos();
        let Some(field_info) = field_infos.field_info(&self.field) else {
            // No docs in this segment indexed this field at all
            return Ok(None);
        };
        LatLonPoint::check_compatible(field_info)?;

        // matching docids
        let max_doc = reader.max_doc();
        let result = DocIdSetBuilder::from_point_values(max_doc, values.as_ref())?;
        Ok(Some(LatLonPointDistanceScorerSupplier {
            bounds: self.bounds.clone(),
            values,
            max_doc,
            result: RefCell::new(Some(result)),
            cost: Cell::new(-1),
            score,
            score_mode,
        }))
    }

    /// Returns the estimated number of documents this query matches in
    /// `reader`.
    ///
    /// Equivalent to the `ScorerSupplier.cost()` Java computes lazily.
    ///
    /// # Errors
    ///
    /// Propagates whatever the point tree raises.
    pub fn cost(&self, reader: &dyn LeafReader) -> Result<i64> {
        let Some(values) = reader.get_point_values(&self.field)? else {
            return Ok(0);
        };
        let mut result = DocIdSetBuilder::from_point_values(reader.max_doc(), values.as_ref())?;
        let mut visitor = DistanceIntersectVisitor {
            bounds: self.bounds.clone(),
            result: &mut result,
        };
        let cost = values.estimate_doc_count(&mut visitor)?;
        debug_assert!(cost >= 0);
        Ok(cost)
    }

    /// Prints this query, omitting `field` when it is the default one.
    ///
    /// Equivalent to `LatLonPointDistanceQuery.toString(String)`.
    pub fn to_query_string(&self, field: &str) -> String {
        let mut sb = String::new();
        if self.field != field {
            sb.push_str(&self.field);
            sb.push(':');
        }
        // `StringBuilder.append(double)` (`LatLonPointDistanceQuery.java:370-383`):
        // Java's `Double.toString`.
        sb.push_str(&format!(
            "{},{} +/- {} meters",
            crate::util::java_fmt::double_to_string(self.latitude),
            crate::util::java_fmt::double_to_string(self.longitude),
            crate::util::java_fmt::double_to_string(self.radius_meters)
        ));
        sb
    }
}

impl PartialEq for LatLonPointDistanceQuery {
    /// Equivalent to the private `LatLonPointDistanceQuery.equalsTo`.
    fn eq(&self, other: &Self) -> bool {
        self.field == other.field
            && self.latitude.to_bits() == other.latitude.to_bits()
            && self.longitude.to_bits() == other.longitude.to_bits()
            && self.radius_meters.to_bits() == other.radius_meters.to_bits()
    }
}

/// The anonymous `ScorerSupplier` of the weight
/// `LatLonPointDistanceQuery.createWeight` builds
/// (`LatLonPointDistanceQuery.java:141-172`).
///
/// The tree is intersected only in [`get`](ScorerSupplier::get); `cost()`
/// estimates with the same visitor, without intersecting, and memoises the
/// estimate in `cost` (`-1` until computed), as Java's `long cost = -1` field.
/// The visitor's `DocIdSetBuilder result` sits in a `RefCell` because `cost()`
/// takes `&self` and the visitor holds the builder by `&mut`; the estimate
/// never adds to it.
struct LatLonPointDistanceScorerSupplier {
    bounds: LatLonDistanceBounds,
    values: Box<dyn PointValues>,
    max_doc: i32,
    /// `DocIdSetBuilder result = new DocIdSetBuilder(reader.maxDoc(), values)`;
    /// `None` once `get` has built it.
    result: RefCell<Option<DocIdSetBuilder>>,
    cost: Cell<i64>,
    score: f32,
    score_mode: ScoreMode,
}

impl std::fmt::Debug for LatLonPointDistanceScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LatLonPointDistanceScorerSupplier")
            .field("max_doc", &self.max_doc)
            .field("cost", &self.cost.get())
            .finish_non_exhaustive()
    }
}

impl LatLonPointDistanceScorerSupplier {
    /// `values.estimateDocCount(visitor)`, with the visitor over `result`.
    fn estimate_doc_count(&self) -> Result<i64> {
        let mut result = self.result.borrow_mut();
        let mut built;
        let result = match result.as_mut() {
            Some(result) => result,
            // `get` already consumed the builder; the estimate only calls
            // `compare`, so a fresh builder stands for it.
            None => {
                built = DocIdSetBuilder::from_point_values(self.max_doc, self.values.as_ref())?;
                &mut built
            }
        };
        let mut visitor = DistanceIntersectVisitor {
            bounds: self.bounds.clone(),
            result,
        };
        self.values.estimate_doc_count(&mut visitor)
    }

    /// The iterator `get(long)` wraps in its `ConstantScoreScorer`
    /// (`LatLonPointDistanceQuery.java:145-162`).
    ///
    /// # Errors
    ///
    /// Propagates whatever the point tree raises; a second call is refused
    /// with [`LuceneErrorKind::IllegalState`](crate::error::LuceneErrorKind::IllegalState),
    /// as `ScorerSupplier.get` may be called at most once.
    fn iterator(&mut self) -> Result<Box<dyn DocIdSetIterator>> {
        let max_doc = self.max_doc;
        if self.values.doc_count()? == max_doc
            && i64::from(self.values.doc_count()?) == self.values.size()?
            && ScorerSupplier::cost(self) > i64::from(max_doc / 2)
        {
            // If all docs have exactly one value and the cost is greater
            // than half the leaf size then maybe we can make things faster
            // by computing the set of documents that do NOT match the range
            let mut result = crate::util::FixedBitSet::new(max_doc as usize);
            result.set_range(0, max_doc as usize);
            let mut cost = i64::from(max_doc);
            let mut inverse = InverseDistanceIntersectVisitor {
                bounds: self.bounds.clone(),
                result: &mut result,
                cost: &mut cost,
            };
            self.values.intersect(&mut inverse)?;
            return Ok(Box::new(crate::util::BitSetIterator::new(
                Arc::new(result),
                cost,
            )?));
        }
        let mut result = self.result.get_mut().take().ok_or_else(|| {
            LuceneError::illegal_state(
                "ScorerSupplier.get(long) must be called at most once".to_string(),
            )
        })?;
        {
            let mut visitor = DistanceIntersectVisitor {
                bounds: self.bounds.clone(),
                result: &mut result,
            };
            self.values.intersect(&mut visitor)?;
        }
        result.build()?.iterator()
    }
}

impl ScorerSupplier for LatLonPointDistanceScorerSupplier {
    fn get(&mut self, _lead_cost: i64) -> Result<Box<dyn Scorer>> {
        let iterator = self.iterator()?;
        Ok(Box::new(ConstantScoreScorer::from_iterator(
            self.score,
            self.score_mode,
            iterator,
        )))
    }

    /// `if (cost == -1) cost = values.estimateDocCount(visitor)`
    /// (`LatLonPointDistanceQuery.java:164-171`).
    ///
    /// # Panics
    ///
    /// Java's `estimateDocCount` rethrows an `IOException` as the unchecked
    /// `UncheckedIOException`, which `cost()` lets propagate; this port
    /// raises the error the estimate returns as a panic payload.
    fn cost(&self) -> i64 {
        if self.cost.get() == -1 {
            let cost = self
                .estimate_doc_count()
                .unwrap_or_else(|e| std::panic::panic_any(e));
            self.cost.set(cost);
        }
        debug_assert!(self.cost.get() >= 0);
        self.cost.get()
    }
}

// -----------------------------------------------------------------------------
// Executing the point queries
// -----------------------------------------------------------------------------
//
// Java's LatLonPointQuery and LatLonPointDistanceQuery each extend
// SpatialQuery, which extends Query and supplies the ConstantScoreWeight. Rust
// has no implementation inheritance, so each states how it iterates through
// SpatialMatcher and the weight is built once by `spatial_create_weight`.

impl SpatialMatcher for LatLonPointQuery {
    fn spatial_field(&self) -> &str {
        self.query.get_field()
    }

    fn spatial_matching_docs(
        &self,
        reader: &dyn LeafReader,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        self.matching_docs(reader)
    }

    fn spatial_cost(&self, reader: &dyn LeafReader) -> Result<i64> {
        let Some(values) = reader.get_point_values(self.query.get_field())? else {
            return Ok(0);
        };
        self.query.cost(values.as_ref(), &self.visitor)
    }

    /// `LatLonPointQuery extends SpatialQuery` and keeps its
    /// `getScorerSupplier`.
    fn spatial_query(&self) -> Option<(&SpatialQuery, &dyn SpatialVisitor)> {
        Some((&self.query, &self.visitor))
    }
}

impl Query for LatLonPointQuery {
    fn to_query_string(&self, field: &str) -> String {
        self.query.to_query_string(
            "LatLonPointQuery",
            field,
            &self
                .geometries
                .iter()
                .map(|g| format!("{g:?}"))
                .collect::<Vec<_>>(),
        )
    }

    /// Equivalent to the inherited `SpatialQuery.visit(QueryVisitor)`.
    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.query.visit(visitor, self);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let matcher = Arc::new(self.clone());
        let query: Arc<dyn Query> = Arc::new(self.clone());
        spatial_create_weight(matcher, query, score_mode, boost)
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| self.query == other.query && self.geometries == other.geometries)
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.LatLonPointQuery"
    }

    /// Equivalent to `LatLonPointQuery.hashCode()`: `super.hashCode()` (the
    /// `SpatialQuery` hash, whose geometries are this query's), then
    /// `31 * hash + Arrays.hashCode(geometries)`.
    fn query_hash(&self) -> i32 {
        let geometries =
            crate::document::spatial_query::lat_lon_geometries_hash_code(&self.geometries);
        let hash = crate::document::spatial_query::spatial_query_hash_code(
            self.class_hash(),
            &self.query,
            geometries,
        );
        31i32.wrapping_mul(hash).wrapping_add(geometries)
    }
}

impl SpatialMatcher for LatLonPointDistanceQuery {
    fn spatial_field(&self) -> &str {
        &self.field
    }

    fn spatial_matching_docs(
        &self,
        reader: &dyn LeafReader,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        self.matching_docs(reader)
    }

    fn spatial_cost(&self, reader: &dyn LeafReader) -> Result<i64> {
        self.cost(reader)
    }

    /// The query's own `ConstantScoreWeight.scorerSupplier`, whose supplier
    /// intersects lazily. Its `score()` is the boost `createWeight` received,
    /// the constant score of the enclosing weight.
    fn spatial_scorer_supplier_override(
        &self,
        reader: &dyn LeafReader,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Option<Option<Box<dyn ScorerSupplier>>>> {
        Ok(Some(self.scorer_supplier(reader, boost, score_mode)?.map(
            |supplier| Box::new(supplier) as Box<dyn ScorerSupplier>,
        )))
    }
}

impl Query for LatLonPointDistanceQuery {
    fn to_query_string(&self, field: &str) -> String {
        LatLonPointDistanceQuery::to_query_string(self, field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(&self.field) {
            visitor.visit_leaf(self);
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let matcher = Arc::new(self.clone());
        let query: Arc<dyn Query> = Arc::new(self.clone());
        spatial_create_weight(matcher, query, score_mode, boost)
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|o| self == o)
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.LatLonPointDistanceQuery"
    }

    /// Equivalent to `LatLonPointDistanceQuery.hashCode()`: `classHash()`,
    /// `field.hashCode()` and the `doubleToLongBits` fold of the latitude,
    /// longitude and radius.
    fn query_hash(&self) -> i32 {
        use crate::util::java_collections::{java_double_hash_code, java_string_hash_code};
        let mut result = self.class_hash();
        result = 31i32
            .wrapping_mul(result)
            .wrapping_add(java_string_hash_code(&self.field));
        result = 31i32
            .wrapping_mul(result)
            .wrapping_add(java_double_hash_code(self.latitude));
        result = 31i32
            .wrapping_mul(result)
            .wrapping_add(java_double_hash_code(self.longitude));
        31i32
            .wrapping_mul(result)
            .wrapping_add(java_double_hash_code(self.radius_meters))
    }
}

// -----------------------------------------------------------------------------
// The Query wrappers of the three doc-values queries
// -----------------------------------------------------------------------------
//
// Java's three doc-values queries each extend Query and supply a
// ConstantScoreWeight whose scorer wraps a TwoPhaseIterator over the field's
// sorted-numeric values. Rust has no implementation inheritance, so each
// supplies that scorer through
// `SpatialMatcher::spatial_scorer_supplier_override` and the weight is built
// once by `spatial_create_weight`.

impl SpatialMatcher for LatLonDocValuesBoxQuery {
    fn spatial_field(&self) -> &str {
        &self.field
    }

    fn spatial_matching_docs(
        &self,
        reader: &dyn LeafReader,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        self.matching_docs(reader)
    }

    fn spatial_cost(&self, reader: &dyn LeafReader) -> Result<i64> {
        Ok(reader
            .get_sorted_numeric_doc_values(&self.field)?
            .map_or(0, |values| values.cost()))
    }

    /// The query's own `ConstantScoreWeight.scorerSupplier`, which scores
    /// through the two-phase iterator.
    fn spatial_scorer_supplier_override(
        &self,
        reader: &dyn LeafReader,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Option<Option<Box<dyn ScorerSupplier>>>> {
        Ok(Some(self.scorer_supplier(reader, score_mode, boost)?))
    }

    /// `DocValues.isCacheable(ctx, field)` (`LatLonDocValuesBoxQuery.java:160-163`).
    fn spatial_is_cacheable(&self, ctx: &crate::index::LeafReaderContext) -> bool {
        crate::index::DocValues::is_cacheable(ctx, &[&self.field])
    }
}

impl Query for LatLonDocValuesBoxQuery {
    fn to_query_string(&self, field: &str) -> String {
        LatLonDocValuesBoxQuery::to_query_string(self, field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(&self.field) {
            visitor.visit_leaf(self);
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| self == other)
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.LatLonDocValuesBoxQuery"
    }

    /// Equivalent to `LatLonDocValuesBoxQuery.hashCode()`.
    fn query_hash(&self) -> i32 {
        use crate::util::java_collections::{java_boolean_hash_code, java_string_hash_code};
        let mut h = self.class_hash();
        h = 31i32
            .wrapping_mul(h)
            .wrapping_add(java_string_hash_code(&self.field));
        h = 31i32
            .wrapping_mul(h)
            .wrapping_add(java_boolean_hash_code(self.crosses_dateline));
        h = 31i32
            .wrapping_mul(h)
            .wrapping_add(self.matcher_min_latitude);
        h = 31i32
            .wrapping_mul(h)
            .wrapping_add(self.matcher_max_latitude);
        h = 31i32
            .wrapping_mul(h)
            .wrapping_add(self.matcher_min_longitude);
        31i32
            .wrapping_mul(h)
            .wrapping_add(self.matcher_max_longitude)
    }

    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let matcher = Arc::new(self.clone());
        let query: Arc<dyn Query> = Arc::new(self.clone());
        spatial_create_weight(matcher, query, score_mode, boost)
    }
}

impl SpatialMatcher for LatLonDocValuesQuery {
    fn spatial_field(&self) -> &str {
        &self.field
    }

    fn spatial_matching_docs(
        &self,
        reader: &dyn LeafReader,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        self.matching_docs(reader)
    }

    fn spatial_cost(&self, reader: &dyn LeafReader) -> Result<i64> {
        Ok(reader
            .get_sorted_numeric_doc_values(&self.field)?
            .map_or(0, |values| values.cost()))
    }

    /// The query's own `ConstantScoreWeight.scorerSupplier`, which scores
    /// through the two-phase iterator.
    fn spatial_scorer_supplier_override(
        &self,
        reader: &dyn LeafReader,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Option<Option<Box<dyn ScorerSupplier>>>> {
        Ok(Some(self.scorer_supplier(reader, score_mode, boost)?))
    }

    /// `DocValues.isCacheable(ctx, field)` (`LatLonDocValuesQuery.java:165-168`).
    fn spatial_is_cacheable(&self, ctx: &crate::index::LeafReaderContext) -> bool {
        crate::index::DocValues::is_cacheable(ctx, &[&self.field])
    }
}

impl Query for LatLonDocValuesQuery {
    fn to_query_string(&self, field: &str) -> String {
        LatLonDocValuesQuery::to_query_string(self, field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(&self.field) {
            visitor.visit_leaf(self);
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        other.as_any().downcast_ref::<Self>().is_some_and(|other| {
            self.field == other.field
                && self.query_relation == other.query_relation
                && self.geometries == other.geometries
        })
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.LatLonDocValuesQuery"
    }

    /// Equivalent to `LatLonDocValuesQuery.hashCode()`: `classHash()`,
    /// `field.hashCode()`, `queryRelation.hashCode()` and
    /// `Arrays.hashCode(geometries)`.
    fn query_hash(&self) -> i32 {
        let mut h = self.class_hash();
        h = 31i32.wrapping_mul(h).wrapping_add(
            crate::util::java_collections::java_string_hash_code(&self.field),
        );
        h = 31i32
            .wrapping_mul(h)
            .wrapping_add(self.query_relation.java_hash_code());
        31i32.wrapping_mul(h).wrapping_add(
            crate::document::spatial_query::lat_lon_geometries_hash_code(&self.geometries),
        )
    }

    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let matcher = Arc::new(self.clone());
        let query: Arc<dyn Query> = Arc::new(self.clone());
        spatial_create_weight(matcher, query, score_mode, boost)
    }
}

impl SpatialMatcher for XYDocValuesPointInGeometryQuery {
    fn spatial_field(&self) -> &str {
        &self.field
    }

    fn spatial_matching_docs(
        &self,
        reader: &dyn LeafReader,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        self.matching_docs(reader)
    }

    fn spatial_cost(&self, reader: &dyn LeafReader) -> Result<i64> {
        Ok(reader
            .get_sorted_numeric_doc_values(&self.field)?
            .map_or(0, |values| values.cost()))
    }

    /// The query's own `ConstantScoreWeight.scorerSupplier`, which scores
    /// through the two-phase iterator.
    fn spatial_scorer_supplier_override(
        &self,
        reader: &dyn LeafReader,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Option<Option<Box<dyn ScorerSupplier>>>> {
        Ok(Some(self.scorer_supplier(reader, score_mode, boost)?))
    }

    /// `DocValues.isCacheable(ctx, field)` (`XYDocValuesPointInGeometryQuery.java:137-140`).
    fn spatial_is_cacheable(&self, ctx: &crate::index::LeafReaderContext) -> bool {
        crate::index::DocValues::is_cacheable(ctx, &[&self.field])
    }
}

impl Query for XYDocValuesPointInGeometryQuery {
    fn to_query_string(&self, field: &str) -> String {
        XYDocValuesPointInGeometryQuery::to_query_string(self, field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(&self.field) {
            visitor.visit_leaf(self);
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| self.field == other.field && self.geometries == other.geometries)
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.XYDocValuesPointInGeometryQuery"
    }

    /// Equivalent to `XYDocValuesPointInGeometryQuery.hashCode()`.
    fn query_hash(&self) -> i32 {
        let mut h = self.class_hash();
        h = 31i32.wrapping_mul(h).wrapping_add(
            crate::util::java_collections::java_string_hash_code(&self.field),
        );
        31i32
            .wrapping_mul(h)
            .wrapping_add(crate::document::spatial_query::xy_geometries_hash_code(
                &self.geometries,
            ))
    }

    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let matcher = Arc::new(self.clone());
        let query: Arc<dyn Query> = Arc::new(self.clone());
        spatial_create_weight(matcher, query, score_mode, boost)
    }
}

#[cfg(test)]
mod xy_point_in_geometry_query_tests {
    use super::*;
    use crate::geo::geometry::XYRectangle;
    use crate::search::query_visitor::QueryVisitor;

    /// Counts the leaves a visitor is shown, and can be told to refuse a field.
    struct RecordingVisitor {
        leaves: usize,
        reject: Option<String>,
    }

    impl QueryVisitor for RecordingVisitor {
        fn visit_leaf(&mut self, _query: &dyn Query) {
            self.leaves += 1;
        }

        fn accept_field(&self, field: &str) -> bool {
            self.reject.as_deref() != Some(field)
        }
    }

    fn query(field: &str, half_side: f32) -> XYPointInGeometryQuery {
        let rectangle = XYRectangle::new(-half_side, half_side, -half_side, half_side).unwrap();
        XYPointInGeometryQuery::new(field, vec![XYGeometryValue::Rectangle(rectangle)]).unwrap()
    }

    #[test]
    fn visits_its_field_as_a_leaf() {
        // Equivalent to `XYPointInGeometryQuery.visit(QueryVisitor)`.
        let q = query("xy", 10.0);
        let mut visitor = RecordingVisitor {
            leaves: 0,
            reject: None,
        };
        Query::visit(&q, &mut visitor);
        assert_eq!(visitor.leaves, 1);

        let mut visitor = RecordingVisitor {
            leaves: 0,
            reject: Some("xy".to_string()),
        };
        Query::visit(&q, &mut visitor);
        assert_eq!(visitor.leaves, 0);
    }

    #[test]
    fn equality_follows_the_field_and_the_geometries() {
        let a = query("xy", 10.0);
        let b = query("xy", 10.0);
        let other_field = query("other", 10.0);
        let other_geometry = query("xy", 20.0);
        assert!(a.query_eq(&b));
        assert_eq!(a.query_hash(), b.query_hash());
        assert!(!a.query_eq(&other_field));
        assert!(!a.query_eq(&other_geometry));
    }

    #[test]
    fn renders_the_field_only_when_it_is_not_the_default() {
        let q = query("xy", 10.0);
        let with_default = Query::to_query_string(&q, "xy");
        let with_other = Query::to_query_string(&q, "other");
        assert!(with_default.starts_with("XYPointInGeometryQuery:"));
        assert!(!with_default.contains("field=xy"));
        assert!(with_other.contains("field=xy"));
    }

    #[test]
    fn empty_geometry_list_is_refused() {
        // Java throws IllegalArgumentException("geometries must not be empty").
        assert!(XYPointInGeometryQuery::new("xy", Vec::new()).is_err());
    }
}

#[cfg(test)]
mod scorer_supplier_tests {
    use super::*;
    use crate::codecs::{register_codec, Lucene104Codec};
    use crate::document::geo_fields::{LatLonDocValuesField, XYDocValuesField};
    use crate::document::Document;
    use crate::geo::geometry::{Rectangle, XYRectangle};
    use crate::index::doc_values::{
        BinaryDocValues, DocValuesSkipper, NumericDocValues, SortedDocValues, SortedSetDocValues,
    };
    use crate::index::index_reader::{CacheHelper, IndexReaderCore, StoredFields};
    use crate::index::index_writer::IndexWriter;
    use crate::index::index_writer_config::IndexWriterConfig;
    use crate::index::leaf_reader::{LeafMetaData, TermVectors};
    use crate::index::point_values::PointValues;
    use crate::index::vector_values::{ByteVectorValues, FloatVectorValues};
    use crate::index::{directory_reader, FieldInfos, IndexReader, LeafReaderContext, Terms};
    use crate::search::knn::KnnCollector;
    use crate::search::AcceptDocs;
    use crate::store::{ByteBuffersDirectory, Directory};
    use crate::util::Bits;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Document `i` of the test index sits at latitude = longitude =
    /// `i * 0.1 - 10` and at `x = y = i`.
    fn coordinate(i: i32) -> f64 {
        f64::from(i) * 0.1 - 10.0
    }

    /// 200 documents, one segment, each with one point and one doc value of
    /// both kinds.
    fn index() -> Arc<dyn IndexReader> {
        let _ = register_codec("Lucene104", Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        for i in 0..200 {
            let mut doc = Document::new();
            doc.add(LatLonPoint::new("ll", coordinate(i), coordinate(i)).unwrap());
            doc.add(XYPointField::new("xy", i as f32, i as f32).unwrap());
            doc.add(LatLonDocValuesField::new("lld", coordinate(i), coordinate(i)).unwrap());
            doc.add(XYDocValuesField::new("xyd", i as f32, i as f32).unwrap());
            writer.add_document(&doc).unwrap();
        }
        writer.commit().unwrap();
        writer.close().unwrap();
        directory_reader::open(dir).unwrap()
    }

    /// Every document the iterator yields.
    fn docs(iterator: &mut dyn DocIdSetIterator) -> Vec<i32> {
        let mut docs = Vec::new();
        loop {
            let doc = iterator.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                return docs;
            }
            docs.push(doc);
        }
    }

    /// Regression for rmp #665: the scorers of `LatLonDocValuesBoxQuery`,
    /// `LatLonDocValuesQuery` and `XYDocValuesPointInGeometryQuery` are
    /// `ConstantScoreScorer`s over a `TwoPhaseIterator`, with Lucene's match
    /// costs (`LatLonDocValuesBoxQuery.java:152-154`,
    /// `LatLonDocValuesQuery.java:212-214`,
    /// `XYDocValuesPointInGeometryQuery.java:129-131`), and still match the
    /// same documents.
    #[test]
    fn doc_values_point_queries_expose_a_two_phase_iterator() {
        let reader = index();
        let searcher = IndexSearcher::new(Arc::clone(&reader)).unwrap();
        let leaves = Arc::clone(&reader).leaves().unwrap();
        assert_eq!(leaves.len(), 1);
        let rectangle = Rectangle::new(-1.55, 1.55, -1.55, 1.55).unwrap();
        let cases: Vec<(Arc<dyn Query>, f32, Vec<i32>)> = vec![
            (
                LatLonDocValuesField::new_slow_box_query(Some("lld"), -1.55, 1.55, -1.55, 1.55)
                    .unwrap(),
                5.0,
                (85..=115).collect(),
            ),
            (
                LatLonDocValuesField::new_slow_geometry_query(
                    Some("lld"),
                    QueryRelation::Within,
                    vec![LatLonGeometryValue::Rectangle(rectangle)],
                )
                .unwrap(),
                1000.0,
                (85..=115).collect(),
            ),
            (
                XYDocValuesField::new_slow_box_query(Some("xyd"), 10.5, 20.5, 10.5, 20.5).unwrap(),
                1000.0,
                (11..=20).collect(),
            ),
        ];
        for (query, match_cost, expected) in cases {
            let class = query.java_class_name();
            let weight = query
                .create_weight(&searcher, ScoreMode::COMPLETE_NO_SCORES, 1.0)
                .unwrap();
            let mut supplier = weight.scorer_supplier(&leaves[0]).unwrap().unwrap();
            let mut scorer = supplier.get(i64::MAX).unwrap();
            let two_phase = scorer
                .two_phase_iterator()
                .unwrap_or_else(|| panic!("{class}: no two-phase iterator"));
            assert_eq!(two_phase.match_cost(), match_cost, "{class}");
            assert_eq!(docs(scorer.iterator()), expected, "{class}");
        }
    }

    /// Point values that count the intersections run over them.
    struct CountingPointValues {
        inner: Box<dyn PointValues>,
        intersections: Arc<AtomicUsize>,
    }

    impl PointValues for CountingPointValues {
        fn point_tree(&self) -> Result<Box<dyn crate::index::point_values::PointTree>> {
            self.inner.point_tree()
        }

        fn intersect(&self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
            self.intersections.fetch_add(1, Ordering::SeqCst);
            self.inner.intersect(visitor)
        }

        fn size(&self) -> Result<i64> {
            self.inner.size()
        }

        fn doc_count(&self) -> Result<i32> {
            self.inner.doc_count()
        }

        fn min_packed_value(&self) -> Result<Option<Vec<u8>>> {
            self.inner.min_packed_value()
        }

        fn max_packed_value(&self) -> Result<Option<Vec<u8>>> {
            self.inner.max_packed_value()
        }

        fn num_dimensions(&self) -> Result<i32> {
            self.inner.num_dimensions()
        }

        fn num_index_dimensions(&self) -> Result<i32> {
            self.inner.num_index_dimensions()
        }

        fn bytes_per_dimension(&self) -> Result<i32> {
            self.inner.bytes_per_dimension()
        }
    }

    /// A leaf that hands out [`CountingPointValues`] over another leaf's.
    #[derive(Debug)]
    struct CountingLeafReader {
        core: IndexReaderCore,
        inner: Arc<dyn LeafReader>,
        intersections: Arc<AtomicUsize>,
        /// A field whose `FieldInfo` reports a doc-values generation, as a
        /// doc-values update would leave it.
        updated_field: Option<&'static str>,
    }

    impl CountingLeafReader {
        fn over(inner: Arc<dyn LeafReader>, updated_field: Option<&'static str>) -> Arc<Self> {
            Arc::new(Self {
                core: IndexReaderCore::new(),
                inner,
                intersections: Arc::new(AtomicUsize::new(0)),
                updated_field,
            })
        }

        fn context(self: &Arc<Self>) -> LeafReaderContext {
            LeafReaderContext::new(
                Arc::clone(self) as Arc<dyn IndexReader>,
                Arc::clone(self) as Arc<dyn LeafReader>,
                None,
                0,
                0,
                0,
                0,
            )
        }
    }

    impl std::fmt::Display for CountingLeafReader {
        /// Test double; it renders under its own name.
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("CountingLeafReader")
        }
    }

    impl LeafReader for CountingLeafReader {
        fn core(&self) -> &IndexReaderCore {
            &self.core
        }
        fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
            self.inner.term_vectors()
        }
        fn num_docs(&self) -> i32 {
            self.inner.num_docs()
        }
        fn max_doc(&self) -> i32 {
            self.inner.max_doc()
        }
        fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
            self.inner.stored_fields()
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
        fn terms(&self, field: &str) -> Result<Option<Arc<dyn Terms>>> {
            self.inner.terms(field)
        }
        fn get_numeric_doc_values(&self, f: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
            self.inner.get_numeric_doc_values(f)
        }
        fn get_binary_doc_values(&self, f: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
            self.inner.get_binary_doc_values(f)
        }
        fn get_sorted_doc_values(&self, f: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
            self.inner.get_sorted_doc_values(f)
        }
        fn get_sorted_numeric_doc_values(
            &self,
            f: &str,
        ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
            self.inner.get_sorted_numeric_doc_values(f)
        }
        fn get_sorted_set_doc_values(
            &self,
            f: &str,
        ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
            self.inner.get_sorted_set_doc_values(f)
        }
        fn get_norm_values(&self, f: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
            self.inner.get_norm_values(f)
        }
        fn get_doc_values_skipper(&self, f: &str) -> Result<Option<Box<dyn DocValuesSkipper>>> {
            self.inner.get_doc_values_skipper(f)
        }
        fn get_float_vector_values(&self, f: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
            self.inner.get_float_vector_values(f)
        }
        fn get_byte_vector_values(&self, f: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
            self.inner.get_byte_vector_values(f)
        }
        fn search_nearest_vectors(
            &self,
            f: &str,
            t: &[f32],
            c: &mut dyn KnnCollector,
            a: &mut dyn AcceptDocs,
        ) -> Result<()> {
            self.inner.search_nearest_vectors(f, t, c, a)
        }
        fn search_nearest_vectors_byte(
            &self,
            f: &str,
            t: &[u8],
            c: &mut dyn KnnCollector,
            a: &mut dyn AcceptDocs,
        ) -> Result<()> {
            self.inner.search_nearest_vectors_byte(f, t, c, a)
        }
        fn get_field_infos(&self) -> FieldInfos {
            let infos = self.inner.get_field_infos();
            let Some(updated) = self.updated_field else {
                return infos;
            };
            FieldInfos::new(
                infos
                    .iter()
                    .map(|info| {
                        let mut info = info.clone();
                        if info.name == updated {
                            info.set_doc_values_gen(1);
                        }
                        info
                    })
                    .collect(),
            )
            .unwrap()
        }
        fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
            self.inner.get_live_docs()
        }
        fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
            Ok(self.inner.get_point_values(field)?.map(|inner| {
                Box::new(CountingPointValues {
                    inner,
                    intersections: Arc::clone(&self.intersections),
                }) as Box<dyn PointValues>
            }))
        }
        fn check_integrity(&self) -> Result<()> {
            self.inner.check_integrity()
        }
        fn get_meta_data(&self) -> LeafMetaData {
            self.inner.get_meta_data()
        }
    }

    /// Asserts that `query`'s supplier walks the tree only in `get`: neither
    /// `scorerSupplier` nor `cost()` (estimated once) intersects, and the
    /// scorer matches `expected`.
    fn assert_intersects_only_in_get(query: Arc<dyn Query>, expected: Vec<i32>) {
        let reader = index();
        let searcher = IndexSearcher::new(Arc::clone(&reader)).unwrap();
        let leaves = Arc::clone(&reader).leaves().unwrap();
        assert_eq!(leaves.len(), 1);
        let class = query.java_class_name();
        let leaf = CountingLeafReader::over(leaves[0].leaf_reader(), None);
        let intersections = Arc::clone(&leaf.intersections);
        let context = leaf.context();
        let weight = query
            .create_weight(&searcher, ScoreMode::COMPLETE_NO_SCORES, 1.0)
            .unwrap();
        let mut supplier = weight.scorer_supplier(&context).unwrap().unwrap();
        assert_eq!(
            intersections.load(Ordering::SeqCst),
            0,
            "{class}: scorerSupplier"
        );
        let cost = supplier.cost();
        assert!(cost > 0, "{class}: cost {cost}");
        assert_eq!(supplier.cost(), cost, "{class}: the estimate is memoised");
        assert_eq!(intersections.load(Ordering::SeqCst), 0, "{class}: cost()");
        let mut scorer = supplier.get(i64::MAX).unwrap();
        assert_eq!(intersections.load(Ordering::SeqCst), 1, "{class}: get");
        assert_eq!(docs(scorer.iterator()), expected, "{class}");
    }

    /// Regression for rmp #666: `LatLonPointDistanceQuery` and
    /// `XYPointInGeometryQuery` intersect the tree only in
    /// `ScorerSupplier.get(leadCost)`; `scorerSupplier` and `cost()` do not,
    /// `cost()` estimating once (`LatLonPointDistanceQuery.java:141-172`,
    /// `XYPointInGeometryQuery.java:151-175`). The scorer matches the same
    /// documents.
    #[test]
    fn point_queries_intersect_only_when_the_scorer_is_built() {
        // Documents 85..=115 lie within about 236 km of the origin, document
        // 84 and 116 about 252 km away.
        assert_intersects_only_in_get(
            Arc::new(LatLonPointDistanceQuery::new("ll", 0.0, 0.0, 244_000.0).unwrap()),
            (85..=115).collect(),
        );
        let xy_rectangle = XYRectangle::new(10.5, 20.5, 10.5, 20.5).unwrap();
        assert_intersects_only_in_get(
            Arc::new(
                XYPointInGeometryQuery::new("xy", vec![XYGeometryValue::Rectangle(xy_rectangle)])
                    .unwrap(),
            ),
            (11..=20).collect(),
        );
    }

    /// Regression for rmp #666 (round 2): a `SpatialQuery` subclass, here
    /// `LatLonPointQuery`, gets `SpatialQuery.getScorerSupplier`'s supplier,
    /// which walks the tree only in `get` and estimates `cost()` once
    /// (`SpatialQuery.java:150-206, 383-391`).
    #[test]
    fn spatial_queries_intersect_only_when_the_scorer_is_built() {
        let rectangle = Rectangle::new(-1.55, 1.55, -1.55, 1.55).unwrap();
        assert_intersects_only_in_get(
            Arc::new(
                LatLonPointQuery::new(
                    "ll",
                    QueryRelation::Intersects,
                    vec![LatLonGeometryValue::Rectangle(rectangle)],
                )
                .unwrap(),
            ),
            (85..=115).collect(),
        );
    }

    /// Regression for rmp #665 (round 2): the weights of the three doc-values
    /// point queries answer `isCacheable` with `DocValues.isCacheable(ctx,
    /// field)` (`LatLonDocValuesBoxQuery.java:160-163`,
    /// `LatLonDocValuesQuery.java:165-168`,
    /// `XYDocValuesPointInGeometryQuery.java:137-140`): not cacheable once
    /// the field carries doc-values updates.
    #[test]
    fn doc_values_point_queries_are_not_cacheable_over_updated_doc_values() {
        let reader = index();
        let searcher = IndexSearcher::new(Arc::clone(&reader)).unwrap();
        let leaves = Arc::clone(&reader).leaves().unwrap();
        let rectangle = Rectangle::new(-1.55, 1.55, -1.55, 1.55).unwrap();
        let cases: Vec<(Arc<dyn Query>, &'static str)> = vec![
            (
                LatLonDocValuesField::new_slow_box_query(Some("lld"), -1.55, 1.55, -1.55, 1.55)
                    .unwrap(),
                "lld",
            ),
            (
                LatLonDocValuesField::new_slow_geometry_query(
                    Some("lld"),
                    QueryRelation::Within,
                    vec![LatLonGeometryValue::Rectangle(rectangle)],
                )
                .unwrap(),
                "lld",
            ),
            (
                XYDocValuesField::new_slow_box_query(Some("xyd"), 10.5, 20.5, 10.5, 20.5).unwrap(),
                "xyd",
            ),
        ];
        for (query, field) in cases {
            let class = query.java_class_name();
            let weight = query
                .create_weight(&searcher, ScoreMode::COMPLETE_NO_SCORES, 1.0)
                .unwrap();
            assert!(weight.is_cacheable(&leaves[0]), "{class}: no update");
            let other_field = if field == "lld" { "xyd" } else { "lld" };
            let other = CountingLeafReader::over(leaves[0].leaf_reader(), Some(other_field));
            assert!(
                weight.is_cacheable(&other.context()),
                "{class}: other field updated"
            );
            let updated = CountingLeafReader::over(leaves[0].leaf_reader(), Some(field));
            assert!(
                !weight.is_cacheable(&updated.context()),
                "{class}: field updated"
            );
        }
    }
}
