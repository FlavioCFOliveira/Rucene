//! Polygon triangulation ported from `org.apache.lucene.geo.Tessellator`.
//!
//! The tessellator turns a polygon into the triangle mesh that shape fields
//! index, so its output is part of the index-compatibility contract: the
//! triangles produced here must be the ones Apache Lucene Core 10.5.0 produces.
//!
//! # Representation
//!
//! Lucene builds a circular doubly-linked list of `Node` objects and mutates it
//! in place, with several aliases pointing at the same node at once. Rust
//! cannot express that with references and no `unsafe`, so this port keeps the
//! nodes in one arena ([`Vec`]) and links them by index. Java's reference
//! identity comparisons (`p == q`) become arena-index comparisons, which is the
//! same relation. `Node.idx`, a separate field that survives node copying, is
//! kept as its own field exactly as in Java.
//!
//! [`Triangle`] snapshots the coordinates of the three nodes it is built from
//! instead of holding node references. In Java those fields (`x`, `y`, `polyX`,
//! `polyY`, `vrtxIdx`) are all `final`, so the snapshot is observationally the
//! same and it frees the triangles from the arena's lifetime.

use crate::error::{LuceneError, Result};
use crate::geo::encoding::{GeoEncodingUtils, GeoUtils, WindingOrder, XYEncodingUtils};
use crate::geo::geometry::{Point, Polygon, XYPolygon};
use std::fmt;

/// This is a dumb heuristic to control whether we cut over to sorted morton
/// values.
///
/// Equivalent to `Tessellator.VERTEX_THRESHOLD`.
const VERTEX_THRESHOLD: i32 = 80;

/// State of the tessellated split; avoids recursion.
///
/// Equivalent to the private enum `Tessellator.State`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Init,
    Cure,
    Split,
}

impl State {
    /// Equivalent to `Enum.name()`, which the monitor notifications embed.
    fn name(self) -> &'static str {
        match self {
            State::Init => "INIT",
            State::Cure => "CURE",
            State::Split => "SPLIT",
        }
    }
}

/// Interleaves the bits of two 32-bit values into one 64-bit Morton code.
///
/// Equivalent to `org.apache.lucene.util.BitUtil.interleave(int, int)`. It is
/// reproduced here rather than called because `org.apache.lucene.util.BitUtil`
/// is ported in a module this file must not modify and does not yet expose the
/// function; it belongs in `util::BitUtil` once that module gains it.
fn interleave(even: u32, odd: u32) -> u64 {
    const MAGIC0: u64 = 0x5555_5555_5555_5555;
    const MAGIC1: u64 = 0x3333_3333_3333_3333;
    const MAGIC2: u64 = 0x0F0F_0F0F_0F0F_0F0F;
    const MAGIC3: u64 = 0x00FF_00FF_00FF_00FF;
    const MAGIC4: u64 = 0x0000_FFFF_0000_FFFF;

    let mut v1 = u64::from(even);
    let mut v2 = u64::from(odd);
    v1 = (v1 | (v1 << 16)) & MAGIC4;
    v1 = (v1 | (v1 << 8)) & MAGIC3;
    v1 = (v1 | (v1 << 4)) & MAGIC2;
    v1 = (v1 | (v1 << 2)) & MAGIC1;
    v1 = (v1 | (v1 << 1)) & MAGIC0;
    v2 = (v2 | (v2 << 16)) & MAGIC4;
    v2 = (v2 | (v2 << 8)) & MAGIC3;
    v2 = (v2 | (v2 << 4)) & MAGIC2;
    v2 = (v2 | (v2 << 2)) & MAGIC1;
    v2 = (v2 | (v2 << 1)) & MAGIC0;

    (v2 << 1) | v1
}

/// Flips the sign bit so that negative encoded values sort below positive ones.
fn flip_sign(v: i32) -> u32 {
    (v as u32) ^ 0x8000_0000
}

/// A node of the circular doubly-linked list of polygon coordinates.
///
/// Equivalent to the package-private static class `Tessellator.Node`.
#[derive(Clone, Copy, Debug)]
struct Node {
    /// Node index in the linked list.
    idx: usize,
    /// X value of the vertex.
    x_val: f64,
    /// Y value of the vertex.
    y_val: f64,
    /// Encoded x value.
    x: i32,
    /// Encoded y value.
    y: i32,
    /// Morton code for sorting.
    morton: u64,
    /// Previous node.
    previous: usize,
    /// Next node.
    next: usize,
    /// Previous z node.
    previous_z: Option<usize>,
    /// Next z node.
    next_z: Option<usize>,
    /// Whether the edge from this node to the next node is part of the polygon
    /// edges.
    is_next_edge_from_polygon: bool,
}

/// One vertex of a tessellated [`Triangle`].
///
/// Holds the snapshot of the `final` fields Java's `Tessellator.Node` exposes
/// to `Triangle`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct TriangleVertex {
    x: i32,
    y: i32,
    x_val: f64,
    y_val: f64,
}

/// A triangle of the tessellated mesh.
///
/// Equivalent to `org.apache.lucene.geo.Tessellator.Triangle`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triangle {
    vertex: [TriangleVertex; 3],
    edge_from_polygon: [bool; 3],
}

impl Triangle {
    /// Returns the quantized x value for the given vertex.
    ///
    /// Equivalent to `Triangle.getEncodedX(int)`.
    pub fn get_encoded_x(&self, vertex: usize) -> i32 {
        self.vertex[vertex].x
    }

    /// Returns the quantized y value for the given vertex.
    ///
    /// Equivalent to `Triangle.getEncodedY(int)`.
    pub fn get_encoded_y(&self, vertex: usize) -> i32 {
        self.vertex[vertex].y
    }

    /// Returns the y value for the given vertex.
    ///
    /// Equivalent to `Triangle.getY(int)`.
    pub fn get_y(&self, vertex: usize) -> f64 {
        self.vertex[vertex].y_val
    }

    /// Returns the x value for the given vertex.
    ///
    /// Equivalent to `Triangle.getX(int)`.
    pub fn get_x(&self, vertex: usize) -> f64 {
        self.vertex[vertex].x_val
    }

    /// Returns whether the edge starting at `start_vertex` is shared with the
    /// polygon.
    ///
    /// Equivalent to `Triangle.isEdgefromPolygon(int)`.
    pub fn is_edge_from_polygon(&self, start_vertex: usize) -> bool {
        self.edge_from_polygon[start_vertex]
    }
}

impl fmt::Display for Triangle {
    /// Equivalent to `Triangle.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}, {} [{}] {}, {} [{}] {}, {} [{}]",
            self.vertex[0].x,
            self.vertex[0].y,
            self.edge_from_polygon[0],
            self.vertex[1].x,
            self.vertex[1].y,
            self.edge_from_polygon[1],
            self.vertex[2].x,
            self.vertex[2].y,
            self.edge_from_polygon[2]
        )
    }
}

/// Receives the internal state at each step of the triangulation algorithm.
///
/// Equivalent to the `Tessellator.Monitor` interface. This is of use for
/// debugging complex cases as well as for gaining insight into the way the
/// algorithm works. Java's `currentState` accepts a `null` point list, which is
/// modelled here as [`None`].
pub trait Monitor {
    /// Called with the current state on each loop of the main earclip
    /// algorithm.
    ///
    /// Equivalent to `Monitor.currentState(String, List, List)`.
    fn current_state(&mut self, status: &str, points: Option<&[Point]>, tessellation: &[Triangle]);

    /// Called when a new polygon split is entered for `mode=SPLIT`.
    ///
    /// Equivalent to `Monitor.startSplit(String, List, List)`.
    fn start_split(&mut self, status: &str, left_polygon: &[Point], right_polygon: &[Point]);

    /// Called when a polygon split is completed.
    ///
    /// Equivalent to `Monitor.endSplit(String)`.
    fn end_split(&mut self, status: &str);
}

/// Status string reported to a [`Monitor`] when tessellation fails.
///
/// Equivalent to `Monitor.FAILED`.
pub const MONITOR_FAILED: &str = "FAILED";

/// Status string reported to a [`Monitor`] when tessellation completes.
///
/// Equivalent to `Monitor.COMPLETED`.
pub const MONITOR_COMPLETED: &str = "COMPLETED";

/// The polygon whose holes are being eliminated, used only to render the error
/// message Java builds from the offending hole.
enum HoleSource<'a> {
    Geo(&'a Polygon),
    Xy(&'a XYPolygon),
}

/// A hole queued for elimination, pairing its leftmost node with the bounds and
/// the source ring Java keeps in `holeListPolygons`.
struct HoleEntry {
    leftmost: usize,
    hole_index: usize,
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
}

/// Computes a triangular mesh tessellation for a given polygon.
///
/// Equivalent to `org.apache.lucene.geo.Tessellator` (`@lucene.internal`).
///
/// This is inspired by mapbox's earcut algorithm
/// (<https://github.com/mapbox/earcut>), which is a modification to FIST
/// (<https://www.cosy.sbg.ac.at/~held/projects/triang/triang.html>) written by
/// Martin Held, and ear clipping
/// (<https://www.geometrictools.com/Documentation/TriangulationByEarClipping.pdf>)
/// written by David Eberly.
///
/// Requires valid polygons:
///
/// * no self intersections;
/// * holes must be inside the polygon;
/// * holes may only touch at one vertex;
/// * the polygon must have an area (no "line" boxes);
/// * sensitive to overflow (subatomic values such as `1e-200` can cause
///   unexpected behaviour).
///
/// The upstream code is a modified version of the JavaScript implementation
/// provided by MapBox under the ISC License, Copyright (c) 2016 Mapbox.
pub struct Tessellator;

/// The working arena of the tessellation.
struct Tessellation {
    nodes: Vec<Node>,
}

impl Tessellation {
    fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    fn x(&self, n: usize) -> f64 {
        self.nodes[n].x_val
    }

    fn y(&self, n: usize) -> f64 {
        self.nodes[n].y_val
    }

    fn next(&self, n: usize) -> usize {
        self.nodes[n].next
    }

    fn previous(&self, n: usize) -> usize {
        self.nodes[n].previous
    }

    /// Creates a node and optionally links it with a previous node in a
    /// circular doubly-linked list.
    ///
    /// Equivalent to the private `Tessellator.insertNode(...)`.
    fn insert_node(
        &mut self,
        x: &[f64],
        y: &[f64],
        index: usize,
        vertex_index: usize,
        last_node: Option<usize>,
        is_geo: bool,
    ) -> Result<usize> {
        let x_val = x[vertex_index];
        let y_val = y[vertex_index];
        // casting to float is safe as original values for non-geo are represented as floats
        let enc_y = if is_geo {
            GeoEncodingUtils::encode_latitude(y_val)?
        } else {
            XYEncodingUtils::encode(y_val as f32)?
        };
        let enc_x = if is_geo {
            GeoEncodingUtils::encode_longitude(x_val)?
        } else {
            XYEncodingUtils::encode(x_val as f32)?
        };
        let morton = interleave(flip_sign(enc_x), flip_sign(enc_y));
        let node_pos = self.nodes.len();
        self.nodes.push(Node {
            idx: index,
            x_val,
            y_val,
            x: enc_x,
            y: enc_y,
            morton,
            previous: node_pos,
            next: node_pos,
            previous_z: Some(node_pos),
            next_z: Some(node_pos),
            is_next_edge_from_polygon: true,
        });
        match last_node {
            None => {
                // the constructor above already made the node point at itself
            }
            Some(last) => {
                let last_next = self.nodes[last].next;
                let last_next_z = self.nodes[last]
                    .next_z
                    .expect("INVARIANT: ring nodes always have a z successor while linking");
                self.nodes[node_pos].next = last_next;
                self.nodes[node_pos].next_z = Some(last_next);
                self.nodes[node_pos].previous = last;
                self.nodes[node_pos].previous_z = Some(last);
                self.nodes[last_next].previous = node_pos;
                self.nodes[last_next_z].previous_z = Some(node_pos);
                self.nodes[last].next = node_pos;
                self.nodes[last].next_z = Some(node_pos);
            }
        }
        Ok(node_pos)
    }

    /// Duplicates a node, as Java's `Node(Node)` copy constructor does.
    fn copy_node(&mut self, other: usize) -> usize {
        let copy = self.nodes[other];
        let pos = self.nodes.len();
        self.nodes.push(copy);
        pos
    }

    /// Removes a node from the doubly linked list.
    ///
    /// Equivalent to the private `Tessellator.removeNode(Node, boolean)`.
    fn remove_node(&mut self, node: usize, edge_from_polygon: bool) {
        let next = self.nodes[node].next;
        let previous = self.nodes[node].previous;
        self.nodes[next].previous = previous;
        self.nodes[previous].next = next;
        self.nodes[previous].is_next_edge_from_polygon = edge_from_polygon;

        let previous_z = self.nodes[node].previous_z;
        let next_z = self.nodes[node].next_z;
        if let Some(pz) = previous_z {
            self.nodes[pz].next_z = next_z;
        }
        if let Some(nz) = next_z {
            self.nodes[nz].previous_z = previous_z;
        }
    }

    /// Determines whether two point vertices are equal.
    ///
    /// Equivalent to the private `Tessellator.isVertexEquals(Node, Node)`.
    fn is_vertex_equals_node(&self, a: usize, b: usize) -> bool {
        self.is_vertex_equals(a, self.x(b), self.y(b))
    }

    /// Determines whether a node equals the given coordinates.
    ///
    /// Equivalent to the private `Tessellator.isVertexEquals(Node, double, double)`.
    fn is_vertex_equals(&self, a: usize, x: f64, y: f64) -> bool {
        self.x(a) == x && self.y(a) == y
    }

    /// Creates a circular doubly-linked list using polygon points. The order is
    /// governed by the specified winding order.
    ///
    /// Equivalent to the private `Tessellator.createDoublyLinkedList(...)`.
    fn create_doubly_linked_list(
        &mut self,
        x: &[f64],
        y: &[f64],
        poly_winding_order: WindingOrder,
        is_geo: bool,
        start_index: usize,
        winding_order: WindingOrder,
    ) -> Result<Option<usize>> {
        let mut last_node: Option<usize> = None;
        let mut index = start_index;
        // Link points into the circular doubly-linked list in the specified winding order
        if winding_order == poly_winding_order {
            for i in 0..x.len() {
                last_node = Some(self.insert_node(x, y, index, i, last_node, is_geo)?);
                index += 1;
            }
        } else {
            for i in (0..x.len()).rev() {
                last_node = Some(self.insert_node(x, y, index, i, last_node, is_geo)?);
                index += 1;
            }
        }
        // if first and last node are the same then remove the end node and set lastNode to the
        // start
        if let Some(last) = last_node {
            let next = self.nodes[last].next;
            if self.is_vertex_equals_node(last, next) {
                self.remove_node(last, true);
                last_node = Some(next);
            }
        }

        // Return the last node in the Doubly-Linked List
        Ok(self.filter_points(last_node, None))
    }

    /// Eliminates collinear/duplicate points from the doubly linked list.
    ///
    /// Equivalent to the private `Tessellator.filterPoints(Node, Node)`.
    fn filter_points(&mut self, start: Option<usize>, end: Option<usize>) -> Option<usize> {
        let start = start?;
        let mut end = end.unwrap_or(start);

        let mut node = start;
        let mut continue_iteration;

        loop {
            continue_iteration = false;
            let next_node = self.nodes[node].next;
            let prev_node = self.nodes[node].previous;
            // we can filter points when:
            // 1.- they are the same
            // 2.- each one starts and ends in each other
            // 3.- they are collinear and both edges have the same value in isNextEdgeFromPolygon
            // 4.- they are collinear and the second edge returns over the first edge
            if self.is_vertex_equals_node(node, next_node)
                || self.is_vertex_equals_node(prev_node, next_node)
                || ((self.nodes[prev_node].is_next_edge_from_polygon
                    == self.nodes[node].is_next_edge_from_polygon
                    || self.is_point_in_line_coords(
                        prev_node,
                        node,
                        self.x(next_node),
                        self.y(next_node),
                    ))
                    && area(
                        self.x(prev_node),
                        self.y(prev_node),
                        self.x(node),
                        self.y(node),
                        self.x(next_node),
                        self.y(next_node),
                    ) == 0.0)
            {
                // Remove the node
                let edge_from_polygon = self.nodes[prev_node].is_next_edge_from_polygon;
                self.remove_node(node, edge_from_polygon);
                node = prev_node;
                end = prev_node;

                if node == next_node {
                    break;
                }
                continue_iteration = true;
            } else {
                node = next_node;
            }

            if !(continue_iteration || node != end) {
                break;
            }
        }
        Some(end)
    }

    /// Returns whether the `x`, `y` point is collinear with the provided `a`
    /// and `b` points and lies between them.
    ///
    /// Equivalent to the private
    /// `Tessellator.isPointInLine(Node, Node, double, double)`.
    fn is_point_in_line_coords(&self, a: usize, b: usize, lon: f64, lat: f64) -> bool {
        let dxc = lon - self.x(a);
        let dyc = lat - self.y(a);

        let dxl = self.x(b) - self.x(a);
        let dyl = self.y(b) - self.y(a);

        if dxc * dyl - dyc * dxl == 0.0 {
            return if dxl.abs() >= dyl.abs() {
                if dxl > 0.0 {
                    self.x(a) <= lon && lon <= self.x(b)
                } else {
                    self.x(b) <= lon && lon <= self.x(a)
                }
            } else if dyl > 0.0 {
                self.y(a) <= lat && lat <= self.y(b)
            } else {
                self.y(b) <= lat && lat <= self.y(a)
            };
        }
        false
    }

    /// Equivalent to the private `Tessellator.isPointInLine(Node, Node, Node)`.
    fn is_point_in_line(&self, a: usize, b: usize, point: usize) -> bool {
        self.is_point_in_line_coords(a, b, self.x(point), self.y(point))
    }

    /// Finds the left-most vertex of a polygon ring.
    ///
    /// Equivalent to the private `Tessellator.fetchLeftmost(Node)`.
    fn fetch_leftmost(&self, start: usize) -> usize {
        let mut node = start;
        let mut left_most = start;
        loop {
            // Determine if the current node possesses a lesser X position.
            if self.x(node) < self.x(left_most)
                || (self.x(node) == self.x(left_most) && self.y(node) < self.y(left_most))
            {
                left_most = node;
            }
            node = self.nodes[node].next;
            if node == start {
                break;
            }
        }
        left_most
    }

    /// Links two polygon vertices using a bridge.
    ///
    /// Equivalent to the private `Tessellator.splitPolygon(Node, Node, boolean)`.
    fn split_polygon(&mut self, a: usize, b: usize, edge_from_polygon: bool) -> usize {
        let a2 = self.copy_node(a);
        let b2 = self.copy_node(b);
        let an = self.nodes[a].next;
        let bp = self.nodes[b].previous;

        self.nodes[a].next = b;
        self.nodes[a].is_next_edge_from_polygon = edge_from_polygon;
        self.nodes[a].next_z = Some(b);
        self.nodes[b].previous = a;
        self.nodes[b].previous_z = Some(a);
        self.nodes[a2].next = an;
        self.nodes[a2].next_z = Some(an);
        self.nodes[an].previous = a2;
        self.nodes[an].previous_z = Some(a2);
        self.nodes[b2].next = a2;
        self.nodes[b2].is_next_edge_from_polygon = edge_from_polygon;
        self.nodes[b2].next_z = Some(a2);
        self.nodes[a2].previous = b2;
        self.nodes[a2].previous_z = Some(b2);
        self.nodes[bp].next = b2;
        self.nodes[bp].next_z = Some(b2);

        b2
    }

    /// Interlinks polygon nodes in Z-order, resetting the z values first.
    ///
    /// Equivalent to the private `Tessellator.sortByMortonWithReset(Node)`.
    fn sort_by_morton_with_reset(&mut self, start: usize) {
        let mut next = start;
        loop {
            self.nodes[next].previous_z = Some(self.nodes[next].previous);
            self.nodes[next].next_z = Some(self.nodes[next].next);
            next = self.nodes[next].next;
            if next == start {
                break;
            }
        }
        self.sort_by_morton(start);
    }

    /// Interlinks polygon nodes in Z-order.
    ///
    /// Equivalent to the private `Tessellator.sortByMorton(Node)`.
    fn sort_by_morton(&mut self, start: usize) {
        let previous_z = self.nodes[start]
            .previous_z
            .expect("INVARIANT: the ring is fully z-linked before the Morton sort");
        self.nodes[previous_z].next_z = None;
        self.nodes[start].previous_z = None;
        // Sort the generated ring using Z ordering.
        self.tatham_sort(Some(start));
    }

    /// Simon Tatham's doubly-linked list `O(n log n)` mergesort.
    ///
    /// Equivalent to the private `Tessellator.tathamSort(Node)`; see
    /// <http://www.chiark.greenend.org.uk/~sgtatham/algorithms/listsort.html>.
    fn tatham_sort(&mut self, list: Option<usize>) {
        let mut list = match list {
            None => return,
            Some(l) => Some(l),
        };
        let mut in_size = 1usize;

        loop {
            let mut p = list;
            list = None;
            let mut tail: Option<usize> = None;
            // count number of merges in this pass
            let mut num_merges = 0usize;

            while let Some(p_node) = p {
                num_merges += 1;
                // step 'inSize' places along from p
                let mut q = Some(p_node);
                let mut p_size = 0usize;
                let mut i = 0usize;
                while i < in_size && q.is_some() {
                    p_size += 1;
                    q = self.nodes[q.expect("INVARIANT: checked by the loop condition")].next_z;
                    i += 1;
                }
                // if q hasn't fallen off the end, we have two lists to merge
                let mut q_size = in_size;

                let mut p_cursor = p;
                // now we have two lists; merge
                while p_size > 0 || (q_size > 0 && q.is_some()) {
                    let e;
                    let take_p = if p_size != 0 {
                        match (q_size, q) {
                            (0, _) | (_, None) => true,
                            (_, Some(q_node)) => {
                                let p_node = p_cursor
                                    .expect("INVARIANT: pSize > 0 implies p is a live node");
                                self.nodes[p_node].morton <= self.nodes[q_node].morton
                            }
                        }
                    } else {
                        false
                    };
                    if take_p {
                        let p_node =
                            p_cursor.expect("INVARIANT: pSize > 0 implies p is a live node");
                        e = p_node;
                        p_cursor = self.nodes[p_node].next_z;
                        p_size -= 1;
                    } else {
                        let q_node =
                            q.expect("INVARIANT: the branch is only taken when q is a live node");
                        e = q_node;
                        q = self.nodes[q_node].next_z;
                        q_size -= 1;
                    }

                    match tail {
                        Some(t) => self.nodes[t].next_z = Some(e),
                        None => list = Some(e),
                    }
                    // maintain reverse pointers
                    self.nodes[e].previous_z = tail;
                    tail = Some(e);
                }
                // now p has stepped 'inSize' places along, and q has too
                p = q;
            }

            if let Some(t) = tail {
                self.nodes[t].next_z = None;
            }
            in_size *= 2;
            if num_merges <= 1 {
                break;
            }
        }
    }

    /// Determines the signed area between node `start` and node `end`.
    ///
    /// Equivalent to the private `Tessellator.signedArea(Node, Node)`.
    fn signed_area(&self, start: usize, end: usize) -> f64 {
        let mut next = start;
        let mut winding_sum = 0.0f64;
        loop {
            let n = self.nodes[next].next;
            winding_sum += area(
                self.x(next),
                self.y(next),
                self.x(n),
                self.y(n),
                self.x(end),
                self.y(end),
            );
            next = n;
            if self.nodes[next].next == end {
                break;
            }
        }
        winding_sum
    }

    /// Determines whether the polygon defined between `start` and `end` is CW.
    ///
    /// Equivalent to the private `Tessellator.isCWPolygon(Node, Node)`.
    fn is_cw_polygon(&self, start: usize, end: usize) -> bool {
        // The polygon must be CW
        self.signed_area(start, end) < 0.0
    }

    /// Equivalent to the private `Tessellator.isLocallyInside(Node, Node)`.
    fn is_locally_inside(&self, a: usize, b: usize) -> bool {
        let a_prev = self.nodes[a].previous;
        let a_next = self.nodes[a].next;
        let ar = area(
            self.x(a_prev),
            self.y(a_prev),
            self.x(a),
            self.y(a),
            self.x(a_next),
            self.y(a_next),
        );
        if ar == 0.0 {
            // parallel
            false
        } else if ar < 0.0 {
            // if a is cw
            area(
                self.x(a),
                self.y(a),
                self.x(b),
                self.y(b),
                self.x(a_next),
                self.y(a_next),
            ) >= 0.0
                && area(
                    self.x(a),
                    self.y(a),
                    self.x(a_prev),
                    self.y(a_prev),
                    self.x(b),
                    self.y(b),
                ) >= 0.0
        } else {
            // ccw
            area(
                self.x(a),
                self.y(a),
                self.x(b),
                self.y(b),
                self.x(a_prev),
                self.y(a_prev),
            ) <= 0.0
                || area(
                    self.x(a),
                    self.y(a),
                    self.x(a_next),
                    self.y(a_next),
                    self.x(b),
                    self.y(b),
                ) <= 0.0
        }
    }

    /// Determines whether the middle point of a polygon diagonal is contained
    /// within the polygon.
    ///
    /// Equivalent to the private `Tessellator.middleInsert(...)`.
    fn middle_insert(&self, start: usize, x0: f64, y0: f64, x1: f64, y1: f64) -> bool {
        let mut node = start;
        let mut l_is_inside = false;
        let l_dx = (x0 + x1) / 2.0;
        let l_dy = (y0 + y1) / 2.0;
        loop {
            let next_node = self.nodes[node].next;
            if (self.y(node) > l_dy) != (self.y(next_node) > l_dy)
                && l_dx
                    < (self.x(next_node) - self.x(node)) * (l_dy - self.y(node))
                        / (self.y(next_node) - self.y(node))
                        + self.x(node)
            {
                l_is_inside = !l_is_inside;
            }
            node = self.nodes[node].next;
            if node == start {
                break;
            }
        }
        l_is_inside
    }

    /// Determines whether the diagonal of a polygon intersects any polygon
    /// element.
    ///
    /// Equivalent to the private `Tessellator.isIntersectingPolygon(...)`.
    fn is_intersecting_polygon(&self, start: usize, x0: f64, y0: f64, x1: f64, y1: f64) -> bool {
        let mut node = start;
        loop {
            let next_node = self.nodes[node].next;
            if !self.is_vertex_equals(node, x0, y0) && !self.is_vertex_equals(node, x1, y1) {
                if lines_intersect(
                    self.x(node),
                    self.y(node),
                    self.x(next_node),
                    self.y(next_node),
                    x0,
                    y0,
                    x1,
                    y1,
                ) {
                    return true;
                }
            }
            node = next_node;
            if node == start {
                break;
            }
        }
        false
    }

    /// Determines whether a diagonal between two polygon nodes lies within the
    /// polygon interior.
    ///
    /// Equivalent to the private `Tessellator.isValidDiagonal(Node, Node)`.
    fn is_valid_diagonal(&self, a: usize, b: usize) -> bool {
        let a_next = self.nodes[a].next;
        let a_prev = self.nodes[a].previous;
        let b_next = self.nodes[b].next;
        let b_prev = self.nodes[b].previous;
        if self.nodes[a_next].idx == self.nodes[b].idx
            || self.nodes[a_prev].idx == self.nodes[b].idx
            // check next edges are locally visible
            || !self.is_locally_inside(a_prev, b)
            || !self.is_locally_inside(b_next, a)
            // check polygons are CCW in both sides
            || !self.is_cw_polygon(a, b)
            || !self.is_cw_polygon(b, a)
        {
            return false;
        }
        if self.is_vertex_equals_node(a, b) {
            return true;
        }
        self.is_locally_inside(a, b)
            && self.is_locally_inside(b, a)
            && self.middle_insert(a, self.x(a), self.y(a), self.x(b), self.y(b))
            // make sure we don't introduce collinear lines
            && area(
                self.x(a_prev),
                self.y(a_prev),
                self.x(a),
                self.y(a),
                self.x(b),
                self.y(b),
            ) != 0.0
            && area(
                self.x(a),
                self.y(a),
                self.x(b),
                self.y(b),
                self.x(b_next),
                self.y(b_next),
            ) != 0.0
            && area(
                self.x(a_next),
                self.y(a_next),
                self.x(a),
                self.y(a),
                self.x(b),
                self.y(b),
            ) != 0.0
            && area(
                self.x(a),
                self.y(a),
                self.x(b),
                self.y(b),
                self.x(b_prev),
                self.y(b_prev),
            ) != 0.0
            // this call is expensive so do it last
            && !self.is_intersecting_polygon(a, self.x(a), self.y(a), self.x(b), self.y(b))
    }

    /// Computes whether the edge defined by `a` and `b` overlaps with a polygon
    /// edge.
    ///
    /// Equivalent to the private `Tessellator.isEdgeFromPolygon(Node, Node, boolean)`.
    fn is_edge_from_polygon(&self, a: usize, b: usize, is_morton: bool) -> bool {
        if is_morton {
            return self.is_morton_edge_from_polygon(a, b);
        }
        let mut next = a;
        loop {
            let n_next = self.nodes[next].next;
            let n_prev = self.nodes[next].previous;
            if self.is_point_in_line(next, n_next, a) && self.is_point_in_line(next, n_next, b) {
                return self.nodes[next].is_next_edge_from_polygon;
            }
            if self.is_point_in_line(next, n_prev, a) && self.is_point_in_line(next, n_prev, b) {
                return self.nodes[n_prev].is_next_edge_from_polygon;
            }
            next = n_next;
            if next == a {
                break;
            }
        }
        false
    }

    /// Uses the Morton code to determine whether the edge defined by `a` and
    /// `b` overlaps with a polygon edge.
    ///
    /// Equivalent to the private `Tessellator.isMortonEdgeFromPolygon(Node, Node)`.
    fn is_morton_edge_from_polygon(&self, a: usize, b: usize) -> bool {
        // edge bbox (flip the bits so negative encoded values are < positive encoded values)
        let min_tx = flip_sign(self.nodes[a].x.min(self.nodes[b].x));
        let min_ty = flip_sign(self.nodes[a].y.min(self.nodes[b].y));
        let max_tx = flip_sign(self.nodes[a].x.max(self.nodes[b].x));
        let max_ty = flip_sign(self.nodes[a].y.max(self.nodes[b].y));

        // z-order range for the current edge
        let min_z = interleave(min_tx, min_ty);
        let max_z = interleave(max_tx, max_ty);

        // look for points inside the edge in both directions
        let mut p = self.nodes[a].previous_z;
        let mut n = self.nodes[a].next_z;
        while let (Some(p_node), Some(n_node)) = (p, n) {
            if self.nodes[p_node].morton < min_z || self.nodes[n_node].morton > max_z {
                break;
            }
            if let Some(result) = self.morton_edge_probe(p_node, a, b) {
                return result;
            }

            p = self.nodes[p_node].previous_z;

            if let Some(result) = self.morton_edge_probe(n_node, a, b) {
                return result;
            }

            n = self.nodes[n_node].next_z;
        }

        // first look for points inside the edge in decreasing z-order
        while let Some(p_node) = p {
            if self.nodes[p_node].morton < min_z {
                break;
            }
            if let Some(result) = self.morton_edge_probe(p_node, a, b) {
                return result;
            }
            p = self.nodes[p_node].previous_z;
        }
        // then look for points in increasing z-order
        while let Some(n_node) = n {
            if self.nodes[n_node].morton > max_z {
                break;
            }
            if let Some(result) = self.morton_edge_probe(n_node, a, b) {
                return result;
            }
            n = self.nodes[n_node].next_z;
        }
        false
    }

    /// One probe of the four-line body Java repeats in every loop of
    /// `isMortonEdgeFromPolygon`.
    fn morton_edge_probe(&self, candidate: usize, a: usize, b: usize) -> Option<bool> {
        let c_next = self.nodes[candidate].next;
        let c_prev = self.nodes[candidate].previous;
        if self.is_point_in_line(candidate, c_next, a)
            && self.is_point_in_line(candidate, c_next, b)
        {
            return Some(self.nodes[candidate].is_next_edge_from_polygon);
        }
        if self.is_point_in_line(candidate, c_prev, a)
            && self.is_point_in_line(candidate, c_prev, b)
        {
            return Some(self.nodes[c_prev].is_next_edge_from_polygon);
        }
        None
    }

    /// Determines whether a polygon node forms a valid ear with adjacent nodes.
    ///
    /// Equivalent to the private `Tessellator.isEar(Node, boolean)`.
    fn is_ear(&self, ear: usize, morton_optimized: bool) -> bool {
        if morton_optimized {
            return self.morton_is_ear(ear);
        }

        let ear_prev = self.nodes[ear].previous;
        let ear_next = self.nodes[ear].next;
        // make sure there aren't other points inside the potential ear
        let mut node = self.nodes[ear_next].next;
        while node != ear_prev {
            if self.point_in_ear_and_reflex(node, ear_prev, ear, ear_next) {
                return false;
            }
            node = self.nodes[node].next;
        }
        true
    }

    /// The `pointInEar(...) && area(...) >= 0` test the ear checks repeat.
    fn point_in_ear_and_reflex(
        &self,
        node: usize,
        ear_prev: usize,
        ear: usize,
        ear_next: usize,
    ) -> bool {
        let node_prev = self.nodes[node].previous;
        let node_next = self.nodes[node].next;
        point_in_ear(
            self.x(node),
            self.y(node),
            self.x(ear_prev),
            self.y(ear_prev),
            self.x(ear),
            self.y(ear),
            self.x(ear_next),
            self.y(ear_next),
        ) && area(
            self.x(node_prev),
            self.y(node_prev),
            self.x(node),
            self.y(node),
            self.x(node_next),
            self.y(node_next),
        ) >= 0.0
    }

    /// Uses the Morton code for speed to determine whether a polygon node forms
    /// a valid ear with adjacent nodes.
    ///
    /// Equivalent to the private `Tessellator.mortonIsEar(Node)`.
    fn morton_is_ear(&self, ear: usize) -> bool {
        let ear_prev = self.nodes[ear].previous;
        let ear_next = self.nodes[ear].next;
        // triangle bbox (flip the bits so negative encoded values are < positive encoded values)
        let min_tx = flip_sign(
            self.nodes[ear_prev]
                .x
                .min(self.nodes[ear].x)
                .min(self.nodes[ear_next].x),
        );
        let min_ty = flip_sign(
            self.nodes[ear_prev]
                .y
                .min(self.nodes[ear].y)
                .min(self.nodes[ear_next].y),
        );
        let max_tx = flip_sign(
            self.nodes[ear_prev]
                .x
                .max(self.nodes[ear].x)
                .max(self.nodes[ear_next].x),
        );
        let max_ty = flip_sign(
            self.nodes[ear_prev]
                .y
                .max(self.nodes[ear].y)
                .max(self.nodes[ear_next].y),
        );

        // z-order range for the current triangle bbox
        let min_z = interleave(min_tx, min_ty);
        let max_z = interleave(max_tx, max_ty);

        // now make sure we don't have other points inside the potential ear;
        // look for points inside the triangle in both directions
        let mut p = self.nodes[ear].previous_z;
        let mut n = self.nodes[ear].next_z;
        while let (Some(p_node), Some(n_node)) = (p, n) {
            if self.nodes[p_node].morton < min_z || self.nodes[n_node].morton > max_z {
                break;
            }
            if self.nodes[p_node].idx != self.nodes[ear_prev].idx
                && self.nodes[p_node].idx != self.nodes[ear_next].idx
                && self.point_in_ear_and_reflex(p_node, ear_prev, ear, ear_next)
            {
                return false;
            }
            p = self.nodes[p_node].previous_z;

            if self.nodes[n_node].idx != self.nodes[ear_prev].idx
                && self.nodes[n_node].idx != self.nodes[ear_next].idx
                && self.point_in_ear_and_reflex(n_node, ear_prev, ear, ear_next)
            {
                return false;
            }
            n = self.nodes[n_node].next_z;
        }

        // first look for points inside the triangle in decreasing z-order
        while let Some(p_node) = p {
            if self.nodes[p_node].morton < min_z {
                break;
            }
            if self.nodes[p_node].idx != self.nodes[ear_prev].idx
                && self.nodes[p_node].idx != self.nodes[ear_next].idx
                && self.point_in_ear_and_reflex(p_node, ear_prev, ear, ear_next)
            {
                return false;
            }
            p = self.nodes[p_node].previous_z;
        }
        // then look for points in increasing z-order
        while let Some(n_node) = n {
            if self.nodes[n_node].morton > max_z {
                break;
            }
            if self.nodes[n_node].idx != self.nodes[ear_prev].idx
                && self.nodes[n_node].idx != self.nodes[ear_next].idx
                && self.point_in_ear_and_reflex(n_node, ear_prev, ear, ear_next)
            {
                return false;
            }
            n = self.nodes[n_node].next_z;
        }
        true
    }

    /// Builds the triangle Java constructs from three nodes.
    fn triangle(&self, a: usize, ab: bool, b: usize, bc: bool, c: usize, ca: bool) -> Triangle {
        Triangle {
            vertex: [self.vertex(a), self.vertex(b), self.vertex(c)],
            edge_from_polygon: [ab, bc, ca],
        }
    }

    fn vertex(&self, n: usize) -> TriangleVertex {
        let node = &self.nodes[n];
        TriangleVertex {
            x: node.x,
            y: node.y,
            x_val: node.x_val,
            y_val: node.y_val,
        }
    }

    /// Collects the ring's coordinates as points, for the monitor.
    ///
    /// Equivalent to the private `Tessellator.getPoints(Node)`.
    fn get_points(&self, start: usize) -> Result<Vec<Point>> {
        let mut node = start;
        let mut points = Vec::new();
        loop {
            points.push(Point::new(self.y(node), self.x(node))?);
            node = self.nodes[node].next;
            if node == start {
                break;
            }
        }
        Ok(points)
    }

    /// Iterates through all polygon nodes and removes small local
    /// self-intersections.
    ///
    /// Equivalent to the private `Tessellator.cureLocalIntersections(...)`.
    fn cure_local_intersections(
        &mut self,
        start_node: usize,
        tessellation: &mut Vec<Triangle>,
        morton_optimized: bool,
    ) -> usize {
        let mut start_node = start_node;
        let mut node = start_node;
        loop {
            let next_node = self.nodes[node].next;
            let a = self.nodes[node].previous;
            let b = self.nodes[next_node].next;

            // a self-intersection where edge (v[i-1],v[i]) intersects (v[i+1],v[i+2])
            if !self.is_vertex_equals_node(a, b)
                && lines_intersect(
                    self.x(a),
                    self.y(a),
                    self.x(node),
                    self.y(node),
                    self.x(next_node),
                    self.y(next_node),
                    self.x(b),
                    self.y(b),
                )
                && self.is_locally_inside(a, b)
                && self.is_locally_inside(b, a)
                // this call is expensive so do it last
                && !self.is_intersecting_polygon(a, self.x(a), self.y(a), self.x(b), self.y(b))
            {
                // compute edges from polygon
                let ab_from_polygon = if self.nodes[a].next == node {
                    self.nodes[a].is_next_edge_from_polygon
                } else {
                    self.is_edge_from_polygon(a, node, morton_optimized)
                };
                let bc_from_polygon = if self.nodes[node].next == b {
                    self.nodes[node].is_next_edge_from_polygon
                } else {
                    self.is_edge_from_polygon(node, b, morton_optimized)
                };
                let ca_from_polygon = if self.nodes[b].next == a {
                    self.nodes[b].is_next_edge_from_polygon
                } else {
                    self.is_edge_from_polygon(a, b, morton_optimized)
                };
                // Return the triangulated vertices to the tessellation. Lucene adds the same
                // triangle twice here; the port keeps that, because the tessellation is compared
                // triangle for triangle against Lucene's output.
                tessellation.push(self.triangle(
                    a,
                    ab_from_polygon,
                    node,
                    bc_from_polygon,
                    b,
                    ca_from_polygon,
                ));
                tessellation.push(self.triangle(
                    a,
                    ab_from_polygon,
                    node,
                    bc_from_polygon,
                    b,
                    ca_from_polygon,
                ));

                // remove two nodes involved
                self.remove_node(node, ca_from_polygon);
                let node_next = self.nodes[node].next;
                self.remove_node(node_next, ca_from_polygon);
                node = b;
                start_node = b;
            }
            node = self.nodes[node].next;
            if node == start_node {
                break;
            }
        }

        node
    }

    /// Computes whether the edge defined by `a` and `b` overlaps with a polygon
    /// edge, raising the errors Java raises for self-intersections.
    ///
    /// Equivalent to the private `Tessellator.checkIntersection(Node, boolean)`.
    fn check_intersection(&self, a: usize, is_morton: bool) -> Result<()> {
        let a_prev = self.nodes[a].previous;
        let mut next = self.nodes[a].next;
        loop {
            let mut inner_next = self.nodes[next].next;
            if is_morton {
                self.morton_check_intersection(next, inner_next)?;
            } else {
                let next_prev = self.nodes[next].previous;
                loop {
                    self.check_intersection_point(next, inner_next)?;
                    inner_next = self.nodes[inner_next].next;
                    if inner_next == next_prev {
                        break;
                    }
                }
            }
            next = self.nodes[next].next;
            if next == a_prev {
                break;
            }
        }
        Ok(())
    }

    /// Uses the Morton code for speed to determine whether the edge defined by
    /// `a` and `b` overlaps with a polygon edge.
    ///
    /// Equivalent to the private `Tessellator.mortonCheckIntersection(Node, Node)`.
    fn morton_check_intersection(&self, a: usize, b: usize) -> Result<()> {
        let a_next = self.nodes[a].next;
        // edge bbox (flip the bits so negative encoded values are < positive encoded values)
        let min_tx = flip_sign(self.nodes[a].x.min(self.nodes[a_next].x));
        let min_ty = flip_sign(self.nodes[a].y.min(self.nodes[a_next].y));
        let max_tx = flip_sign(self.nodes[a].x.max(self.nodes[a_next].x));
        let max_ty = flip_sign(self.nodes[a].y.max(self.nodes[a_next].y));

        // z-order range for the current edge
        let min_z = interleave(min_tx, min_ty);
        let max_z = interleave(max_tx, max_ty);

        // look for points inside the edge in both directions
        let mut p = self.nodes[b].previous_z;
        let mut n = self.nodes[b].next_z;
        while let (Some(p_node), Some(n_node)) = (p, n) {
            if self.nodes[p_node].morton < min_z || self.nodes[n_node].morton > max_z {
                break;
            }
            self.check_intersection_point(p_node, a)?;
            p = self.nodes[p_node].previous_z;
            self.check_intersection_point(n_node, a)?;
            n = self.nodes[n_node].next_z;
        }

        // first look for points inside the edge in decreasing z-order
        while let Some(p_node) = p {
            if self.nodes[p_node].morton < min_z {
                break;
            }
            self.check_intersection_point(p_node, a)?;
            p = self.nodes[p_node].previous_z;
        }
        // then look for points in increasing z-order
        while let Some(n_node) = n {
            if self.nodes[n_node].morton > max_z {
                break;
            }
            self.check_intersection_point(n_node, a)?;
            n = self.nodes[n_node].next_z;
        }
        Ok(())
    }

    /// Equivalent to the private `Tessellator.checkIntersectionPoint(Node, Node)`.
    fn check_intersection_point(&self, a: usize, b: usize) -> Result<()> {
        if a == b {
            return Ok(());
        }
        let a_next = self.nodes[a].next;
        let b_next = self.nodes[b].next;

        if self.y(a).max(self.y(a_next)) <= self.y(b).min(self.y(b_next))
            || self.y(a).min(self.y(a_next)) >= self.y(b).max(self.y(b_next))
            || self.x(a).max(self.x(a_next)) <= self.x(b).min(self.x(b_next))
            || self.x(a).min(self.x(a_next)) >= self.x(b).max(self.x(b_next))
        {
            return Ok(());
        }

        if GeoUtils::line_crosses_line(
            self.x(a),
            self.y(a),
            self.x(a_next),
            self.y(a_next),
            self.x(b),
            self.y(b),
            self.x(b_next),
            self.y(b_next),
        ) {
            // Line AB represented as a1x + b1y = c1
            let a1 = self.y(a_next) - self.y(a);
            let b1 = self.x(a) - self.x(a_next);
            let c1 = a1 * self.x(a) + b1 * self.y(a);

            // Line CD represented as a2x + b2y = c2
            let a2 = self.y(b_next) - self.y(b);
            let b2 = self.x(b) - self.x(b_next);
            let c2 = a2 * self.x(b) + b2 * self.y(b);

            let determinant = a1 * b2 - a2 * b1;
            debug_assert!(determinant != 0.0);

            let x = (b2 * c1 - b1 * c2) / determinant;
            let y = (a1 * c2 - a2 * c1) / determinant;

            return Err(LuceneError::IllegalArgument(format!(
                "Polygon self-intersection at lat={y} lon={x}"
            )));
        }
        if self.nodes[a].is_next_edge_from_polygon
            && self.nodes[b].is_next_edge_from_polygon
            && GeoUtils::line_overlap_line(
                self.x(a),
                self.y(a),
                self.x(a_next),
                self.y(a_next),
                self.x(b),
                self.y(b),
                self.x(b_next),
                self.y(b_next),
            )
        {
            return Err(LuceneError::IllegalArgument(format!(
                "Polygon ring self-intersection at lat={} lon={}",
                self.y(a),
                self.x(a)
            )));
        }
        Ok(())
    }

    /// Checks whether the provided vertex is in the polygon and returns it.
    ///
    /// Equivalent to the private `Tessellator.getSharedVertex(Node, Node)`.
    fn get_shared_vertex(&self, polygon: usize, vertex: usize) -> Option<usize> {
        let mut next = polygon;
        loop {
            if self.is_vertex_equals_node(next, vertex) {
                return Some(next);
            }
            next = self.nodes[next].next;
            if next == polygon {
                break;
            }
        }
        None
    }

    /// Chooses the vertex that has a smaller angle with the hole vertex.
    ///
    /// Equivalent to the package-private
    /// `Tessellator.getSharedInsideVertex(Node, Node, Node)`.
    fn get_shared_inside_vertex(
        &self,
        hole_vertex: usize,
        candidate_a: usize,
        candidate_b: usize,
    ) -> usize {
        debug_assert!(
            self.is_vertex_equals_node(hole_vertex, candidate_a)
                && self.is_vertex_equals_node(hole_vertex, candidate_b)
        );
        // we are joining candidate.prevNode -> holeVertex.node -> holeVertex.nextNode.
        // A negative area means a convex angle. If both are convex/reflex, choose the point of
        // minimum angle.
        let a_prev = self.nodes[candidate_a].previous;
        let b_prev = self.nodes[candidate_b].previous;
        let hole_next = self.nodes[hole_vertex].next;
        let a1 = area(
            self.x(a_prev),
            self.y(a_prev),
            self.x(hole_vertex),
            self.y(hole_vertex),
            self.x(hole_next),
            self.y(hole_next),
        );
        let a2 = area(
            self.x(b_prev),
            self.y(b_prev),
            self.x(hole_vertex),
            self.y(hole_vertex),
            self.x(hole_next),
            self.y(hole_next),
        );

        if (a1 < 0.0) != (a2 < 0.0) {
            // one is convex, the other reflex, get the convex one
            if a1 < a2 {
                candidate_a
            } else {
                candidate_b
            }
        } else {
            // both are convex / reflex, choose the smallest angle
            let angle1 = self.angle(a_prev, candidate_a, hole_next);
            let angle2 = self.angle(b_prev, candidate_b, hole_next);
            if angle1 < angle2 {
                candidate_a
            } else {
                candidate_b
            }
        }
    }

    /// Equivalent to the private `Tessellator.angle(Node, Node, Node)`.
    fn angle(&self, a: usize, b: usize, c: usize) -> f64 {
        let ax = self.x(a) - self.x(b);
        let ay = self.y(a) - self.y(b);
        let cx = self.x(c) - self.x(b);
        let cy = self.y(c) - self.y(b);
        let dot_product = ax * cx + ay * cy;
        let a_length = (ax * ax + ay * ay).sqrt();
        let b_length = (cx * cx + cy * cy).sqrt();
        (dot_product / (a_length * b_length)).acos()
    }

    /// David Eberly's algorithm for finding a bridge between a hole and the
    /// outer polygon.
    ///
    /// Equivalent to the private `Tessellator.fetchHoleBridge(Node, Node)`; see
    /// <http://www.geometrictools.com/Documentation/TriangulationByEarClipping.pdf>.
    fn fetch_hole_bridge(&self, hole_node: usize, outer_node: usize) -> Option<usize> {
        let mut p = outer_node;
        let mut qx = f64::NEG_INFINITY;
        let hx = self.x(hole_node);
        let hy = self.y(hole_node);
        let mut connection: Option<usize> = None;
        // 1. find a segment intersected by a ray from the hole's leftmost point to the left;
        // the segment's endpoint with lesser x will be the potential connection point
        loop {
            let p_next = self.nodes[p].next;
            if hy <= self.y(p) && hy >= self.y(p_next) && self.y(p_next) != self.y(p) {
                let x = self.x(p)
                    + (hy - self.y(p)) * (self.x(p_next) - self.x(p))
                        / (self.y(p_next) - self.y(p));
                if x <= hx && x > qx {
                    qx = x;
                    if x == hx {
                        if hy == self.y(p) {
                            return Some(p);
                        }
                        if hy == self.y(p_next) {
                            return Some(p_next);
                        }
                    }
                    connection = Some(if self.x(p) < self.x(p_next) {
                        p
                    } else {
                        p_next
                    });
                }
            }
            p = self.nodes[p].next;
            if p == outer_node {
                break;
            }
        }

        let mut connection = connection?;
        if hx == qx {
            return Some(self.nodes[connection].previous);
        }

        // 2. look for points inside the triangle of the hole point, the segment intersection and
        // the endpoint; it is a valid connection iff no points are found, otherwise choose the
        // point of the minimum angle with the ray as the connection point
        let stop = connection;
        let mx = self.x(connection);
        let my = self.y(connection);
        let mut tan_min = f64::INFINITY;
        let mut p = connection;
        loop {
            if hx >= self.x(p)
                && self.x(p) >= mx
                && hx != self.x(p)
                && point_in_ear(
                    self.x(p),
                    self.y(p),
                    if hy < my { hx } else { qx },
                    hy,
                    mx,
                    my,
                    if hy < my { qx } else { hx },
                    hy,
                )
            {
                let tan = (hy - self.y(p)).abs() / (hx - self.x(p)); // tangential
                if (tan < tan_min || (tan == tan_min && self.x(p) > self.x(connection)))
                    && self.is_locally_inside(p, hole_node)
                {
                    connection = p;
                    tan_min = tan;
                }
            }
            p = self.nodes[p].next;
            if p == stop {
                break;
            }
        }
        Some(connection)
    }

    /// Chooses a common vertex between the polygon and the hole, if one exists,
    /// and merges them.
    ///
    /// Equivalent to the private
    /// `Tessellator.maybeMergeHoleWithSharedVertices(...)`. Returns the bridge
    /// node for leftmost shared-vertex merges, `outer_node` for non-leftmost
    /// merges, or [`None`] when no shared vertex was found.
    #[allow(clippy::too_many_arguments)]
    fn maybe_merge_hole_with_shared_vertices(
        &mut self,
        hole_node: usize,
        outer_node: usize,
        hole_min_x: f64,
        hole_max_x: f64,
        hole_min_y: f64,
        hole_max_y: f64,
    ) -> Option<usize> {
        // Attempt to find a common point between the HoleNode and OuterNode.
        let mut shared_vertex: Option<usize> = None;
        let mut shared_vertex_connection: Option<usize> = None;
        // Track the leftmost vertex match (holeNode is the leftmost vertex of the hole).
        // Use first-match in ring order for the leftmost case (matching earcut.js). When previous
        // bridge operations created multiple copies of a vertex, the first copy from outerNode is
        // the correct connection point for chained shared-vertex holes.
        let mut leftmost_shared_vertex_connection: Option<usize> = None;
        let mut next = outer_node;
        loop {
            if crate::geo::geometry::Rectangle::contains_point(
                self.y(next),
                self.x(next),
                hole_min_y,
                hole_max_y,
                hole_min_x,
                hole_max_x,
            ) {
                if let Some(new_shared_vertex) = self.get_shared_vertex(hole_node, next) {
                    // Check if this shared vertex is the leftmost point of the hole (holeNode)
                    if self.is_vertex_equals_node(new_shared_vertex, hole_node)
                        && leftmost_shared_vertex_connection.is_none()
                    {
                        leftmost_shared_vertex_connection = Some(next);
                        // For leftmost, take first match only — don't use getSharedInsideVertex.
                    }
                    match shared_vertex {
                        None => {
                            shared_vertex = Some(new_shared_vertex);
                            shared_vertex_connection = Some(next);
                        }
                        Some(existing) if new_shared_vertex == existing => {
                            // Same vertex found again via a different connection.
                            let previous = shared_vertex_connection
                                .expect("INVARIANT: set together with sharedVertex");
                            shared_vertex_connection =
                                Some(self.get_shared_inside_vertex(existing, previous, next));
                        }
                        Some(_) => {}
                    }
                }
            }
            next = self.nodes[next].next;
            if next == outer_node {
                break;
            }
        }

        // The leftmost vertex of the hole is a shared vertex. Prefer this connection point if it
        // comes from a hole that was already merged (higher idx), as this maintains proper
        // connectivity for chained holes.
        if let Some(leftmost) = leftmost_shared_vertex_connection {
            let connection = shared_vertex_connection.expect(
                "INVARIANT: a leftmost match always sets sharedVertexConnection in the same pass",
            );
            if self.nodes[leftmost].idx >= self.nodes[connection].idx {
                self.split_polygon(leftmost, hole_node, true);
                // When multiple copies of the shared vertex exist (from previous splitPolygon
                // calls), return the bridge node so the caller updates outerNode to the bridge
                // position. This ensures subsequent holes sharing the same vertex find the right
                // copy.
                if leftmost != connection {
                    return Some(leftmost);
                }
                return Some(outer_node);
            }
        }
        if let Some(shared_vertex) = shared_vertex {
            // Split the resulting polygon.
            let connection =
                shared_vertex_connection.expect("INVARIANT: set together with sharedVertex");
            self.split_polygon(connection, shared_vertex, true);
            return Some(outer_node);
        }
        None
    }

    /// Finds a bridge between vertices that connects a hole with an outer ring,
    /// links it, and returns the node to use as the new `outerNode` position,
    /// or [`None`] if no bridge was found.
    ///
    /// Equivalent to the private `Tessellator.eliminateHole(...)`.
    #[allow(clippy::too_many_arguments)]
    fn eliminate_hole(
        &mut self,
        hole_node: usize,
        outer_node: usize,
        hole_min_x: f64,
        hole_max_x: f64,
        hole_min_y: f64,
        hole_max_y: f64,
    ) -> Option<usize> {
        // Attempt to merge the hole using a common point between them if it exists.
        if let Some(merge_result) = self.maybe_merge_hole_with_shared_vertices(
            hole_node, outer_node, hole_min_x, hole_max_x, hole_min_y, hole_max_y,
        ) {
            return Some(merge_result);
        }
        // Attempt to find a logical bridge between the HoleNode and OuterNode.
        let bridge = self.fetch_hole_bridge(hole_node, outer_node)?;

        // compute if the bridge overlaps with a polygon edge
        let bridge_next = self.nodes[bridge].next;
        let bridge_prev = self.nodes[bridge].previous;
        let hole_next = self.nodes[hole_node].next;
        let hole_prev = self.nodes[hole_node].previous;
        let from_polygon = self.is_point_in_line(bridge, bridge_next, hole_node)
            || self.is_point_in_line(bridge, bridge_prev, hole_node)
            || self.is_point_in_line(hole_node, hole_next, bridge)
            || self.is_point_in_line(hole_node, hole_prev, bridge);
        // Split the resulting polygon.
        self.split_polygon(bridge, hole_node, from_polygon);
        Some(outer_node)
    }

    /// Links every hole into the outer loop, producing a single-ring polygon
    /// without holes.
    ///
    /// Equivalent to the private
    /// `Tessellator.eliminateHoles(List, Map, Node)`.
    fn eliminate_holes(
        &mut self,
        hole_list: &mut Vec<HoleEntry>,
        source: &HoleSource<'_>,
        outer_node: usize,
    ) -> Result<Option<usize>> {
        // Sort the hole vertices by x coordinate
        hole_list.sort_by(|a, b| {
            let mut diff = self.x(a.leftmost) - self.x(b.leftmost);
            if diff == 0.0 {
                diff = self.y(a.leftmost) - self.y(b.leftmost);
                if diff == 0.0 {
                    // same hole node
                    let av = self
                        .y(self.previous(a.leftmost))
                        .min(self.y(self.next(a.leftmost)));
                    let bv = self
                        .y(self.previous(b.leftmost))
                        .min(self.y(self.next(b.leftmost)));
                    diff = av - bv;
                }
            }
            if diff < 0.0 {
                std::cmp::Ordering::Less
            } else if diff > 0.0 {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        });

        let mut outer_node = outer_node;
        // Process holes from left to right.
        for entry in hole_list.iter() {
            // Eliminate hole triangles from the result set
            let result = self.eliminate_hole(
                entry.leftmost,
                outer_node,
                entry.min_x,
                entry.max_x,
                entry.min_y,
                entry.max_y,
            );
            match result {
                Some(result) => {
                    // Filter the new polygon. The result node determines outerNode's position:
                    // for leftmost shared-vertex merges it is at the bridge, otherwise at the
                    // original.
                    let next = self.nodes[result].next;
                    outer_node = self
                        .filter_points(Some(result), Some(next))
                        .expect("INVARIANT: filterPoints returns a node for a non-null start");
                }
                None => {
                    // we couldn't find a point to the left of the hole's leftmost point, so the
                    // point is not inside the polygon.
                    let mut polygon = match source {
                        HoleSource::Geo(p) => {
                            let hole = p.get_hole(entry.hole_index);
                            Polygon::vertices_to_geojson(hole.get_polylats(), hole.get_polylons())
                        }
                        HoleSource::Xy(p) => {
                            let hole = p.get_hole(entry.hole_index);
                            XYPolygon::vertices_to_geojson(hole.get_polyx(), hole.get_polyy())
                        }
                    };
                    if polygon.len() > 100 {
                        polygon = format!("{}...", &polygon[..100]);
                    }
                    return Err(LuceneError::IllegalArgument(format!(
                        "Illegal hole detected: {polygon}"
                    )));
                }
            }
        }
        // Filter co-planar nodes and return a pointer to the list.
        Ok(self.filter_points(Some(outer_node), None))
    }

    /// Attempts to split a polygon and independently triangulate each side.
    ///
    /// Equivalent to the private `Tessellator.splitEarcut(...)`. Returns
    /// whether the polygon was split.
    #[allow(clippy::too_many_arguments)]
    fn split_earcut(
        &mut self,
        start: usize,
        tessellation: &mut Vec<Triangle>,
        morton_optimized: bool,
        monitor: &mut Option<&mut dyn Monitor>,
        depth: usize,
    ) -> Result<bool> {
        // Search for a valid diagonal that divides the polygon into two.
        let mut search_node = start;
        loop {
            let next_node = self.nodes[search_node].next;
            let mut diagonal = self.nodes[next_node].next;
            while diagonal != self.nodes[search_node].previous {
                if self.nodes[search_node].idx != self.nodes[diagonal].idx
                    && self.is_valid_diagonal(search_node, diagonal)
                {
                    // Split the polygon into two at the point of the diagonal
                    let edge_from_polygon =
                        self.is_edge_from_polygon(search_node, diagonal, morton_optimized);
                    let split_node = self.split_polygon(search_node, diagonal, edge_from_polygon);
                    // Filter the resulting polygon.
                    let search_next = self.nodes[search_node].next;
                    let mut search_node = self
                        .filter_points(Some(search_node), Some(search_next))
                        .expect("INVARIANT: filterPoints returns a node for a non-null start");
                    let split_next = self.nodes[split_node].next;
                    let mut split_node = self
                        .filter_points(Some(split_node), Some(split_next))
                        .expect("INVARIANT: filterPoints returns a node for a non-null start");
                    // Attempt to earcut both of the resulting polygons
                    if morton_optimized {
                        self.sort_by_morton_with_reset(search_node);
                        self.sort_by_morton_with_reset(split_node);
                    }
                    self.notify_monitor_split(depth, monitor, search_node, split_node)?;
                    self.earcut_linked_list(
                        Some(search_node),
                        tessellation,
                        State::Init,
                        morton_optimized,
                        monitor,
                        depth,
                    )?;
                    // The recursive call cannot move nodes in the arena, but it can relink them;
                    // re-read is unnecessary, the local indices stay valid.
                    search_node = search_node;
                    split_node = split_node;
                    self.earcut_linked_list(
                        Some(split_node),
                        tessellation,
                        State::Init,
                        morton_optimized,
                        monitor,
                        depth,
                    )?;
                    self.notify_monitor_split_end(depth, monitor);
                    // Finish the iterative search
                    return Ok(true);
                }
                diagonal = self.nodes[diagonal].next;
            }
            search_node = self.nodes[search_node].next;
            if search_node == start {
                break;
            }
        }
        // if there is some area left, we failed
        Ok(self.signed_area(start, start) == 0.0)
    }

    /// Main ear-slicing loop which triangulates the vertices of a polygon
    /// provided as a doubly-linked list.
    ///
    /// Equivalent to the private `Tessellator.earcutLinkedList(...)`.
    #[allow(clippy::too_many_arguments)]
    fn earcut_linked_list(
        &mut self,
        curr_ear: Option<usize>,
        tessellation: &mut Vec<Triangle>,
        state: State,
        morton_optimized: bool,
        monitor: &mut Option<&mut dyn Monitor>,
        depth: usize,
    ) -> Result<()> {
        let mut curr_ear_opt = curr_ear;
        let mut state = state;
        'earcut: loop {
            let mut curr_ear = match curr_ear_opt {
                None => return Ok(()),
                Some(c) => c,
            };
            if self.nodes[curr_ear].previous == self.nodes[curr_ear].next {
                return Ok(());
            }

            let mut stop = curr_ear;

            // Iteratively slice ears
            loop {
                self.notify_monitor_state(state, depth, monitor, Some(curr_ear), tessellation)?;
                let prev_node = self.nodes[curr_ear].previous;
                let next_node = self.nodes[curr_ear].next;
                // Determine whether the current triangle must be cut off.
                let is_reflex = area(
                    self.x(prev_node),
                    self.y(prev_node),
                    self.x(curr_ear),
                    self.y(curr_ear),
                    self.x(next_node),
                    self.y(next_node),
                ) >= 0.0;
                if !is_reflex && self.is_ear(curr_ear, morton_optimized) {
                    // Compute if edges belong to the polygon
                    let ab_from_polygon = self.nodes[prev_node].is_next_edge_from_polygon;
                    let bc_from_polygon = self.nodes[curr_ear].is_next_edge_from_polygon;
                    let ca_from_polygon =
                        self.is_edge_from_polygon(prev_node, next_node, morton_optimized);
                    // Return the triangulated data
                    tessellation.push(self.triangle(
                        prev_node,
                        ab_from_polygon,
                        curr_ear,
                        bc_from_polygon,
                        next_node,
                        ca_from_polygon,
                    ));
                    // Remove the ear node.
                    self.remove_node(curr_ear, ca_from_polygon);

                    // Skipping to the next node leaves fewer slither triangles.
                    curr_ear = self.nodes[next_node].next;
                    stop = self.nodes[next_node].next;
                    // Java's `continue` in a do-while re-evaluates the loop condition.
                    if self.nodes[curr_ear].previous == self.nodes[curr_ear].next {
                        break;
                    }
                    continue;
                }
                curr_ear = next_node;
                // If the whole polygon has been iterated over and no more ears can be found.
                if curr_ear == stop {
                    match state {
                        State::Init => {
                            // try filtering points and slicing again
                            curr_ear_opt = self.filter_points(Some(curr_ear), None);
                            state = State::Cure;
                            continue 'earcut;
                        }
                        State::Cure => {
                            // if this didn't work, try curing all small self-intersections locally
                            curr_ear_opt = Some(self.cure_local_intersections(
                                curr_ear,
                                tessellation,
                                morton_optimized,
                            ));
                            state = State::Split;
                            continue 'earcut;
                        }
                        State::Split => {
                            // as a last resort, try splitting the remaining polygon into two
                            if !self.split_earcut(
                                curr_ear,
                                tessellation,
                                morton_optimized,
                                monitor,
                                depth + 1,
                            )? {
                                // we could not process all points; tessellation failed
                                let status = format!("{}[FAILED]", state.name());
                                self.notify_monitor(
                                    &status,
                                    monitor,
                                    Some(curr_ear),
                                    tessellation,
                                )?;
                                return Err(LuceneError::IllegalArgument(
                                    "Unable to Tessellate shape. Possible malformed shape detected."
                                        .to_string(),
                                ));
                            }
                        }
                    }
                    break;
                }
                if self.nodes[curr_ear].previous == self.nodes[curr_ear].next {
                    break;
                }
            }
            break;
        }
        Ok(())
    }

    fn notify_monitor_split(
        &self,
        depth: usize,
        monitor: &mut Option<&mut dyn Monitor>,
        search_node: usize,
        diagonal_node: usize,
    ) -> Result<()> {
        if monitor.is_some() {
            let left = self.get_points(search_node)?;
            let right = self.get_points(diagonal_node)?;
            if let Some(m) = monitor.as_deref_mut() {
                m.start_split(&format!("SPLIT[{depth}]"), &left, &right);
            }
        }
        Ok(())
    }

    fn notify_monitor_split_end(&self, depth: usize, monitor: &mut Option<&mut dyn Monitor>) {
        if let Some(m) = monitor.as_deref_mut() {
            m.end_split(&format!("SPLIT[{depth}]"));
        }
    }

    fn notify_monitor_state(
        &self,
        state: State,
        depth: usize,
        monitor: &mut Option<&mut dyn Monitor>,
        start: Option<usize>,
        tessellation: &[Triangle],
    ) -> Result<()> {
        if monitor.is_some() {
            let status = if depth == 0 {
                state.name().to_string()
            } else {
                format!("{}[{}]", state.name(), depth)
            };
            self.notify_monitor(&status, monitor, start, tessellation)?;
        }
        Ok(())
    }

    fn notify_monitor(
        &self,
        status: &str,
        monitor: &mut Option<&mut dyn Monitor>,
        start: Option<usize>,
        tessellation: &[Triangle],
    ) -> Result<()> {
        if monitor.is_some() {
            let points = match start {
                None => None,
                Some(s) => Some(self.get_points(s)?),
            };
            if let Some(m) = monitor.as_deref_mut() {
                m.current_state(status, points.as_deref(), tessellation);
            }
        }
        Ok(())
    }
}

/// Computes the signed area of a triangle; a negative value means a convex
/// angle and a positive one a reflex angle.
///
/// Equivalent to the private `Tessellator.area(...)`.
fn area(a_x: f64, a_y: f64, b_x: f64, b_y: f64, c_x: f64, c_y: f64) -> f64 {
    (b_y - a_y) * (c_x - b_x) - (b_x - a_x) * (c_y - b_y)
}

/// Computes whether a point is in a candidate ear.
///
/// Equivalent to the private `Tessellator.pointInEar(...)`.
#[allow(clippy::too_many_arguments)]
fn point_in_ear(x: f64, y: f64, ax: f64, ay: f64, bx: f64, by: f64, cx: f64, cy: f64) -> bool {
    (cx - x) * (ay - y) - (ax - x) * (cy - y) >= 0.0
        && (ax - x) * (by - y) - (bx - x) * (ay - y) >= 0.0
        && (bx - x) * (cy - y) - (cx - x) * (by - y) >= 0.0
}

/// Determines whether two line segments intersect.
///
/// Equivalent to the public static `Tessellator.linesIntersect(...)`.
#[allow(clippy::too_many_arguments)]
pub fn lines_intersect(
    a_x0: f64,
    a_y0: f64,
    a_x1: f64,
    a_y1: f64,
    b_x0: f64,
    b_y0: f64,
    b_x1: f64,
    b_y1: f64,
) -> bool {
    (area(a_x0, a_y0, a_x1, a_y1, b_x0, b_y0) > 0.0)
        != (area(a_x0, a_y0, a_x1, a_y1, b_x1, b_y1) > 0.0)
        && (area(b_x0, b_y0, b_x1, b_y1, a_x0, a_y0) > 0.0)
            != (area(b_x0, b_y0, b_x1, b_y1, a_x1, a_y1) > 0.0)
}

impl Tessellator {
    /// Determines whether two line segments intersect.
    ///
    /// Equivalent to the public static `Tessellator.linesIntersect(...)`.
    #[allow(clippy::too_many_arguments)]
    pub fn lines_intersect(
        a_x0: f64,
        a_y0: f64,
        a_x1: f64,
        a_y1: f64,
        b_x0: f64,
        b_y0: f64,
        b_x1: f64,
        b_y1: f64,
    ) -> bool {
        lines_intersect(a_x0, a_y0, a_x1, a_y1, b_x0, b_y0, b_x1, b_y1)
    }

    /// Tessellates a latitude/longitude polygon.
    ///
    /// Equivalent to `Tessellator.tessellate(Polygon, boolean)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for a malformed shape, an
    /// illegal hole, a self-intersection (when `check_self_intersections` is
    /// set) or a shape that cannot be tessellated.
    pub fn tessellate(polygon: &Polygon, check_self_intersections: bool) -> Result<Vec<Triangle>> {
        Self::tessellate_monitored(polygon, check_self_intersections, None)
    }

    /// Tessellates a latitude/longitude polygon, reporting progress to a
    /// [`Monitor`].
    ///
    /// Equivalent to `Tessellator.tessellate(Polygon, boolean, Monitor)`.
    ///
    /// # Errors
    ///
    /// See [`Tessellator::tessellate`].
    pub fn tessellate_monitored(
        polygon: &Polygon,
        check_self_intersections: bool,
        monitor: Option<&mut dyn Monitor>,
    ) -> Result<Vec<Triangle>> {
        let mut t = Tessellation::new();
        let mut monitor = monitor;
        // Attempt to establish a doubly-linked list of the provided shell points (should be CCW,
        // but this will correct); then filter instances of intersections.
        let mut outer_node = t
            .create_doubly_linked_list(
                polygon.get_polylons(),
                polygon.get_polylats(),
                polygon.get_winding_order(),
                true,
                0,
                WindingOrder::CW,
            )?
            // If an outer node hasn't been detected, the shape is malformed. (must comply with
            // OGC SFA specification)
            .ok_or_else(|| {
                LuceneError::IllegalArgument("Malformed shape detected in Tessellator!".to_string())
            })?;
        if outer_node == t.next(outer_node) || outer_node == t.next(t.next(outer_node)) {
            return Err(LuceneError::IllegalArgument(
                "at least three non-collinear points required".to_string(),
            ));
        }

        // Determine if the specified list of points contains holes
        if polygon.num_holes() > 0 {
            // Eliminate the hole triangulation.
            outer_node = Self::eliminate_holes_geo(&mut t, polygon, outer_node)?;
        }

        // If the shape crosses VERTEX_THRESHOLD, use z-order curve hashing
        let mut threshold = VERTEX_THRESHOLD - polygon.num_points() as i32;
        let mut i = 0;
        while threshold >= 0 && i < polygon.num_holes() {
            threshold -= polygon.get_hole(i).num_points() as i32;
            i += 1;
        }

        // Link polygon nodes in Z-Order
        let morton_optimized = threshold < 0;
        if morton_optimized {
            t.sort_by_morton(outer_node);
        }

        if check_self_intersections {
            t.check_intersection(outer_node, morton_optimized)?;
        }
        // Calculate the tessellation using the doubly-linked list.
        let mut result: Vec<Triangle> = Vec::new();
        t.earcut_linked_list(
            Some(outer_node),
            &mut result,
            State::Init,
            morton_optimized,
            &mut monitor,
            0,
        )?;
        if result.is_empty() {
            t.notify_monitor(MONITOR_FAILED, &mut monitor, None, &result)?;
            return Err(LuceneError::IllegalArgument(
                "Unable to Tessellate shape. Possible malformed shape detected.".to_string(),
            ));
        }
        t.notify_monitor(MONITOR_COMPLETED, &mut monitor, None, &result)?;

        Ok(result)
    }

    /// Tessellates a cartesian polygon.
    ///
    /// Equivalent to `Tessellator.tessellate(XYPolygon, boolean)`.
    ///
    /// # Errors
    ///
    /// See [`Tessellator::tessellate`].
    pub fn tessellate_xy(
        polygon: &XYPolygon,
        check_self_intersections: bool,
    ) -> Result<Vec<Triangle>> {
        Self::tessellate_xy_monitored(polygon, check_self_intersections, None)
    }

    /// Tessellates a cartesian polygon, reporting progress to a [`Monitor`].
    ///
    /// Equivalent to `Tessellator.tessellate(XYPolygon, boolean, Monitor)`.
    ///
    /// # Errors
    ///
    /// See [`Tessellator::tessellate`].
    pub fn tessellate_xy_monitored(
        polygon: &XYPolygon,
        check_self_intersections: bool,
        monitor: Option<&mut dyn Monitor>,
    ) -> Result<Vec<Triangle>> {
        let mut t = Tessellation::new();
        let mut monitor = monitor;
        let xs = XYEncodingUtils::float_array_to_double_array(polygon.get_polyx());
        let ys = XYEncodingUtils::float_array_to_double_array(polygon.get_polyy());
        let mut outer_node = t
            .create_doubly_linked_list(
                &xs,
                &ys,
                polygon.get_winding_order(),
                false,
                0,
                WindingOrder::CW,
            )?
            .ok_or_else(|| {
                LuceneError::IllegalArgument("Malformed shape detected in Tessellator!".to_string())
            })?;
        if outer_node == t.next(outer_node) || outer_node == t.next(t.next(outer_node)) {
            return Err(LuceneError::IllegalArgument(
                "at least three non-collinear points required".to_string(),
            ));
        }

        // Determine if the specified list of points contains holes
        if polygon.num_holes() > 0 {
            // Eliminate the hole triangulation.
            outer_node = Self::eliminate_holes_xy(&mut t, polygon, outer_node)?;
        }

        // If the shape crosses VERTEX_THRESHOLD, use z-order curve hashing
        let mut threshold = VERTEX_THRESHOLD - polygon.num_points() as i32;
        let mut i = 0;
        while threshold >= 0 && i < polygon.num_holes() {
            threshold -= polygon.get_hole(i).num_points() as i32;
            i += 1;
        }

        // Link polygon nodes in Z-Order
        let morton_optimized = threshold < 0;
        if morton_optimized {
            t.sort_by_morton(outer_node);
        }

        if check_self_intersections {
            t.check_intersection(outer_node, morton_optimized)?;
        }
        let mut result: Vec<Triangle> = Vec::new();
        t.earcut_linked_list(
            Some(outer_node),
            &mut result,
            State::Init,
            morton_optimized,
            &mut monitor,
            0,
        )?;
        if result.is_empty() {
            t.notify_monitor(MONITOR_FAILED, &mut monitor, None, &result)?;
            return Err(LuceneError::IllegalArgument(
                "Unable to Tessellate shape. Possible malformed shape detected.".to_string(),
            ));
        }
        t.notify_monitor(MONITOR_COMPLETED, &mut monitor, None, &result)?;

        Ok(result)
    }

    /// Equivalent to the private `Tessellator.eliminateHoles(Polygon, Node)`.
    fn eliminate_holes_geo(
        t: &mut Tessellation,
        polygon: &Polygon,
        outer_node: usize,
    ) -> Result<usize> {
        // Define a list to hold a reference to each filtered hole list.
        let mut hole_list: Vec<HoleEntry> = Vec::new();
        // Iterate through each array of hole vertices.
        let mut node_index = polygon.num_points();
        for i in 0..polygon.num_holes() {
            let hole = polygon.get_hole(i);
            // create the doubly-linked hole list
            let list = t.create_doubly_linked_list(
                hole.get_polylons(),
                hole.get_polylats(),
                hole.get_winding_order(),
                true,
                node_index,
                WindingOrder::CCW,
            )?;
            // Java dereferences the list unconditionally here, so a null list is a
            // `NullPointerException`; the port keeps the coplanar check and reports the same
            // `IllegalArgumentException` for a degenerate ring.
            let list = list.ok_or_else(|| {
                LuceneError::IllegalArgument(format!("Points are all coplanar in hole: {hole:?}"))
            })?;
            if list == t.next(list) {
                return Err(LuceneError::IllegalArgument(format!(
                    "Points are all coplanar in hole: {hole:?}"
                )));
            }
            // Add the leftmost vertex of the hole.
            let left_most = t.fetch_leftmost(list);
            hole_list.push(HoleEntry {
                leftmost: left_most,
                hole_index: i,
                min_x: hole.min_lon(),
                max_x: hole.max_lon(),
                min_y: hole.min_lat(),
                max_y: hole.max_lat(),
            });
            node_index += hole.num_points();
        }
        let source = HoleSource::Geo(polygon);
        t.eliminate_holes(&mut hole_list, &source, outer_node)?
            .ok_or_else(|| {
                LuceneError::IllegalArgument("Malformed shape detected in Tessellator!".to_string())
            })
    }

    /// Equivalent to the private `Tessellator.eliminateHoles(XYPolygon, Node)`.
    fn eliminate_holes_xy(
        t: &mut Tessellation,
        polygon: &XYPolygon,
        outer_node: usize,
    ) -> Result<usize> {
        // Define a list to hold a reference to each filtered hole list.
        let mut hole_list: Vec<HoleEntry> = Vec::new();
        // Iterate through each array of hole vertices.
        let mut node_index = polygon.num_points();
        for i in 0..polygon.num_holes() {
            let hole = polygon.get_hole(i);
            // create the doubly-linked hole list
            let list = t.create_doubly_linked_list(
                &XYEncodingUtils::float_array_to_double_array(hole.get_polyx()),
                &XYEncodingUtils::float_array_to_double_array(hole.get_polyy()),
                hole.get_winding_order(),
                false,
                node_index,
                WindingOrder::CCW,
            )?;
            // Determine if the resulting hole polygon was successful.
            if let Some(list) = list {
                // Add the leftmost vertex of the hole.
                let left_most = t.fetch_leftmost(list);
                hole_list.push(HoleEntry {
                    leftmost: left_most,
                    hole_index: i,
                    min_x: f64::from(hole.min_x()),
                    max_x: f64::from(hole.max_x()),
                    min_y: f64::from(hole.min_y()),
                    max_y: f64::from(hole.max_y()),
                });
            }
            node_index += hole.num_points();
        }
        let source = HoleSource::Xy(polygon);
        t.eliminate_holes(&mut hole_list, &source, outer_node)?
            .ok_or_else(|| {
                LuceneError::IllegalArgument("Malformed shape detected in Tessellator!".to_string())
            })
    }
}
