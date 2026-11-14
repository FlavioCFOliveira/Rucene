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

//! Skeletons of the Java package `org.apache.lucene.queryparser.xml.builders`.

pub mod boolean_query_builder;
pub mod boosting_term_builder;
pub mod constant_score_query_builder;
pub mod disjunction_max_query_builder;
pub mod fuzzy_like_this_query_builder;
pub mod like_this_query_builder;
pub mod match_all_docs_query_builder;
pub mod point_range_query_builder;
pub mod range_query_builder;
pub mod span_builder_base;
pub mod span_first_builder;
pub mod span_near_builder;
pub mod span_not_builder;
pub mod span_or_builder;
pub mod span_or_terms_builder;
pub mod span_position_range_builder;
pub mod span_query_builder;
pub mod span_query_builder_factory;
pub mod span_term_builder;
pub mod term_query_builder;
pub mod terms_query_builder;
pub mod user_input_query_builder;
