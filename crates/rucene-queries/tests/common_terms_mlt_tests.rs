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

//! Port of the test classes of Lucene 10.5.0 `lucene/queries/src/test` that
//! rmp task #477 lists, each run as a suite by the ported runner:
//! `org.apache.lucene.queries.TestCommonTermsQuery` and
//! `org.apache.lucene.queries.mlt.TestMoreLikeThis`.

// The ports keep Java's indexed loops and counters line by line.
#![allow(clippy::needless_range_loop)]

#[macro_use]
mod queries_support;

mod queries {
    pub mod test_common_terms_query;
}

mod mlt {
    pub mod test_more_like_this;
}

use queries_support::core_support::run;

#[test]
fn test_common_terms_query() {
    run(queries::test_common_terms_query::class_def);
}

#[test]
fn test_more_like_this() {
    run(mlt::test_more_like_this::class_def);
}
