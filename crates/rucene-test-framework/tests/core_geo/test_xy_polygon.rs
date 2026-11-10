//! Port of `lucene/core/src/test/org/apache/lucene/geo/TestXYPolygon.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::geo::XYPolygon;
use rucene_test_framework::geo::shape_test_util;

use crate::core_support::{
    assert_contains, assert_equals, assert_not_equals, class, iae_message, test,
};
use crate::test_xy_line::arrays_equals;

/// `TestXYPolygon`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.geo.TestXYPolygon",
        vec![
            test("testPolygonNullPolyLats", test_polygon_null_poly_lats),
            test("testPolygonNullPolyLons", test_polygon_null_poly_lons),
            test("testPolygonLine", test_polygon_line),
            test("testPolygonBogus", test_polygon_bogus),
            test("testPolygonNotClosed", test_polygon_not_closed),
            test("testPolygonNaN", test_polygon_nan),
            test(
                "testPolygonPositiveInfinite",
                test_polygon_positive_infinite,
            ),
            test(
                "testPolygonNegativeInfinite",
                test_polygon_negative_infinite,
            ),
            test("testEqualsAndHashCode", test_equals_and_hash_code),
        ],
    )
}

/// null x not allowed
///
/// Port of `TestXYPolygon.testPolygonNullPolyLats`.
fn test_polygon_null_poly_lats() -> Result<()> {
    let expected =
        iae_message(|| XYPolygon::new(None, vec![-66.0, -65.0, -65.0, -66.0, -66.0], vec![]));
    Assert::assert_true(expected.contains("x must not be null"));
    Ok(())
}

/// null y not allowed
///
/// Port of `TestXYPolygon.testPolygonNullPolyLons`.
fn test_polygon_null_poly_lons() -> Result<()> {
    let expected = iae_message(|| XYPolygon::new(vec![18.0, 18.0, 19.0, 19.0, 18.0], None, vec![]));
    Assert::assert_true(expected.contains("y must not be null"));
    Ok(())
}

/// polygon needs at least 3 vertices
///
/// Port of `TestXYPolygon.testPolygonLine`.
fn test_polygon_line() -> Result<()> {
    let expected =
        iae_message(|| XYPolygon::new(vec![18.0, 18.0, 18.0], vec![-66.0, -65.0, -66.0], vec![]));
    Assert::assert_true(expected.contains("at least 4 polygon points required"));
    Ok(())
}

/// polygon needs same number of latitudes as longitudes
///
/// Port of `TestXYPolygon.testPolygonBogus`.
fn test_polygon_bogus() -> Result<()> {
    let expected = iae_message(|| {
        XYPolygon::new(
            vec![18.0, 18.0, 19.0, 19.0],
            vec![-66.0, -65.0, -65.0, -66.0, -66.0],
            vec![],
        )
    });
    Assert::assert_true(expected.contains("must be equal length"));
    Ok(())
}

/// polygon must be closed
///
/// Port of `TestXYPolygon.testPolygonNotClosed`.
fn test_polygon_not_closed() -> Result<()> {
    let expected = iae_message(|| {
        XYPolygon::new(
            vec![18.0, 18.0, 19.0, 19.0, 19.0],
            vec![-66.0, -65.0, -65.0, -66.0, -67.0],
            vec![],
        )
    });
    assert_contains(&expected, "it must close itself");
    Ok(())
}

/// polygon values cannot be NaN
///
/// Port of `TestXYPolygon.testPolygonNaN`.
fn test_polygon_nan() -> Result<()> {
    let expected = iae_message(|| {
        XYPolygon::new(
            vec![18.0, 18.0, 19.0, f32::NAN, 18.0],
            vec![-66.0, -65.0, -65.0, -66.0, -66.0],
            vec![],
        )
    });
    assert_contains(&expected, "invalid value NaN");
    Ok(())
}

/// polygon values cannot be finite
///
/// Port of `TestXYPolygon.testPolygonPositiveInfinite`.
fn test_polygon_positive_infinite() -> Result<()> {
    let expected = iae_message(|| {
        XYPolygon::new(
            vec![18.0, 18.0, 19.0, 19.0, 18.0],
            vec![-66.0, f32::INFINITY, -65.0, -66.0, -66.0],
            vec![],
        )
    });
    assert_contains(&expected, "invalid value Inf");
    Ok(())
}

/// polygon values cannot be finite
///
/// Port of `TestXYPolygon.testPolygonNegativeInfinite`.
fn test_polygon_negative_infinite() -> Result<()> {
    let expected = iae_message(|| {
        XYPolygon::new(
            vec![18.0, 18.0, 19.0, 19.0, 18.0],
            vec![-66.0, -65.0, -65.0, f32::NEG_INFINITY, -66.0],
            vec![],
        )
    });
    assert_contains(&expected, "invalid value -Inf");
    Ok(())
}

/// equals and hashcode
///
/// Port of `TestXYPolygon.testEqualsAndHashCode`.
fn test_equals_and_hash_code() -> Result<()> {
    let polygon = shape_test_util::next_polygon();
    let copy = XYPolygon::new(
        polygon.get_polyx().to_vec(),
        polygon.get_polyy().to_vec(),
        polygon.get_holes().to_vec(),
    )?;
    assert_equals(&polygon, &copy);
    assert_equals(&polygon.hash_code(), &copy.hash_code());
    let other_polygon = shape_test_util::next_polygon();
    if !arrays_equals(polygon.get_polyx(), other_polygon.get_polyx())
        || !arrays_equals(polygon.get_polyy(), other_polygon.get_polyy())
        || polygon.get_holes() != other_polygon.get_holes()
    {
        assert_not_equals(&polygon, &other_polygon);
        assert_not_equals(&polygon.hash_code(), &other_polygon.hash_code());
    } else {
        assert_equals(&polygon, &other_polygon);
        assert_equals(&polygon.hash_code(), &other_polygon.hash_code());
    }
    Ok(())
}
