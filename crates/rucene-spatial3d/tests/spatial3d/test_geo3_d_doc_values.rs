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

//! Port of `TestGeo3DDocValues.java` (package `org.apache.lucene.spatial3d`).

use randomizedtesting::generators::RandomPicks;
use rucene_spatial3d::geo3_d_util::Geo3DUtil;
use rucene_test_framework::util::TestUtil;

use crate::spatial3d_support::prelude::{
    class, random, test, Arc, Assert, Class, GeoPoint, PlanetModel, Random, Result, CLARKE_1866,
    WGS84,
};

/// `TestGeo3DDocValues`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.spatial3d.TestGeo3DDocValues",
        vec![test("testBasic", test_basic)],
    )
}

fn test_basic() -> Result<()> {
    check_point_encoding(0.0, 0.0)?;
    check_point_encoding(45.0, 72.0)?;
    check_point_encoding(-45.0, -100.0)?;
    let test_amt = TestUtil::next_int(&*random(), 1000, 2000);
    for _i in 0..test_amt {
        check_point_encoding(
            random().next_double() * 180.0 - 90.0,
            random().next_double() * 360.0 - 180.0,
        )?;
    }
    Ok(())
}

fn check_point_encoding(latitude: f64, longitude: f64) -> Result<()> {
    let planet_model: Arc<PlanetModel> =
        RandomPicks::random_from(&*random(), &[WGS84.clone(), CLARKE_1866.clone()]).clone();
    let point = GeoPoint::new_lat_lon(
        &planet_model,
        Geo3DUtil::from_degrees(latitude),
        Geo3DUtil::from_degrees(longitude),
    )?;
    let mut point_value = planet_model.get_doc_value_encoder().encode_point(&point)?;
    let x = planet_model
        .get_doc_value_encoder()
        .decode_x_value(point_value);
    let y = planet_model
        .get_doc_value_encoder()
        .decode_y_value(point_value);
    let z = planet_model
        .get_doc_value_encoder()
        .decode_z_value(point_value);
    let point_r = GeoPoint::new(x, y, z);
    // Check whether stable
    point_value = planet_model
        .get_doc_value_encoder()
        .encode_point_xyz(x, y, z)?;
    Assert::assert_equals_double_msg(
        None,
        x,
        planet_model
            .get_doc_value_encoder()
            .decode_x_value(point_value),
        0.0,
    );
    Assert::assert_equals_double_msg(
        None,
        y,
        planet_model
            .get_doc_value_encoder()
            .decode_y_value(point_value),
        0.0,
    );
    Assert::assert_equals_double_msg(
        None,
        z,
        planet_model
            .get_doc_value_encoder()
            .decode_z_value(point_value),
        0.0,
    );
    // Check whether has some relationship with original point
    Assert::assert_equals_double_msg(None, 0.0, point.arc_distance(&point_r), 0.02);
    Ok(())
}
