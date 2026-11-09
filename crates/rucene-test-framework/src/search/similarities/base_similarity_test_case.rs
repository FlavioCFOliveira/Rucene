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

//! Port of `org.apache.lucene.tests.search.similarities.BaseSimilarityTestCase`:
//! abstract class to do basic tests for a similarity. NOTE: This test focuses
//! on the similarity impl, nothing else. The [stretch] goal is for this test
//! to be so thorough in testing a new Similarity that if this test passes,
//! then all Lucene tests should also pass. Ie, if there is some bug in a
//! given Similarity that this test fails to catch then this test needs to be
//! improved!
//!
//! The class is declared by [`class()`]; its members are the trait
//! [`BaseSimilarityTestCase`] (the abstract `getSimilarity(Random)` and the
//! tests) and the static helpers [`new_corpus`] and [`new_term`]. The class
//! state (`READER`, `DIR`) is [`CLASS_STATE`].
//!
//! **Divergence** (`doTestScoring`'s `finally`): Java prints the similarity,
//! corpus, term, norm and freq when the case fails; the port prints the
//! same lines from a drop guard that runs while a failure unwinds.

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::document::{Document, FieldType, TextField};
use rucene::error::Result;
use rucene::index::{IndexOptions, IndexReader, IndexableFieldType, LeafReader};
use rucene::search::similarities::{CollectionStatistics, Explanation, Similarity, TermStatistics};
use rucene::util::small_float::SmallFloat;
use rucene::util::{ArrayUtil, BytesRef};

use crate::analysis::MockAnalyzer;
use crate::index::RandomIndexWriter;
use crate::search::CheckHits;
use crate::util::java_exceptions::rethrow_java;
use crate::util::lucene_test_case::TestDirectory;
use crate::util::{LuceneTestCase, TestUtil};

/// `static LeafReader READER; static Directory DIR;`.
pub static CLASS_STATE: Mutex<Option<(Arc<dyn LeafReader>, TestDirectory)>> = Mutex::new(None);

/// The declaration of
/// `org.apache.lucene.tests.search.similarities.BaseSimilarityTestCase`
/// (`public abstract class BaseSimilarityTestCase extends LuceneTestCase`)
/// with its hooks and tests.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        fn t(name: &str, body: fn(&dyn BaseSimilarityTestCase) -> Result<()>) -> Method {
            similarity_test(name, body)
        }
        Class::builder("org.apache.lucene.tests.search.similarities.BaseSimilarityTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&LuceneTestCase::class())
            .method(
                Method::new("beforeClass")
                    .modifiers(modifier::PUBLIC | modifier::STATIC)
                    .throws(&["java.lang.Exception"])
                    .annotate(BeforeClass)
                    .static_body(|| before_class().unwrap_or_else(|e| rethrow_java(e))),
            )
            .method(
                Method::new("afterClass")
                    .modifiers(modifier::PUBLIC | modifier::STATIC)
                    .throws(&["java.lang.Exception"])
                    .annotate(AfterClass)
                    .static_body(|| after_class().unwrap_or_else(|e| rethrow_java(e))),
            )
            .method(t("testRandomScoring", |c| c.test_random_scoring()))
            .method(t("testBulkScore", |c| c.test_bulk_score()))
            .build()
    });
    Arc::clone(&CLASS)
}

/// A test method of a `BaseSimilarityTestCase` subclass.
pub fn similarity_test(
    name: &str,
    body: impl Fn(&dyn BaseSimilarityTestCase) -> Result<()> + Send + Sync + 'static,
) -> Method {
    Method::new(name).throws(&["java.lang.Exception"]).body(
        move |inst: &mut SimilarityTestInstance| {
            let case = Arc::clone(&inst.case);
            body(&*case).unwrap_or_else(|e| rethrow_java(e))
        },
    )
}

/// The instance of a `BaseSimilarityTestCase` subclass the runner creates.
pub struct SimilarityTestInstance {
    /// The subclass.
    pub case: Arc<dyn BaseSimilarityTestCase>,
}

impl SimilarityTestInstance {
    /// The instance of a subclass.
    pub fn new<T: BaseSimilarityTestCase>(case: T) -> Self {
        SimilarityTestInstance {
            case: Arc::new(case),
        }
    }

    /// The instance of a subclass the caller also keeps (for the members
    /// the subclass declares itself).
    pub fn shared(case: Arc<dyn BaseSimilarityTestCase>) -> Self {
        SimilarityTestInstance { case }
    }
}

/// `beforeClass()` (`@BeforeClass`): with norms.
pub fn before_class() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let mut field_type = FieldType::new_from(TextField::type_not_stored());
    field_type.set_omit_norms(true)?;
    if LuceneTestCase::random().next_boolean() {
        field_type.set_index_options(IndexOptions::DOCS_AND_FREQS)?;
    } else {
        field_type.set_index_options(IndexOptions::DOCS_AND_CUSTOM_FREQS)?;
    }
    let field = LuceneTestCase::new_field("field", "value", &field_type);
    let writer = if field.field_type().index_options() == IndexOptions::DOCS_AND_CUSTOM_FREQS {
        let analyzer = MockAnalyzer::with_term_freq(
            &*LuceneTestCase::random(),
            Arc::new(|_x: &str| {
                if LuceneTestCase::random().next_boolean() {
                    100
                } else {
                    i32::MAX
                }
            }),
        );
        RandomIndexWriter::new(
            &*LuceneTestCase::random(),
            dir.as_directory(),
            Arc::new(analyzer),
        )?
    } else {
        RandomIndexWriter::new_with_mock_analyzer(&*LuceneTestCase::random(), dir.as_directory())?
    };
    let mut doc = Document::new();
    doc.add(Box::new(field));
    writer.add_document(&doc)?;
    let reader = writer.get_reader()?;
    let reader = LuceneTestCase::get_only_leaf_reader(reader as Arc<dyn IndexReader>)?;
    writer.close()?;
    drop(writer);
    *CLASS_STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((reader, dir));
    Ok(())
}

/// `afterClass()` (`@AfterClass`): `IOUtils.close(READER, DIR)`.
pub fn after_class() -> Result<()> {
    let state = CLASS_STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some((reader, mut dir)) = state {
        let closed_reader = Arc::clone(&reader).as_index_reader().close();
        drop(reader);
        let closed_dir = dir.close();
        closed_reader.and(closed_dir)?;
    }
    Ok(())
}

/// `MAXDOC_FORTESTING`.
pub const MAXDOC_FORTESTING: i64 = 1 << 48;
/// `MAXTOKENS_FORTESTING`: must be at least `MAXDOC_FORTESTING +
/// Integer.MAX_VALUE`.
pub const MAXTOKENS_FORTESTING: i64 = 1 << 49;

/// `newCorpus(Random random, int norm)`: returns a random corpus that is at
/// least possible given the norm value for a single document.
pub fn new_corpus(random: &dyn Random, norm: i32) -> Result<CollectionStatistics> {
    // lower bound of tokens in the collection (you produced this norm somehow)
    let lower_bound: i64 = if norm == 0 {
        // norms are omitted, but there must have been at least one token to produce that norm
        1
    } else {
        // minimum value that would decode to such a norm
        i64::from(SmallFloat::byte4_to_int(norm as u8))
    };
    let max_doc: i64 = match random.next_int_bound(6) {
        // 1 doc collection
        0 => 1,
        // 2 doc collection
        1 => 2,
        // tiny collection
        2 => TestUtil::next_long(random, 3, 16),
        // small collection
        3 => TestUtil::next_long(random, 16, 100000),
        // big collection
        4 => TestUtil::next_long(random, 100000, MAXDOC_FORTESTING),
        // yuge collection
        _ => MAXDOC_FORTESTING,
    };
    let doc_count: i64 = match random.next_int_bound(3) {
        // sparsest field
        0 => 1,
        // sparse field
        1 => TestUtil::next_long(random, 1, max_doc),
        // fully populated
        _ => max_doc,
    };
    // random docsize: but can't require docs to have > 2B tokens
    let upper_bound = match doc_count.checked_mul(i64::from(i32::MAX)) {
        Some(product) => MAXTOKENS_FORTESTING.min(product),
        None => MAXTOKENS_FORTESTING,
    };
    let sum_doc_freq: i64 = match random.next_int_bound(3) {
        // shortest possible docs
        0 => doc_count,
        // biggest possible docs
        1 => upper_bound + 1 - lower_bound,
        // random docsize
        _ => TestUtil::next_long(random, doc_count, upper_bound + 1 - lower_bound),
    };
    let sum_total_term_freq: i64 = match random.next_int_bound(4) {
        // term frequencies were omitted
        0 => sum_doc_freq,
        // no repetition of terms (except to satisfy this norm)
        1 => sum_doc_freq - 1 + lower_bound,
        // maximum repetition of terms
        2 => upper_bound,
        // random repetition
        _ => {
            randomizedtesting::jdk::exception::java_assert_no_message(
                sum_doc_freq - 1 + lower_bound <= upper_bound,
            );
            TestUtil::next_long(random, sum_doc_freq - 1 + lower_bound, upper_bound)
        }
    };
    CollectionStatistics::new(
        "field",
        max_doc,
        doc_count,
        sum_total_term_freq,
        sum_doc_freq,
    )
}

/// `TERM`.
fn term_bytes() -> BytesRef {
    BytesRef::new(b"term".to_vec())
}

/// `newTerm(Random random, CollectionStatistics corpus)`: returns new random
/// term, that fits within the bounds of the corpus.
pub fn new_term(random: &dyn Random, corpus: &CollectionStatistics) -> Result<TermStatistics> {
    let doc_freq: i64 = match random.next_int_bound(3) {
        // rare term
        0 => 1,
        // common term
        1 => corpus.doc_count(),
        // random specificity
        _ => TestUtil::next_long(random, 1, corpus.doc_count()),
    };
    // can't require docs to have > 2B tokens
    let upper_bound = match doc_freq.checked_mul(i64::from(i32::MAX)) {
        Some(product) => corpus.sum_total_term_freq().min(product),
        None => corpus.sum_total_term_freq(),
    };
    let total_term_freq = if corpus.sum_total_term_freq() == corpus.sum_doc_freq() {
        // omitTF
        doc_freq
    } else {
        match random.next_int_bound(3) {
            // no repetition
            0 => doc_freq,
            // maximum repetition
            1 => upper_bound,
            // random repetition
            _ => TestUtil::next_long(random, doc_freq, upper_bound),
        }
    };
    TermStatistics::new(term_bytes(), doc_freq, total_term_freq)
}

/// `Math.toIntExact(value)`.
fn to_int_exact(value: i64) -> Result<i32> {
    i32::try_from(value)
        .map_err(|_| rucene::error::LuceneError::Arithmetic("integer overflow".to_string()))
}

/// `Explanation.match(freq, "freq, occurrences of term within document")`.
fn freq_explanation(freq: f32) -> Explanation {
    Explanation::matched(
        freq,
        "freq, occurrences of term within document",
        Vec::new(),
    )
}

/// Port of the members of `BaseSimilarityTestCase`.
pub trait BaseSimilarityTestCase: Send + Sync + 'static {
    /// `getSimilarity(Random random)`: return a new similarity with all
    /// parameters randomized within valid ranges.
    fn get_similarity(&self, random: &dyn Random) -> Result<Arc<dyn Similarity>>;

    /// `testRandomScoring()`: tests scoring across a bunch of random
    /// terms/corpora/frequencies for each possible document length.
    fn test_random_scoring(&self) -> Result<()> {
        let random = LuceneTestCase::random();
        let iterations = LuceneTestCase::at_least(1);
        for _ in 0..iterations {
            // pull a new similarity to switch up parameters
            let similarity = self.get_similarity(&*random)?;
            for _ in 0..3 {
                // for each norm value...
                for k in 1..256 {
                    let corpus = new_corpus(&*random, k)?;
                    for _ in 0..10 {
                        let term = new_term(&*random, &corpus)?;
                        let freq: f32 = if term.total_term_freq() == term.doc_freq() {
                            // omit TF
                            1.0
                        } else if term.doc_freq() == 1 {
                            // only one document, all the instances must be here.
                            to_int_exact(term.total_term_freq())? as f32
                        } else {
                            // there is at least one other document, and those must have at least
                            // 1 instance each.
                            let upper_bound = to_int_exact(
                                (term.total_term_freq() - term.doc_freq() + 1)
                                    .min(i64::from(i32::MAX)),
                            )?;
                            if random.next_boolean() {
                                // integer freq
                                match random.next_int_bound(3) {
                                    // smallest freq
                                    0 => 1.0,
                                    // largest freq
                                    1 => upper_bound as f32,
                                    // random freq
                                    _ => TestUtil::next_int(&*random, 1, upper_bound) as f32,
                                }
                            } else {
                                // float freq
                                let mut freq_candidate = match random.next_int_bound(2) {
                                    // smallest freq
                                    0 => f32::from_bits(1),
                                    // random freq
                                    _ => upper_bound as f32 * random.next_float(),
                                };
                                // we need to be 2nd float value at a minimum, the pairwise test will
                                // check MIN_VALUE in this case.
                                // this avoids testing frequencies of 0 which seem wrong to allow (we
                                // should enforce computeSlopFactor etc)
                                if freq_candidate <= f32::from_bits(1) {
                                    freq_candidate = f32::from_bits(1).next_up();
                                }
                                freq_candidate
                            }
                        };
                        // we just limit the test to "reasonable" boost values but don't enforce
                        // this anywhere.
                        let boost: f32 = match random.next_int_bound(5) {
                            // minimum value (not enforced)
                            0 => 0.0,
                            // tiny value
                            1 => f32::from_bits(1),
                            // no-op value (sometimes treated special in explanations)
                            2 => 1.0,
                            // maximum value (not enforceD)
                            3 => i32::MAX as f32,
                            // random value
                            _ => random.next_float() * i32::MAX as f32,
                        };
                        do_test_scoring(&*similarity, &corpus, &term, boost, freq, k)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// `testBulkScore()`.
    fn test_bulk_score(&self) -> Result<()> {
        let random = LuceneTestCase::random();
        let similarity = self.get_similarity(&*random)?;
        let corpus = new_corpus(&*random, 1)?;
        let term = new_term(&*random, &corpus)?;
        let boost = LuceneTestCase::random().next_float_bound(5.0);
        let scorer = similarity.scorer(boost, &corpus, std::slice::from_ref(&term));
        let mut bulk_scorer = scorer.as_bulk_sim_scorer();
        let freq_upper_bound =
            to_int_exact((term.total_term_freq() - term.doc_freq() + 1).min(i64::from(i32::MAX)))?;
        let mut freqs: Vec<f32> = Vec::new();
        let mut norms: Vec<i64> = Vec::new();
        let mut scores: Vec<f32> = Vec::new();
        let iters = LuceneTestCase::at_least(3);
        for _ in 0..iters {
            let size = TestUtil::next_int(&*random, 0, 200) as usize;
            if size > freqs.len() {
                freqs = vec![0f32; ArrayUtil::oversize(size, 4)];
                norms = vec![0i64; freqs.len()];
                scores = vec![0f32; freqs.len()];
            }
            for i in 0..size {
                freqs[i] = TestUtil::next_int(&*random, 1, freq_upper_bound) as f32;
                norms[i] = TestUtil::next_long(&*random, 1, 255);
            }
            let expected_scores: Vec<f32> = (0..size)
                .map(|i| scorer.score(freqs[i], norms[i]))
                .collect();
            bulk_scorer.score(size, &freqs, &norms, &mut scores);
            for (i, (e, a)) in expected_scores.iter().zip(&scores[..size]).enumerate() {
                if e.to_bits() != a.to_bits() && e != a {
                    Assert::fail(&format!(
                        "arrays first differed at element [{i}]; expected:<{e:?}> but was:<{a:?}>"
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Prints what Java's `finally` prints when a case fails.
struct FailureReport<'a> {
    success: bool,
    similarity: &'a dyn Similarity,
    corpus: &'a CollectionStatistics,
    term: &'a TermStatistics,
    norm: i32,
    freq: f32,
}

impl Drop for FailureReport<'_> {
    fn drop(&mut self) {
        if !self.success {
            println!("{:?}", self.similarity);
            println!("{:?}", self.corpus);
            println!("{:?}", self.term);
            if self.norm == 0 {
                println!("norms=omitted");
            } else {
                println!(
                    "norm={} (doc length ~ {})",
                    self.norm,
                    SmallFloat::byte4_to_int(self.norm as u8)
                );
            }
            println!("freq={:?}", self.freq);
        }
    }
}

/// `doTestScoring(...)` (`private static`): runs for a single test case, so
/// that if you hit a test failure you can write a reproducer just for that
/// scenario.
pub fn do_test_scoring(
    similarity: &dyn Similarity,
    corpus: &CollectionStatistics,
    term: &TermStatistics,
    boost: f32,
    freq: f32,
    norm: i32,
) -> Result<()> {
    let mut report = FailureReport {
        success: false,
        similarity,
        corpus,
        term,
        norm,
        freq,
    };
    let is_indri = similarity.java_class_name()
        == "org.apache.lucene.search.similarities.IndriDirichletSimilarity";
    let norm_l = i64::from(norm);
    let scorer = similarity.scorer(boost, corpus, std::slice::from_ref(term));
    let max_score = scorer.score(f32::MAX, 1);
    Assert::assert_false_msg("maxScore is NaN", max_score.is_nan());

    let score = scorer.score(freq, norm_l);
    // check that score isn't infinite or negative
    Assert::assert_true_msg(&format!("infinite/NaN score: {score:?}"), score.is_finite());
    if !is_indri {
        Assert::assert_true_msg(&format!("negative score: {score:?}"), score >= 0.0);
    }
    Assert::assert_true_msg(
        &format!("greater than maxScore: {score:?}>{max_score:?}"),
        score <= max_score,
    );
    // check explanation matches
    let explanation = scorer.explain(&freq_explanation(freq), norm_l);
    if f64::from(score) != explanation.value().double_value() {
        Assert::fail(&format!("expected: {score:?}, got: {explanation}"));
    }
    if LuceneTestCase::rarely() {
        CheckHits::verify_explanation("<test query>", 0, score, true, &explanation);
    }

    // check score(freq-1), given the same norm it should be <= score(freq) [scores non-decreasing
    // for more term occurrences]
    let prev_freq = if LuceneTestCase::random().next_boolean()
        && freq == (freq as i32) as f32
        && freq > 1.0
        && term.doc_freq() > 1
    {
        // previous in integer space
        freq - 1.0
    } else {
        // previous in float space (e.g. for sloppyPhrase)
        freq.next_down()
    };
    let prev_score = scorer.score(prev_freq, norm_l);
    // check that score isn't infinite or negative
    Assert::assert_true(prev_score.is_finite());
    if !is_indri {
        Assert::assert_true(prev_score >= 0.0);
    }
    // check explanation matches
    let prev_explanation = scorer.explain(&freq_explanation(prev_freq), norm_l);
    if f64::from(prev_score) != prev_explanation.value().double_value() {
        Assert::fail(&format!(
            "expected: {prev_score:?}, got: {prev_explanation}"
        ));
    }
    if LuceneTestCase::rarely() {
        CheckHits::verify_explanation(
            "test query (prevFreq)",
            0,
            prev_score,
            true,
            &prev_explanation,
        );
    }

    if prev_score > score {
        println!("{prev_explanation}");
        println!("{explanation}");
        Assert::fail(&format!(
            "score({prev_freq:?})={prev_score:?} > score({freq:?})={score:?}"
        ));
    }

    // check score(norm-1), given the same freq it should be >= score(norm) [scores non-decreasing
    // as docs get shorter]
    if norm > 1 {
        let prev_norm_score = scorer.score(freq, norm_l - 1);
        // check that score isn't infinite or negative
        Assert::assert_true(prev_norm_score.is_finite());
        if !is_indri {
            Assert::assert_true(prev_norm_score >= 0.0);
        }
        // check explanation matches
        let prev_norm_explanation = scorer.explain(&freq_explanation(freq), norm_l - 1);
        if f64::from(prev_norm_score) != prev_norm_explanation.value().double_value() {
            Assert::fail(&format!(
                "expected: {prev_norm_score:?}, got: {prev_norm_explanation}"
            ));
        }
        if LuceneTestCase::rarely() {
            CheckHits::verify_explanation(
                "test query (prevNorm)",
                0,
                prev_norm_score,
                true,
                &prev_norm_explanation,
            );
        }
        if prev_norm_score < score {
            println!("{prev_norm_explanation}");
            println!("{explanation}");
            Assert::fail(&format!(
                "score({freq:?},{})={prev_norm_score:?} < score({freq:?},{norm})={score:?}",
                norm - 1
            ));
        }
    }

    // check score(term-1), given the same freq/norm it should be >= score(term) [scores
    // non-decreasing as terms get rarer]
    // `freq < term.totalTermFreq()` compares a `float` with a `long`, so Java
    // converts the `long` to `float` (JLS 5.6.2): an `f64` comparison answers
    // differently once the `long` exceeds `float` precision.
    if term.doc_freq() > 1 && freq < term.total_term_freq() as f32 {
        let prev_term = TermStatistics::new(
            term.term().clone(),
            term.doc_freq() - 1,
            term.total_term_freq() - 1,
        )?;
        let prev_term_scorer = similarity.scorer(boost, corpus, std::slice::from_ref(term));
        let prev_term_score = prev_term_scorer.score(freq, norm_l);
        // check that score isn't infinite or negative
        Assert::assert_true(prev_term_score.is_finite());
        if !is_indri {
            Assert::assert_true(prev_term_score >= 0.0);
        }
        // check explanation matches
        let prev_term_explanation = prev_term_scorer.explain(&freq_explanation(freq), norm_l);
        if f64::from(prev_term_score) != prev_term_explanation.value().double_value() {
            Assert::fail(&format!(
                "expected: {prev_term_score:?}, got: {prev_term_explanation}"
            ));
        }
        if LuceneTestCase::rarely() {
            CheckHits::verify_explanation(
                "test query (prevTerm)",
                0,
                prev_term_score,
                true,
                &prev_term_explanation,
            );
        }

        if prev_term_score < score {
            println!("{prev_term_explanation}");
            println!("{explanation}");
            Assert::fail(&format!(
                "score({freq:?},{prev_term:?})={prev_term_score:?} < score({freq:?},{term:?})={score:?}"
            ));
        }
    }

    report.success = true;
    Ok(())
}
