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
//! `lucene/highlighter/src/test/org/apache/lucene/search/vectorhighlight`
//! (rmp #629), each run as a suite by the ported runner.

// The ports keep Java's indexed loops and counters line by line.
#![allow(clippy::needless_range_loop)]

mod highlighter_support;

mod vectorhighlight {
    pub mod abstract_test_case;
    pub mod test_field_phrase_list;
    pub mod test_field_query;
    pub mod test_field_term_stack;
    pub mod test_index_time_synonym;
    pub mod test_score_order_fragments_builder;
    pub mod test_simple_boundary_scanner;
    pub mod test_simple_frag_list_builder;
    pub mod test_simple_fragments_builder;
    pub mod test_single_frag_list_builder;
    pub mod test_weighted_frag_list_builder;
}

use highlighter_support::core_support::run;

macro_rules! suites {
    ($($name:ident),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                run(vectorhighlight::$name::class_def);
            }
        )*
    };
}

suites!(
    test_field_phrase_list,
    test_field_query,
    test_field_term_stack,
    test_index_time_synonym,
    test_score_order_fragments_builder,
    test_simple_boundary_scanner,
    test_simple_frag_list_builder,
    test_simple_fragments_builder,
    test_single_frag_list_builder,
    test_weighted_frag_list_builder,
);
