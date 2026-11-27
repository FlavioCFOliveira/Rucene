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

//! Port of `TestTermQueryPrefixGridStrategy.java` (package
//! `org.apache.lucene.spatial.prefix`).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::Test;
use rucene::document::{Document, Store, StoredField, StringField};
use rucene::error::Result;
use rucene_spatial_extras::prefix::term_query_prefix_tree_strategy::TermQueryPrefixTreeStrategy;
use rucene_spatial_extras::prefix::tree::quad_prefix_tree::QuadPrefixTree;
use rucene_spatial_extras::spatial_strategy::SpatialStrategy;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::shape::shape::Shape;

use crate::spatial::spatial_test_case::{
    spatial_class, spatial_test_case_class, ti, SpatialCase, SpatialFixture, SpatialTestCaseOps,
};

const NAME: &str = "org.apache.lucene.spatial.prefix.TestTermQueryPrefixGridStrategy";

/// `TestTermQueryPrefixGridStrategy`.
pub fn class_def() -> Arc<Class> {
    type T = TestTermQueryPrefixGridStrategy;
    spatial_class::<T>(
        NAME,
        &spatial_test_case_class(),
        Constructor::no_args(|| TestTermQueryPrefixGridStrategy {
            fx: SpatialFixture::new(NAME),
        }),
        vec![ti::<T>(
            "testNGramPrefixGridLosAngeles",
            T::test_n_gram_prefix_grid_los_angeles,
        )
        .annotate(Test::new())],
    )
}

/// The `TestTermQueryPrefixGridStrategy` instance.
pub struct TestTermQueryPrefixGridStrategy {
    fx: SpatialFixture,
}

impl SpatialCase for TestTermQueryPrefixGridStrategy {
    fn fx(&self) -> &SpatialFixture {
        &self.fx
    }

    fn fx_mut(&mut self) -> &mut SpatialFixture {
        &mut self.fx
    }
}

impl TestTermQueryPrefixGridStrategy {
    /// `testNGramPrefixGridLosAngeles()`.
    fn test_n_gram_prefix_grid_los_angeles(&mut self) -> Result<()> {
        let ctx = SpatialContext::geo();
        let prefix_grid_strategy = TermQueryPrefixTreeStrategy::new(
            QuadPrefixTree::new(Arc::clone(&ctx), QuadPrefixTree::DEFAULT_MAX_LEVELS),
            "geo",
        )?;

        let point: Arc<dyn Shape> = ctx.make_point(-118.243680, 34.052230)?;

        let mut los_angeles = Document::new();
        los_angeles.add(StringField::new(
            "name",
            "Los Angeles".to_string(),
            Store::YES,
        )?);
        for field in prefix_grid_strategy.create_indexable_fields(&point)? {
            los_angeles.add_shared(field);
        }
        los_angeles.add(StoredField::new_string(
            prefix_grid_strategy.get_field_name().to_string(),
            point.to_string(),
        )?); // just for diagnostics

        self.add_documents_and_commit(&[los_angeles])?;

        // This won't work with simple spatial context...
        // SpatialArgsParser spatialArgsParser = new SpatialArgsParser();
        // TODO... use a non polygon query
        //    SpatialArgs spatialArgs = spatialArgsParser.parse(
        //        "Intersects(POLYGON((-127.00390625 39.8125,-112.765625 39.98828125,-111.53515625
        // 31.375,-125.94921875 30.14453125,-127.00390625 39.8125)))",
        //        new SimpleSpatialContext());

        //    Query query = prefixGridStrategy.makeQuery(spatialArgs, fieldInfo);
        //    SearchResults searchResults = executeQuery(query, 1);
        //    assertEquals(1, searchResults.numFound);
        Ok(())
    }
}
