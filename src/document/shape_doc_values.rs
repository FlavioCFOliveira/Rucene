//! The binary doc-values representation of a shape, ported from
//! `org.apache.lucene.document`.
//!
//! [`LatLonShape`](crate::document::LatLonShape) and
//! [`XYShape`](crate::document::XYShape) index a shape as a flat list of
//! triangles. The same tessellation can instead be stored as a *single* binary
//! doc value, which this module writes and reads: the triangles are arranged
//! into a balanced binary tree (the same construction
//! `org.apache.lucene.geo.ComponentTree` performs in memory) and serialised
//! depth-first, every coordinate delta-encoded against its parent's maximum so
//! that a variable-length integer suffices. A spatial relation is then answered
//! by walking that tree straight out of the stored bytes, skipping whole
//! subtrees whose bounding box cannot match.

use crate::document::shape_field::{DecodedTriangle, TriangleType};
use crate::document::FieldType;
use crate::error::{LuceneError, Result};
use crate::geo::encoding::{GeoEncodingUtils, XYEncodingUtils};
use crate::geo::geometry::{Point, Rectangle, XYPoint, XYRectangle};
use crate::geo::Component2D;
use crate::index::point_values::Relation;
use crate::index::{DocValuesType, IndexableField, IndexableFieldType};
use crate::store::{DataInput, DataOutput};
use crate::util::selector::{intro_select, PivotOps};
use crate::util::BytesRef;

/// Doc-value format version, used to support backward compatibility for any
/// encoding change.
///
/// Equivalent to `ShapeDocValues.VERSION`.
pub const VERSION: u8 = 0;

// -----------------------------------------------------------------------------
// Encoder
// -----------------------------------------------------------------------------

/// Which coordinate system a shape doc value was encoded in.
///
/// Equivalent to `ShapeDocValues.Encoder` together with its two anonymous
/// implementations, in `LatLonShapeDocValues.getEncoder()` and
/// `XYShapeDocValues.getEncoder()`.
///
/// **Divergence from Lucene 10.5.0.** Java expresses the choice as an abstract
/// method returning a freshly allocated `Encoder` object. There are exactly two
/// implementations and they are stateless, so this port names them as an enum:
/// the dispatch is the same, the allocation disappears, and
/// [`ShapeDocValues`] stays `Clone` and `Debug`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeCoordinateSystem {
    /// Latitude/longitude degrees, encoded by
    /// [`GeoEncodingUtils`].
    Geographic,
    /// Unitless cartesian values, encoded by [`XYEncodingUtils`].
    Cartesian,
}

impl ShapeCoordinateSystem {
    /// Encodes an x coordinate.
    ///
    /// Equivalent to `ShapeDocValues.Encoder.encodeX(double)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for a coordinate outside the
    /// system's valid range, exactly as the Java encoders do.
    pub fn encode_x(self, x: f64) -> Result<i32> {
        match self {
            Self::Geographic => GeoEncodingUtils::encode_longitude(x),
            Self::Cartesian => XYEncodingUtils::encode(x as f32),
        }
    }

    /// Encodes a y coordinate.
    ///
    /// Equivalent to `ShapeDocValues.Encoder.encodeY(double)`.
    ///
    /// # Errors
    ///
    /// As [`Self::encode_x`].
    pub fn encode_y(self, y: f64) -> Result<i32> {
        match self {
            Self::Geographic => GeoEncodingUtils::encode_latitude(y),
            Self::Cartesian => XYEncodingUtils::encode(y as f32),
        }
    }

    /// Decodes an x coordinate.
    ///
    /// Equivalent to `ShapeDocValues.Encoder.decodeX(int)`.
    pub fn decode_x(self, encoded: i32) -> f64 {
        match self {
            Self::Geographic => GeoEncodingUtils::decode_longitude(encoded),
            Self::Cartesian => XYEncodingUtils::decode(encoded) as f64,
        }
    }

    /// Decodes a y coordinate.
    ///
    /// Equivalent to `ShapeDocValues.Encoder.decodeY(int)`.
    pub fn decode_y(self, encoded: i32) -> f64 {
        match self {
            Self::Geographic => GeoEncodingUtils::decode_latitude(encoded),
            Self::Cartesian => XYEncodingUtils::decode(encoded) as f64,
        }
    }
}

// -----------------------------------------------------------------------------
// Variable-length sizes
// -----------------------------------------------------------------------------

/// Returns how many bytes a variable-length long occupies.
///
/// Equivalent to the static `ShapeDocValues.vLongSize(long)`.
pub fn v_long_size(i: i64) -> i32 {
    let mut i = i as u64;
    let mut size = 0;
    while i & !0x7F != 0 {
        i >>= 7;
        size += 1;
    }
    size + 1
}

/// Returns how many bytes a variable-length integer occupies.
///
/// Equivalent to the static `ShapeDocValues.vIntSize(int)`.
pub fn v_int_size(i: i32) -> i32 {
    let mut i = i as u32;
    let mut size = 0;
    while i & !0x7F != 0 {
        i >>= 7;
        size += 1;
    }
    size + 1
}

// -----------------------------------------------------------------------------
// Reader
// -----------------------------------------------------------------------------

/// Reads a shape doc value out of its serialised bytes.
///
/// Equivalent to the private inner class `ShapeDocValues.Reader`, which wraps a
/// `ByteArrayDataInput`. This port borrows the bytes instead of copying them,
/// so a relation walk allocates nothing.
struct SliceDataInput<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> SliceDataInput<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    /// Equivalent to `Reader.rewind()`.
    fn rewind(&mut self) {
        self.pos = 0;
    }

    /// Equivalent to `ByteArrayDataInput.getPosition()`.
    fn position(&self) -> usize {
        self.pos
    }

    fn eof(&self) -> LuceneError {
        LuceneError::corrupt_index(
            "read past the end of the shape doc value".to_string(),
            "shape doc values",
        )
    }

    /// Reads the four variable longs of a bounding box, translating each back
    /// from the positive space it was written in.
    ///
    /// Equivalent to `Reader.readBBox()`.
    fn read_bbox(&mut self) -> Result<EncodedBounds> {
        let min_x = self.read_translated()?;
        let max_x = self.read_translated()?;
        let min_y = self.read_translated()?;
        let max_y = self.read_translated()?;
        Ok(EncodedBounds {
            min_x,
            max_x,
            min_y,
            max_y,
        })
    }

    /// Reads one variable long and translates it back by `Integer.MIN_VALUE`.
    fn read_translated(&mut self) -> Result<i32> {
        let raw = self.read_v_long()?;
        to_int_exact(raw + i64::from(i32::MIN))
    }

    /// Reads one variable long and subtracts it from `base`.
    ///
    /// This is the `Math.toIntExact(base - readVLong())` idiom the traversal
    /// applies to every delta-encoded coordinate.
    fn read_delta_from(&mut self, base: i32) -> Result<i32> {
        let raw = self.read_v_long()?;
        to_int_exact(i64::from(base) - raw)
    }
}

impl DataInput for SliceDataInput<'_> {
    fn read_byte(&mut self) -> Result<u8> {
        let b = *self.bytes.get(self.pos).ok_or_else(|| self.eof())?;
        self.pos += 1;
        Ok(b)
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        if self.pos + len > self.bytes.len() {
            return Err(self.eof());
        }
        b[offset..offset + len].copy_from_slice(&self.bytes[self.pos..self.pos + len]);
        self.pos += len;
        Ok(())
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        let num_bytes = usize::try_from(num_bytes).map_err(|_| {
            LuceneError::corrupt_index(
                format!("negative skip of {num_bytes} bytes"),
                "shape doc values",
            )
        })?;
        if self.pos + num_bytes > self.bytes.len() {
            return Err(self.eof());
        }
        self.pos += num_bytes;
        Ok(())
    }
}

/// Equivalent to `Math.toIntExact(long)`, which throws `ArithmeticException`
/// when the value does not fit.
fn to_int_exact(value: i64) -> Result<i32> {
    i32::try_from(value).map_err(|_| {
        LuceneError::corrupt_index(
            format!("shape doc value holds {value}, which is not an int"),
            "shape doc values",
        )
    })
}

/// The four header bits a serialised node carries.
///
/// Equivalent to the static helpers of `ShapeDocValues.Reader.Header`.
struct Header;

impl Header {
    /// Equivalent to `Header.readType(int)`.
    fn read_type(bits: i32) -> TriangleType {
        if bits & 0x04 == 0x04 {
            TriangleType::Point
        } else if bits & 0x08 == 0x08 {
            TriangleType::Line
        } else {
            TriangleType::Triangle
        }
    }

    /// Equivalent to `Header.readHasLeftSubtree(int)`.
    fn read_has_left_subtree(bits: i32) -> bool {
        bits & 0x02 == 0x02
    }

    /// Equivalent to `Header.readHasRightSubtree(int)`.
    fn read_has_right_subtree(bits: i32) -> bool {
        bits & 0x01 == 0x01
    }
}

/// A bounding box in the encoded integer space.
///
/// Equivalent to the fields `SpatialQuery.EncodedRectangle` exposes to the
/// shape doc-values reader, which uses one as a scratch box while walking the
/// tree. The full spatial logic of that class lives in
/// [`EncodedRectangle`](crate::document::EncodedRectangle); the reader needs
/// only the four bounds, so this port keeps them in a plain value type instead
/// of reusing a mutable scratch instance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EncodedBounds {
    /// Smallest x.
    pub min_x: i32,
    /// Largest x.
    pub max_x: i32,
    /// Smallest y.
    pub min_y: i32,
    /// Largest y.
    pub max_y: i32,
}

// -----------------------------------------------------------------------------
// Tree construction
// -----------------------------------------------------------------------------

/// One node of the in-memory tessellation tree.
///
/// Equivalent to the private inner class `ShapeDocValues.TreeNode`.
///
/// **Divergence from Lucene 10.5.0.** Java links nodes with object references.
/// This port keeps them in an arena and links them with indices, which is the
/// standard Rust rendering of a mutable graph and changes nothing observable:
/// the traversal order, the pulled-up bounds and the computed byte sizes are
/// identical.
#[derive(Clone, Debug)]
struct TreeNode {
    triangle: DecodedTriangle,
    mid_x: f64,
    mid_y: f64,
    /// Units are encoded space; used **only** to compute the centroid in
    /// encoded space. Triangles are guaranteed counter-clockwise, so this is
    /// always positive unless the component is a point or a line.
    signed_area: f64,
    /// Units are encoded space; used **only** to compute the centroid in
    /// encoded space. Always positive unless the component is a point or a
    /// triangle.
    length: f64,
    highest_type: TriangleType,
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
    left: Option<usize>,
    right: Option<usize>,
    parent: Option<usize>,
    /// Header size is one byte; the remainder accumulates during construction.
    byte_size: i32,
}

impl TreeNode {
    /// Equivalent to `TreeNode(ShapeField.DecodedTriangle)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for a triangle whose type was
    /// never resolved, which Java reports as `invalid type [...] found`.
    fn new(t: DecodedTriangle, encoder: ShapeCoordinateSystem) -> Result<Self> {
        let min_x = t.a_x.min(t.b_x).min(t.c_x);
        let min_y = t.a_y.min(t.b_y).min(t.c_y);
        let max_x = t.a_x.max(t.b_x).max(t.c_x);
        let max_y = t.a_y.max(t.b_y).max(t.c_y);

        let ax = encoder.decode_x(t.a_x);
        let ay = encoder.decode_y(t.a_y);
        let (mid_x, mid_y, signed_area, length) = match t.triangle_type {
            TriangleType::Point => (ax, ay, 0.0, 0.0),
            TriangleType::Line => {
                let bx = encoder.decode_x(t.b_x);
                let by = encoder.decode_y(t.b_y);
                let length = (ax - bx).hypot(ay - by);
                // Weighted by length.
                (
                    0.5 * (ax + bx) * length,
                    0.5 * (ay + by) * length,
                    0.0,
                    length,
                )
            }
            TriangleType::Triangle => {
                let bx = encoder.decode_x(t.b_x);
                let by = encoder.decode_y(t.b_y);
                let cx = encoder.decode_x(t.c_x);
                let cy = encoder.decode_y(t.c_y);
                let signed_area = (0.5 * ((bx - ax) * (cy - ay) - (cx - ax) * (by - ay))).abs();
                // Weighted by signed area.
                (
                    ((ax + bx + cx) / 3.0) * signed_area,
                    ((ay + by + cy) / 3.0) * signed_area,
                    signed_area,
                    0.0,
                )
            }
        };

        Ok(Self {
            triangle: t,
            mid_x,
            mid_y,
            signed_area,
            length,
            highest_type: t.triangle_type,
            min_x,
            max_x,
            min_y,
            max_y,
            left: None,
            right: None,
            parent: None,
            byte_size: 1,
        })
    }
}

/// Orders tree nodes on one axis while [`intro_select`] partitions them.
///
/// Equivalent to the `Comparator<TreeNode>` `ShapeDocValues.createTree` hands
/// to `ArrayUtil.select`: compare the minimum first, then the maximum, on the
/// axis the current tree level splits.
struct NodeSelector<'a> {
    order: &'a mut [usize],
    nodes: &'a [TreeNode],
    split_x: bool,
    pivot: usize,
}

impl NodeSelector<'_> {
    fn key(&self, index: usize) -> (i32, i32) {
        let node = &self.nodes[index];
        if self.split_x {
            (node.min_x, node.max_x)
        } else {
            (node.min_y, node.max_y)
        }
    }
}

impl PivotOps for NodeSelector<'_> {
    fn swap(&mut self, i: usize, j: usize) {
        self.order.swap(i, j);
    }

    fn set_pivot(&mut self, i: usize) {
        self.pivot = self.order[i];
    }

    fn compare_pivot(&mut self, j: usize) -> i32 {
        let left = self.key(self.pivot);
        let right = self.key(self.order[j]);
        match left.cmp(&right) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }
}

/// Builds the tessellation tree and returns the nodes in depth-first order.
///
/// Equivalent to `ShapeDocValues.buildTree(List<DecodedTriangle>, List<TreeNode>)`,
/// which returns the root and fills the depth-first list; here the list is
/// returned and its first entry is the root, exactly as the writer assumes.
fn build_tree(
    tessellation: &[DecodedTriangle],
    encoder: ShapeCoordinateSystem,
) -> Result<(Vec<TreeNode>, Vec<usize>)> {
    if tessellation.is_empty() {
        return Err(LuceneError::IllegalArgument(
            "the tessellation must not be empty".to_string(),
        ));
    }

    let mut nodes = Vec::with_capacity(tessellation.len());
    let mut dfs = Vec::with_capacity(tessellation.len());

    if tessellation.len() == 1 {
        let t = tessellation[0];
        let mut node = TreeNode::new(t, encoder)?;
        match t.triangle_type {
            TriangleType::Line if node.length != 0.0 => {
                node.mid_x /= node.length;
                node.mid_y /= node.length;
            }
            TriangleType::Triangle if node.signed_area != 0.0 => {
                node.mid_x /= node.signed_area;
                node.mid_y /= node.signed_area;
            }
            _ => {}
        }
        node.highest_type = t.triangle_type;
        nodes.push(node);
        dfs.push(0);
        return Ok((nodes, dfs));
    }

    let mut min_y = i32::MAX;
    let mut min_x = i32::MAX;
    let mut max_y = i32::MIN;
    let mut max_x = i32::MIN;

    // Running statistics for the centroid.
    let mut total_signed_area = 0.0f64;
    let mut total_length = 0.0f64;
    let (mut num_x_pnt, mut num_y_pnt) = (0.0f64, 0.0f64);
    let (mut num_x_lin, mut num_y_lin) = (0.0f64, 0.0f64);
    let (mut num_x_ply, mut num_y_ply) = (0.0f64, 0.0f64);
    let mut highest_type = TriangleType::Point;

    for t in tessellation {
        let node = TreeNode::new(*t, encoder)?;
        min_y = min_y.min(node.min_y);
        min_x = min_x.min(node.min_x);
        max_y = max_y.max(node.max_y);
        max_x = max_x.max(node.max_x);

        // Non-zero if any component is a triangle.
        total_signed_area += node.signed_area;
        // Non-zero if any component is a line segment.
        total_length += node.length;
        match t.triangle_type {
            TriangleType::Point => {
                num_x_pnt += node.mid_x;
                num_y_pnt += node.mid_y;
            }
            TriangleType::Line => {
                if highest_type == TriangleType::Point {
                    highest_type = TriangleType::Line;
                }
                num_x_lin += node.mid_x;
                num_y_lin += node.mid_y;
            }
            TriangleType::Triangle => {
                if highest_type != TriangleType::Triangle {
                    highest_type = TriangleType::Triangle;
                }
                num_x_ply += node.mid_x;
                num_y_ply += node.mid_y;
            }
        }
        nodes.push(node);
    }

    let count = nodes.len();
    let mut order: Vec<usize> = (0..count).collect();
    let root = create_tree(
        &mut nodes,
        &mut order,
        0,
        count as isize - 1,
        false,
        None,
        &mut dfs,
    )
    .expect("INVARIANT: a non-empty tessellation always yields a root");

    // Pull up the minimum values for the root node so the bounding box is
    // consistent.
    nodes[root].min_y = min_y;
    nodes[root].min_x = min_x;
    // Set the highest dimensional type.
    nodes[root].highest_type = highest_type;

    // Compute the centroid values for the root node so the centroid is
    // consistent.
    match highest_type {
        TriangleType::Point => {
            nodes[root].mid_x = num_x_pnt / count as f64;
            nodes[root].mid_y = num_y_pnt / count as f64;
        }
        TriangleType::Line => {
            // The numerator is the sum of the segment midpoints times the
            // segment length; divide by the total length.
            nodes[root].mid_x = num_x_lin;
            nodes[root].mid_y = num_y_lin;
            if total_length != 0.0 {
                nodes[root].mid_x /= total_length;
                nodes[root].mid_y /= total_length;
            }
        }
        TriangleType::Triangle => {
            // The numerator is the sum of the triangle centroids times the
            // triangle signed area; divide by the total signed area.
            nodes[root].mid_x = num_x_ply;
            nodes[root].mid_y = num_y_ply;
            if total_signed_area != 0.0 {
                nodes[root].mid_x /= total_signed_area;
                nodes[root].mid_y /= total_signed_area;
            }
        }
    }

    Ok((nodes, dfs))
}

/// Equivalent to `ShapeDocValues.createTree(...)`.
fn create_tree(
    nodes: &mut Vec<TreeNode>,
    order: &mut Vec<usize>,
    low: isize,
    high: isize,
    split_x: bool,
    parent: Option<usize>,
    dfs: &mut Vec<usize>,
) -> Option<usize> {
    if low > high {
        return None;
    }
    // Add the midpoint.
    let mid = ((low as usize) + (high as usize)) >> 1;
    if low < high {
        let mut selector = NodeSelector {
            order,
            nodes,
            split_x,
            pivot: 0,
        };
        intro_select(&mut selector, low as usize, high as usize + 1, mid);
    }
    let new_node = order[mid];
    dfs.push(new_node);
    nodes[new_node].parent = parent;

    // Add the children.
    let left = create_tree(
        nodes,
        order,
        low,
        mid as isize - 1,
        !split_x,
        Some(new_node),
        dfs,
    );
    let right = create_tree(
        nodes,
        order,
        mid as isize + 1,
        high,
        !split_x,
        Some(new_node),
        dfs,
    );
    nodes[new_node].left = left;
    nodes[new_node].right = right;

    // Pull the child bounds up to this node.
    if let Some(left) = left {
        let (l_min_x, l_min_y, l_max_x, l_max_y) = {
            let n = &nodes[left];
            (n.min_x, n.min_y, n.max_x, n.max_y)
        };
        let n = &mut nodes[new_node];
        n.min_x = n.min_x.min(l_min_x);
        n.min_y = n.min_y.min(l_min_y);
        n.max_x = n.max_x.max(l_max_x);
        n.max_y = n.max_y.max(l_max_y);
    }
    if let Some(right) = right {
        let (r_min_x, r_min_y, r_max_x, r_max_y) = {
            let n = &nodes[right];
            (n.min_x, n.min_y, n.max_x, n.max_y)
        };
        let n = &mut nodes[new_node];
        n.min_x = n.min_x.min(r_min_x);
        n.min_y = n.min_y.min(r_min_y);
        n.max_x = n.max_x.max(r_max_x);
        n.max_y = n.max_y.max(r_max_y);
    }

    // Adjust the byte sizes based on the new parent bounding box.
    let (p_max_x, p_max_y) = (nodes[new_node].max_x, nodes[new_node].max_y);
    for child in [left, right].into_iter().flatten() {
        let extra = {
            let c = &nodes[child];
            // Bounding box size.
            v_long_size(i64::from(p_max_x) - i64::from(c.min_x))
                + v_long_size(i64::from(p_max_y) - i64::from(c.min_y))
                + v_long_size(i64::from(p_max_x) - i64::from(c.max_x))
                + v_long_size(i64::from(p_max_y) - i64::from(c.max_y))
                // Component size.
                + compute_component_size(c, p_max_x, p_max_y)
        };
        nodes[child].byte_size += extra;
        let child_size = nodes[child].byte_size;
        // Include the byte size, so the subtree can be skipped whole.
        nodes[new_node].byte_size += v_int_size(child_size) + child_size;
    }

    Some(new_node)
}

/// Equivalent to `ShapeDocValues.computeComponentSize(TreeNode, int, int)`.
fn compute_component_size(node: &TreeNode, max_x: i32, max_y: i32) -> i32 {
    let t = &node.triangle;
    let mut size = v_long_size(i64::from(max_x) - i64::from(t.a_x))
        + v_long_size(i64::from(max_y) - i64::from(t.a_y));
    if matches!(t.triangle_type, TriangleType::Line | TriangleType::Triangle) {
        size += v_long_size(i64::from(max_x) - i64::from(t.b_x))
            + v_long_size(i64::from(max_y) - i64::from(t.b_y));
    }
    if t.triangle_type == TriangleType::Triangle {
        size += v_long_size(i64::from(max_x) - i64::from(t.c_x))
            + v_long_size(i64::from(max_y) - i64::from(t.c_y));
    }
    size
}

// -----------------------------------------------------------------------------
// Writer
// -----------------------------------------------------------------------------

/// Serialises the depth-first node list.
///
/// Equivalent to the private inner class `ShapeDocValues.Writer`.
fn write_tree(
    nodes: &[TreeNode],
    dfs: &[usize],
    encoder: ShapeCoordinateSystem,
) -> Result<BytesRef> {
    let mut output = crate::store::ByteBuffersDataOutput::new();
    // Write the encoding version.
    output.write_byte(VERSION)?;
    // Write the number of terms (triangles).
    output.write_v_int(dfs.len() as i32)?;
    // Write the root.
    let root = dfs[0];
    let r = &nodes[root];
    // Write the bounding box, converted to a variable long by translating it
    // into positive space.
    output.write_v_long(i64::from(r.min_x) - i64::from(i32::MIN))?;
    output.write_v_long(i64::from(r.max_x) - i64::from(i32::MIN))?;
    output.write_v_long(i64::from(r.min_y) - i64::from(i32::MIN))?;
    output.write_v_long(i64::from(r.max_y) - i64::from(i32::MIN))?;
    // Write the centroid.
    output.write_v_long(i64::from(encoder.encode_x(r.mid_x)?) - i64::from(i32::MIN))?;
    output.write_v_long(i64::from(encoder.encode_y(r.mid_y)?) - i64::from(i32::MIN))?;
    // Write the highest dimensional type.
    output.write_v_int(r.highest_type.ordinal())?;
    // Write the header.
    write_header(&mut output, r)?;
    // Write the component.
    write_component(&mut output, r, r.max_x, r.max_y)?;

    for &index in &dfs[1..] {
        write_node(&mut output, nodes, index)?;
    }

    Ok(BytesRef::new(output.to_array_copy()))
}

/// Equivalent to `Writer.writeNode(TreeNode)`.
fn write_node(
    output: &mut crate::store::ByteBuffersDataOutput,
    nodes: &[TreeNode],
    index: usize,
) -> Result<()> {
    let node = &nodes[index];
    let parent = &nodes[node
        .parent
        .expect("INVARIANT: only the root has no parent, and it is written separately")];
    // Write the total subtree size.
    output.write_v_int(node.byte_size)?;
    // Write the bounds.
    output.write_v_long(i64::from(parent.max_x) - i64::from(node.min_x))?;
    output.write_v_long(i64::from(parent.max_y) - i64::from(node.min_y))?;
    output.write_v_long(i64::from(parent.max_x) - i64::from(node.max_x))?;
    output.write_v_long(i64::from(parent.max_y) - i64::from(node.max_y))?;
    write_header(output, node)?;
    write_component(output, node, parent.max_x, parent.max_y)
}

/// Equivalent to `Writer.writeComponent(TreeNode, int, int)`.
fn write_component(
    output: &mut crate::store::ByteBuffersDataOutput,
    node: &TreeNode,
    p_max_x: i32,
    p_max_y: i32,
) -> Result<()> {
    let t = &node.triangle;
    output.write_v_long(i64::from(p_max_x) - i64::from(t.a_x))?;
    output.write_v_long(i64::from(p_max_y) - i64::from(t.a_y))?;
    if matches!(t.triangle_type, TriangleType::Line | TriangleType::Triangle) {
        output.write_v_long(i64::from(p_max_x) - i64::from(t.b_x))?;
        output.write_v_long(i64::from(p_max_y) - i64::from(t.b_y))?;
    }
    if t.triangle_type == TriangleType::Triangle {
        output.write_v_long(i64::from(p_max_x) - i64::from(t.c_x))?;
        output.write_v_long(i64::from(p_max_y) - i64::from(t.c_y))?;
    }
    Ok(())
}

/// Equivalent to `Writer.writeHeader(TreeNode)`.
fn write_header(output: &mut crate::store::ByteBuffersDataOutput, node: &TreeNode) -> Result<()> {
    let mut header = 0x00;
    // Left and right subtrees.
    if node.right.is_some() {
        header |= 0x01;
    }
    if node.left.is_some() {
        header |= 0x02;
    }
    // Type.
    match node.triangle.triangle_type {
        TriangleType::Point => header |= 0x04,
        TriangleType::Line => header |= 0x08,
        TriangleType::Triangle => {}
    }
    // Which edges belong to the original shape.
    if node.triangle.ab {
        header |= 0x10;
    }
    if node.triangle.bc {
        header |= 0x20;
    }
    if node.triangle.ca {
        header |= 0x40;
    }
    output.write_v_int(header)
}

// -----------------------------------------------------------------------------
// ShapeDocValues
// -----------------------------------------------------------------------------

/// A shape stored as one binary doc value.
///
/// Equivalent to the abstract class `org.apache.lucene.document.ShapeDocValues`,
/// whose two concrete subclasses are
/// [`LatLonShapeDocValues`] and [`XYShapeDocValues`]. It cannot be built
/// without naming a coordinate system, because the centroid, the bounding box
/// and every relation are computed in decoded space.
///
/// Multi-geometries are *not* supported, exactly as in Java: an accurate
/// centroid needs the area of each original geometry, and a binary doc value
/// holds a single value per document.
///
/// **Divergence from Lucene 10.5.0.** Java splits the class in two: an
/// abstract base plus a per-coordinate-system subclass that supplies the
/// `Encoder` and materialises the centroid and bounding box as `Geometry`
/// objects. Rust has no implementation inheritance, so this type carries the
/// coordinate system as a value ([`ShapeCoordinateSystem`]) and the two
/// subclasses become thin wrappers that add the typed accessors. The bytes and
/// the relations are unchanged.
#[derive(Clone, Debug)]
pub struct ShapeDocValues {
    data: BytesRef,
    encoder: ShapeCoordinateSystem,
    number_of_terms: i32,
    bounding_box: EncodedBounds,
    centroid_x: i32,
    centroid_y: i32,
    highest_dimension: TriangleType,
}

impl ShapeDocValues {
    /// Creates the value from a shape tessellation.
    ///
    /// Equivalent to `ShapeDocValues(List<ShapeField.DecodedTriangle>)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for an empty tessellation or a
    /// centroid that falls outside the coordinate system, and
    /// [`LuceneError::CorruptIndex`] when the bytes just written cannot be read
    /// back — which Java reports as
    /// `"unable to read binary shape doc value field"`.
    pub fn from_tessellation(
        tessellation: &[DecodedTriangle],
        encoder: ShapeCoordinateSystem,
    ) -> Result<Self> {
        let (nodes, dfs) = build_tree(tessellation, encoder)?;
        let data = write_tree(&nodes, &dfs, encoder)?;
        Self::from_binary_value(data, encoder)
    }

    /// Creates the value from an already serialised representation.
    ///
    /// Equivalent to `ShapeDocValues(BytesRef)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::CorruptIndex`] when the bytes are not a shape doc
    /// value, which Java reports as
    /// `"unable to read binary shape doc value field"`.
    pub fn from_binary_value(data: BytesRef, encoder: ShapeCoordinateSystem) -> Result<Self> {
        // Equivalent to the `ShapeComparator(BytesRef)` constructor, which
        // reads the fixed header and then rewinds.
        let mut reader = SliceDataInput::new(data.slice());
        let version = reader.read_byte()?;
        if version != VERSION {
            return Err(LuceneError::corrupt_index(
                format!("unexpected shape doc value version {version}, expected {VERSION}"),
                "shape doc values",
            ));
        }
        let number_of_terms = reader.read_v_int()?;
        let bounding_box = reader.read_bbox()?;
        let centroid_x = reader.read_translated()?;
        let centroid_y = reader.read_translated()?;
        let highest_dimension = TriangleType::from_ordinal(reader.read_v_int()?)?;
        Ok(Self {
            data,
            encoder,
            number_of_terms,
            bounding_box,
            centroid_x,
            centroid_y,
            highest_dimension,
        })
    }

    /// Returns the encoded doc-values field.
    ///
    /// Equivalent to `ShapeDocValues.binaryValue()`.
    pub fn binary_value(&self) -> &BytesRef {
        &self.data
    }

    /// Returns the coordinate system the shape was encoded in.
    pub fn coordinate_system(&self) -> ShapeCoordinateSystem {
        self.encoder
    }

    /// Returns the number of terms (tessellated triangles) of this shape.
    ///
    /// Equivalent to `ShapeDocValues.numberOfTerms()`.
    pub fn number_of_terms(&self) -> i32 {
        self.number_of_terms
    }

    /// Returns the minimum x of the shape's bounding box.
    ///
    /// Equivalent to `ShapeDocValues.getEncodedMinX()`.
    pub fn get_encoded_min_x(&self) -> i32 {
        self.bounding_box.min_x
    }

    /// Returns the minimum y of the shape's bounding box.
    ///
    /// Equivalent to `ShapeDocValues.getEncodedMinY()`.
    pub fn get_encoded_min_y(&self) -> i32 {
        self.bounding_box.min_y
    }

    /// Returns the maximum x of the shape's bounding box.
    ///
    /// Equivalent to `ShapeDocValues.getEncodedMaxX()`.
    pub fn get_encoded_max_x(&self) -> i32 {
        self.bounding_box.max_x
    }

    /// Returns the maximum y of the shape's bounding box.
    ///
    /// Equivalent to `ShapeDocValues.getEncodedMaxY()`.
    pub fn get_encoded_max_y(&self) -> i32 {
        self.bounding_box.max_y
    }

    /// Returns the encoded x centroid of the geometry.
    ///
    /// Equivalent to `ShapeDocValues.getEncodedCentroidX()`.
    pub fn get_encoded_centroid_x(&self) -> i32 {
        self.centroid_x
    }

    /// Returns the encoded y centroid of the geometry.
    ///
    /// Equivalent to `ShapeDocValues.getEncodedCentroidY()`.
    pub fn get_encoded_centroid_y(&self) -> i32 {
        self.centroid_y
    }

    /// Returns the highest dimensional type of the geometry, which is what the
    /// centroid computation is based on.
    ///
    /// Equivalent to `ShapeDocValues.getHighestDimension()`.
    pub fn get_highest_dimension(&self) -> TriangleType {
        self.highest_dimension
    }

    /// Computes the relation of `component` with this shape, walking the
    /// serialised tessellation tree.
    ///
    /// Equivalent to `ShapeDocValues.relate(Component2D)`, which delegates to
    /// `ShapeComparator.relate(Component2D)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::CorruptIndex`] when the serialised tree is
    /// malformed.
    pub fn relate(&self, component: &dyn Component2D) -> Result<Relation> {
        let mut reader = SliceDataInput::new(self.data.slice());
        let result = self.relate_root(&mut reader, component);
        reader.rewind();
        result
    }

    /// The body of `ShapeComparator.relate(Component2D)`: the entry point at the
    /// root of the binary tree.
    fn relate_root(
        &self,
        reader: &mut SliceDataInput<'_>,
        query: &dyn Component2D,
    ) -> Result<Relation> {
        // Skip the version.
        reader.read_byte()?;
        // Skip the number of terms.
        reader.read_v_int()?;
        // Read the bounding box.
        let bbox = reader.read_bbox()?;
        let t_min_x = bbox.min_x;
        let t_max_x = bbox.max_x;
        let t_max_y = bbox.max_y;

        // Relate the query to the shape's bounding box.
        let r = query.relate(
            self.encoder.decode_x(bbox.min_x),
            self.encoder.decode_x(bbox.max_x),
            self.encoder.decode_y(bbox.min_y),
            self.encoder.decode_y(bbox.max_y),
        );
        if r != Relation::CellCrossesQuery {
            return Ok(r);
        }

        // Traverse the tessellation tree; skip the centroid and the highest
        // dimension.
        reader.read_v_long()?;
        reader.read_v_long()?;
        reader.read_v_int()?;
        // Read the header.
        let header_bits = reader.read_v_int()?;
        let x = reader.read_delta_from(t_max_x)?;
        // Relate the component.
        if self.relate_component(
            reader,
            Header::read_type(header_bits),
            t_max_x,
            t_max_y,
            self.encoder.decode_x(x),
            query,
        )? == Relation::CellCrossesQuery
        {
            return Ok(Relation::CellCrossesQuery);
        }
        let mut r = Relation::CellOutsideQuery;

        // Recurse into the left subtree.
        if Header::read_has_left_subtree(header_bits) {
            let size = reader.read_v_int()?;
            r = self.relate_node(reader, query, false, t_max_x, t_max_y, size)?;
            if r == Relation::CellCrossesQuery {
                return Ok(Relation::CellCrossesQuery);
            }
        }
        // Recurse into the right subtree.
        if Header::read_has_right_subtree(header_bits)
            && query.get_max_x() >= self.encoder.decode_x(t_min_x)
        {
            let size = reader.read_v_int()?;
            r = self.relate_node(reader, query, false, t_max_x, t_max_y, size)?;
            if r == Relation::CellCrossesQuery {
                return Ok(Relation::CellCrossesQuery);
            }
        }
        Ok(r)
    }

    /// Equivalent to the recursive
    /// `ShapeComparator.relate(Component2D, boolean, int, int, int)`.
    fn relate_node(
        &self,
        reader: &mut SliceDataInput<'_>,
        query: &dyn Component2D,
        split_x: bool,
        p_max_x: i32,
        p_max_y: i32,
        mut node_size: i32,
    ) -> Result<Relation> {
        // Mark the position, because the bounds and the header must be
        // subtracted from the node's byte size.
        let pre_pos = reader.position();
        let t_min_x = reader.read_delta_from(p_max_x)?;
        let t_min_y = reader.read_delta_from(p_max_y)?;
        let t_max_x = reader.read_delta_from(p_max_x)?;
        let t_max_y = reader.read_delta_from(p_max_y)?;
        let header_bits = reader.read_v_int()?;
        node_size -= (reader.position() - pre_pos) as i32;

        // Base case: the query is disjoint from this subtree.
        if query.get_min_x() > self.encoder.decode_x(t_max_x)
            || query.get_min_y() > self.encoder.decode_y(t_max_y)
        {
            reader.skip_bytes(i64::from(node_size))?;
            return Ok(Relation::CellOutsideQuery);
        }

        let x = reader.read_delta_from(p_max_x)?;
        if self.relate_component(
            reader,
            Header::read_type(header_bits),
            p_max_x,
            p_max_y,
            self.encoder.decode_x(x),
            query,
        )? == Relation::CellCrossesQuery
        {
            return Ok(Relation::CellCrossesQuery);
        }

        // Traverse the left subtree.
        if Header::read_has_left_subtree(header_bits) {
            let size = reader.read_v_int()?;
            if self.relate_node(reader, query, !split_x, t_max_x, t_max_y, size)?
                == Relation::CellCrossesQuery
            {
                return Ok(Relation::CellCrossesQuery);
            }
        }

        // Traverse the right subtree.
        if Header::read_has_right_subtree(header_bits) {
            let size = reader.read_v_int()?;
            if (!split_x && query.get_max_y() >= self.encoder.decode_y(t_min_y))
                || (split_x && query.get_max_x() >= self.encoder.decode_x(t_min_x))
            {
                if self.relate_node(reader, query, !split_x, t_max_x, t_max_y, size)?
                    == Relation::CellCrossesQuery
                {
                    return Ok(Relation::CellCrossesQuery);
                }
            } else {
                // Skip the subtree when its bounding box cannot match.
                reader.skip_bytes(i64::from(size))?;
            }
        }
        Ok(Relation::CellOutsideQuery)
    }

    /// Equivalent to
    /// `ShapeComparator.relateComponent(TYPE, EncodedRectangle, int, int, double, Component2D)`.
    ///
    /// Java threads a scratch `EncodedRectangle` through this call and its three
    /// callees; none of them reads it, so this port drops the parameter.
    fn relate_component(
        &self,
        reader: &mut SliceDataInput<'_>,
        triangle_type: TriangleType,
        p_max_x: i32,
        p_max_y: i32,
        ax: f64,
        query: &dyn Component2D,
    ) -> Result<Relation> {
        let crosses = match triangle_type {
            TriangleType::Point => {
                // `ShapeComparator.relatePoint`.
                let y = reader.read_delta_from(p_max_y)?;
                query.contains(ax, self.encoder.decode_y(y))
            }
            TriangleType::Line => {
                // `ShapeComparator.relateLine`.
                let ay = reader.read_delta_from(p_max_y)?;
                let bx = self.encoder.decode_x(reader.read_delta_from(p_max_x)?);
                let by = reader.read_delta_from(p_max_y)?;
                query.intersects_line_bbox(
                    ax,
                    self.encoder.decode_y(ay),
                    bx,
                    self.encoder.decode_y(by),
                )
            }
            TriangleType::Triangle => {
                // `ShapeComparator.relateTriangle`.
                let ay = reader.read_delta_from(p_max_y)?;
                let bx = self.encoder.decode_x(reader.read_delta_from(p_max_x)?);
                let by = reader.read_delta_from(p_max_y)?;
                let cx = self.encoder.decode_x(reader.read_delta_from(p_max_x)?);
                let cy = reader.read_delta_from(p_max_y)?;
                query.intersects_triangle_bbox(
                    ax,
                    self.encoder.decode_y(ay),
                    bx,
                    self.encoder.decode_y(by),
                    cx,
                    self.encoder.decode_y(cy),
                )
            }
        };
        Ok(if crosses {
            Relation::CellCrossesQuery
        } else {
            Relation::CellOutsideQuery
        })
    }
}

/// The geographic [`ShapeDocValues`].
///
/// Equivalent to `org.apache.lucene.document.LatLonShapeDocValues`. Instances
/// come from the factory methods of [`LatLonShape`](crate::document::LatLonShape).
#[derive(Clone, Debug)]
pub struct LatLonShapeDocValues {
    inner: ShapeDocValues,
}

impl LatLonShapeDocValues {
    /// Creates the value from a tessellation.
    ///
    /// Equivalent to `LatLonShapeDocValues(List<ShapeField.DecodedTriangle>)`.
    ///
    /// # Errors
    ///
    /// As [`ShapeDocValues::from_tessellation`].
    pub fn from_tessellation(tessellation: &[DecodedTriangle]) -> Result<Self> {
        Ok(Self {
            inner: ShapeDocValues::from_tessellation(
                tessellation,
                ShapeCoordinateSystem::Geographic,
            )?,
        })
    }

    /// Creates the value from an already serialised representation.
    ///
    /// Equivalent to `LatLonShapeDocValues(BytesRef)`.
    ///
    /// # Errors
    ///
    /// As [`ShapeDocValues::from_binary_value`].
    pub fn from_binary_value(binary_value: BytesRef) -> Result<Self> {
        Ok(Self {
            inner: ShapeDocValues::from_binary_value(
                binary_value,
                ShapeCoordinateSystem::Geographic,
            )?,
        })
    }

    /// Returns the untyped value.
    pub fn shape_doc_values(&self) -> &ShapeDocValues {
        &self.inner
    }

    /// Returns the shape's centroid.
    ///
    /// Equivalent to `LatLonShapeDocValues.getCentroid()`, which returns the
    /// `Point` `computeCentroid()` built.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the decoded centroid is
    /// not a valid latitude/longitude pair, which `new Point(...)` rejects.
    pub fn get_centroid(&self) -> Result<Point> {
        Point::new(
            self.inner
                .encoder
                .decode_y(self.inner.get_encoded_centroid_y()),
            self.inner
                .encoder
                .decode_x(self.inner.get_encoded_centroid_x()),
        )
    }

    /// Returns the shape's bounding box.
    ///
    /// Equivalent to `LatLonShapeDocValues.getBoundingBox()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the decoded bounds are not
    /// a valid rectangle, which `new Rectangle(...)` rejects.
    pub fn get_bounding_box(&self) -> Result<Rectangle> {
        let e = self.inner.encoder;
        Rectangle::new(
            e.decode_y(self.inner.get_encoded_min_y()),
            e.decode_y(self.inner.get_encoded_max_y()),
            e.decode_x(self.inner.get_encoded_min_x()),
            e.decode_x(self.inner.get_encoded_max_x()),
        )
    }
}

/// The cartesian [`ShapeDocValues`].
///
/// Equivalent to `org.apache.lucene.document.XYShapeDocValues`. Instances come
/// from the factory methods of [`XYShape`](crate::document::XYShape).
#[derive(Clone, Debug)]
pub struct XYShapeDocValues {
    inner: ShapeDocValues,
}

impl XYShapeDocValues {
    /// Creates the value from a tessellation.
    ///
    /// Equivalent to `XYShapeDocValues(List<ShapeField.DecodedTriangle>)`.
    ///
    /// # Errors
    ///
    /// As [`ShapeDocValues::from_tessellation`].
    pub fn from_tessellation(tessellation: &[DecodedTriangle]) -> Result<Self> {
        Ok(Self {
            inner: ShapeDocValues::from_tessellation(
                tessellation,
                ShapeCoordinateSystem::Cartesian,
            )?,
        })
    }

    /// Creates the value from an already serialised representation.
    ///
    /// Equivalent to `XYShapeDocValues(BytesRef)`.
    ///
    /// # Errors
    ///
    /// As [`ShapeDocValues::from_binary_value`].
    pub fn from_binary_value(binary_value: BytesRef) -> Result<Self> {
        Ok(Self {
            inner: ShapeDocValues::from_binary_value(
                binary_value,
                ShapeCoordinateSystem::Cartesian,
            )?,
        })
    }

    /// Returns the untyped value.
    pub fn shape_doc_values(&self) -> &ShapeDocValues {
        &self.inner
    }

    /// Returns the shape's centroid.
    ///
    /// Equivalent to `XYShapeDocValues.getCentroid()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the decoded centroid is
    /// not finite, which `new XYPoint(...)` rejects.
    pub fn get_centroid(&self) -> Result<XYPoint> {
        let e = self.inner.encoder;
        XYPoint::new(
            e.decode_x(self.inner.get_encoded_centroid_x()) as f32,
            e.decode_y(self.inner.get_encoded_centroid_y()) as f32,
        )
    }

    /// Returns the shape's bounding box.
    ///
    /// Equivalent to `XYShapeDocValues.getBoundingBox()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the decoded bounds are not
    /// a valid rectangle, which `new XYRectangle(...)` rejects.
    pub fn get_bounding_box(&self) -> Result<XYRectangle> {
        let e = self.inner.encoder;
        XYRectangle::new(
            e.decode_x(self.inner.get_encoded_min_x()) as f32,
            e.decode_x(self.inner.get_encoded_max_x()) as f32,
            e.decode_y(self.inner.get_encoded_min_y()) as f32,
            e.decode_y(self.inner.get_encoded_max_y()) as f32,
        )
    }
}

// -----------------------------------------------------------------------------
// ShapeDocValuesField
// -----------------------------------------------------------------------------

/// Builds the field type a shape doc-values field uses: binary doc values with
/// norms omitted.
///
/// Equivalent to the static `ShapeDocValuesField.FIELD_TYPE` initialiser.
///
/// # Errors
///
/// Propagates the error [`FieldType`] raises when a property cannot be set.
pub fn shape_doc_values_field_type() -> Result<FieldType> {
    let mut ft = FieldType::new();
    ft.set_doc_values_type(DocValuesType::BINARY)?;
    ft.set_omit_norms(true)?;
    ft.freeze();
    Ok(ft)
}

/// A shape as an indexable doc-values field.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.document.ShapeDocValuesField`, whose two concrete
/// subclasses are [`LatLonShapeDocValuesField`] and [`XYShapeDocValuesField`].
///
/// **Divergence from Lucene 10.5.0.** Java's class extends `Field` and leaves
/// the centroid, the bounding box and the coordinate decoding abstract. Rust
/// has no implementation inheritance, so this type holds the shared state and
/// the shared behaviour, and the two subclasses wrap it and add the typed
/// accessors.
#[derive(Clone, Debug)]
pub struct ShapeDocValuesField {
    name: String,
    field_type: FieldType,
    shape_doc_values: ShapeDocValues,
}

impl ShapeDocValuesField {
    /// Creates the field.
    ///
    /// Equivalent to `ShapeDocValuesField(String, ShapeDocValues)`.
    ///
    /// # Errors
    ///
    /// Propagates the error [`shape_doc_values_field_type`] raises.
    pub fn new(name: impl Into<String>, shape_doc_values: ShapeDocValues) -> Result<Self> {
        Ok(Self {
            name: name.into(),
            field_type: shape_doc_values_field_type()?,
            shape_doc_values,
        })
    }

    /// Returns the field's name.
    ///
    /// Equivalent to `ShapeDocValuesField.name()`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the field's type.
    ///
    /// Equivalent to `ShapeDocValuesField.fieldType()`.
    pub fn field_type(&self) -> &FieldType {
        &self.field_type
    }

    /// Returns the shape.
    pub fn shape_doc_values(&self) -> &ShapeDocValues {
        &self.shape_doc_values
    }

    /// Returns the stored bytes.
    ///
    /// Equivalent to the `fieldsData` a `ShapeDocValuesField` is constructed
    /// with, which is `shapeDocValues.binaryValue()`.
    pub fn binary_value(&self) -> &BytesRef {
        self.shape_doc_values.binary_value()
    }

    /// Returns the number of terms (tessellated triangles) of this shape.
    ///
    /// Equivalent to `ShapeDocValuesField.numberOfTerms()`.
    pub fn number_of_terms(&self) -> i32 {
        self.shape_doc_values.number_of_terms()
    }

    /// Returns the highest dimensional type of the geometry.
    ///
    /// Equivalent to `ShapeDocValuesField.getHighestDimensionType()`.
    pub fn get_highest_dimension_type(&self) -> TriangleType {
        self.shape_doc_values.get_highest_dimension()
    }

    /// Refuses to build a geometry query over shape doc values.
    ///
    /// Equivalent to the static `ShapeDocValuesField.newGeometryQuery`, which
    /// throws `IllegalStateException` because the general geometry query has
    /// not been written yet. The two bounding-box doc-values queries that *do*
    /// exist are
    /// [`LatLonShapeDocValuesQuery`](crate::document::LatLonShapeDocValuesQuery)
    /// and [`XYShapeDocValuesQuery`](crate::document::XYShapeDocValuesQuery).
    ///
    /// # Errors
    ///
    /// Always returns [`LuceneError::IllegalState`].
    pub fn new_geometry_query(field: &str) -> Result<()> {
        Err(LuceneError::IllegalState(format!(
            "geometry queries not yet supported on shape doc values for field [{field}]"
        )))
    }
}

/// Implements the accessors every shape doc-values field shares.
macro_rules! shape_doc_values_field_delegates {
    ($name:ident, $equiv:literal) => {
        impl $name {
            /// Returns the field's name.
            pub fn name(&self) -> &str {
                self.inner.name()
            }

            /// Returns the field's type.
            pub fn field_type(&self) -> &FieldType {
                self.inner.field_type()
            }

            /// Returns the stored bytes.
            pub fn binary_value(&self) -> &BytesRef {
                self.inner.binary_value()
            }

            /// Returns the number of terms (tessellated triangles).
            pub fn number_of_terms(&self) -> i32 {
                self.inner.number_of_terms()
            }

            /// Returns the highest dimensional type of the geometry.
            pub fn get_highest_dimension_type(&self) -> TriangleType {
                self.inner.get_highest_dimension_type()
            }

            /// Returns this field as an untyped [`ShapeDocValuesField`].
            pub fn as_shape_doc_values_field(&self) -> &ShapeDocValuesField {
                &self.inner
            }
        }

        impl IndexableField for $name {
            fn name(&self) -> &str {
                self.inner.name()
            }

            fn field_type(&self) -> &dyn IndexableFieldType {
                self.inner.field_type()
            }

            fn token_stream(
                &self,
                _analyzer: &dyn crate::analysis::Analyzer,
                _reuse: Option<&mut dyn crate::analysis::TokenStream>,
            ) -> Box<dyn crate::analysis::TokenStream> {
                // Java answers `null`: token streams are not supported, and the
                // field is never inverted because its `indexOptions` is `NONE`.
                Box::new(
                    crate::analysis::StringTokenStream::new(String::new())
                        .expect("INVARIANT: an empty StringTokenStream is always well formed"),
                )
            }

            fn binary_value(&self) -> Option<BytesRef> {
                Some(self.inner.binary_value().clone())
            }

            fn string_value(&self) -> Option<String> {
                // There is currently no string representation for a shape
                // doc-values field; Java's `stringValue()` answers `null`.
                let _ = $equiv;
                None
            }

            fn reader_value(&mut self) -> Option<&mut dyn std::io::Read> {
                None
            }

            fn numeric_value(&self) -> Option<crate::document::NumericValue> {
                None
            }

            fn stored_value(&self) -> Result<Option<crate::document::StoredValue>> {
                Ok(None)
            }

            fn invertable_type(&self) -> Option<crate::document::InvertableType> {
                None
            }
        }
    };
}

/// The geographic [`ShapeDocValuesField`].
///
/// Equivalent to `org.apache.lucene.document.LatLonShapeDocValuesField`. Build
/// one through the `create_doc_value_field_*` factories of
/// [`LatLonShape`](crate::document::LatLonShape).
///
/// **Warning:** like [`LatLonShape`](crate::document::LatLonShape), vertex
/// values are indexed with some loss of precision from the original `f64`
/// values.
#[derive(Clone, Debug)]
pub struct LatLonShapeDocValuesField {
    inner: ShapeDocValuesField,
    shape: LatLonShapeDocValues,
}

impl LatLonShapeDocValuesField {
    /// Creates the field from a pre-tessellated geometry.
    ///
    /// Equivalent to
    /// `LatLonShapeDocValuesField(String, List<ShapeField.DecodedTriangle>)`.
    ///
    /// # Errors
    ///
    /// As [`LatLonShapeDocValues::from_tessellation`].
    pub fn from_tessellation(
        name: impl Into<String>,
        tessellation: &[DecodedTriangle],
    ) -> Result<Self> {
        let shape = LatLonShapeDocValues::from_tessellation(tessellation)?;
        Ok(Self {
            inner: ShapeDocValuesField::new(name, shape.shape_doc_values().clone())?,
            shape,
        })
    }

    /// Creates the field from an already serialised value.
    ///
    /// Equivalent to `LatLonShapeDocValuesField(String, BytesRef)`.
    ///
    /// # Errors
    ///
    /// As [`LatLonShapeDocValues::from_binary_value`].
    pub fn from_binary_value(name: impl Into<String>, binary_value: BytesRef) -> Result<Self> {
        let shape = LatLonShapeDocValues::from_binary_value(binary_value)?;
        Ok(Self {
            inner: ShapeDocValuesField::new(name, shape.shape_doc_values().clone())?,
            shape,
        })
    }

    /// Returns the shape this field carries.
    pub fn shape(&self) -> &LatLonShapeDocValues {
        &self.shape
    }

    /// Returns the centroid of the geometry.
    ///
    /// Equivalent to `LatLonShapeDocValuesField.getCentroid()`.
    ///
    /// # Errors
    ///
    /// As [`LatLonShapeDocValues::get_centroid`].
    pub fn get_centroid(&self) -> Result<Point> {
        self.shape.get_centroid()
    }

    /// Returns the bounding box of the geometry.
    ///
    /// Equivalent to `LatLonShapeDocValuesField.getBoundingBox()`.
    ///
    /// # Errors
    ///
    /// As [`LatLonShapeDocValues::get_bounding_box`].
    pub fn get_bounding_box(&self) -> Result<Rectangle> {
        self.shape.get_bounding_box()
    }

    /// Decodes an x coordinate from encoded space.
    ///
    /// Equivalent to `LatLonShapeDocValuesField.decodeX(int)`.
    pub fn decode_x(encoded: i32) -> f64 {
        GeoEncodingUtils::decode_longitude(encoded)
    }

    /// Decodes a y coordinate from encoded space.
    ///
    /// Equivalent to `LatLonShapeDocValuesField.decodeY(int)`.
    pub fn decode_y(encoded: i32) -> f64 {
        GeoEncodingUtils::decode_latitude(encoded)
    }
}

shape_doc_values_field_delegates!(
    LatLonShapeDocValuesField,
    "org.apache.lucene.document.LatLonShapeDocValuesField"
);

/// The cartesian [`ShapeDocValuesField`].
///
/// Equivalent to `org.apache.lucene.document.XYShapeDocValuesField`. Build one
/// through the `create_doc_value_field_*` factories of
/// [`XYShape`](crate::document::XYShape).
#[derive(Clone, Debug)]
pub struct XYShapeDocValuesField {
    inner: ShapeDocValuesField,
    shape: XYShapeDocValues,
}

impl XYShapeDocValuesField {
    /// Creates the field from a pre-tessellated geometry.
    ///
    /// Equivalent to
    /// `XYShapeDocValuesField(String, List<ShapeField.DecodedTriangle>)`.
    ///
    /// # Errors
    ///
    /// As [`XYShapeDocValues::from_tessellation`].
    pub fn from_tessellation(
        name: impl Into<String>,
        tessellation: &[DecodedTriangle],
    ) -> Result<Self> {
        let shape = XYShapeDocValues::from_tessellation(tessellation)?;
        Ok(Self {
            inner: ShapeDocValuesField::new(name, shape.shape_doc_values().clone())?,
            shape,
        })
    }

    /// Creates the field from an already serialised value.
    ///
    /// Equivalent to `XYShapeDocValuesField(String, BytesRef)`.
    ///
    /// # Errors
    ///
    /// As [`XYShapeDocValues::from_binary_value`].
    pub fn from_binary_value(name: impl Into<String>, binary_value: BytesRef) -> Result<Self> {
        let shape = XYShapeDocValues::from_binary_value(binary_value)?;
        Ok(Self {
            inner: ShapeDocValuesField::new(name, shape.shape_doc_values().clone())?,
            shape,
        })
    }

    /// Returns the shape this field carries.
    pub fn shape(&self) -> &XYShapeDocValues {
        &self.shape
    }

    /// Returns the centroid of the geometry.
    ///
    /// Equivalent to `XYShapeDocValuesField.getCentroid()`.
    ///
    /// # Errors
    ///
    /// As [`XYShapeDocValues::get_centroid`].
    pub fn get_centroid(&self) -> Result<XYPoint> {
        self.shape.get_centroid()
    }

    /// Returns the bounding box of the geometry.
    ///
    /// Equivalent to `XYShapeDocValuesField.getBoundingBox()`.
    ///
    /// # Errors
    ///
    /// As [`XYShapeDocValues::get_bounding_box`].
    pub fn get_bounding_box(&self) -> Result<XYRectangle> {
        self.shape.get_bounding_box()
    }

    /// Decodes an x coordinate from encoded space.
    ///
    /// Equivalent to `XYShapeDocValuesField.decodeX(int)`.
    pub fn decode_x(encoded: i32) -> f64 {
        XYEncodingUtils::decode(encoded) as f64
    }

    /// Decodes a y coordinate from encoded space.
    ///
    /// Equivalent to `XYShapeDocValuesField.decodeY(int)`.
    pub fn decode_y(encoded: i32) -> f64 {
        XYEncodingUtils::decode(encoded) as f64
    }
}

shape_doc_values_field_delegates!(
    XYShapeDocValuesField,
    "org.apache.lucene.document.XYShapeDocValuesField"
);
