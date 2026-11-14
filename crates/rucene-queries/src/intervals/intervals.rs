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

//! Port of `Intervals.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::sync::Arc;

use rucene::analysis::{Analyzer, CachingTokenFilter, SharedTokenStream, TokenStream};
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::fuzzy_query::{FuzzyQuery, DEFAULT_PREFIX_LENGTH, DEFAULT_TRANSPOSITIONS};
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::term_range_query::TermRangeQuery;
use rucene::search::wildcard_query::WildcardQuery;
use rucene::util::automaton::compiled_automaton::CompiledAutomaton;
use rucene::util::automaton::operations::{Operations, DEFAULT_DETERMINIZE_WORK_LIMIT};
use rucene::util::automaton::reg_exp::RegExp;
use rucene::util::BytesRef;

use crate::intervals::block_intervals_source::BlockIntervalsSource;
use crate::intervals::contained_by_intervals_source::ContainedByIntervalsSource;
use crate::intervals::containing_intervals_source::ContainingIntervalsSource;
use crate::intervals::disjunction_intervals_source::DisjunctionIntervalsSource;
use crate::intervals::extended_intervals_source::ExtendedIntervalsSource;
use crate::intervals::filtered_intervals_source::FilteredIntervalsSource;
use crate::intervals::fixed_field_intervals_source::FixedFieldIntervalsSource;
use crate::intervals::interval_builder::IntervalBuilder;
use crate::intervals::intervals_source::{utf8_to_string, IntervalsSource};
use crate::intervals::minimum_should_match_intervals_source::MinimumShouldMatchIntervalsSource;
use crate::intervals::multi_term_intervals_source::MultiTermIntervalsSource;
use crate::intervals::no_match_intervals_source::NoMatchIntervalsSource;
use crate::intervals::non_overlapping_intervals_source::NonOverlappingIntervalsSource;
use crate::intervals::not_contained_by_intervals_source::NotContainedByIntervalsSource;
use crate::intervals::not_containing_intervals_source::NotContainingIntervalsSource;
use crate::intervals::offset_intervals_source::OffsetIntervalsSource;
use crate::intervals::ordered_intervals_source::OrderedIntervalsSource;
use crate::intervals::overlapping_intervals_source::OverlappingIntervalsSource;
use crate::intervals::payload_filtered_term_intervals_source::{
    PayloadFilter, PayloadFilteredTermIntervalsSource,
};
use crate::intervals::term_intervals_source::TermIntervalsSource;
use crate::intervals::unordered_intervals_source::UnorderedIntervalsSource;

/// Factory functions for creating [`IntervalsSource`]s.
///
/// Equivalent to the final class `org.apache.lucene.queries.intervals.Intervals`.
///
/// These sources implement minimum-interval algorithms taken from the paper
/// "Efficient Optimally Lazy Algorithms for Minimal-Interval Semantics". By
/// default, sources that are sensitive to internal gaps (`phrase`, `maxgaps`)
/// rewrite their sub-sources so that disjunctions of different lengths are
/// pulled up to the top of the interval tree, which can load more terms; use
/// [`or_with_rewrite`](Self::or_with_rewrite) with `false` to prevent it.
///
/// Java's overloads are distinct functions here, each naming the overload it
/// stands for; Java's varargs are vectors.
#[derive(Debug, Clone, Copy)]
pub struct Intervals;

impl Intervals {
    /// The default number of expansions of
    /// [`multiterm`](Self::multiterm) and the expanding factories.
    ///
    /// Equivalent to `Intervals.DEFAULT_MAX_EXPANSIONS`.
    pub const DEFAULT_MAX_EXPANSIONS: i32 = 128;

    /// Returns a source exposing intervals for a term.
    ///
    /// Equivalent to `Intervals.term(BytesRef)`.
    pub fn term(term: BytesRef) -> Arc<dyn IntervalsSource> {
        Arc::new(TermIntervalsSource::new(term))
    }

    /// Returns a source exposing intervals for a term.
    ///
    /// Equivalent to `Intervals.term(String)`.
    pub fn term_str(term: &str) -> Arc<dyn IntervalsSource> {
        Arc::new(TermIntervalsSource::new(BytesRef::from_char_sequence(term)))
    }

    /// Returns a source exposing intervals for a term, filtered by the value
    /// of the term's payload at each position.
    ///
    /// Equivalent to `Intervals.term(String, Predicate<BytesRef>)`.
    pub fn term_str_with_payload_filter(
        term: &str,
        payload_filter: PayloadFilter,
    ) -> Arc<dyn IntervalsSource> {
        Self::term_with_payload_filter(BytesRef::from_char_sequence(term), payload_filter)
    }

    /// Returns a source exposing intervals for a term, filtered by the value
    /// of the term's payload at each position.
    ///
    /// Equivalent to `Intervals.term(BytesRef, Predicate<BytesRef>)`.
    pub fn term_with_payload_filter(
        term: BytesRef,
        payload_filter: PayloadFilter,
    ) -> Arc<dyn IntervalsSource> {
        Arc::new(PayloadFilteredTermIntervalsSource::new(
            term,
            payload_filter,
        ))
    }

    /// Returns a source exposing intervals for a phrase consisting of a list
    /// of terms.
    ///
    /// Equivalent to `Intervals.phrase(String...)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of pulling up too many disjunctions.
    pub fn phrase_terms(terms: &[&str]) -> Result<Arc<dyn IntervalsSource>> {
        if terms.len() == 1 {
            return Ok(Self::term_str(terms[0]));
        }
        let sources: Vec<Arc<dyn IntervalsSource>> =
            terms.iter().map(|term| Self::term_str(term)).collect();
        Self::phrase(sources)
    }

    /// Returns a source exposing intervals for a phrase consisting of a list
    /// of sources.
    ///
    /// Equivalent to `Intervals.phrase(IntervalsSource...)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of pulling up too many disjunctions.
    pub fn phrase(sub_sources: Vec<Arc<dyn IntervalsSource>>) -> Result<Arc<dyn IntervalsSource>> {
        BlockIntervalsSource::build(sub_sources)
    }

    /// Returns a source over the disjunction of a set of sub-sources, which
    /// rewrites if wrapped by a source that is sensitive to internal gaps.
    ///
    /// Equivalent to `Intervals.or(IntervalsSource...)` and
    /// `Intervals.or(List<IntervalsSource>)`.
    ///
    /// # Errors
    ///
    /// Propagates the error of a sub-disjunction's pull-up.
    pub fn or(sub_sources: Vec<Arc<dyn IntervalsSource>>) -> Result<Arc<dyn IntervalsSource>> {
        Self::or_with_rewrite(true, sub_sources)
    }

    /// Returns a source over the disjunction of a set of sub-sources; with
    /// `rewrite` false, sources that are sensitive to internal gaps do not
    /// rewrite it, which may run more efficiently but can miss valid hits due
    /// to minimisation.
    ///
    /// Equivalent to `Intervals.or(boolean, IntervalsSource...)` and
    /// `Intervals.or(boolean, List<IntervalsSource>)`.
    ///
    /// # Errors
    ///
    /// Propagates the error of a sub-disjunction's pull-up.
    pub fn or_with_rewrite(
        rewrite: bool,
        sub_sources: Vec<Arc<dyn IntervalsSource>>,
    ) -> Result<Arc<dyn IntervalsSource>> {
        DisjunctionIntervalsSource::create(sub_sources, rewrite)
    }

    /// Returns a source over the disjunction of all terms that begin with a
    /// prefix, expanding to at most [`DEFAULT_MAX_EXPANSIONS`](Self::DEFAULT_MAX_EXPANSIONS)
    /// terms.
    ///
    /// Equivalent to `Intervals.prefix(BytesRef)`.
    ///
    /// # Errors
    ///
    /// The errors of [`prefix_with_max_expansions`](Self::prefix_with_max_expansions).
    pub fn prefix(prefix: &BytesRef) -> Result<Arc<dyn IntervalsSource>> {
        Self::prefix_with_max_expansions(prefix, Self::DEFAULT_MAX_EXPANSIONS)
    }

    /// Expert: returns a source over the disjunction of all terms that begin
    /// with a prefix, expanding to at most `max_expansions` terms. Setting
    /// `max_expansions` higher than the default can be both slow and
    /// memory-intensive; the iterator raises `IllegalStateException` when the
    /// prefix expands to more terms.
    ///
    /// Equivalent to `Intervals.prefix(BytesRef, int)`.
    ///
    /// # Errors
    ///
    /// The errors of compiling the automaton, and
    /// `IllegalArgumentException` for `max_expansions` above the maximum
    /// clause count.
    pub fn prefix_with_max_expansions(
        prefix: &BytesRef,
        max_expansions: i32,
    ) -> Result<Arc<dyn IntervalsSource>> {
        let ca = CompiledAutomaton::new(PrefixQuery::to_automaton(prefix), false, true, true)?;
        Ok(Arc::new(MultiTermIntervalsSource::new(
            ca,
            max_expansions,
            format!("{}*", utf8_to_string(prefix)),
        )?))
    }

    /// Returns a source over the disjunction of all terms that match a
    /// wildcard glob, expanding to at most
    /// [`DEFAULT_MAX_EXPANSIONS`](Self::DEFAULT_MAX_EXPANSIONS) terms.
    ///
    /// Equivalent to `Intervals.wildcard(BytesRef)`.
    ///
    /// # Errors
    ///
    /// The errors of [`wildcard_with_max_expansions`](Self::wildcard_with_max_expansions).
    pub fn wildcard(wildcard: &BytesRef) -> Result<Arc<dyn IntervalsSource>> {
        Self::wildcard_with_max_expansions(wildcard, Self::DEFAULT_MAX_EXPANSIONS)
    }

    /// Expert: returns a source over the disjunction of all terms that match
    /// a wildcard glob, expanding to at most `max_expansions` terms.
    ///
    /// Equivalent to `Intervals.wildcard(BytesRef, int)`.
    ///
    /// # Errors
    ///
    /// The errors of building and compiling the automaton, and
    /// `IllegalArgumentException` for `max_expansions` above the maximum
    /// clause count.
    pub fn wildcard_with_max_expansions(
        wildcard: &BytesRef,
        max_expansions: i32,
    ) -> Result<Arc<dyn IntervalsSource>> {
        let ca = CompiledAutomaton::from_automaton(WildcardQuery::to_automaton(
            &Term::new(String::new(), wildcard.clone()),
            DEFAULT_DETERMINIZE_WORK_LIMIT,
        )?)?;
        Ok(Arc::new(MultiTermIntervalsSource::new(
            ca,
            max_expansions,
            utf8_to_string(wildcard),
        )?))
    }

    /// Returns a source over the disjunction of all terms that match a
    /// regular expression, expanding to at most
    /// [`DEFAULT_MAX_EXPANSIONS`](Self::DEFAULT_MAX_EXPANSIONS) terms.
    ///
    /// Equivalent to `Intervals.regexp(BytesRef)`.
    ///
    /// # Errors
    ///
    /// The errors of [`regexp_with_max_expansions`](Self::regexp_with_max_expansions).
    pub fn regexp(regexp: &BytesRef) -> Result<Arc<dyn IntervalsSource>> {
        Self::regexp_with_max_expansions(regexp, Self::DEFAULT_MAX_EXPANSIONS)
    }

    /// Expert: returns a source over the disjunction of all terms that match
    /// a regular expression, expanding to at most `max_expansions` terms.
    ///
    /// Equivalent to `Intervals.regexp(BytesRef, int)`.
    ///
    /// # Errors
    ///
    /// The errors of parsing, determinising and compiling the automaton, and
    /// `IllegalArgumentException` for `max_expansions` above the maximum
    /// clause count.
    pub fn regexp_with_max_expansions(
        regexp: &BytesRef,
        max_expansions: i32,
    ) -> Result<Arc<dyn IntervalsSource>> {
        let mut automaton =
            RegExp::new(&Term::new(String::new(), regexp.clone()).text())?.to_automaton()?;
        automaton = Operations::determinize(&automaton, DEFAULT_DETERMINIZE_WORK_LIMIT)?;
        let ca = CompiledAutomaton::new(automaton, false, true, false)?;
        Ok(Arc::new(MultiTermIntervalsSource::new(
            ca,
            max_expansions,
            utf8_to_string(regexp),
        )?))
    }

    /// Returns a source over the disjunction of all terms that fall within
    /// the given range, expanding to at most
    /// [`DEFAULT_MAX_EXPANSIONS`](Self::DEFAULT_MAX_EXPANSIONS) terms; `None`
    /// bounds are open.
    ///
    /// Equivalent to `Intervals.range(BytesRef, BytesRef, boolean, boolean)`.
    ///
    /// # Errors
    ///
    /// The errors of [`range_with_max_expansions`](Self::range_with_max_expansions).
    pub fn range(
        lower_term: Option<&BytesRef>,
        upper_term: Option<&BytesRef>,
        include_lower: bool,
        include_upper: bool,
    ) -> Result<Arc<dyn IntervalsSource>> {
        Self::range_with_max_expansions(
            lower_term,
            upper_term,
            include_lower,
            include_upper,
            Self::DEFAULT_MAX_EXPANSIONS,
        )
    }

    /// Expert: returns a source over the disjunction of all terms that fall
    /// within the given range, expanding to at most `max_expansions` terms.
    ///
    /// Equivalent to `Intervals.range(BytesRef, BytesRef, boolean, boolean,
    /// int)`.
    ///
    /// # Errors
    ///
    /// The errors of building and compiling the automaton, and
    /// `IllegalArgumentException` for `max_expansions` above the maximum
    /// clause count.
    pub fn range_with_max_expansions(
        lower_term: Option<&BytesRef>,
        upper_term: Option<&BytesRef>,
        include_lower: bool,
        include_upper: bool,
        max_expansions: i32,
    ) -> Result<Arc<dyn IntervalsSource>> {
        let automaton =
            TermRangeQuery::to_automaton(lower_term, upper_term, include_lower, include_upper)?;
        let ca = CompiledAutomaton::new(automaton, false, true, true)?;
        let mut buffer = String::new();
        buffer.push('{');
        match lower_term {
            None => buffer.push_str("* "),
            Some(lower) => buffer.push_str(&utf8_to_string(lower)),
        }
        buffer.push(',');
        match upper_term {
            None => buffer.push('*'),
            Some(upper) => buffer.push_str(&utf8_to_string(upper)),
        }
        buffer.push('}');
        Ok(Arc::new(MultiTermIntervalsSource::new(
            ca,
            max_expansions,
            buffer,
        )?))
    }

    /// A fuzzy term source: the disjunction of intervals of terms that are
    /// within `max_edits` of the term, with the default prefix length,
    /// transpositions and maximum expansions.
    ///
    /// Equivalent to `Intervals.fuzzyTerm(String, int)`.
    ///
    /// # Errors
    ///
    /// The errors of [`fuzzy_term_with`](Self::fuzzy_term_with).
    pub fn fuzzy_term(term: &str, max_edits: i32) -> Result<Arc<dyn IntervalsSource>> {
        Self::fuzzy_term_with(
            term,
            max_edits,
            DEFAULT_PREFIX_LENGTH,
            DEFAULT_TRANSPOSITIONS,
            Self::DEFAULT_MAX_EXPANSIONS,
        )
    }

    /// A fuzzy term source: the disjunction of intervals of terms that are
    /// within `max_edits` of the term, delegated to a
    /// [`multiterm`](Self::multiterm_with_max_expansions) source over
    /// `FuzzyQuery`'s automaton.
    ///
    /// Equivalent to `Intervals.fuzzyTerm(String, int, int, boolean, int)`.
    ///
    /// # Errors
    ///
    /// The errors of building the fuzzy automaton, and
    /// `IllegalArgumentException` for `max_expansions` above the maximum
    /// clause count.
    pub fn fuzzy_term_with(
        term: &str,
        max_edits: i32,
        prefix_length: i32,
        transpositions: bool,
        max_expansions: i32,
    ) -> Result<Arc<dyn IntervalsSource>> {
        Self::multiterm_with_max_expansions(
            FuzzyQuery::get_fuzzy_automaton(term, max_edits, prefix_length, transpositions)?,
            max_expansions,
            &format!("{term}~{max_edits}"),
        )
    }

    /// Expert: returns a source over the disjunction of all terms that are
    /// accepted by the given automaton, expanding to at most
    /// [`DEFAULT_MAX_EXPANSIONS`](Self::DEFAULT_MAX_EXPANSIONS) terms;
    /// `pattern` renders the automaton, mostly in exception messages.
    ///
    /// Equivalent to `Intervals.multiterm(CompiledAutomaton, String)`.
    ///
    /// # Errors
    ///
    /// As [`multiterm_with_max_expansions`](Self::multiterm_with_max_expansions).
    pub fn multiterm(ca: CompiledAutomaton, pattern: &str) -> Result<Arc<dyn IntervalsSource>> {
        Self::multiterm_with_max_expansions(ca, Self::DEFAULT_MAX_EXPANSIONS, pattern)
    }

    /// Expert: returns a source over the disjunction of all terms that are
    /// accepted by the given automaton, expanding to at most
    /// `max_expansions` terms.
    ///
    /// Equivalent to `Intervals.multiterm(CompiledAutomaton, int, String)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for `max_expansions` above the maximum
    /// clause count.
    pub fn multiterm_with_max_expansions(
        ca: CompiledAutomaton,
        max_expansions: i32,
        pattern: &str,
    ) -> Result<Arc<dyn IntervalsSource>> {
        Ok(Arc::new(MultiTermIntervalsSource::new(
            ca,
            max_expansions,
            pattern.to_string(),
        )?))
    }

    /// Creates a source that filters a sub-source by the width of its
    /// intervals.
    ///
    /// Equivalent to `Intervals.maxwidth(int, IntervalsSource)`.
    pub fn maxwidth(width: i32, sub_source: Arc<dyn IntervalsSource>) -> Arc<dyn IntervalsSource> {
        FilteredIntervalsSource::max_width(sub_source, width)
    }

    /// Creates a source that filters a sub-source by its gaps.
    ///
    /// Equivalent to `Intervals.maxgaps(int, IntervalsSource)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of pulling up too many disjunctions.
    pub fn maxgaps(
        gaps: i32,
        sub_source: Arc<dyn IntervalsSource>,
    ) -> Result<Arc<dyn IntervalsSource>> {
        FilteredIntervalsSource::max_gaps(sub_source, gaps)
    }

    /// Creates a source that wraps another, extending its intervals by a
    /// number of positions before and after; the gaps of the extended
    /// intervals are those of the wrapped ones.
    ///
    /// Equivalent to `Intervals.extend(IntervalsSource, int, int)`.
    pub fn extend(
        source: Arc<dyn IntervalsSource>,
        before: i32,
        after: i32,
    ) -> Arc<dyn IntervalsSource> {
        Arc::new(ExtendedIntervalsSource::new(source, before, after))
    }

    /// Creates an ordered source: intervals in which the sub-sources all
    /// appear in the given order.
    ///
    /// Equivalent to `Intervals.ordered(IntervalsSource...)`.
    pub fn ordered(sub_sources: Vec<Arc<dyn IntervalsSource>>) -> Arc<dyn IntervalsSource> {
        OrderedIntervalsSource::build(sub_sources)
    }

    /// Creates an unordered source: intervals in which all the sub-sources
    /// appear, possibly overlapping; of several eligible intervals ending at
    /// the same position, only the narrowest is returned.
    ///
    /// Equivalent to `Intervals.unordered(IntervalsSource...)`.
    pub fn unordered(sub_sources: Vec<Arc<dyn IntervalsSource>>) -> Arc<dyn IntervalsSource> {
        UnorderedIntervalsSource::build(sub_sources)
    }

    /// Creates an unordered source allowing no overlaps between the
    /// sub-sources.
    ///
    /// Equivalent to `Intervals.unorderedNoOverlaps(IntervalsSource,
    /// IntervalsSource)`.
    ///
    /// # Errors
    ///
    /// Propagates the error of the disjunction.
    pub fn unordered_no_overlaps(
        a: Arc<dyn IntervalsSource>,
        b: Arc<dyn IntervalsSource>,
    ) -> Result<Arc<dyn IntervalsSource>> {
        Self::or(vec![
            Self::ordered(vec![Arc::clone(&a), Arc::clone(&b)]),
            Self::ordered(vec![b, a]),
        ])
    }

    /// Creates a source that always reads its intervals from a specific
    /// field, to compare intervals across fields.
    ///
    /// Equivalent to `Intervals.fixField(String, IntervalsSource)`.
    pub fn fix_field(field: &str, source: Arc<dyn IntervalsSource>) -> Arc<dyn IntervalsSource> {
        Arc::new(FixedFieldIntervalsSource::new(field.to_string(), source))
    }

    /// Creates a source of the intervals of the minuend that do not overlap
    /// intervals of the subtrahend.
    ///
    /// Equivalent to `Intervals.nonOverlapping(IntervalsSource,
    /// IntervalsSource)`.
    pub fn non_overlapping(
        minuend: Arc<dyn IntervalsSource>,
        subtrahend: Arc<dyn IntervalsSource>,
    ) -> Arc<dyn IntervalsSource> {
        Arc::new(NonOverlappingIntervalsSource::new(minuend, subtrahend))
    }

    /// Returns intervals from a source that overlap intervals from another.
    ///
    /// Equivalent to `Intervals.overlapping(IntervalsSource,
    /// IntervalsSource)`.
    pub fn overlapping(
        source: Arc<dyn IntervalsSource>,
        reference: Arc<dyn IntervalsSource>,
    ) -> Arc<dyn IntervalsSource> {
        Arc::new(OverlappingIntervalsSource::new(source, reference))
    }

    /// Creates a not-within source: intervals of the minuend that do not
    /// appear within `positions` of intervals of the subtrahend.
    ///
    /// Equivalent to `Intervals.notWithin(IntervalsSource, int,
    /// IntervalsSource)`.
    pub fn not_within(
        minuend: Arc<dyn IntervalsSource>,
        positions: i32,
        subtrahend: Arc<dyn IntervalsSource>,
    ) -> Arc<dyn IntervalsSource> {
        Arc::new(NonOverlappingIntervalsSource::new(
            minuend,
            Self::extend(subtrahend, positions, positions),
        ))
    }

    /// Returns intervals of the source that appear within `positions` of
    /// intervals of the reference.
    ///
    /// Equivalent to `Intervals.within(IntervalsSource, int,
    /// IntervalsSource)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of pulling up too many disjunctions.
    pub fn within(
        source: Arc<dyn IntervalsSource>,
        positions: i32,
        reference: Arc<dyn IntervalsSource>,
    ) -> Result<Arc<dyn IntervalsSource>> {
        Self::contained_by(source, Self::extend(reference, positions, positions))
    }

    /// Creates a not-containing source: intervals of the minuend that do not
    /// contain intervals of the subtrahend.
    ///
    /// Equivalent to `Intervals.notContaining(IntervalsSource,
    /// IntervalsSource)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of pulling up too many disjunctions.
    pub fn not_containing(
        minuend: Arc<dyn IntervalsSource>,
        subtrahend: Arc<dyn IntervalsSource>,
    ) -> Result<Arc<dyn IntervalsSource>> {
        NotContainingIntervalsSource::build(minuend, subtrahend)
    }

    /// Creates a containing source: intervals of the big source that contain
    /// one or more intervals of the small source.
    ///
    /// Equivalent to `Intervals.containing(IntervalsSource, IntervalsSource)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of pulling up too many disjunctions.
    pub fn containing(
        big: Arc<dyn IntervalsSource>,
        small: Arc<dyn IntervalsSource>,
    ) -> Result<Arc<dyn IntervalsSource>> {
        ContainingIntervalsSource::build(big, small)
    }

    /// Creates a not-contained-by source: intervals of the small source that
    /// do not appear within intervals of the big source.
    ///
    /// Equivalent to `Intervals.notContainedBy(IntervalsSource,
    /// IntervalsSource)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of pulling up too many disjunctions.
    pub fn not_contained_by(
        small: Arc<dyn IntervalsSource>,
        big: Arc<dyn IntervalsSource>,
    ) -> Result<Arc<dyn IntervalsSource>> {
        NotContainedByIntervalsSource::build(small, big)
    }

    /// Creates a contained-by source: intervals of the small source that
    /// appear within intervals of the big source.
    ///
    /// Equivalent to `Intervals.containedBy(IntervalsSource,
    /// IntervalsSource)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of pulling up too many disjunctions.
    pub fn contained_by(
        small: Arc<dyn IntervalsSource>,
        big: Arc<dyn IntervalsSource>,
    ) -> Result<Arc<dyn IntervalsSource>> {
        ContainedByIntervalsSource::build(small, big)
    }

    /// Returns intervals that span combinations of intervals from
    /// `min_should_match` of the sources.
    ///
    /// Equivalent to `Intervals.atLeast(int, IntervalsSource...)`.
    pub fn at_least(
        min_should_match: i32,
        sources: Vec<Arc<dyn IntervalsSource>>,
    ) -> Arc<dyn IntervalsSource> {
        if min_should_match == sources.len() as i32 {
            return Self::unordered(sources);
        }
        if min_should_match > sources.len() as i32 {
            let parts: Vec<String> = sources.iter().map(|s| s.to_source_string()).collect();
            return Arc::new(NoMatchIntervalsSource::new(format!(
                "Too few sources to match minimum of [{min_should_match}]: [{}]",
                parts.join(", ")
            )));
        }
        Arc::new(MinimumShouldMatchIntervalsSource::new(
            sources,
            min_should_match,
        ))
    }

    /// Returns intervals from the source that appear before intervals from
    /// the reference.
    ///
    /// Equivalent to `Intervals.before(IntervalsSource, IntervalsSource)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of pulling up too many disjunctions.
    pub fn before(
        source: Arc<dyn IntervalsSource>,
        reference: Arc<dyn IntervalsSource>,
    ) -> Result<Arc<dyn IntervalsSource>> {
        ContainedByIntervalsSource::build(
            source,
            Self::extend(
                Arc::new(OffsetIntervalsSource::new(reference, true)),
                i32::MAX,
                0,
            ),
        )
    }

    /// Returns intervals from the source that appear after intervals from the
    /// reference.
    ///
    /// Equivalent to `Intervals.after(IntervalsSource, IntervalsSource)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of pulling up too many disjunctions.
    pub fn after(
        source: Arc<dyn IntervalsSource>,
        reference: Arc<dyn IntervalsSource>,
    ) -> Result<Arc<dyn IntervalsSource>> {
        ContainedByIntervalsSource::build(
            source,
            Self::extend(
                Arc::new(OffsetIntervalsSource::new(reference, false)),
                0,
                i32::MAX,
            ),
        )
    }

    /// Returns a source that produces no intervals; `reason` appears in its
    /// rendering.
    ///
    /// Equivalent to `Intervals.noIntervals(String)`.
    pub fn no_intervals(reason: &str) -> Arc<dyn IntervalsSource> {
        Arc::new(NoMatchIntervalsSource::new(reason.to_string()))
    }

    /// Returns intervals that correspond to the tokens `analyzer` produces
    /// for `text` as the content of `field`; ordered or unordered, with at
    /// most `max_gaps` gaps between sub-intervals (`-1` for no limit). The
    /// token stream is closed afterwards.
    ///
    /// Equivalent to `Intervals.analyzedText(String, Analyzer, String, int,
    /// boolean)`, whose try-with-resources closes the stream and adds a close
    /// failure to the suppressed exceptions of a body failure.
    ///
    /// # Errors
    ///
    /// Propagates any error of the analysis chain.
    pub fn analyzed_text(
        text: &str,
        analyzer: &dyn Analyzer,
        field: &str,
        max_gaps: i32,
        ordered: bool,
    ) -> Result<Arc<dyn IntervalsSource>> {
        let ts = analyzer.token_stream_from_str(field, text)?;
        let result = Self::analyzed_text_stream(
            Box::new(SharedTokenStream::new(Arc::clone(&ts))),
            max_gaps,
            ordered,
        );
        let closed = ts.borrow_mut().close();
        match (result, closed) {
            (Ok(source), Ok(())) => Ok(source),
            (Ok(_), Err(e)) => Err(e),
            (Err(mut e), Err(suppressed)) => {
                e.add_suppressed(suppressed);
                Err(e)
            }
            (Err(e), Ok(())) => Err(e),
        }
    }

    /// Returns intervals that correspond to the tokens of a token stream; the
    /// stream may be fully or partially consumed afterwards.
    ///
    /// Equivalent to `Intervals.analyzedText(TokenStream, int, boolean)` for a
    /// stream that is not a `CachingTokenFilter`, which it wraps in one; see
    /// [`analyzed_text_caching`](Self::analyzed_text_caching) for one that is.
    ///
    /// # Errors
    ///
    /// Propagates any error of the analysis chain.
    pub fn analyzed_text_stream(
        token_stream: Box<dyn TokenStream>,
        max_gaps: i32,
        ordered: bool,
    ) -> Result<Arc<dyn IntervalsSource>> {
        let mut stream = CachingTokenFilter::new(token_stream);
        IntervalBuilder::analyze_text(&mut stream, max_gaps, ordered)
    }

    /// Returns intervals that correspond to the tokens of a caching token
    /// filter.
    ///
    /// Equivalent to `Intervals.analyzedText(TokenStream, int, boolean)` for
    /// a stream that is a `CachingTokenFilter`, which Java uses as is (its
    /// `instanceof` branch).
    ///
    /// # Errors
    ///
    /// Propagates any error of the analysis chain.
    pub fn analyzed_text_caching(
        stream: &mut CachingTokenFilter,
        max_gaps: i32,
        ordered: bool,
    ) -> Result<Arc<dyn IntervalsSource>> {
        IntervalBuilder::analyze_text(stream, max_gaps, ordered)
    }
}
