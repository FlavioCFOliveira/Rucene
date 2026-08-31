//! Coordinate encodings ported from `org.apache.lucene.geo`.

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::geo::component2d::Component2D;
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
    /// Creates a predicate that answers whether an encoded point lies within
    /// `radius_meters` of `(lat, lon)`.
    ///
    /// Equivalent to `GeoEncodingUtils.createDistancePredicate(double, double,
    /// double)`. The predicate pre-computes the relation of every cell of a
    /// coarse grid to the query circle, so that most points are answered by one
    /// array lookup and only the cells the circle crosses need the distance.
    ///
    /// # Errors
    ///
    /// Propagates whatever building the bounding box or encoding it raises.
    pub fn create_distance_predicate(
        lat: f64,
        lon: f64,
        radius_meters: f64,
    ) -> Result<DistancePredicate> {
        let bounding_box = Rectangle::from_point_distance(lat, lon, radius_meters)?;
        let axis_lat = Rectangle::axis_lat(lat, radius_meters);
        let distance_sort_key = GeoUtils::distance_query_sort_key(radius_meters);
        let sub_boxes = Self::create_sub_boxes(
            bounding_box.min_lat(),
            bounding_box.max_lat(),
            bounding_box.min_lon(),
            bounding_box.max_lon(),
            &|min_lat, max_lat, min_lon, max_lon| {
                GeoUtils::relate(
                    min_lat,
                    max_lat,
                    min_lon,
                    max_lon,
                    lat,
                    lon,
                    distance_sort_key,
                    axis_lat,
                )
            },
        )?;
        Ok(DistancePredicate {
            grid: sub_boxes,
            lat,
            lon,
            distance_key: distance_sort_key,
        })
    }

    /// Creates a predicate that answers whether an encoded point lies within a
    /// geometry.
    ///
    /// Equivalent to `GeoEncodingUtils.createComponentPredicate(Component2D)`,
    /// which works the same way as
    /// [`create_distance_predicate`](Self::create_distance_predicate).
    ///
    /// # Errors
    ///
    /// Propagates whatever encoding the geometry's bounds raises.
    pub fn create_component_predicate(tree: Arc<dyn Component2D>) -> Result<Component2DPredicate> {
        let bounds = (
            tree.get_min_y(),
            tree.get_max_y(),
            tree.get_min_x(),
            tree.get_max_x(),
        );
        let relate_tree = Arc::clone(&tree);
        let sub_boxes = Self::create_sub_boxes(
            bounds.0,
            bounds.1,
            bounds.2,
            bounds.3,
            &|min_lat, max_lat, min_lon, max_lon| {
                Ok(relate_tree.relate(min_lon, max_lon, min_lat, max_lat))
            },
        )?;
        Ok(Component2DPredicate {
            grid: sub_boxes,
            tree,
        })
    }

    /// Builds the coarse grid both predicates test against.
    ///
    /// Equivalent to the private `GeoEncodingUtils.createSubBoxes(double,
    /// double, double, double, Function<Rectangle, Relation>)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever encoding a coordinate raises.
    fn create_sub_boxes(
        shape_min_lat: f64,
        shape_max_lat: f64,
        shape_min_lon: f64,
        shape_max_lon: f64,
        box_to_relation: &dyn Fn(f64, f64, f64, f64) -> Result<Relation>,
    ) -> Result<Grid> {
        let min_lat = Self::encode_latitude_ceil(shape_min_lat)?;
        let max_lat = Self::encode_latitude(shape_max_lat)?;
        let min_lon = Self::encode_longitude_ceil(shape_min_lon)?;
        let max_lon = Self::encode_longitude(shape_max_lon)?;

        if max_lat < min_lat || (shape_max_lon >= shape_min_lon && max_lon < min_lon) {
            // The box cannot match any quantized point.
            return Ok(Grid {
                lat_shift: 1,
                lon_shift: 1,
                lat_base: 0,
                lon_base: 0,
                max_lat_delta: 0,
                max_lon_delta: 0,
                relations: Vec::new(),
            });
        }

        let min_lat2 = i64::from(min_lat) - i64::from(i32::MIN);
        let max_lat2 = i64::from(max_lat) - i64::from(i32::MIN);
        let lat_shift = Self::compute_shift(min_lat2, max_lat2);
        let lat_base = (min_lat2 >> lat_shift) as i32;
        let max_lat_delta = (max_lat2 >> lat_shift) as i32 - lat_base + 1;
        debug_assert!(max_lat_delta > 0);

        let min_lon2 = i64::from(min_lon) - i64::from(i32::MIN);
        let mut max_lon2 = i64::from(max_lon) - i64::from(i32::MIN);
        if shape_max_lon < shape_min_lon {
            // Crosses the dateline: wrap.
            max_lon2 += 1i64 << 32;
        }
        let lon_shift = Self::compute_shift(min_lon2, max_lon2);
        let lon_base = (min_lon2 >> lon_shift) as i32;
        let max_lon_delta = (max_lon2 >> lon_shift) as i32 - lon_base + 1;
        debug_assert!(max_lon_delta > 0);

        let mut relations = vec![0u8; (max_lat_delta * max_lon_delta) as usize];
        for i in 0..max_lat_delta {
            for j in 0..max_lon_delta {
                let box_min_lat = ((lat_base + i) << lat_shift).wrapping_add(i32::MIN);
                let box_min_lon = ((lon_base + j) << lon_shift).wrapping_add(i32::MIN);
                let box_max_lat = box_min_lat + (1 << lat_shift) - 1;
                let box_max_lon = box_min_lon + (1 << lon_shift) - 1;
                let relation = box_to_relation(
                    Self::decode_latitude(box_min_lat),
                    Self::decode_latitude(box_max_lat),
                    Self::decode_longitude(box_min_lon),
                    Self::decode_longitude(box_max_lon),
                )?;
                relations[(i * max_lon_delta + j) as usize] = relation_ordinal(relation);
            }
        }

        Ok(Grid {
            lat_shift,
            lon_shift,
            lat_base,
            lon_base,
            max_lat_delta,
            max_lon_delta,
            relations,
        })
    }

    /// Returns the smallest shift for which `(b >> shift) - (a >> shift)` is
    /// under [`GRID_ARITY`].
    ///
    /// Equivalent to the private `GeoEncodingUtils.computeShift(long, long)`.
    /// The shift is at least one so that, working with unsigned values through
    /// `value - i32::MIN`, the sign bit is already clear and a comparison of
    /// the shifted values needs no further care.
    fn compute_shift(a: i64, b: i64) -> u32 {
        debug_assert!(a <= b);
        let mut shift = 1u32;
        loop {
            let delta = (b >> shift) - (a >> shift);
            if delta >= 0 && delta < i64::from(GRID_ARITY) {
                return shift;
            }
            shift += 1;
        }
    }

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

/// How many cells a grid dimension may hold.
///
/// Equivalent to `GeoEncodingUtils.Grid.ARITY`.
const GRID_ARITY: i32 = 64;

/// Renders a [`Relation`] the way Java's `Relation.ordinal()` does, which is
/// how the grid stores it.
fn relation_ordinal(relation: Relation) -> u8 {
    match relation {
        Relation::CellInsideQuery => 0,
        Relation::CellOutsideQuery => 1,
        Relation::CellCrossesQuery => 2,
    }
}

/// The coarse grid a [`DistancePredicate`] or [`Component2DPredicate`] tests
/// against.
///
/// Equivalent to the private `GeoEncodingUtils.Grid`.
#[derive(Clone, Debug)]
pub struct Grid {
    lat_shift: u32,
    lon_shift: u32,
    lat_base: i32,
    lon_base: i32,
    max_lat_delta: i32,
    max_lon_delta: i32,
    relations: Vec<u8>,
}

impl Grid {
    /// Returns the cell `(lat, lon)` falls in, or `None` when the point is
    /// outside the grid entirely.
    ///
    /// This is the head both predicates' `test` share.
    fn cell_relation(&self, lat: i32, lon: i32) -> Option<u8> {
        let lat2 = ((lat.wrapping_sub(i32::MIN)) as u32 >> self.lat_shift) as i32;
        if lat2 < self.lat_base || lat2 - self.lat_base >= self.max_lat_delta {
            return None;
        }
        let mut lon2 = ((lon.wrapping_sub(i32::MIN)) as u32 >> self.lon_shift) as i32;
        if lon2 < self.lon_base {
            // Wrap.
            lon2 += 1 << (32 - self.lon_shift);
        }
        debug_assert!(lon2 - self.lon_base >= 0);
        if lon2 - self.lon_base >= self.max_lon_delta {
            return None;
        }
        Some(
            self.relations
                [((lat2 - self.lat_base) * self.max_lon_delta + (lon2 - self.lon_base)) as usize],
        )
    }
}

/// Answers whether an encoded point lies within a distance of another.
///
/// Equivalent to `GeoEncodingUtils.DistancePredicate`.
#[derive(Clone, Debug)]
pub struct DistancePredicate {
    grid: Grid,
    lat: f64,
    lon: f64,
    distance_key: f64,
}

impl DistancePredicate {
    /// Returns whether the point lies within the query distance.
    ///
    /// Equivalent to `DistancePredicate.test(int, int)`, which operates
    /// directly on the encoded representation.
    pub fn test(&self, lat: i32, lon: i32) -> bool {
        let Some(relation) = self.grid.cell_relation(lat, lon) else {
            return false;
        };
        if relation == relation_ordinal(Relation::CellCrossesQuery) {
            SloppyMath::haversin_sort_key(
                GeoEncodingUtils::decode_latitude(lat),
                GeoEncodingUtils::decode_longitude(lon),
                self.lat,
                self.lon,
            ) <= self.distance_key
        } else {
            relation == relation_ordinal(Relation::CellInsideQuery)
        }
    }
}

/// Answers whether an encoded point lies within a geometry.
///
/// Equivalent to `GeoEncodingUtils.Component2DPredicate`.
#[derive(Clone)]
pub struct Component2DPredicate {
    grid: Grid,
    tree: Arc<dyn Component2D>,
}

impl std::fmt::Debug for Component2DPredicate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Component2DPredicate")
            .field("grid", &self.grid)
            .finish_non_exhaustive()
    }
}

impl Component2DPredicate {
    /// Returns whether the point lies within the geometry.
    ///
    /// Equivalent to `Component2DPredicate.test(int, int)`.
    pub fn test(&self, lat: i32, lon: i32) -> bool {
        let Some(relation) = self.grid.cell_relation(lat, lon) else {
            return false;
        };
        if relation == relation_ordinal(Relation::CellCrossesQuery) {
            self.tree.contains(
                GeoEncodingUtils::decode_longitude(lon),
                GeoEncodingUtils::decode_latitude(lat),
            )
        } else {
            relation == relation_ordinal(Relation::CellInsideQuery)
        }
    }
}
