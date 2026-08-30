//! Coordinate encodings ported from `org.apache.lucene.geo`.

use crate::error::{LuceneError, Result};
use crate::util::NumericUtils;

/// Bounds and constants every geo coordinate is checked against.
///
/// Equivalent to `org.apache.lucene.geo.GeoUtils`.
pub struct GeoUtils;

impl GeoUtils {
    /// Smallest valid longitude.
    pub const MIN_LON_INCL: f64 = -180.0;
    /// Largest valid longitude.
    pub const MAX_LON_INCL: f64 = 180.0;
    /// Smallest valid latitude.
    pub const MIN_LAT_INCL: f64 = -90.0;
    /// Largest valid latitude.
    pub const MAX_LAT_INCL: f64 = 90.0;
    /// Mean earth radius, in metres, from the WGS84 ellipsoid.
    pub const EARTH_MEAN_RADIUS_METERS: f64 = 6_371_008.7714;

    /// Smallest valid longitude, in radians.
    pub fn min_lon_radians() -> f64 {
        Self::MIN_LON_INCL.to_radians()
    }

    /// Largest valid longitude, in radians.
    pub fn max_lon_radians() -> f64 {
        Self::MAX_LON_INCL.to_radians()
    }

    /// Smallest valid latitude, in radians.
    pub fn min_lat_radians() -> f64 {
        Self::MIN_LAT_INCL.to_radians()
    }

    /// Largest valid latitude, in radians.
    pub fn max_lat_radians() -> f64 {
        Self::MAX_LAT_INCL.to_radians()
    }

    /// Fails when `latitude` is outside the valid range.
    ///
    /// Equivalent to `GeoUtils.checkLatitude(double)`.
    pub fn check_latitude(latitude: f64) -> Result<()> {
        if latitude.is_nan() || !(Self::MIN_LAT_INCL..=Self::MAX_LAT_INCL).contains(&latitude) {
            return Err(LuceneError::IllegalArgument(format!(
                "invalid latitude {latitude}; must be between {} and {}",
                Self::MIN_LAT_INCL,
                Self::MAX_LAT_INCL
            )));
        }
        Ok(())
    }

    /// Fails when `longitude` is outside the valid range.
    ///
    /// Equivalent to `GeoUtils.checkLongitude(double)`.
    pub fn check_longitude(longitude: f64) -> Result<()> {
        if longitude.is_nan() || !(Self::MIN_LON_INCL..=Self::MAX_LON_INCL).contains(&longitude) {
            return Err(LuceneError::IllegalArgument(format!(
                "invalid longitude {longitude}; must be between {} and {}",
                Self::MIN_LON_INCL,
                Self::MAX_LON_INCL
            )));
        }
        Ok(())
    }
}

/// Encodes latitude and longitude into the fixed-width integers a geo point
/// field indexes.
///
/// Equivalent to `org.apache.lucene.geo.GeoEncodingUtils`.
pub struct GeoEncodingUtils;

impl GeoEncodingUtils {
    /// How many bits one coordinate occupies.
    pub const BITS: i16 = 32;

    /// Encoded units per degree of latitude.
    fn lat_scale() -> f64 {
        (1i64 << Self::BITS) as f64 / 180.0
    }

    /// Degrees of latitude per encoded unit.
    fn lat_decode() -> f64 {
        1.0 / Self::lat_scale()
    }

    /// Encoded units per degree of longitude.
    fn lon_scale() -> f64 {
        (1i64 << Self::BITS) as f64 / 360.0
    }

    /// Degrees of longitude per encoded unit.
    fn lon_decode() -> f64 {
        1.0 / Self::lon_scale()
    }

    /// Encodes a latitude, rounding down.
    ///
    /// Equivalent to `GeoEncodingUtils.encodeLatitude(double)`. The maximum
    /// value is nudged down because it cannot be encoded without overflow.
    pub fn encode_latitude(latitude: f64) -> Result<i32> {
        GeoUtils::check_latitude(latitude)?;
        let latitude = if latitude == 90.0 {
            next_down(latitude)
        } else {
            latitude
        };
        Ok((latitude / Self::lat_decode()).floor() as i32)
    }

    /// Encodes a latitude, rounding up.
    ///
    /// Equivalent to `GeoEncodingUtils.encodeLatitudeCeil(double)`.
    pub fn encode_latitude_ceil(latitude: f64) -> Result<i32> {
        GeoUtils::check_latitude(latitude)?;
        let latitude = if latitude == 90.0 {
            next_down(latitude)
        } else {
            latitude
        };
        Ok((latitude / Self::lat_decode()).ceil() as i32)
    }

    /// Encodes a longitude, rounding down.
    ///
    /// Equivalent to `GeoEncodingUtils.encodeLongitude(double)`.
    pub fn encode_longitude(longitude: f64) -> Result<i32> {
        GeoUtils::check_longitude(longitude)?;
        let longitude = if longitude == 180.0 {
            next_down(longitude)
        } else {
            longitude
        };
        Ok((longitude / Self::lon_decode()).floor() as i32)
    }

    /// Encodes a longitude, rounding up.
    ///
    /// Equivalent to `GeoEncodingUtils.encodeLongitudeCeil(double)`.
    pub fn encode_longitude_ceil(longitude: f64) -> Result<i32> {
        GeoUtils::check_longitude(longitude)?;
        let longitude = if longitude == 180.0 {
            next_down(longitude)
        } else {
            longitude
        };
        Ok((longitude / Self::lon_decode()).ceil() as i32)
    }

    /// Decodes a latitude.
    ///
    /// Equivalent to `GeoEncodingUtils.decodeLatitude(int)`.
    pub fn decode_latitude(encoded: i32) -> f64 {
        f64::from(encoded) * Self::lat_decode()
    }

    /// Decodes a latitude from its sortable-bytes form.
    pub fn decode_latitude_bytes(src: &[u8], offset: usize) -> f64 {
        Self::decode_latitude(NumericUtils::sortable_bytes_to_int(src, offset))
    }

    /// Decodes a longitude.
    ///
    /// Equivalent to `GeoEncodingUtils.decodeLongitude(int)`.
    pub fn decode_longitude(encoded: i32) -> f64 {
        f64::from(encoded) * Self::lon_decode()
    }

    /// Decodes a longitude from its sortable-bytes form.
    pub fn decode_longitude_bytes(src: &[u8], offset: usize) -> f64 {
        Self::decode_longitude(NumericUtils::sortable_bytes_to_int(src, offset))
    }

    /// Smallest encoded longitude.
    pub fn min_lon_encoded() -> i32 {
        Self::encode_longitude(GeoUtils::MIN_LON_INCL).unwrap_or(i32::MIN)
    }

    /// Largest encoded longitude.
    pub fn max_lon_encoded() -> i32 {
        Self::encode_longitude(GeoUtils::MAX_LON_INCL).unwrap_or(i32::MAX)
    }
}

/// Returns the largest float below `value`, as `Math.nextDown` does.
fn next_down(value: f64) -> f64 {
    if value.is_nan() || value == f64::NEG_INFINITY {
        return value;
    }
    if value == 0.0 {
        return -f64::MIN_POSITIVE;
    }
    let bits = value.to_bits();
    f64::from_bits(if value > 0.0 { bits - 1 } else { bits + 1 })
}

/// Encodes the coordinates of a cartesian point field.
///
/// Equivalent to `org.apache.lucene.geo.XYEncodingUtils`, which stores a float
/// in its sortable-int form rather than quantising a degree range.
pub struct XYEncodingUtils;

impl XYEncodingUtils {
    /// Smallest valid coordinate.
    pub const MIN_VAL_INCL: f64 = -(f32::MAX as f64);
    /// Largest valid coordinate.
    pub const MAX_VAL_INCL: f64 = f32::MAX as f64;

    /// Fails when `x` is not finite.
    ///
    /// Equivalent to `XYEncodingUtils.checkVal(float)`.
    pub fn check_val(x: f32) -> Result<f32> {
        if !x.is_finite() {
            return Err(LuceneError::IllegalArgument(format!(
                "invalid value {x}; must be between {} and {}",
                Self::MIN_VAL_INCL,
                Self::MAX_VAL_INCL
            )));
        }
        Ok(x)
    }

    /// Encodes a coordinate.
    ///
    /// Equivalent to `XYEncodingUtils.encode(float)`.
    pub fn encode(x: f32) -> Result<i32> {
        Ok(NumericUtils::float_to_sortable_int(Self::check_val(x)?))
    }

    /// Decodes a coordinate.
    ///
    /// Equivalent to `XYEncodingUtils.decode(int)`.
    pub fn decode(encoded: i32) -> f32 {
        NumericUtils::sortable_int_to_float(encoded)
    }

    /// Decodes a coordinate from its sortable-bytes form.
    pub fn decode_bytes(src: &[u8], offset: usize) -> f32 {
        Self::decode(NumericUtils::sortable_bytes_to_int(src, offset))
    }
}
