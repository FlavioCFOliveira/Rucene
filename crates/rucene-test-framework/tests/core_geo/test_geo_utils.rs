//! Port of `lucene/core/src/test/org/apache/lucene/geo/TestGeoUtils.java`.

use std::f64::consts::PI;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::geo::{GeoUtils, Rectangle};
use rucene::util::SloppyMath;
use rucene_test_framework::geo::earth_debugger::EarthDebugger;
use rucene_test_framework::geo::geo_test_util;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals_delta, class, random, test, try_catch};

/// `TestGeoUtils`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.geo.TestGeoUtils",
        vec![
            test("testRandomCircleToBBox", test_random_circle_to_bbox),
            test("testBoundingBoxOpto", test_bounding_box_opto),
            test("testHaversinOpto", test_haversin_opto),
            test("testInfiniteRect", test_infinite_rect),
            test("testAxisLat", test_axis_lat),
            test("testCircleOpto", test_circle_opto),
            test("testWithin90LonDegrees", test_within90_lon_degrees),
        ],
    )
}

/// We rely heavily on GeoUtils.circleToBBox so we test it here:
///
/// Port of `TestGeoUtils.testRandomCircleToBBox`.
fn test_random_circle_to_bbox() -> Result<()> {
    let iters = LuceneTestCase::at_least(100);
    for _iter in 0..iters {
        let center_lat = geo_test_util::next_latitude();
        let center_lon = geo_test_util::next_longitude();

        let radius_meters = if random().next_boolean() {
            // Approx 4 degrees lon at the equator:
            random().next_double() * 444000.0
        } else {
            random().next_double() * 50000000.0
        };

        // TODO: randomly quantize radius too, to provoke exact math errors?

        let bbox = Rectangle::from_point_distance(center_lat, center_lon, radius_meters)?;

        let num_points_to_try = 1000;
        for _i in 0..num_points_to_try {
            let point = geo_test_util::next_point_near_rectangle(&bbox);
            let lat = point[0];
            let lon = point[1];

            let distance_meters = SloppyMath::haversin_meters(center_lat, center_lon, lat, lon);

            // Haversin says it's within the circle:
            let haversin_says = distance_meters <= radius_meters;

            // BBox says its within the box:
            let bbox_says = if bbox.crosses_dateline() {
                if lat >= bbox.min_lat() && lat <= bbox.max_lat() {
                    lon <= bbox.max_lon() || lon >= bbox.min_lon()
                } else {
                    false
                }
            } else {
                lat >= bbox.min_lat()
                    && lat <= bbox.max_lat()
                    && lon >= bbox.min_lon()
                    && lon <= bbox.max_lon()
            };

            if haversin_says && !bbox_says {
                println!(
                    "centerLat={center_lat} centerLon={center_lon} radiusMeters={radius_meters}"
                );
                println!(
                    "  bbox: lat={} to {} lon={} to {}",
                    bbox.min_lat(),
                    bbox.max_lat(),
                    bbox.min_lon(),
                    bbox.max_lon()
                );
                println!("  point: lat={lat} lon={lon}");
                println!("  haversin: {distance_meters}");
                Assert::fail(
                    "point was within the distance according to haversin, but the bbox doesn't contain it",
                );
            }
            // else: it's fine if haversin said it was outside the radius and
            // bbox said it was inside the box
        }
    }
    Ok(())
}

/// similar to testRandomCircleToBBox, but different, less evil, maybe simpler
///
/// Port of `TestGeoUtils.testBoundingBoxOpto`.
fn test_bounding_box_opto() -> Result<()> {
    let iters = LuceneTestCase::at_least(100);
    for _i in 0..iters {
        let lat = geo_test_util::next_latitude();
        let lon = geo_test_util::next_longitude();
        let radius = 50000000.0 * random().next_double();
        let r#box = Rectangle::from_point_distance(lat, lon, radius)?;

        let (box1, box2) = if r#box.crosses_dateline() {
            (
                Rectangle::new(r#box.min_lat(), r#box.max_lat(), -180.0, r#box.max_lon())?,
                Some(Rectangle::new(
                    r#box.min_lat(),
                    r#box.max_lat(),
                    r#box.min_lon(),
                    180.0,
                )?),
            )
        } else {
            (r#box, None)
        };

        for _j in 0..1000 {
            let point = geo_test_util::next_point_near_rectangle(&r#box);
            let lat2 = point[0];
            let lon2 = point[1];
            // if the point is within radius, then it should be in our bounding box
            if SloppyMath::haversin_meters(lat, lon, lat2, lon2) <= radius {
                Assert::assert_true(lat >= r#box.min_lat() && lat <= r#box.max_lat());
                Assert::assert_true(
                    lon >= box1.min_lon() && lon <= box1.max_lon()
                        || box2
                            .as_ref()
                            .is_some_and(|b| lon >= b.min_lon() && lon <= b.max_lon()),
                );
            }
        }
    }
    Ok(())
}

/// test we can use haversinSortKey() for distance queries.
///
/// Port of `TestGeoUtils.testHaversinOpto`.
fn test_haversin_opto() -> Result<()> {
    let iters = LuceneTestCase::at_least(100);
    for _i in 0..iters {
        let lat = geo_test_util::next_latitude();
        let lon = geo_test_util::next_longitude();
        let radius = 50000000.0 * random().next_double();
        let r#box = Rectangle::from_point_distance(lat, lon, radius)?;

        if r#box.max_lon() - lon < 90.0 && lon - r#box.min_lon() < 90.0 {
            let min_partial_distance =
                SloppyMath::haversin_sort_key(lat, lon, lat, r#box.max_lon()).max(
                    SloppyMath::haversin_sort_key(lat, lon, r#box.max_lat(), lon),
                );

            for _j in 0..10000 {
                let point = geo_test_util::next_point_near_rectangle(&r#box);
                let lat2 = point[0];
                let lon2 = point[1];
                // if the point is within radius, then it should be <= our sort key
                if SloppyMath::haversin_meters(lat, lon, lat2, lon2) <= radius {
                    Assert::assert_true(
                        SloppyMath::haversin_sort_key(lat, lon, lat2, lon2) <= min_partial_distance,
                    );
                }
            }
        }
    }
    Ok(())
}

/// Test infinite radius covers whole earth
///
/// Port of `TestGeoUtils.testInfiniteRect`.
fn test_infinite_rect() -> Result<()> {
    for _i in 0..1000 {
        let center_lat = geo_test_util::next_latitude();
        let center_lon = geo_test_util::next_longitude();
        let rect = Rectangle::from_point_distance(center_lat, center_lon, f64::INFINITY)?;
        assert_equals_delta(-180.0, rect.min_lon(), 0.0);
        assert_equals_delta(180.0, rect.max_lon(), 0.0);
        assert_equals_delta(-90.0, rect.min_lat(), 0.0);
        assert_equals_delta(90.0, rect.max_lat(), 0.0);
        Assert::assert_false(rect.crosses_dateline());
    }
    Ok(())
}

/// Port of `TestGeoUtils.testAxisLat`.
fn test_axis_lat() -> Result<()> {
    let earth_circumference = 2.0 * PI * GeoUtils::EARTH_MEAN_RADIUS_METERS;
    assert_equals_delta(
        90.0,
        Rectangle::axis_lat(0.0, earth_circumference / 4.0),
        0.0,
    );

    for _i in 0..100 {
        let really_big = random().next_int_bound(10) == 0;
        let max_radius = if really_big {
            1.1 * earth_circumference
        } else {
            earth_circumference / 8.0
        };
        let radius = max_radius * random().next_double();
        let mut prev_axis_lat = Rectangle::axis_lat(0.0, radius);
        let mut lat = 0.1;
        while lat < 90.0 {
            let next_axis_lat = Rectangle::axis_lat(lat, radius);
            let bbox = Rectangle::from_point_distance(lat, 180.0, radius)?;
            let dist = SloppyMath::haversin_meters(lat, 180.0, next_axis_lat, bbox.max_lon());
            if next_axis_lat < GeoUtils::MAX_LAT_INCL {
                Assert::assert_equals_double_msg(Some(&format!("lat = {lat}")), dist, radius, 0.1);
            }
            Assert::assert_true_msg(&format!("lat = {lat}"), prev_axis_lat <= next_axis_lat);
            prev_axis_lat = next_axis_lat;
            lat += 0.1;
        }

        prev_axis_lat = Rectangle::axis_lat(-0.0, radius);
        let mut lat = -0.1;
        while lat > -90.0 {
            let next_axis_lat = Rectangle::axis_lat(lat, radius);
            let bbox = Rectangle::from_point_distance(lat, 180.0, radius)?;
            let dist = SloppyMath::haversin_meters(lat, 180.0, next_axis_lat, bbox.max_lon());
            if next_axis_lat > GeoUtils::MIN_LAT_INCL {
                Assert::assert_equals_double_msg(Some(&format!("lat = {lat}")), dist, radius, 0.1);
            }
            Assert::assert_true_msg(&format!("lat = {lat}"), prev_axis_lat >= next_axis_lat);
            prev_axis_lat = next_axis_lat;
            lat -= 0.1;
        }
    }
    Ok(())
}

/// TODO: does not really belong here, but we test it like this for now
/// we can make a fake IndexReader to send boxes directly to Point visitors
/// instead?
///
/// Port of `TestGeoUtils.testCircleOpto`.
fn test_circle_opto() -> Result<()> {
    let random = random();
    let iters = LuceneTestCase::at_least_with(&*random, 3);
    let mut i = 0;
    while i < iters {
        // circle
        let center_lat = -90.0 + 180.0 * random.next_double();
        let center_lon = -180.0 + 360.0 * random.next_double();
        let radius = 50_000_000.0 * random.next_double();
        let r#box = Rectangle::from_point_distance(center_lat, center_lon, radius)?;
        // TODO: remove this leniency!
        if r#box.crosses_dateline() {
            // --i; try again... (the loop's i++ is skipped instead)
            continue;
        }
        let axis_lat = Rectangle::axis_lat(center_lat, radius);

        let inner_iters = LuceneTestCase::at_least(100);
        for _k in 0..inner_iters {
            let lat_bounds = [-90.0, r#box.min_lat(), axis_lat, r#box.max_lat(), 90.0];
            let lon_bounds = [-180.0, r#box.min_lon(), center_lon, r#box.max_lon(), 180.0];
            // first choose an upper left corner
            let max_lat_row = random.next_int_bound(4) as usize;
            let lat_max = random_in_range(
                &*random,
                lat_bounds[max_lat_row],
                lat_bounds[max_lat_row + 1],
            );
            let min_lon_col = random.next_int_bound(4) as usize;
            let lon_min = random_in_range(
                &*random,
                lon_bounds[min_lon_col],
                lon_bounds[min_lon_col + 1],
            );
            // now choose a lower right corner
            // make sure it will at least cross into the bbox
            let min_lat_max_row = if max_lat_row == 3 { 3 } else { max_lat_row + 1 };
            let min_lat_row = random.next_int_bound(min_lat_max_row as i32) as usize;
            let lat_min = random_in_range(
                &*random,
                lat_bounds[min_lat_row],
                lat_bounds[min_lat_row + 1].min(lat_max),
            );
            // make sure it will at least cross into the bbox
            let max_lon_min_col = min_lon_col.max(1);
            let max_lon_col =
                max_lon_min_col + random.next_int_bound(4 - max_lon_min_col as i32) as usize;
            let lon_max = random_in_range(
                &*random,
                lon_bounds[max_lon_col].max(lon_min),
                lon_bounds[max_lon_col + 1],
            );

            debug_assert!(lat_max >= lat_min);
            debug_assert!(lon_max >= lon_min);

            if is_disjoint(
                center_lat, center_lon, radius, axis_lat, lat_min, lat_max, lon_min, lon_max,
            ) {
                // intersects says false: test a ton of points
                for _j in 0..200 {
                    let mut lat = lat_min + (lat_max - lat_min) * random.next_double();
                    let mut lon = lon_min + (lon_max - lon_min) * random.next_double();

                    if random.next_boolean() {
                        // explicitly test an edge
                        let edge = random.next_int_bound(4);
                        if edge == 0 {
                            lat = lat_min;
                        } else if edge == 1 {
                            lat = lat_max;
                        } else if edge == 2 {
                            lon = lon_min;
                        } else if edge == 3 {
                            lon = lon_max;
                        }
                    }
                    let distance = SloppyMath::haversin_meters(center_lat, center_lon, lat, lon);
                    let checked = try_catch("java.lang.AssertionError", || {
                        let bbox = Rectangle::from_point_distance(center_lat, center_lon, radius)
                            .map(|b| b.to_string())
                            .unwrap_or_default();
                        Assert::assert_true_msg(
                            &format!(
                                "\nisDisjoint(\ncenterLat={center_lat}\ncenterLon={center_lon}\nradius={radius}\nlatMin={lat_min}\nlatMax={lat_max}\nlonMin={lon_min}\nlonMax={lon_max}) == false BUT\nhaversin({center_lat}, {center_lon}, {lat}, {lon}) = {distance}\nbbox={bbox}"
                            ),
                            distance > radius,
                        );
                    });
                    if let Err(e) = checked {
                        let mut ed = EarthDebugger::new();
                        ed.add_rect(lat_min, lat_max, lon_min, lon_max);
                        ed.add_circle(center_lat, center_lon, radius, true)?;
                        println!("{}", ed.finish()?);
                        e.throw();
                    }
                }
            }
        }
        i += 1;
    }
    Ok(())
}

/// Port of `TestGeoUtils.randomInRange`.
fn random_in_range(random: &dyn Random, min: f64, max: f64) -> f64 {
    min + (max - min) * random.next_double()
}

/// Port of `TestGeoUtils.isDisjoint`.
#[allow(clippy::too_many_arguments)]
fn is_disjoint(
    center_lat: f64,
    center_lon: f64,
    radius: f64,
    axis_lat: f64,
    lat_min: f64,
    lat_max: f64,
    lon_min: f64,
    lon_max: f64,
) -> bool {
    if (center_lon < lon_min || center_lon > lon_max)
        && (axis_lat + Rectangle::AXISLAT_ERROR < lat_min
            || axis_lat - Rectangle::AXISLAT_ERROR > lat_max)
    {
        // circle not fully inside / crossing axis
        if SloppyMath::haversin_meters(center_lat, center_lon, lat_min, lon_min) > radius
            && SloppyMath::haversin_meters(center_lat, center_lon, lat_min, lon_max) > radius
            && SloppyMath::haversin_meters(center_lat, center_lon, lat_max, lon_min) > radius
            && SloppyMath::haversin_meters(center_lat, center_lon, lat_max, lon_max) > radius
        {
            // no points inside
            return true;
        }
    }
    false
}

/// Port of `TestGeoUtils.testWithin90LonDegrees`.
fn test_within90_lon_degrees() -> Result<()> {
    Assert::assert_true(GeoUtils::within_90_lon_degrees(0.0, -80.0, 80.0));
    Assert::assert_false(GeoUtils::within_90_lon_degrees(0.0, -100.0, 80.0));
    Assert::assert_false(GeoUtils::within_90_lon_degrees(0.0, -80.0, 100.0));

    Assert::assert_true(GeoUtils::within_90_lon_degrees(-150.0, 140.0, 170.0));
    Assert::assert_false(GeoUtils::within_90_lon_degrees(-150.0, 120.0, 150.0));

    Assert::assert_true(GeoUtils::within_90_lon_degrees(150.0, -170.0, -140.0));
    Assert::assert_false(GeoUtils::within_90_lon_degrees(150.0, -150.0, -120.0));
    Ok(())
}
