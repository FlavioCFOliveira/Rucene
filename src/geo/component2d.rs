//! The 2D spatial-relation interface ported from `org.apache.lucene.geo`.
//!
//! Every geometry a spatial query is built from is turned into a
//! [`Component2D`], which answers the relation questions the BKD tree asks
//! while it descends: does the component contain this point, how does it relate
//! to this cell, does it intersect or contain this indexed triangle.

use crate::geo::encoding::GeoUtils;
use crate::index::point_values::Relation;
use std::fmt;

/// Relation of a query shape to an indexed triangle, used by
/// [`Component2D::within_triangle`].
///
/// Equivalent to `org.apache.lucene.geo.Component2D.WithinRelation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WithinRelation {
    /// The shape is a candidate for within. Typically returned when the query
    /// shape is fully inside the triangle, or when it intersects only edges
    /// that do not belong to the original shape.
    Candidate,
    /// The query shape intersects an edge that does belong to the original
    /// shape, or any point of the triangle is inside the shape.
    NotWithin,
    /// The query shape is disjoint with the triangle.
    Disjoint,
}

/// A 2D geometry supporting spatial relationships with bounding boxes,
/// triangles and points.
///
/// Equivalent to `org.apache.lucene.geo.Component2D` (`@lucene.internal`).
///
/// Java's interface carries no thread-safety marker; every implementation is
/// immutable after construction and is shared across search threads, so this
/// port states that explicitly with the `Send + Sync` supertraits, and adds
/// [`fmt::Debug`] because Rust cannot fall back to `Object.toString()`.
pub trait Component2D: fmt::Debug + Send + Sync {
    /// Minimum X value of the component.
    ///
    /// Equivalent to `Component2D.getMinX()`.
    fn get_min_x(&self) -> f64;

    /// Maximum X value of the component.
    ///
    /// Equivalent to `Component2D.getMaxX()`.
    fn get_max_x(&self) -> f64;

    /// Minimum Y value of the component.
    ///
    /// Equivalent to `Component2D.getMinY()`.
    fn get_min_y(&self) -> f64;

    /// Maximum Y value of the component.
    ///
    /// Equivalent to `Component2D.getMaxY()`.
    fn get_max_y(&self) -> f64;

    /// Relates this component with a point.
    ///
    /// Equivalent to `Component2D.contains(double, double)`.
    fn contains(&self, x: f64, y: f64) -> bool;

    /// Relates this component with a bounding box.
    ///
    /// Equivalent to `Component2D.relate(double, double, double, double)`.
    fn relate(&self, min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> Relation;

    /// Returns whether this component intersects the provided line.
    ///
    /// Equivalent to the eight-argument `Component2D.intersectsLine(...)`.
    #[allow(clippy::too_many_arguments)]
    fn intersects_line(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        a_x: f64,
        a_y: f64,
        b_x: f64,
        b_y: f64,
    ) -> bool;

    /// Returns whether this component intersects the provided triangle.
    ///
    /// Equivalent to the ten-argument `Component2D.intersectsTriangle(...)`.
    #[allow(clippy::too_many_arguments)]
    fn intersects_triangle(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        a_x: f64,
        a_y: f64,
        b_x: f64,
        b_y: f64,
        c_x: f64,
        c_y: f64,
    ) -> bool;

    /// Returns whether this component contains the provided line.
    ///
    /// Equivalent to the eight-argument `Component2D.containsLine(...)`.
    #[allow(clippy::too_many_arguments)]
    fn contains_line(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        a_x: f64,
        a_y: f64,
        b_x: f64,
        b_y: f64,
    ) -> bool;

    /// Returns whether this component contains the provided triangle.
    ///
    /// Equivalent to the ten-argument `Component2D.containsTriangle(...)`.
    #[allow(clippy::too_many_arguments)]
    fn contains_triangle(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        a_x: f64,
        a_y: f64,
        b_x: f64,
        b_y: f64,
        c_x: f64,
        c_y: f64,
    ) -> bool;

    /// Computes the within relation of this component with a point.
    ///
    /// Equivalent to `Component2D.withinPoint(double, double)`.
    fn within_point(&self, x: f64, y: f64) -> WithinRelation;

    /// Computes the within relation of this component with a line.
    ///
    /// Equivalent to the nine-argument `Component2D.withinLine(...)`.
    #[allow(clippy::too_many_arguments)]
    fn within_line(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        a_x: f64,
        a_y: f64,
        ab: bool,
        b_x: f64,
        b_y: f64,
    ) -> WithinRelation;

    /// Computes the within relation of this component with a triangle.
    ///
    /// Equivalent to the thirteen-argument `Component2D.withinTriangle(...)`.
    #[allow(clippy::too_many_arguments)]
    fn within_triangle(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        a_x: f64,
        a_y: f64,
        ab: bool,
        b_x: f64,
        b_y: f64,
        bc: bool,
        c_x: f64,
        c_y: f64,
        ca: bool,
    ) -> WithinRelation;

    /// Returns whether this component intersects the provided line, computing
    /// the line's bounding box first.
    ///
    /// Equivalent to the four-argument default method
    /// `Component2D.intersectsLine(double, double, double, double)`.
    fn intersects_line_bbox(&self, a_x: f64, a_y: f64, b_x: f64, b_y: f64) -> bool {
        let min_y = a_y.min(b_y);
        let min_x = a_x.min(b_x);
        let max_y = a_y.max(b_y);
        let max_x = a_x.max(b_x);
        self.intersects_line(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y)
    }

    /// Returns whether this component intersects the provided triangle,
    /// computing the triangle's bounding box first.
    ///
    /// Equivalent to the six-argument default method
    /// `Component2D.intersectsTriangle(...)`.
    fn intersects_triangle_bbox(
        &self,
        a_x: f64,
        a_y: f64,
        b_x: f64,
        b_y: f64,
        c_x: f64,
        c_y: f64,
    ) -> bool {
        let min_y = a_y.min(b_y).min(c_y);
        let min_x = a_x.min(b_x).min(c_x);
        let max_y = a_y.max(b_y).max(c_y);
        let max_x = a_x.max(b_x).max(c_x);
        self.intersects_triangle(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, c_x, c_y)
    }

    /// Returns whether this component contains the provided line, computing the
    /// line's bounding box first.
    ///
    /// Equivalent to the four-argument default method
    /// `Component2D.containsLine(double, double, double, double)`.
    fn contains_line_bbox(&self, a_x: f64, a_y: f64, b_x: f64, b_y: f64) -> bool {
        let min_y = a_y.min(b_y);
        let min_x = a_x.min(b_x);
        let max_y = a_y.max(b_y);
        let max_x = a_x.max(b_x);
        self.contains_line(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y)
    }

    /// Returns whether this component contains the provided triangle, computing
    /// the triangle's bounding box first.
    ///
    /// Equivalent to the six-argument default method
    /// `Component2D.containsTriangle(...)`.
    fn contains_triangle_bbox(
        &self,
        a_x: f64,
        a_y: f64,
        b_x: f64,
        b_y: f64,
        c_x: f64,
        c_y: f64,
    ) -> bool {
        let min_y = a_y.min(b_y).min(c_y);
        let min_x = a_x.min(b_x).min(c_x);
        let max_y = a_y.max(b_y).max(c_y);
        let max_x = a_x.max(b_x).max(c_x);
        self.contains_triangle(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, c_x, c_y)
    }

    /// Computes the within relation of this component with a line, computing
    /// the line's bounding box first.
    ///
    /// Equivalent to the five-argument default method
    /// `Component2D.withinLine(...)`.
    fn within_line_bbox(&self, a_x: f64, a_y: f64, ab: bool, b_x: f64, b_y: f64) -> WithinRelation {
        let min_y = a_y.min(b_y);
        let min_x = a_x.min(b_x);
        let max_y = a_y.max(b_y);
        let max_x = a_x.max(b_x);
        self.within_line(min_x, max_x, min_y, max_y, a_x, a_y, ab, b_x, b_y)
    }

    /// Computes the within relation of this component with a triangle,
    /// computing the triangle's bounding box first.
    ///
    /// Equivalent to the nine-argument default method
    /// `Component2D.withinTriangle(...)`.
    #[allow(clippy::too_many_arguments)]
    fn within_triangle_bbox(
        &self,
        a_x: f64,
        a_y: f64,
        ab: bool,
        b_x: f64,
        b_y: f64,
        bc: bool,
        c_x: f64,
        c_y: f64,
        ca: bool,
    ) -> WithinRelation {
        let min_y = a_y.min(b_y).min(c_y);
        let min_x = a_x.min(b_x).min(c_x);
        let max_y = a_y.max(b_y).max(c_y);
        let max_x = a_x.max(b_x).max(c_x);
        self.within_triangle(
            min_x, max_x, min_y, max_y, a_x, a_y, ab, b_x, b_y, bc, c_x, c_y, ca,
        )
    }
}

/// Returns whether the two bounding boxes are disjoint.
///
/// Equivalent to the static `Component2D.disjoint(...)`.
#[allow(clippy::too_many_arguments)]
pub fn disjoint(
    min_x1: f64,
    max_x1: f64,
    min_y1: f64,
    max_y1: f64,
    min_x2: f64,
    max_x2: f64,
    min_y2: f64,
    max_y2: f64,
) -> bool {
    max_y1 < min_y2 || min_y1 > max_y2 || max_x1 < min_x2 || min_x1 > max_x2
}

/// Returns whether the first bounding box is within the second.
///
/// Equivalent to the static `Component2D.within(...)`.
#[allow(clippy::too_many_arguments)]
pub fn within(
    min_x1: f64,
    max_x1: f64,
    min_y1: f64,
    max_y1: f64,
    min_x2: f64,
    max_x2: f64,
    min_y2: f64,
    max_y2: f64,
) -> bool {
    min_y2 <= min_y1 && max_y2 >= max_y1 && min_x2 <= min_x1 && max_x2 >= max_x1
}

/// Returns whether the rectangle given by the four bounds contains `(x, y)`.
///
/// Equivalent to the static `Component2D.containsPoint(...)`.
pub fn contains_point(x: f64, y: f64, min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> bool {
    x >= min_x && x <= max_x && y >= min_y && y <= max_y
}

/// Returns whether `(x, y)` lies in the triangle, using the winding-order
/// method.
///
/// Equivalent to the static `Component2D.pointInTriangle(...)`.
#[allow(clippy::too_many_arguments)]
pub fn point_in_triangle(
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
    x: f64,
    y: f64,
    a_x: f64,
    a_y: f64,
    b_x: f64,
    b_y: f64,
    c_x: f64,
    c_y: f64,
) -> bool {
    // check the bounding box because if the triangle is degenerated, e.g. points and lines, we
    // need to filter out coplanar points that are not part of the triangle.
    if x >= min_x && x <= max_x && y >= min_y && y <= max_y {
        let a = GeoUtils::orient(x, y, a_x, a_y, b_x, b_y);
        let b = GeoUtils::orient(x, y, b_x, b_y, c_x, c_y);
        if a == 0 || b == 0 || (a < 0) == (b < 0) {
            let c = GeoUtils::orient(x, y, c_x, c_y, a_x, a_y);
            return c == 0 || ((c < 0) == (b < 0 || a < 0));
        }
        false
    } else {
        false
    }
}
