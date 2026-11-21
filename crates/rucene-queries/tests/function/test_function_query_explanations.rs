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

//! Port of `TestFunctionQueryExplanations.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::boost_query::BoostQuery;
use rucene::search::query::Query;
use rucene_queries::function::function_query::FunctionQuery;
use rucene_queries::function::value_source::{new_context, ValueSource};
use rucene_queries::function::valuesource::const_value_source::ConstValueSource;
use rucene_queries::function::valuesource::range_map_float_function::RangeMapFloatFunction;
use rucene_test_framework::search::base_explanation_test_case::{
    self, qtest, ExplanationTestInstance,
};

use crate::queries_support::core_support::test;

/// `TestFunctionQueryExplanations`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.queries.function.TestFunctionQueryExplanations")
        .extends(&base_explanation_test_case::class())
        .constructor(Constructor::no_args(ExplanationTestInstance::default))
        .method(test("testSimple", test_simple))
        .method(test("testBoost", test_boost))
        .method(test("testMapFunction", test_map_function))
        .build()
}

fn test_simple() -> Result<()> {
    let q: Arc<dyn Query> = Arc::new(FunctionQuery::new(Arc::new(ConstValueSource::new(5.0))));
    qtest(q, &[0, 1, 2, 3])
}

fn test_boost() -> Result<()> {
    let q: Arc<dyn Query> = Arc::new(BoostQuery::new(
        Arc::new(FunctionQuery::new(Arc::new(ConstValueSource::new(5.0)))),
        2.0,
    )?);
    qtest(q, &[0, 1, 2, 3])
}

fn test_map_function() -> Result<()> {
    let rff: Arc<dyn ValueSource> = Arc::new(RangeMapFloatFunction::with_floats(
        Arc::new(ConstValueSource::new(3.0)),
        0.0,
        1.0,
        2.0,
        Some(4.0),
    ));
    let q: Arc<dyn Query> = Arc::new(FunctionQuery::new(Arc::clone(&rff)));
    qtest(q, &[0, 1, 2, 3])?;
    Assert::assert_equals_str(
        None,
        Some("map(const(3.0),0.0,1.0,const(2.0),const(4.0))"),
        Some(&rff.description()),
    );
    Assert::assert_equals_str(
        None,
        Some("map(const(3.0),min=0.0,max=1.0,target=const(2.0),defaultVal=const(4.0))"),
        Some(&values_to_string(&rff, 123)?),
    );

    // DefaultValue is null -> defaults to source value
    let rff: Arc<dyn ValueSource> = Arc::new(RangeMapFloatFunction::with_floats(
        Arc::new(ConstValueSource::new(3.0)),
        0.0,
        1.0,
        2.0,
        None,
    ));
    Assert::assert_equals_str(
        None,
        Some("map(const(3.0),0.0,1.0,const(2.0),null)"),
        Some(&rff.description()),
    );
    Assert::assert_equals_str(
        None,
        Some("map(const(3.0),min=0.0,max=1.0,target=const(2.0),defaultVal=null)"),
        Some(&values_to_string(&rff, 123)?),
    );
    Ok(())
}

/// `rff.getValues(null, null).toString(doc)`.
///
/// Java passes `null` for both the context map and the leaf: the constant
/// sources ignore them. The port's `getValues` takes a context and a leaf
/// by reference, so it passes an empty context (`newContext` without a
/// searcher, as no source reads it) and a leaf of an empty reader, which no
/// constant source reads either.
fn values_to_string(rff: &Arc<dyn ValueSource>, doc: i32) -> Result<String> {
    let (_reader, leaves) = crate::queries_support::search_support::null_leaf_context()?;
    let mut values = Arc::clone(rff).get_values(&new_context(None), &leaves[0])?;
    values.to_doc_string(doc)
}
