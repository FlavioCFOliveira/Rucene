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

//! Port of the Java package `org.apache.lucene.search.matchhighlight`.

pub mod break_iterator_shrinking_adjuster;
pub mod char_sequence_iterator;
pub mod field_value_highlighters;
pub mod match_highlighter;
pub mod match_region_retriever;
pub mod offset_range;
pub mod offsets_from_match_iterator;
pub mod offsets_from_positions;
pub mod offsets_from_tokens;
pub mod offsets_from_values;
pub mod offsets_retrieval_strategy;
pub mod offsets_retrieval_strategy_supplier;
pub mod passage;
pub mod passage_adjuster;
pub mod passage_formatter;
pub mod passage_selector;
