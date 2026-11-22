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

//! Skeletons of the Java package `org.apache.lucene.search.uhighlight`.

pub mod analysis_offset_strategy;
pub mod char_array_matcher;
pub mod custom_separator_break_iterator;
pub mod default_passage_formatter;
pub mod field_highlighter;
pub mod field_offset_strategy;
pub mod labelled_char_array_matcher;
pub mod length_goal_break_iterator;
pub mod memory_index_offset_strategy;
pub mod multi_fields_offset_strategy;
pub mod multi_term_highlighting;
pub mod no_op_offset_strategy;
pub mod offsets_enum;
pub mod overlay_single_doc_terms_leaf_reader;
pub mod passage;
pub mod passage_formatter;
pub mod passage_scorer;
pub mod phrase_helper;
pub mod postings_offset_strategy;
pub mod postings_with_term_vectors_offset_strategy;
pub mod splitting_break_iterator;
pub mod term_vector_filtered_leaf_reader;
pub mod term_vector_offset_strategy;
pub mod token_stream_offset_strategy;
pub mod uh_components;
pub mod unified_highlighter;
pub mod whole_break_iterator;
