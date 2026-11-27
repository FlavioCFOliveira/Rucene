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

//! Port of `TestSpatialArgs.java` (package `org.apache.lucene.spatial`).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{Assert, Test};
use rucene::error::Result;
use rucene_spatial_extras::query::spatial_args::SpatialArgs;
use rucene_spatial_extras::spatial4j::geo3d_spatial_context_factory::Geo3dSpatialContextFactory;
use rucene_test_framework::util::LuceneTestCase;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::context::spatial_context_factory::SpatialContextFactoryOps;
use spatial4j::shape::shape::Shape;

use crate::core_support::{class, test};

/// `TestSpatialArgs`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.spatial.TestSpatialArgs",
        vec![test("calcDistanceFromErrPct", calc_distance_from_err_pct).annotate(Test::new())],
    )
}

/// `calcDistanceFromErrPct()`.
fn calc_distance_from_err_pct() -> Result<()> {
    let ctx: Arc<SpatialContext> = if LuceneTestCase::usually() {
        SpatialContext::geo()
    } else {
        Geo3dSpatialContextFactory::new().new_spatial_context()?
    };
    let dep = 0.5; // distErrPct

    // the result is the diagonal distance from the center to the closest corner,
    // times distErrPct

    let superwide: Arc<dyn Shape> = ctx.make_rectangle(-180.0, 180.0, 0.0, 0.0)?;
    // 0 distErrPct means 0 distance always
    Assert::assert_equals_double_msg(
        None,
        0.0,
        SpatialArgs::calc_distance_from_err_pct(&*superwide, 0.0, &ctx)?,
        0.0,
    );
    Assert::assert_equals_double_msg(
        None,
        180.0 * dep,
        SpatialArgs::calc_distance_from_err_pct(&*superwide, dep, &ctx)?,
        0.0,
    );

    let supertall: Arc<dyn Shape> = ctx.make_rectangle(0.0, 0.0, -90.0, 90.0)?;
    Assert::assert_equals_double_msg(
        None,
        90.0 * dep,
        SpatialArgs::calc_distance_from_err_pct(&*supertall, dep, &ctx)?,
        0.0,
    );

    let upperhalf: Arc<dyn Shape> = ctx.make_rectangle(-180.0, 180.0, 0.0, 90.0)?;
    Assert::assert_equals_double_msg(
        None,
        45.0 * dep,
        SpatialArgs::calc_distance_from_err_pct(&*upperhalf, dep, &ctx)?,
        0.0001,
    );

    let mid_circle: Arc<dyn Shape> = ctx.make_circle(0.0, 0.0, 45.0)?;
    Assert::assert_equals_double_msg(
        None,
        60.0 * dep,
        SpatialArgs::calc_distance_from_err_pct(&*mid_circle, dep, &ctx)?,
        0.0001,
    );
    Ok(())
}
