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

//! Port of `TestRandomSpatialOpFuzzyPrefixTree50.java` (package
//! `org.apache.lucene.spatial.prefix`): test
//! RandomSpatialOpFuzzyPrefixTreeTest using the PrefixTree index format
//! found in 5.0 and prior.
//!
//! The anonymous `RecursivePrefixTreeStrategy` subclass of `newRPT()` is
//! [`RecursivePrefixTreeStrategy50`]; the class inherits every test method
//! of `TestRandomSpatialOpFuzzyPrefixTree`, whose instance type it shares.

use std::any::Any;
use std::fmt;
use std::sync::{Arc, LazyLock, Weak};

use randomizedtesting::jdk::reflect::{Class, Constructor};
use rucene::error::Result;
use rucene::index::IndexableField;
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::Query;
use rucene_spatial_extras::prefix::cell_to_bytes_ref_iterator::CellToBytesRefIteratorOps;
use rucene_spatial_extras::prefix::prefix_tree_strategy::{
    prefix_create_indexable_fields, prefix_make_distance_value_source, PrefixTreeStrategy,
    PrefixTreeStrategyBase,
};
use rucene_spatial_extras::prefix::recursive_prefix_tree_strategy::{
    rpt_create_cell_iterator_to_index, rpt_make_query, rpt_to_string,
    RecursivePrefixTreeStrategyBase, RecursivePrefixTreeStrategyOps,
};
use rucene_spatial_extras::prefix::tree::cell_iterator::CellIterator;
use rucene_spatial_extras::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;
use rucene_spatial_extras::query::spatial_args::SpatialArgs;
use rucene_spatial_extras::spatial_strategy::{SpatialStrategy, SpatialStrategyBase};
use spatial4j::shape::point::Point;
use spatial4j::shape::shape::Shape;

use crate::prefix::cell_to_bytes_ref_iterator50::CellToBytesRefIterator50;
use crate::prefix::test_random_spatial_op_fuzzy_prefix_tree::{
    fuzzy_class, TestRandomSpatialOpFuzzyPrefixTree,
};

const NAME: &str = "org.apache.lucene.spatial.prefix.TestRandomSpatialOpFuzzyPrefixTree50";

/// `TestRandomSpatialOpFuzzyPrefixTree50`.
pub fn class_def() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder(NAME)
            .extends(&fuzzy_class())
            .constructor(Constructor::no_args(|| {
                TestRandomSpatialOpFuzzyPrefixTree::new(
                    NAME,
                    "TestRandomSpatialOpFuzzyPrefixTree50",
                    true,
                )
            }))
            .build()
    });
    Arc::clone(&CLASS)
}

/// The anonymous `RecursivePrefixTreeStrategy` subclass of `newRPT()`,
/// which overrides `newCellToBytesRefIterator()`.
#[derive(Debug)]
pub struct RecursivePrefixTreeStrategy50 {
    base: RecursivePrefixTreeStrategyBase,
    me: Weak<RecursivePrefixTreeStrategy50>,
}

impl RecursivePrefixTreeStrategy50 {
    /// `new RecursivePrefixTreeStrategy(this.grid, getClass().getSimpleName()) {…}`.
    pub fn new(
        grid: Arc<dyn SpatialPrefixTree>,
        field_name: &str,
    ) -> Result<Arc<RecursivePrefixTreeStrategy50>> {
        let base = RecursivePrefixTreeStrategyBase::new(grid, field_name)?;
        Ok(Arc::new_cyclic(|me| RecursivePrefixTreeStrategy50 {
            base,
            me: me.clone(),
        }))
    }
}

impl fmt::Display for RecursivePrefixTreeStrategy50 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&rpt_to_string(self))
    }
}

impl SpatialStrategy for RecursivePrefixTreeStrategy50 {
    fn as_recursive_prefix_tree_strategy(&self) -> Option<&dyn RecursivePrefixTreeStrategyOps> {
        Some(self)
    }

    fn strategy_base(&self) -> &SpatialStrategyBase {
        &self.base.prefix.strategy
    }

    fn create_indexable_fields(
        &self,
        shape: &Arc<dyn Shape>,
    ) -> Result<Vec<Arc<dyn IndexableField>>> {
        prefix_create_indexable_fields(self, shape)
    }

    fn make_distance_value_source(
        &self,
        query_point: &Arc<dyn Point>,
        multiplier: f64,
    ) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(prefix_make_distance_value_source(
            self,
            query_point,
            multiplier,
        ))
    }

    fn make_query(&self, args: &SpatialArgs) -> Result<Arc<dyn Query>> {
        rpt_make_query(self, args)
    }
}

impl PrefixTreeStrategy for RecursivePrefixTreeStrategy50 {
    fn prefix_base(&self) -> &PrefixTreeStrategyBase {
        &self.base.prefix
    }

    fn me_prefix(&self) -> Arc<dyn PrefixTreeStrategy> {
        self.me.upgrade().expect("shared as an Arc")
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_cell_iterator_to_index(
        &self,
        shape: &Arc<dyn Shape>,
        detail_level: i32,
    ) -> Result<Box<dyn CellIterator>> {
        rpt_create_cell_iterator_to_index(self, shape, detail_level)
    }

    /// `newCellToBytesRefIterator()`.
    fn new_cell_to_bytes_ref_iterator(&self) -> Box<dyn CellToBytesRefIteratorOps> {
        Box::new(CellToBytesRefIterator50::new())
    }
}

impl RecursivePrefixTreeStrategyOps for RecursivePrefixTreeStrategy50 {
    fn rpt_base(&self) -> &RecursivePrefixTreeStrategyBase {
        &self.base
    }

    /// `getClass().getSimpleName()` of an anonymous class: `""`.
    fn simple_name(&self) -> &'static str {
        ""
    }
}
