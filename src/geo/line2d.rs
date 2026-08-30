//! The 2D line component ported from `org.apache.lucene.geo.Line2D`.

use crate::geo::component2d::{
    contains_point, disjoint, point_in_triangle, within, Component2D, WithinRelation,
};
use crate::geo::edge_tree::EdgeTree;
use crate::geo::encoding::XYEncodingUtils;
use crate::geo::geometry::{Line, XYLine};
use crate::index::point_values::Relation;
use std::sync::Arc;

/// 2D geo line implementation represented as a balanced interval tree of edges.
///
/// Equivalent to `org.apache.lucene.geo.Line2D`, which is package-private in
/// Lucene; Rust has no package-private visibility, so the type is `pub` and is
/// marked internal by this documentation instead.
///
/// Construction takes `O(n log n)` for sorting and tree building;
/// [`Component2D::relate`] is `O(n)` in the worst case but much faster than
/// brute force for most practical lines.
#[derive(Debug)]
pub struct Line2D {
    /// Minimum Y of this geometry's bounding box area.
    min_y: f64,
    /// Maximum Y of this geometry's bounding box area.
    max_y: f64,
    /// Minimum X of this geometry's bounding box area.
    min_x: f64,
    /// Maximum X of this geometry's bounding box area.
    max_x: f64,
    /// Lines represented as a 2-d interval tree.
    tree: EdgeTree,
}

impl Line2D {
    /// Creates a `Line2D` from the provided latitude/longitude linestring.
    ///
    /// Equivalent to `Line2D.create(Line)`.
    pub fn create_from_line(line: &Line) -> Arc<dyn Component2D> {
        Arc::new(Line2D {
            min_y: line.min_lat(),
            max_y: line.max_lat(),
            min_x: line.min_lon(),
            max_x: line.max_lon(),
            tree: EdgeTree::create_tree(line.lons(), line.lats()),
        })
    }

    /// Creates a `Line2D` from the provided cartesian linestring.
    ///
    /// Equivalent to `Line2D.create(XYLine)`.
    pub fn create_from_xy_line(line: &XYLine) -> Arc<dyn Component2D> {
        Arc::new(Line2D {
            min_y: f64::from(line.min_y()),
            max_y: f64::from(line.max_y()),
            min_x: f64::from(line.min_x()),
            max_x: f64::from(line.max_x()),
            tree: EdgeTree::create_tree(
                &XYEncodingUtils::float_array_to_double_array(line.get_x()),
                &XYEncodingUtils::float_array_to_double_array(line.get_y()),
            ),
        })
    }
}

impl Component2D for Line2D {
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
        if contains_point(x, y, self.min_x, self.max_x, self.min_y, self.max_y) {
            return self.tree.is_point_on_line(x, y);
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
        if self.tree.crosses_box(min_x, max_x, min_y, max_y, true) {
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
        if disjoint(
            self.min_x, self.max_x, self.min_y, self.max_y, min_x, max_x, min_y, max_y,
        ) {
            return false;
        }
        self.tree
            .crosses_line(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, true)
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
        point_in_triangle(
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
        ) || self.tree.crosses_triangle(
            min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y, c_x, c_y, true,
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
        // can be improved?
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
        if ab && self.intersects_line(min_x, max_x, min_y, max_y, a_x, a_y, b_x, b_y) {
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
