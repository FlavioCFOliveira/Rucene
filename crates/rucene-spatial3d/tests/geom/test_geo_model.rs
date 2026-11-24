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

//! Port of `TestGeoModel.java`.

use crate::spatial3d_support::prelude::*;

/// `TestGeoModel`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.spatial3d.geom.TestGeoModel",
        vec![
            test("testBasicCircle", test_basic_circle).annotate(Test::new()),
            test("testBasicRectangle", test_basic_rectangle).annotate(Test::new()),
        ],
    )
}

static SCALEDMODEL: LazyLock<Arc<PlanetModel>> =
    LazyLock::new(|| Arc::new(PlanetModel::new(1.2, 1.5)));

fn test_basic_circle() -> Result<()> {
    // The point of this test is just to make sure nothing blows up doing normal things with a quite
    // non-spherical model
    // Make sure that the north pole is in the circle, and south pole isn't
    let north_pole = GeoPoint::new_lat_lon(&SCALEDMODEL, PI * 0.5, 0.0)?;
    let south_pole = GeoPoint::new_lat_lon(&SCALEDMODEL, (-PI) * 0.5, 0.0)?;
    let point1 = GeoPoint::new_lat_lon(&SCALEDMODEL, PI * 0.25, 0.0)?;
    let point2 = GeoPoint::new_lat_lon(&SCALEDMODEL, PI * 0.125, 0.0)?;
    let mut circle: Arc<dyn GeoCircle> =
        Arc::new(GeoStandardCircle::new(&SCALEDMODEL, PI * 0.5, 0.0, 0.01)?);
    Assert::assert_true(circle.is_within(&north_pole));
    Assert::assert_false(circle.is_within(&south_pole));
    Assert::assert_false(circle.is_within(&point1));
    let mut bounds: LatLonBounds;
    bounds = LatLonBounds::new();
    circle.get_bounds(&mut bounds);
    Assert::assert_true(bounds.check_no_longitude_bound());
    Assert::assert_true(bounds.check_no_top_latitude_bound());
    Assert::assert_false(bounds.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(
        None,
        PI * 0.5 - 0.01,
        unbox(bounds.get_min_latitude()),
        0.01,
    );
    circle = Arc::new(GeoStandardCircle::new(&SCALEDMODEL, PI * 0.25, 0.0, 0.01)?);
    Assert::assert_true(circle.is_within(&point1));
    Assert::assert_false(circle.is_within(&north_pole));
    Assert::assert_false(circle.is_within(&south_pole));
    bounds = LatLonBounds::new();
    circle.get_bounds(&mut bounds);
    Assert::assert_false(bounds.check_no_top_latitude_bound());
    Assert::assert_false(bounds.check_no_longitude_bound());
    Assert::assert_false(bounds.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(
        None,
        PI * 0.25 + 0.01,
        unbox(bounds.get_max_latitude()),
        0.00001,
    );
    Assert::assert_equals_double_msg(
        None,
        PI * 0.25 - 0.01,
        unbox(bounds.get_min_latitude()),
        0.00001,
    );
    Assert::assert_equals_double_msg(None, -0.0125, unbox(bounds.get_left_longitude()), 0.0001);
    Assert::assert_equals_double_msg(None, 0.0125, unbox(bounds.get_right_longitude()), 0.0001);
    circle = Arc::new(GeoStandardCircle::new(&SCALEDMODEL, PI * 0.125, 0.0, 0.01)?);
    Assert::assert_true(circle.is_within(&point2));
    Assert::assert_false(circle.is_within(&north_pole));
    Assert::assert_false(circle.is_within(&south_pole));
    bounds = LatLonBounds::new();
    circle.get_bounds(&mut bounds);
    Assert::assert_false(bounds.check_no_longitude_bound());
    Assert::assert_false(bounds.check_no_top_latitude_bound());
    Assert::assert_false(bounds.check_no_bottom_latitude_bound());
    // Symmetric, as expected
    Assert::assert_equals_double_msg(
        None,
        PI * 0.125 - 0.01,
        unbox(bounds.get_min_latitude()),
        0.00001,
    );
    Assert::assert_equals_double_msg(
        None,
        PI * 0.125 + 0.01,
        unbox(bounds.get_max_latitude()),
        0.00001,
    );
    Assert::assert_equals_double_msg(None, -0.0089, unbox(bounds.get_left_longitude()), 0.0001);
    Assert::assert_equals_double_msg(None, 0.0089, unbox(bounds.get_right_longitude()), 0.0001);
    Ok(())
}

fn test_basic_rectangle() -> Result<()> {
    let bbox: Arc<dyn GeoBBox> = GeoBBoxFactory::make_geo_bbox(&SCALEDMODEL, 1.0, 0.0, 0.0, 1.0)?;
    let inside_point = GeoPoint::new_lat_lon(&SCALEDMODEL, 0.5, 0.5)?;
    Assert::assert_true(bbox.is_within(&inside_point));
    let top_outside_point = GeoPoint::new_lat_lon(&SCALEDMODEL, 1.01, 0.5)?;
    Assert::assert_false(bbox.is_within(&top_outside_point));
    let bottom_outside_point = GeoPoint::new_lat_lon(&SCALEDMODEL, -0.01, 0.5)?;
    Assert::assert_false(bbox.is_within(&bottom_outside_point));
    let left_outside_point = GeoPoint::new_lat_lon(&SCALEDMODEL, 0.5, -0.01)?;
    Assert::assert_false(bbox.is_within(&left_outside_point));
    let right_outside_point = GeoPoint::new_lat_lon(&SCALEDMODEL, 0.5, 1.01)?;
    Assert::assert_false(bbox.is_within(&right_outside_point));
    let mut bounds = LatLonBounds::new();
    bbox.get_bounds(&mut bounds);
    Assert::assert_false(bounds.check_no_longitude_bound());
    Assert::assert_false(bounds.check_no_top_latitude_bound());
    Assert::assert_false(bounds.check_no_bottom_latitude_bound());
    Assert::assert_equals_double_msg(None, 1.0, unbox(bounds.get_max_latitude()), 0.00001);
    Assert::assert_equals_double_msg(None, 0.0, unbox(bounds.get_min_latitude()), 0.00001);
    Assert::assert_equals_double_msg(None, 1.0, unbox(bounds.get_right_longitude()), 0.00001);
    Assert::assert_equals_double_msg(None, 0.0, unbox(bounds.get_left_longitude()), 0.00001);
    Ok(())
}
