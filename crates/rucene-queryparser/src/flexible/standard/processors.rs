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

//! Skeletons of the Java package `org.apache.lucene.queryparser.flexible.standard.processors`.

pub mod allow_leading_wildcard_processor;
pub mod analyzer_query_node_processor;
pub mod boolean_query2_modifier_node_processor;
pub mod boolean_single_child_optimization_query_node_processor;
pub mod boost_query_node_processor;
pub mod default_phrase_slop_query_node_processor;
pub mod fuzzy_query_node_processor;
pub mod interval_query_node_processor;
pub mod match_all_docs_query_node_processor;
pub mod multi_field_query_node_processor;
pub mod multi_term_rewrite_method_processor;
pub mod open_range_query_node_processor;
pub mod phrase_slop_query_node_processor;
pub mod point_query_node_processor;
pub mod point_range_query_node_processor;
pub mod regexp_query_node_processor;
pub mod remove_empty_non_leaf_query_node_processor;
pub mod standard_query_node_processor_pipeline;
pub mod term_range_query_node_processor;
pub mod wildcard_query_node_processor;
