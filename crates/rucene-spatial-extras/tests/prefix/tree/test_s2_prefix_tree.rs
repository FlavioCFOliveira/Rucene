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

//! Port of `TestS2PrefixTree.java` (package
//! `org.apache.lucene.spatial.prefix.tree`): test for S2 Spatial prefix
//! tree.

use std::sync::Arc;

use randomizedtesting::annotations::Repeat;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{Assert, Test};
use rucene::error::Result;
use rucene_spatial_extras::prefix::tree::cell::Cell;
use rucene_spatial_extras::prefix::tree::s2_prefix_tree::S2PrefixTree;
use rucene_spatial_extras::prefix::tree::s2_prefix_tree_cell::S2PrefixTreeCell;
use rucene_spatial_extras::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;
use rucene_spatial_extras::spatial4j::geo3d_spatial_context_factory::Geo3dSpatialContextFactory;
use s2_geometry::s2_cell_id::S2CellId;
use s2_geometry::s2_projections::S2Projections;
use spatial4j::context::spatial_context_factory::SpatialContextFactoryOps;

use crate::core_support::{class, test};
use crate::spatial_support::random;

/// `TestS2PrefixTree`.
pub fn class_def() -> Arc<Class> {
    let repeat = || Repeat {
        iterations: 10,
        use_constant_seed: false,
    };
    class(
        "org.apache.lucene.spatial.prefix.tree.TestS2PrefixTree",
        vec![
            test("testCells", test_cells)
                .annotate(Test::new())
                .annotate(repeat()),
            test("testDistanceAndLevels", test_distance_and_levels)
                .annotate(Test::new())
                .annotate(repeat()),
            test("testPrecision", test_precision)
                .annotate(Test::new())
                .annotate(repeat()),
        ],
    )
}

/// `testCells()`.
fn test_cells() -> Result<()> {
    let face = random().next_int_bound(6);
    let mut id = S2CellId::from_face_pos_level(face, 0, 0);
    let arity = random().next_int_bound(3) + 1;
    let mut level = random().next_int_bound(S2PrefixTree::get_max_levels_for_arity(arity));
    level *= arity;
    for _ in 0..level {
        let pos = random().next_int_bound(4);
        id = id.child_begin();
        if pos == 0 {
            continue;
        }
        id = id.next();
        if pos == 1 {
            continue;
        }
        id = id.next();
        if pos == 2 {
            continue;
        }
        id = id.next();
    }
    let tree = S2PrefixTree::with_arity(
        Geo3dSpatialContextFactory::new().new_spatial_context()?,
        S2PrefixTree::get_max_levels_for_arity(arity),
        arity,
    )?;
    let cell = S2PrefixTreeCell::new(&tree, Some(id));
    let mut r = cell.get_token_bytes_with_leaf(None);
    if random().next_boolean() {
        let new_offset = random().next_int_bound(10) as usize + 1;
        let mut new_bytes = vec![0u8; r.bytes.len() + new_offset];
        new_bytes[new_offset..].copy_from_slice(&r.bytes);
        r.bytes = new_bytes;
        r.offset += new_offset;
    }
    let cell2 = S2PrefixTreeCell::new(&tree, None);
    cell2.read_cell(&tree, &r);
    if !cell.cell_equals(&cell2) {
        // `assertEquals(cell, cell2)`
        Assert::fail(&format!("expected:<{cell}> but was:<{cell2}>"));
    }
    Ok(())
}

/// `testDistanceAndLevels()`.
fn test_distance_and_levels() -> Result<()> {
    let mut tree = S2PrefixTree::with_arity(
        Geo3dSpatialContextFactory::new().new_spatial_context()?,
        S2PrefixTree::get_max_levels_for_arity(1),
        1,
    )?;

    let random_dist = random().next_double() * 5.0;
    let mut level_distance = tree.get_level_for_distance(random_dist);
    let mut distance_level = tree.get_distance_for_level(level_distance)?;
    Assert::assert_true(random_dist > distance_level);

    tree = S2PrefixTree::with_arity(
        Geo3dSpatialContextFactory::new().new_spatial_context()?,
        S2PrefixTree::get_max_levels_for_arity(2),
        2,
    )?;

    level_distance = tree.get_level_for_distance(random_dist);
    distance_level = tree.get_distance_for_level(level_distance)?;
    Assert::assert_true(random_dist > distance_level);

    tree = S2PrefixTree::with_arity(
        Geo3dSpatialContextFactory::new().new_spatial_context()?,
        S2PrefixTree::get_max_levels_for_arity(3),
        3,
    )?;

    level_distance = tree.get_level_for_distance(random_dist);
    distance_level = tree.get_distance_for_level(level_distance)?;
    Assert::assert_true(random_dist > distance_level);
    Ok(())
}

/// `testPrecision()`.
fn test_precision() -> Result<()> {
    let arity = random().next_int_bound(3) + 1;
    let context = Geo3dSpatialContextFactory::new().new_spatial_context()?;
    let tree = S2PrefixTree::with_arity(
        Arc::clone(&context),
        S2PrefixTree::get_max_levels_for_arity(arity),
        arity,
    )?;
    let precision = random().next_double();
    let level = tree.get_level_for_distance(precision);
    let point = context.get_shape_factory().point_xy(0.0, 0.0)?;
    let mut iterator = tree.get_tree_cell_iterator(point, level)?;
    let mut cell = None;
    while iterator.has_next() {
        cell = Some(iterator.next());
    }
    let cell = cell.expect("java.lang.NullPointerException: cell");
    let s2_cell = cell
        .as_any()
        .downcast_ref::<S2PrefixTreeCell>()
        .expect("java.lang.ClassCastException: an S2PrefixTreeCell");
    Assert::assert_true(s2_cell.get_level() == level);
    let precision_cell = S2Projections::MAX_WIDTH.get_value(
        s2_cell
            .cell_id()
            .expect("java.lang.NullPointerException: cellId")
            .level(),
    );
    Assert::assert_true(precision > precision_cell);
    Ok(())
}
