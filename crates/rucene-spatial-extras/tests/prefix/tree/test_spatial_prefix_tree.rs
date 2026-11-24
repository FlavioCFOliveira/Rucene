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

//! Port of `TestSpatialPrefixTree.java` (package
//! `org.apache.lucene.spatial.prefix.tree`).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::{Assert, Test};
use rucene::document::{Document, Store, TextField};
use rucene::error::Result;
use rucene_spatial_extras::prefix::prefix_tree_strategy::PrefixTreeStrategy;
use rucene_spatial_extras::prefix::term_query_prefix_tree_strategy::TermQueryPrefixTreeStrategy;
use rucene_spatial_extras::prefix::tree::cell::CellRef;
use rucene_spatial_extras::prefix::tree::geohash_prefix_tree::GeohashPrefixTree;
use rucene_spatial_extras::prefix::tree::quad_prefix_tree::QuadPrefixTree;
use rucene_spatial_extras::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;
use rucene_spatial_extras::query::spatial_args::SpatialArgs;
use rucene_spatial_extras::query::spatial_operation::SpatialOperation;
use rucene_spatial_extras::spatial_strategy::SpatialStrategy;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::shape::randomized_shape_test::assert_equals_shape;
use spatial4j::shape::shape::Shape;

use crate::spatial::spatial_test_case::{
    spatial_class, spatial_set_up, spatial_test_case_class, ti, SpatialCase, SpatialFixture,
    SpatialTestCaseOps,
};
use crate::spatial_support::random;

const NAME: &str = "org.apache.lucene.spatial.prefix.tree.TestSpatialPrefixTree";

/// `TestSpatialPrefixTree`.
pub fn class_def() -> Arc<Class> {
    type T = TestSpatialPrefixTree;
    spatial_class::<T>(
        NAME,
        &spatial_test_case_class(),
        Constructor::no_args(|| TestSpatialPrefixTree {
            fx: SpatialFixture::new(NAME),
            ctx: None,
            trie: None,
        }),
        vec![
            ti::<T>("testCellTraverse", T::test_cell_traverse).annotate(Test::new()),
            ti::<T>("testBadPrefixTreePrune", T::test_bad_prefix_tree_prune).annotate(Test::new()),
        ],
    )
}

/// The `TestSpatialPrefixTree` instance.
pub struct TestSpatialPrefixTree {
    fx: SpatialFixture,
    // TODO plug in others and test them
    /// `ctx` (`private`, shadowing `SpatialTestCase.ctx`).
    ctx: Option<Arc<SpatialContext>>,
    /// `trie`.
    trie: Option<Arc<dyn SpatialPrefixTree>>,
}

impl SpatialCase for TestSpatialPrefixTree {
    fn fx(&self) -> &SpatialFixture {
        &self.fx
    }

    fn fx_mut(&mut self) -> &mut SpatialFixture {
        &mut self.fx
    }

    /// `setUp()`.
    fn set_up(&mut self) -> Result<()> {
        spatial_set_up(&mut self.fx)?;
        self.ctx = Some(SpatialContext::geo());
        Ok(())
    }
}

impl TestSpatialPrefixTree {
    fn ctx(&self) -> Arc<SpatialContext> {
        Arc::clone(self.ctx.as_ref().expect("setUp() sets ctx"))
    }

    /// `testCellTraverse()`.
    fn test_cell_traverse(&mut self) -> Result<()> {
        let ctx = self.ctx();
        let trie: Arc<dyn SpatialPrefixTree> = GeohashPrefixTree::new(Arc::clone(&ctx), 4)?;
        self.trie = Some(Arc::clone(&trie));

        let mut c: CellRef = trie.get_world_cell();
        Assert::assert_equals(0, c.get_level());
        assert_equals_shape(None, ctx.get_world_bounds().as_shape(), &*c.get_shape());
        while c.get_level() < trie.get_max_levels() {
            let prev_c = c;
            let mut sub_cells: Vec<CellRef> = Vec::new();
            let mut sub_cells_iter = prev_c.get_next_level_cells(None);
            while sub_cells_iter.has_next() {
                sub_cells.push(sub_cells_iter.next());
            }
            c = Arc::clone(
                &sub_cells[random().next_int_bound(sub_cells.len() as i32 - 1) as usize],
            );

            Assert::assert_equals(prev_c.get_level() + 1, c.get_level());
            let prev_n_shape_shape = prev_c.get_shape();
            let prev_n_shape = prev_n_shape_shape
                .as_rectangle()
                .expect("java.lang.ClassCastException: the cell shape is a Rectangle");
            let s = c.get_shape();
            let sbox = s.get_bounding_box();
            Assert::assert_true(prev_n_shape.get_width() > sbox.get_width());
            Assert::assert_true(prev_n_shape.get_height() > sbox.get_height());
        }
        Ok(())
    }

    /// `testBadPrefixTreePrune()`: a PrefixTree pruning optimization gone
    /// bad, applicable when optimize=true. See
    /// <https://issues.apache.org/jira/browse/LUCENE-4770>.
    fn test_bad_prefix_tree_prune(&mut self) -> Result<()> {
        let ctx = self.ctx();
        let trie: Arc<dyn SpatialPrefixTree> = QuadPrefixTree::new(Arc::clone(&ctx), 12);
        self.trie = Some(Arc::clone(&trie));
        let strategy = TermQueryPrefixTreeStrategy::new(trie, "geo")?;
        let mut doc = Document::new();
        doc.add(TextField::new("id", "1".to_string(), Store::YES)?);

        let area: Arc<dyn Shape> = ctx.make_rectangle(-122.82, -122.78, 48.54, 48.56)?;

        let fields = strategy.create_indexable_fields_with_dist_err(&area, 0.025)?;
        for field in fields {
            doc.add_shared(field);
        }
        self.add_document(&doc)?;

        let upperleft = ctx.make_point(-122.88, 48.54)?;
        let lowerright = ctx.make_point(-122.82, 48.62)?;

        let query = strategy.make_query(&SpatialArgs::new(
            SpatialOperation::Intersects,
            ctx.make_rectangle_from_points(&*upperleft, &*lowerright)?,
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
