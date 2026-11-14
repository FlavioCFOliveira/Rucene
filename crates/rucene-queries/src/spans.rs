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

//! Skeletons of the Java package `org.apache.lucene.queries.spans`.

pub mod conjunction_spans;
pub mod contain_spans;
pub mod field_masking_span_query;
pub mod filter_spans;
pub mod near_spans_ordered;
pub mod near_spans_unordered;
pub mod span_collector;
pub mod span_contain_query;
pub mod span_containing_query;
pub mod span_disi_priority_queue;
pub mod span_disi_wrapper;
pub mod span_disjunction_disi_approximation;
pub mod span_first_query;
pub mod span_multi_term_query_wrapper;
pub mod span_near_query;
pub mod span_not_query;
pub mod span_or_query;
pub mod span_position_check_query;
pub mod span_position_queue;
pub mod span_position_range_query;
pub mod span_query;
pub mod span_scorer;
pub mod span_term_query;
pub mod span_weight;
pub mod span_within_query;
#[allow(clippy::module_inception)]
pub mod spans;
pub mod term_spans;
