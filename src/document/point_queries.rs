//! The queries over indexed points and over location doc values, ported from
//! `org.apache.lucene.document`.
//!
//! [`LatLonPointQuery`] and [`XYPointInGeometryQuery`] read the BKD tree a
//! [`LatLonPoint`](crate::document::LatLonPoint) or an
//! [`XYPointField`](crate::document::XYPointField) writes; the three
//! doc-values queries scan the packed longs a
//! [`LatLonDocValuesField`](crate::document::LatLonDocValuesField) or an
//! [`XYDocValuesField`](crate::document::XYDocValuesField) writes instead.
//!
//! # Divergence from Lucene 10.5.0: the query hierarchy
//!
//! As in [`spatial_query`](crate::document::spatial_query), these types stop
//! one step short of `Query`/`Weight`/`Scorer` and expose the
//! [`DocIdSetIterator`] a `ConstantScoreScorer` would have wrapped. The three
//! doc-values queries additionally collapse Java's `TwoPhaseIterator` into a
//! single filtering iterator: the same documents are produced, in the same
//! order, but a consumer cannot defer the per-document check.

use std::sync::Arc;

use crate::document::geo_fields::{LatLonPoint, XYPointField};
use crate::document::shape_field::QueryRelation;
use crate::document::spatial_query::{
    LatLonGeometryValue, SpatialQuery, SpatialVisitor, XYGeometryValue,
};
use crate::error::{LuceneError, Result};
use crate::geo::component2d::WithinRelation;
use crate::geo::encoding::{GeoEncodingUtils, GeoUtils, XYEncodingUtils};
use crate::geo::Component2D;
use crate::index::point_values::{IntersectVisitor, Relation};
use crate::index::{LeafReader, SortedNumericDocValues};
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
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
/// **Divergence from Lucene 10.5.0.** Java precomputes a
/// `GeoEncodingUtils.Component2DPredicate`, a grid of per-sub-box relations
/// that answers `contains` for most encoded points without touching the
/// geometry. That accelerator lives in `org.apache.lucene.geo` and is not
/// ported yet, so the predicate calls
/// [`Component2D::contains`] directly. The grid is exact — a sub-box marked
/// inside or outside decides every quantised point it holds, and a crossing one
/// falls through to the very same `contains` call — so the answer is
/// unchanged; only the speed is.
#[derive(Clone)]
pub struct LatLonPointSpatialVisitor {
    component2d: Arc<dyn Component2D>,
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
            component2d,
        })
    }

    /// The body of the `Component2DPredicate.test(int, int)` Java precomputes;
    /// see the divergence note on the type.
    fn test(&self, lat: i32, lon: i32) -> bool {
        self.component2d.contains(
            GeoEncodingUtils::decode_longitude(lon),
            GeoEncodingUtils::decode_latitude(lat),
        )
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
/// [`LatLonPoint`](crate::document::LatLonPoint) per document.
#[derive(Clone, Debug)]
pub struct LatLonPointQuery {
    query: SpatialQuery,
    geometries: Vec<LatLonGeometryValue>,
    visitor: LatLonPointSpatialVisitor,
}

impl LatLonPointQuery {
    /// Creates the query.
    ///
    /// Equivalent to
    /// `LatLonPointQuery(String, QueryRelation, LatLonGeometry...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for a `WITHIN` query over a
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
                return Err(LuceneError::IllegalArgument(
                    "LatLonPointQuery does not support Within queries with line geometries"
                        .to_string(),
                ));
            }
        }
    }
    if query_relation == QueryRelation::Contains {
        for geometry in geometries {
            if !matches!(geometry, LatLonGeometryValue::Point(_)) {
                return Err(LuceneError::IllegalArgument(
                    "LatLonPointQuery does not support Contains queries with non-points geometries"
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

/// Filters a `SortedNumericDocValues` iterator down to the matching documents.
///
/// See the module divergence note.
struct SortedNumericMatchIterator {
    values: Box<dyn SortedNumericDocValues>,
    matcher: Box<dyn SortedNumericMatcher>,
}

impl SortedNumericMatchIterator {
    fn confirm(&mut self, mut doc: i32) -> Result<i32> {
        while doc != NO_MORE_DOCS {
            if self.matcher.matches(self.values.as_mut())? {
                return Ok(doc);
            }
            doc = self.values.next_doc()?;
        }
        Ok(NO_MORE_DOCS)
    }
}

impl DocIdSetIterator for SortedNumericMatchIterator {
    fn doc_id(&self) -> i32 {
        self.values.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.values.next_doc()?;
        self.confirm(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.values.advance(target)?;
        self.confirm(doc)
    }

    fn cost(&self) -> i64 {
        self.values.cost()
    }
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
            QueryRelation::Disjoint => {
                for _ in 0..count {
                    let (lat, lon) = split_packed(values.next_value()?);
                    if self.test(lat, lon) {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
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
/// [`LatLonDocValuesField`](crate::document::LatLonDocValuesField) per
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
    /// Returns [`LuceneError::IllegalArgument`] for a `WITHIN` query over a
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
                    return Err(LuceneError::IllegalArgument(
                        "LatLonDocValuesPointQuery does not support Within queries with line \
                         geometries"
                            .to_string(),
                    ));
                }
            }
        }
        if query_relation == QueryRelation::Contains {
            for geometry in &geometries {
                if !matches!(geometry, LatLonGeometryValue::Point(_)) {
                    return Err(LuceneError::IllegalArgument(
                        "LatLonDocValuesPointQuery does not support Contains queries with \
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

    /// Returns the documents of `reader` this query matches.
    ///
    /// Equivalent to the `ScorerSupplier` of the `ConstantScoreWeight`
    /// `LatLonDocValuesQuery.createWeight` builds; see the module divergence
    /// note.
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
        let mut contains_components = Vec::new();
        if self.query_relation == QueryRelation::Contains {
            for geometry in &self.geometries {
                contains_components.push(geometry.to_component2d()?);
            }
        }
        Ok(Some(Box::new(SortedNumericMatchIterator {
            values,
            matcher: Box::new(LatLonDocValuesMatcher {
                query_relation: self.query_relation,
                component2d: Arc::clone(&self.component2d),
                contains_components,
            }),
        })))
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
    /// Returns [`LuceneError::IllegalArgument`] for a coordinate outside its
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

    /// Returns the documents of `reader` this query matches.
    ///
    /// See the module divergence note.
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
        Ok(Some(Box::new(SortedNumericMatchIterator {
            values,
            matcher: Box::new(LatLonBoxMatcher {
                min_latitude: self.matcher_min_latitude,
                max_latitude: self.matcher_max_latitude,
                min_longitude: self.matcher_min_longitude,
                max_longitude: self.matcher_max_longitude,
                crosses_dateline: self.crosses_dateline,
            }),
        })))
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
/// [`XYDocValuesField`](crate::document::XYDocValuesField) per document.
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
    /// Returns [`LuceneError::IllegalArgument`] for an empty geometry list.
    pub fn new(field: impl Into<String>, geometries: Vec<XYGeometryValue>) -> Result<Self> {
        if geometries.is_empty() {
            return Err(LuceneError::IllegalArgument(
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

    /// Returns the documents of `reader` this query matches.
    ///
    /// See the module divergence note.
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
        Ok(Some(Box::new(SortedNumericMatchIterator {
            values,
            matcher: Box::new(XYDocValuesMatcher {
                component2d: Arc::clone(&self.component2d),
            }),
        })))
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

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Relation {
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
        self.tree
            .relate(cell_min_x, cell_max_x, cell_min_y, cell_max_y)
    }
}

/// Finds every previously indexed point that falls within the specified
/// cartesian geometries.
///
/// Equivalent to `org.apache.lucene.document.XYPointInGeometryQuery`. The field
/// must be indexed with an [`XYPointField`](crate::document::XYPointField) per
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
    /// Returns [`LuceneError::IllegalArgument`] for an empty geometry list.
    pub fn new(field: impl Into<String>, xy_geometries: Vec<XYGeometryValue>) -> Result<Self> {
        if xy_geometries.is_empty() {
            return Err(LuceneError::IllegalArgument(
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

    /// Returns the documents of `reader` this query matches.
    ///
    /// Equivalent to the `ScorerSupplier` of the `ConstantScoreWeight`
    /// `XYPointInGeometryQuery.createWeight` builds; see the module divergence
    /// note.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the field was indexed with
    /// an incompatible point type, and propagates whatever the point tree
    /// raises.
    pub fn matching_docs(
        &self,
        reader: &dyn LeafReader,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        let Some(values) = reader.get_point_values(&self.field)? else {
            // No document in this segment has any point for this field.
            return Ok(None);
        };
        let field_infos = reader.get_field_infos();
        let Some(field_info) = field_infos.field_info(&self.field) else {
            // No document in this segment indexed this field at all.
            return Ok(None);
        };
        XYPointField::check_compatible(field_info)?;

        let mut result = DocIdSetBuilder::from_point_values(reader.max_doc(), values.as_ref());
        {
            let mut visitor = XYPointIntersectVisitor {
                result: &mut result,
                tree: self.tree.as_ref(),
            };
            values.intersect(&mut visitor)?;
        }
        Ok(Some(result.build()?.iterator()?))
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
        let mut result = DocIdSetBuilder::from_point_values(reader.max_doc(), values.as_ref());
        let mut visitor = XYPointIntersectVisitor {
            result: &mut result,
            tree: self.tree.as_ref(),
        };
        values.estimate_doc_count(&mut visitor)
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

impl PartialEq for XYPointInGeometryQuery {
    /// Equivalent to the private `XYPointInGeometryQuery.equalsTo`.
    fn eq(&self, other: &Self) -> bool {
        self.field == other.field && self.xy_geometries == other.xy_geometries
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
///
/// **Divergence from Lucene 10.5.0.** Java precomputes a
/// `GeoEncodingUtils.DistancePredicate`, a grid of per-sub-box relations that
/// answers most encoded points without a haversine call. That accelerator lives
/// in `org.apache.lucene.geo` and is not ported yet, so the predicate computes
/// [`SloppyMath::haversin_sort_key`] directly and compares it against the same
/// sort key — which is exactly what the grid falls through to for a crossing
/// sub-box, and what an inside/outside sub-box decides in advance. The answer
/// is unchanged; only the speed is.
#[derive(Clone, Copy, Debug)]
struct LatLonDistanceBounds {
    latitude: f64,
    longitude: f64,
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
        })
    }

    /// The `DistancePredicate.test(int, int)` Java precomputes; see the
    /// divergence note on the type.
    fn test(&self, lat: i32, lon: i32) -> bool {
        crate::util::sloppy_math::SloppyMath::haversin_sort_key(
            GeoEncodingUtils::decode_latitude(lat),
            GeoEncodingUtils::decode_longitude(lon),
            self.latitude,
            self.longitude,
        ) <= self.sort_key
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
    fn grow(&mut self, count: i32) {
        self.result.grow(count);
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

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Relation {
        self.bounds
            .relate(min_packed_value, max_packed_value)
            .unwrap_or(Relation::CellCrossesQuery)
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

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Relation {
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
        }
    }
}

/// Finds every previously indexed geographic point within a radius of an
/// origin.
///
/// Equivalent to `org.apache.lucene.document.LatLonPointDistanceQuery`, which
/// [`LatLonPoint::new_distance_query`](crate::document::LatLonPoint) builds.
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
    /// Returns [`LuceneError::IllegalArgument`] for a non-finite or negative
    /// radius, and for an invalid latitude or longitude.
    pub fn new(
        field: impl Into<String>,
        latitude: f64,
        longitude: f64,
        radius_meters: f64,
    ) -> Result<Self> {
        if !radius_meters.is_finite() || radius_meters < 0.0 {
            return Err(LuceneError::IllegalArgument(format!(
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

    /// Returns the documents of `reader` this query matches.
    ///
    /// Equivalent to the `ScorerSupplier.get(long)` of the weight
    /// `LatLonPointDistanceQuery.createWeight` builds; see the module
    /// divergence note.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the field was indexed with
    /// an incompatible point type, and propagates whatever the point tree
    /// raises.
    pub fn matching_docs(
        &self,
        reader: &dyn LeafReader,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        let Some(values) = reader.get_point_values(&self.field)? else {
            // No document in this segment has any point for this field.
            return Ok(None);
        };
        let field_infos = reader.get_field_infos();
        let Some(field_info) = field_infos.field_info(&self.field) else {
            // No document in this segment indexed this field at all.
            return Ok(None);
        };
        LatLonPoint::check_compatible(field_info)?;

        let max_doc = reader.max_doc();
        if values.doc_count() == max_doc
            && i64::from(values.doc_count()) == values.size()
            && self.cost(reader)? > i64::from(max_doc) / 2
        {
            // Every document has exactly one value and the cost is more than
            // half the leaf, so computing the complement may be faster.
            let mut result = crate::util::FixedBitSet::new(max_doc as usize);
            result.set_range(0, max_doc as usize);
            let mut cost = i64::from(max_doc);
            let mut inverse = InverseDistanceIntersectVisitor {
                bounds: self.bounds,
                result: &mut result,
                cost: &mut cost,
            };
            values.intersect(&mut inverse)?;
            return Ok(Some(Box::new(crate::util::BitSetIterator::new(
                Arc::new(result),
                cost,
            )?)));
        }

        let mut result = DocIdSetBuilder::from_point_values(max_doc, values.as_ref());
        {
            let mut visitor = DistanceIntersectVisitor {
                bounds: self.bounds,
                result: &mut result,
            };
            values.intersect(&mut visitor)?;
        }
        Ok(Some(result.build()?.iterator()?))
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
        let mut result = DocIdSetBuilder::from_point_values(reader.max_doc(), values.as_ref());
        let mut visitor = DistanceIntersectVisitor {
            bounds: self.bounds,
            result: &mut result,
        };
        values.estimate_doc_count(&mut visitor)
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
        sb.push_str(&format!(
            "{},{} +/- {} meters",
            self.latitude, self.longitude, self.radius_meters
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
