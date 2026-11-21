//! Port of `lucene/core/src/test/org/apache/lucene/document/BaseShapeEncodingTestCase.java`:
//! base shape encoding class for testing encoding of tessellated `XYShape`
//! and `LatLonShape`.
//!
//! The abstract class becomes the trait [`ShapeEncodingCase`] (its abstract
//! methods) and generic functions (its test methods), which each concrete
//! class lists in its declaration.

#![allow(unused_assignments, clippy::too_many_arguments)]

use std::sync::Arc;

use randomizedtesting::junit::Assert;
use rucene::document::shape_field::{DecodedTriangle, ShapeField, TriangleType, BYTES};
use rucene::error::Result;
use rucene::geo::component2d::Component2D;
use rucene::geo::GeoUtils;

use crate::core_support::assert_equals;

/// The abstract methods of `BaseShapeEncodingTestCase`.
pub trait ShapeEncodingCase {
    /// The polygon type `nextPolygon()` returns.
    type Polygon;

    /// `encodeX(x)`.
    fn encode_x(&self, x: f64) -> Result<i32>;
    /// `decodeX(x)`.
    fn decode_x(&self, x: i32) -> f64;
    /// `encodeY(y)`.
    fn encode_y(&self, y: f64) -> Result<i32>;
    /// `decodeY(y)`.
    fn decode_y(&self, y: i32) -> f64;
    /// `nextX()`.
    fn next_x(&self) -> f64;
    /// `nextY()`.
    fn next_y(&self) -> f64;
    /// `nextPolygon()`.
    fn next_polygon(&self) -> Self::Polygon;
    /// `createPolygon2D(polygon)`.
    fn create_polygon_2d(&self, polygon: &Self::Polygon) -> Result<Arc<dyn Component2D>>;
}

/// The inherited test methods of `BaseShapeEncodingTestCase`, bound to the
/// concrete case `C`.
pub fn base_methods<C: ShapeEncodingCase + Default + 'static>(
) -> Vec<randomizedtesting::jdk::reflect::Method> {
    fn t<C: ShapeEncodingCase + Default>(f: fn(&C) -> Result<()>) -> impl Fn() -> Result<()> {
        move || f(&C::default())
    }
    macro_rules! m {
        ($name:literal, $f:ident) => {
            crate::core_support::test_closure($name, t::<C>($f::<C>))
        };
    }
    vec![
        m!(
            "testPolygonEncodingMinLatMinLon",
            test_polygon_encoding_min_lat_min_lon
        ),
        m!(
            "testPolygonEncodingMinLatMaxLon",
            test_polygon_encoding_min_lat_max_lon
        ),
        m!(
            "testPolygonEncodingMaxLatMaxLon",
            test_polygon_encoding_max_lat_max_lon
        ),
        m!(
            "testPolygonEncodingMaxLatMinLon",
            test_polygon_encoding_max_lat_min_lon
        ),
        m!(
            "testPolygonEncodingMinLatMinLonMaxLatMaxLonBelow",
            test_polygon_encoding_min_lat_min_lon_max_lat_max_lon_below
        ),
        m!(
            "testPolygonEncodingMinLatMinLonMaxLatMaxLonAbove",
            test_polygon_encoding_min_lat_min_lon_max_lat_max_lon_above
        ),
        m!(
            "testPolygonEncodingMinLatMaxLonMaxLatMinLonBelow",
            test_polygon_encoding_min_lat_max_lon_max_lat_min_lon_below
        ),
        m!(
            "testPolygonEncodingMinLatMaxLonMaxLatMinLonAbove",
            test_polygon_encoding_min_lat_max_lon_max_lat_min_lon_above
        ),
        m!(
            "testPolygonEncodingAllSharedAbove",
            test_polygon_encoding_all_shared_above
        ),
        m!(
            "testPolygonEncodingAllSharedBelow",
            test_polygon_encoding_all_shared_below
        ),
        m!("testPointEncoding", test_point_encoding),
        m!("testLineEncodingSameLat", test_line_encoding_same_lat),
        m!("testLineEncodingSameLon", test_line_encoding_same_lon),
        m!("testLineEncoding", test_line_encoding),
        m!("testRandomPointEncoding", test_random_point_encoding),
        m!("testRandomLineEncoding", test_random_line_encoding),
        m!("testRandomPolygonEncoding", test_random_polygon_encoding),
        m!("testDegeneratedTriangle", test_degenerated_triangle),
    ]
}

/// Port of `testPolygonEncodingMinLatMinLon`.
pub fn test_polygon_encoding_min_lat_min_lon<C: ShapeEncodingCase>(c: &C) -> Result<()> {
    let ay: f64 = 0.0;
    let ax: f64 = 0.0;
    let by: f64 = 1.0;
    let blon: f64 = 2.0;
    let cy: f64 = 2.0;
    let cx: f64 = 1.0;
    let ay_enc = c.encode_y(ay)?;
    let ax_enc = c.encode_x(ax)?;
    let by_enc = c.encode_y(by)?;
    let bx_enc = c.encode_x(blon)?;
    let cy_enc = c.encode_y(cy)?;
    let cx_enc = c.encode_x(cx)?;
    verify_encoding_permutations(ay_enc, ax_enc, by_enc, bx_enc, cy_enc, cx_enc)?;
    let mut b = vec![0u8; 7 * BYTES];
    ShapeField::encode_triangle(
        &mut b, ay_enc, ax_enc, true, by_enc, bx_enc, true, cy_enc, cx_enc, true,
    )?;
    let mut encoded = DecodedTriangle::default();
    encoded = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded.a_y, &ay_enc);
    assert_equals(&encoded.a_x, &ax_enc);
    assert_equals(&encoded.b_y, &by_enc);
    assert_equals(&encoded.b_x, &bx_enc);
    assert_equals(&encoded.c_y, &cy_enc);
    assert_equals(&encoded.c_x, &cx_enc);
    Ok(())
}
// One shared point with MBR -> MinLat, MaxLon

/// Port of `testPolygonEncodingMinLatMaxLon`.
pub fn test_polygon_encoding_min_lat_max_lon<C: ShapeEncodingCase>(c: &C) -> Result<()> {
    let ay: f64 = 1.0;
    let ax: f64 = 0.0;
    let by: f64 = 0.0;
    let blon: f64 = 2.0;
    let cy: f64 = 2.0;
    let cx: f64 = 1.0;
    let ay_enc = c.encode_y(ay)?;
    let ax_enc = c.encode_x(ax)?;
    let by_enc = c.encode_y(by)?;
    let bx_enc = c.encode_x(blon)?;
    let cy_enc = c.encode_y(cy)?;
    let cx_enc = c.encode_x(cx)?;
    verify_encoding_permutations(ay_enc, ax_enc, by_enc, bx_enc, cy_enc, cx_enc)?;
    let mut b = vec![0u8; 7 * BYTES];
    ShapeField::encode_triangle(
        &mut b, ay_enc, ax_enc, true, by_enc, bx_enc, true, cy_enc, cx_enc, true,
    )?;
    let mut encoded = DecodedTriangle::default();
    encoded = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded.a_y, &ay_enc);
    assert_equals(&encoded.a_x, &ax_enc);
    assert_equals(&encoded.b_y, &by_enc);
    assert_equals(&encoded.b_x, &bx_enc);
    assert_equals(&encoded.c_y, &cy_enc);
    assert_equals(&encoded.c_x, &cx_enc);
    Ok(())
}
// One shared point with MBR -> MaxLat, MaxLon

/// Port of `testPolygonEncodingMaxLatMaxLon`.
pub fn test_polygon_encoding_max_lat_max_lon<C: ShapeEncodingCase>(c: &C) -> Result<()> {
    let ay: f64 = 1.0;
    let ax: f64 = 0.0;
    let by: f64 = 2.0;
    let blon: f64 = 2.0;
    let cy: f64 = 0.0;
    let cx: f64 = 1.0;
    let ay_enc = c.encode_y(ay)?;
    let ax_enc = c.encode_x(ax)?;
    let by_enc = c.encode_y(cy)?;
    let bx_enc = c.encode_x(cx)?;
    let cy_enc = c.encode_y(by)?;
    let cx_enc = c.encode_x(blon)?;
    verify_encoding_permutations(ay_enc, ax_enc, by_enc, bx_enc, cy_enc, cx_enc)?;
    let mut b = vec![0u8; 7 * BYTES];
    ShapeField::encode_triangle(
        &mut b, ay_enc, ax_enc, true, by_enc, bx_enc, true, cy_enc, cx_enc, true,
    )?;
    let mut encoded = DecodedTriangle::default();
    encoded = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded.a_y, &ay_enc);
    assert_equals(&encoded.a_x, &ax_enc);
    assert_equals(&encoded.b_y, &by_enc);
    assert_equals(&encoded.b_x, &bx_enc);
    assert_equals(&encoded.c_y, &cy_enc);
    assert_equals(&encoded.c_x, &cx_enc);
    Ok(())
}
// One shared point with MBR -> MaxLat, MinLon

/// Port of `testPolygonEncodingMaxLatMinLon`.
pub fn test_polygon_encoding_max_lat_min_lon<C: ShapeEncodingCase>(c: &C) -> Result<()> {
    let ay: f64 = 2.0;
    let ax: f64 = 0.0;
    let by: f64 = 1.0;
    let blon: f64 = 2.0;
    let cy: f64 = 0.0;
    let cx: f64 = 1.0;
    let ay_enc = c.encode_y(ay)?;
    let ax_enc = c.encode_x(ax)?;
    let by_enc = c.encode_y(cy)?;
    let bx_enc = c.encode_x(cx)?;
    let cy_enc = c.encode_y(by)?;
    let cx_enc = c.encode_x(blon)?;
    verify_encoding_permutations(ay_enc, ax_enc, by_enc, bx_enc, cy_enc, cx_enc)?;
    let mut b = vec![0u8; 7 * BYTES];
    ShapeField::encode_triangle(
        &mut b, ay_enc, ax_enc, true, by_enc, bx_enc, true, cy_enc, cx_enc, true,
    )?;
    let mut encoded = DecodedTriangle::default();
    encoded = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded.a_y, &ay_enc);
    assert_equals(&encoded.a_x, &ax_enc);
    assert_equals(&encoded.b_y, &by_enc);
    assert_equals(&encoded.b_x, &bx_enc);
    assert_equals(&encoded.c_y, &cy_enc);
    assert_equals(&encoded.c_x, &cx_enc);
    Ok(())
}
// Two shared point with MBR -> [MinLat, MinLon], [MaxLat, MaxLon], third point below

/// Port of `testPolygonEncodingMinLatMinLonMaxLatMaxLonBelow`.
pub fn test_polygon_encoding_min_lat_min_lon_max_lat_max_lon_below<C: ShapeEncodingCase>(
    c: &C,
) -> Result<()> {
    let ay: f64 = 0.0;
    let ax: f64 = 0.0;
    let by: f64 = 0.25;
    let blon: f64 = 0.75;
    let cy: f64 = 2.0;
    let cx: f64 = 2.0;
    let ay_enc = c.encode_y(ay)?;
    let ax_enc = c.encode_x(ax)?;
    let by_enc = c.encode_y(by)?;
    let bx_enc = c.encode_x(blon)?;
    let cy_enc = c.encode_y(cy)?;
    let cx_enc = c.encode_x(cx)?;
    verify_encoding_permutations(ay_enc, ax_enc, by_enc, bx_enc, cy_enc, cx_enc)?;
    let mut b = vec![0u8; 7 * BYTES];
    ShapeField::encode_triangle(
        &mut b, ay_enc, ax_enc, true, by_enc, bx_enc, true, cy_enc, cx_enc, true,
    )?;
    let mut encoded = DecodedTriangle::default();
    encoded = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded.a_y, &ay_enc);
    assert_equals(&encoded.a_x, &ax_enc);
    assert_equals(&encoded.b_y, &by_enc);
    assert_equals(&encoded.b_x, &bx_enc);
    assert_equals(&encoded.c_y, &cy_enc);
    assert_equals(&encoded.c_x, &cx_enc);
    Ok(())
}
// Two shared point with MBR -> [MinLat, MinLon], [MaxLat, MaxLon], third point above

/// Port of `testPolygonEncodingMinLatMinLonMaxLatMaxLonAbove`.
pub fn test_polygon_encoding_min_lat_min_lon_max_lat_max_lon_above<C: ShapeEncodingCase>(
    c: &C,
) -> Result<()> {
    let ay: f64 = 0.0;
    let ax: f64 = 0.0;
    let by: f64 = 2.0;
    let bx: f64 = 2.0;
    let cy: f64 = 1.75;
    let cx: f64 = 1.25;
    let ay_enc = c.encode_y(ay)?;
    let ax_enc = c.encode_x(ax)?;
    let by_enc = c.encode_y(by)?;
    let bx_enc = c.encode_x(bx)?;
    let cy_enc = c.encode_y(cy)?;
    let cx_enc = c.encode_x(cx)?;
    verify_encoding_permutations(ay_enc, ax_enc, by_enc, bx_enc, cy_enc, cx_enc)?;
    let mut b = vec![0u8; 7 * BYTES];
    ShapeField::encode_triangle(
        &mut b, ay_enc, ax_enc, true, by_enc, bx_enc, true, cy_enc, cx_enc, true,
    )?;
    let mut encoded = DecodedTriangle::default();
    encoded = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded.a_y, &ay_enc);
    assert_equals(&encoded.a_x, &ax_enc);
    assert_equals(&encoded.b_y, &by_enc);
    assert_equals(&encoded.b_x, &bx_enc);
    assert_equals(&encoded.c_y, &cy_enc);
    assert_equals(&encoded.c_x, &cx_enc);
    Ok(())
}
// Two shared point with MBR -> [MinLat, MaxLon], [MaxLat, MinLon], third point below

/// Port of `testPolygonEncodingMinLatMaxLonMaxLatMinLonBelow`.
pub fn test_polygon_encoding_min_lat_max_lon_max_lat_min_lon_below<C: ShapeEncodingCase>(
    c: &C,
) -> Result<()> {
    let ay: f64 = 8.0;
    let ax: f64 = 6.0;
    let by: f64 = 6.25;
    let bx: f64 = 6.75;
    let cy: f64 = 6.0;
    let cx: f64 = 8.0;
    let ay_enc = c.encode_y(ay)?;
    let ax_enc = c.encode_x(ax)?;
    let by_enc = c.encode_y(by)?;
    let bx_enc = c.encode_x(bx)?;
    let cy_enc = c.encode_y(cy)?;
    let cx_enc = c.encode_x(cx)?;
    verify_encoding_permutations(ay_enc, ax_enc, by_enc, bx_enc, cy_enc, cx_enc)?;
    let mut b = vec![0u8; 7 * BYTES];
    ShapeField::encode_triangle(
        &mut b, ay_enc, ax_enc, true, by_enc, bx_enc, true, cy_enc, cx_enc, true,
    )?;
    let mut encoded = DecodedTriangle::default();
    encoded = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded.a_y, &ay_enc);
    assert_equals(&encoded.a_x, &ax_enc);
    assert_equals(&encoded.b_y, &by_enc);
    assert_equals(&encoded.b_x, &bx_enc);
    assert_equals(&encoded.c_y, &cy_enc);
    assert_equals(&encoded.c_x, &cx_enc);
    Ok(())
}
// Two shared point with MBR -> [MinLat, MaxLon], [MaxLat, MinLon], third point above

/// Port of `testPolygonEncodingMinLatMaxLonMaxLatMinLonAbove`.
pub fn test_polygon_encoding_min_lat_max_lon_max_lat_min_lon_above<C: ShapeEncodingCase>(
    c: &C,
) -> Result<()> {
    let ay: f64 = 2.0;
    let ax: f64 = 0.0;
    let by: f64 = 0.0;
    let bx: f64 = 2.0;
    let cy: f64 = 1.75;
    let cx: f64 = 1.25;
    let ay_enc = c.encode_y(ay)?;
    let ax_enc = c.encode_x(ax)?;
    let by_enc = c.encode_y(by)?;
    let bx_enc = c.encode_x(bx)?;
    let cy_enc = c.encode_y(cy)?;
    let cx_enc = c.encode_x(cx)?;
    verify_encoding_permutations(ay_enc, ax_enc, by_enc, bx_enc, cy_enc, cx_enc)?;
    let mut b = vec![0u8; 7 * BYTES];
    ShapeField::encode_triangle(
        &mut b, ay_enc, ax_enc, true, by_enc, bx_enc, true, cy_enc, cx_enc, true,
    )?;
    let mut encoded = DecodedTriangle::default();
    encoded = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded.a_y, &ay_enc);
    assert_equals(&encoded.a_x, &ax_enc);
    assert_equals(&encoded.b_y, &by_enc);
    assert_equals(&encoded.b_x, &bx_enc);
    assert_equals(&encoded.c_y, &cy_enc);
    assert_equals(&encoded.c_x, &cx_enc);
    Ok(())
}
// all points shared with MBR

/// Port of `testPolygonEncodingAllSharedAbove`.
pub fn test_polygon_encoding_all_shared_above<C: ShapeEncodingCase>(c: &C) -> Result<()> {
    let ay: f64 = 0.0;
    let ax: f64 = 0.0;
    let by: f64 = 0.0;
    let bx: f64 = 2.0;
    let cy: f64 = 2.0;
    let cx: f64 = 2.0;
    let ay_enc = c.encode_y(ay)?;
    let ax_enc = c.encode_x(ax)?;
    let by_enc = c.encode_y(by)?;
    let bx_enc = c.encode_x(bx)?;
    let cy_enc = c.encode_y(cy)?;
    let cx_enc = c.encode_x(cx)?;
    verify_encoding_permutations(ay_enc, ax_enc, by_enc, bx_enc, cy_enc, cx_enc)?;
    let mut b = vec![0u8; 7 * BYTES];
    ShapeField::encode_triangle(
        &mut b, ay_enc, ax_enc, true, by_enc, bx_enc, true, cy_enc, cx_enc, true,
    )?;
    let mut encoded = DecodedTriangle::default();
    encoded = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded.a_y, &ay_enc);
    assert_equals(&encoded.a_x, &ax_enc);
    assert_equals(&encoded.b_y, &by_enc);
    assert_equals(&encoded.b_x, &bx_enc);
    assert_equals(&encoded.c_y, &cy_enc);
    assert_equals(&encoded.c_x, &cx_enc);
    Ok(())
}
// all points shared with MBR

/// Port of `testPolygonEncodingAllSharedBelow`.
pub fn test_polygon_encoding_all_shared_below<C: ShapeEncodingCase>(c: &C) -> Result<()> {
    let ay: f64 = 2.0;
    let ax: f64 = 0.0;
    let by: f64 = 0.0;
    let bx: f64 = 0.0;
    let cy: f64 = 2.0;
    let cx: f64 = 2.0;
    let ay_enc = c.encode_y(ay)?;
    let ax_enc = c.encode_x(ax)?;
    let by_enc = c.encode_y(by)?;
    let bx_enc = c.encode_x(bx)?;
    let cy_enc = c.encode_y(cy)?;
    let cx_enc = c.encode_x(cx)?;
    let mut b = vec![0u8; 7 * BYTES];
    ShapeField::encode_triangle(
        &mut b, ay_enc, ax_enc, true, by_enc, bx_enc, true, cy_enc, cx_enc, true,
    )?;
    let mut encoded = DecodedTriangle::default();
    encoded = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded.a_y, &ay_enc);
    assert_equals(&encoded.a_x, &ax_enc);
    assert_equals(&encoded.b_y, &by_enc);
    assert_equals(&encoded.b_x, &bx_enc);
    assert_equals(&encoded.c_y, &cy_enc);
    assert_equals(&encoded.c_x, &cx_enc);
    Ok(())
}
// [a,b,c] == [c,a,b] == [b,c,a] == [c,b,a] == [b,a,c] == [a,c,b]

/// `verifyEncodingPermutations(ayEnc, axEnc, byEnc, bxEnc, cyEnc, cxEnc)`: [a,b,c] == [c,a,b] == [b,c,a] == [c,b,a] == [b,a,c] == [a,c,b].
#[allow(clippy::too_many_arguments)]
pub fn verify_encoding_permutations(
    ay_enc: i32,
    ax_enc: i32,
    by_enc: i32,
    bx_enc: i32,
    cy_enc: i32,
    cx_enc: i32,
) -> Result<()> {
    // this is only valid when points are not co-planar
    // this is only valid when points are not co-planar
    Assert::assert_true(
        GeoUtils::orient(
            ay_enc as f64,
            ax_enc as f64,
            by_enc as f64,
            bx_enc as f64,
            cy_enc as f64,
            cx_enc as f64,
        ) != 0,
    );
    let mut b = vec![0u8; 7 * BYTES];
    // [a,b,c]
    ShapeField::encode_triangle(
        &mut b, ay_enc, ax_enc, true, by_enc, bx_enc, true, cy_enc, cx_enc, false,
    )?;
    let mut encoded_abc = DecodedTriangle::default();
    encoded_abc = ShapeField::decode_triangle(&b)?;
    // [c,a,b]
    ShapeField::encode_triangle(
        &mut b, cy_enc, cx_enc, false, ay_enc, ax_enc, true, by_enc, bx_enc, true,
    )?;
    let mut encoded_cab = DecodedTriangle::default();
    encoded_cab = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded_abc, &encoded_cab);
    // [b,c,a]
    ShapeField::encode_triangle(
        &mut b, by_enc, bx_enc, true, cy_enc, cx_enc, false, ay_enc, ax_enc, true,
    )?;
    let mut encoded_bca = DecodedTriangle::default();
    encoded_bca = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded_abc, &encoded_bca);
    // [c,b,a]
    ShapeField::encode_triangle(
        &mut b, cy_enc, cx_enc, true, by_enc, bx_enc, true, ay_enc, ax_enc, false,
    )?;
    let mut encoded_cba = DecodedTriangle::default();
    encoded_cba = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded_abc, &encoded_cba);
    // [b,a,c]
    ShapeField::encode_triangle(
        &mut b, by_enc, bx_enc, true, ay_enc, ax_enc, false, cy_enc, cx_enc, true,
    )?;
    let mut encoded_bac = DecodedTriangle::default();
    encoded_bac = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded_abc, &encoded_bac);
    // [a,c,b]
    ShapeField::encode_triangle(
        &mut b, ay_enc, ax_enc, false, cy_enc, cx_enc, true, by_enc, bx_enc, true,
    )?;
    let mut encoded_acb = DecodedTriangle::default();
    encoded_acb = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded_abc, &encoded_acb);
    Ok(())
}

/// Port of `testPointEncoding`.
pub fn test_point_encoding<C: ShapeEncodingCase>(c: &C) -> Result<()> {
    let lat: f64 = 45.0;
    let lon: f64 = 45.0;
    let lat_enc = c.encode_y(lat)?;
    let lon_enc = c.encode_x(lon)?;
    let mut b = vec![0u8; 7 * BYTES];
    ShapeField::encode_triangle(
        &mut b, lat_enc, lon_enc, true, lat_enc, lon_enc, true, lat_enc, lon_enc, true,
    )?;
    let mut encoded = DecodedTriangle::default();
    encoded = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded.a_y, &lat_enc);
    assert_equals(&encoded.a_x, &lon_enc);
    assert_equals(&encoded.b_y, &lat_enc);
    assert_equals(&encoded.b_x, &lon_enc);
    assert_equals(&encoded.c_y, &lat_enc);
    assert_equals(&encoded.c_x, &lon_enc);
    Ok(())
}

/// Port of `testLineEncodingSameLat`.
pub fn test_line_encoding_same_lat<C: ShapeEncodingCase>(c: &C) -> Result<()> {
    let lat: f64 = 2.0;
    let ax: f64 = 0.0;
    let bx: f64 = 2.0;
    let lat_enc = c.encode_y(lat)?;
    let ax_enc = c.encode_x(ax)?;
    let bx_enc = c.encode_x(bx)?;
    let mut b = vec![0u8; 7 * BYTES];
    ShapeField::encode_triangle(
        &mut b, lat_enc, ax_enc, true, lat_enc, bx_enc, true, lat_enc, ax_enc, true,
    )?;
    let mut encoded = DecodedTriangle::default();
    encoded = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded.a_y, &lat_enc);
    assert_equals(&encoded.a_x, &ax_enc);
    assert_equals(&encoded.b_y, &lat_enc);
    assert_equals(&encoded.b_x, &bx_enc);
    assert_equals(&encoded.c_y, &lat_enc);
    assert_equals(&encoded.c_x, &ax_enc);
    ShapeField::encode_triangle(
        &mut b, lat_enc, ax_enc, true, lat_enc, ax_enc, true, lat_enc, bx_enc, true,
    )?;
    encoded = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded.a_y, &lat_enc);
    assert_equals(&encoded.a_x, &ax_enc);
    assert_equals(&encoded.b_y, &lat_enc);
    assert_equals(&encoded.b_x, &bx_enc);
    assert_equals(&encoded.c_y, &lat_enc);
    assert_equals(&encoded.c_x, &ax_enc);
    ShapeField::encode_triangle(
        &mut b, lat_enc, bx_enc, true, lat_enc, ax_enc, true, lat_enc, ax_enc, true,
    )?;
    encoded = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded.a_y, &lat_enc);
    assert_equals(&encoded.a_x, &ax_enc);
    assert_equals(&encoded.b_y, &lat_enc);
    assert_equals(&encoded.b_x, &bx_enc);
    assert_equals(&encoded.c_y, &lat_enc);
    assert_equals(&encoded.c_x, &ax_enc);
    Ok(())
}

/// Port of `testLineEncodingSameLon`.
pub fn test_line_encoding_same_lon<C: ShapeEncodingCase>(c: &C) -> Result<()> {
    let ay: f64 = 0.0;
    let by: f64 = 2.0;
    let lon: f64 = 2.0;
    let ay_enc = c.encode_y(ay)?;
    let by_enc = c.encode_y(by)?;
    let lon_enc = c.encode_x(lon)?;
    let mut b = vec![0u8; 7 * BYTES];
    ShapeField::encode_triangle(
        &mut b, ay_enc, lon_enc, true, by_enc, lon_enc, true, ay_enc, lon_enc, true,
    )?;
    let mut encoded = DecodedTriangle::default();
    encoded = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded.a_y, &ay_enc);
    assert_equals(&encoded.a_x, &lon_enc);
    assert_equals(&encoded.b_y, &by_enc);
    assert_equals(&encoded.b_x, &lon_enc);
    assert_equals(&encoded.c_y, &ay_enc);
    assert_equals(&encoded.c_x, &lon_enc);
    ShapeField::encode_triangle(
        &mut b, ay_enc, lon_enc, true, ay_enc, lon_enc, true, by_enc, lon_enc, true,
    )?;
    encoded = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded.a_y, &ay_enc);
    assert_equals(&encoded.a_x, &lon_enc);
    assert_equals(&encoded.b_y, &by_enc);
    assert_equals(&encoded.b_x, &lon_enc);
    assert_equals(&encoded.c_y, &ay_enc);
    assert_equals(&encoded.c_x, &lon_enc);
    ShapeField::encode_triangle(
        &mut b, by_enc, lon_enc, true, ay_enc, lon_enc, true, ay_enc, lon_enc, true,
    )?;
    encoded = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded.a_y, &ay_enc);
    assert_equals(&encoded.a_x, &lon_enc);
    assert_equals(&encoded.b_y, &by_enc);
    assert_equals(&encoded.b_x, &lon_enc);
    assert_equals(&encoded.c_y, &ay_enc);
    assert_equals(&encoded.c_x, &lon_enc);
    Ok(())
}

/// Port of `testLineEncoding`.
pub fn test_line_encoding<C: ShapeEncodingCase>(c: &C) -> Result<()> {
    let ay: f64 = 0.0;
    let by: f64 = 2.0;
    let ax: f64 = 0.0;
    let bx: f64 = 2.0;
    let ay_enc = c.encode_y(ay)?;
    let by_enc = c.encode_y(by)?;
    let ax_enc = c.encode_x(ax)?;
    let bx_enc = c.encode_x(bx)?;
    let mut b = vec![0u8; 7 * BYTES];
    ShapeField::encode_triangle(
        &mut b, ay_enc, ax_enc, true, by_enc, bx_enc, true, ay_enc, ax_enc, true,
    )?;
    let mut encoded = DecodedTriangle::default();
    encoded = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded.a_y, &ay_enc);
    assert_equals(&encoded.a_x, &ax_enc);
    assert_equals(&encoded.b_y, &by_enc);
    assert_equals(&encoded.b_x, &bx_enc);
    assert_equals(&encoded.c_y, &ay_enc);
    assert_equals(&encoded.c_x, &ax_enc);
    ShapeField::encode_triangle(
        &mut b, ay_enc, ax_enc, true, ay_enc, ax_enc, true, by_enc, bx_enc, true,
    )?;
    encoded = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded.a_y, &ay_enc);
    assert_equals(&encoded.a_x, &ax_enc);
    assert_equals(&encoded.b_y, &by_enc);
    assert_equals(&encoded.b_x, &bx_enc);
    assert_equals(&encoded.c_y, &ay_enc);
    assert_equals(&encoded.c_x, &ax_enc);
    ShapeField::encode_triangle(
        &mut b, by_enc, bx_enc, true, ay_enc, ax_enc, true, ay_enc, ax_enc, true,
    )?;
    encoded = ShapeField::decode_triangle(&b)?;
    assert_equals(&encoded.a_y, &ay_enc);
    assert_equals(&encoded.a_x, &ax_enc);
    assert_equals(&encoded.b_y, &by_enc);
    assert_equals(&encoded.b_x, &bx_enc);
    assert_equals(&encoded.c_y, &ay_enc);
    assert_equals(&encoded.c_x, &ax_enc);
    Ok(())
}

/// Port of `testRandomPointEncoding`.
pub fn test_random_point_encoding<C: ShapeEncodingCase>(c: &C) -> Result<()> {
    let ay = c.next_y();
    let ax = c.next_x();
    verify_encoding(c, ay, ax, ay, ax, ay, ax)?;
    Ok(())
}

/// Port of `testRandomLineEncoding`.
pub fn test_random_line_encoding<C: ShapeEncodingCase>(c: &C) -> Result<()> {
    let ay = c.next_y();
    let ax = c.next_x();
    let by = c.next_y();
    let bx = c.next_x();
    verify_encoding(c, ay, ax, by, bx, ay, ax)?;
    Ok(())
}

/// Port of `testRandomPolygonEncoding`.
pub fn test_random_polygon_encoding<C: ShapeEncodingCase>(c: &C) -> Result<()> {
    let ay = c.next_y();
    let ax = c.next_x();
    let by = c.next_y();
    let bx = c.next_x();
    let cy = c.next_y();
    let cx = c.next_x();
    verify_encoding(c, ay, ax, by, bx, cy, cx)?;
    Ok(())
}

/// Port of `testDegeneratedTriangle`.
pub fn test_degenerated_triangle<C: ShapeEncodingCase>(c: &C) -> Result<()> {
    let ay: f64 = 1e-26;
    let ax: f64 = 0.0;
    let by: f64 = -1.0;
    let bx: f64 = 0.0;
    let cy: f64 = 1.0;
    let cx: f64 = 0.0;
    let ay_enc = c.encode_y(ay)?;
    let ax_enc = c.encode_x(ax)?;
    let by_enc = c.encode_y(by)?;
    let bx_enc = c.encode_x(bx)?;
    let cy_enc = c.encode_y(cy)?;
    let cx_enc = c.encode_x(cx)?;
    let mut b = vec![0u8; 7 * BYTES];
    ShapeField::encode_triangle(
        &mut b, ay_enc, ax_enc, true, by_enc, bx_enc, true, cy_enc, cx_enc, true,
    )?;
    let mut encoded = DecodedTriangle::default();
    encoded = ShapeField::decode_triangle(&b)?;
    Assert::assert_true(encoded.a_y == by_enc);
    Assert::assert_true(encoded.a_x == bx_enc);
    Assert::assert_true(encoded.b_y == cy_enc);
    Assert::assert_true(encoded.b_x == cx_enc);
    Assert::assert_true(encoded.c_y == ay_enc);
    Assert::assert_true(encoded.c_x == ax_enc);
    Ok(())
}

/// `verifyEncoding(ay, ax, by, bx, cy, cx)`.
pub fn verify_encoding<C: ShapeEncodingCase>(
    c: &C,
    ay: f64,
    ax: f64,
    by: f64,
    bx: f64,
    cy: f64,
    cx: f64,
) -> Result<()> {
    // encode triangle
    let original = [
        c.encode_x(ax)?,
        c.encode_y(ay)?,
        c.encode_x(bx)?,
        c.encode_y(by)?,
        c.encode_x(cx)?,
        c.encode_y(cy)?,
    ];
    let mut b = vec![0u8; 7 * BYTES];
    ShapeField::encode_triangle(
        &mut b,
        original[1],
        original[0],
        true,
        original[3],
        original[2],
        true,
        original[5],
        original[4],
        true,
    )?;
    let encoded = ShapeField::decode_triangle(&b)?;
    // quantize decoded triangle
    let encoded_quantize = [
        c.decode_x(encoded.a_x),
        c.decode_y(encoded.a_y),
        c.decode_x(encoded.b_x),
        c.decode_y(encoded.b_y),
        c.decode_x(encoded.c_x),
        c.decode_y(encoded.c_y),
    ];
    // quantize original and order it to reflect encoding
    let original_quantize = order_triangle(
        c,
        original[0],
        original[1],
        original[2],
        original[3],
        original[4],
        original[5],
    );
    // check spatial property
    for _i in 0..100 {
        let polygon_2d = c.create_polygon_2d(&c.next_polygon())?;
        let mut original_intersects = false;
        let mut encoded_intersects = false;
        let mut original_contains = false;
        let mut encoded_contains = false;
        let o = &original_quantize;
        let e = &encoded_quantize;
        match encoded.triangle_type {
            TriangleType::Point => {
                original_intersects = polygon_2d.contains(o[0], o[1]);
                encoded_intersects = polygon_2d.contains(e[0], e[1]);
                original_contains = polygon_2d.contains(o[0], o[1]);
                encoded_contains = polygon_2d.contains(e[0], e[1]);
            }
            TriangleType::Line => {
                original_intersects = polygon_2d.intersects_line_bbox(o[0], o[1], o[2], o[3]);
                encoded_intersects = polygon_2d.intersects_line_bbox(e[0], e[1], e[2], e[3]);
                original_contains = polygon_2d.contains_line_bbox(o[0], o[1], o[2], o[3]);
                encoded_contains = polygon_2d.contains_line_bbox(e[0], e[1], e[2], e[3]);
            }
            TriangleType::Triangle => {
                // Java compares the original quantized triangle with itself here.
                original_intersects =
                    polygon_2d.intersects_triangle_bbox(o[0], o[1], o[2], o[3], o[4], o[5]);
                encoded_intersects =
                    polygon_2d.intersects_triangle_bbox(o[0], o[1], o[2], o[3], o[4], o[5]);
                original_contains =
                    polygon_2d.contains_triangle_bbox(o[0], o[1], o[2], o[3], o[4], o[5]);
                encoded_contains =
                    polygon_2d.contains_triangle_bbox(o[0], o[1], o[2], o[3], o[4], o[5]);
            }
        }
        Assert::assert_true(original_intersects == encoded_intersects);
        Assert::assert_true(original_contains == encoded_contains);
    }
    Ok(())
}

/// `orderTriangle(aX, aY, bX, bY, cX, cY)`: quantizes the original and
/// orders it to reflect the encoding.
fn order_triangle<C: ShapeEncodingCase>(
    c: &C,
    a_x: i32,
    a_y: i32,
    b_x: i32,
    b_y: i32,
    c_x: i32,
    c_y: i32,
) -> [f64; 6] {
    let orientation = GeoUtils::orient(
        a_x as f64, a_y as f64, b_x as f64, b_y as f64, c_x as f64, c_y as f64,
    );
    if orientation == -1 {
        // we need to change the orientation if CW for triangles
        return [
            c.decode_x(c_x),
            c.decode_y(c_y),
            c.decode_x(b_x),
            c.decode_y(b_y),
            c.decode_x(a_x),
            c.decode_y(a_y),
        ];
    } else if a_x == b_x && a_y == b_y {
        if a_x != c_x || a_y != c_y {
            // not a point
            if a_x < c_x {
                return [
                    c.decode_x(a_x),
                    c.decode_y(a_y),
                    c.decode_x(c_x),
                    c.decode_y(c_y),
                    c.decode_x(a_x),
                    c.decode_y(a_y),
                ];
            } else {
                return [
                    c.decode_x(c_x),
                    c.decode_y(c_y),
                    c.decode_x(a_x),
                    c.decode_y(a_y),
                    c.decode_x(c_x),
                    c.decode_y(c_y),
                ];
            }
        }
    } else if (a_x == c_x && a_y == c_y) || (b_x == c_x && b_y == c_y) {
        if a_x < b_x {
            return [
                c.decode_x(a_x),
                c.decode_y(a_y),
                c.decode_x(b_x),
                c.decode_y(b_y),
                c.decode_x(a_x),
                c.decode_y(a_y),
            ];
        } else {
            return [
                c.decode_x(b_x),
                c.decode_y(b_y),
                c.decode_x(a_x),
                c.decode_y(a_y),
                c.decode_x(b_x),
                c.decode_y(b_y),
            ];
        }
    }
    [
        c.decode_x(a_x),
        c.decode_y(a_y),
        c.decode_x(b_x),
        c.decode_y(b_y),
        c.decode_x(c_x),
        c.decode_y(c_y),
    ]
}
