//! Port of `lucene/core/src/test/org/apache/lucene/util/TestSloppyMath.java`.
//!
//! Java's reference `StrictMath.cos`, `sin` and `asin` (fdlibm) are Rust's
//! `f64::cos`, `sin` and `asin` (the platform libm); both are within one ulp
//! of the exact result, far inside every delta the test uses.

use std::f64::consts::PI;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::SloppyMath;
use rucene_test_framework::geo::geo_test_util;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_equals_delta, class, random, test};

/// `TestSloppyMath`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestSloppyMath",
        vec![
            test("testCos", test_cos),
            test("testSin", test_sin),
            test("testAsin", test_asin),
            test("testHaversin", test_haversin),
            test("testHaversinSortKey", test_haversin_sort_key),
            test("testHaversinFromSortKey", test_haversin_from_sort_key),
            test("testAgainstSlowVersion", test_against_slow_version),
            test("testAcrossWholeWorldSteps", test_across_whole_world_steps),
            test(
                "testAgainstSlowVersionReasonable",
                test_against_slow_version_reasonable,
            ),
        ],
    )
}

// accuracy for cos()
const COS_DELTA: f64 = 1E-15;
// accuracy for sin()
const SIN_DELTA: f64 = 1E-12;
// accuracy for asin()
const ASIN_DELTA: f64 = 1E-7;
// accuracy for haversinMeters()
const HAVERSIN_DELTA: f64 = 38E-2;
// accuracy for haversinMeters() for "reasonable" distances (< 1000km)
const REASONABLE_HAVERSIN_DELTA: f64 = 1E-5;

fn cos(a: f64) -> f64 {
    SloppyMath::cos(a)
}
fn sin(a: f64) -> f64 {
    SloppyMath::sin(a)
}
fn asin(a: f64) -> f64 {
    SloppyMath::asin(a)
}
fn haversin_meters(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    SloppyMath::haversin_meters(lat1, lon1, lat2, lon2)
}

fn test_cos() -> Result<()> {
    Assert::assert_true(cos(f64::NAN).is_nan());
    Assert::assert_true(cos(f64::NEG_INFINITY).is_nan());
    Assert::assert_true(cos(f64::INFINITY).is_nan());
    assert_equals_delta(1f64.cos(), cos(1.0), COS_DELTA);
    assert_equals_delta(0f64.cos(), cos(0.0), COS_DELTA);
    assert_equals_delta((PI / 2.0).cos(), cos(PI / 2.0), COS_DELTA);
    assert_equals_delta((-PI / 2.0).cos(), cos(-PI / 2.0), COS_DELTA);
    assert_equals_delta((PI / 4.0).cos(), cos(PI / 4.0), COS_DELTA);
    assert_equals_delta((-PI / 4.0).cos(), cos(-PI / 4.0), COS_DELTA);
    assert_equals_delta((PI * 2.0 / 3.0).cos(), cos(PI * 2.0 / 3.0), COS_DELTA);
    assert_equals_delta((-PI * 2.0 / 3.0).cos(), cos(-PI * 2.0 / 3.0), COS_DELTA);
    assert_equals_delta((PI / 6.0).cos(), cos(PI / 6.0), COS_DELTA);
    assert_equals_delta((-PI / 6.0).cos(), cos(-PI / 6.0), COS_DELTA);

    // testing purely random longs is inefficent, as for stupid parameters we just
    // pass thru to Math.cos() instead of doing some huperduper arg reduction
    for _ in 0..10000 {
        let mut d = random().next_double() * SloppyMath::sin_cos_max_value_for_int_modulo();
        if random().next_boolean() {
            d = -d;
        }
        assert_equals_delta(d.cos(), cos(d), COS_DELTA);
    }
    Ok(())
}

fn test_sin() -> Result<()> {
    Assert::assert_true(sin(f64::NAN).is_nan());
    Assert::assert_true(sin(f64::NEG_INFINITY).is_nan());
    Assert::assert_true(sin(f64::INFINITY).is_nan());
    assert_equals_delta(1f64.sin(), sin(1.0), SIN_DELTA);
    assert_equals_delta(0f64.sin(), sin(0.0), SIN_DELTA);
    assert_equals_delta((PI / 2.0).sin(), sin(PI / 2.0), SIN_DELTA);
    assert_equals_delta((-PI / 2.0).sin(), sin(-PI / 2.0), SIN_DELTA);
    assert_equals_delta((PI / 4.0).sin(), sin(PI / 4.0), SIN_DELTA);
    assert_equals_delta((-PI / 4.0).sin(), sin(-PI / 4.0), SIN_DELTA);
    assert_equals_delta((PI * 2.0 / 3.0).sin(), sin(PI * 2.0 / 3.0), SIN_DELTA);
    assert_equals_delta((-PI * 2.0 / 3.0).sin(), sin(-PI * 2.0 / 3.0), SIN_DELTA);
    assert_equals_delta((PI / 6.0).sin(), sin(PI / 6.0), SIN_DELTA);
    assert_equals_delta((-PI / 6.0).sin(), sin(-PI / 6.0), SIN_DELTA);

    // testing purely random longs is inefficent, as for stupid parameters we just
    // pass thru to Math.cos() instead of doing some huperduper arg reduction
    for _ in 0..10000 {
        let mut d = random().next_double() * SloppyMath::sin_cos_max_value_for_int_modulo();
        if random().next_boolean() {
            d = -d;
        }
        assert_equals_delta(d.sin(), sin(d), SIN_DELTA);
    }
    Ok(())
}

fn test_asin() -> Result<()> {
    Assert::assert_true(asin(f64::NAN).is_nan());
    Assert::assert_true(asin(2.0).is_nan());
    Assert::assert_true(asin(-2.0).is_nan());
    assert_equals_delta(-PI / 2.0, asin(-1.0), ASIN_DELTA);
    assert_equals_delta(-PI / 3.0, asin(-0.8660254), ASIN_DELTA);
    #[allow(clippy::approx_constant)] // the Java test's literal input, not the constant
    assert_equals_delta(-PI / 4.0, asin(-0.7071068), ASIN_DELTA);
    assert_equals_delta(-PI / 6.0, asin(-0.5), ASIN_DELTA);
    assert_equals_delta(0.0, asin(0.0), ASIN_DELTA);
    assert_equals_delta(PI / 6.0, asin(0.5), ASIN_DELTA);
    #[allow(clippy::approx_constant)] // the Java test's literal input, not the constant
    assert_equals_delta(PI / 4.0, asin(0.7071068), ASIN_DELTA);
    assert_equals_delta(PI / 3.0, asin(0.8660254), ASIN_DELTA);
    assert_equals_delta(PI / 2.0, asin(1.0), ASIN_DELTA);
    // only values -1..1 are useful
    for _ in 0..10000 {
        let mut d = random().next_double();
        if random().next_boolean() {
            d = -d;
        }
        assert_equals_delta(d.asin(), asin(d), ASIN_DELTA);
        Assert::assert_true(asin(d) >= -PI / 2.0);
        Assert::assert_true(asin(d) <= PI / 2.0);
    }
    Ok(())
}

fn test_haversin() -> Result<()> {
    Assert::assert_true(haversin_meters(1.0, 1.0, 1.0, f64::NAN).is_nan());
    Assert::assert_true(haversin_meters(1.0, 1.0, f64::NAN, 1.0).is_nan());
    Assert::assert_true(haversin_meters(1.0, f64::NAN, 1.0, 1.0).is_nan());
    Assert::assert_true(haversin_meters(f64::NAN, 1.0, 1.0, 1.0).is_nan());

    assert_equals_delta(0.0, haversin_meters(0.0, 0.0, 0.0, 0.0), 0.0);
    assert_equals_delta(0.0, haversin_meters(0.0, -180.0, 0.0, -180.0), 0.0);
    assert_equals_delta(0.0, haversin_meters(0.0, -180.0, 0.0, 180.0), 0.0);
    assert_equals_delta(0.0, haversin_meters(0.0, 180.0, 0.0, 180.0), 0.0);
    assert_equals_delta(0.0, haversin_meters(90.0, 0.0, 90.0, 0.0), 0.0);
    assert_equals_delta(0.0, haversin_meters(90.0, -180.0, 90.0, -180.0), 0.0);
    assert_equals_delta(0.0, haversin_meters(90.0, -180.0, 90.0, 180.0), 0.0);
    assert_equals_delta(0.0, haversin_meters(90.0, 180.0, 90.0, 180.0), 0.0);

    // Test half a circle on the equator, using WGS84 mean earth radius in meters
    let earth_radius_ms = 6_371_008.771_4;
    let half_circle = earth_radius_ms * PI;
    assert_equals_delta(half_circle, haversin_meters(0.0, 0.0, 0.0, 180.0), 0.0);

    let r = random();
    let random_lat1 = 40.7143528 + ((r.next_int_bound(10) - 5) * 360) as f64;
    let random_lon1 = -74.0059731 + ((r.next_int_bound(10) - 5) * 360) as f64;

    let random_lat2 = 40.65 + ((r.next_int_bound(10) - 5) * 360) as f64;
    let random_lon2 = -73.95 + ((r.next_int_bound(10) - 5) * 360) as f64;

    assert_equals_delta(
        8_572.113_7,
        haversin_meters(random_lat1, random_lon1, random_lat2, random_lon2),
        0.01,
    );

    // from solr and ES tests (with their respective epsilons)
    assert_equals_delta(
        0.0,
        haversin_meters(40.7143528, -74.0059731, 40.7143528, -74.0059731),
        0.0,
    );
    assert_equals_delta(
        5_285.89,
        haversin_meters(40.7143528, -74.0059731, 40.759011, -73.9844722),
        0.01,
    );
    assert_equals_delta(
        462.10,
        haversin_meters(40.7143528, -74.0059731, 40.718266, -74.007819),
        0.01,
    );
    assert_equals_delta(
        1_054.98,
        haversin_meters(40.7143528, -74.0059731, 40.7051157, -74.0088305),
        0.01,
    );
    assert_equals_delta(
        1_258.12,
        haversin_meters(40.7143528, -74.0059731, 40.7247222, -74.0),
        0.01,
    );
    assert_equals_delta(
        2_028.52,
        haversin_meters(40.7143528, -74.0059731, 40.731033, -73.9962255),
        0.01,
    );
    assert_equals_delta(
        8_572.11,
        haversin_meters(40.7143528, -74.0059731, 40.65, -73.95),
        0.01,
    );
    Ok(())
}

/// `Integer.signum(Double.compare(a, b))`.
fn compare_signum(a: f64, b: f64) -> i32 {
    match a.total_cmp(&b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// Test this method sorts the same way as real haversin
fn test_haversin_sort_key() -> Result<()> {
    let iters = LuceneTestCase::at_least(10000);
    for _ in 0..iters {
        let center_lat = geo_test_util::next_latitude();
        let center_lon = geo_test_util::next_longitude();

        let lat1 = geo_test_util::next_latitude();
        let lon1 = geo_test_util::next_longitude();

        let lat2 = geo_test_util::next_latitude();
        let lon2 = geo_test_util::next_longitude();

        let expected = compare_signum(
            haversin_meters(center_lat, center_lon, lat1, lon1),
            haversin_meters(center_lat, center_lon, lat2, lon2),
        );
        let actual = compare_signum(
            SloppyMath::haversin_sort_key(center_lat, center_lon, lat1, lon1),
            SloppyMath::haversin_sort_key(center_lat, center_lon, lat2, lon2),
        );
        assert_equals(&expected, &actual);
        assert_equals_delta(
            haversin_meters(center_lat, center_lon, lat1, lon1),
            SloppyMath::haversin_meters_from_sort_key(SloppyMath::haversin_sort_key(
                center_lat, center_lon, lat1, lon1,
            )),
            0.0,
        );
        assert_equals_delta(
            haversin_meters(center_lat, center_lon, lat2, lon2),
            SloppyMath::haversin_meters_from_sort_key(SloppyMath::haversin_sort_key(
                center_lat, center_lon, lat2, lon2,
            )),
            0.0,
        );
    }
    Ok(())
}

fn test_haversin_from_sort_key() -> Result<()> {
    assert_equals_delta(0.0, SloppyMath::haversin_meters_from_sort_key(0.0), 0.0);
    Ok(())
}

fn test_against_slow_version() -> Result<()> {
    for _ in 0..100_000 {
        let lat1 = geo_test_util::next_latitude();
        let lon1 = geo_test_util::next_longitude();
        let lat2 = geo_test_util::next_latitude();
        let lon2 = geo_test_util::next_longitude();

        let expected = slow_haversin(lat1, lon1, lat2, lon2);
        let actual = haversin_meters(lat1, lon1, lat2, lon2);
        assert_equals_delta(expected, actual, HAVERSIN_DELTA);
    }
    Ok(())
}

/// Step across the whole world to find huge absolute errors. Don't rely on
/// random number generator to pick these massive distances.
fn test_across_whole_world_steps() -> Result<()> {
    let mut lat1 = -90;
    while lat1 <= 90 {
        let mut lon1 = -180;
        while lon1 <= 180 {
            let mut lat2 = -90;
            while lat2 <= 90 {
                let mut lon2 = -180;
                while lon2 <= 180 {
                    let (a, b, c, d) = (lat1 as f64, lon1 as f64, lat2 as f64, lon2 as f64);
                    let expected = slow_haversin(a, b, c, d);
                    let actual = haversin_meters(a, b, c, d);
                    assert_equals_delta(expected, actual, HAVERSIN_DELTA);
                    lon2 += 10;
                }
                lat2 += 10;
            }
            lon1 += 10;
        }
        lat1 += 10;
    }
    Ok(())
}

fn test_against_slow_version_reasonable() -> Result<()> {
    for _ in 0..100_000 {
        let lat1 = geo_test_util::next_latitude();
        let lon1 = geo_test_util::next_longitude();
        let lat2 = geo_test_util::next_latitude();
        let lon2 = geo_test_util::next_longitude();

        let expected = haversin_meters(lat1, lon1, lat2, lon2);
        if expected < 1_000_000.0 {
            let actual = slow_haversin(lat1, lon1, lat2, lon2);
            assert_equals_delta(expected, actual, REASONABLE_HAVERSIN_DELTA);
        }
    }
    Ok(())
}

// simple incorporation of the wikipedia formula
fn slow_haversin(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let h1 = (1.0 - (lat2.to_radians() - lat1.to_radians()).cos()) / 2.0;
    let h2 = (1.0 - (lon2.to_radians() - lon1.to_radians()).cos()) / 2.0;
    let h = h1 + lat1.to_radians().cos() * lat2.to_radians().cos() * h2;
    2.0 * 6371008.7714 * 1f64.min(h.sqrt()).asin()
}
