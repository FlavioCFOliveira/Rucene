//! Coordinate encodings ported from `org.apache.lucene.geo`.

use crate::error::{LuceneError, Result};
use crate::geo::geometry::Rectangle;
use crate::index::point_values::Relation;
use crate::util::{NumericUtils, SloppyMath};

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

// -----------------------------------------------------------------------------
// GeoUtils: winding order and planar predicates
// -----------------------------------------------------------------------------

/// Orientation of three points.
///
/// Equivalent to `org.apache.lucene.geo.GeoUtils.WindingOrder`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindingOrder {
    /// Clockwise (sign `-1`).
    CW,
    /// Collinear (sign `0`).
    COLINEAR,
    /// Counter-clockwise (sign `1`).
    CCW,
}

impl WindingOrder {
    /// Returns the sign this winding order is defined by.
    ///
    /// Equivalent to `WindingOrder.sign()`.
    pub fn sign(self) -> i32 {
        match self {
            Self::CW => -1,
            Self::COLINEAR => 0,
            Self::CCW => 1,
        }
    }

    /// Returns the winding order carrying `sign`.
    ///
    /// Equivalent to `WindingOrder.fromSign(int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `sign` is not `-1`, `0` or
    /// `1`, as Java throws `IllegalArgumentException`.
    pub fn from_sign(sign: i32) -> Result<Self> {
        match sign {
            -1 => Ok(Self::CW),
            0 => Ok(Self::COLINEAR),
            1 => Ok(Self::CCW),
            other => Err(LuceneError::IllegalArgument(format!(
                "Invalid WindingOrder sign: {other}"
            ))),
        }
    }
}

impl GeoUtils {
    /// Binary-searches the exact sort key that matches `radius`: any sort key
    /// less than or equal to the returned value is a query match.
    ///
    /// Equivalent to `GeoUtils.distanceQuerySortKey(double)`.
    pub fn distance_query_sort_key(radius: f64) -> f64 {
        let effectively_infinite = SloppyMath::haversin_meters_from_sort_key(f64::MAX);
        // effectively infinite
        if radius >= effectively_infinite {
            return effectively_infinite;
        }

        // this is a search through non-negative long space only
        let mut lo: i64 = 0;
        let mut hi: i64 = f64::MAX.to_bits() as i64;
        while lo <= hi {
            // Java uses `>>>` so that the sum may wrap into the sign bit.
            let mid = (((lo as u64).wrapping_add(hi as u64)) >> 1) as i64;
            let sort_key = f64::from_bits(mid as u64);
            let mid_radius = SloppyMath::haversin_meters_from_sort_key(sort_key);
            if mid_radius == radius {
                return sort_key;
            } else if mid_radius > radius {
                hi = mid - 1;
            } else {
                lo = mid + 1;
            }
        }

        // not found: this is because a user can supply an arbitrary radius, one that we will never
        // calculate exactly via our haversin method.
        f64::from_bits(lo as u64)
    }

    /// Computes the relation between the provided box and a distance query.
    ///
    /// Equivalent to `GeoUtils.relate(...)`. Only works for boxes that do not
    /// cross the dateline.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `min_lon > max_lon`, as
    /// Java throws `IllegalArgumentException`.
    #[allow(clippy::too_many_arguments)]
    pub fn relate(
        min_lat: f64,
        max_lat: f64,
        min_lon: f64,
        max_lon: f64,
        lat: f64,
        lon: f64,
        distance_sort_key: f64,
        axis_lat: f64,
    ) -> Result<Relation> {
        if min_lon > max_lon {
            return Err(LuceneError::IllegalArgument(
                "Box crosses the dateline".to_string(),
            ));
        }

        if (lon < min_lon || lon > max_lon)
            && (axis_lat + Rectangle::AXISLAT_ERROR < min_lat
                || axis_lat - Rectangle::AXISLAT_ERROR > max_lat)
        {
            // circle not fully inside / crossing axis
            if SloppyMath::haversin_sort_key(lat, lon, min_lat, min_lon) > distance_sort_key
                && SloppyMath::haversin_sort_key(lat, lon, min_lat, max_lon) > distance_sort_key
                && SloppyMath::haversin_sort_key(lat, lon, max_lat, min_lon) > distance_sort_key
                && SloppyMath::haversin_sort_key(lat, lon, max_lat, max_lon) > distance_sort_key
            {
                // no points inside
                return Ok(Relation::CellOutsideQuery);
            }
        }

        if Self::within_90_lon_degrees(lon, min_lon, max_lon)
            && SloppyMath::haversin_sort_key(lat, lon, min_lat, min_lon) <= distance_sort_key
            && SloppyMath::haversin_sort_key(lat, lon, min_lat, max_lon) <= distance_sort_key
            && SloppyMath::haversin_sort_key(lat, lon, max_lat, min_lon) <= distance_sort_key
            && SloppyMath::haversin_sort_key(lat, lon, max_lat, max_lon) <= distance_sort_key
        {
            // we are fully enclosed, collect everything within this subtree
            return Ok(Relation::CellInsideQuery);
        }

        Ok(Relation::CellCrossesQuery)
    }

    /// Returns whether all points of `[min_lon, max_lon]` are within 90 degrees
    /// of `lon`.
    ///
    /// Equivalent to the package-private `GeoUtils.within90LonDegrees`.
    pub(crate) fn within_90_lon_degrees(lon: f64, min_lon: f64, max_lon: f64) -> bool {
        let mut lon = lon;
        if max_lon <= lon - 180.0 {
            lon -= 360.0;
        } else if min_lon >= lon + 180.0 {
            lon += 360.0;
        }
        max_lon - lon < 90.0 && lon - min_lon < 90.0
    }

    /// Returns a positive value when `a`, `b` and `c` are counter-clockwise, a
    /// negative value when clockwise, and zero when collinear.
    ///
    /// Equivalent to `GeoUtils.orient(...)`, the non-robust `Orient2D`
    /// predicate. Like Lucene, this does not apply the floating-point tricks
    /// that would make it exact.
    pub fn orient(ax: f64, ay: f64, bx: f64, by: f64, cx: f64, cy: f64) -> i32 {
        let v1 = (bx - ax) * (cy - ay);
        let v2 = (cx - ax) * (by - ay);
        if v1 > v2 {
            1
        } else if v1 < v2 {
            -1
        } else {
            0
        }
    }

    /// Returns whether two line segments cross, excluding segments that merely
    /// terminate on each other.
    ///
    /// Equivalent to `GeoUtils.lineCrossesLine(...)`.
    #[allow(clippy::too_many_arguments)]
    pub fn line_crosses_line(
        a1x: f64,
        a1y: f64,
        b1x: f64,
        b1y: f64,
        a2x: f64,
        a2y: f64,
        b2x: f64,
        b2y: f64,
    ) -> bool {
        Self::orient(a2x, a2y, b2x, b2y, a1x, a1y) * Self::orient(a2x, a2y, b2x, b2y, b1x, b1y) < 0
            && Self::orient(a1x, a1y, b1x, b1y, a2x, a2y)
                * Self::orient(a1x, a1y, b1x, b1y, b2x, b2y)
                < 0
    }

    /// Returns whether two line segments overlap each other.
    ///
    /// Equivalent to `GeoUtils.lineOverlapLine(...)`.
    #[allow(clippy::too_many_arguments)]
    pub fn line_overlap_line(
        a1x: f64,
        a1y: f64,
        b1x: f64,
        b1y: f64,
        a2x: f64,
        a2y: f64,
        b2x: f64,
        b2y: f64,
    ) -> bool {
        Self::orient(a2x, a2y, b2x, b2y, a1x, a1y) == 0
            && Self::orient(a2x, a2y, b2x, b2y, b1x, b1y) == 0
            && Self::orient(a1x, a1y, b1x, b1y, a2x, a2y) == 0
            && Self::orient(a1x, a1y, b1x, b1y, b2x, b2y) == 0
    }

    /// Returns whether two line segments cross, boundaries included, so that
    /// segments terminating on each other count as crossing.
    ///
    /// Equivalent to `GeoUtils.lineCrossesLineWithBoundary(...)`. Use
    /// [`GeoUtils::line_crosses_line`] to exclude those cases.
    #[allow(clippy::too_many_arguments)]
    pub fn line_crosses_line_with_boundary(
        a1x: f64,
        a1y: f64,
        b1x: f64,
        b1y: f64,
        a2x: f64,
        a2y: f64,
        b2x: f64,
        b2y: f64,
    ) -> bool {
        Self::orient(a2x, a2y, b2x, b2y, a1x, a1y) * Self::orient(a2x, a2y, b2x, b2y, b1x, b1y) <= 0
            && Self::orient(a1x, a1y, b1x, b1y, a2x, a2y)
                * Self::orient(a1x, a1y, b1x, b1y, b2x, b2y)
                <= 0
    }
}

impl XYEncodingUtils {
    /// Converts an array of `f32` coordinates to `f64`.
    ///
    /// Equivalent to `XYEncodingUtils.floatArrayToDoubleArray(float[])`.
    pub fn float_array_to_double_array(f: &[f32]) -> Vec<f64> {
        f.iter().map(|&v| f64::from(v)).collect()
    }
}

/// Returns the smallest float above `value`, as `Math.nextUp(float)` does.
pub(crate) fn next_up_f32(value: f32) -> f32 {
    if value.is_nan() || value == f32::INFINITY {
        return value;
    }
    if value == 0.0 {
        return f32::from_bits(1);
    }
    let bits = value.to_bits();
    f32::from_bits(if value > 0.0 { bits + 1 } else { bits - 1 })
}
