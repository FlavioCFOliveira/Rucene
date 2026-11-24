/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `TestRandomGeoPolygon.java`.

use randomizedtesting::generators::BiasedNumbers;

use crate::spatial3d_support::prelude::*;

/// `TestRandomGeoPolygon`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.spatial3d.geom.TestRandomGeoPolygon",
        vec![
            test("testRandomLUCENE8157", test_random_lucene8157).annotate(Test::new()),
            test("testLUCENE8157", test_lucene8157),
            test("testCoplanarityTilePolygon", test_coplanarity_tile_polygon).annotate(Test::new()),
            test("testCompareBigPolygons", test_compare_big_polygons).annotate(Test::new()),
            test("testCompareSmallPolygons", test_compare_small_polygons).annotate(Test::new()),
        ],
    )
}

fn test_random_lucene8157() -> Result<()> {
    let planet_model = RandomGeo3dShapeGenerator::random_planet_model();
    let start_point = RandomGeo3dShapeGenerator::random_geo_point(&planet_model)?;
    let d = random().next_double();
    let distance_small = d * 1e-9 + Vector::MINIMUM_ANGULAR_RESOLUTION;
    let distance_big = d * 1e-7 + Vector::MINIMUM_ANGULAR_RESOLUTION;
    let bearing = random().next_double() * PI;
    let point1 =
        planet_model.surface_point_on_bearing(&start_point, distance_small, bearing * 1.001)?;
    let point2 = planet_model.surface_point_on_bearing(&start_point, distance_big, bearing)?;
    let point3 =
        planet_model.surface_point_on_bearing(&start_point, distance_big, bearing - 0.5 * PI)?;
    let points = vec![start_point, point1, point2, point3];
    match GeoPolygonFactory::make_geo_polygon(&planet_model, points.clone()) {
        Ok(polygon) => Assert::assert_true(polygon.is_some()),
        Err(_) => Assert::fail(&list_to_string(&points)),
    }
    Ok(())
}

fn test_lucene8157() -> Result<()> {
    let point1 = GeoPoint::new_lat_lon(&SPHERE, 0.281855362988772, -0.7816673189809037)?;
    let point2 = GeoPoint::new_lat_lon(&SPHERE, 0.28185536309057774, -0.7816673188511931)?;
    let point3 = GeoPoint::new_lat_lon(&SPHERE, 0.28186535556824205, -0.7816546103463846)?;
    let point4 = GeoPoint::new_lat_lon(&SPHERE, 0.28186757010406716, -0.7816777221140381)?;
    let points = vec![point1, point2, point3, point4];
    if GeoPolygonFactory::make_geo_polygon(&SPHERE, points.clone()).is_err() {
        Assert::fail(&list_to_string(&points));
    }
    Ok(())
}

fn test_coplanarity_tile_polygon() -> Result<()> {
    // POLYGON((-90.55764 -0.34907,-90.55751 -0.34868,-90.55777 -0.34842,-90.55815
    // -0.34766,-90.55943 -0.34842, -90.55918 -0.34842,-90.55764 -0.34907))
    let mut points: Vec<GeoPoint> = Vec::new();
    for (lat, lon) in [
        (-0.34907, -90.55764),
        (-0.34868, -90.55751),
        (-0.34842, -90.55777),
        (-0.34766, -90.55815),
        (-0.34842, -90.55943),
        (-0.34842, -90.55918),
    ] {
        points.push(GeoPoint::new_lat_lon(
            &SPHERE,
            Geo3DUtil::from_degrees(lat),
            Geo3DUtil::from_degrees(lon),
        )?);
    }
    let polygon = npe(GeoPolygonFactory::make_geo_polygon(&SPHERE, points)?)?;
    let polygon = class_cast::<GeoCompositePolygon>(
        &*polygon,
        "org.apache.lucene.spatial3d.geom.GeoCompositePolygon",
    )?;
    Assert::assert_true(polygon.size() == 3);
    Ok(())
}

/// Test comparing different polygon (Big) technologies using random biased
/// doubles.
fn test_compare_big_polygons() -> Result<()> {
    test_compare_polygons(PI)
}

/// Test comparing different polygon (Small) technologies using random biased
/// doubles.
fn test_compare_small_polygons() -> Result<()> {
    test_compare_polygons(1e-4 * PI)
}

/// The `StringBuilder` message a failed build reports.
fn build_failure(header: &str, points: &[GeoPoint], ordered_points: &[GeoPoint]) -> String {
    let mut buffer = String::from(header);
    buffer.push_str(&format!("{}\n", list_to_string(points)));
    buffer.push_str(&format!("WKT:{}", get_wkt(ordered_points)));
    buffer
}

fn test_compare_polygons(limit_distance: f64) -> Result<()> {
    let planet_model = RandomGeo3dShapeGenerator::random_planet_model();
    // Create polygon points using a reference point and a maximum distance to the point
    let reference_point = if random().next_boolean() {
        get_biased_point(&planet_model)?
    } else {
        RandomGeo3dShapeGenerator::random_geo_point(&planet_model)?
    };
    let n = random().next_int_bound(4) + 4;

    let mut ordered_points: Vec<GeoPoint>;
    let mut polygon: Option<Arc<dyn GeoPolygon>>;
    let mut large_polygon: Option<Arc<dyn GeoPolygon>>;
    loop {
        let mut points: Vec<GeoPoint> = Vec::with_capacity(n as usize);
        let mut max_distance = random().next_double() * limit_distance;
        // if distance is too small we can fail
        // building the polygon.
        while max_distance < 1e-7 {
            max_distance = random().next_double() * limit_distance;
        }
        for _i in 0..n {
            loop {
                let distance = BiasedNumbers::random_double_between(&*random(), 0.0, max_distance); // random().nextDouble() * maxDistance;
                let bearing = random().next_double() * 2.0 * PI;
                let p =
                    planet_model.surface_point_on_bearing(&reference_point, distance, bearing)?;
                if !contains(&p, &points) {
                    if points.len() > 1
                        && Plane::are_points_coplanar(
                            &points[points.len() - 1],
                            &points[points.len() - 2],
                            &p,
                        )?
                    {
                        continue;
                    }
                    points.push(p);
                    break;
                }
            }
        }
        // order points so we don't get crossing edges
        ordered_points = RandomGeo3dShapeGenerator::order_points(&points);
        if random().next_boolean() && random().next_boolean() {
            ordered_points.reverse();
        }
        let polygon_description = PolygonDescription::new(ordered_points.clone());

        match GeoPolygonFactory::make_geo_polygon_from_description(
            &planet_model,
            &polygon_description,
        ) {
            Ok(p) => polygon = p,
            Err(e) => {
                let mut buffer = build_failure(
                    "Polygon failed to build with an exception:\n",
                    &points,
                    &ordered_points,
                );
                buffer.push_str(&e.to_string());
                Assert::fail(&buffer);
            }
        }
        if polygon.is_none() {
            Assert::fail(&build_failure(
                "Polygon failed to build:\n",
                &points,
                &ordered_points,
            ));
        }
        match GeoPolygonFactory::make_large_geo_polygon(
            &planet_model,
            std::slice::from_ref(&polygon_description),
        ) {
            Ok(p) => large_polygon = Some(p),
            Err(e) => {
                let mut buffer = build_failure(
                    "Large polygon failed to build with an exception:\n",
                    &points,
                    &ordered_points,
                );
                buffer.push_str(&e.to_string());
                Assert::fail(&buffer);
            }
        }
        if large_polygon.is_none() {
            Assert::fail(&build_failure(
                "Large polygon failed to build:\n",
                &points,
                &ordered_points,
            ));
        }
        let (p, lp) = (npe(polygon.clone())?, npe(large_polygon.clone())?);
        if p.class_name() != lp.class_name() {
            break;
        }
    }
    let polygon = npe(polygon)?;
    let large_polygon = npe(large_polygon)?;
    // Some of these do not work but it seems it s from the way the point is created
    // GeoPoint centerOfMass = getCenterOfMass(planetModel, orderedPoints);
    // checkPoint(polygon, largePolygon, centerOfMass, orderedPoints);
    // checkPoint(polygon, largePolygon, new GeoPoint(-centerOfMass.x, -centerOfMass.y,
    // -centerOfMass.z), orderedPoints);
    // checkPoint(polygon, largePolygon, new GeoPoint(centerOfMass.x, -centerOfMass.y,
    // -centerOfMass.z), orderedPoints);
    // checkPoint(polygon, largePolygon, new GeoPoint(centerOfMass.x, centerOfMass.y,
    // -centerOfMass.z), orderedPoints);
    // checkPoint(polygon, largePolygon, new GeoPoint(-centerOfMass.x, -centerOfMass.y,
    // centerOfMass.z), orderedPoints);
    // checkPoint(polygon, largePolygon, new GeoPoint(-centerOfMass.x, centerOfMass.y,
    // -centerOfMass.z), orderedPoints);
    // checkPoint(polygon, largePolygon, new GeoPoint(centerOfMass.x, -centerOfMass.y,
    // centerOfMass.z), orderedPoints);
    for _i in 0..100000 {
        let point = if random().next_boolean() {
            get_biased_point(&planet_model)?
        } else {
            RandomGeo3dShapeGenerator::random_geo_point(&planet_model)?
        };
        check_point(&polygon, &large_polygon, &point, &ordered_points)?;
    }
    Ok(())
}

fn check_point(
    polygon: &Arc<dyn GeoPolygon>,
    large_polygon: &Arc<dyn GeoPolygon>,
    point: &GeoPoint,
    ordered_points: &[GeoPoint],
) -> Result<()> {
    let with_in1 = polygon.is_within(point);
    let with_in2 = large_polygon.is_within(point);
    let mut buffer = String::new();
    if with_in1 != with_in2 {
        // NOTE: Standard and large polygon are mathematically slightly different
        // close to the edges (due to bounding planes). Nothing we can do about that
        // so we filter the differences.
        let d1 = polygon.compute_outside_distance(&ARC, point)?;
        let d2 = large_polygon.compute_outside_distance(&ARC, point)?;
        if d1 == 0.0 && d2 == 0.0 {
            return Ok(());
        }
        buffer.push_str(&format!("\nStandard polygon: {polygon}\n"));
        buffer.push_str(&format!("\nLarge polygon: {large_polygon}\n"));
        buffer.push_str(&format!("\nPoint: {point}\n"));
        buffer.push_str(&format!("\nWKT: {}", get_wkt(ordered_points)));
        buffer.push_str(&format!(
            "\nWKT: POINT({} {})\n",
            d2s(Geo3DUtil::to_degrees(point.get_longitude())),
            d2s(Geo3DUtil::to_degrees(point.get_latitude()))
        ));
        buffer.push_str(&format!("normal polygon: {with_in1}\n"));
        buffer.push_str(&format!("large polygon: {with_in2}\n"));
    }
    Assert::assert_true_msg(&buffer, with_in1 == with_in2);
    Ok(())
}

fn get_biased_point(planet_model: &Arc<PlanetModel>) -> Result<GeoPoint> {
    let mut lat = BiasedNumbers::random_double_between(&*random(), 0.0, PI / 2.0);
    if random().next_boolean() {
        lat *= -1.0;
    }
    let mut lon = BiasedNumbers::random_double_between(&*random(), 0.0, PI);
    if random().next_boolean() {
        lon *= -1.0;
    }
    GeoPoint::new_lat_lon(planet_model, lat, lon)
}

fn get_wkt(points: &[GeoPoint]) -> String {
    let mut buffer = String::from("POLYGON((");
    for point in points {
        buffer.push_str(&format!(
            "{} {},",
            d2s(Geo3DUtil::to_degrees(point.get_longitude())),
            d2s(Geo3DUtil::to_degrees(point.get_latitude()))
        ));
    }
    buffer.push_str(&format!(
        "{} {}))\n",
        d2s(Geo3DUtil::to_degrees(points[0].get_longitude())),
        d2s(Geo3DUtil::to_degrees(points[0].get_latitude()))
    ));
    buffer
}

fn contains(p: &GeoPoint, points: &[GeoPoint]) -> bool {
    for point in points {
        if point.is_numerically_identical(p) {
            return true;
        }
    }
    false
}
