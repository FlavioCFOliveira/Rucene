//! The 2D rectangle component ported from `org.apache.lucene.geo.Rectangle2D`.

use crate::error::Result;
use crate::geo::component2d::{
    contains_point, disjoint, point_in_triangle, within, Component2D, WithinRelation,
};
use crate::geo::component_tree::ComponentTree;
use crate::geo::encoding::{GeoEncodingUtils, GeoUtils};
use crate::geo::geometry::{Rectangle, XYRectangle};
use crate::index::point_values::Relation;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// 2D rectangle implementation containing cartesian spatial logic.
///
/// Equivalent to `org.apache.lucene.geo.Rectangle2D`, which is package-private
/// in Lucene; Rust has no package-private visibility, so the type is `pub` and
/// is marked internal by this documentation instead.
#[derive(Clone, Copy, Debug)]
pub struct Rectangle2D {
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
}

impl Rectangle2D {
    fn new(min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> Self {
        Self {
            min_x,
            max_x,
            min_y,
            max_y,
        }
    }

    /// Creates a component from the provided cartesian rectangle.
    ///
    /// Equivalent to `Rectangle2D.create(XYRectangle)`.
    pub fn create_from_xy_rectangle(rectangle: &XYRectangle) -> Arc<dyn Component2D> {
        Arc::new(Rectangle2D::new(
            f64::from(rectangle.min_x()),
            f64::from(rectangle.max_x()),
            f64::from(rectangle.min_y()),
            f64::from(rectangle.max_y()),
        ))
    }

    /// Creates a component from the provided latitude/longitude rectangle.
    ///
    /// Equivalent to `Rectangle2D.create(Rectangle)`, reproducing the behaviour
    /// of `LatLonPoint.newBoxQuery()`: the bounds are quantised, and a box that
    /// crosses the dateline becomes two components.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when a bound is out of
    /// range, which the encoders reject.
    pub fn create_from_rectangle(rectangle: &Rectangle) -> Result<Arc<dyn Component2D>> {
        // behavior of LatLonPoint.newBoxQuery()
        let mut min_longitude = rectangle.min_lon();
        let mut crosses_dateline = rectangle.min_lon() > rectangle.max_lon();
        if min_longitude == 180.0 && crosses_dateline {
            min_longitude = -180.0;
            crosses_dateline = false;
        }
        // need to quantize!
        let q_min_lat = GeoEncodingUtils::decode_latitude(GeoEncodingUtils::encode_latitude_ceil(
            rectangle.min_lat(),
        )?);
        let q_max_lat = GeoEncodingUtils::decode_latitude(GeoEncodingUtils::encode_latitude(
            rectangle.max_lat(),
        )?);
        let q_min_lon = GeoEncodingUtils::decode_longitude(
            GeoEncodingUtils::encode_longitude_ceil(min_longitude)?,
        );
        let q_max_lon = GeoEncodingUtils::decode_longitude(GeoEncodingUtils::encode_longitude(
            rectangle.max_lon(),
        )?);
        if crosses_dateline {
            // for rectangles that cross the dateline we need to create two components
            let min_lon_incl_quantize =
                GeoEncodingUtils::decode_longitude(GeoEncodingUtils::min_lon_encoded());
            let max_lon_incl_quantize =
                GeoEncodingUtils::decode_longitude(GeoEncodingUtils::max_lon_encoded());
            let components: Vec<Arc<dyn Component2D>> = vec![
                Arc::new(Rectangle2D::new(
                    min_lon_incl_quantize,
                    q_max_lon,
                    q_min_lat,
                    q_max_lat,
                )),
                Arc::new(Rectangle2D::new(
                    q_min_lon,
                    max_lon_incl_quantize,
                    q_min_lat,
                    q_max_lat,
                )),
            ];
            Ok(ComponentTree::create(components))
        } else {
            Ok(Arc::new(Rectangle2D::new(
                q_min_lon, q_max_lon, q_min_lat, q_max_lat,
            )))
        }
    }

    /// Returns whether the segment `a`-`b` crosses any of the rectangle edges.
    ///
    /// Equivalent to the private `Rectangle2D.edgesIntersect(...)`.
    fn edges_intersect(&self, a_x: f64, a_y: f64, b_x: f64, b_y: f64) -> bool {
        // shortcut: check bboxes of edges are disjoint
        if a_x.max(b_x) < self.min_x
            || a_x.min(b_x) > self.max_x
            || a_y.min(b_y) > self.max_y
            || a_y.max(b_y) < self.min_y
        {
            return false;
        }
        // top
        GeoUtils::line_crosses_line_with_boundary(
            a_x, a_y, b_x, b_y, self.min_x, self.max_y, self.max_x, self.max_y,
        )
        // bottom
        || GeoUtils::line_crosses_line_with_boundary(
            a_x, a_y, b_x, b_y, self.max_x, self.max_y, self.max_x, self.min_y,
        )
        // left
        || GeoUtils::line_crosses_line_with_boundary(
            a_x, a_y, b_x, b_y, self.max_x, self.min_y, self.min_x, self.min_y,
        )
        // right
        || GeoUtils::line_crosses_line_with_boundary(
            a_x, a_y, b_x, b_y, self.min_x, self.min_y, self.min_x, self.max_y,
        )
    }
}

impl PartialEq for Rectangle2D {
    /// Equivalent to `Rectangle2D.equals(Object)`, which compares with
    /// `Double.compare`; [`f64::total_cmp`] gives that exact ordering.
    fn eq(&self, other: &Self) -> bool {
        self.min_x.total_cmp(&other.min_x).is_eq()
            && self.max_x.total_cmp(&other.max_x).is_eq()
            && self.min_y.total_cmp(&other.min_y).is_eq()
            && self.max_y.total_cmp(&other.max_y).is_eq()
    }
}

impl Eq for Rectangle2D {}

impl Hash for Rectangle2D {
    /// Equivalent to `Rectangle2D.hashCode()`, which hashes the four bounds.
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.min_x.to_bits().hash(state);
        self.max_x.to_bits().hash(state);
        self.min_y.to_bits().hash(state);
        self.max_y.to_bits().hash(state);
    }
}

impl fmt::Display for Rectangle2D {
    /// Equivalent to `Rectangle2D.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Rectangle2D(x={} TO {} y={} TO {})",
            self.min_x, self.max_x, self.min_y, self.max_y
        )
    }
}

impl Component2D for Rectangle2D {
    fn get_min_x(&self) -> f64 {
        self.min_x
    }

    fn get_max_x(&self) -> f64 {
        self.max_x
    }

    fn get_min_y(&self) -> f64 {
        self.min_y
    }

    fn get_max_y(&self) -> f64 {
        self.max_y
    }

    fn contains(&self, x: f64, y: f64) -> bool {
        contains_point(x, y, self.min_x, self.max_x, self.min_y, self.max_y)
    }

    fn relate(&self, min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> Relation {
        if disjoint(
            self.min_x, self.max_x, self.min_y, self.max_y, min_x, max_x, min_y, max_y,
        ) {
            return Relation::CellOutsideQuery;
        }
        if within(
            min_x, max_x, min_y, max_y, self.min_x, self.max_x, self.min_y, self.max_y,
        ) {
            return Relation::CellInsideQuery;
        }
        Relation::CellCrossesQuery
    }

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
    ) -> bool {
        if disjoint(
            self.min_x, self.max_x, self.min_y, self.max_y, min_x, max_x, min_y, max_y,
        ) {
            return false;
        }
        self.contains(a_x, a_y)
            || self.contains(b_x, b_y)
            || self.edges_intersect(a_x, a_y, b_x, b_y)
    }

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
    ) -> bool {
        if disjoint(
            self.min_x, self.max_x, self.min_y, self.max_y, min_x, max_x, min_y, max_y,
        ) {
            return false;
        }
        self.contains(a_x, a_y)
            || self.contains(b_x, b_y)
            || self.contains(c_x, c_y)
            || point_in_triangle(
                min_x, max_x, min_y, max_y, self.min_x, self.min_y, a_x, a_y, b_x, b_y, c_x, c_y,
            )
            || self.edges_intersect(a_x, a_y, b_x, b_y)
            || self.edges_intersect(b_x, b_y, c_x, c_y)
            || self.edges_intersect(c_x, c_y, a_x, a_y)
    }

    fn contains_line(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        _a_x: f64,
        _a_y: f64,
        _b_x: f64,
        _b_y: f64,
    ) -> bool {
        within(
            min_x, max_x, min_y, max_y, self.min_x, self.max_x, self.min_y, self.max_y,
        )
    }

    fn contains_triangle(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        _a_x: f64,
        _a_y: f64,
        _b_x: f64,
        _b_y: f64,
        _c_x: f64,
        _c_y: f64,
    ) -> bool {
        within(
            min_x, max_x, min_y, max_y, self.min_x, self.max_x, self.min_y, self.max_y,
        )
    }

    fn within_point(&self, x: f64, y: f64) -> WithinRelation {
        if self.contains(x, y) {
            WithinRelation::NotWithin
        } else {
            WithinRelation::Disjoint
        }
    }

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
    ) -> WithinRelation {
        if disjoint(
            self.min_x, self.max_x, self.min_y, self.max_y, min_x, max_x, min_y, max_y,
        ) {
            return WithinRelation::Disjoint;
        }
        if self.contains(a_x, a_y) || self.contains(b_x, b_y) {
            return WithinRelation::NotWithin;
        }
        if ab && self.edges_intersect(a_x, a_y, b_x, b_y) {
            return WithinRelation::NotWithin;
        }
        WithinRelation::Disjoint
    }

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
    ) -> WithinRelation {
        // Bounding boxes disjoint?
        if disjoint(
            self.min_x, self.max_x, self.min_y, self.max_y, min_x, max_x, min_y, max_y,
        ) {
            return WithinRelation::Disjoint;
        }

        // Points belong to the shape so if points are inside the rectangle then it cannot be
        // within.
        if self.contains(a_x, a_y) || self.contains(b_x, b_y) || self.contains(c_x, c_y) {
            return WithinRelation::NotWithin;
        }
        // If any of the edges intersects an edge belonging to the shape then it cannot be within.
        let mut relation = WithinRelation::Disjoint;
        if self.edges_intersect(a_x, a_y, b_x, b_y) {
            if ab {
                return WithinRelation::NotWithin;
            }
            relation = WithinRelation::Candidate;
        }
        if self.edges_intersect(b_x, b_y, c_x, c_y) {
            if bc {
                return WithinRelation::NotWithin;
            }
            relation = WithinRelation::Candidate;
        }
        if self.edges_intersect(c_x, c_y, a_x, a_y) {
            if ca {
                return WithinRelation::NotWithin;
            }
            relation = WithinRelation::Candidate;
        }
        // If any of the rectangle edges crosses a triangle edge that does not belong to the shape
        // then it is a candidate for within
        if relation == WithinRelation::Candidate {
            return WithinRelation::Candidate;
        }
        // Check if shape is within the triangle
        if point_in_triangle(
            min_x, max_x, min_y, max_y, self.min_x, self.min_y, a_x, a_y, b_x, b_y, c_x, c_y,
        ) {
            return WithinRelation::Candidate;
        }
        relation
    }
}
