//! The base of every spatial query, ported from
//! `org.apache.lucene.document.SpatialQuery` and its subclasses.
//!
//! A shape is indexed as a set of triangles packed into seven-dimension points
//! whose first four dimensions are the triangle's bounding box, so a BKD tree
//! can prune whole subtrees. This module walks that tree: a
//! [`SpatialVisitor`] answers the three questions the walk asks — how a cell
//! relates to the query, whether one indexed triangle intersects it, and
//! whether it is within or contains it — and [`SpatialQuery`] turns those
//! answers into the set of matching documents, choosing between a sparse and a
//! dense strategy exactly as Lucene does.
//!
//! # Divergence from Lucene 10.5.0: the query hierarchy
//!
//! Java's `SpatialQuery` extends `org.apache.lucene.search.Query` and produces
//! a `Weight` whose `ScorerSupplier` builds a `ConstantScoreScorer` over the
//! iterator computed here. The `Query`/`Weight`/`Scorer` hierarchy is not part
//! of this crate's public search surface yet, so these types stop one step
//! earlier and expose the [`DocIdSetIterator`] the scorer would have wrapped,
//! through [`SpatialQuery::matching_docs`]. Every decision that selects
//! documents — the relation, the predicates, the sparse/dense choice, the
//! adversarial `hasAnyHits` short-circuit — is ported unchanged; only the
//! constant-score wrapper is missing.

use std::sync::Arc;

use crate::document::shape_doc_values::{ShapeCoordinateSystem, ShapeDocValues};
use crate::document::shape_field::{QueryRelation, ShapeField, BYTES};
use crate::error::{LuceneError, Result};
use crate::geo::component2d::WithinRelation;
use crate::geo::encoding::{GeoEncodingUtils, GeoUtils, XYEncodingUtils};
use crate::geo::geometry::{
    Circle, Line, Point, Polygon, Rectangle, XYCircle, XYLine, XYPoint, XYPolygon, XYRectangle,
};
use crate::geo::Component2D;
use crate::index::point_values::Relation;
use crate::index::{LeafReader, PointValues};
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
use crate::util::doc_id_set::DocIdSetBuilder;
use crate::util::{BitSet, BitSetIterator, FixedBitSet, IntsRef, NumericUtils};

// -----------------------------------------------------------------------------
// Geometry unions
// -----------------------------------------------------------------------------

/// A geometry a geographic spatial query can be built from.
///
/// Equivalent to `org.apache.lucene.geo.LatLonGeometry`, of which Lucene core
/// declares exactly five implementations.
///
/// **Divergence from Lucene 10.5.0.** Java stores `Geometry[]` in
/// `SpatialQuery` and relies on `equals`, `hashCode` and `toString` of the
/// concrete geometry, plus an `instanceof Rectangle` test in the query
/// factories. A `Vec<Arc<dyn LatLonGeometry>>` could carry none of those, so
/// this port names the five concrete types as an enum. The set is the same, the
/// component built from it is the same, and the `instanceof` test becomes a
/// pattern match.
#[derive(Clone, Debug, PartialEq)]
pub enum LatLonGeometryValue {
    /// A point.
    Point(Point),
    /// An axis-aligned bounding box, which may cross the dateline.
    Rectangle(Rectangle),
    /// A linestring.
    Line(Line),
    /// A polygon, possibly with holes.
    Polygon(Polygon),
    /// A circle of a radius in metres.
    Circle(Circle),
}

impl LatLonGeometryValue {
    /// Returns the [`Component2D`] this geometry relates against.
    ///
    /// Equivalent to `Geometry.toComponent2D()`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the geometry's component factory raises.
    pub fn to_component2d(&self) -> Result<Arc<dyn Component2D>> {
        use crate::geo::geometry::Geometry;
        match self {
            Self::Point(g) => g.to_component2d(),
            Self::Rectangle(g) => g.to_component2d(),
            Self::Line(g) => g.to_component2d(),
            Self::Polygon(g) => g.to_component2d(),
            Self::Circle(g) => g.to_component2d(),
        }
    }

    /// Creates a [`Component2D`] from several geometries.
    ///
    /// Equivalent to the static `LatLonGeometry.create(LatLonGeometry...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `geometries` is empty.
    pub fn create(geometries: &[Self]) -> Result<Arc<dyn Component2D>> {
        if geometries.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "geometries must not be empty".to_string(),
            ));
        }
        if geometries.len() == 1 {
            return geometries[0].to_component2d();
        }
        let mut components = Vec::with_capacity(geometries.len());
        for g in geometries {
            components.push(g.to_component2d()?);
        }
        Ok(crate::geo::ComponentTree::create(components))
    }
}

/// A geometry a cartesian spatial query can be built from.
///
/// Equivalent to `org.apache.lucene.geo.XYGeometry`; see the divergence note on
/// [`LatLonGeometryValue`].
#[derive(Clone, Debug, PartialEq)]
pub enum XYGeometryValue {
    /// A point.
    Point(XYPoint),
    /// An axis-aligned bounding box.
    Rectangle(XYRectangle),
    /// A linestring.
    Line(XYLine),
    /// A polygon, possibly with holes.
    Polygon(XYPolygon),
    /// A circle.
    Circle(XYCircle),
}

impl XYGeometryValue {
    /// Returns the [`Component2D`] this geometry relates against.
    ///
    /// Equivalent to `Geometry.toComponent2D()`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the geometry's component factory raises.
    pub fn to_component2d(&self) -> Result<Arc<dyn Component2D>> {
        use crate::geo::geometry::Geometry;
        match self {
            Self::Point(g) => g.to_component2d(),
            Self::Rectangle(g) => g.to_component2d(),
            Self::Line(g) => g.to_component2d(),
            Self::Polygon(g) => g.to_component2d(),
            Self::Circle(g) => g.to_component2d(),
        }
    }

    /// Creates a [`Component2D`] from several geometries.
    ///
    /// Equivalent to the static `XYGeometry.create(XYGeometry...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `geometries` is empty.
    pub fn create(geometries: &[Self]) -> Result<Arc<dyn Component2D>> {
        if geometries.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "geometries must not be empty".to_string(),
            ));
        }
        if geometries.len() == 1 {
            return geometries[0].to_component2d();
        }
        let mut components = Vec::with_capacity(geometries.len());
        for g in geometries {
            components.push(g.to_component2d()?);
        }
        Ok(crate::geo::ComponentTree::create(components))
    }
}

// -----------------------------------------------------------------------------
// Relations
// -----------------------------------------------------------------------------

/// Transposes a relation: inside becomes outside, outside becomes inside, and
/// crosses is unchanged.
///
/// Equivalent to the static `SpatialQuery.transposeRelation(Relation)`.
pub fn transpose_relation(r: Relation) -> Relation {
    match r {
        Relation::CellInsideQuery => Relation::CellOutsideQuery,
        Relation::CellOutsideQuery => Relation::CellInsideQuery,
        Relation::CellCrossesQuery => Relation::CellCrossesQuery,
    }
}

/// Answers the three questions a BKD walk asks of a spatial query.
///
/// Equivalent to the abstract static nested class
/// `SpatialQuery.SpatialVisitor`.
///
/// **Divergence from Lucene 10.5.0.** Java's `intersects()`, `within()` and
/// `contains()` each *return a closure* that captures a reusable
/// `DecodedTriangle` scratch instance. Decoding returns a value type here, so
/// no scratch is needed and the three become plain methods taking the packed
/// triangle.
pub trait SpatialVisitor {
    /// Relates a range of points — an internal node — to the query.
    ///
    /// Equivalent to `SpatialVisitor.relate(byte[], byte[])`.
    ///
    /// # Errors
    ///
    /// Propagates a decoding failure.
    fn relate(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Result<Relation>;

    /// Returns whether the indexed triangle intersects the query.
    ///
    /// Equivalent to the predicate `SpatialVisitor.intersects()` returns.
    ///
    /// # Errors
    ///
    /// Propagates a decoding failure.
    fn intersects(&self, triangle: &[u8]) -> Result<bool>;

    /// Returns whether the indexed triangle lies within the query.
    ///
    /// Equivalent to the predicate `SpatialVisitor.within()` returns.
    ///
    /// # Errors
    ///
    /// Propagates a decoding failure.
    fn within(&self, triangle: &[u8]) -> Result<bool>;

    /// Returns how the query relates to the indexed triangle, for the
    /// `CONTAINS` relation.
    ///
    /// Equivalent to the function `SpatialVisitor.contains()` returns.
    ///
    /// # Errors
    ///
    /// Propagates a decoding failure, and returns
    /// [`LuceneError::IllegalArgument`] when the query cannot answer a contains
    /// question at all — which is what
    /// `LatLonShapeBoundingBoxQuery.contains()` throws for a rectangle crossing
    /// the dateline.
    fn contains(&self, triangle: &[u8]) -> Result<WithinRelation>;

    /// Returns whether the triangle makes the document a `CONTAINS` candidate.
    ///
    /// Equivalent to the private `SpatialVisitor.containsPredicate()`.
    ///
    /// # Errors
    ///
    /// As [`Self::contains`].
    fn contains_predicate(&self, triangle: &[u8]) -> Result<bool> {
        Ok(self.contains(triangle)? == WithinRelation::Candidate)
    }

    /// Relates an internal node, transposing the relation for a `DISJOINT`
    /// query.
    ///
    /// Equivalent to the private `SpatialVisitor.getInnerFunction(QueryRelation)`.
    ///
    /// # Errors
    ///
    /// As [`Self::relate`].
    fn inner_function(
        &self,
        query_relation: QueryRelation,
        min_packed_value: &[u8],
        max_packed_value: &[u8],
    ) -> Result<Relation> {
        let relation = self.relate(min_packed_value, max_packed_value)?;
        Ok(if query_relation == QueryRelation::Disjoint {
            transpose_relation(relation)
        } else {
            relation
        })
    }

    /// Tests one indexed triangle under the query's relation.
    ///
    /// Equivalent to the private `SpatialVisitor.getLeafPredicate(QueryRelation)`.
    ///
    /// # Errors
    ///
    /// Propagates a decoding failure.
    fn leaf_predicate(&self, query_relation: QueryRelation, triangle: &[u8]) -> Result<bool> {
        match query_relation {
            QueryRelation::Intersects => self.intersects(triangle),
            QueryRelation::Within => self.within(triangle),
            QueryRelation::Disjoint => Ok(!self.intersects(triangle)?),
            QueryRelation::Contains => self.contains_predicate(triangle),
        }
    }
}

// -----------------------------------------------------------------------------
// EncodedRectangle
// -----------------------------------------------------------------------------

/// Spatial logic for a bounding box that works entirely in the encoded integer
/// space, so a query never has to decode an indexed triangle.
///
/// Equivalent to the public static nested class
/// `SpatialQuery.EncodedRectangle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncodedRectangle {
    /// Smallest encoded x.
    pub min_x: i32,
    /// Largest encoded x.
    pub max_x: i32,
    /// Smallest encoded y.
    pub min_y: i32,
    /// Largest encoded y.
    pub max_y: i32,
    /// Whether the box wraps the coordinate system, which for a geographic box
    /// means crossing the dateline.
    pub wraps_coordinate_system: bool,
}

impl EncodedRectangle {
    /// Creates the box.
    ///
    /// Equivalent to
    /// `EncodedRectangle(int, int, int, int, boolean)`.
    pub fn new(
        min_x: i32,
        max_x: i32,
        min_y: i32,
        max_y: i32,
        wraps_coordinate_system: bool,
    ) -> Self {
        Self {
            min_x,
            max_x,
            min_y,
            max_y,
            wraps_coordinate_system,
        }
    }

    /// Returns whether the box wraps the coordinate system.
    ///
    /// Equivalent to `EncodedRectangle.wrapsCoordinateSystem()`.
    pub fn wraps_coordinate_system(&self) -> bool {
        self.wraps_coordinate_system
    }

    /// Returns whether the box contains the point.
    ///
    /// Equivalent to `EncodedRectangle.contains(int, int)`.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        if y < self.min_y || y > self.max_y {
            return false;
        }
        if self.wraps_coordinate_system() {
            !(x > self.max_x && x < self.min_x)
        } else {
            !(x > self.max_x || x < self.min_x)
        }
    }

    /// Returns whether the box intersects the line.
    ///
    /// Equivalent to `EncodedRectangle.intersectsLine(int, int, int, int)`.
    pub fn intersects_line(&self, a_x: i32, a_y: i32, b_x: i32, b_y: i32) -> bool {
        if self.contains(a_x, a_y) || self.contains(b_x, b_y) {
            return true;
        }
        // Check that the bounding boxes are not disjoint.
        if a_y.max(b_y) < self.min_y || a_y.min(b_y) > self.max_y {
            return false;
        }
        if self.wraps_coordinate_system {
            // Crosses the dateline.
            if a_x.min(b_x) > self.max_x && a_x.max(b_x) < self.min_x {
                return false;
            }
        } else if a_x.min(b_x) > self.max_x || a_x.max(b_x) < self.min_x {
            return false;
        }
        // The expensive part.
        self.edge_intersects_query(a_x, a_y, b_x, b_y)
    }

    /// Returns whether the box intersects the triangle.
    ///
    /// Equivalent to `EncodedRectangle.intersectsTriangle(...)`.
    pub fn intersects_triangle(
        &self,
        a_x: i32,
        a_y: i32,
        b_x: i32,
        b_y: i32,
        c_x: i32,
        c_y: i32,
    ) -> bool {
        // The query contains one of the triangle's vertices.
        if self.contains(a_x, a_y) || self.contains(b_x, b_y) || self.contains(c_x, c_y) {
            return true;
        }
        let t_min_y = a_y.min(b_y).min(c_y);
        let t_max_y = a_y.max(b_y).max(c_y);
        if t_max_y < self.min_y || t_min_y > self.max_y {
            return false;
        }
        let t_min_x = a_x.min(b_x).min(c_x);
        let t_max_x = a_x.max(b_x).max(c_x);
        if self.wraps_coordinate_system {
            if t_min_x > self.max_x && t_max_x < self.min_x {
                return false;
            }
        } else if t_min_x > self.max_x || t_max_x < self.min_x {
            return false;
        }
        // The expensive part.
        crate::geo::component2d::point_in_triangle(
            f64::from(t_min_x),
            f64::from(t_max_x),
            f64::from(t_min_y),
            f64::from(t_max_y),
            f64::from(self.min_x),
            f64::from(self.min_y),
            f64::from(a_x),
            f64::from(a_y),
            f64::from(b_x),
            f64::from(b_y),
            f64::from(c_x),
            f64::from(c_y),
        ) || self.edge_intersects_query(a_x, a_y, b_x, b_y)
            || self.edge_intersects_query(b_x, b_y, c_x, c_y)
            || self.edge_intersects_query(c_x, c_y, a_x, a_y)
    }

    /// Returns whether the box intersects the rectangle.
    ///
    /// Equivalent to `EncodedRectangle.intersectsRectangle(int, int, int, int)`.
    pub fn intersects_rectangle(&self, min_x: i32, max_x: i32, min_y: i32, max_y: i32) -> bool {
        // A simple check on y.
        if self.min_y > max_y || self.max_y < min_y {
            return false;
        }
        if self.min_x <= max_x {
            // The triangle's minimum x is less than the query's maximum x, so
            // they intersect when the query box wraps — the western box — or
            // when the triangle's maximum x is greater than the query's
            // minimum x.
            if self.wraps_coordinate_system || self.max_x >= min_x {
                return true;
            }
        }
        self.wraps_coordinate_system
    }

    /// Returns whether the box contains the rectangle.
    ///
    /// Equivalent to `EncodedRectangle.containsRectangle(int, int, int, int)`.
    pub fn contains_rectangle(&self, min_x: i32, max_x: i32, min_y: i32, max_y: i32) -> bool {
        self.min_x <= min_x && self.max_x >= max_x && self.min_y <= min_y && self.max_y >= max_y
    }

    /// Returns whether the box contains the line.
    ///
    /// Equivalent to `EncodedRectangle.containsLine(int, int, int, int)`.
    pub fn contains_line(&self, a_x: i32, a_y: i32, b_x: i32, b_y: i32) -> bool {
        if a_y < self.min_y || b_y < self.min_y || a_y > self.max_y || b_y > self.max_y {
            return false;
        }
        if self.wraps_coordinate_system {
            (a_x >= self.min_x && b_x >= self.min_x) || (a_x <= self.max_x && b_x <= self.max_x)
        } else {
            a_x >= self.min_x && b_x >= self.min_x && a_x <= self.max_x && b_x <= self.max_x
        }
    }

    /// Returns whether the box contains the triangle.
    ///
    /// Equivalent to `EncodedRectangle.containsTriangle(...)`.
    pub fn contains_triangle(
        &self,
        a_x: i32,
        a_y: i32,
        b_x: i32,
        b_y: i32,
        c_x: i32,
        c_y: i32,
    ) -> bool {
        if a_y < self.min_y
            || b_y < self.min_y
            || c_y < self.min_y
            || a_y > self.max_y
            || b_y > self.max_y
            || c_y > self.max_y
        {
            return false;
        }
        if self.wraps_coordinate_system {
            (a_x >= self.min_x && b_x >= self.min_x && c_x >= self.min_x)
                || (a_x <= self.max_x && b_x <= self.max_x && c_x <= self.max_x)
        } else {
            a_x >= self.min_x
                && b_x >= self.min_x
                && c_x >= self.min_x
                && a_x <= self.max_x
                && b_x <= self.max_x
                && c_x <= self.max_x
        }
    }

    /// Returns the within relation of the box to the line.
    ///
    /// Equivalent to `EncodedRectangle.withinLine(int, int, boolean, int, int)`.
    pub fn within_line(&self, a_x: i32, a_y: i32, ab: bool, b_x: i32, b_y: i32) -> WithinRelation {
        if self.contains(a_x, a_y) || self.contains(b_x, b_y) {
            return WithinRelation::NotWithin;
        }
        if ab
            && edge_intersects_box(
                a_x, a_y, b_x, b_y, self.min_x, self.max_x, self.min_y, self.max_y,
            )
        {
            return WithinRelation::NotWithin;
        }
        WithinRelation::Disjoint
    }

    /// Returns the within relation of the box to the triangle.
    ///
    /// Equivalent to `EncodedRectangle.withinTriangle(...)`.
    #[allow(clippy::too_many_arguments)]
    pub fn within_triangle(
        &self,
        a_x: i32,
        a_y: i32,
        ab: bool,
        b_x: i32,
        b_y: i32,
        bc: bool,
        c_x: i32,
        c_y: i32,
        ca: bool,
    ) -> WithinRelation {
        // The vertices belong to the shape, so a vertex inside the rectangle
        // rules out "within".
        if self.contains(a_x, a_y) || self.contains(b_x, b_y) || self.contains(c_x, c_y) {
            return WithinRelation::NotWithin;
        }

        let t_min_y = a_y.min(b_y).min(c_y);
        let t_max_y = a_y.max(b_y).max(c_y);
        if t_max_y < self.min_y || t_min_y > self.max_y {
            return WithinRelation::Disjoint;
        }
        let t_min_x = a_x.min(b_x).min(c_x);
        let t_max_x = a_x.max(b_x).max(c_x);
        if self.wraps_coordinate_system {
            if t_min_x > self.max_x && t_max_x < self.min_x {
                return WithinRelation::Disjoint;
            }
        } else if t_min_x > self.max_x || t_max_x < self.min_x {
            return WithinRelation::Disjoint;
        }

        // Intersecting an edge that belongs to the shape rules out "within".
        let mut relation = WithinRelation::Disjoint;
        if edge_intersects_box(
            a_x, a_y, b_x, b_y, self.min_x, self.max_x, self.min_y, self.max_y,
        ) {
            if ab {
                return WithinRelation::NotWithin;
            }
            relation = WithinRelation::Candidate;
        }
        if edge_intersects_box(
            b_x, b_y, c_x, c_y, self.min_x, self.max_x, self.min_y, self.max_y,
        ) {
            if bc {
                return WithinRelation::NotWithin;
            }
            relation = WithinRelation::Candidate;
        }
        if edge_intersects_box(
            c_x, c_y, a_x, a_y, self.min_x, self.max_x, self.min_y, self.max_y,
        ) {
            if ca {
                return WithinRelation::NotWithin;
            }
            relation = WithinRelation::Candidate;
        }
        // Is the shape within the triangle?
        if relation == WithinRelation::Candidate
            || crate::geo::component2d::point_in_triangle(
                f64::from(t_min_x),
                f64::from(t_max_x),
                f64::from(t_min_y),
                f64::from(t_max_y),
                f64::from(self.min_x),
                f64::from(self.min_y),
                f64::from(a_x),
                f64::from(a_y),
                f64::from(b_x),
                f64::from(b_y),
                f64::from(c_x),
                f64::from(c_y),
            )
        {
            return WithinRelation::Candidate;
        }
        relation
    }

    /// Returns whether the edge intersects the query box.
    ///
    /// Equivalent to the private `EncodedRectangle.edgeIntersectsQuery(...)`.
    fn edge_intersects_query(&self, a_x: i32, a_y: i32, b_x: i32, b_y: i32) -> bool {
        if self.wraps_coordinate_system {
            edge_intersects_box(
                a_x,
                a_y,
                b_x,
                b_y,
                GeoEncodingUtils::min_lon_encoded(),
                self.max_x,
                self.min_y,
                self.max_y,
            ) || edge_intersects_box(
                a_x,
                a_y,
                b_x,
                b_y,
                self.min_x,
                GeoEncodingUtils::max_lon_encoded(),
                self.min_y,
                self.max_y,
            )
        } else {
            edge_intersects_box(
                a_x, a_y, b_x, b_y, self.min_x, self.max_x, self.min_y, self.max_y,
            )
        }
    }
}

/// Returns whether the edge `(a_x, a_y)`–`(b_x, b_y)` intersects the box.
///
/// Equivalent to the private static
/// `EncodedRectangle.edgeIntersectsBox(...)`.
#[allow(clippy::too_many_arguments)]
fn edge_intersects_box(
    a_x: i32,
    a_y: i32,
    b_x: i32,
    b_y: i32,
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
) -> bool {
    if a_x.max(b_x) < min_x || a_x.min(b_x) > max_x || a_y.min(b_y) > max_y || a_y.max(b_y) < min_y
    {
        return false;
    }
    let (a_x, a_y, b_x, b_y) = (
        f64::from(a_x),
        f64::from(a_y),
        f64::from(b_x),
        f64::from(b_y),
    );
    let (min_x, max_x, min_y, max_y) = (
        f64::from(min_x),
        f64::from(max_x),
        f64::from(min_y),
        f64::from(max_y),
    );
    // Top, bottom, left, right.
    GeoUtils::line_crosses_line_with_boundary(a_x, a_y, b_x, b_y, min_x, max_y, max_x, max_y)
        || GeoUtils::line_crosses_line_with_boundary(a_x, a_y, b_x, b_y, max_x, max_y, max_x, min_y)
        || GeoUtils::line_crosses_line_with_boundary(a_x, a_y, b_x, b_y, max_x, min_y, min_x, min_y)
        || GeoUtils::line_crosses_line_with_boundary(a_x, a_y, b_x, b_y, min_x, min_y, min_x, max_y)
}

// -----------------------------------------------------------------------------
// SpatialQuery
// -----------------------------------------------------------------------------

/// The state every spatial query carries, and the BKD walk that turns a
/// [`SpatialVisitor`] into the set of matching documents.
///
/// Equivalent to the abstract class `org.apache.lucene.document.SpatialQuery`.
#[derive(Clone)]
pub struct SpatialQuery {
    field: String,
    query_relation: QueryRelation,
    query_component2d: Arc<dyn Component2D>,
}

impl std::fmt::Debug for SpatialQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpatialQuery")
            .field("field", &self.field)
            .field("query_relation", &self.query_relation)
            .finish_non_exhaustive()
    }
}

impl SpatialQuery {
    /// Creates the shared state.
    ///
    /// Equivalent to `SpatialQuery(String, QueryRelation, Geometry...)`, whose
    /// `field == null` check Rust's type system already enforces.
    pub fn new(
        field: impl Into<String>,
        query_relation: QueryRelation,
        query_component2d: Arc<dyn Component2D>,
    ) -> Self {
        Self {
            field: field.into(),
            query_relation,
            query_component2d,
        }
    }

    /// Returns the field name.
    ///
    /// Equivalent to `SpatialQuery.getField()`.
    pub fn get_field(&self) -> &str {
        &self.field
    }

    /// Returns the query relation.
    ///
    /// Equivalent to `SpatialQuery.getQueryRelation()`.
    pub fn get_query_relation(&self) -> QueryRelation {
        self.query_relation
    }

    /// Returns the component the query relates against.
    ///
    /// Equivalent to the `SpatialQuery.queryComponent2D` field.
    pub fn query_component2d(&self) -> &Arc<dyn Component2D> {
        &self.query_component2d
    }

    /// Returns whether the query may be cached for this leaf.
    ///
    /// Equivalent to `SpatialQuery.queryIsCacheable(LeafReaderContext)`, which
    /// answers `true`.
    pub fn query_is_cacheable(&self) -> bool {
        true
    }

    /// Returns the documents of `reader` this query matches, or `None` when it
    /// matches none.
    ///
    /// Equivalent to
    /// `SpatialQuery.getScorerSupplier(LeafReader, SpatialVisitor, ScoreMode, float, float)`
    /// minus the constant-score wrapper; see the module divergence note.
    ///
    /// # Errors
    ///
    /// Propagates whatever the point tree or the visitor raises.
    pub fn matching_docs(
        &self,
        reader: &dyn LeafReader,
        visitor: &dyn SpatialVisitor,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        let Some(values) = reader.get_point_values(&self.field)? else {
            // No document in this segment has any point for this field.
            return Ok(None);
        };
        if reader.get_field_infos().field_info(&self.field).is_none() {
            // No document in this segment indexed this field at all.
            return Ok(None);
        }
        let (Some(min_packed), Some(max_packed)) =
            (values.min_packed_value()?, values.max_packed_value()?)
        else {
            return Ok(None);
        };

        let rel = visitor.inner_function(self.query_relation, &min_packed, &max_packed)?;
        if rel == Relation::CellOutsideQuery
            || (rel == Relation::CellInsideQuery && self.query_relation == QueryRelation::Contains)
        {
            // No document matches the query.
            return Ok(None);
        }
        if values.doc_count() == reader.max_doc() && rel == Relation::CellInsideQuery {
            // Every document matches the query.
            return Ok(Some(Box::new(crate::search::all(reader.max_doc())?)));
        }

        if self.query_relation != QueryRelation::Intersects
            && self.query_relation != QueryRelation::Contains
            && i64::from(values.doc_count()) != values.size()
            && !has_any_hits(visitor, self.query_relation, values.as_ref())?
        {
            // Check for any hit first, so the adversarial dense case — a shape
            // matching no document at all — is answered quickly.
            return Ok(None);
        }
        // Walk the tree to get the matching documents.
        self.build_iterator(reader, values.as_ref(), visitor)
    }

    /// Returns the estimated number of documents this query matches in
    /// `reader`.
    ///
    /// Equivalent to `SpatialQuery.RelationScorerSupplier.cost()`, which
    /// Lucene computes lazily because it is expensive.
    ///
    /// # Errors
    ///
    /// Propagates whatever the point tree raises.
    pub fn cost(&self, values: &dyn PointValues, visitor: &dyn SpatialVisitor) -> Result<i64> {
        let mut estimate = EstimateVisitor {
            visitor,
            query_relation: self.query_relation,
        };
        values.estimate_doc_count(&mut estimate)
    }

    /// Equivalent to `RelationScorerSupplier.getScorer(LeafReader, float, ScoreMode)`.
    fn build_iterator(
        &self,
        reader: &dyn LeafReader,
        values: &dyn PointValues,
        visitor: &dyn SpatialVisitor,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        match self.query_relation {
            QueryRelation::Intersects => self.sparse_iterator(reader, values, visitor),
            QueryRelation::Contains => self.contains_dense_iterator(reader, values, visitor),
            QueryRelation::Within | QueryRelation::Disjoint => {
                if i64::from(values.doc_count()) == values.size() {
                    self.sparse_iterator(reader, values, visitor)
                } else {
                    self.dense_iterator(reader, values, visitor)
                }
            }
        }
    }

    /// Equivalent to `RelationScorerSupplier.getSparseScorer(...)`, used for
    /// `INTERSECTS` and for single-valued points.
    fn sparse_iterator(
        &self,
        reader: &dyn LeafReader,
        values: &dyn PointValues,
        visitor: &dyn SpatialVisitor,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        let max_doc = reader.max_doc();
        if self.query_relation == QueryRelation::Disjoint
            && values.doc_count() == max_doc
            && i64::from(values.doc_count()) == values.size()
            && self.cost(values, visitor)? > i64::from(max_doc) / 2
        {
            // Every document has exactly one value and the cost is more than
            // half the leaf, so computing the complement may be faster.
            let mut result = FixedBitSet::new(max_doc as usize);
            result.set_range(0, max_doc as usize);
            let mut cost = i64::from(max_doc);
            let mut inverse = InverseDenseVisitor {
                visitor,
                query_relation: self.query_relation,
                result: &mut result,
                cost: &mut cost,
            };
            values.intersect(&mut inverse)?;
            return Ok(Some(bit_set_iterator(result, cost)?));
        }
        if i64::from(values.doc_count()) < (values.size() >> 2) {
            // Use a dense structure, so an already visited document is skipped.
            let mut result = FixedBitSet::new(max_doc as usize);
            let mut cost = 0i64;
            let mut dense = IntersectsDenseVisitor {
                visitor,
                query_relation: self.query_relation,
                result: &mut result,
                cost: &mut cost,
            };
            values.intersect(&mut dense)?;
            debug_assert!(cost > 0 || result.cardinality() == 0);
            if cost == 0 {
                return Ok(Some(Box::new(crate::search::empty())));
            }
            return Ok(Some(bit_set_iterator(result, cost)?));
        }
        let mut builder = DocIdSetBuilder::from_point_values(max_doc, values);
        {
            let mut sparse = SparseVisitor {
                visitor,
                query_relation: self.query_relation,
                result: &mut builder,
            };
            values.intersect(&mut sparse)?;
        }
        Ok(Some(builder.build()?.iterator()?))
    }

    /// Equivalent to `RelationScorerSupplier.getDenseScorer(...)`, used for
    /// `WITHIN` and `DISJOINT`.
    fn dense_iterator(
        &self,
        reader: &dyn LeafReader,
        values: &dyn PointValues,
        visitor: &dyn SpatialVisitor,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        let max_doc = reader.max_doc();
        let mut result = FixedBitSet::new(max_doc as usize);
        let mut cost;
        if values.doc_count() == max_doc {
            cost = values.size();
            // One visit to the tree can be spared: every document is a
            // potential match.
            result.set_range(0, max_doc as usize);
            // Remove the false positives.
            let mut inverse = InverseDenseVisitor {
                visitor,
                query_relation: self.query_relation,
                result: &mut result,
                cost: &mut cost,
            };
            values.intersect(&mut inverse)?;
        } else {
            cost = 0;
            // Collect the potential documents.
            let mut excluded = FixedBitSet::new(max_doc as usize);
            {
                let mut dense = DenseVisitor {
                    visitor,
                    query_relation: self.query_relation,
                    result: &mut result,
                    excluded: &mut excluded,
                    cost: &mut cost,
                };
                values.intersect(&mut dense)?;
            }
            and_not(&mut result, &excluded);
            // Remove the false positives. Only the inner nodes matter, because
            // the intersecting leaf nodes have already been taken into account;
            // this still reads the leaf nodes, unfortunately.
            let mut shallow = ShallowInverseDenseVisitor {
                visitor,
                query_relation: self.query_relation,
                result: &mut result,
            };
            values.intersect(&mut shallow)?;
        }
        debug_assert!(cost > 0 || result.cardinality() == 0);
        if cost == 0 {
            return Ok(Some(Box::new(crate::search::empty())));
        }
        Ok(Some(bit_set_iterator(result, cost)?))
    }

    /// Equivalent to `RelationScorerSupplier.getContainsDenseScorer(...)`.
    fn contains_dense_iterator(
        &self,
        reader: &dyn LeafReader,
        values: &dyn PointValues,
        visitor: &dyn SpatialVisitor,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        let max_doc = reader.max_doc();
        let mut result = FixedBitSet::new(max_doc as usize);
        let mut cost = 0i64;
        let mut excluded = FixedBitSet::new(max_doc as usize);
        {
            let mut contains = ContainsDenseVisitor {
                visitor,
                query_relation: self.query_relation,
                result: &mut result,
                excluded: &mut excluded,
                cost: &mut cost,
            };
            values.intersect(&mut contains)?;
        }
        and_not(&mut result, &excluded);
        debug_assert!(cost > 0 || result.cardinality() == 0);
        if cost == 0 {
            return Ok(Some(Box::new(crate::search::empty())));
        }
        Ok(Some(bit_set_iterator(result, cost)?))
    }

    /// Prints this query, given the concrete class name Java's
    /// `getClass().getSimpleName()` would produce and the geometries it holds.
    ///
    /// Equivalent to `SpatialQuery.toString(String)`.
    pub fn to_query_string(&self, class_name: &str, field: &str, geometries: &[String]) -> String {
        let mut sb = String::from(class_name);
        sb.push(':');
        if self.field != field {
            sb.push_str(" field=");
            sb.push_str(&self.field);
            sb.push(':');
        }
        sb.push('[');
        for g in geometries {
            sb.push_str(g);
            sb.push(',');
        }
        sb.push(']');
        sb
    }
}

/// Wraps a bit set in the iterator Lucene builds with `new BitSetIterator(...)`.
fn bit_set_iterator(result: FixedBitSet, cost: i64) -> Result<Box<dyn DocIdSetIterator>> {
    Ok(Box::new(BitSetIterator::new(Arc::new(result), cost)?))
}

/// Clears in `result` every bit set in `excluded`.
///
/// Equivalent to `FixedBitSet.andNot(FixedBitSet)`, which this port's
/// `FixedBitSet` does not expose; walking the set bits of `excluded` gives the
/// same result.
fn and_not(result: &mut FixedBitSet, excluded: &FixedBitSet) {
    let length = excluded.length() as i32;
    let mut doc = if length == 0 {
        -1
    } else {
        excluded.next_set_bit(0)
    };
    while doc >= 0 && doc < length {
        result.clear(doc as usize);
        if doc + 1 >= length {
            break;
        }
        doc = excluded.next_set_bit(doc + 1);
    }
}

/// Clears in `result` every document `iterator` produces.
///
/// Equivalent to `FixedBitSet.andNot(DocIdSetIterator)`.
fn and_not_iterator(result: &mut FixedBitSet, iterator: &mut dyn DocIdSetIterator) -> Result<()> {
    loop {
        let doc = iterator.next_doc()?;
        if doc == NO_MORE_DOCS {
            return Ok(());
        }
        result.clear(doc as usize);
    }
}

// -----------------------------------------------------------------------------
// Intersect visitors
// -----------------------------------------------------------------------------

/// Counts the points a relation would visit.
///
/// Equivalent to the visitor `SpatialQuery.getEstimateVisitor` returns.
struct EstimateVisitor<'a> {
    visitor: &'a dyn SpatialVisitor,
    query_relation: QueryRelation,
}

impl crate::index::point_values::IntersectVisitor for EstimateVisitor<'_> {
    fn visit(&mut self, _doc_id: i32) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "the estimate visitor never visits a document".to_string(),
        ))
    }

    fn visit_with_value(&mut self, _doc_id: i32, _packed_value: &[u8]) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "the estimate visitor never visits a document".to_string(),
        ))
    }

    fn compare(&self, min_triangle: &[u8], max_triangle: &[u8]) -> Relation {
        self.visitor
            .inner_function(self.query_relation, min_triangle, max_triangle)
            .unwrap_or(Relation::CellCrossesQuery)
    }
}

/// Adds matching documents to a sparse builder, used by `INTERSECTS` when the
/// number of documents is at most four times the number of points.
///
/// Equivalent to the visitor `SpatialQuery.getSparseVisitor` returns.
///
/// **Divergence from Lucene 10.5.0.** Java keeps the `DocIdSetBuilder.BulkAdder`
/// obtained in `grow(int)` in a field and reuses it across `visit` calls. A
/// Rust adder borrows the builder, so it cannot outlive the call; `grow(0)`
/// re-obtains one without reserving anything or touching the cost counter, so
/// the reservation still happens exactly once per batch.
struct SparseVisitor<'a> {
    visitor: &'a dyn SpatialVisitor,
    query_relation: QueryRelation,
    result: &'a mut DocIdSetBuilder,
}

impl crate::index::point_values::IntersectVisitor for SparseVisitor<'_> {
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
        if self
            .visitor
            .leaf_predicate(self.query_relation, packed_value)?
        {
            self.visit(doc_id)?;
        }
        Ok(())
    }

    fn visit_iterator_with_value(
        &mut self,
        iterator: &mut dyn DocIdSetIterator,
        packed_value: &[u8],
    ) -> Result<()> {
        if self
            .visitor
            .leaf_predicate(self.query_relation, packed_value)?
        {
            self.result.grow(0).add_iterator(iterator)?;
        }
        Ok(())
    }

    fn compare(&self, min_triangle: &[u8], max_triangle: &[u8]) -> Relation {
        self.visitor
            .inner_function(self.query_relation, min_triangle, max_triangle)
            .unwrap_or(Relation::CellCrossesQuery)
    }
}

/// Adds matching documents to a dense bit set, used by `INTERSECTS` when the
/// number of points is more than four times the number of documents.
///
/// Equivalent to the visitor `SpatialQuery.getIntersectsDenseVisitor` returns.
struct IntersectsDenseVisitor<'a> {
    visitor: &'a dyn SpatialVisitor,
    query_relation: QueryRelation,
    result: &'a mut FixedBitSet,
    cost: &'a mut i64,
}

impl crate::index::point_values::IntersectVisitor for IntersectsDenseVisitor<'_> {
    fn visit(&mut self, doc_id: i32) -> Result<()> {
        self.result.set(doc_id as usize);
        *self.cost += 1;
        Ok(())
    }

    fn visit_iterator(&mut self, iterator: &mut dyn DocIdSetIterator) -> Result<()> {
        let cost = iterator.cost();
        BitSet::or(self.result, iterator)?;
        *self.cost += cost;
        Ok(())
    }

    fn visit_ints_ref(&mut self, ints_ref: &IntsRef) -> Result<()> {
        for doc_id in ints_ref.slice().iter().copied() {
            self.result.set(doc_id as usize);
        }
        *self.cost += ints_ref.length as i64;
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        if !self.result.get(doc_id as usize)
            && self
                .visitor
                .leaf_predicate(self.query_relation, packed_value)?
        {
            self.visit(doc_id)?;
        }
        Ok(())
    }

    fn visit_iterator_with_value(
        &mut self,
        iterator: &mut dyn DocIdSetIterator,
        packed_value: &[u8],
    ) -> Result<()> {
        if self
            .visitor
            .leaf_predicate(self.query_relation, packed_value)?
        {
            self.visit_iterator(iterator)?;
        }
        Ok(())
    }

    fn compare(&self, min_triangle: &[u8], max_triangle: &[u8]) -> Relation {
        self.visitor
            .inner_function(self.query_relation, min_triangle, max_triangle)
            .unwrap_or(Relation::CellCrossesQuery)
    }
}

/// Adds matching documents to a dense bit set while tracking the ones already
/// ruled out, used by `WITHIN` and `DISJOINT`.
///
/// Equivalent to the visitor `SpatialQuery.getDenseVisitor` returns.
struct DenseVisitor<'a> {
    visitor: &'a dyn SpatialVisitor,
    query_relation: QueryRelation,
    result: &'a mut FixedBitSet,
    excluded: &'a mut FixedBitSet,
    cost: &'a mut i64,
}

impl crate::index::point_values::IntersectVisitor for DenseVisitor<'_> {
    fn visit(&mut self, doc_id: i32) -> Result<()> {
        self.result.set(doc_id as usize);
        *self.cost += 1;
        Ok(())
    }

    fn visit_ints_ref(&mut self, ints_ref: &IntsRef) -> Result<()> {
        for doc_id in ints_ref.slice().iter().copied() {
            self.result.set(doc_id as usize);
        }
        *self.cost += ints_ref.length as i64;
        Ok(())
    }

    fn visit_iterator(&mut self, iterator: &mut dyn DocIdSetIterator) -> Result<()> {
        let cost = iterator.cost();
        BitSet::or(self.result, iterator)?;
        *self.cost += cost;
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        if !self.excluded.get(doc_id as usize) {
            if self
                .visitor
                .leaf_predicate(self.query_relation, packed_value)?
            {
                self.visit(doc_id)?;
            } else {
                self.excluded.set(doc_id as usize);
            }
        }
        Ok(())
    }

    fn visit_iterator_with_value(
        &mut self,
        iterator: &mut dyn DocIdSetIterator,
        packed_value: &[u8],
    ) -> Result<()> {
        if self
            .visitor
            .leaf_predicate(self.query_relation, packed_value)?
        {
            self.visit_iterator(iterator)?;
        } else {
            BitSet::or(self.excluded, iterator)?;
        }
        Ok(())
    }

    fn compare(&self, min_triangle: &[u8], max_triangle: &[u8]) -> Relation {
        self.visitor
            .inner_function(self.query_relation, min_triangle, max_triangle)
            .unwrap_or(Relation::CellCrossesQuery)
    }
}

/// Adds candidate documents to a dense bit set, used by `CONTAINS`.
///
/// Equivalent to the visitor `SpatialQuery.getContainsDenseVisitor` returns.
struct ContainsDenseVisitor<'a> {
    visitor: &'a dyn SpatialVisitor,
    #[allow(dead_code)]
    query_relation: QueryRelation,
    result: &'a mut FixedBitSet,
    excluded: &'a mut FixedBitSet,
    cost: &'a mut i64,
}

impl crate::index::point_values::IntersectVisitor for ContainsDenseVisitor<'_> {
    fn visit(&mut self, doc_id: i32) -> Result<()> {
        self.excluded.set(doc_id as usize);
        Ok(())
    }

    fn visit_iterator(&mut self, iterator: &mut dyn DocIdSetIterator) -> Result<()> {
        BitSet::or(self.excluded, iterator)
    }

    fn visit_ints_ref(&mut self, ints_ref: &IntsRef) -> Result<()> {
        for doc_id in ints_ref.slice().iter().copied() {
            self.visit(doc_id)?;
        }
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        if !self.excluded.get(doc_id as usize) {
            match self.visitor.contains(packed_value)? {
                WithinRelation::Candidate => {
                    *self.cost += 1;
                    self.result.set(doc_id as usize);
                }
                WithinRelation::NotWithin => self.excluded.set(doc_id as usize),
                WithinRelation::Disjoint => {}
            }
        }
        Ok(())
    }

    fn visit_iterator_with_value(
        &mut self,
        iterator: &mut dyn DocIdSetIterator,
        packed_value: &[u8],
    ) -> Result<()> {
        let within = self.visitor.contains(packed_value)?;
        loop {
            let doc_id = iterator.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                return Ok(());
            }
            match within {
                WithinRelation::Candidate => {
                    *self.cost += 1;
                    self.result.set(doc_id as usize);
                }
                WithinRelation::NotWithin => self.excluded.set(doc_id as usize),
                WithinRelation::Disjoint => {}
            }
        }
    }

    fn compare(&self, min_triangle: &[u8], max_triangle: &[u8]) -> Relation {
        self.visitor
            .inner_function(self.query_relation, min_triangle, max_triangle)
            .unwrap_or(Relation::CellCrossesQuery)
    }
}

/// Clears the documents that do *not* match, used by `WITHIN` and `DISJOINT`.
///
/// Equivalent to the visitor `SpatialQuery.getInverseDenseVisitor` returns.
struct InverseDenseVisitor<'a> {
    visitor: &'a dyn SpatialVisitor,
    query_relation: QueryRelation,
    result: &'a mut FixedBitSet,
    cost: &'a mut i64,
}

impl crate::index::point_values::IntersectVisitor for InverseDenseVisitor<'_> {
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
        and_not_iterator(self.result, iterator)?;
        *self.cost = 0.max(*self.cost - cost);
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        if self.result.get(doc_id as usize)
            && !self
                .visitor
                .leaf_predicate(self.query_relation, packed_value)?
        {
            self.visit(doc_id)?;
        }
        Ok(())
    }

    fn visit_iterator_with_value(
        &mut self,
        iterator: &mut dyn DocIdSetIterator,
        packed_value: &[u8],
    ) -> Result<()> {
        if !self
            .visitor
            .leaf_predicate(self.query_relation, packed_value)?
        {
            self.visit_iterator(iterator)?;
        }
        Ok(())
    }

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Relation {
        transpose_relation(
            self.visitor
                .inner_function(self.query_relation, min_packed_value, max_packed_value)
                .unwrap_or(Relation::CellCrossesQuery),
        )
    }
}

/// Clears the documents that do not match, considering only the inner nodes.
///
/// Equivalent to the visitor `SpatialQuery.getShallowInverseDenseVisitor`
/// returns.
struct ShallowInverseDenseVisitor<'a> {
    visitor: &'a dyn SpatialVisitor,
    query_relation: QueryRelation,
    result: &'a mut FixedBitSet,
}

impl crate::index::point_values::IntersectVisitor for ShallowInverseDenseVisitor<'_> {
    fn visit(&mut self, doc_id: i32) -> Result<()> {
        self.result.clear(doc_id as usize);
        Ok(())
    }

    fn visit_iterator(&mut self, iterator: &mut dyn DocIdSetIterator) -> Result<()> {
        and_not_iterator(self.result, iterator)
    }

    fn visit_ints_ref(&mut self, ints_ref: &IntsRef) -> Result<()> {
        for doc_id in ints_ref.slice().iter().copied() {
            self.visit(doc_id)?;
        }
        Ok(())
    }

    fn visit_with_value(&mut self, _doc_id: i32, _packed_value: &[u8]) -> Result<()> {
        // No-op, as in Java: only the inner nodes matter here.
        Ok(())
    }

    fn visit_iterator_with_value(
        &mut self,
        _iterator: &mut dyn DocIdSetIterator,
        _packed_value: &[u8],
    ) -> Result<()> {
        // No-op, as in Java.
        Ok(())
    }

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Relation {
        transpose_relation(
            self.visitor
                .inner_function(self.query_relation, min_packed_value, max_packed_value)
                .unwrap_or(Relation::CellCrossesQuery),
        )
    }
}

/// Returns whether the query matches at least one document.
///
/// Equivalent to the private static `SpatialQuery.hasAnyHits(...)`.
///
/// **Divergence from Lucene 10.5.0.** Java terminates the walk by throwing
/// `CollectionTerminatedException` from inside the visitor. The point-tree walk
/// here reports failure through `Result`, and an early *success* is not a
/// failure, so the visitor instead records the hit and then answers
/// [`Relation::CellOutsideQuery`] to every subsequent cell, which prunes the
/// remainder of the tree. The answer is identical and at most the leaf being
/// scanned is finished before the walk stops descending.
fn has_any_hits(
    visitor: &dyn SpatialVisitor,
    query_relation: QueryRelation,
    values: &dyn PointValues,
) -> Result<bool> {
    struct HasAnyHits<'a> {
        visitor: &'a dyn SpatialVisitor,
        query_relation: QueryRelation,
        found: std::cell::Cell<bool>,
    }

    impl crate::index::point_values::IntersectVisitor for HasAnyHits<'_> {
        fn visit(&mut self, _doc_id: i32) -> Result<()> {
            self.found.set(true);
            Ok(())
        }

        fn visit_with_value(&mut self, _doc_id: i32, packed_value: &[u8]) -> Result<()> {
            if self
                .visitor
                .leaf_predicate(self.query_relation, packed_value)?
            {
                self.found.set(true);
            }
            Ok(())
        }

        fn visit_iterator_with_value(
            &mut self,
            _iterator: &mut dyn DocIdSetIterator,
            packed_value: &[u8],
        ) -> Result<()> {
            if self
                .visitor
                .leaf_predicate(self.query_relation, packed_value)?
            {
                self.found.set(true);
            }
            Ok(())
        }

        fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Relation {
            if self.found.get() {
                return Relation::CellOutsideQuery;
            }
            let rel = self
                .visitor
                .inner_function(self.query_relation, min_packed_value, max_packed_value)
                .unwrap_or(Relation::CellCrossesQuery);
            if rel == Relation::CellInsideQuery {
                self.found.set(true);
                return Relation::CellOutsideQuery;
            }
            rel
        }
    }

    let mut has_any_hits = HasAnyHits {
        visitor,
        query_relation,
        found: std::cell::Cell::new(false),
    };
    values.intersect(&mut has_any_hits)?;
    Ok(has_any_hits.found.get())
}

// -----------------------------------------------------------------------------
// LatLonShapeQuery
// -----------------------------------------------------------------------------

/// The [`SpatialVisitor`] of a geographic shape query.
///
/// Equivalent to the anonymous `SpatialVisitor` that the static
/// `LatLonShapeQuery.getSpatialVisitor(Component2D)` returns; the doc-values
/// query reuses it, which is why Lucene made it static.
#[derive(Clone)]
pub struct LatLonShapeSpatialVisitor {
    component2d: Arc<dyn Component2D>,
}

impl LatLonShapeSpatialVisitor {
    /// Creates the visitor over `component2d`.
    ///
    /// Equivalent to `LatLonShapeQuery.getSpatialVisitor(Component2D)`.
    pub fn new(component2d: Arc<dyn Component2D>) -> Self {
        Self { component2d }
    }
}

impl std::fmt::Debug for LatLonShapeSpatialVisitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LatLonShapeSpatialVisitor").finish()
    }
}

impl SpatialVisitor for LatLonShapeSpatialVisitor {
    fn relate(&self, min_triangle: &[u8], max_triangle: &[u8]) -> Result<Relation> {
        let min_lat =
            GeoEncodingUtils::decode_latitude(NumericUtils::sortable_bytes_to_int(min_triangle, 0));
        let min_lon = GeoEncodingUtils::decode_longitude(NumericUtils::sortable_bytes_to_int(
            min_triangle,
            BYTES,
        ));
        let max_lat = GeoEncodingUtils::decode_latitude(NumericUtils::sortable_bytes_to_int(
            max_triangle,
            2 * BYTES,
        ));
        let max_lon = GeoEncodingUtils::decode_longitude(NumericUtils::sortable_bytes_to_int(
            max_triangle,
            3 * BYTES,
        ));
        // Check the internal node against the query.
        Ok(self.component2d.relate(min_lon, max_lon, min_lat, max_lat))
    }

    fn intersects(&self, triangle: &[u8]) -> Result<bool> {
        let t = ShapeField::decode_triangle(triangle)?;
        let (alat, alon) = (
            GeoEncodingUtils::decode_latitude(t.a_y),
            GeoEncodingUtils::decode_longitude(t.a_x),
        );
        Ok(match t.triangle_type {
            crate::document::shape_field::TriangleType::Point => {
                self.component2d.contains(alon, alat)
            }
            crate::document::shape_field::TriangleType::Line => {
                let blat = GeoEncodingUtils::decode_latitude(t.b_y);
                let blon = GeoEncodingUtils::decode_longitude(t.b_x);
                self.component2d
                    .intersects_line_bbox(alon, alat, blon, blat)
            }
            crate::document::shape_field::TriangleType::Triangle => {
                let blat = GeoEncodingUtils::decode_latitude(t.b_y);
                let blon = GeoEncodingUtils::decode_longitude(t.b_x);
                let clat = GeoEncodingUtils::decode_latitude(t.c_y);
                let clon = GeoEncodingUtils::decode_longitude(t.c_x);
                self.component2d
                    .intersects_triangle_bbox(alon, alat, blon, blat, clon, clat)
            }
        })
    }

    fn within(&self, triangle: &[u8]) -> Result<bool> {
        let t = ShapeField::decode_triangle(triangle)?;
        let (alat, alon) = (
            GeoEncodingUtils::decode_latitude(t.a_y),
            GeoEncodingUtils::decode_longitude(t.a_x),
        );
        Ok(match t.triangle_type {
            crate::document::shape_field::TriangleType::Point => {
                self.component2d.contains(alon, alat)
            }
            crate::document::shape_field::TriangleType::Line => {
                let blat = GeoEncodingUtils::decode_latitude(t.b_y);
                let blon = GeoEncodingUtils::decode_longitude(t.b_x);
                self.component2d.contains_line_bbox(alon, alat, blon, blat)
            }
            crate::document::shape_field::TriangleType::Triangle => {
                let blat = GeoEncodingUtils::decode_latitude(t.b_y);
                let blon = GeoEncodingUtils::decode_longitude(t.b_x);
                let clat = GeoEncodingUtils::decode_latitude(t.c_y);
                let clon = GeoEncodingUtils::decode_longitude(t.c_x);
                self.component2d
                    .contains_triangle_bbox(alon, alat, blon, blat, clon, clat)
            }
        })
    }

    fn contains(&self, triangle: &[u8]) -> Result<WithinRelation> {
        let t = ShapeField::decode_triangle(triangle)?;
        let (alat, alon) = (
            GeoEncodingUtils::decode_latitude(t.a_y),
            GeoEncodingUtils::decode_longitude(t.a_x),
        );
        Ok(match t.triangle_type {
            crate::document::shape_field::TriangleType::Point => {
                self.component2d.within_point(alon, alat)
            }
            crate::document::shape_field::TriangleType::Line => {
                let blat = GeoEncodingUtils::decode_latitude(t.b_y);
                let blon = GeoEncodingUtils::decode_longitude(t.b_x);
                self.component2d
                    .within_line_bbox(alon, alat, t.ab, blon, blat)
            }
            crate::document::shape_field::TriangleType::Triangle => {
                let blat = GeoEncodingUtils::decode_latitude(t.b_y);
                let blon = GeoEncodingUtils::decode_longitude(t.b_x);
                let clat = GeoEncodingUtils::decode_latitude(t.c_y);
                let clon = GeoEncodingUtils::decode_longitude(t.c_x);
                self.component2d
                    .within_triangle_bbox(alon, alat, t.ab, blon, blat, t.bc, clon, clat, t.ca)
            }
        })
    }
}

/// Finds every indexed geographic shape that complies with the given
/// [`QueryRelation`] against an array of geometries.
///
/// Equivalent to `org.apache.lucene.document.LatLonShapeQuery`. The field must
/// have been indexed with
/// [`LatLonShape::create_indexable_fields_*`](crate::document::LatLonShape).
#[derive(Clone, Debug)]
pub struct LatLonShapeQuery {
    query: SpatialQuery,
    geometries: Vec<LatLonGeometryValue>,
    visitor: LatLonShapeSpatialVisitor,
}

impl LatLonShapeQuery {
    /// Creates the query.
    ///
    /// Equivalent to
    /// `LatLonShapeQuery(String, QueryRelation, LatLonGeometry...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for a `WITHIN` query over a
    /// line geometry, which Lucene does not support, and for an empty geometry
    /// list.
    pub fn new(
        field: impl Into<String>,
        query_relation: QueryRelation,
        geometries: Vec<LatLonGeometryValue>,
    ) -> Result<Self> {
        validate_geometries(query_relation, &geometries)?;
        let component2d = LatLonGeometryValue::create(&geometries)?;
        Ok(Self {
            query: SpatialQuery::new(field, query_relation, Arc::clone(&component2d)),
            geometries,
            visitor: LatLonShapeSpatialVisitor::new(component2d),
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
    /// Equivalent to `LatLonShapeQuery.getSpatialVisitor()`.
    pub fn get_spatial_visitor(&self) -> &LatLonShapeSpatialVisitor {
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
    /// Equivalent to `SpatialQuery.toString(String)` for this class.
    pub fn to_query_string(&self, field: &str) -> String {
        self.query.to_query_string(
            "LatLonShapeQuery",
            field,
            &self
                .geometries
                .iter()
                .map(|g| format!("{g:?}"))
                .collect::<Vec<_>>(),
        )
    }
}

/// Rejects a `WITHIN` query over a line geometry.
///
/// Equivalent to the private static `LatLonShapeQuery.validateGeometries`.
fn validate_geometries(
    query_relation: QueryRelation,
    geometries: &[LatLonGeometryValue],
) -> Result<()> {
    if query_relation == QueryRelation::Within {
        for geometry in geometries {
            if matches!(geometry, LatLonGeometryValue::Line(_)) {
                return Err(LuceneError::IllegalArgument(
                    "LatLonShapeQuery does not support Within queries with line geometries"
                        .to_string(),
                ));
            }
        }
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// XYShapeQuery
// -----------------------------------------------------------------------------

/// The [`SpatialVisitor`] of a cartesian shape query.
///
/// Equivalent to the anonymous `SpatialVisitor` that the static
/// `XYShapeQuery.getSpatialVisitor(Component2D)` returns.
#[derive(Clone)]
pub struct XYShapeSpatialVisitor {
    component2d: Arc<dyn Component2D>,
}

impl XYShapeSpatialVisitor {
    /// Creates the visitor over `component2d`.
    ///
    /// Equivalent to `XYShapeQuery.getSpatialVisitor(Component2D)`.
    pub fn new(component2d: Arc<dyn Component2D>) -> Self {
        Self { component2d }
    }
}

impl std::fmt::Debug for XYShapeSpatialVisitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XYShapeSpatialVisitor").finish()
    }
}

/// Decodes one encoded cartesian coordinate.
fn xy_decode(encoded: i32) -> f64 {
    XYEncodingUtils::decode(encoded) as f64
}

impl SpatialVisitor for XYShapeSpatialVisitor {
    fn relate(&self, min_triangle: &[u8], max_triangle: &[u8]) -> Result<Relation> {
        let min_y = xy_decode(NumericUtils::sortable_bytes_to_int(min_triangle, 0));
        let min_x = xy_decode(NumericUtils::sortable_bytes_to_int(min_triangle, BYTES));
        let max_y = xy_decode(NumericUtils::sortable_bytes_to_int(max_triangle, 2 * BYTES));
        let max_x = xy_decode(NumericUtils::sortable_bytes_to_int(max_triangle, 3 * BYTES));
        // Check the internal node against the query.
        Ok(self.component2d.relate(min_x, max_x, min_y, max_y))
    }

    fn intersects(&self, triangle: &[u8]) -> Result<bool> {
        let t = ShapeField::decode_triangle(triangle)?;
        let (a_y, a_x) = (xy_decode(t.a_y), xy_decode(t.a_x));
        Ok(match t.triangle_type {
            crate::document::shape_field::TriangleType::Point => {
                self.component2d.contains(a_x, a_y)
            }
            crate::document::shape_field::TriangleType::Line => {
                let (b_y, b_x) = (xy_decode(t.b_y), xy_decode(t.b_x));
                self.component2d.intersects_line_bbox(a_x, a_y, b_x, b_y)
            }
            crate::document::shape_field::TriangleType::Triangle => {
                let (b_y, b_x) = (xy_decode(t.b_y), xy_decode(t.b_x));
                let (c_y, c_x) = (xy_decode(t.c_y), xy_decode(t.c_x));
                self.component2d
                    .intersects_triangle_bbox(a_x, a_y, b_x, b_y, c_x, c_y)
            }
        })
    }

    fn within(&self, triangle: &[u8]) -> Result<bool> {
        let t = ShapeField::decode_triangle(triangle)?;
        let (a_y, a_x) = (xy_decode(t.a_y), xy_decode(t.a_x));
        Ok(match t.triangle_type {
            crate::document::shape_field::TriangleType::Point => {
                self.component2d.contains(a_x, a_y)
            }
            crate::document::shape_field::TriangleType::Line => {
                let (b_y, b_x) = (xy_decode(t.b_y), xy_decode(t.b_x));
                self.component2d.contains_line_bbox(a_x, a_y, b_x, b_y)
            }
            crate::document::shape_field::TriangleType::Triangle => {
                let (b_y, b_x) = (xy_decode(t.b_y), xy_decode(t.b_x));
                let (c_y, c_x) = (xy_decode(t.c_y), xy_decode(t.c_x));
                self.component2d
                    .contains_triangle_bbox(a_x, a_y, b_x, b_y, c_x, c_y)
            }
        })
    }

    fn contains(&self, triangle: &[u8]) -> Result<WithinRelation> {
        let t = ShapeField::decode_triangle(triangle)?;
        let (a_y, a_x) = (xy_decode(t.a_y), xy_decode(t.a_x));
        Ok(match t.triangle_type {
            crate::document::shape_field::TriangleType::Point => {
                self.component2d.within_point(a_x, a_y)
            }
            crate::document::shape_field::TriangleType::Line => {
                let (b_y, b_x) = (xy_decode(t.b_y), xy_decode(t.b_x));
                self.component2d.within_line_bbox(a_x, a_y, t.ab, b_x, b_y)
            }
            crate::document::shape_field::TriangleType::Triangle => {
                let (b_y, b_x) = (xy_decode(t.b_y), xy_decode(t.b_x));
                let (c_y, c_x) = (xy_decode(t.c_y), xy_decode(t.c_x));
                self.component2d
                    .within_triangle_bbox(a_x, a_y, t.ab, b_x, b_y, t.bc, c_x, c_y, t.ca)
            }
        })
    }
}

/// Finds every indexed cartesian shape that complies with the given
/// [`QueryRelation`] against an array of geometries.
///
/// Equivalent to `org.apache.lucene.document.XYShapeQuery`.
#[derive(Clone, Debug)]
pub struct XYShapeQuery {
    query: SpatialQuery,
    geometries: Vec<XYGeometryValue>,
    visitor: XYShapeSpatialVisitor,
}

impl XYShapeQuery {
    /// Creates the query.
    ///
    /// Equivalent to `XYShapeQuery(String, QueryRelation, XYGeometry...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for an empty geometry list.
    pub fn new(
        field: impl Into<String>,
        query_relation: QueryRelation,
        geometries: Vec<XYGeometryValue>,
    ) -> Result<Self> {
        let component2d = XYGeometryValue::create(&geometries)?;
        Ok(Self {
            query: SpatialQuery::new(field, query_relation, Arc::clone(&component2d)),
            geometries,
            visitor: XYShapeSpatialVisitor::new(component2d),
        })
    }

    /// Returns the shared query state.
    pub fn spatial_query(&self) -> &SpatialQuery {
        &self.query
    }

    /// Returns the geometries the query relates against.
    pub fn geometries(&self) -> &[XYGeometryValue] {
        &self.geometries
    }

    /// Returns the visitor that drives the BKD walk.
    ///
    /// Equivalent to `XYShapeQuery.getSpatialVisitor()`.
    pub fn get_spatial_visitor(&self) -> &XYShapeSpatialVisitor {
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
    pub fn to_query_string(&self, field: &str) -> String {
        self.query.to_query_string(
            "XYShapeQuery",
            field,
            &self
                .geometries
                .iter()
                .map(|g| format!("{g:?}"))
                .collect::<Vec<_>>(),
        )
    }
}

// -----------------------------------------------------------------------------
// LatLonShapeBoundingBoxQuery
// -----------------------------------------------------------------------------

/// Compares four unsigned bytes of `a` at `a_offset` with four of `b` at
/// `b_offset`.
///
/// Equivalent to `org.apache.lucene.util.ArrayUtil.compareUnsigned4`.
fn compare_unsigned4(a: &[u8], a_offset: usize, b: &[u8], b_offset: usize) -> std::cmp::Ordering {
    a[a_offset..a_offset + 4].cmp(&b[b_offset..b_offset + 4])
}

/// The encoded bounding box a [`LatLonShapeBoundingBoxQuery`] compares against,
/// which also keeps the packed form the BKD comparison needs.
///
/// Equivalent to the private static nested class
/// `LatLonShapeBoundingBoxQuery.EncodedLatLonRectangle`.
#[derive(Clone, Debug)]
struct EncodedLatLonRectangle {
    rectangle: EncodedRectangle,
    /// The eastern box when the query crosses the dateline, otherwise the whole
    /// box, packed as a shape field's first four dimensions.
    bbox: [u8; 4 * BYTES],
    /// The western box, present only when the query crosses the dateline.
    west: Option<[u8; 4 * BYTES]>,
}

impl EncodedLatLonRectangle {
    /// Equivalent to
    /// `EncodedLatLonRectangle(double, double, double, double)`.
    fn new(min_lat: f64, max_lat: f64, min_lon: f64, max_lon: f64) -> Result<Self> {
        let validated_min_lon = validate_min_lon(min_lon, max_lon);
        let rectangle = EncodedRectangle::new(
            GeoEncodingUtils::encode_longitude_ceil(validated_min_lon)?,
            GeoEncodingUtils::encode_longitude(max_lon)?,
            GeoEncodingUtils::encode_latitude_ceil(min_lat)?,
            GeoEncodingUtils::encode_latitude(max_lat)?,
            validated_min_lon > max_lon,
        );
        let mut bbox = [0u8; 4 * BYTES];
        let west = if rectangle.wraps_coordinate_system {
            // Crossing the dateline splits the box into an eastern and a
            // western half.
            let mut west = [0u8; 4 * BYTES];
            encode_bbox(
                GeoEncodingUtils::min_lon_encoded(),
                rectangle.max_x,
                rectangle.min_y,
                rectangle.max_y,
                &mut west,
            );
            encode_bbox(
                rectangle.min_x,
                GeoEncodingUtils::max_lon_encoded(),
                rectangle.min_y,
                rectangle.max_y,
                &mut bbox,
            );
            Some(west)
        } else {
            encode_bbox(
                rectangle.min_x,
                rectangle.max_x,
                rectangle.min_y,
                rectangle.max_y,
                &mut bbox,
            );
            None
        };
        Ok(Self {
            rectangle,
            bbox,
            west,
        })
    }

    /// Equivalent to `EncodedLatLonRectangle.crossesDateline()`.
    fn crosses_dateline(&self) -> bool {
        self.rectangle.wraps_coordinate_system
    }

    /// Equivalent to `EncodedLatLonRectangle.relateRangeBBox(...)`.
    fn relate_range_bbox(
        &self,
        min_x_offset: usize,
        min_y_offset: usize,
        min_triangle: &[u8],
        max_x_offset: usize,
        max_y_offset: usize,
        max_triangle: &[u8],
    ) -> Relation {
        let east = compare_bbox_to_range_bbox(
            &self.bbox,
            min_x_offset,
            min_y_offset,
            min_triangle,
            max_x_offset,
            max_y_offset,
            max_triangle,
        );
        if self.crosses_dateline() && east == Relation::CellOutsideQuery {
            if let Some(west) = &self.west {
                return compare_bbox_to_range_bbox(
                    west,
                    min_x_offset,
                    min_y_offset,
                    min_triangle,
                    max_x_offset,
                    max_y_offset,
                    max_triangle,
                );
            }
        }
        east
    }

    /// Equivalent to `EncodedLatLonRectangle.intersectRangeBBox(...)`.
    fn intersect_range_bbox(
        &self,
        min_x_offset: usize,
        min_y_offset: usize,
        min_triangle: &[u8],
        max_x_offset: usize,
        max_y_offset: usize,
        max_triangle: &[u8],
    ) -> Relation {
        let east = intersect_bbox_with_range_bbox(
            &self.bbox,
            min_x_offset,
            min_y_offset,
            min_triangle,
            max_x_offset,
            max_y_offset,
            max_triangle,
        );
        if self.crosses_dateline() && east == Relation::CellOutsideQuery {
            if let Some(west) = &self.west {
                return intersect_bbox_with_range_bbox(
                    west,
                    min_x_offset,
                    min_y_offset,
                    min_triangle,
                    max_x_offset,
                    max_y_offset,
                    max_triangle,
                );
            }
        }
        east
    }
}

/// Returns `-180` when the box splits the dateline, so the encoder sees a valid
/// longitude.
///
/// Equivalent to the private static
/// `EncodedLatLonRectangle.validateMinLon(double, double)`.
fn validate_min_lon(min_lon: f64, max_lon: f64) -> f64 {
    if min_lon == 180.0 && min_lon > max_lon {
        -180.0
    } else {
        min_lon
    }
}

/// Packs a bounding box the way a shape field's first four dimensions are
/// packed.
///
/// Equivalent to the private static
/// `EncodedLatLonRectangle.encode(int, int, int, int, byte[])`.
fn encode_bbox(min_x: i32, max_x: i32, min_y: i32, max_y: i32, b: &mut [u8]) {
    NumericUtils::int_to_sortable_bytes(min_y, b, 0);
    NumericUtils::int_to_sortable_bytes(min_x, b, BYTES);
    NumericUtils::int_to_sortable_bytes(max_y, b, 2 * BYTES);
    NumericUtils::int_to_sortable_bytes(max_x, b, 3 * BYTES);
}

/// Equivalent to the private
/// `EncodedLatLonRectangle.compareBBoxToRangeBBox(...)`.
#[allow(clippy::too_many_arguments)]
fn compare_bbox_to_range_bbox(
    bbox: &[u8],
    min_x_offset: usize,
    min_y_offset: usize,
    min_triangle: &[u8],
    max_x_offset: usize,
    max_y_offset: usize,
    max_triangle: &[u8],
) -> Relation {
    use std::cmp::Ordering::{Greater, Less};
    if bbox_disjoint(
        bbox,
        min_x_offset,
        min_y_offset,
        min_triangle,
        max_x_offset,
        max_y_offset,
        max_triangle,
    ) {
        return Relation::CellOutsideQuery;
    }
    if compare_unsigned4(min_triangle, min_x_offset, bbox, BYTES) != Less
        && compare_unsigned4(max_triangle, max_x_offset, bbox, 3 * BYTES) != Greater
        && compare_unsigned4(min_triangle, min_y_offset, bbox, 0) != Less
        && compare_unsigned4(max_triangle, max_y_offset, bbox, 2 * BYTES) != Greater
    {
        return Relation::CellInsideQuery;
    }
    Relation::CellCrossesQuery
}

/// Equivalent to the private
/// `EncodedLatLonRectangle.intersectBBoxWithRangeBBox(...)`.
#[allow(clippy::too_many_arguments)]
fn intersect_bbox_with_range_bbox(
    bbox: &[u8],
    min_x_offset: usize,
    min_y_offset: usize,
    min_triangle: &[u8],
    max_x_offset: usize,
    max_y_offset: usize,
    max_triangle: &[u8],
) -> Relation {
    use std::cmp::Ordering::{Greater, Less};
    if bbox_disjoint(
        bbox,
        min_x_offset,
        min_y_offset,
        min_triangle,
        max_x_offset,
        max_y_offset,
        max_triangle,
    ) {
        return Relation::CellOutsideQuery;
    }
    if compare_unsigned4(min_triangle, min_x_offset, bbox, BYTES) != Less
        && compare_unsigned4(min_triangle, min_y_offset, bbox, 0) != Less
    {
        if compare_unsigned4(max_triangle, min_x_offset, bbox, 3 * BYTES) != Greater
            && compare_unsigned4(max_triangle, max_y_offset, bbox, 2 * BYTES) != Greater
        {
            return Relation::CellInsideQuery;
        }
        if compare_unsigned4(max_triangle, max_x_offset, bbox, 3 * BYTES) != Greater
            && compare_unsigned4(max_triangle, min_y_offset, bbox, 2 * BYTES) != Greater
        {
            return Relation::CellInsideQuery;
        }
    }

    if compare_unsigned4(max_triangle, max_x_offset, bbox, 3 * BYTES) != Greater
        && compare_unsigned4(max_triangle, max_y_offset, bbox, 2 * BYTES) != Greater
    {
        if compare_unsigned4(min_triangle, min_x_offset, bbox, BYTES) != Less
            && compare_unsigned4(min_triangle, max_y_offset, bbox, 0) != Less
        {
            return Relation::CellInsideQuery;
        }
        if compare_unsigned4(min_triangle, max_x_offset, bbox, BYTES) != Less
            && compare_unsigned4(min_triangle, min_y_offset, bbox, 0) != Less
        {
            return Relation::CellInsideQuery;
        }
    }

    Relation::CellCrossesQuery
}

/// Equivalent to the private `EncodedLatLonRectangle.disjoint(...)`.
#[allow(clippy::too_many_arguments)]
fn bbox_disjoint(
    bbox: &[u8],
    min_x_offset: usize,
    min_y_offset: usize,
    min_triangle: &[u8],
    max_x_offset: usize,
    max_y_offset: usize,
    max_triangle: &[u8],
) -> bool {
    use std::cmp::Ordering::{Greater, Less};
    compare_unsigned4(min_triangle, min_x_offset, bbox, 3 * BYTES) == Greater
        || compare_unsigned4(max_triangle, max_x_offset, bbox, BYTES) == Less
        || compare_unsigned4(min_triangle, min_y_offset, bbox, 2 * BYTES) == Greater
        || compare_unsigned4(max_triangle, max_y_offset, bbox, 0) == Less
}

/// The [`SpatialVisitor`] of a geographic bounding-box shape query, which works
/// entirely in the encoded integer space.
///
/// Equivalent to the anonymous `SpatialVisitor` that
/// `LatLonShapeBoundingBoxQuery.getSpatialVisitor()` returns.
#[derive(Clone, Debug)]
pub struct LatLonShapeBoundingBoxVisitor {
    encoded_rectangle: EncodedLatLonRectangle,
    query_relation: QueryRelation,
}

impl SpatialVisitor for LatLonShapeBoundingBoxVisitor {
    fn relate(&self, min_triangle: &[u8], max_triangle: &[u8]) -> Result<Relation> {
        if self.query_relation == QueryRelation::Intersects
            || self.query_relation == QueryRelation::Disjoint
        {
            return Ok(self.encoded_rectangle.intersect_range_bbox(
                BYTES,
                0,
                min_triangle,
                3 * BYTES,
                2 * BYTES,
                max_triangle,
            ));
        }
        Ok(self.encoded_rectangle.relate_range_bbox(
            BYTES,
            0,
            min_triangle,
            3 * BYTES,
            2 * BYTES,
            max_triangle,
        ))
    }

    fn intersects(&self, triangle: &[u8]) -> Result<bool> {
        let t = ShapeField::decode_triangle(triangle)?;
        let r = &self.encoded_rectangle.rectangle;
        Ok(match t.triangle_type {
            crate::document::shape_field::TriangleType::Point => r.contains(t.a_x, t.a_y),
            crate::document::shape_field::TriangleType::Line => {
                r.intersects_line(t.a_x, t.a_y, t.b_x, t.b_y)
            }
            crate::document::shape_field::TriangleType::Triangle => {
                r.intersects_triangle(t.a_x, t.a_y, t.b_x, t.b_y, t.c_x, t.c_y)
            }
        })
    }

    fn within(&self, triangle: &[u8]) -> Result<bool> {
        let t = ShapeField::decode_triangle(triangle)?;
        let r = &self.encoded_rectangle.rectangle;
        Ok(match t.triangle_type {
            crate::document::shape_field::TriangleType::Point => r.contains(t.a_x, t.a_y),
            crate::document::shape_field::TriangleType::Line => {
                r.contains_line(t.a_x, t.a_y, t.b_x, t.b_y)
            }
            crate::document::shape_field::TriangleType::Triangle => {
                r.contains_triangle(t.a_x, t.a_y, t.b_x, t.b_y, t.c_x, t.c_y)
            }
        })
    }

    fn contains(&self, triangle: &[u8]) -> Result<WithinRelation> {
        if self.encoded_rectangle.crosses_dateline() {
            return Err(LuceneError::IllegalArgument(
                "withinTriangle is not supported for rectangles crossing the date line".to_string(),
            ));
        }
        let t = ShapeField::decode_triangle(triangle)?;
        let r = &self.encoded_rectangle.rectangle;
        Ok(match t.triangle_type {
            crate::document::shape_field::TriangleType::Point => {
                if r.contains(t.a_x, t.a_y) {
                    WithinRelation::NotWithin
                } else {
                    WithinRelation::Disjoint
                }
            }
            crate::document::shape_field::TriangleType::Line => {
                r.within_line(t.a_x, t.a_y, t.ab, t.b_x, t.b_y)
            }
            crate::document::shape_field::TriangleType::Triangle => {
                r.within_triangle(t.a_x, t.a_y, t.ab, t.b_x, t.b_y, t.bc, t.c_x, t.c_y, t.ca)
            }
        })
    }
}

/// Finds every indexed geographic shape that relates to a bounding box.
///
/// Equivalent to `org.apache.lucene.document.LatLonShapeBoundingBoxQuery`. It
/// is the specialised form
/// [`LatLonShape::new_box_query`](crate::document::LatLonShape::new_box_query)
/// builds, and it never decodes an indexed triangle: every comparison happens
/// in the encoded integer space.
#[derive(Clone, Debug)]
pub struct LatLonShapeBoundingBoxQuery {
    query: SpatialQuery,
    rectangle: Rectangle,
    visitor: LatLonShapeBoundingBoxVisitor,
}

impl LatLonShapeBoundingBoxQuery {
    /// Creates the query.
    ///
    /// Equivalent to
    /// `LatLonShapeBoundingBoxQuery(String, QueryRelation, Rectangle)`.
    ///
    /// # Errors
    ///
    /// Propagates the encoding errors the rectangle's bounds may raise.
    pub fn new(
        field: impl Into<String>,
        query_relation: QueryRelation,
        rectangle: Rectangle,
    ) -> Result<Self> {
        let component2d =
            LatLonGeometryValue::create(&[LatLonGeometryValue::Rectangle(rectangle.clone())])?;
        Ok(Self {
            query: SpatialQuery::new(field, query_relation, component2d),
            visitor: LatLonShapeBoundingBoxVisitor {
                encoded_rectangle: EncodedLatLonRectangle::new(
                    rectangle.min_lat(),
                    rectangle.max_lat(),
                    rectangle.min_lon(),
                    rectangle.max_lon(),
                )?,
                query_relation,
            },
            rectangle,
        })
    }

    /// Returns the shared query state.
    pub fn spatial_query(&self) -> &SpatialQuery {
        &self.query
    }

    /// Returns the bounding box the query relates against.
    pub fn rectangle(&self) -> &Rectangle {
        &self.rectangle
    }

    /// Returns the visitor that drives the BKD walk.
    ///
    /// Equivalent to `LatLonShapeBoundingBoxQuery.getSpatialVisitor()`.
    pub fn get_spatial_visitor(&self) -> &LatLonShapeBoundingBoxVisitor {
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
    /// Equivalent to `LatLonShapeBoundingBoxQuery.toString(String)`.
    pub fn to_query_string(&self, field: &str) -> String {
        let mut sb = String::from("LatLonShapeBoundingBoxQuery:");
        if self.query.get_field() != field {
            sb.push_str(" field=");
            sb.push_str(self.query.get_field());
            sb.push(':');
        }
        sb.push_str(&format!("{:?}", self.rectangle));
        sb
    }
}

impl PartialEq for LatLonShapeBoundingBoxQuery {
    /// Equivalent to `LatLonShapeBoundingBoxQuery.equalsTo(Object)` combined
    /// with `SpatialQuery.equalsTo(Object)`.
    fn eq(&self, other: &Self) -> bool {
        self.query.get_field() == other.query.get_field()
            && self.query.get_query_relation() == other.query.get_query_relation()
            && self.rectangle == other.rectangle
    }
}

// -----------------------------------------------------------------------------
// Shape doc-values queries
// -----------------------------------------------------------------------------

/// The base of the two shape doc-values queries.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.document.BaseShapeDocValuesQuery`, whose concrete
/// subclasses are [`LatLonShapeDocValuesQuery`] and [`XYShapeDocValuesQuery`].
///
/// It answers a relation by reading the shape back out of its binary doc value
/// and walking the serialised tessellation tree, which is slower than the BKD
/// walk of a [`SpatialQuery`] but needs no points index.
#[derive(Clone, Debug)]
pub struct BaseShapeDocValuesQuery {
    query: SpatialQuery,
    coordinate_system: ShapeCoordinateSystem,
}

impl BaseShapeDocValuesQuery {
    /// Creates the shared state.
    ///
    /// Equivalent to
    /// `BaseShapeDocValuesQuery(String, QueryRelation, Geometry...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for a `CONTAINS` relation,
    /// which Lucene does not support here.
    pub fn new(
        field: impl Into<String>,
        query_relation: QueryRelation,
        query_component2d: Arc<dyn Component2D>,
        coordinate_system: ShapeCoordinateSystem,
    ) -> Result<Self> {
        Ok(Self {
            query: SpatialQuery::new(
                field,
                Self::validate_relation(query_relation)?,
                query_component2d,
            ),
            coordinate_system,
        })
    }

    /// Equivalent to the private static
    /// `BaseShapeDocValuesQuery.validateRelation(QueryRelation)`.
    fn validate_relation(query_relation: QueryRelation) -> Result<QueryRelation> {
        if query_relation == QueryRelation::Contains {
            return Err(LuceneError::IllegalArgument(
                "ShapeDocValuesBoundingBoxQuery does not yet support CONTAINS queries".to_string(),
            ));
        }
        Ok(query_relation)
    }

    /// Returns the shared query state.
    pub fn spatial_query(&self) -> &SpatialQuery {
        &self.query
    }

    /// Returns the coordinate system the stored shapes are encoded in.
    pub fn coordinate_system(&self) -> ShapeCoordinateSystem {
        self.coordinate_system
    }

    /// Returns the cost of one match, in comparisons.
    ///
    /// Equivalent to `BaseShapeDocValuesQuery.matchCost()`: an estimated 60
    /// comparisons times an average of 100 terms.
    pub fn match_cost(&self) -> f32 {
        60.0 * 100.0
    }

    /// Returns whether the stored shape matches the query.
    ///
    /// Equivalent to `BaseShapeDocValuesQuery.match(ShapeDocValues)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the serialised shape raises.
    pub fn match_shape(&self, shape_doc_values: &ShapeDocValues) -> Result<bool> {
        let result = self.matches_component(
            shape_doc_values,
            self.query.get_query_relation(),
            self.query.query_component2d().as_ref(),
        )?;
        Ok(
            if self.query.get_query_relation() == QueryRelation::Disjoint {
                !result
            } else {
                result
            },
        )
    }

    /// Returns whether the stored shape relates to `component` under
    /// `query_relation`.
    ///
    /// Equivalent to
    /// `BaseShapeDocValuesQuery.matchesComponent(ShapeDocValues, QueryRelation, Component2D)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the serialised shape raises.
    pub fn matches_component(
        &self,
        dv: &ShapeDocValues,
        query_relation: QueryRelation,
        component: &dyn Component2D,
    ) -> Result<bool> {
        let r = dv.relate(component)?;
        if r != Relation::CellOutsideQuery {
            if query_relation == QueryRelation::Within {
                return Ok(r == Relation::CellInsideQuery);
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// Returns the documents of `reader` this query matches.
    ///
    /// Equivalent to
    /// `BaseShapeDocValuesQuery.getScorerSupplier(LeafReader, SpatialVisitor, ScoreMode, float, float)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java returns a `ConstantScoreScorer`
    /// over a `TwoPhaseIterator` whose approximation is the `BinaryDocValues`
    /// iterator and whose `matches()` runs [`Self::match_shape`]. Neither
    /// `Scorer` nor `TwoPhaseIterator` is part of this crate's public search
    /// surface yet, so the two phases are collapsed into a single filtering
    /// [`DocIdSetIterator`]: the same documents are produced, in the same
    /// order, but a consumer cannot defer the expensive check the way a
    /// two-phase scorer can. The cost is still `reader.max_doc()`, as Java's
    /// `ScorerSupplier.cost()` reports.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the doc values raises.
    pub fn matching_docs(
        &self,
        reader: &dyn LeafReader,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        let Some(values) = reader.get_binary_doc_values(self.query.get_field())? else {
            return Ok(None);
        };
        if reader
            .get_field_infos()
            .field_info(self.query.get_field())
            .is_none()
        {
            // No document in this segment indexed this field at all.
            return Ok(None);
        }
        Ok(Some(Box::new(ShapeDocValuesMatchIterator {
            values,
            query: self.clone(),
            cost: i64::from(reader.max_doc()),
        })))
    }
}

/// Filters a `BinaryDocValues` iterator down to the documents whose stored
/// shape matches.
///
/// See the divergence note on [`BaseShapeDocValuesQuery::matching_docs`].
struct ShapeDocValuesMatchIterator {
    values: Box<dyn crate::index::BinaryDocValues>,
    query: BaseShapeDocValuesQuery,
    cost: i64,
}

impl ShapeDocValuesMatchIterator {
    /// Equivalent to the `matches()` of the `TwoPhaseIterator` Java builds.
    fn matches(&self) -> Result<bool> {
        let binary_value = self.values.binary_value()?;
        let shape =
            ShapeDocValues::from_binary_value(binary_value, self.query.coordinate_system())?;
        self.query.match_shape(&shape)
    }

    /// Advances past the documents that do not match.
    fn confirm(&mut self, mut doc: i32) -> Result<i32> {
        while doc != NO_MORE_DOCS {
            if self.matches()? {
                return Ok(doc);
            }
            doc = self.values.next_doc()?;
        }
        Ok(NO_MORE_DOCS)
    }
}

impl DocIdSetIterator for ShapeDocValuesMatchIterator {
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
        self.cost
    }
}

/// A bounding-box query over geographic shape doc values.
///
/// Equivalent to `org.apache.lucene.document.LatLonShapeDocValuesQuery`, which
/// [`LatLonShape::new_slow_doc_values_box_query`](crate::document::LatLonShape::new_slow_doc_values_box_query)
/// builds.
#[derive(Clone, Debug)]
pub struct LatLonShapeDocValuesQuery {
    base: BaseShapeDocValuesQuery,
    geometries: Vec<LatLonGeometryValue>,
    visitor: LatLonShapeSpatialVisitor,
}

impl LatLonShapeDocValuesQuery {
    /// Creates the query.
    ///
    /// Equivalent to
    /// `LatLonShapeDocValuesQuery(String, QueryRelation, LatLonGeometry...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for a `CONTAINS` relation and
    /// for an empty geometry list.
    pub fn new(
        field: impl Into<String>,
        query_relation: QueryRelation,
        geometries: Vec<LatLonGeometryValue>,
    ) -> Result<Self> {
        let component2d = LatLonGeometryValue::create(&geometries)?;
        Ok(Self {
            base: BaseShapeDocValuesQuery::new(
                field,
                query_relation,
                Arc::clone(&component2d),
                ShapeCoordinateSystem::Geographic,
            )?,
            geometries,
            visitor: LatLonShapeSpatialVisitor::new(component2d),
        })
    }

    /// Returns the shared doc-values query state.
    pub fn base(&self) -> &BaseShapeDocValuesQuery {
        &self.base
    }

    /// Returns the geometries the query relates against.
    pub fn geometries(&self) -> &[LatLonGeometryValue] {
        &self.geometries
    }

    /// Returns the shape a stored binary value holds.
    ///
    /// Equivalent to `LatLonShapeDocValuesQuery.getShapeDocValues(BytesRef)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the serialised shape raises.
    pub fn get_shape_doc_values(
        &self,
        binary_value: crate::util::BytesRef,
    ) -> Result<crate::document::LatLonShapeDocValues> {
        crate::document::LatLonShapeDocValues::from_binary_value(binary_value)
    }

    /// Returns the cost of one match.
    ///
    /// Equivalent to `LatLonShapeDocValuesQuery.matchCost()`.
    pub fn match_cost(&self) -> f32 {
        60.0 * 100.0
    }

    /// Returns the visitor a BKD walk would use.
    ///
    /// Equivalent to `LatLonShapeDocValuesQuery.getSpatialVisitor()`, which
    /// reuses `LatLonShapeQuery`'s.
    pub fn get_spatial_visitor(&self) -> &LatLonShapeSpatialVisitor {
        &self.visitor
    }

    /// Returns the documents of `reader` this query matches.
    ///
    /// See [`BaseShapeDocValuesQuery::matching_docs`].
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the doc values raises.
    pub fn matching_docs(
        &self,
        reader: &dyn LeafReader,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        self.base.matching_docs(reader)
    }

    /// Prints this query, omitting `field` when it is the default one.
    pub fn to_query_string(&self, field: &str) -> String {
        self.base.spatial_query().to_query_string(
            "LatLonShapeDocValuesQuery",
            field,
            &self
                .geometries
                .iter()
                .map(|g| format!("{g:?}"))
                .collect::<Vec<_>>(),
        )
    }
}

/// A bounding-box query over cartesian shape doc values.
///
/// Equivalent to `org.apache.lucene.document.XYShapeDocValuesQuery`, which
/// [`XYShape::new_slow_doc_values_box_query`](crate::document::XYShape::new_slow_doc_values_box_query)
/// builds.
#[derive(Clone, Debug)]
pub struct XYShapeDocValuesQuery {
    base: BaseShapeDocValuesQuery,
    geometries: Vec<XYGeometryValue>,
    visitor: XYShapeSpatialVisitor,
}

impl XYShapeDocValuesQuery {
    /// Creates the query.
    ///
    /// Equivalent to
    /// `XYShapeDocValuesQuery(String, QueryRelation, XYGeometry...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for a `CONTAINS` relation and
    /// for an empty geometry list.
    pub fn new(
        field: impl Into<String>,
        query_relation: QueryRelation,
        geometries: Vec<XYGeometryValue>,
    ) -> Result<Self> {
        let component2d = XYGeometryValue::create(&geometries)?;
        Ok(Self {
            base: BaseShapeDocValuesQuery::new(
                field,
                query_relation,
                Arc::clone(&component2d),
                ShapeCoordinateSystem::Cartesian,
            )?,
            geometries,
            visitor: XYShapeSpatialVisitor::new(component2d),
        })
    }

    /// Returns the shared doc-values query state.
    pub fn base(&self) -> &BaseShapeDocValuesQuery {
        &self.base
    }

    /// Returns the geometries the query relates against.
    pub fn geometries(&self) -> &[XYGeometryValue] {
        &self.geometries
    }

    /// Returns the shape a stored binary value holds.
    ///
    /// Equivalent to `XYShapeDocValuesQuery.getShapeDocValues(BytesRef)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the serialised shape raises.
    pub fn get_shape_doc_values(
        &self,
        binary_value: crate::util::BytesRef,
    ) -> Result<crate::document::XYShapeDocValues> {
        crate::document::XYShapeDocValues::from_binary_value(binary_value)
    }

    /// Returns the visitor a BKD walk would use.
    ///
    /// Equivalent to `XYShapeDocValuesQuery.getSpatialVisitor()`, which reuses
    /// `XYShapeQuery`'s.
    pub fn get_spatial_visitor(&self) -> &XYShapeSpatialVisitor {
        &self.visitor
    }

    /// Returns the documents of `reader` this query matches.
    ///
    /// See [`BaseShapeDocValuesQuery::matching_docs`].
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the doc values raises.
    pub fn matching_docs(
        &self,
        reader: &dyn LeafReader,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        self.base.matching_docs(reader)
    }

    /// Prints this query, omitting `field` when it is the default one.
    pub fn to_query_string(&self, field: &str) -> String {
        self.base.spatial_query().to_query_string(
            "XYShapeDocValuesQuery",
            field,
            &self
                .geometries
                .iter()
                .map(|g| format!("{g:?}"))
                .collect::<Vec<_>>(),
        )
    }
}
