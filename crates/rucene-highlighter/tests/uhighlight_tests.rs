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
//! `lucene/highlighter/src/test/org/apache/lucene/search/uhighlight`
//! (rmp #629), each run as a suite by the ported runner.

// The ports keep Java's indexed loops and counters line by line.
#![allow(clippy::needless_range_loop)]

mod highlighter_support;

mod uhighlight {
    pub mod test_custom_separator_break_iterator;
    pub mod test_default_passage_formatter;
    pub mod test_length_goal_break_iterator;
    pub mod test_splitting_break_iterator;
    pub mod test_whole_break_iterator;
    pub mod test_unified_highlighter;
    pub mod unified_highlighter_test_base;
}

use highlighter_support::core_support::run;

macro_rules! suites {
    ($($name:ident),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                run(uhighlight::$name::class_def);
            }
        )*
    };
}

suites!(
    test_custom_separator_break_iterator,
    test_default_passage_formatter,
    test_length_goal_break_iterator,
    test_splitting_break_iterator,
    test_unified_highlighter,
    test_whole_break_iterator,
);
