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

//! Port of the test classes of Lucene 10.5.0 `lucene/benchmark/src/test`
//! (rmp #633), each run as a suite by the ported runner.

// The ports keep Java's indexed loops and counters line by line.
#![allow(clippy::needless_range_loop)]

mod benchmark_support;

mod by_task {
    pub mod test_perf_tasks_logic;
    pub mod test_perf_tasks_parse;
    pub mod feeds {
        pub mod test_doc_maker;
        pub mod test_enwiki_content_source;
        pub mod test_html_parser;
        pub mod test_line_doc_source;
        pub mod test_trec_content_source;
    }
    pub mod tasks {
        pub mod alt {
            pub mod alt_test_task;
            pub mod test_alt_package_task;
        }
        pub mod counting_search_test_task;
        pub mod test_add_indexes_task;
        pub mod test_commit_index_task;
        pub mod test_create_index_task;
        pub mod test_perf_task;
        pub mod test_search_with_sort_task;
        pub mod test_write_enwiki_line_doc_task;
        pub mod test_write_line_doc_task;
    }
    pub mod utils {
        pub mod test_config;
        pub mod test_stream_utils;
    }
}

mod quality {
    pub mod test_quality_run;
}

use benchmark_support::core_support::run;

macro_rules! suites {
    ($($name:ident => $path:path),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                run($path);
            }
        )*
    };
}

suites! {
    test_perf_tasks_logic => by_task::test_perf_tasks_logic::class_def,
    test_perf_tasks_parse => by_task::test_perf_tasks_parse::class_def,
    test_doc_maker => by_task::feeds::test_doc_maker::class_def,
    test_enwiki_content_source => by_task::feeds::test_enwiki_content_source::class_def,
    test_html_parser => by_task::feeds::test_html_parser::class_def,
    test_line_doc_source => by_task::feeds::test_line_doc_source::class_def,
    test_trec_content_source => by_task::feeds::test_trec_content_source::class_def,
    test_alt_package_task => by_task::tasks::alt::test_alt_package_task::class_def,
    test_add_indexes_task => by_task::tasks::test_add_indexes_task::class_def,
    test_commit_index_task => by_task::tasks::test_commit_index_task::class_def,
    test_create_index_task => by_task::tasks::test_create_index_task::class_def,
    test_perf_task => by_task::tasks::test_perf_task::class_def,
    test_search_with_sort_task => by_task::tasks::test_search_with_sort_task::class_def,
    test_write_enwiki_line_doc_task => by_task::tasks::test_write_enwiki_line_doc_task::class_def,
    test_write_line_doc_task => by_task::tasks::test_write_line_doc_task::class_def,
    test_config => by_task::utils::test_config::class_def,
    test_stream_utils => by_task::utils::test_stream_utils::class_def,
    test_quality_run => quality::test_quality_run::class_def,
}
