//! Port of `lucene/core/src/test/org/apache/lucene/geo/TestXYLine.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::geo::XYLine;
use rucene_test_framework::geo::shape_test_util;

use crate::core_support::{
    assert_contains, assert_equals, assert_not_equals, class, iae_message, test,
};

/// `TestXYLine`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.geo.TestXYLine",
        vec![
            test("testLineNullXs", test_line_null_xs),
            test("testPolygonNullYs", test_polygon_null_ys),
            test("testLineEnoughPoints", test_line_enough_points),
            test("testLinesBogus", test_lines_bogus),
            test("testLineNaN", test_line_nan),
            test("testLinePositiveInfinite", test_line_positive_infinite),
            test("testLineNegativeInfinite", test_line_negative_infinite),
            test("testEqualsAndHashCode", test_equals_and_hash_code),
        ],
    )
}

/// null x not allowed
///
/// Port of `TestXYLine.testLineNullXs`.
fn test_line_null_xs() -> Result<()> {
    let expected = iae_message(|| XYLine::new(None, vec![-66.0, -65.0, -65.0, -66.0, -66.0]));
    Assert::assert_true(expected.contains("x must not be null"));
    Ok(())
}

/// null y not allowed
///
/// Port of `TestXYLine.testPolygonNullYs`.
fn test_polygon_null_ys() -> Result<()> {
    let expected = iae_message(|| XYLine::new(vec![18.0, 18.0, 19.0, 19.0, 18.0], None));
    Assert::assert_true(expected.contains("y must not be null"));
    Ok(())
}

/// needs at least 3 vertices
///
/// Port of `TestXYLine.testLineEnoughPoints`.
fn test_line_enough_points() -> Result<()> {
    let expected = iae_message(|| XYLine::new(vec![18.0], vec![-66.0]));
    Assert::assert_true(expected.contains("at least 2 line points required"));
    Ok(())
}

/// lines needs same number of x as y
///
/// Port of `TestXYLine.testLinesBogus`.
fn test_lines_bogus() -> Result<()> {
    let expected = iae_message(|| {
        XYLine::new(
            vec![18.0, 18.0, 19.0, 19.0],
            vec![-66.0, -65.0, -65.0, -66.0, -66.0],
        )
    });
    Assert::assert_true(expected.contains("must be equal length"));
    Ok(())
}

/// line values cannot be NaN
///
/// Port of `TestXYLine.testLineNaN`.
fn test_line_nan() -> Result<()> {
    let expected = iae_message(|| {
        XYLine::new(
            vec![18.0, 18.0, 19.0, f32::NAN, 18.0],
            vec![-66.0, -65.0, -65.0, -66.0, -66.0],
        )
    });
    assert_contains(&expected, "invalid value NaN");
    Ok(())
}

/// line values cannot be finite
///
/// Port of `TestXYLine.testLinePositiveInfinite`.
fn test_line_positive_infinite() -> Result<()> {
    let expected = iae_message(|| {
        XYLine::new(
            vec![18.0, 18.0, 19.0, 19.0, 18.0],
            vec![-66.0, f32::INFINITY, -65.0, -66.0, -66.0],
        )
    });
    assert_contains(&expected, "invalid value Inf");
    Ok(())
}

/// line values cannot be finite
///
/// Port of `TestXYLine.testLineNegativeInfinite`.
fn test_line_negative_infinite() -> Result<()> {
    let expected = iae_message(|| {
        XYLine::new(
            vec![18.0, 18.0, 19.0, 19.0, 18.0],
            vec![-66.0, -65.0, -65.0, f32::NEG_INFINITY, -66.0],
        )
    });
    assert_contains(&expected, "invalid value -Inf");
    Ok(())
}

/// equals and hashcode
///
/// Port of `TestXYLine.testEqualsAndHashCode`.
fn test_equals_and_hash_code() -> Result<()> {
    let line = shape_test_util::next_line();
    let copy = XYLine::new(line.get_x().to_vec(), line.get_y().to_vec())?;
    assert_equals(&line, &copy);
    assert_equals(&line.hash_code(), &copy.hash_code());
    let other_line = shape_test_util::next_line();
    if !arrays_equals(line.get_x(), other_line.get_x())
        || !arrays_equals(line.get_y(), other_line.get_y())
    {
        assert_not_equals(&line, &other_line);
        assert_not_equals(&line.hash_code(), &other_line.hash_code());
    } else {
        assert_equals(&line, &other_line);
        assert_equals(&line.hash_code(), &other_line.hash_code());
    }
    Ok(())
}

/// `Arrays.equals(float[], float[])`: `Float.floatToIntBits` equality.
pub fn arrays_equals(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| float_to_int_bits(*x) == float_to_int_bits(*y))
}

/// `Float.floatToIntBits(v)`: NaN canonicalized.
fn float_to_int_bits(v: f32) -> u32 {
    if v.is_nan() {
        0x7fc0_0000
    } else {
        v.to_bits()
    }
}
