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

//! Port of `TestSpatialArgsParser.java` (package
//! `org.apache.lucene.spatial.query`).
// Tests SpatialOperation somewhat too

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{Assert, Test};
use rucene::error::Result;
use rucene_spatial_extras::query::spatial_args_parser::SpatialArgsParser;
use rucene_spatial_extras::query::spatial_operation::SpatialOperation;
use rucene_test_framework::util::java_exceptions::expect_throws;
use spatial4j::context::spatial_context::SpatialContext;

use crate::core_support::{class, test};

/// `TestSpatialArgsParser`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.spatial.query.TestSpatialArgsParser",
        vec![test("testArgsParser", test_args_parser).annotate(Test::new())],
    )
}

/// `ctx` (`private SpatialContext ctx = SpatialContext.GEO`).
// The args parser is only dependent on the ctx for IO so I don't care to test
// with other implementations.
fn ctx() -> Arc<SpatialContext> {
    SpatialContext::geo()
}

/// `testArgsParser()`.
fn test_args_parser() -> Result<()> {
    let parser = SpatialArgsParser::new();

    let mut arg = format!("{}(Envelope(-10, 10, 20, -20))", SpatialOperation::IsWithin);
    let mut out = parser.parse(&arg, &ctx())?;
    Assert::assert_equals(SpatialOperation::IsWithin, out.get_operation());
    let shape = out.get_shape();
    let bounds = shape
        .as_rectangle()
        .expect("java.lang.ClassCastException: the shape is a Rectangle");
    Assert::assert_equals_double_msg(None, -10.0, bounds.get_min_x(), 0.0);
    Assert::assert_equals_double_msg(None, 10.0, bounds.get_max_x(), 0.0);

    // Disjoint should not be scored
    arg = format!(
        "{} (Envelope(-10,-20,20,10))",
        SpatialOperation::IsDisjointTo
    );
    out = parser.parse(&arg, &ctx())?;
    Assert::assert_equals(SpatialOperation::IsDisjointTo, out.get_operation());

    // spatial operations need args
    expect_throws("java.lang.Exception", || {
        parser.parse(&format!("{}[ ]", SpatialOperation::IsDisjointTo), &ctx())
    });

    // unknown operation
    expect_throws("java.lang.Exception", || {
        parser.parse("XXXX(Envelope(-10, 10, 20, -20))", &ctx())
    });

    assert_alias(SpatialOperation::IsWithin, "CoveredBy")?;
    assert_alias(SpatialOperation::IsWithin, "COVEREDBY")?;
    assert_alias(SpatialOperation::IsWithin, "coveredBy")?;
    assert_alias(SpatialOperation::IsWithin, "Within")?;
    assert_alias(SpatialOperation::IsEqualTo, "Equals")?;
    assert_alias(SpatialOperation::IsDisjointTo, "disjoint")?;
    assert_alias(SpatialOperation::Contains, "Covers")?;
    Ok(())
}

/// `assertAlias(SpatialOperation op, String name)`.
fn assert_alias(op: SpatialOperation, name: &str) -> Result<()> {
    let arg = format!("{name}(Point(0 0))");
    let out = SpatialArgsParser::new().parse(&arg, &ctx())?;
    Assert::assert_equals(op, out.get_operation());
    Ok(())
}
