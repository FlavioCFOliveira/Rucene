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

//! Port of `TestQueryEqualsHashCode.java` (package `org.apache.lucene.spatial`).
//!
//! The anonymous `ObjGenerator`s return a `Query` or a `DoubleValuesSource`
//! as `Object`: the port's [`Obj`] holds either, with Java's `equals` and
//! `hashCode` of the object it holds.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{Assert, Test};
use rucene::error::Result;
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::Query;
use rucene_spatial_extras::bbox::b_box_strategy::BBoxStrategy;
use rucene_spatial_extras::composite::composite_spatial_strategy::CompositeSpatialStrategy;
use rucene_spatial_extras::prefix::recursive_prefix_tree_strategy::RecursivePrefixTreeStrategy;
use rucene_spatial_extras::prefix::term_query_prefix_tree_strategy::TermQueryPrefixTreeStrategy;
use rucene_spatial_extras::prefix::tree::geohash_prefix_tree::GeohashPrefixTree;
use rucene_spatial_extras::prefix::tree::quad_prefix_tree::QuadPrefixTree;
use rucene_spatial_extras::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;
use rucene_spatial_extras::query::spatial_args::SpatialArgs;
use rucene_spatial_extras::query::spatial_operation::SpatialOperation;
use rucene_spatial_extras::serialized::serialized_dv_strategy::SerializedDVStrategy;
use rucene_spatial_extras::spatial_strategy::SpatialStrategy;
use rucene_spatial_extras::vector::point_vector_strategy::PointVectorStrategy;
use spatial4j::context::spatial_context::SpatialContext;

use crate::core_support::{class, test};
use crate::spatial_support::random;

/// `TestQueryEqualsHashCode`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.spatial.TestQueryEqualsHashCode",
        vec![test("testEqualsHashCode", test_equals_hash_code).annotate(Test::new())],
    )
}

/// The `Object` an `ObjGenerator` returns.
enum Obj {
    Query(Arc<dyn Query>),
    Source(Arc<dyn DoubleValuesSource>),
}

impl Obj {
    /// `equals(Object)`.
    fn equals(&self, other: &Obj) -> bool {
        match (self, other) {
            (Obj::Query(a), Obj::Query(b)) => a.query_eq(&**b),
            (Obj::Source(a), Obj::Source(b)) => a.source_eq(&**b),
            _ => false,
        }
    }

    /// `toString()`.
    fn to_java_string(&self) -> String {
        match self {
            Obj::Query(q) => q.to_query_string(""),
            Obj::Source(s) => s.to_source_string(),
        }
    }

    /// `hashCode()`.
    fn hash_code(&self) -> i32 {
        match self {
            Obj::Query(q) => q.query_hash(),
            Obj::Source(s) => s.source_hash(),
        }
    }
}

/// `testEqualsHashCode()`.
fn test_equals_hash_code() -> Result<()> {
    let ctx = SpatialContext::geo();
    // `predicate`
    let predicate = match random().next_int_bound(4) {
        // 0-3
        0 => SpatialOperation::Contains,
        1 => SpatialOperation::IsWithin,
        _ => SpatialOperation::Intersects,
    };
    let grid_quad: Arc<dyn SpatialPrefixTree> = QuadPrefixTree::new(Arc::clone(&ctx), 10);
    let grid_geohash: Arc<dyn SpatialPrefixTree> = GeohashPrefixTree::new(Arc::clone(&ctx), 10)?;

    let mut strategies: Vec<Arc<dyn SpatialStrategy>> = Vec::new();
    let recursive_geohash = RecursivePrefixTreeStrategy::new(grid_geohash, "recursive_geohash")?;
    strategies.push(recursive_geohash.clone());
    strategies.push(TermQueryPrefixTreeStrategy::new(
        grid_quad,
        "termquery_quad",
    )?);
    strategies.push(PointVectorStrategy::new_instance(
        Arc::clone(&ctx),
        "pointvector",
    )?);
    strategies.push(BBoxStrategy::new_instance(Arc::clone(&ctx), "bbox")?);
    let serialized = Arc::new(SerializedDVStrategy::new(Arc::clone(&ctx), "serialized")?);
    strategies.push(serialized.clone());
    strategies.push(Arc::new(CompositeSpatialStrategy::new(
        "composite",
        recursive_geohash,
        serialized,
    )?));
    for strategy in &strategies {
        test_equals_hashcode_strategy(&ctx, predicate, strategy)?;
    }
    Ok(())
}

/// `testEqualsHashcode(SpatialStrategy strategy)`.
fn test_equals_hashcode_strategy(
    ctx: &Arc<SpatialContext>,
    predicate: SpatialOperation,
    strategy: &Arc<dyn SpatialStrategy>,
) -> Result<()> {
    let args1 = make_args1(ctx, predicate)?;
    let args2 = make_args2(ctx, predicate)?;
    test_equals_hashcode(&args1, &args2, &|args: &SpatialArgs| {
        Ok(Obj::Query(strategy.make_query(args)?))
    })?;
    test_equals_hashcode(&args1, &args2, &|args: &SpatialArgs| {
        Ok(Obj::Source(strategy.make_distance_value_source_default(
            &args.get_shape().get_center(),
        )?))
    })
}

/// `testEqualsHashcode(SpatialArgs args1, SpatialArgs args2, ObjGenerator
/// generator)`.
fn test_equals_hashcode(
    args1: &SpatialArgs,
    args2: &SpatialArgs,
    generator: &dyn Fn(&SpatialArgs) -> Result<Obj>,
) -> Result<()> {
    let first = match generator(args1) {
        Ok(first) => first,
        Err(e) if e.is_unsupported_operation_exception() => return Ok(()),
        Err(e) => return Err(e),
    };
    // (`if (first == null) return; // unsupported op?`: a generated object is never null)
    let mut second = generator(args1)?; // should be the same
    if !first.equals(&second) {
        // `assertEquals(first, second)`
        Assert::fail(&format!(
            "expected:<{}> but was:<{}>",
            first.to_java_string(),
            second.to_java_string()
        ));
    }
    Assert::assert_equals(first.hash_code(), second.hash_code());
    // `args1.equals(args2)`: SpatialArgs does not override equals (identity)
    Assert::assert_true(!std::ptr::eq(args1, args2));
    second = generator(args2)?; // now should be different
    Assert::assert_true(!first.equals(&second));
    Assert::assert_true(first.hash_code() != second.hash_code());
    Ok(())
}

/// `makeArgs1()`.
fn make_args1(ctx: &SpatialContext, predicate: SpatialOperation) -> Result<SpatialArgs> {
    let shape1 = ctx.get_shape_factory().rect(0.0, 0.0, 10.0, 10.0)?;
    Ok(SpatialArgs::new(predicate, shape1))
}

/// `makeArgs2()`.
fn make_args2(ctx: &SpatialContext, predicate: SpatialOperation) -> Result<SpatialArgs> {
    let shape2 = ctx.get_shape_factory().rect(0.0, 0.0, 20.0, 20.0)?;
    Ok(SpatialArgs::new(predicate, shape2))
}
