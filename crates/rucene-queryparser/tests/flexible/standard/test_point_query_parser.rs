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

//! Port of `TestPointQueryParser.java`: simple test for point field
//! integration into the flexible QP.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{DoublePoint, FloatPoint, IntPoint, LongPoint};
use rucene::error::{LuceneError, Result};
use rucene::search::Query;
use rucene_queryparser::flexible::standard::config::points_config::PointsConfig;
use rucene_queryparser::flexible::standard::standard_query_parser::StandardQueryParser;
use rucene_queryparser::jdk::class::{JavaClass, DOUBLE, FLOAT, INTEGER, LONG};
use rucene_queryparser::jdk::decimal_format::DecimalFormat;
use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::number_format::{
    get_integer_instance, get_number_instance, NumberFormat,
};

use crate::qp_support::core_support::{class, test};
use crate::qp_support::{assert_query_eq, jdk_default_hooks};

/// `TestPointQueryParser`.
pub fn class_def() -> Arc<Class> {
    let mut methods = jdk_default_hooks();
    methods.extend([
        test("testIntegers", test_integers),
        test("testLongs", test_longs),
        test("testFloats", test_floats),
        test("testDoubles", test_doubles),
    ]);
    class(
        "org.apache.lucene.queryparser.flexible.standard.TestPointQueryParser",
        methods,
    )
}

/// `new StandardQueryParser()` with `setPointsConfigMap` of a `HashMap`
/// holding `field -> new PointsConfig(format, type)`.
fn parser(
    field: &str,
    format: DecimalFormat,
    type_: &'static JavaClass,
) -> Result<StandardQueryParser> {
    let mut parser = StandardQueryParser::new();
    let mut points_config = HashMap::new();
    let format: Arc<dyn NumberFormat> = Arc::new(format);
    points_config.insert(
        field.to_string(),
        Arc::new(PointsConfig::new(Some(format), Some(type_))?),
    );
    parser.set_points_config_map(Some(Arc::new(RwLock::new(points_config))))?;
    Ok(parser)
}

/// `parser.parse(query, "body")`.
fn parse(parser: &StandardQueryParser, query: &str) -> Result<Arc<dyn Query>> {
    parser
        .parse(query, Some("body"))?
        .ok_or_else(LuceneError::null_pointer)
}

fn test_integers() -> Result<()> {
    let parser = parser("intField", get_integer_instance(&Locale::root())?, &INTEGER)?;
    assert_query_eq(
        &*IntPoint::new_range_query("intField", &[1], &[3])?,
        &*parse(&parser, "intField:[1 TO 3]")?,
    );
    assert_query_eq(
        &*IntPoint::new_range_query("intField", &[1], &[1])?,
        &*parse(&parser, "intField:1")?,
    );
    Ok(())
}

fn test_longs() -> Result<()> {
    let parser = parser("longField", get_integer_instance(&Locale::root())?, &LONG)?;
    assert_query_eq(
        &*LongPoint::new_range_query("longField", &[1], &[3])?,
        &*parse(&parser, "longField:[1 TO 3]")?,
    );
    assert_query_eq(
        &*LongPoint::new_range_query("longField", &[1], &[1])?,
        &*parse(&parser, "longField:1")?,
    );
    Ok(())
}

fn test_floats() -> Result<()> {
    let parser = parser("floatField", get_number_instance(&Locale::root())?, &FLOAT)?;
    assert_query_eq(
        &*FloatPoint::new_range_query("floatField", &[1.5], &[3.6])?,
        &*parse(&parser, "floatField:[1.5 TO 3.6]")?,
    );
    assert_query_eq(
        &*FloatPoint::new_range_query("floatField", &[1.5], &[1.5])?,
        &*parse(&parser, "floatField:1.5")?,
    );
    Ok(())
}

fn test_doubles() -> Result<()> {
    let parser = parser(
        "doubleField",
        get_number_instance(&Locale::root())?,
        &DOUBLE,
    )?;
    assert_query_eq(
        &*DoublePoint::new_range_query("doubleField", &[1.5], &[3.6])?,
        &*parse(&parser, "doubleField:[1.5 TO 3.6]")?,
    );
    assert_query_eq(
        &*DoublePoint::new_range_query("doubleField", &[1.5], &[1.5])?,
        &*parse(&parser, "doubleField:1.5")?,
    );
    Ok(())
}
