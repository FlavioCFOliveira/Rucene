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

//! The Java package `org.apache.lucene.spatial.prefix.tree`.

pub mod cell;
pub mod cell_can_prune;
pub mod cell_iterator;
pub mod date_range_prefix_tree;
pub mod filter_cell_iterator;
pub mod geohash_prefix_tree;
pub mod legacy_cell;
pub mod legacy_prefix_tree;
pub mod number_range_prefix_tree;
pub mod packed_quad_prefix_tree;
pub mod quad_prefix_tree;
pub mod s2_prefix_tree;
pub mod s2_prefix_tree_cell;
pub mod s2_shape_factory;
pub mod singleton_cell_iterator;
pub mod spatial_prefix_tree;
pub mod spatial_prefix_tree_factory;
pub mod tree_cell_iterator;
