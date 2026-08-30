//! The 2D polygon component ported from `org.apache.lucene.geo.Polygon2D`.

use crate::error::Result;
use crate::geo::component2d::{
    contains_point, disjoint, point_in_triangle, within, Component2D, WithinRelation,
};
use crate::geo::edge_tree::EdgeTree;
use crate::geo::encoding::XYEncodingUtils;
use crate::geo::geometry::{LatLonGeometry, Polygon, XYGeometry, XYPolygon};
use crate::index::point_values::Relation;
use std::sync::Arc;

/// 2D polygon implementation represented as a balanced interval tree of edges.
///
/// Equivalent to `org.apache.lucene.geo.Polygon2D`, which is package-private in
/// Lucene; Rust has no package-private visibility, so the type is `pub` and is
/// marked internal by this documentation instead.
///
/// Loosely based on the algorithm described in
/// <http://www-ma2.upc.es/geoc/Schirra-pointPolygon.pdf>.
#[derive(Debug)]
pub struct Polygon2D {
    /// Minimum Y of this geometry's bounding box area.
    min_y: f64,
    /// Maximum Y of this geometry's bounding box area.
    max_y: f64,
    /// Minimum X of this geometry's bounding box area.
    min_x: f64,
    /// Maximum X of this geometry's bounding box area.
    max_x: f64,
    /// Tree of holes, if any.
    holes: Option<Arc<dyn Component2D>>,
    /// Edges of the polygon represented as a 2-d interval tree.
    tree: EdgeTree,
}

impl Polygon2D {
    #[allow(clippy::too_many_arguments)]
    fn new(
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        x: &[f64],
        y: &[f64],
        holes: Option<Arc<dyn Component2D>>,
    ) -> Self {
        Self {
            min_y,
            max_y,
            min_x,
            max_x,
            holes,
            tree: EdgeTree::create_tree(x, y),
        }
    }

    /// Builds a `Polygon2D` from a latitude/longitude polygon.
    ///
    /// Equivalent to `Polygon2D.create(Polygon)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when a hole cannot be
    /// turned into a component.
    pub fn create_from_polygon(polygon: &Polygon) -> Result<Arc<dyn Component2D>> {
        let gon_holes = polygon.get_holes();
        let holes = if !gon_holes.is_empty() {
            let refs: Vec<&dyn LatLonGeometry> =
                gon_holes.iter().map(|h| h as &dyn LatLonGeometry).collect();
            Some(<dyn LatLonGeometry>::create(&refs)?)
        } else {
            None
        };
        Ok(Arc::new(Polygon2D::new(
            polygon.min_lon(),
            polygon.max_lon(),
            polygon.min_lat(),
            polygon.max_lat(),
            polygon.get_polylons(),
            polygon.get_polylats(),
            holes,
        )))
    }

    /// Builds a `Polygon2D` from a cartesian polygon.
    ///
    /// Equivalent to `Polygon2D.create(XYPolygon)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when a hole cannot be
    /// turned into a component.
    pub fn create_from_xy_polygon(polygon: &XYPolygon) -> Result<Arc<dyn Component2D>> {
        let gon_holes = polygon.get_holes();
        let holes = if !gon_holes.is_empty() {
            let refs: Vec<&dyn XYGeometry> =
                gon_holes.iter().map(|h| h as &dyn XYGeometry).collect();
            Some(<dyn XYGeometry>::create(&refs)?)
        } else {
            None
        };
        Ok(Arc::new(Polygon2D::new(
            f64::from(polygon.min_x()),
            f64::from(polygon.max_x()),
            f64::from(polygon.min_y()),
            f64::from(polygon.max_y()),
            &XYEncodingUtils::float_array_to_double_array(polygon.get_polyx()),
            &XYEncodingUtils::float_array_to_double_array(polygon.get_polyy()),
            holes,
        )))
    }

    /// Returns 0, 4, or something in between.
    ///
    /// Equivalent to the private `Polygon2D.numberOfCorners(...)`.
    fn number_of_corners(&self, min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> i32 {
        let mut contains_count = 0;
        if self.contains(min_x, min_y) {
            contains_count += 1;
        }
        if self.contains(max_x, min_y) {
            contains_count += 1;
        }
        if contains_count == 1 {
            return contains_count;
        }
        if self.contains(max_x, max_y) {
            contains_count += 1;
        }
        if contains_count == 2 {
            return contains_count;
        }
        if self.contains(min_x, max_y) {
            contains_count += 1;
        }
        contains_count
    }
}

impl Component2D for Polygon2D {
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

    /// Returns whether the point is contained within this polygon.
    ///
    /// See <https://www.ecse.rpi.edu/~wrf/Research/Short_Notes/pnpoly.html> for
    /// more information.
    fn contains(&self, x: f64, y: f64) -> bool {
        if contains_point(x, y, self.min_x, self.max_x, self.min_y, self.max_y)
            && self.tree.contains(x, y)
        {
            return match &self.holes {
                None => true,
                Some(holes) => !holes.contains(x, y),
            };
        }
        false
    }

    fn relate(&self, min_x: f64, max_x: f64, min_y: f64, max_y: f64) -> Relation {
        if disjoint(
            self.min_x, self.max_x, self.min_y, self.max_y, min_x, max_x, min_y, max_y,
        ) {
            return Relation::CellOutsideQuery;
        }
        if within(
            self.min_x, self.max_x, self.min_y, self.max_y, min_x, max_x, min_y, max_y,
        ) {
            return Relation::CellCrossesQuery;
        }
        // check any holes
        if let Some(holes) = &self.holes {
            let hole_relation = holes.relate(min_x, max_x, min_y, max_y);
            if hole_relation == Relation::CellCrossesQuery {
                return Relation::CellCrossesQuery;
            } else if hole_relation == Relation::CellInsideQuery {
                return Relation::CellOutsideQuery;
            }
        }
        // check each corner: if < 4 && > 0 are present, it is cheaper than crossesSlowly
        let num_corners = self.number_of_corners(min_x, max_x, min_y, max_y);
        if num_corners == 4 {
            if self.tree.crosses_box(min_x, max_x, min_y, max_y, true) {
                return Relation::CellCrossesQuery;
            }
            return Relation::CellInsideQuery;
        } else if num_corners == 0 {
            if contains_point(self.tree.x1, self.tree.y1, min_x, max_x, min_y, max_y) {
                return Relation::CellCrossesQuery;
            }
            if self.tree.crosses_box(min_x, max_x, min_y, max_y, true) {
                return Relation::CellCrossesQuery;
            }
            return Relation::CellOutsideQuery;
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
        if self.contains(a_x, a_y)
            || self.contains(b_x, b_y)
            || self
                .tree
                .crosses_line(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, true)
        {
            return match &self.holes {
                None => true,
                Some(holes) => !holes.contains_line(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y),
            };
        }
        false
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
        if self.contains(a_x, a_y)
            || self.contains(b_x, b_y)
            || self.contains(c_x, c_y)
            || point_in_triangle(
                min_x,
                max_x,
                min_y,
                max_y,
                self.tree.x1,
                self.tree.y1,
                a_x,
                a_y,
                b_x,
                b_y,
                c_x,
                c_y,
            )
            || self.tree.crosses_triangle(
                min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, c_x, c_y, true,
            )
        {
            return match &self.holes {
                None => true,
                Some(holes) => !holes
                    .contains_triangle(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, c_x, c_y),
            };
        }
        false
    }

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
    ) -> bool {
        if disjoint(
            self.min_x, self.max_x, self.min_y, self.max_y, min_x, max_x, min_y, max_y,
        ) {
            return false;
        }
        if self.contains(a_x, a_y)
            && self.contains(b_x, b_y)
            && !self
                .tree
                .crosses_line(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, false)
        {
            return match &self.holes {
                None => true,
                Some(holes) => {
                    !holes.intersects_line(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y)
                }
            };
        }
        false
    }

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
    ) -> bool {
        if disjoint(
            self.min_x, self.max_x, self.min_y, self.max_y, min_x, max_x, min_y, max_y,
        ) {
            return false;
        }
        if self.contains(a_x, a_y)
            && self.contains(b_x, b_y)
            && self.contains(c_x, c_y)
            && !self.tree.crosses_triangle(
                min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, c_x, c_y, false,
            )
        {
            return match &self.holes {
                None => true,
                Some(holes) => !holes
                    .intersects_triangle(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, c_x, c_y),
            };
        }
        false
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
        if ab
            && self
                .tree
                .crosses_line(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, true)
        {
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
        if disjoint(
            self.min_x, self.max_x, self.min_y, self.max_y, min_x, max_x, min_y, max_y,
        ) {
            return WithinRelation::Disjoint;
        }

        // if any of the points is inside the polygon, the polygon cannot be within this indexed
        // shape because points belong to the original indexed shape.
        if self.contains(a_x, a_y) || self.contains(b_x, b_y) || self.contains(c_x, c_y) {
            return WithinRelation::NotWithin;
        }

        let mut relation = WithinRelation::Disjoint;
        // if any of the edges intersects and the edge belongs to the shape then it cannot be
        // within. If it only intersects edges that do not belong to the shape, then it is a
        // candidate. We skip edges at the dateline to support shapes crossing it.
        if self
            .tree
            .crosses_line(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, true)
        {
            if ab {
                return WithinRelation::NotWithin;
            }
            relation = WithinRelation::Candidate;
        }

        if self
            .tree
            .crosses_line(min_x, max_x, min_y, max_y, b_x, b_y, c_x, c_y, true)
        {
            if bc {
                return WithinRelation::NotWithin;
            }
            relation = WithinRelation::Candidate;
        }
        if self
            .tree
            .crosses_line(min_x, max_x, min_y, max_y, c_x, c_y, a_x, a_y, true)
        {
            if ca {
                return WithinRelation::NotWithin;
            }
            relation = WithinRelation::Candidate;
        }

        // if any of the edges crosses an edge that does not belong to the shape then it is a
        // candidate for within
        if relation == WithinRelation::Candidate {
            return WithinRelation::Candidate;
        }

        // Check if shape is within the triangle
        if point_in_triangle(
            min_x,
            max_x,
            min_y,
            max_y,
            self.tree.x1,
            self.tree.y1,
            a_x,
            a_y,
            b_x,
            b_y,
            c_x,
            c_y,
        ) {
            return WithinRelation::Candidate;
        }
        relation
    }
}
