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

//! Skeletons of the Java package `org.apache.lucene.queries.intervals`.

pub mod block_intervals_source;
pub mod caching_matches_iterator;
pub mod conjunction_interval_iterator;
pub mod conjunction_intervals_source;
pub mod conjunction_matches_iterator;
pub mod contained_by_intervals_source;
pub mod containing_intervals_source;
pub mod difference_intervals_source;
pub mod disi_priority_queue;
pub mod disi_wrapper;
pub mod disjunction_disi_approximation;
pub mod disjunction_intervals_source;
pub mod disjunctions;
pub mod extended_interval_iterator;
pub mod extended_intervals_source;
pub mod filtered_intervals_source;
pub mod filtering_interval_iterator;
pub mod fixed_field_intervals_source;
pub mod interval_builder;
pub mod interval_filter;
pub mod interval_iterator;
pub mod interval_matches;
pub mod interval_matches_iterator;
pub mod interval_query;
pub mod interval_score_function;
pub mod interval_scorer;
#[allow(clippy::module_inception)]
pub mod intervals;
pub mod intervals_source;
pub mod minimizing_conjunction_intervals_source;
pub mod minimum_should_match_intervals_source;
pub mod multi_term_intervals_source;
pub mod no_match_intervals_source;
pub mod non_overlapping_intervals_source;
pub mod not_contained_by_intervals_source;
pub mod not_containing_intervals_source;
pub mod offset_intervals_source;
pub mod ordered_intervals_source;
pub mod overlapping_intervals_source;
pub mod payload_filtered_term_intervals_source;
pub mod relative_iterator;
pub mod repeating_intervals_source;
pub mod term_intervals_source;
pub mod unordered_intervals_source;
