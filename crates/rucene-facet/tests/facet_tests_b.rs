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

//! Port of the test classes of Lucene 10.5.0 `lucene/facet/src/test`
//! packages `range`, `rangeonrange`, `facetset` and `sortedset` (rmp #630,
//! lane A), each run as a suite by the ported runner.

#[path = "facet_support/mod.rs"]
mod facet_support;

mod facetset {
    pub mod test_exact_facet_set_matcher;
    pub mod test_matching_facet_sets_counts;
    pub mod test_range_facet_set_matcher;
}

mod range {
    pub mod test_dynamic_range_util;
    pub mod test_range_facet_counts;
}

mod rangeonrange {
    pub mod test_range_on_range_facet_counts;
}

mod sortedset {
    pub mod test_sorted_set_doc_values_facets;
}

use facet_support::core_support::run;

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
    test_exact_facet_set_matcher => facetset::test_exact_facet_set_matcher::class_def,
    test_matching_facet_sets_counts => facetset::test_matching_facet_sets_counts::class_def,
    test_range_facet_set_matcher => facetset::test_range_facet_set_matcher::class_def,
    test_dynamic_range_util => range::test_dynamic_range_util::class_def,
    test_range_facet_counts => range::test_range_facet_counts::class_def,
    test_range_on_range_facet_counts => rangeonrange::test_range_on_range_facet_counts::class_def,
    test_sorted_set_doc_values_facets => sortedset::test_sorted_set_doc_values_facets::class_def,
}
