//! `ShapeField` and `LateInteractionField` ported from
//! `org.apache.lucene.document`.
//!
//! A shape is indexed as a set of triangles, each packed into a seven-dimension
//! point whose first four dimensions are its bounding box — which is what lets
//! a BKD tree prune whole subtrees of a polygon.

use crate::document::{FieldData, FieldType};
use crate::error::{LuceneError, Result};
use crate::util::{BytesRef, NumericUtils};

/// How many bytes one encoded coordinate occupies.
pub const BYTES: usize = 4;

/// How the query shape must relate to a document's shape for it to match.
///
/// Equivalent to `ShapeField.QueryRelation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryRelation {
    /// The two shapes share any point.
    Intersects,
    /// The document's shape lies inside the query's.
    Within,
    /// The two shapes share no point.
    Disjoint,
    /// The document's shape encloses the query's.
    Contains,
}

/// Which degenerate form a decoded triangle takes.
///
/// Equivalent to `ShapeField.DecodedTriangle.TYPE`, declared in the same order:
/// the ordinal is written to the shape doc-values stream by
/// [`ShapeDocValues`](crate::document::ShapeDocValues), so the order is part of
/// the file format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TriangleType {
    /// All three vertices coincide.
    ///
    /// Also the default, because a default-constructed
    /// [`DecodedTriangle`] has all six coordinates at zero, which is exactly
    /// what [`DecodedTriangle::resolve_triangle_type`] resolves to a point.
    #[default]
    Point,
    /// The first and third vertices coincide.
    Line,
    /// All three vertices differ.
    Triangle,
}

impl TriangleType {
    /// Returns the ordinal this type is written with.
    ///
    /// Equivalent to `ShapeField.DecodedTriangle.TYPE.ordinal()`.
    pub fn ordinal(self) -> i32 {
        match self {
            Self::Point => 0,
            Self::Line => 1,
            Self::Triangle => 2,
        }
    }

    /// Returns the type with the given ordinal.
    ///
    /// Equivalent to `ShapeField.DecodedTriangle.TYPE.values()[ordinal]`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::CorruptIndex`] for an ordinal outside `[0, 2]`,
    /// where Java throws `ArrayIndexOutOfBoundsException`.
    pub fn from_ordinal(ordinal: i32) -> Result<Self> {
        match ordinal {
            0 => Ok(Self::Point),
            1 => Ok(Self::Line),
            2 => Ok(Self::Triangle),
            other => Err(LuceneError::corrupt_index(
                format!("invalid triangle type ordinal {other}"),
                "shape doc values",
            )),
        }
    }
}

/// A triangle read back out of the index.
///
/// Equivalent to `ShapeField.DecodedTriangle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DecodedTriangle {
    /// Which degenerate form the triangle takes.
    ///
    /// Equivalent to the `DecodedTriangle.type` field, which
    /// [`resolve_triangle_type`](Self::resolve_triangle_type) sets. Note that
    /// Java excludes it from `equals`/`hashCode`, which compare only the
    /// coordinates and the edge flags; `PartialEq` here includes it, and cannot
    /// disagree, because the type is a function of the coordinates.
    pub triangle_type: TriangleType,
    /// X of the first vertex.
    pub a_x: i32,
    /// Y of the first vertex.
    pub a_y: i32,
    /// X of the second vertex.
    pub b_x: i32,
    /// Y of the second vertex.
    pub b_y: i32,
    /// X of the third vertex.
    pub c_x: i32,
    /// Y of the third vertex.
    pub c_y: i32,
    /// Whether edge `a`→`b` belongs to the original shape's outline.
    pub ab: bool,
    /// Whether edge `b`→`c` belongs to the original shape's outline.
    pub bc: bool,
    /// Whether edge `c`→`a` belongs to the original shape's outline.
    pub ca: bool,
}

impl DecodedTriangle {
    /// Sets the six coordinates and the three edge flags.
    ///
    /// Equivalent to `DecodedTriangle.setValues(...)`, which takes its
    /// arguments in the same `(x, y, edge)` triples per vertex. It does *not*
    /// set the type: call
    /// [`resolve_triangle_type`](Self::resolve_triangle_type) afterwards, as
    /// `ShapeField.decodeTriangle` does.
    #[allow(clippy::too_many_arguments)]
    pub fn set_values(
        &mut self,
        a_x: i32,
        a_y: i32,
        ab: bool,
        b_x: i32,
        b_y: i32,
        bc: bool,
        c_x: i32,
        c_y: i32,
        ca: bool,
    ) {
        self.a_x = a_x;
        self.a_y = a_y;
        self.ab = ab;
        self.b_x = b_x;
        self.b_y = b_y;
        self.bc = bc;
        self.c_x = c_x;
        self.c_y = c_y;
        self.ca = ca;
    }

    /// Resolves the triangle's type, **normalising its vertices and edge flags
    /// in the process**.
    ///
    /// Equivalent to the package-private static
    /// `ShapeField.resolveTriangleType(DecodedTriangle)`. A triangle with two
    /// coincident vertices is a line segment, and Lucene rewrites it into the
    /// canonical line form — `a`→`b` with `c` repeating `a` — merging the edge
    /// flags of the two edges that collapse into one. Callers of
    /// [`ShapeField::decode_triangle`] therefore see the same `b`/`c`
    /// coordinates and the same `ab` flag Java's do; skipping this step changes
    /// which documents a shape query matches, and what a shape doc value
    /// serialises to.
    pub fn resolve_triangle_type(&mut self) {
        if self.a_x == self.b_x && self.a_y == self.b_y {
            if self.a_x == self.c_x && self.a_y == self.c_y {
                self.triangle_type = TriangleType::Point;
            } else {
                // `a` and `b` are identical: remove `ab`, and merge `bc` and `ca`.
                self.ab = self.bc | self.ca;
                self.b_x = self.c_x;
                self.b_y = self.c_y;
                self.c_x = self.a_x;
                self.c_y = self.a_y;
                self.triangle_type = TriangleType::Line;
            }
        } else if self.a_x == self.c_x && self.a_y == self.c_y {
            // `a` and `c` are identical: remove `ac`, and merge `ab` and `bc`.
            self.ab |= self.bc;
            self.triangle_type = TriangleType::Line;
        } else if self.b_x == self.c_x && self.b_y == self.c_y {
            // `b` and `c` are identical: remove `bc`, and merge `ab` and `ca`.
            self.ab |= self.ca;
            self.c_x = self.a_x;
            self.c_y = self.a_y;
            self.triangle_type = TriangleType::Line;
        } else {
            self.triangle_type = TriangleType::Triangle;
        }
    }
}

/// The eight vertex orderings the encoding distinguishes, so a decoder can
/// recover which vertex held the minimum and maximum of each axis.
const MINY_MINX_MAXY_MAXX_Y_X: i32 = 0;
const MINY_MINX_Y_X_MAXY_MAXX: i32 = 1;
const MAXY_MINX_Y_X_MINY_MAXX: i32 = 2;
const MAXY_MINX_MINY_MAXX_Y_X: i32 = 3;
const Y_MINX_MINY_X_MAXY_MAXX: i32 = 4;
const Y_MINX_MINY_MAXX_MAXY_X: i32 = 5;
const MAXY_MINX_MINY_X_Y_MAXX: i32 = 6;
const MINY_MINX_Y_MAXX_MAXY_X: i32 = 7;

/// Packs and unpacks the triangles a shape is decomposed into.
///
/// Equivalent to `org.apache.lucene.document.ShapeField`.
pub struct ShapeField;

impl ShapeField {
    /// Builds the field type a shape's triangles are indexed with: seven
    /// dimensions, of which the first four are indexed as the bounding box.
    pub fn field_type() -> Result<FieldType> {
        let mut ft = FieldType::new();
        ft.set_dimensions_with_index_count(7, 4, BYTES as i32)?;
        ft.freeze();
        Ok(ft)
    }

    /// Packs one triangle into `bytes`.
    ///
    /// Equivalent to `ShapeField.encodeTriangle`. The vertices are first
    /// rotated so the one with the smallest x comes first, then the bounding
    /// box and the remaining vertex are written, with a bit field recording
    /// which rotation was applied and which edges belong to the shape.
    #[allow(clippy::too_many_arguments)]
    pub fn encode_triangle(
        bytes: &mut [u8],
        mut a_y: i32,
        mut a_x: i32,
        mut ab: bool,
        mut b_y: i32,
        mut b_x: i32,
        mut bc: bool,
        mut c_y: i32,
        mut c_x: i32,
        mut ca: bool,
    ) -> Result<()> {
        if bytes.len() != 7 * BYTES {
            return Err(LuceneError::IllegalArgument(format!(
                "bytes must be {} long, got {}",
                7 * BYTES,
                bytes.len()
            )));
        }

        // Rotate so that vertex `a` holds the smallest x, breaking a tie on the
        // smallest y.
        if b_x < a_x || c_x < a_x {
            let (temp_x, temp_y, temp_bool) = (a_x, a_y, ab);
            if b_x < c_x {
                a_x = b_x;
                a_y = b_y;
                ab = bc;
                b_x = c_x;
                b_y = c_y;
                bc = ca;
                c_x = temp_x;
                c_y = temp_y;
                ca = temp_bool;
            } else {
                a_x = c_x;
                a_y = c_y;
                ab = ca;
                c_x = b_x;
                c_y = b_y;
                ca = bc;
                b_x = temp_x;
                b_y = temp_y;
                bc = temp_bool;
            }
        } else if a_x == b_x && a_x == c_x && (b_y < a_y || c_y < a_y) {
            let (temp_x, temp_y, temp_bool) = (a_x, a_y, ab);
            if b_y < c_y {
                a_x = b_x;
                a_y = b_y;
                ab = bc;
                b_x = c_x;
                b_y = c_y;
                bc = ca;
                c_x = temp_x;
                c_y = temp_y;
                ca = temp_bool;
            } else {
                a_x = c_x;
                a_y = c_y;
                ab = ca;
                c_x = b_x;
                c_y = b_y;
                ca = bc;
                b_x = temp_x;
                b_y = temp_y;
                bc = temp_bool;
            }
        }

        let min_x = a_x;
        let min_y = a_y.min(b_y).min(c_y);
        let max_x = a_x.max(b_x).max(c_x);
        let max_y = a_y.max(b_y).max(c_y);

        let (mut bits, x, y);
        if min_y == a_y {
            if max_y == b_y && max_x == b_x {
                y = c_y;
                x = c_x;
                bits = MINY_MINX_MAXY_MAXX_Y_X;
            } else if max_y == c_y && max_x == c_x {
                y = b_y;
                x = b_x;
                bits = MINY_MINX_Y_X_MAXY_MAXX;
            } else {
                y = b_y;
                x = c_x;
                bits = MINY_MINX_Y_MAXX_MAXY_X;
            }
        } else if max_y == a_y {
            if min_y == b_y && max_x == b_x {
                y = c_y;
                x = c_x;
                bits = MAXY_MINX_MINY_MAXX_Y_X;
            } else if min_y == c_y && max_x == c_x {
                y = b_y;
                x = b_x;
                bits = MAXY_MINX_Y_X_MINY_MAXX;
            } else {
                y = c_y;
                x = b_x;
                bits = MAXY_MINX_MINY_X_Y_MAXX;
            }
        } else if max_x == b_x && min_y == b_y {
            y = a_y;
            x = c_x;
            bits = Y_MINX_MINY_MAXX_MAXY_X;
        } else if max_x == c_x && max_y == c_y {
            y = a_y;
            x = b_x;
            bits = Y_MINX_MINY_X_MAXY_MAXX;
        } else {
            return Err(LuceneError::IllegalArgument(
                "Could not encode the provided triangle".to_string(),
            ));
        }

        // The three high bits record which edges belong to the shape's outline.
        bits |= if ab { 1 << 3 } else { 0 };
        bits |= if bc { 1 << 4 } else { 0 };
        bits |= if ca { 1 << 5 } else { 0 };

        NumericUtils::int_to_sortable_bytes(min_y, bytes, 0);
        NumericUtils::int_to_sortable_bytes(min_x, bytes, BYTES);
        NumericUtils::int_to_sortable_bytes(max_y, bytes, 2 * BYTES);
        NumericUtils::int_to_sortable_bytes(max_x, bytes, 3 * BYTES);
        NumericUtils::int_to_sortable_bytes(y, bytes, 4 * BYTES);
        NumericUtils::int_to_sortable_bytes(x, bytes, 5 * BYTES);
        NumericUtils::int_to_sortable_bytes(bits, bytes, 6 * BYTES);
        Ok(())
    }

    /// Unpacks one triangle from `bytes`.
    ///
    /// Equivalent to `ShapeField.decodeTriangle`, which reverses the rotation
    /// the bit field records.
    pub fn decode_triangle(bytes: &[u8]) -> Result<DecodedTriangle> {
        if bytes.len() < 7 * BYTES {
            return Err(LuceneError::IllegalArgument(format!(
                "bytes must be at least {} long, got {}",
                7 * BYTES,
                bytes.len()
            )));
        }
        let min_y = NumericUtils::sortable_bytes_to_int(bytes, 0);
        let min_x = NumericUtils::sortable_bytes_to_int(bytes, BYTES);
        let max_y = NumericUtils::sortable_bytes_to_int(bytes, 2 * BYTES);
        let max_x = NumericUtils::sortable_bytes_to_int(bytes, 3 * BYTES);
        let y = NumericUtils::sortable_bytes_to_int(bytes, 4 * BYTES);
        let x = NumericUtils::sortable_bytes_to_int(bytes, 5 * BYTES);
        let bits = NumericUtils::sortable_bytes_to_int(bytes, 6 * BYTES);

        let mut t = DecodedTriangle {
            ab: bits & (1 << 3) != 0,
            bc: bits & (1 << 4) != 0,
            ca: bits & (1 << 5) != 0,
            ..Default::default()
        };

        match bits & 0x07 {
            MINY_MINX_MAXY_MAXX_Y_X => {
                t.a_y = min_y;
                t.a_x = min_x;
                t.b_y = max_y;
                t.b_x = max_x;
                t.c_y = y;
                t.c_x = x;
            }
            MINY_MINX_Y_X_MAXY_MAXX => {
                t.a_y = min_y;
                t.a_x = min_x;
                t.b_y = y;
                t.b_x = x;
                t.c_y = max_y;
                t.c_x = max_x;
            }
            MAXY_MINX_Y_X_MINY_MAXX => {
                t.a_y = max_y;
                t.a_x = min_x;
                t.b_y = y;
                t.b_x = x;
                t.c_y = min_y;
                t.c_x = max_x;
            }
            MAXY_MINX_MINY_MAXX_Y_X => {
                t.a_y = max_y;
                t.a_x = min_x;
                t.b_y = min_y;
                t.b_x = max_x;
                t.c_y = y;
                t.c_x = x;
            }
            Y_MINX_MINY_X_MAXY_MAXX => {
                t.a_y = y;
                t.a_x = min_x;
                t.b_y = min_y;
                t.b_x = x;
                t.c_y = max_y;
                t.c_x = max_x;
            }
            Y_MINX_MINY_MAXX_MAXY_X => {
                t.a_y = y;
                t.a_x = min_x;
                t.b_y = min_y;
                t.b_x = max_x;
                t.c_y = max_y;
                t.c_x = x;
            }
            MAXY_MINX_MINY_X_Y_MAXX => {
                t.a_y = max_y;
                t.a_x = min_x;
                t.b_y = min_y;
                t.b_x = x;
                t.c_y = y;
                t.c_x = max_x;
            }
            MINY_MINX_Y_MAXX_MAXY_X => {
                t.a_y = min_y;
                t.a_x = min_x;
                t.b_y = y;
                t.b_x = max_x;
                t.c_y = max_y;
                t.c_x = x;
            }
            other => {
                return Err(LuceneError::corrupt_index(
                    format!("Could not decode the provided triangle: bits {other}"),
                    "shape field",
                ))
            }
        }
        // Java's `decodeTriangle` finishes with `resolveTriangleType(triangle)`,
        // which both classifies the triangle and normalises a degenerate one.
        t.resolve_triangle_type();
        Ok(t)
    }
}

/// One triangle of a shape, as an indexable field.
///
/// Equivalent to `ShapeField.Triangle`.
#[derive(Debug)]
pub struct Triangle {
    name: String,
    field_type: FieldType,
    fields_data: FieldData,
}

impl Triangle {
    /// Creates the field from three already encoded vertices.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: impl Into<String>,
        a_x: i32,
        a_y: i32,
        b_x: i32,
        b_y: i32,
        c_x: i32,
        c_y: i32,
    ) -> Result<Self> {
        let mut bytes = vec![0u8; 7 * BYTES];
        ShapeField::encode_triangle(&mut bytes, a_y, a_x, true, b_y, b_x, true, c_y, c_x, true)?;
        Ok(Self {
            name: name.into(),
            field_type: ShapeField::field_type()?,
            fields_data: FieldData::Bytes(BytesRef::new(bytes)),
        })
    }

    /// Returns the field's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the field's type.
    pub fn field_type(&self) -> &FieldType {
        &self.field_type
    }

    /// Creates the field from a tessellated triangle, carrying over which of
    /// its edges belong to the original polygon.
    ///
    /// Equivalent to `ShapeField.Triangle(String, Tessellator.Triangle)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`ShapeField::encode_triangle`] raises.
    pub fn from_tessellator_triangle(
        name: impl Into<String>,
        t: &crate::geo::tessellator::Triangle,
    ) -> Result<Self> {
        let mut bytes = vec![0u8; 7 * BYTES];
        ShapeField::encode_triangle(
            &mut bytes,
            t.get_encoded_y(0),
            t.get_encoded_x(0),
            t.is_edge_from_polygon(0),
            t.get_encoded_y(1),
            t.get_encoded_x(1),
            t.is_edge_from_polygon(1),
            t.get_encoded_y(2),
            t.get_encoded_x(2),
            t.is_edge_from_polygon(2),
        )?;
        Ok(Self {
            name: name.into(),
            field_type: ShapeField::field_type()?,
            fields_data: FieldData::Bytes(BytesRef::new(bytes)),
        })
    }

    /// Returns the packed triangle.
    pub fn packed_value(&self) -> Option<&[u8]> {
        match &self.fields_data {
            FieldData::Bytes(bytes) => Some(bytes.slice()),
            _ => None,
        }
    }

    /// Returns the packed triangle as a `BytesRef`, which is what
    /// `Field.binaryValue()` answers for a shape field.
    pub fn binary_value(&self) -> Option<&BytesRef> {
        match &self.fields_data {
            FieldData::Bytes(bytes) => Some(bytes),
            _ => None,
        }
    }
}

/// A multi-vector value, stored as binary doc values.
///
/// Equivalent to `org.apache.lucene.document.LateInteractionField`, which holds
/// one vector per token so a late-interaction model can score against every one.
#[derive(Clone, Debug)]
pub struct LateInteractionField {
    name: String,
    value: BytesRef,
}

impl LateInteractionField {
    /// Creates the field from one vector per token.
    pub fn new(name: impl Into<String>, value: &[Vec<f32>]) -> Result<Self> {
        Ok(Self {
            name: name.into(),
            value: Self::encode(value)?,
        })
    }

    /// Replaces the value.
    pub fn set_value(&mut self, value: &[Vec<f32>]) -> Result<()> {
        self.value = Self::encode(value)?;
        Ok(())
    }

    /// Returns the field's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the stored bytes.
    pub fn binary_value(&self) -> &BytesRef {
        &self.value
    }

    /// Returns the vectors the field holds.
    pub fn get_value(&self) -> Result<Vec<Vec<f32>>> {
        Self::decode(&self.value)
    }

    /// Packs the vectors: the dimension as a little-endian int, then every
    /// vector's components as little-endian floats.
    ///
    /// Equivalent to `LateInteractionField.encode(float[][])`.
    pub fn encode(value: &[Vec<f32>]) -> Result<BytesRef> {
        if value.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "Value should not be null or empty".to_string(),
            ));
        }
        let dimension = value[0].len();
        if dimension == 0 {
            return Err(LuceneError::IllegalArgument(
                "Composing token vectors should not be null or empty".to_string(),
            ));
        }
        let mut bytes = Vec::with_capacity(4 + value.len() * dimension * 4);
        bytes.extend_from_slice(&(dimension as i32).to_le_bytes());
        for (i, vector) in value.iter().enumerate() {
            if vector.len() != dimension {
                return Err(LuceneError::IllegalArgument(format!(
                    "Composing token vectors should have the same dimension. Mismatching \
                     dimensions detected between token[0] and token[{i}], {dimension} != {}",
                    vector.len()
                )));
            }
            for &component in vector {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
        Ok(BytesRef::new(bytes))
    }

    /// Unpacks what [`encode`](Self::encode) wrote.
    ///
    /// Equivalent to `LateInteractionField.decode(BytesRef)`.
    pub fn decode(payload: &BytesRef) -> Result<Vec<Vec<f32>>> {
        let bytes = payload.slice();
        if bytes.len() < 4 {
            return Err(LuceneError::IllegalArgument(
                "Provided payload does not appear to have been encoded via \
                 LateInteractionField::encode"
                    .to_string(),
            ));
        }
        let dimension = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        if dimension == 0 {
            return Err(LuceneError::IllegalArgument(
                "Provided payload declares a zero dimension".to_string(),
            ));
        }
        let num_vectors = (bytes.len() - 4) / (dimension * 4);
        if num_vectors * dimension * 4 + 4 != bytes.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "Provided payload does not appear to have been encoded via \
                 LateInteractionField::encode. Payload length should be equal to 4 + numVectors * \
                 tokenVectorDimension, got {} != 4 + {num_vectors} * {dimension}",
                bytes.len()
            )));
        }
        let mut value = Vec::with_capacity(num_vectors);
        let mut pos = 4;
        for _ in 0..num_vectors {
            let mut vector = Vec::with_capacity(dimension);
            for _ in 0..dimension {
                vector.push(f32::from_le_bytes([
                    bytes[pos],
                    bytes[pos + 1],
                    bytes[pos + 2],
                    bytes[pos + 3],
                ]));
                pos += 4;
            }
            value.push(vector);
        }
        Ok(value)
    }
}
