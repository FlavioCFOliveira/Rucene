//! The 2D point component ported from `org.apache.lucene.geo.Point2D`.

use crate::error::Result;
use crate::geo::component2d::{contains_point, point_in_triangle, Component2D, WithinRelation};
use crate::geo::encoding::{GeoEncodingUtils, GeoUtils};
use crate::geo::geometry::{Point, XYPoint};
use crate::index::point_values::Relation;
use std::sync::Arc;

/// 2D point implementation containing geo spatial logic.
///
/// Equivalent to `org.apache.lucene.geo.Point2D`, which is package-private in
/// Lucene; Rust has no package-private visibility, so the type is `pub` and is
/// marked internal by this documentation instead.
#[derive(Clone, Copy, Debug)]
pub struct Point2D {
    x: f64,
    y: f64,
}

impl Point2D {
    fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Creates a component from a latitude/longitude point.
    ///
    /// Equivalent to `Point2D.create(Point)`. Points behave as rectangles, so
    /// both coordinates are quantised the way a `LatLonPoint` box query
    /// quantises them.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when a coordinate is out
    /// of range, which the encoders reject.
    pub fn create_from_point(point: &Point) -> Result<Arc<dyn Component2D>> {
        // Points behave as rectangles
        let q_lat = if point.lat() == GeoUtils::MAX_LAT_INCL {
            point.lat()
        } else {
            GeoEncodingUtils::decode_latitude(GeoEncodingUtils::encode_latitude_ceil(point.lat())?)
        };
        let q_lon = if point.lon() == GeoUtils::MAX_LON_INCL {
            point.lon()
        } else {
            GeoEncodingUtils::decode_longitude(GeoEncodingUtils::encode_longitude_ceil(
                point.lon(),
            )?)
        };
        Ok(Arc::new(Point2D::new(q_lon, q_lat)))
    }

    /// Creates a component from a cartesian point.
    ///
    /// Equivalent to `Point2D.create(XYPoint)`.
    pub fn create_from_xy_point(xy_point: &XYPoint) -> Arc<dyn Component2D> {
        Arc::new(Point2D::new(
            f64::from(xy_point.get_x()),
            f64::from(xy_point.get_y()),
        ))
    }
}

impl Component2D for Point2D {
    fn get_min_x(&self) -> f64 {
        self.x
    }

    fn get_max_x(&self) -> f64 {
        self.x
    }

    fn get_min_y(&self) -> f64 {
        self.y
    }

    fn get_max_y(&self) -> f64 {
        self.y
    }

    fn contains(&self, x: f64, y: f64) -> bool {
        x == self.x && y == self.y
    }

    fn relate(&self, min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> Relation {
        if contains_point(self.x, self.y, min_x, max_x, min_y, max_y) {
            return Relation::CellCrossesQuery;
        }
        Relation::CellOutsideQuery
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
        contains_point(self.x, self.y, min_x, max_x, min_y, max_y)
            && GeoUtils::orient(a_x, a_y, b_x, b_y, self.x, self.y) == 0
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
        point_in_triangle(
            min_x, max_x, min_y, max_y, self.x, self.y, a_x, a_y, b_x, b_y, c_x, c_y,
        )
    }

    fn contains_line(
        &self,
        _min_x: f64,
        _max_x: f64,
        _min_y: f64,
        _max_y: f64,
        _a_x: f64,
        _a_y: f64,
        _b_x: f64,
        _b_y: f64,
    ) -> bool {
        false
    }

    fn contains_triangle(
        &self,
        _min_x: f64,
        _max_x: f64,
        _min_y: f64,
        _max_y: f64,
        _a_x: f64,
        _a_y: f64,
        _b_x: f64,
        _b_y: f64,
        _c_x: f64,
        _c_y: f64,
    ) -> bool {
        false
    }

    fn within_point(&self, x: f64, y: f64) -> WithinRelation {
        if self.contains(x, y) {
            WithinRelation::Candidate
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
        _ab: bool,
        b_x: f64,
        b_y: f64,
    ) -> WithinRelation {
        // can be improved?
        if self.intersects_line(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y) {
            WithinRelation::Candidate
        } else {
            WithinRelation::Disjoint
        }
    }

    fn within_triangle(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        a_x: f64,
        a_y: f64,
        _ab: bool,
        b_x: f64,
        b_y: f64,
        _bc: bool,
        c_x: f64,
        c_y: f64,
        _ca: bool,
    ) -> WithinRelation {
        if point_in_triangle(
            min_x, max_x, min_y, max_y, self.x, self.y, a_x, a_y, b_x, b_y, c_x, c_y,
        ) {
            return WithinRelation::Candidate;
        }
        WithinRelation::Disjoint
    }
}
