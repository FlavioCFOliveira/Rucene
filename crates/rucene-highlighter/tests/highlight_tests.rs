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
//! `lucene/highlighter/src/test/org/apache/lucene/search/highlight`
//! (rmp #629), each run as a suite by the ported runner.

// The ports keep Java's indexed loops and counters line by line.
#![allow(clippy::needless_range_loop)]

mod highlighter_support;

mod highlight {
    pub mod custom {
        pub mod test_highlight_custom_query;
    }
    pub mod test_highlighter;
    pub mod test_highlighter_phrase;
    pub mod test_misses;
    pub mod test_offset_limit_token_filter;
    pub mod test_token_sources;
}

use highlighter_support::core_support::run;

macro_rules! suites {
    ($($name:ident),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                run(highlight::$name::class_def);
            }
        )*
    };
}

suites!(
    test_highlighter,
    test_highlighter_phrase,
    test_misses,
    test_offset_limit_token_filter,
    test_token_sources,
);

#[test]
fn test_highlight_custom_query() {
    run(highlight::custom::test_highlight_custom_query::class_def);
}
