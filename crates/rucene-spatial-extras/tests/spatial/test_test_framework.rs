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

//! Port of `TestTestFramework.java` (package `org.apache.lucene.spatial`):
//! make sure we are reading the tests as expected.

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{Assert, Test};
use rucene::error::{LuceneError, Result};
use rucene_spatial_extras::query::spatial_args_parser::SpatialArgsParser;
use rucene_spatial_extras::query::spatial_operation::SpatialOperation;
use spatial4j::context::spatial_context::SpatialContext;
use std::sync::Arc;

use crate::core_support::{class, test};
use crate::spatial::spatial_test_query::SpatialTestQuery;
use crate::spatial::strategy_test_case::{get_resource_as_stream, QTEST_CITIES_INTERSECTS_BBOX};

/// `TestTestFramework`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.spatial.TestTestFramework",
        vec![
            test("testQueries", test_queries).annotate(Test::new()),
            test("spatialExample", spatial_example).annotate(Test::new()),
        ],
    )
}

/// `testQueries()`.
fn test_queries() -> Result<()> {
    let name = QTEST_CITIES_INTERSECTS_BBOX;

    let input = get_resource_as_stream(name).ok_or_else(LuceneError::null_pointer)?;
    let ctx = SpatialContext::geo();
    let iter = SpatialTestQuery::get_test_queries(&SpatialArgsParser::new(), &ctx, name, &input)?; // closes the InputStream
    let mut tests = Vec::new();
    for t in iter {
        tests.push(t);
    }
    Assert::assert_equals(3, tests.len() as i32);

    let sf = &tests[0];
    // assert
    Assert::assert_equals(1, sf.ids.len() as i32);
    Assert::assert_true(sf.ids[0] == "G5391959");
    Assert::assert_true(sf.args.get_shape().as_rectangle().is_some());
    Assert::assert_equals(SpatialOperation::Intersects, sf.args.get_operation());
    Ok(())
}

/// `spatialExample()`: kind of a hack so that SpatialExample is tested
/// despite it not starting or ending with "Test".
fn spatial_example() -> Result<()> {
    crate::spatial::test_spatial_example::main()
}
