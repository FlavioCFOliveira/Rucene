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

//! The `Intervals` factories as the Java tests call them (with `import
//! static org.apache.lucene.queries.intervals.Intervals.*`), and the JUnit
//! assertions over `IntervalsSource` objects.
//!
//! The factories Java declares without a checked exception return a
//! `Result` here; a failure is the unchecked exception it stands for
//! ([`unchecked`]).

#![allow(dead_code)]

use std::fmt;
use std::sync::Arc;

use randomizedtesting::junit::Assert;
use rucene::util::BytesRef;
use rucene_queries::intervals::intervals::Intervals;
use rucene_queries::intervals::intervals_source::IntervalsSource;

use crate::queries_support::core_support::unchecked;

/// An `IntervalsSource` compared with `equals(Object)` and printed with
/// `toString()`, as JUnit's `assertEquals(Object, Object)` does.
#[derive(Clone)]
pub struct Src(pub Arc<dyn IntervalsSource>);

impl PartialEq for Src {
    fn eq(&self, other: &Self) -> bool {
        self.0.source_eq(&*other.0)
    }
}

impl fmt::Display for Src {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.to_source_string())
    }
}

/// `assertEquals(Object expected, Object actual)` over two sources.
pub fn assert_source_equals(
    expected: &Arc<dyn IntervalsSource>,
    actual: &Arc<dyn IntervalsSource>,
) {
    Assert::assert_equals(Src(Arc::clone(expected)), Src(Arc::clone(actual)));
}

/// `assertNotEquals(Object unexpected, Object actual)` over two sources:
/// JUnit's `Values should be different. Actual: <actual>`.
pub fn assert_source_not_equals(
    unexpected: &Arc<dyn IntervalsSource>,
    actual: &Arc<dyn IntervalsSource>,
) {
    if unexpected.source_eq(&**actual) {
        Assert::fail(&format!(
            "Values should be different. Actual: {}",
            actual.to_source_string()
        ));
    }
}

/// `Intervals.term(String)`.
pub fn term(t: &str) -> Arc<dyn IntervalsSource> {
    Intervals::term_str(t)
}

/// `Intervals.phrase(String...)`.
pub fn phrase_s(terms: &[&str]) -> Arc<dyn IntervalsSource> {
    unchecked(Intervals::phrase_terms(terms))
}

/// `Intervals.phrase(IntervalsSource...)`.
pub fn phrase(sources: Vec<Arc<dyn IntervalsSource>>) -> Arc<dyn IntervalsSource> {
    unchecked(Intervals::phrase(sources))
}

/// `Intervals.or(IntervalsSource...)`.
pub fn or(sources: Vec<Arc<dyn IntervalsSource>>) -> Arc<dyn IntervalsSource> {
    unchecked(Intervals::or(sources))
}

/// `Intervals.or(boolean, IntervalsSource...)`.
pub fn or_rewrite(
    rewrite: bool,
    sources: Vec<Arc<dyn IntervalsSource>>,
) -> Arc<dyn IntervalsSource> {
    unchecked(Intervals::or_with_rewrite(rewrite, sources))
}

/// `Intervals.ordered(IntervalsSource...)`.
pub fn ordered(sources: Vec<Arc<dyn IntervalsSource>>) -> Arc<dyn IntervalsSource> {
    Intervals::ordered(sources)
}

/// `Intervals.unordered(IntervalsSource...)`.
pub fn unordered(sources: Vec<Arc<dyn IntervalsSource>>) -> Arc<dyn IntervalsSource> {
    Intervals::unordered(sources)
}

/// `Intervals.unorderedNoOverlaps(IntervalsSource, IntervalsSource)`.
pub fn unordered_no_overlaps(
    a: Arc<dyn IntervalsSource>,
    b: Arc<dyn IntervalsSource>,
) -> Arc<dyn IntervalsSource> {
    unchecked(Intervals::unordered_no_overlaps(a, b))
}

/// `Intervals.maxgaps(int, IntervalsSource)`.
pub fn maxgaps(gaps: i32, source: Arc<dyn IntervalsSource>) -> Arc<dyn IntervalsSource> {
    unchecked(Intervals::maxgaps(gaps, source))
}

/// `Intervals.maxwidth(int, IntervalsSource)`.
pub fn maxwidth(width: i32, source: Arc<dyn IntervalsSource>) -> Arc<dyn IntervalsSource> {
    Intervals::maxwidth(width, source)
}

/// `Intervals.extend(IntervalsSource, int, int)`.
pub fn extend(
    source: Arc<dyn IntervalsSource>,
    before: i32,
    after: i32,
) -> Arc<dyn IntervalsSource> {
    Intervals::extend(source, before, after)
}

/// `Intervals.fixField(String, IntervalsSource)`.
pub fn fix_field(field: &str, source: Arc<dyn IntervalsSource>) -> Arc<dyn IntervalsSource> {
    Intervals::fix_field(field, source)
}

/// `Intervals.containedBy(IntervalsSource, IntervalsSource)`.
pub fn contained_by(
    small: Arc<dyn IntervalsSource>,
    big: Arc<dyn IntervalsSource>,
) -> Arc<dyn IntervalsSource> {
    unchecked(Intervals::contained_by(small, big))
}

/// `Intervals.notContainedBy(IntervalsSource, IntervalsSource)`.
pub fn not_contained_by(
    small: Arc<dyn IntervalsSource>,
    big: Arc<dyn IntervalsSource>,
) -> Arc<dyn IntervalsSource> {
    unchecked(Intervals::not_contained_by(small, big))
}

/// `Intervals.containing(IntervalsSource, IntervalsSource)`.
pub fn containing(
    big: Arc<dyn IntervalsSource>,
    small: Arc<dyn IntervalsSource>,
) -> Arc<dyn IntervalsSource> {
    unchecked(Intervals::containing(big, small))
}

/// `Intervals.notContaining(IntervalsSource, IntervalsSource)`.
pub fn not_containing(
    minuend: Arc<dyn IntervalsSource>,
    subtrahend: Arc<dyn IntervalsSource>,
) -> Arc<dyn IntervalsSource> {
    unchecked(Intervals::not_containing(minuend, subtrahend))
}

/// `Intervals.notWithin(IntervalsSource, int, IntervalsSource)`.
pub fn not_within(
    minuend: Arc<dyn IntervalsSource>,
    positions: i32,
    subtrahend: Arc<dyn IntervalsSource>,
) -> Arc<dyn IntervalsSource> {
    Intervals::not_within(minuend, positions, subtrahend)
}

/// `Intervals.within(IntervalsSource, int, IntervalsSource)`.
pub fn within(
    source: Arc<dyn IntervalsSource>,
    positions: i32,
    reference: Arc<dyn IntervalsSource>,
) -> Arc<dyn IntervalsSource> {
    unchecked(Intervals::within(source, positions, reference))
}

/// `Intervals.overlapping(IntervalsSource, IntervalsSource)`.
pub fn overlapping(
    source: Arc<dyn IntervalsSource>,
    reference: Arc<dyn IntervalsSource>,
) -> Arc<dyn IntervalsSource> {
    Intervals::overlapping(source, reference)
}

/// `Intervals.nonOverlapping(IntervalsSource, IntervalsSource)`.
pub fn non_overlapping(
    minuend: Arc<dyn IntervalsSource>,
    subtrahend: Arc<dyn IntervalsSource>,
) -> Arc<dyn IntervalsSource> {
    Intervals::non_overlapping(minuend, subtrahend)
}

/// `Intervals.before(IntervalsSource, IntervalsSource)`.
pub fn before(
    source: Arc<dyn IntervalsSource>,
    reference: Arc<dyn IntervalsSource>,
) -> Arc<dyn IntervalsSource> {
    unchecked(Intervals::before(source, reference))
}

/// `Intervals.after(IntervalsSource, IntervalsSource)`.
pub fn after(
    source: Arc<dyn IntervalsSource>,
    reference: Arc<dyn IntervalsSource>,
) -> Arc<dyn IntervalsSource> {
    unchecked(Intervals::after(source, reference))
}

/// `Intervals.atLeast(int, IntervalsSource...)`.
pub fn at_least(min: i32, sources: Vec<Arc<dyn IntervalsSource>>) -> Arc<dyn IntervalsSource> {
    Intervals::at_least(min, sources)
}

/// `Intervals.prefix(BytesRef)`.
pub fn prefix(p: &str) -> Arc<dyn IntervalsSource> {
    unchecked(Intervals::prefix(&BytesRef::from_char_sequence(p)))
}

/// `Intervals.prefix(BytesRef, int)`.
pub fn prefix_max(p: &str, max_expansions: i32) -> Arc<dyn IntervalsSource> {
    unchecked(Intervals::prefix_with_max_expansions(
        &BytesRef::from_char_sequence(p),
        max_expansions,
    ))
}

/// `Intervals.wildcard(BytesRef)`.
pub fn wildcard(w: &str) -> Arc<dyn IntervalsSource> {
    unchecked(Intervals::wildcard(&BytesRef::from_char_sequence(w)))
}

/// `Intervals.regexp(BytesRef)`.
pub fn regexp(r: &str) -> Arc<dyn IntervalsSource> {
    unchecked(Intervals::regexp(&BytesRef::from_char_sequence(r)))
}

/// `Intervals.fuzzyTerm(String, int)`.
pub fn fuzzy_term(t: &str, max_edits: i32) -> Arc<dyn IntervalsSource> {
    unchecked(Intervals::fuzzy_term(t, max_edits))
}

/// `Intervals.range(BytesRef, BytesRef, boolean, boolean)`.
pub fn range(
    lower: Option<&str>,
    upper: Option<&str>,
    include_lower: bool,
    include_upper: bool,
) -> Arc<dyn IntervalsSource> {
    let lower = lower.map(BytesRef::from_char_sequence);
    let upper = upper.map(BytesRef::from_char_sequence);
    unchecked(Intervals::range(
        lower.as_ref(),
        upper.as_ref(),
        include_lower,
        include_upper,
    ))
}
