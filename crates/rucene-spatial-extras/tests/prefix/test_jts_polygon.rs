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

//! Port of `TestJtsPolygon.java` (package `org.apache.lucene.spatial.prefix`).
//!
//! The test needs spatial4j's `JtsSpatialContextFactory`, which needs the
//! JTS library. Java's constructor turns a missing JTS jar
//! (`NoClassDefFoundError`) into a failed assumption. The spatial4j port
//! excludes the JTS-dependent package `org.locationtech.spatial4j.context.jts`
//! and JTS itself (user decision, sprint 17), so loading the factory fails
//! with `ClassNotFoundException`; the constructor treats that exactly as
//! Java treats the missing jar.

use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::{Assert, Test};
use rucene::document::{Document, Store, TextField};
use rucene::error::{LuceneError, Result};
use rucene_spatial_extras::prefix::prefix_tree_strategy::PrefixTreeStrategy;
use rucene_spatial_extras::prefix::recursive_prefix_tree_strategy::RecursivePrefixTreeStrategy;
use rucene_spatial_extras::prefix::term_query_prefix_tree_strategy::TermQueryPrefixTreeStrategy;
use rucene_spatial_extras::prefix::tree::geohash_prefix_tree::GeohashPrefixTree;
use rucene_spatial_extras::prefix::tree::quad_prefix_tree::QuadPrefixTree;
use rucene_spatial_extras::query::spatial_args::SpatialArgs;
use rucene_spatial_extras::query::spatial_operation::SpatialOperation;
use rucene_spatial_extras::spatial_strategy::SpatialStrategy;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::LuceneTestCase;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::context::spatial_context_factory::SpatialContextFactory;
use spatial4j::java_map::StringMap;
use spatial4j::shape::shape::Shape;

use crate::spatial::spatial_test_case::{
    spatial_class, strategy_test_case_class, ti, SpatialCase, SpatialFixture, SpatialTestCaseOps,
};
use crate::spatial::strategy_test_case::StrategyTestCaseOps;

const NAME: &str = "org.apache.lucene.spatial.prefix.TestJtsPolygon";
/// `getClass().getSimpleName()`.
const SIMPLE_NAME: &str = "TestJtsPolygon";

/// `LUCENE_4464_distErrPct` (`private static final`).
const LUCENE_4464_DIST_ERR_PCT: f64 = SpatialArgs::DEFAULT_DISTERRPCT; // DEFAULT 2.5%

/// The JTS context factory class `spatialContextFactory` names.
const JTS_FACTORY: &str = "org.locationtech.spatial4j.context.jts.JtsSpatialContextFactory";

/// `TestJtsPolygon`.
pub fn class_def() -> Arc<Class> {
    type T = TestJtsPolygon;
    spatial_class::<T>(
        NAME,
        &strategy_test_case_class(),
        Constructor::no_args(TestJtsPolygon::new),
        vec![
            ti::<T>("testCloseButNoMatch", T::test_close_but_no_match).annotate(Test::new()),
            ti::<T>("testBadPrefixTreePrune", T::test_bad_prefix_tree_prune).annotate(Test::new()),
        ],
    )
}

/// The `TestJtsPolygon` instance.
pub struct TestJtsPolygon {
    fx: SpatialFixture,
}

impl SpatialCase for TestJtsPolygon {
    fn fx(&self) -> &SpatialFixture {
        &self.fx
    }

    fn fx_mut(&mut self) -> &mut SpatialFixture {
        &mut self.fx
    }
}

/// Whether `e` is the failure to load the JTS context factory: Java's
/// `NoClassDefFoundError` of a missing JTS jar (see the module docs).
fn is_missing_jts(e: &LuceneError) -> bool {
    let text = format!("{e:?} {e}");
    text.contains("java.lang.ClassNotFoundException") && text.contains(JTS_FACTORY)
}

impl TestJtsPolygon {
    /// `TestJtsPolygon()`.
    fn new() -> Self {
        let mut fx = SpatialFixture::new(NAME);
        let mut args: HashMap<String, String> = HashMap::new();
        args.insert("spatialContextFactory".to_string(), JTS_FACTORY.to_string());
        let args: Arc<dyn StringMap> = Arc::new(args);
        match SpatialContextFactory::make_spatial_context(&args) {
            Ok(ctx) => fx.ctx = Some(ctx),
            Err(e) if is_missing_jts(&e) => {
                LuceneTestCase::assume_true(&format!("This test requires JTS jar: {e}"), false);
            }
            Err(e) => rethrow_java(e),
        }

        let build = || -> Result<Arc<RecursivePrefixTreeStrategy>> {
            let grid = GeohashPrefixTree::new(fx.ctx(), 11)?; // < 1 meter == 11 maxLevels
            let strategy = RecursivePrefixTreeStrategy::new(grid, SIMPLE_NAME)?;
            strategy.set_dist_err_pct(LUCENE_4464_DIST_ERR_PCT); // 1% radius (small!)
            Ok(strategy)
        };
        let strategy = build().unwrap_or_else(|e| rethrow_java(e));
        fx.strategy = Some(strategy);
        Self { fx }
    }

    /// `testCloseButNoMatch()` (LUCENE-4464).
    fn test_close_but_no_match(&mut self) -> Result<()> {
        self.get_add_and_verify_indexed_documents("LUCENE-4464.txt")?;
        let args = self.q(
            "POLYGON((-93.18100824442227 45.25676372469945,-93.23182001200654 45.21421290799412,-93.16315546122038 45.23742639412364,-93.18100824442227 45.25676372469945))",
            LUCENE_4464_DIST_ERR_PCT,
        )?;
        let got = self.execute_query(self.fx.strategy().make_query(&args)?, 100)?;
        Assert::assert_equals(1i64, got.num_found);
        Assert::assert_equals_str(
            None,
            Some("poly2"),
            got.results[0].document.get("id").as_deref(),
        );
        // did not find poly 1 !
        Ok(())
    }

    /// `q(String shapeStr, double distErrPct)`.
    fn q(&self, shape_str: &str, dist_err_pct: f64) -> Result<SpatialArgs> {
        let shape = self.fx.ctx().read_shape_from_wkt(shape_str)?;
        let args = SpatialArgs::new(SpatialOperation::Intersects, shape);
        args.set_dist_err_pct(Some(dist_err_pct));
        Ok(args)
    }

    /// `testBadPrefixTreePrune()`: a PrefixTree pruning optimization gone
    /// bad. See <https://issues.apache.org/jira/browse/LUCENE-4770>.
    fn test_bad_prefix_tree_prune(&mut self) -> Result<()> {
        let ctx: Arc<SpatialContext> = self.fx.ctx();
        let area: Arc<dyn Shape> = ctx.read_shape_from_wkt(
            "POLYGON((-122.83 48.57, -122.77 48.56, -122.79 48.53, -122.83 48.57))",
        )?;

        let trie = QuadPrefixTree::new(Arc::clone(&ctx), 12);
        let strategy = TermQueryPrefixTreeStrategy::new(trie, "geo")?;
        let mut doc = Document::new();
        doc.add(TextField::new("id", "1".to_string(), Store::YES)?);

        let fields = strategy.create_indexable_fields_with_dist_err(&area, 0.025)?;
        for field in fields {
            doc.add_shared(field);
        }
        self.add_document(&doc)?;

        let upperleft = ctx.get_shape_factory().point_xy(-122.88, 48.54)?;
        let lowerright = ctx.get_shape_factory().point_xy(-122.82, 48.62)?;

        let query = strategy.make_query(&SpatialArgs::new(
            SpatialOperation::Intersects,
            ctx.get_shape_factory()
                .rect_from_points(&*upperleft, &*lowerright)?,
        ))?;
        self.commit()?;

        let search = self.fx.index_searcher().search(query, 10)?;
        let score_docs = &search.score_docs;
        for score_doc in score_docs {
            randomizedtesting::jdk::system::out_println(
                &self
                    .fx
                    .index_searcher()
                    .stored_fields()?
                    .document(score_doc.doc)?
                    .to_string(),
            );
        }

        Assert::assert_equals(1i64, search.total_hits.value());
        Ok(())
    }
}
