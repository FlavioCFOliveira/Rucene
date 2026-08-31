//! Geospatial fields ported from `org.apache.lucene.document`.
//!
//! The fields that index a location, in both the geographic and the cartesian
//! coordinate systems, as points for a BKD lookup or as doc values for a scan.

use std::io::Read;

use crate::analysis::{Analyzer, TokenStream};
use crate::document::{FieldData, FieldType, InvertableType, NumericValue, StoredValue};
use crate::error::{LuceneError, Result};
use crate::geo::encoding::{GeoEncodingUtils, XYEncodingUtils};
use crate::geo::geometry::{Rectangle, XYRectangle};
use crate::index::{DocValuesType, IndexableField, IndexableFieldType};
use crate::util::{BytesRef, NumericUtils};

/// Body shared by every packed-point field's `IndexableField::token_stream`.
fn point_token_stream(packed_value: Option<&[u8]>) -> Box<dyn TokenStream> {
    let value = packed_value
        .map(|bytes| BytesRef::new(bytes.to_vec()))
        .unwrap_or_else(|| BytesRef::new(Vec::new()));
    Box::new(crate::analysis::BinaryTokenStream::new(value).unwrap())
}

/// How many bytes one encoded coordinate occupies.
pub const COORDINATE_BYTES: usize = 4;

/// Builds the field type a two-dimensional point field uses.
fn point_field_type() -> Result<FieldType> {
    let mut ft = FieldType::new();
    ft.set_dimensions(2, COORDINATE_BYTES as i32)?;
    ft.freeze();
    Ok(ft)
}

/// Builds the field type a location doc-values field uses.
fn doc_values_field_type() -> Result<FieldType> {
    let mut ft = FieldType::new();
    ft.set_doc_values_type(DocValuesType::SORTED_NUMERIC)?;
    ft.freeze();
    Ok(ft)
}

/// A latitude/longitude pair indexed as a point.
///
/// Equivalent to `org.apache.lucene.document.LatLonPoint`.
#[derive(Debug)]
pub struct LatLonPoint {
    name: String,
    field_type: FieldType,
    fields_data: FieldData,
}

impl LatLonPoint {
    /// How many bytes one coordinate occupies.
    pub const BYTES: usize = COORDINATE_BYTES;

    /// Creates the field.
    pub fn new(name: impl Into<String>, latitude: f64, longitude: f64) -> Result<Self> {
        let mut field = Self {
            name: name.into(),
            field_type: point_field_type()?,
            fields_data: FieldData::Bytes(BytesRef::new(vec![0u8; 2 * COORDINATE_BYTES])),
        };
        field.set_location_value(latitude, longitude)?;
        Ok(field)
    }

    /// Replaces the location.
    ///
    /// Equivalent to `LatLonPoint.setLocationValue(double, double)`: the two
    /// encoded coordinates are packed as sortable bytes, latitude first.
    pub fn set_location_value(&mut self, latitude: f64, longitude: f64) -> Result<()> {
        let mut bytes = vec![0u8; 2 * COORDINATE_BYTES];
        NumericUtils::int_to_sortable_bytes(
            GeoEncodingUtils::encode_latitude(latitude)?,
            &mut bytes,
            0,
        );
        NumericUtils::int_to_sortable_bytes(
            GeoEncodingUtils::encode_longitude(longitude)?,
            &mut bytes,
            COORDINATE_BYTES,
        );
        self.fields_data = FieldData::Bytes(BytesRef::new(bytes));
        Ok(())
    }

    /// Returns the field's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the field's type.
    pub fn field_type(&self) -> &FieldType {
        &self.field_type
    }

    /// Returns the packed coordinates.
    pub fn packed_value(&self) -> Option<&[u8]> {
        match &self.fields_data {
            FieldData::Bytes(bytes) => Some(bytes.slice()),
            _ => None,
        }
    }

    /// Checks that `field_info` describes a field this point type can read.
    ///
    /// Equivalent to the package-private static
    /// `LatLonPoint.checkCompatible(FieldInfo)`. The point properties may be
    /// *unset* when, for instance, only a `StoredField` of the same name was
    /// written into the segment, so a zero is accepted.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the field was indexed with
    /// a different number of dimensions or a different width.
    pub fn check_compatible(field_info: &crate::index::FieldInfo) -> Result<()> {
        check_point_compatible(field_info, "LatLonPoint")
    }

    /// Encodes a bounding box into the packed form a range query compares
    /// against.
    ///
    /// Equivalent to the encoding `LatLonPoint.newBoxQuery` performs before it
    /// builds a `PointRangeQuery`.
    pub fn encode_box(rectangle: &Rectangle) -> Result<(Vec<u8>, Vec<u8>)> {
        let mut lower = vec![0u8; 2 * COORDINATE_BYTES];
        let mut upper = vec![0u8; 2 * COORDINATE_BYTES];
        NumericUtils::int_to_sortable_bytes(
            GeoEncodingUtils::encode_latitude_ceil(rectangle.min_lat())?,
            &mut lower,
            0,
        );
        NumericUtils::int_to_sortable_bytes(
            GeoEncodingUtils::encode_latitude(rectangle.max_lat())?,
            &mut upper,
            0,
        );
        NumericUtils::int_to_sortable_bytes(
            GeoEncodingUtils::encode_longitude_ceil(rectangle.min_lon())?,
            &mut lower,
            COORDINATE_BYTES,
        );
        NumericUtils::int_to_sortable_bytes(
            GeoEncodingUtils::encode_longitude(rectangle.max_lon())?,
            &mut upper,
            COORDINATE_BYTES,
        );
        Ok((lower, upper))
    }
}

impl IndexableField for LatLonPoint {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        point_token_stream(self.packed_value())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        match &self.fields_data {
            FieldData::Bytes(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::BINARY)
    }
}

/// A latitude/longitude pair stored as doc values.
///
/// Equivalent to `org.apache.lucene.document.LatLonDocValuesField`.
#[derive(Debug)]
pub struct LatLonDocValuesField {
    name: String,
    field_type: FieldType,
    value: i64,
}

impl LatLonDocValuesField {
    /// Creates the field.
    pub fn new(name: impl Into<String>, latitude: f64, longitude: f64) -> Result<Self> {
        let mut field = Self {
            name: name.into(),
            field_type: doc_values_field_type()?,
            value: 0,
        };
        field.set_location_value(latitude, longitude)?;
        Ok(field)
    }

    /// Replaces the location.
    ///
    /// Equivalent to `LatLonDocValuesField.setLocationValue`: the two encoded
    /// coordinates are packed into one long, latitude in the high half.
    pub fn set_location_value(&mut self, latitude: f64, longitude: f64) -> Result<()> {
        let lat = GeoEncodingUtils::encode_latitude(latitude)?;
        let lon = GeoEncodingUtils::encode_longitude(longitude)?;
        self.value = (i64::from(lat) << 32) | (i64::from(lon) & 0xFFFF_FFFF);
        Ok(())
    }

    /// Returns the field's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the field's type.
    pub fn field_type(&self) -> &FieldType {
        &self.field_type
    }

    /// Returns the packed value.
    pub fn numeric_value(&self) -> i64 {
        self.value
    }

    /// Checks that `field_info` describes a field this doc-values type can
    /// read.
    ///
    /// Equivalent to the package-private static
    /// `LatLonDocValuesField.checkCompatible(FieldInfo)`. The doc-values type
    /// may be *unset* when, for instance, only a `StoredField` of the same name
    /// was written into the segment.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the field was indexed with
    /// a different doc-values type.
    pub fn check_compatible(field_info: &crate::index::FieldInfo) -> Result<()> {
        check_doc_values_compatible(field_info, "LatLonDocValuesField")
    }

    /// Returns the latitude a packed value encodes.
    pub fn decode_latitude(value: i64) -> f64 {
        GeoEncodingUtils::decode_latitude((value >> 32) as i32)
    }

    /// Returns the longitude a packed value encodes.
    pub fn decode_longitude(value: i64) -> f64 {
        GeoEncodingUtils::decode_longitude((value & 0xFFFF_FFFF) as i32)
    }
}

impl IndexableField for LatLonDocValuesField {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        Box::new(crate::analysis::BinaryTokenStream::new(BytesRef::new(Vec::new())).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        None
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        Some(NumericValue::Long(self.value))
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        None
    }
}

/// An x/y pair indexed as a point.
///
/// Equivalent to `org.apache.lucene.document.XYPointField`.
#[derive(Debug)]
pub struct XYPointField {
    name: String,
    field_type: FieldType,
    fields_data: FieldData,
}

impl XYPointField {
    /// How many bytes one coordinate occupies.
    pub const BYTES: usize = COORDINATE_BYTES;

    /// Creates the field.
    pub fn new(name: impl Into<String>, x: f32, y: f32) -> Result<Self> {
        let mut field = Self {
            name: name.into(),
            field_type: point_field_type()?,
            fields_data: FieldData::Bytes(BytesRef::new(vec![0u8; 2 * COORDINATE_BYTES])),
        };
        field.set_location_value(x, y)?;
        Ok(field)
    }

    /// Replaces the location.
    pub fn set_location_value(&mut self, x: f32, y: f32) -> Result<()> {
        let mut bytes = vec![0u8; 2 * COORDINATE_BYTES];
        NumericUtils::int_to_sortable_bytes(XYEncodingUtils::encode(x)?, &mut bytes, 0);
        NumericUtils::int_to_sortable_bytes(
            XYEncodingUtils::encode(y)?,
            &mut bytes,
            COORDINATE_BYTES,
        );
        self.fields_data = FieldData::Bytes(BytesRef::new(bytes));
        Ok(())
    }

    /// Returns the field's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the field's type.
    pub fn field_type(&self) -> &FieldType {
        &self.field_type
    }

    /// Returns the packed coordinates.
    pub fn packed_value(&self) -> Option<&[u8]> {
        match &self.fields_data {
            FieldData::Bytes(bytes) => Some(bytes.slice()),
            _ => None,
        }
    }

    /// Checks that `field_info` describes a field this point type can read.
    ///
    /// Equivalent to the package-private static
    /// `XYPointField.checkCompatible(FieldInfo)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the field was indexed with
    /// a different number of dimensions or a different width.
    pub fn check_compatible(field_info: &crate::index::FieldInfo) -> Result<()> {
        check_point_compatible(field_info, "XYPoint")
    }

    /// Encodes a bounding box into the packed form a range query compares
    /// against.
    pub fn encode_box(rectangle: &XYRectangle) -> Result<(Vec<u8>, Vec<u8>)> {
        let mut lower = vec![0u8; 2 * COORDINATE_BYTES];
        let mut upper = vec![0u8; 2 * COORDINATE_BYTES];
        NumericUtils::int_to_sortable_bytes(
            XYEncodingUtils::encode(rectangle.min_x())?,
            &mut lower,
            0,
        );
        NumericUtils::int_to_sortable_bytes(
            XYEncodingUtils::encode(rectangle.max_x())?,
            &mut upper,
            0,
        );
        NumericUtils::int_to_sortable_bytes(
            XYEncodingUtils::encode(rectangle.min_y())?,
            &mut lower,
            COORDINATE_BYTES,
        );
        NumericUtils::int_to_sortable_bytes(
            XYEncodingUtils::encode(rectangle.max_y())?,
            &mut upper,
            COORDINATE_BYTES,
        );
        Ok((lower, upper))
    }
}

impl IndexableField for XYPointField {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        point_token_stream(self.packed_value())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        match &self.fields_data {
            FieldData::Bytes(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::BINARY)
    }
}

/// An x/y pair stored as doc values.
///
/// Equivalent to `org.apache.lucene.document.XYDocValuesField`.
#[derive(Debug)]
pub struct XYDocValuesField {
    name: String,
    field_type: FieldType,
    value: i64,
}

impl XYDocValuesField {
    /// Creates the field.
    pub fn new(name: impl Into<String>, x: f32, y: f32) -> Result<Self> {
        let mut field = Self {
            name: name.into(),
            field_type: doc_values_field_type()?,
            value: 0,
        };
        field.set_location_value(x, y)?;
        Ok(field)
    }

    /// Replaces the location.
    pub fn set_location_value(&mut self, x: f32, y: f32) -> Result<()> {
        let x = XYEncodingUtils::encode(x)?;
        let y = XYEncodingUtils::encode(y)?;
        self.value = (i64::from(x) << 32) | (i64::from(y) & 0xFFFF_FFFF);
        Ok(())
    }

    /// Returns the field's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the field's type.
    pub fn field_type(&self) -> &FieldType {
        &self.field_type
    }

    /// Returns the packed value.
    pub fn numeric_value(&self) -> i64 {
        self.value
    }

    /// Checks that `field_info` describes a field this doc-values type can
    /// read.
    ///
    /// Equivalent to the package-private static
    /// `XYDocValuesField.checkCompatible(FieldInfo)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the field was indexed with
    /// a different doc-values type.
    pub fn check_compatible(field_info: &crate::index::FieldInfo) -> Result<()> {
        check_doc_values_compatible(field_info, "XYDocValuesField")
    }

    /// Returns the x coordinate a packed value encodes.
    pub fn decode_x(value: i64) -> f32 {
        XYEncodingUtils::decode((value >> 32) as i32)
    }

    /// Returns the y coordinate a packed value encodes.
    pub fn decode_y(value: i64) -> f32 {
        XYEncodingUtils::decode((value & 0xFFFF_FFFF) as i32)
    }
}

impl IndexableField for XYDocValuesField {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        Box::new(crate::analysis::BinaryTokenStream::new(BytesRef::new(Vec::new())).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        None
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        Some(NumericValue::Long(self.value))
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        None
    }
}

/// An IPv4 or IPv6 address indexed as a point.
///
/// Equivalent to `org.apache.lucene.document.InetAddressPoint`.
#[derive(Debug)]
pub struct InetAddressPoint {
    name: String,
    field_type: FieldType,
    fields_data: FieldData,
}

impl InetAddressPoint {
    /// How many bytes an encoded address occupies: every address is stored in
    /// its IPv6 form.
    pub const BYTES: usize = 16;

    /// The prefix that maps an IPv4 address into IPv6 space.
    ///
    /// Equivalent to `InetAddressPoint.IPV4_PREFIX`.
    pub const IPV4_PREFIX: [u8; 12] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF];

    /// Creates the field from an address already in its 4- or 16-byte form.
    pub fn new(name: impl Into<String>, address: &[u8]) -> Result<Self> {
        let mut ft = FieldType::new();
        ft.set_dimensions(1, Self::BYTES as i32)?;
        ft.freeze();
        Ok(Self {
            name: name.into(),
            field_type: ft,
            fields_data: FieldData::Bytes(BytesRef::new(Self::encode(address)?)),
        })
    }

    /// Encodes an address into its 16-byte form.
    ///
    /// Equivalent to `InetAddressPoint.encode(InetAddress)`: an IPv4 address is
    /// mapped into IPv6 space so both sort in one order.
    pub fn encode(address: &[u8]) -> Result<Vec<u8>> {
        match address.len() {
            4 => {
                let mut bytes = vec![0u8; Self::BYTES];
                bytes[..12].copy_from_slice(&Self::IPV4_PREFIX);
                bytes[12..].copy_from_slice(address);
                Ok(bytes)
            }
            16 => Ok(address.to_vec()),
            other => Err(LuceneError::IllegalArgument(format!(
                "invalid address length {other}; must be 4 or 16 bytes"
            ))),
        }
    }

    /// Returns the field's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the field's type.
    pub fn field_type(&self) -> &FieldType {
        &self.field_type
    }

    /// Returns the encoded address.
    pub fn packed_value(&self) -> Option<&[u8]> {
        match &self.fields_data {
            FieldData::Bytes(bytes) => Some(bytes.slice()),
            _ => None,
        }
    }

    /// Returns the address one greater than `address`, or `None` when it is
    /// already the largest.
    ///
    /// Equivalent to `InetAddressPoint.nextUp(InetAddress)`, which a range
    /// query uses to turn an exclusive bound into an inclusive one.
    pub fn next_up(address: &[u8]) -> Option<Vec<u8>> {
        let mut bytes = address.to_vec();
        for i in (0..bytes.len()).rev() {
            if bytes[i] != 0xFF {
                bytes[i] += 1;
                for slot in bytes.iter_mut().skip(i + 1) {
                    *slot = 0;
                }
                return Some(bytes);
            }
        }
        None
    }

    /// Returns the address one less than `address`, or `None` when it is
    /// already the smallest.
    ///
    /// Equivalent to `InetAddressPoint.nextDown(InetAddress)`.
    pub fn next_down(address: &[u8]) -> Option<Vec<u8>> {
        let mut bytes = address.to_vec();
        for i in (0..bytes.len()).rev() {
            if bytes[i] != 0x00 {
                bytes[i] -= 1;
                for slot in bytes.iter_mut().skip(i + 1) {
                    *slot = 0xFF;
                }
                return Some(bytes);
            }
        }
        None
    }
}

impl IndexableField for InetAddressPoint {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        point_token_stream(self.packed_value())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        match &self.fields_data {
            FieldData::Bytes(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::BINARY)
    }
}

/// A range of IP addresses indexed as a two-dimensional range.
///
/// Equivalent to `org.apache.lucene.document.InetAddressRange`.
#[derive(Debug)]
pub struct InetAddressRange {
    name: String,
    field_type: FieldType,
    fields_data: FieldData,
}

impl InetAddressRange {
    /// How many bytes one address occupies.
    pub const BYTES: usize = InetAddressPoint::BYTES;

    /// Creates the field from a minimum and a maximum address.
    pub fn new(name: impl Into<String>, min: &[u8], max: &[u8]) -> Result<Self> {
        let mut ft = FieldType::new();
        ft.set_dimensions(2, Self::BYTES as i32)?;
        ft.freeze();
        let mut field = Self {
            name: name.into(),
            field_type: ft,
            fields_data: FieldData::Bytes(BytesRef::new(vec![0u8; 2 * Self::BYTES])),
        };
        field.set_range_values(min, max)?;
        Ok(field)
    }

    /// Replaces the range.
    ///
    /// Equivalent to `InetAddressRange.setRangeValues`.
    pub fn set_range_values(&mut self, min: &[u8], max: &[u8]) -> Result<()> {
        let min = InetAddressPoint::encode(min)?;
        let max = InetAddressPoint::encode(max)?;
        if min > max {
            return Err(LuceneError::IllegalArgument(
                "min address cannot be greater than max address".to_string(),
            ));
        }
        let mut bytes = vec![0u8; 2 * Self::BYTES];
        bytes[..Self::BYTES].copy_from_slice(&min);
        bytes[Self::BYTES..].copy_from_slice(&max);
        self.fields_data = FieldData::Bytes(BytesRef::new(bytes));
        Ok(())
    }

    /// Returns the field's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the field's type.
    pub fn field_type(&self) -> &FieldType {
        &self.field_type
    }

    /// Returns the packed range.
    pub fn packed_value(&self) -> Option<&[u8]> {
        match &self.fields_data {
            FieldData::Bytes(bytes) => Some(bytes.slice()),
            _ => None,
        }
    }
}

impl IndexableField for InetAddressRange {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        point_token_stream(self.packed_value())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        match &self.fields_data {
            FieldData::Bytes(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::BINARY)
    }
}

/// The body shared by `LatLonPoint.checkCompatible` and
/// `XYPointField.checkCompatible`, which differ only in the type name they
/// report.
///
/// Both point types declare two dimensions of [`COORDINATE_BYTES`] bytes each.
fn check_point_compatible(field_info: &crate::index::FieldInfo, type_name: &str) -> Result<()> {
    let dimension_count = 2;
    let num_bytes = COORDINATE_BYTES as i32;
    if field_info.point_dimension_count != 0 && field_info.point_dimension_count != dimension_count
    {
        return Err(LuceneError::IllegalArgument(format!(
            "field=\"{}\" was indexed with numDims={} but this point type has \
             numDims={dimension_count}, is the field really a {type_name}?",
            field_info.name, field_info.point_dimension_count
        )));
    }
    if field_info.point_num_bytes != 0 && field_info.point_num_bytes != num_bytes {
        return Err(LuceneError::IllegalArgument(format!(
            "field=\"{}\" was indexed with bytesPerDim={} but this point type has \
             bytesPerDim={num_bytes}, is the field really a {type_name}?",
            field_info.name, field_info.point_num_bytes
        )));
    }
    Ok(())
}

/// The body shared by `LatLonDocValuesField.checkCompatible` and
/// `XYDocValuesField.checkCompatible`, which differ only in the type name they
/// report. Both declare [`DocValuesType::SORTED_NUMERIC`].
fn check_doc_values_compatible(
    field_info: &crate::index::FieldInfo,
    type_name: &str,
) -> Result<()> {
    if field_info.doc_values_type != DocValuesType::NONE
        && field_info.doc_values_type != DocValuesType::SORTED_NUMERIC
    {
        return Err(LuceneError::IllegalArgument(format!(
            "field=\"{}\" was indexed with docValuesType={:?} but this type has \
             docValuesType=SORTED_NUMERIC, is the field really a {type_name}?",
            field_info.name, field_info.doc_values_type
        )));
    }
    Ok(())
}
