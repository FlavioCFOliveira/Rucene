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

//! The Java package `org.apache.lucene.spatial.prefix`.

pub mod abstract_prefix_tree_query;
pub mod abstract_visiting_prefix_tree_query;
pub mod bytes_ref_iterator_token_stream;
pub mod cell_to_bytes_ref_iterator;
pub mod contains_prefix_tree_query;
pub mod heatmap_facet_counter;
pub mod intersects_prefix_tree_query;
pub mod number_range_prefix_tree_strategy;
pub mod point_prefix_tree_field_cache_provider;
pub mod prefix_tree_facet_counter;
pub mod prefix_tree_strategy;
pub mod recursive_prefix_tree_strategy;
pub mod term_query_prefix_tree_strategy;
pub mod tree;
pub mod within_prefix_tree_query;
