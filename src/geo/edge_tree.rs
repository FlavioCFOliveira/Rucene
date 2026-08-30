//! The edge interval tree ported from `org.apache.lucene.geo.EdgeTree`.

use crate::geo::encoding::GeoUtils;
use crate::geo::geometry::Rectangle;
use std::cmp::Ordering;

/// Result of the point-in-polygon crossing count.
///
/// Equivalent to the `FALSE` / `TRUE` / `ON_EDGE` bytes in
/// `org.apache.lucene.geo.EdgeTree`.
const FALSE: u8 = 0x00;
const TRUE: u8 = 0x01;
const ON_EDGE: u8 = 0x02;

/// Internal tree node representing a geometry edge from `(x1, y1)` to
/// `(x2, y2)`.
///
/// Equivalent to `org.apache.lucene.geo.EdgeTree`, which is package-private in
/// Lucene; Rust has no package-private visibility, so the type is `pub` and is
/// marked internal by this documentation instead.
///
/// The sort value is `low`, the minimum Y of the edge; `max` stores the maximum
/// Y of this edge or of any of its children. Construction takes `O(n log n)`
/// for sorting and tree building; the query methods are `O(n)` in the worst
/// case but are much faster than brute force for most practical geometries.
#[derive(Clone, Debug)]
pub struct EdgeTree {
    /// Y of the first vertex, in original order.
    pub y1: f64,
    /// Y of the second vertex, in original order.
    pub y2: f64,
    /// X of the first vertex, in original order.
    pub x1: f64,
    /// X of the second vertex, in original order.
    pub x2: f64,
    /// Minimum Y of this edge.
    pub low: f64,
    /// Maximum Y of this edge or of any of its children.
    pub max: f64,
    /// Left child edge, if any.
    left: Option<Box<EdgeTree>>,
    /// Right child edge, if any.
    right: Option<Box<EdgeTree>>,
}

impl EdgeTree {
    fn new(x1: f64, y1: f64, x2: f64, y2: f64, low: f64, max: f64) -> Self {
        Self {
            y1,
            y2,
            x1,
            x2,
            low,
            max,
            left: None,
            right: None,
        }
    }

    /// Returns whether the point is on an edge or crosses the edge subtree an
    /// odd number of times.
    ///
    /// Equivalent to `EdgeTree.contains(double, double)`.
    pub fn contains(&self, x: f64, y: f64) -> bool {
        self.contains_pn_poly(x, y) > FALSE
    }

    /// Returns `0x00` when the point crosses this edge subtree an even number
    /// of times, `0x01` when it crosses an odd number of times, and `0x02` when
    /// the point lies on one of the edges.
    ///
    /// Equivalent to `EdgeTree.containsPnPoly(double, double)`, itself ported
    /// from W. Randolph Franklin's `pnpoly` (BSD licensed); see
    /// <https://www.ecse.rpi.edu/~wrf/Research/Short_Notes/pnpoly.html>.
    fn contains_pn_poly(&self, x: f64, y: f64) -> u8 {
        let mut res = FALSE;
        if y <= self.max {
            if (y == self.y1 && y == self.y2)
                || ((y <= self.y1 && y >= self.y2) != (y >= self.y1 && y <= self.y2))
            {
                if (x == self.x1 && x == self.x2)
                    || ((x <= self.x1 && x >= self.x2) != (x >= self.x1 && x <= self.x2)
                        && GeoUtils::orient(self.x1, self.y1, self.x2, self.y2, x, y) == 0)
                {
                    return ON_EDGE;
                } else if (self.y1 > y) != (self.y2 > y) {
                    res = if x < (self.x2 - self.x1) * (y - self.y1) / (self.y2 - self.y1) + self.x1
                    {
                        TRUE
                    } else {
                        FALSE
                    };
                }
            }
            if let Some(left) = &self.left {
                res ^= left.contains_pn_poly(x, y);
                if (res & 0x02) == 0x02 {
                    return ON_EDGE;
                }
            }

            if let Some(right) = &self.right {
                if y >= self.low {
                    res ^= right.contains_pn_poly(x, y);
                    if (res & 0x02) == 0x02 {
                        return ON_EDGE;
                    }
                }
            }
        }
        debug_assert!(res <= ON_EDGE);
        res
    }

    /// Returns whether the provided point lies on the line.
    ///
    /// Equivalent to `EdgeTree.isPointOnLine(double, double)`.
    pub fn is_point_on_line(&self, x: f64, y: f64) -> bool {
        if y <= self.max {
            let a1x = self.x1;
            let a1y = self.y1;
            let b1x = self.x2;
            let b1y = self.y2;
            let outside = (a1y < y && b1y < y)
                || (a1y > y && b1y > y)
                || (a1x < x && b1x < x)
                || (a1x > x && b1x > x);
            if !outside && GeoUtils::orient(a1x, a1y, b1x, b1y, x, y) == 0 {
                return true;
            }
            if let Some(left) = &self.left {
                if left.is_point_on_line(x, y) {
                    return true;
                }
            }
            if let Some(right) = &self.right {
                if y >= self.low && right.is_point_on_line(x, y) {
                    return true;
                }
            }
        }
        false
    }

    /// Returns whether the triangle crosses any edge in this edge subtree.
    ///
    /// Equivalent to `EdgeTree.crossesTriangle(...)`.
    #[allow(clippy::too_many_arguments)]
    pub fn crosses_triangle(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        ax: f64,
        ay: f64,
        bx: f64,
        by: f64,
        cx: f64,
        cy: f64,
        include_boundary: bool,
    ) -> bool {
        if min_y <= self.max {
            let dy = self.y1;
            let ey = self.y2;
            let dx = self.x1;
            let ex = self.x2;

            // optimization: see if the rectangle is outside of the "bounding box" of the polyline
            // at all; if not, don't waste our time trying more complicated stuff
            let outside = (dy < min_y && ey < min_y)
                || (dy > max_y && ey > max_y)
                || (dx < min_x && ex < min_x)
                || (dx > max_x && ex > max_x);

            if !outside {
                if include_boundary {
                    if GeoUtils::line_crosses_line_with_boundary(dx, dy, ex, ey, ax, ay, bx, by)
                        || GeoUtils::line_crosses_line_with_boundary(dx, dy, ex, ey, bx, by, cx, cy)
                        || GeoUtils::line_crosses_line_with_boundary(dx, dy, ex, ey, cx, cy, ax, ay)
                    {
                        return true;
                    }
                } else if GeoUtils::line_crosses_line(dx, dy, ex, ey, ax, ay, bx, by)
                    || GeoUtils::line_crosses_line(dx, dy, ex, ey, bx, by, cx, cy)
                    || GeoUtils::line_crosses_line(dx, dy, ex, ey, cx, cy, ax, ay)
                {
                    return true;
                }
            }

            if let Some(left) = &self.left {
                if left.crosses_triangle(
                    min_x,
                    max_x,
                    min_y,
                    max_y,
                    ax,
                    ay,
                    bx,
                    by,
                    cx,
                    cy,
                    include_boundary,
                ) {
                    return true;
                }
            }

            if let Some(right) = &self.right {
                if max_y >= self.low
                    && right.crosses_triangle(
                        min_x,
                        max_x,
                        min_y,
                        max_y,
                        ax,
                        ay,
                        bx,
                        by,
                        cx,
                        cy,
                        include_boundary,
                    )
                {
                    return true;
                }
            }
        }
        false
    }

    /// Returns whether the box crosses any edge in this edge subtree.
    ///
    /// Equivalent to `EdgeTree.crossesBox(...)`.
    pub fn crosses_box(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        include_boundary: bool,
    ) -> bool {
        // we just have to cross one edge to answer the question, so we descend the tree and return
        // when we do.
        if min_y <= self.max {
            // we compute line intersections of every polygon edge with every box line; if we find
            // one, return true. For each box line (AB) and each poly line (CD):
            //   intersects = orient(C,D,A) * orient(C,D,B) <= 0 && orient(A,B,C) * orient(A,B,D) <= 0
            let cy = self.y1;
            let dy = self.y2;
            let cx = self.x1;
            let dx = self.x2;

            // optimization: see if either end of the line segment is contained by the rectangle
            if Rectangle::contains_point(cy, cx, min_y, max_y, min_x, max_x)
                || Rectangle::contains_point(dy, dx, min_y, max_y, min_x, max_x)
            {
                return true;
            }

            // optimization: see if the rectangle is outside of the "bounding box" of the polyline
            // at all; if not, don't waste our time trying more complicated stuff
            let outside = (cy < min_y && dy < min_y)
                || (cy > max_y && dy > max_y)
                || (cx < min_x && dx < min_x)
                || (cx > max_x && dx > max_x);

            if !outside {
                if include_boundary {
                    // include boundaries: ensures box edges that terminate on the polygon are
                    // included
                    if GeoUtils::line_crosses_line_with_boundary(
                        cx, cy, dx, dy, min_x, min_y, max_x, min_y,
                    ) || GeoUtils::line_crosses_line_with_boundary(
                        cx, cy, dx, dy, max_x, min_y, max_x, max_y,
                    ) || GeoUtils::line_crosses_line_with_boundary(
                        cx, cy, dx, dy, max_x, max_y, min_x, max_y,
                    ) || GeoUtils::line_crosses_line_with_boundary(
                        cx, cy, dx, dy, min_x, max_y, min_x, min_y,
                    ) {
                        return true;
                    }
                } else if GeoUtils::line_crosses_line(cx, cy, dx, dy, min_x, min_y, max_x, min_y)
                    || GeoUtils::line_crosses_line(cx, cy, dx, dy, max_x, min_y, max_x, max_y)
                    || GeoUtils::line_crosses_line(cx, cy, dx, dy, max_x, max_y, min_x, max_y)
                    || GeoUtils::line_crosses_line(cx, cy, dx, dy, min_x, max_y, min_x, min_y)
                {
                    return true;
                }
            }

            if let Some(left) = &self.left {
                if left.crosses_box(min_x, max_x, min_y, max_y, include_boundary) {
                    return true;
                }
            }

            if let Some(right) = &self.right {
                if max_y >= self.low
                    && right.crosses_box(min_x, max_x, min_y, max_y, include_boundary)
                {
                    return true;
                }
            }
        }
        false
    }

    /// Returns whether the line crosses any edge in this edge subtree.
    ///
    /// Equivalent to `EdgeTree.crossesLine(...)`.
    #[allow(clippy::too_many_arguments)]
    pub fn crosses_line(
        &self,
        min_x: f64,
        max_x: f64,
        min_y: f64,
        max_y: f64,
        a2x: f64,
        a2y: f64,
        b2x: f64,
        b2y: f64,
        include_boundary: bool,
    ) -> bool {
        if min_y <= self.max {
            let a1x = self.x1;
            let a1y = self.y1;
            let b1x = self.x2;
            let b1y = self.y2;

            let outside = (a1y < min_y && b1y < min_y)
                || (a1y > max_y && b1y > max_y)
                || (a1x < min_x && b1x < min_x)
                || (a1x > max_x && b1x > max_x);
            if !outside {
                if include_boundary {
                    if GeoUtils::line_crosses_line_with_boundary(
                        a1x, a1y, b1x, b1y, a2x, a2y, b2x, b2y,
                    ) {
                        return true;
                    }
                } else if GeoUtils::line_crosses_line(a1x, a1y, b1x, b1y, a2x, a2y, b2x, b2y) {
                    return true;
                }
            }
            if let Some(left) = &self.left {
                if left.crosses_line(
                    min_x,
                    max_x,
                    min_y,
                    max_y,
                    a2x,
                    a2y,
                    b2x,
                    b2y,
                    include_boundary,
                ) {
                    return true;
                }
            }
            if let Some(right) = &self.right {
                if max_y >= self.low
                    && right.crosses_line(
                        min_x,
                        max_x,
                        min_y,
                        max_y,
                        a2x,
                        a2y,
                        b2x,
                        b2y,
                        include_boundary,
                    )
                {
                    return true;
                }
            }
        }
        false
    }

    /// Creates an edge interval tree from a set of geometry vertices, returning
    /// the root node.
    ///
    /// Equivalent to `EdgeTree.createTree(double[], double[])`.
    ///
    /// # Panics
    ///
    /// Panics when `x` and `y` do not have the same length or hold fewer than
    /// two vertices, which the geometry constructors already rule out.
    pub fn create_tree(x: &[f64], y: &[f64]) -> EdgeTree {
        assert_eq!(
            x.len(),
            y.len(),
            "INVARIANT: geometry constructors keep the coordinate arrays parallel"
        );
        assert!(
            x.len() >= 2,
            "INVARIANT: geometry constructors require at least two vertices"
        );
        let mut edges: Vec<Option<EdgeTree>> = Vec::with_capacity(x.len() - 1);
        for i in 1..x.len() {
            let x1 = x[i - 1];
            let y1 = y[i - 1];
            let x2 = x[i];
            let y2 = y[i];
            edges.push(Some(EdgeTree::new(x1, y1, x2, y2, y1.min(y2), y1.max(y2))));
        }
        // sort the edges then build a balanced tree from them
        edges.sort_by(|left, right| {
            let left = left.as_ref().expect("INVARIANT: no edge taken yet");
            let right = right.as_ref().expect("INVARIANT: no edge taken yet");
            // Java compares with `Double.compare`, whose ordering is IEEE 754
            // totalOrder; `f64::total_cmp` is exactly that.
            match left.low.total_cmp(&right.low) {
                Ordering::Equal => left.max.total_cmp(&right.max),
                other => other,
            }
        });
        let high = edges.len() as isize - 1;
        *Self::create_tree_range(&mut edges, 0, high)
            .expect("INVARIANT: at least one edge exists for a two-vertex geometry")
    }

    /// Creates a tree from sorted edges, with `low` and `high` inclusive.
    ///
    /// Equivalent to the private `EdgeTree.createTree(EdgeTree[], int, int)`.
    fn create_tree_range(
        edges: &mut [Option<EdgeTree>],
        low: isize,
        high: isize,
    ) -> Option<Box<EdgeTree>> {
        if low > high {
            return None;
        }
        // add midpoint
        let mid = (low + high) / 2;
        let mut new_node = edges[mid as usize]
            .take()
            .expect("INVARIANT: each edge is consumed exactly once");
        // add children
        new_node.left = Self::create_tree_range(edges, low, mid - 1);
        new_node.right = Self::create_tree_range(edges, mid + 1, high);
        // pull up max values to this node
        if let Some(left) = &new_node.left {
            new_node.max = new_node.max.max(left.max);
        }
        if let Some(right) = &new_node.right {
            new_node.max = new_node.max.max(right.max);
        }
        Some(Box::new(new_node))
    }
}
