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

//! Port of the test classes of Lucene 10.5.0
//! `lucene/queries/src/test/org/apache/lucene/queries/function/` that rmp
//! tasks #478 and #479 list, each run as a suite by the ported runner.
//!
//! * #478 (`function`, `function.docvalues`): `TestBoolValOfNumericDVs`,
//!   `TestFunctionScoreExplanations`, `TestIndexReaderFunctions`.
//! * #479 (`function.valuesource`): `TestDocValuesFieldSources`,
//!   `TestFieldScoreQuery`, `TestFunctionMatchQuery`,
//!   `TestFunctionQueryExplanations`, `TestFunctionQuerySort`,
//!   `TestFunctionRangeQuery`, `TestFunctionScoreQuery`,
//!   `TestKnnVectorSimilarityFunctions`, `TestLongNormValueSource`,
//!   `TestSortedSetFieldSource`, `TestValueSources`; the test helper
//!   `FunctionTestSetup` is [`function::function_test_setup`].

// The ports keep Java's indexed loops and counters line by line.
#![allow(clippy::needless_range_loop)]

#[macro_use]
mod queries_support;

mod function {
    pub mod docvalues {
        pub mod test_bool_val_of_numeric_dvs;
    }
    pub mod function_test_setup;
    pub mod test_doc_values_field_sources;
    pub mod test_field_score_query;
    pub mod test_function_match_query;
    pub mod test_function_query_explanations;
    pub mod test_function_query_sort;
    pub mod test_function_range_query;
    pub mod test_function_score_explanations;
    pub mod test_function_score_query;
    pub mod test_index_reader_functions;
    pub mod test_knn_vector_similarity_functions;
    pub mod test_long_norm_value_source;
    pub mod test_sorted_set_field_source;
    pub mod test_value_sources;
}

use queries_support::core_support::run;

#[test]
fn test_bool_val_of_numeric_dvs() {
    run(function::docvalues::test_bool_val_of_numeric_dvs::class_def);
}

#[test]
fn test_doc_values_field_sources() {
    run(function::test_doc_values_field_sources::class_def);
}

#[test]
fn test_field_score_query() {
    run(function::test_field_score_query::class_def);
}

#[test]
fn test_function_match_query() {
    run(function::test_function_match_query::class_def);
}

#[test]
fn test_function_query_explanations() {
    run(function::test_function_query_explanations::class_def);
}

#[test]
fn test_function_query_sort() {
    run(function::test_function_query_sort::class_def);
}

#[test]
fn test_function_range_query() {
    run(function::test_function_range_query::class_def);
}

#[test]
fn test_function_score_explanations() {
    run(function::test_function_score_explanations::class_def);
}

#[test]
fn test_function_score_query() {
    run(function::test_function_score_query::class_def);
}

#[test]
fn test_index_reader_functions() {
    run(function::test_index_reader_functions::class_def);
}

#[test]
fn test_knn_vector_similarity_functions() {
    run(function::test_knn_vector_similarity_functions::class_def);
}

#[test]
fn test_long_norm_value_source() {
    run(function::test_long_norm_value_source::class_def);
}

#[test]
fn test_sorted_set_field_source() {
    run(function::test_sorted_set_field_source::class_def);
}

#[test]
fn test_value_sources() {
    run(function::test_value_sources::class_def);
}
