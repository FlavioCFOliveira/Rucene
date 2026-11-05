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

//! Port of `org.apache.lucene.tests.search.CheckHits`.
//!
//! **Divergences.**
//!
//! * `defaultFieldName` may be `null` in Java; it is an `Option`, and `None`
//!   renders the query with `toString()`.
//! * `ExplanationAsserter` and `MatchesAsserter` keep a clone of the searcher
//!   (a clone shares the reader, the overrides, the similarity and the
//!   caches): a collector of a `CollectorManager` must own its state here.
//! * `ExplanationAssertingSearcher` is a wrapper around an [`IndexSearcher`]
//!   whose inherent `search*` methods check the explanations first, instead
//!   of a subclass.
//! * `verifyExplanation` lower-cases the description with Rust's Unicode
//!   lower-casing, which is `toLowerCase(Locale.ROOT)` except for the
//!   characters whose Java mapping depends on context beyond final sigma.

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use randomizedtesting::jdk::exception::{throw, RUNTIME_EXCEPTION};
use randomizedtesting::jdk::lang::float_to_string;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::{IndexReader, LeafReaderContext};
use rucene::search::similarities::Explanation;
use rucene::search::{
    query_to_string, CollectionResult, Collector, CollectorManager, IndexSearcher, LeafCollector,
    Matches, Query, Scorable, ScoreDoc, ScoreMode, Scorer, Sort, TopDocs, TopFieldDocs,
    TopScoreDocCollectorManager, Weight, NO_MORE_DOCS,
};
use rucene::util::java_fmt::parse_float;

use crate::search::asserting_matches::AssertingMatches;
use crate::search::query_utils::QueryUtils;
use crate::util::lucene_test_case::LuceneTestCase;

/// `q.toString(defaultFieldName)`.
fn query_string(q: &dyn Query, default_field_name: Option<&str>) -> String {
    match default_field_name {
        Some(field) => q.to_query_string(field),
        None => query_to_string(q),
    }
}

/// `AbstractCollection.toString()` of a `TreeSet<Integer>`.
fn set_string(set: &BTreeSet<i32>) -> String {
    let items: Vec<String> = set.iter().map(i32::to_string).collect();
    format!("[{}]", items.join(", "))
}

/// `assertEquals(message, Set, Set)`.
fn assert_set_equals(message: &str, expected: &BTreeSet<i32>, actual: &BTreeSet<i32>) {
    if expected != actual {
        Assert::fail(&format!(
            "{message} expected:<{}> but was:<{}>",
            set_string(expected),
            set_string(actual)
        ));
    }
}

/// `Math.ulp(float)`.
fn ulp(f: f32) -> f32 {
    const SIGNIFICAND_WIDTH: i32 = 24;
    const MIN_EXPONENT: i32 = -126;
    const MAX_EXPONENT: i32 = 127;
    let exp = (((f.to_bits() & 0x7f80_0000) >> 23) as i32) - 127;
    if exp == MAX_EXPONENT + 1 {
        f.abs() // NaN or infinity
    } else if exp == MIN_EXPONENT - 1 {
        f32::from_bits(1) // zero or subnormal: Float.MIN_VALUE
    } else {
        let exp = exp - (SIGNIFICAND_WIDTH - 1);
        if exp >= MIN_EXPONENT {
            f32::from_bits(((exp + 127) as u32) << 23)
        } else {
            f32::from_bits(1u32 << (exp - (MIN_EXPONENT - (SIGNIFICAND_WIDTH - 1))))
        }
    }
}

/// `Math.max(float, float)`: NaN if either is NaN, and `+0` above `-0`.
fn java_max(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_negative() {
            b
        } else {
            a
        }
    } else if a >= b {
        a
    } else {
        b
    }
}

/// `COMPUTED_FROM_PATTERN.matcher(descr).matches()`: the pattern
/// `.*, computed as .* from:` over the whole string; `.` matches no line
/// terminator.
fn computed_from_matches(descr: &str) -> bool {
    if descr.contains(['\n', '\r', '\u{0085}', '\u{2028}', '\u{2029}']) {
        return false;
    }
    match descr.strip_suffix(" from:") {
        Some(body) => body.contains(", computed as "),
        None => false,
    }
}

/// `Object.toString()` of a `Matches`: the class name and the identity hash
/// of an `AssertingMatches`; `<matches>` for the other classes, whose names a
/// trait object cannot give.
fn matches_string(matches: &dyn Matches) -> String {
    if matches.as_any().is::<AssertingMatches>() {
        format!(
            "org.apache.lucene.tests.search.AssertingMatches@{:x}",
            std::ptr::from_ref(matches).cast::<()>() as usize
        )
    } else {
        "<matches>".to_string()
    }
}

/// `String.trim()`: strips the characters up to `' '`.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// The `COMPLETE` and `TOP_SCORES` scorers of one leaf (`s1`, `s2`).
type ScorerPair = (Box<dyn Scorer>, Box<dyn Scorer>);

/// Utility class for asserting expected hits in tests.
///
/// Equivalent to `org.apache.lucene.tests.search.CheckHits`.
pub struct CheckHits;

impl CheckHits {
    /// `checkNoMatchExplanations(q, defaultFieldName, searcher, results)`:
    /// every document up to `maxDoc` that is not in `results` has an
    /// explanation that indicates that it does not match.
    ///
    /// # Errors
    ///
    /// Propagates the searcher's I/O errors.
    pub fn check_no_match_explanations(
        q: &Arc<dyn Query>,
        default_field_name: Option<&str>,
        searcher: &IndexSearcher,
        results: &[i32],
    ) -> Result<()> {
        let d = query_string(&**q, default_field_name);
        let ignore: BTreeSet<i32> = results.iter().copied().collect();

        let max_doc = searcher.get_index_reader().max_doc();
        for doc in 0..max_doc {
            if ignore.contains(&doc) {
                continue;
            }

            let exp = searcher.explain(Arc::clone(q), doc)?;
            Assert::assert_false_msg(
                &format!("Explanation of [[{d}]] for #{doc} doesn't indicate non-match: {exp}"),
                exp.is_match(),
            );
        }
        Ok(())
    }

    /// `checkHitCollector(random, query, defaultFieldName, searcher,
    /// results)`: the query matches exactly `results` through a collector,
    /// also over the three `wrapUnderlyingReader` searchers, after
    /// `QueryUtils.check(random, query, searcher)`.
    ///
    /// # Errors
    ///
    /// Propagates the searcher's I/O errors.
    pub fn check_hit_collector(
        random: &dyn Random,
        query: &Arc<dyn Query>,
        default_field_name: Option<&str>,
        searcher: &IndexSearcher,
        results: &[i32],
    ) -> Result<()> {
        QueryUtils::check_searcher(random, query, Some(searcher));

        let correct: BTreeSet<i32> = results.iter().copied().collect();

        let actual = searcher.search_with_manager(Arc::clone(query), &SetCollectorManager)?;
        assert_set_equals(
            &format!("Simple: {}", query_string(&**query, default_field_name)),
            &correct,
            &actual,
        );

        for i in -1..2 {
            let s = QueryUtils::wrap_underlying_reader(random, searcher, i)?;
            let actual = s.search_with_manager(Arc::clone(query), &SetCollectorManager)?;
            assert_set_equals(
                &format!(
                    "Wrap Reader {i}: {}",
                    query_string(&**query, default_field_name)
                ),
                &correct,
                &actual,
            );
        }
        Ok(())
    }

    /// `checkHits(random, query, defaultFieldName, searcher, results)`: the
    /// top `max(10, 2 * results.length)` hits are exactly `results`, then
    /// `QueryUtils.check(random, query, searcher, rarely(random))`.
    ///
    /// # Errors
    ///
    /// Propagates the searcher's I/O errors.
    pub fn check_hits(
        random: &dyn Random,
        query: &Arc<dyn Query>,
        default_field_name: Option<&str>,
        searcher: &IndexSearcher,
        results: &[i32],
    ) -> Result<()> {
        let n = 10.max(results.len() as i32 * 2);
        let hits = searcher.search(Arc::clone(query), n)?.score_docs;

        let correct: BTreeSet<i32> = results.iter().copied().collect();
        let actual: BTreeSet<i32> = hits.iter().map(|hit| hit.doc).collect();

        assert_set_equals(
            &query_string(&**query, default_field_name),
            &correct,
            &actual,
        );

        QueryUtils::check_searcher_wrap(
            random,
            query,
            Some(searcher),
            LuceneTestCase::rarely_with(random),
        );
        Ok(())
    }

    /// `checkDocIds(mes, results, hits)`: the hits are `results`, in order.
    pub fn check_doc_ids(mes: &str, results: &[i32], hits: &[ScoreDoc]) {
        Assert::assert_equals_msg(
            Some(&format!("{mes} nr of hits")),
            hits.len() as i64,
            results.len() as i64,
        );
        for (i, &result) in results.iter().enumerate() {
            Assert::assert_equals_msg(
                Some(&format!("{mes} doc nrs for hit {i}")),
                i64::from(result),
                i64::from(hits[i].doc),
            );
        }
    }

    /// `checkHitsQuery(query, hits1, hits2, results)`: both hit lists are
    /// `results`, in order, with the same scores.
    pub fn check_hits_query(
        query: &dyn Query,
        hits1: &[ScoreDoc],
        hits2: &[ScoreDoc],
        results: &[i32],
    ) {
        Self::check_doc_ids("hits1", results, hits1);
        Self::check_doc_ids("hits2", results, hits2);
        Self::check_equal(query, hits1, hits2);
    }

    /// `checkEqual(query, hits1, hits2)`: same docs, scores within `1e-6`.
    pub fn check_equal(query: &dyn Query, hits1: &[ScoreDoc], hits2: &[ScoreDoc]) {
        let score_tolerance = 1.0e-6f32;
        if hits1.len() != hits2.len() {
            Assert::fail(&format!(
                "Unequal lengths: hits1={},hits2={}",
                hits1.len(),
                hits2.len()
            ));
        }
        for i in 0..hits1.len() {
            if hits1[i].doc != hits2[i].doc {
                Assert::fail(&format!(
                    "Hit {i} docnumbers don't match\n{}for query:{}",
                    Self::hits2str(Some(hits1), Some(hits2), 0, 0),
                    query_to_string(query)
                ));
            }

            if hits1[i].doc != hits2[i].doc
                || (hits1[i].score - hits2[i].score).abs() > score_tolerance
            {
                Assert::fail(&format!(
                    "Hit {i}, doc nrs {} and {}\nunequal       : {}\n           and: {}\nfor query:{}",
                    hits1[i].doc,
                    hits2[i].doc,
                    float_to_string(hits1[i].score),
                    float_to_string(hits2[i].score),
                    query_to_string(query)
                ));
            }
        }
    }

    /// `hits2str(hits1, hits2, start, end)`: the two hit lists side by side.
    pub fn hits2str(
        hits1: Option<&[ScoreDoc]>,
        hits2: Option<&[ScoreDoc]>,
        start: i32,
        end: i32,
    ) -> String {
        let mut sb = String::new();
        let len1 = hits1.map_or(0, <[ScoreDoc]>::len) as i32;
        let len2 = hits2.map_or(0, <[ScoreDoc]>::len) as i32;
        let end = if end <= 0 { len1.max(len2) } else { end };

        sb.push_str(&format!("Hits length1={len1}\tlength2={len2}"));

        sb.push('\n');
        for i in start..end {
            sb.push_str(&format!("hit={i}:"));
            match hits1 {
                Some(hits) if i < len1 => {
                    let hit = &hits[i as usize];
                    sb.push_str(&format!(
                        " doc{}={} shardIndex={}",
                        hit.doc,
                        float_to_string(hit.score),
                        hit.shard_index
                    ));
                }
                _ => sb.push_str("               "),
            }
            sb.push_str(",\t");
            if let Some(hits) = hits2 {
                if i < len2 {
                    let hit = &hits[i as usize];
                    sb.push_str(&format!(
                        " doc{}={} shardIndex={}",
                        hit.doc,
                        float_to_string(hit.score),
                        hit.shard_index
                    ));
                }
            }

            sb.push('\n');
        }
        sb
    }

    /// `topdocsString(docs, start, end)`.
    pub fn topdocs_string(docs: &TopDocs, start: i32, end: i32) -> String {
        let mut sb = String::new();
        sb.push_str(&format!(
            "TopDocs totalHits={} top={}\n",
            docs.total_hits,
            docs.score_docs.len()
        ));
        let len = docs.score_docs.len() as i32;
        let end = if end <= 0 { len } else { end.min(len) };
        for i in start..end {
            let hit = &docs.score_docs[i as usize];
            sb.push_str(&format!(
                "\t{i}) doc={}\tscore={}\n",
                hit.doc,
                float_to_string(hit.score)
            ));
        }
        sb
    }

    /// `checkExplanations(query, defaultFieldName, searcher)`: shallow.
    ///
    /// # Errors
    ///
    /// Propagates the searcher's I/O errors.
    pub fn check_explanations(
        query: &Arc<dyn Query>,
        default_field_name: Option<&str>,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        Self::check_explanations_deep(query, default_field_name, searcher, false)
    }

    /// `checkExplanations(query, defaultFieldName, searcher, deep)`: the
    /// explanation value of every document matching the query is its score;
    /// `deep` also checks the sub-details.
    ///
    /// # Errors
    ///
    /// Propagates the searcher's I/O errors.
    pub fn check_explanations_deep(
        query: &Arc<dyn Query>,
        default_field_name: Option<&str>,
        searcher: &IndexSearcher,
        deep: bool,
    ) -> Result<()> {
        searcher.search_with_manager(
            Arc::clone(query),
            &ExplanationAsserterManager {
                query: Arc::clone(query),
                default_field_name: default_field_name.map(str::to_string),
                searcher: searcher.clone(),
                deep,
            },
        )
    }

    /// `checkMatches(query, searcher)`: `Weight.matches` is non-null for every
    /// document matching the query.
    ///
    /// # Errors
    ///
    /// Propagates the searcher's I/O errors.
    pub fn check_matches(query: &Arc<dyn Query>, searcher: &IndexSearcher) -> Result<()> {
        searcher.search_with_manager(
            Arc::clone(query),
            &MatchesAsserterManager {
                query: Arc::clone(query),
                searcher: searcher.clone(),
            },
        )
    }

    /// `verifyExplanation(q, doc, score, deep, expl)`: the explanation has
    /// the expected score, and with `deep` its sub-details' max/sum/product
    /// match that score.
    ///
    /// Java catches `Exception` around its two `assertEquals`, which throw an
    /// `AssertionError` (an `Error`), so the `fail` of each catch is never
    /// reached: the failure is the `assertEquals(double, double, double)`
    /// message.
    ///
    /// # Panics
    ///
    /// With Java's `AssertionError` when the explanation does not match.
    // `doc` only reached Java's two unreachable `fail` messages (see above); the
    // signature is Java's.
    #[allow(clippy::only_used_in_recursion)]
    pub fn verify_explanation(q: &str, doc: i32, score: f32, deep: bool, expl: &Explanation) {
        let value = expl.value().float_value();
        Assert::assert_equals_double_msg(None, f64::from(score), f64::from(value), 0.0);

        if !deep {
            return;
        }

        let detail = expl.details();
        // TODO: can we improve this entire method? it's really geared to work only with TF/IDF
        if expl.description().ends_with("computed from:") {
            return; // something more complicated.
        }
        let descr = expl.description().to_lowercase();
        if descr.starts_with("score based on ") && descr.contains("child docs in range") {
            Assert::assert_true_msg("Child doc explanations are missing", !detail.is_empty());
        }
        if !detail.is_empty() && expl.is_match() {
            if detail.len() == 1 && !computed_from_matches(&descr) {
                // simple containment, unless it's a freq of: (which lets a query explain how the
                // freq is calculated), just verify contained expl has same score
                if !expl.description().ends_with("with freq of:")
                    // with dismax, even if there is a single sub explanation, its
                    // score might be different if the score is negative
                    && (score >= 0.0 || !expl.description().ends_with("times others of:"))
                {
                    Self::verify_explanation(q, doc, score, deep, &detail[0]);
                }
            } else {
                // explanation must either:
                // - end with one of: "product of:", "sum of:", "max of:", or
                // - have "max plus <x> times others" (where <x> is float).
                let mut x = 0f32;
                let product_of = descr.ends_with("product of:");
                let sum_of = descr.ends_with("sum of:");
                let max_of = descr.ends_with("max of:");
                let computed_of = descr.find("computed as").is_some_and(|i| i > 0)
                    && computed_from_matches(&descr);
                let mut max_times_others = false;
                if !(product_of || sum_of || max_of || computed_of) {
                    // maybe 'max plus x times others'
                    if let Some(k1) = descr.find("max plus ") {
                        let k1 = k1 + "max plus ".len();
                        let Some(k2) = descr[k1..].find(' ').map(|k| k + k1) else {
                            throw(
                                "java.lang.StringIndexOutOfBoundsException",
                                format!(
                                    "Range [{}, -1) out of bounds for length {}",
                                    descr[..k1].encode_utf16().count(),
                                    descr.encode_utf16().count()
                                ),
                            )
                        };
                        if let Some(parsed) = parse_float(java_trim(&descr[k1..k2])) {
                            x = parsed;
                            if java_trim(&descr[k2..]) == "times others of:" {
                                max_times_others = true;
                            }
                        }
                    }
                }
                // TODO: this is a TERRIBLE assertion!!!!
                if !(product_of || sum_of || max_of || computed_of || max_times_others) {
                    Assert::fail(&format!(
                        "{q}: multi valued explanation description=\"{descr}\" must be 'max of \
                         plus x times others', 'computed as x from:' or end with 'product of' or \
                         'sum of:' or 'max of:' - {expl}"
                    ));
                }
                let mut sum = 0f64;
                let mut product = 1f32;
                let mut max = f32::NEG_INFINITY;
                let mut max_error = 0f64;
                for d in detail {
                    let dval = d.value().float_value();
                    Self::verify_explanation(q, doc, dval, deep, d);
                    product *= dval;
                    sum += f64::from(dval);
                    max = java_max(max, dval);

                    if sum_of {
                        // "sum of" is used by BooleanQuery. Making it accurate is not
                        // easy since ReqOptSumScorer casts some intermediate
                        // contributions to the score to a float before doing another sum.
                        // So we introduce some (reasonable) leniency.
                        // TODO: remove this leniency
                        max_error += f64::from(ulp(dval) * 2.0);
                    }
                }
                let combined: f32 = if product_of {
                    product
                } else if sum_of {
                    sum as f32
                } else if max_of {
                    max
                } else if max_times_others {
                    (f64::from(max) + f64::from(x) * (sum - f64::from(max))) as f32
                } else {
                    Assert::assert_true_msg("should never get here!", computed_of);
                    value
                };
                Assert::assert_equals_double_msg(
                    None,
                    f64::from(combined),
                    f64::from(value),
                    max_error,
                );
            }
        }
    }

    /// `checkTopScores(random, query, searcher)`: the top hits computed with
    /// `TOP_SCORES` are those computed with `COMPLETE` (1 and 10 hits), and
    /// the exposed max scores and block boundaries are valid.
    ///
    /// # Errors
    ///
    /// Propagates the searcher's I/O errors.
    pub fn check_top_scores(
        random: &dyn Random,
        query: &Arc<dyn Query>,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        // Check it computed the top hits correctly
        Self::do_check_top_scores(query, searcher, 1)?;
        Self::do_check_top_scores(query, searcher, 10)?;

        // Now check that the exposed max scores and block boundaries are valid
        Self::do_check_max_scores(random, query, searcher)
    }

    /// The private `doCheckTopScores(query, searcher, numHits)`.
    fn do_check_top_scores(
        query: &Arc<dyn Query>,
        searcher: &IndexSearcher,
        num_hits: i32,
    ) -> Result<()> {
        let complete = TopScoreDocCollectorManager::new(num_hits, None, i32::MAX)?; // COMPLETE
        let top_scores = TopScoreDocCollectorManager::new(num_hits, None, 1)?; // TOP_SCORES
        let complete_top_docs = searcher.search_with_manager(Arc::clone(query), &complete)?;
        let top_scores_top_docs = searcher.search_with_manager(Arc::clone(query), &top_scores)?;
        Self::check_equal(
            &**query,
            &complete_top_docs.score_docs,
            &top_scores_top_docs.score_docs,
        );
        Ok(())
    }

    /// The scorers of one leaf for `doCheckMaxScores`: `w1.scorer(ctx)` and,
    /// with its top-level scoring clause set, `w2.scorerSupplier(ctx)
    /// .get(Long.MAX_VALUE)`; `None` when the leaf has nothing to check.
    fn max_scores_scorers(
        w1: &Arc<dyn Weight>,
        w2: &Arc<dyn Weight>,
        ctx: &LeafReaderContext,
    ) -> Result<Option<ScorerPair>> {
        let s1 = w1.scorer(ctx)?;
        let s2 = match w2.scorer_supplier(ctx)? {
            None => None,
            Some(mut ss2) => {
                // We'll call setMinCompetitiveScore on s2
                ss2.set_top_level_scoring_clause()?;
                Some(ss2.get(i64::MAX)?)
            }
        };
        match (s1, s2) {
            (None, s2) => {
                let exhausted = match s2 {
                    None => true,
                    Some(mut s2) => s2.iterator().next_doc()? == NO_MORE_DOCS,
                };
                Assert::assert_true(exhausted);
                Ok(None)
            }
            (Some(mut s1), None) => {
                Assert::assert_true(s1.iterator().next_doc()? == NO_MORE_DOCS);
                Ok(None)
            }
            (Some(s1), Some(s2)) => Ok(Some((s1, s2))),
        }
    }

    /// The private `doCheckMaxScores(random, query, searcher)`.
    fn do_check_max_scores(
        random: &dyn Random,
        query: &Arc<dyn Query>,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        let query = searcher.rewrite(Arc::clone(query))?;
        let w1 = searcher.create_weight(Arc::clone(&query), ScoreMode::COMPLETE, 1.0)?;
        let w2 = searcher.create_weight(query, ScoreMode::TOP_SCORES, 1.0)?;

        // Check boundaries and max scores when iterating all matches
        for ctx in Arc::clone(searcher.get_index_reader()).leaves()? {
            let Some((mut s1, mut s2)) = Self::max_scores_scorers(&w1, &w2, &ctx)? else {
                continue;
            };
            let two_phase1 = s1.two_phase_iterator().is_some();
            let two_phase2 = s2.two_phase_iterator().is_some();
            let mut up_to = -1;
            let mut max_score = 0f32;
            let mut min_score = 0f32;
            let mut doc2 = approx_next_doc(&mut *s2, two_phase2)?;
            loop {
                let mut doc1 = approx_next_doc(&mut *s1, two_phase1)?;
                while doc1 < doc2 {
                    if !two_phase1 || matches(&mut *s1)? {
                        Assert::assert_true(s1.score()? < min_score);
                    }
                    doc1 = approx_next_doc(&mut *s1, two_phase1)?;
                }
                Assert::assert_equals(i64::from(doc1), i64::from(doc2));
                if doc2 == NO_MORE_DOCS {
                    break;
                }

                if doc2 > up_to {
                    up_to = s2.advance_shallow(doc2)?;
                    Assert::assert_true(up_to >= doc2);
                    max_score = s2.get_max_score(up_to)?;
                }

                if !two_phase2 || matches(&mut *s2)? {
                    Assert::assert_true(!two_phase1 || matches(&mut *s1)?);
                    let score = s2.score()?;
                    Assert::assert_equals_float_msg(None, s1.score()?, score, 0.0);
                    Assert::assert_true_msg(
                        &format!(
                            "{} > {} up to {up_to}",
                            float_to_string(score),
                            float_to_string(max_score)
                        ),
                        score <= max_score,
                    );

                    if score >= min_score && random.next_int_bound(10) == 0 {
                        // On some scorers, changing the min score changes the way that docs are
                        // iterated
                        min_score = score;
                        s2.set_min_competitive_score(min_score)?;
                    }
                }
                doc2 = approx_next_doc(&mut *s2, two_phase2)?;
            }
        }

        // Now check advancing
        for ctx in Arc::clone(searcher.get_index_reader()).leaves()? {
            let Some((mut s1, mut s2)) = Self::max_scores_scorers(&w1, &w2, &ctx)? else {
                continue;
            };
            let two_phase1 = s1.two_phase_iterator().is_some();
            let two_phase2 = s2.two_phase_iterator().is_some();

            let mut up_to = -1;
            let mut min_score = 0f32;
            let mut max_score = 0f32;
            loop {
                let mut doc2 = s2.doc_id();
                let advance;
                let target;
                if random.next_boolean() {
                    advance = false;
                    target = doc2.wrapping_add(1);
                } else {
                    advance = true;
                    // Java's int arithmetic wraps (doc2 = -1 makes NO_MORE_DOCS - doc2 overflow).
                    let delta =
                        (1 + random.next_int_bound(512)).min(NO_MORE_DOCS.wrapping_sub(doc2));
                    target = s2.doc_id().wrapping_add(delta);
                }

                if target > up_to && random.next_boolean() {
                    let delta = random
                        .next_int_bound(512)
                        .min(NO_MORE_DOCS.wrapping_sub(target));
                    up_to = target.wrapping_add(delta);
                    let m = s2.advance_shallow(target)?;
                    Assert::assert_true(m >= target);
                    max_score = s2.get_max_score(up_to)?;
                }

                doc2 = if advance {
                    approx_advance(&mut *s2, two_phase2, target)?
                } else {
                    approx_next_doc(&mut *s2, two_phase2)?
                };

                let mut doc1 = approx_advance(&mut *s1, two_phase1, target)?;
                while doc1 < doc2 {
                    if !two_phase1 || matches(&mut *s1)? {
                        Assert::assert_true(s1.score()? < min_score);
                    }
                    doc1 = approx_next_doc(&mut *s1, two_phase1)?;
                }
                Assert::assert_equals(i64::from(doc1), i64::from(doc2));

                if doc2 == NO_MORE_DOCS {
                    break;
                }

                if !two_phase2 || matches(&mut *s2)? {
                    Assert::assert_true(!two_phase1 || matches(&mut *s1)?);
                    let score = s2.score()?;
                    Assert::assert_equals_float_msg(None, s1.score()?, score, 0.0);

                    if doc2 > up_to {
                        up_to = s2.advance_shallow(doc2)?;
                        Assert::assert_true(up_to >= doc2);
                        max_score = s2.get_max_score(up_to)?;
                    }

                    Assert::assert_true(score <= max_score);

                    if score >= min_score && random.next_int_bound(10) == 0 {
                        // On some scorers, changing the min score changes the way that docs are
                        // iterated
                        min_score = score;
                        s2.set_min_competitive_score(min_score)?;
                    }
                }
            }
        }
        Ok(())
    }
}

/// `approx.nextDoc()`, where `approx` is the scorer's two-phase
/// approximation, or its iterator.
fn approx_next_doc(scorer: &mut dyn Scorer, two_phase: bool) -> Result<i32> {
    if two_phase {
        scorer
            .two_phase_iterator()
            .expect("INVARIANT: the scorer exposed a two-phase view")
            .approximation()
            .next_doc()
    } else {
        scorer.iterator().next_doc()
    }
}

/// `approx.advance(target)` (see [`approx_next_doc`]).
fn approx_advance(scorer: &mut dyn Scorer, two_phase: bool, target: i32) -> Result<i32> {
    if two_phase {
        scorer
            .two_phase_iterator()
            .expect("INVARIANT: the scorer exposed a two-phase view")
            .approximation()
            .advance(target)
    } else {
        scorer.iterator().advance(target)
    }
}

/// `twoPhase.matches()`.
fn matches(scorer: &mut dyn Scorer) -> Result<bool> {
    scorer
        .two_phase_iterator()
        .expect("INVARIANT: the scorer exposed a two-phase view")
        .matches()
}

/// The private `SetCollectorManager`: one [`SetCollector`] per slice, reduced
/// to a sorted set.
struct SetCollectorManager;

impl CollectorManager for SetCollectorManager {
    type Collector = SetCollector;
    type Output = BTreeSet<i32>;

    fn new_collector(&self) -> Result<SetCollector> {
        Ok(SetCollector::new(HashSet::new()))
    }

    fn reduce(&self, collectors: Vec<SetCollector>) -> Result<BTreeSet<i32>> {
        let mut ids = BTreeSet::new();
        for collector in collectors {
            ids.extend(collector.bag);
        }
        Ok(ids)
    }
}

/// Just collects document ids into a set.
///
/// Equivalent to `CheckHits.SetCollector`; Java's collector adds to a set its
/// caller keeps, this one owns it ([`into_bag`](Self::into_bag)).
#[derive(Debug)]
pub struct SetCollector {
    /// `bag`.
    pub bag: HashSet<i32>,
    base: i32,
}

impl SetCollector {
    /// `new SetCollector(bag)`.
    pub fn new(bag: HashSet<i32>) -> SetCollector {
        SetCollector { bag, base: 0 }
    }

    /// The collected ids.
    pub fn into_bag(self) -> HashSet<i32> {
        self.bag
    }
}

impl LeafCollector for SetCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.bag.insert(doc + self.base);
        Ok(())
    }
}

impl Collector for SetCollector {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        // `doSetNextReader`
        self.base = context.doc_base();
        Ok(Box::new(self))
    }

    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE_NO_SCORES
    }
}

/// The anonymous `CollectorManager` of `checkExplanations`.
struct ExplanationAsserterManager {
    query: Arc<dyn Query>,
    default_field_name: Option<String>,
    searcher: IndexSearcher,
    deep: bool,
}

impl CollectorManager for ExplanationAsserterManager {
    type Collector = ExplanationAsserter;
    type Output = ();

    fn new_collector(&self) -> Result<ExplanationAsserter> {
        Ok(ExplanationAsserter::new_deep(
            Arc::clone(&self.query),
            self.default_field_name.as_deref(),
            self.searcher.clone(),
            self.deep,
        ))
    }

    fn reduce(&self, _collectors: Vec<ExplanationAsserter>) -> Result<()> {
        Ok(())
    }
}

/// An [`IndexSearcher`] that implicitly checks the explanation of every
/// match whenever it executes a search.
///
/// Equivalent to `CheckHits.ExplanationAssertingSearcher` (see the module
/// divergences: a wrapper).
#[derive(Debug, Clone)]
pub struct ExplanationAssertingSearcher {
    searcher: IndexSearcher,
}

impl std::ops::Deref for ExplanationAssertingSearcher {
    type Target = IndexSearcher;

    fn deref(&self) -> &IndexSearcher {
        &self.searcher
    }
}

impl ExplanationAssertingSearcher {
    /// `new ExplanationAssertingSearcher(r)`.
    ///
    /// # Errors
    ///
    /// As [`IndexSearcher::new`].
    pub fn new(r: Arc<dyn IndexReader>) -> Result<ExplanationAssertingSearcher> {
        Ok(ExplanationAssertingSearcher {
            searcher: IndexSearcher::new(r)?,
        })
    }

    /// The searcher, for its setters.
    pub fn searcher_mut(&mut self) -> &mut IndexSearcher {
        &mut self.searcher
    }

    /// `checkExplanations(q)`: `super.search(q, manager)` with an
    /// `ExplanationAsserter` per slice.
    ///
    /// # Errors
    ///
    /// Propagates the searcher's I/O errors.
    pub fn check_explanations(&self, q: &Arc<dyn Query>) -> Result<()> {
        self.searcher.search_with_manager(
            Arc::clone(q),
            &ExplanationAsserterManager {
                query: Arc::clone(q),
                default_field_name: None,
                searcher: self.searcher.clone(),
                deep: false,
            },
        )
    }

    /// `search(query, n, sort)`.
    ///
    /// # Errors
    ///
    /// Propagates the searcher's errors.
    pub fn search_with_sort(
        &self,
        query: Arc<dyn Query>,
        n: i32,
        sort: &Sort,
    ) -> Result<TopFieldDocs> {
        self.check_explanations(&query)?;
        self.searcher.super_search_with_sort(query, n, sort)
    }

    /// `search(query, collector)`.
    ///
    /// # Errors
    ///
    /// Propagates the searcher's I/O errors.
    pub fn search_with_collector(
        &self,
        query: Arc<dyn Query>,
        collector: &mut dyn Collector,
    ) -> Result<()> {
        self.check_explanations(&query)?;
        self.searcher.search_with_collector(query, collector)
    }

    /// `search(query, collectorManager)`.
    ///
    /// # Errors
    ///
    /// Propagates the searcher's I/O errors.
    pub fn search_with_manager<M>(&self, query: Arc<dyn Query>, manager: &M) -> Result<M::Output>
    where
        M: CollectorManager,
        M::Collector: Send + 'static,
    {
        self.check_explanations(&query)?;
        self.searcher.search_with_manager(query, manager)
    }

    /// `search(query, n)`.
    ///
    /// # Errors
    ///
    /// Propagates the searcher's I/O errors.
    pub fn search(&self, query: Arc<dyn Query>, n: i32) -> Result<TopDocs> {
        self.check_explanations(&query)?;
        self.searcher.search(query, n)
    }
}

/// Asserts that the score explanation for every document matching a query
/// corresponds with the true score.
///
/// Equivalent to `CheckHits.ExplanationAsserter`. It should only be used with
/// the query and searcher it was constructed with.
pub struct ExplanationAsserter {
    q: Arc<dyn Query>,
    s: IndexSearcher,
    d: String,
    deep: bool,
    base: i32,
}

impl std::fmt::Debug for ExplanationAsserter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExplanationAsserter")
            .field("d", &self.d)
            .field("deep", &self.deep)
            .finish_non_exhaustive()
    }
}

impl ExplanationAsserter {
    /// `new ExplanationAsserter(q, defaultFieldName, s)`: shallow tests.
    pub fn new(
        q: Arc<dyn Query>,
        default_field_name: Option<&str>,
        s: IndexSearcher,
    ) -> ExplanationAsserter {
        Self::new_deep(q, default_field_name, s, false)
    }

    /// `new ExplanationAsserter(q, defaultFieldName, s, deep)`.
    pub fn new_deep(
        q: Arc<dyn Query>,
        default_field_name: Option<&str>,
        s: IndexSearcher,
        deep: bool,
    ) -> ExplanationAsserter {
        let d = query_string(&*q, default_field_name);
        ExplanationAsserter {
            q,
            s,
            d,
            deep,
            base: 0,
        }
    }
}

impl LeafCollector for ExplanationAsserter {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        let doc = doc + self.base;
        let exp = match self.s.explain(Arc::clone(&self.q), doc) {
            Ok(exp) => exp,
            Err(e) => throw(
                RUNTIME_EXCEPTION,
                format!(
                    "exception in hitcollector of [[{}]] for #{doc}: {e}",
                    self.d
                ),
            ),
        };

        CheckHits::verify_explanation(&self.d, doc, scorer.score()?, self.deep, &exp);
        Assert::assert_true_msg(
            &format!(
                "Explanation of [[{}]] for #{doc} does not indicate match: {exp}",
                self.d
            ),
            exp.is_match(),
        );
        Ok(())
    }
}

impl Collector for ExplanationAsserter {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        // `doSetNextReader`
        self.base = context.doc_base();
        Ok(Box::new(self))
    }

    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE
    }
}

/// The anonymous `CollectorManager` of `checkMatches`.
struct MatchesAsserterManager {
    query: Arc<dyn Query>,
    searcher: IndexSearcher,
}

impl CollectorManager for MatchesAsserterManager {
    type Collector = MatchesAsserter;
    type Output = ();

    fn new_collector(&self) -> Result<MatchesAsserter> {
        MatchesAsserter::new(&self.query, &self.searcher)
    }

    fn reduce(&self, _collectors: Vec<MatchesAsserter>) -> Result<()> {
        Ok(())
    }
}

/// Asserts that the `Matches` of a query is non-null whenever the document it
/// is created for is a hit, and that the previous non-matching document has a
/// null `Matches`.
///
/// Equivalent to `CheckHits.MatchesAsserter`.
pub struct MatchesAsserter {
    weight: Arc<dyn Weight>,
    /// The searcher's leaves, to keep the context `doSetNextReader` receives.
    leaves: Vec<Arc<LeafReaderContext>>,
    context: Option<Arc<LeafReaderContext>>,
    last_checked_doc: i32,
    // with intra-segment concurrency, we may start from a doc id that isn't -1. We need to make
    // sure that we don't go outside of the bounds of the current slice, meaning -1 can't be
    // reliably used to signal that we are collecting the first doc for a given segment partition.
    collected_once: bool,
}

impl std::fmt::Debug for MatchesAsserter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MatchesAsserter")
            .field("last_checked_doc", &self.last_checked_doc)
            .finish_non_exhaustive()
    }
}

impl MatchesAsserter {
    /// `new MatchesAsserter(query, searcher)`.
    ///
    /// # Errors
    ///
    /// Propagates the searcher's I/O errors.
    pub fn new(query: &Arc<dyn Query>, searcher: &IndexSearcher) -> Result<MatchesAsserter> {
        let weight = searcher.create_weight(
            searcher.rewrite(Arc::clone(query))?,
            ScoreMode::COMPLETE_NO_SCORES,
            1.0,
        )?;
        Ok(MatchesAsserter {
            weight,
            leaves: searcher.get_leaf_contexts().to_vec(),
            context: None,
            last_checked_doc: -1,
            collected_once: false,
        })
    }
}

impl LeafCollector for MatchesAsserter {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        let context = self
            .context
            .as_ref()
            .expect("INVARIANT: doSetNextReader ran before collect");
        let matches = self.weight.matches(context, doc)?;
        if matches.is_none() {
            Assert::fail(&format!(
                "Unexpected null Matches object in doc{doc} for query {}",
                query_to_string(&*self.weight.get_query())
            ));
        }
        if self.collected_once && self.last_checked_doc != doc - 1 {
            if let Some(previous) = self.weight.matches(context, doc - 1)? {
                // `assertNull(message, object)`: `message + " expected null, but was:<" +
                // object + ">"`.
                Assert::fail(&format!(
                    "Unexpected non-null Matches object in non-matching doc{doc} for query {} \
                     expected null, but was:<{}>",
                    query_to_string(&*self.weight.get_query()),
                    matches_string(&*previous)
                ));
            }
        }
        self.collected_once = true;
        self.last_checked_doc = doc;
        Ok(())
    }
}

impl Collector for MatchesAsserter {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        // `doSetNextReader`
        let ours = self
            .leaves
            .iter()
            .find(|leaf| std::ptr::eq(&***leaf, context))
            .cloned()
            .expect("INVARIANT: the searcher collects its own leaf contexts");
        self.context = Some(ours);
        self.last_checked_doc = -1;
        Ok(Box::new(self))
    }

    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE_NO_SCORES
    }
}
