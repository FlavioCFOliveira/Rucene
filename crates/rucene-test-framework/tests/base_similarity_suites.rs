//! The `BaseSimilarityTestCase` suites (rmp #356): the inherited cases run,
//! under the ported runner and `LuceneTestCase`'s rule chain, on the
//! similarities Lucene's own subclasses bind
//! (`lucene/core/src/test/org/apache/lucene/search/similarities/`) when
//! those subclasses declare no test method of their own:
//! `TestIndependenceChiSquared`, `TestLMJelinekMercerSimilarity`,
//! `TestIndependenceStandardized`, `TestIndependenceSaturated`,
//! `TestIndriDirichletSimilarity`, `TestLMDirichletSimilarity`, and the
//! abstract `AxiomaticTestCase`, `DistributionTestCase` and
//! `BasicModelTestCase` through their concrete subclasses, as Lucene runs
//! them (`TestAxiomaticF1EXP` … `TestAxiomaticF3LOG`, `TestDistributionLL`,
//! `TestDistributionSPL`, `TestBasicModelG`, `TestBasicModelIF`,
//! `TestBasicModelIn`, `TestBasicModelIne`).
//!
//! `TestBM25Similarity`, `TestBooleanSimilarity`, `TestClassicSimilarity`
//! and `TestRawTFSimilarity` declare test methods of their own: each is
//! bound once, with them, in `core_search_tests.rs` (rmp #389).
//! `TestBM25Similarity.getSimilarity` stays here for the draw oracle.
//!
//! `RUCENE_SUITE_SEEDS=<n>` runs each suite on the first `n` seeds of
//! `suite_seeds::SEEDS`; `test_similarity_suite_draws_match_lucene`
//! compares the seeded draws with `BaseSuitesDrawsOracle`'s.

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{modifier, Class, Constructor};
use rucene::error::Result;
use rucene::search::similarities::{
    AfterEffect, AfterEffectB, AfterEffectL, AxiomaticF1EXP, AxiomaticF1LOG, AxiomaticF2EXP,
    AxiomaticF2LOG, AxiomaticF3EXP, AxiomaticF3LOG, BM25Similarity, BasicModel, BasicModelG,
    BasicModelIF, BasicModelIn, BasicModelIne, DFISimilarity, DFRSimilarity, Distribution,
    DistributionLL, DistributionSPL, IBSimilarity, IndependenceChiSquared, IndependenceSaturated,
    IndependenceStandardized, IndriDirichletSimilarity, LMDirichletSimilarity,
    LMJelinekMercerSimilarity, Lambda, LambdaDF, LambdaTTF, NoNormalization, Normalization,
    NormalizationH1, NormalizationH2, NormalizationH3, NormalizationZ, Similarity,
};
use rucene_test_framework::search::similarities::base_similarity_test_case::{
    self, BaseSimilarityTestCase, SimilarityTestInstance,
};

mod draw_hooks;
mod suite_draws;
mod suite_seeds;
use suite_seeds::run_suite;

const MIN_VALUE: f32 = f32::from_bits(1);
const INT_MAX: f32 = i32::MAX as f32;

/// A similarity binding whose `getSimilarity` is `make`.
struct SimilarityBinding {
    make: fn(&dyn Random) -> Result<Arc<dyn Similarity>>,
}

impl BaseSimilarityTestCase for SimilarityBinding {
    fn get_similarity(&self, random: &dyn Random) -> Result<Arc<dyn Similarity>> {
        (self.make)(random)
    }
}

/// `TestIndependenceChiSquared.getSimilarity`.
fn independence_chi_squared(_random: &dyn Random) -> Result<Arc<dyn Similarity>> {
    Ok(Arc::new(DFISimilarity::new(Box::new(
        IndependenceChiSquared::new(),
    ))))
}

/// `TestIndependenceStandardized.getSimilarity`.
fn independence_standardized(_random: &dyn Random) -> Result<Arc<dyn Similarity>> {
    Ok(Arc::new(DFISimilarity::new(Box::new(
        IndependenceStandardized::new(),
    ))))
}

/// `TestIndependenceSaturated.getSimilarity`.
fn independence_saturated(_random: &dyn Random) -> Result<Arc<dyn Similarity>> {
    Ok(Arc::new(DFISimilarity::new(Box::new(
        IndependenceSaturated::new(),
    ))))
}

/// `TestLMJelinekMercerSimilarity.getSimilarity`: smoothing parameter
/// lambda: (0..1].
fn lm_jelinek_mercer(random: &dyn Random) -> Result<Arc<dyn Similarity>> {
    let lambda = match random.next_int_bound(3) {
        // tiny value
        0 => MIN_VALUE,
        // maximum value
        1 => 1.0,
        // random value
        _ => random.next_float(),
    };
    Ok(Arc::new(LMJelinekMercerSimilarity::with_lambda(lambda)?))
}

/// `mu` as `TestIndriDirichletSimilarity` and `TestLMDirichletSimilarity`
/// draw it: smoothing parameter mu, unbounded.
fn draw_mu(random: &dyn Random) -> f32 {
    match random.next_int_bound(4) {
        // minimum value
        0 => 0.0,
        // tiny value
        1 => MIN_VALUE,
        // maximum value
        2 => INT_MAX,
        // random value
        _ => INT_MAX * random.next_float(),
    }
}

/// `TestIndriDirichletSimilarity.getSimilarity`.
fn indri_dirichlet(random: &dyn Random) -> Result<Arc<dyn Similarity>> {
    let mu = draw_mu(random);
    Ok(Arc::new(IndriDirichletSimilarity::with_mu(mu)))
}

/// `TestLMDirichletSimilarity.getSimilarity`.
fn lm_dirichlet(random: &dyn Random) -> Result<Arc<dyn Similarity>> {
    let mu = draw_mu(random);
    Ok(Arc::new(LMDirichletSimilarity::with_mu(mu)?))
}

/// `TestBM25Similarity.getSimilarity`.
fn bm25(random: &dyn Random) -> Result<Arc<dyn Similarity>> {
    // term frequency normalization parameter k1
    let k1 = match random.next_int_bound(4) {
        // minimum value
        0 => 0.0,
        // tiny value
        1 => MIN_VALUE,
        // maximum value
        // upper bounds on individual term's score is 43.262806 * (k1 + 1) * boost
        // we just limit the test to "reasonable" k1 values but don't enforce this anywhere.
        2 => INT_MAX,
        // random value
        _ => INT_MAX * random.next_float(),
    };

    // length normalization parameter b [0 .. 1]
    let b = match random.next_int_bound(4) {
        // minimum value
        0 => 0.0,
        // tiny value
        1 => MIN_VALUE,
        // maximum value
        2 => 1.0,
        // random value
        _ => random.next_float(),
    };
    Ok(Arc::new(BM25Similarity::with_parameters(k1, b)?))
}

/// `AxiomaticTestCase.getSimilarity`, building the model with
/// `getAxiomaticModel(s, queryLen, k)`.
fn axiomatic(
    random: &dyn Random,
    model: fn(f32, i32, f32) -> Result<Arc<dyn Similarity>>,
) -> Result<Arc<dyn Similarity>> {
    // axiomatic parameter s
    let s = match random.next_int_bound(4) {
        // minimum value
        0 => 0.0,
        // tiny value
        1 => MIN_VALUE,
        // maximum value
        2 => 1.0,
        // random value
        _ => random.next_float(),
    };
    // axiomatic query length
    let query_len = match random.next_int_bound(4) {
        // minimum value
        0 => 0,
        // tiny value
        1 => 1,
        // maximum value
        2 => i32::MAX,
        // random value
        _ => random.next_int_bound(i32::MAX),
    };
    // axiomatic parameter k
    let k = match random.next_int_bound(4) {
        // minimum value
        0 => 0.0,
        // tiny value
        1 => MIN_VALUE,
        // maximum value
        2 => 1.0,
        // random value
        _ => random.next_float(),
    };
    model(s, query_len, k)
}

/// The normalization hyper-parameters `c`, `z` and `mu` and the
/// normalization of `DistributionTestCase` and `BasicModelTestCase`.
fn draw_normalization(random: &dyn Random) -> Result<Box<dyn Normalization>> {
    // normalization hyper-parameter c
    let c = match random.next_int_bound(4) {
        // minimum value
        0 => 0.0,
        // tiny value
        1 => MIN_VALUE,
        // maximum value
        // we just limit the test to "reasonable" c values but don't enforce this anywhere.
        2 => INT_MAX,
        // random value
        _ => INT_MAX * random.next_float(),
    };
    // normalization hyper-parameter z
    let z = match random.next_int_bound(3) {
        // minimum value
        0 => MIN_VALUE,
        // maximum value
        1 => 0.5f32.next_down(),
        // random value
        _ => {
            let zcand = random.next_float() / 2.0;
            if zcand == 0.0 {
                zcand.next_up()
            } else {
                zcand
            }
        }
    };
    // normalization hyper-parameter mu
    let mu = match random.next_int_bound(4) {
        // minimum value
        0 => 0.0,
        // tiny value
        1 => MIN_VALUE,
        // maximum value
        // we just limit the test to "reasonable" mu values but don't enforce this anywhere.
        2 => INT_MAX,
        // random value
        _ => INT_MAX * random.next_float(),
    };
    Ok(match random.next_int_bound(5) {
        0 => Box::new(NoNormalization::new()),
        1 => Box::new(NormalizationH1::with_c(c)?),
        2 => Box::new(NormalizationH2::with_c(c)?),
        3 => Box::new(NormalizationH3::with_mu(mu)?),
        _ => Box::new(NormalizationZ::with_z(z)?),
    })
}

/// `DistributionTestCase.getSimilarity` over `getDistribution()`.
fn distribution(
    random: &dyn Random,
    get_distribution: fn() -> Box<dyn Distribution>,
) -> Result<Arc<dyn Similarity>> {
    let lambda: Box<dyn Lambda> = if random.next_boolean() {
        Box::new(LambdaDF::new())
    } else {
        Box::new(LambdaTTF::new())
    };
    let normalization = draw_normalization(random)?;
    Ok(Arc::new(IBSimilarity::new(
        get_distribution(),
        lambda,
        normalization,
    )))
}

/// `BasicModelTestCase.getSimilarity` over `getBasicModel()`.
fn basic_model(
    random: &dyn Random,
    get_basic_model: fn() -> Box<dyn BasicModel>,
) -> Result<Arc<dyn Similarity>> {
    let after_effect: Box<dyn AfterEffect> = match random.next_int_bound(2) {
        0 => Box::new(AfterEffectL::new()),
        _ => Box::new(AfterEffectB::new()),
    };
    let normalization = draw_normalization(random)?;
    Ok(Arc::new(DFRSimilarity::new(
        get_basic_model(),
        after_effect,
        normalization,
    )))
}

// -----------------------------------------------------------------------------
// The classes
// -----------------------------------------------------------------------------

/// An abstract intermediate class (`AxiomaticTestCase` …), which adds no
/// test of its own.
fn abstract_class(name: &str) -> Arc<Class> {
    Class::builder(name)
        .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
        .extends(&base_similarity_test_case::class())
        .build()
}

static AXIOMATIC_TEST_CASE: LazyLock<Arc<Class>> =
    LazyLock::new(|| abstract_class("org.apache.lucene.search.similarities.AxiomaticTestCase"));
static DISTRIBUTION_TEST_CASE: LazyLock<Arc<Class>> =
    LazyLock::new(|| abstract_class("org.apache.lucene.search.similarities.DistributionTestCase"));
static BASIC_MODEL_TEST_CASE: LazyLock<Arc<Class>> =
    LazyLock::new(|| abstract_class("org.apache.lucene.search.similarities.BasicModelTestCase"));

/// A suite class of this binary.
fn suite(
    name: &str,
    parent: &Arc<Class>,
    make: fn(&dyn Random) -> Result<Arc<dyn Similarity>>,
) -> Arc<Class> {
    Class::builder(name)
        .extends(parent)
        .constructor(Constructor::no_args(move || {
            SimilarityTestInstance::new(SimilarityBinding { make })
        }))
        .build()
}

fn base() -> Arc<Class> {
    base_similarity_test_case::class()
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestSimilaritySuiteDraws
// -----------------------------------------------------------------------------

static DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestSimilaritySuiteDraws` of `BaseSuitesDrawsOracle`: the cases on
/// `TestBM25Similarity`'s similarities.
fn draws_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.search.similarities.TestSimilaritySuiteDraws")
        .extends(&base())
        .constructor(Constructor::no_args(|| {
            SimilarityTestInstance::new(SimilarityBinding { make: bm25 })
        }))
        .method(draw_hooks::record_tear_down(
            &DRAWS,
            draw_hooks::lucene_tear_down::<SimilarityTestInstance>,
        ))
        .method(draw_hooks::record_class_draw(&DRAWS))
        .build()
}

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

macro_rules! similarity_suites {
    ($($test:ident => $name:literal, $parent:expr, $make:expr;)*) => {
        $(
            #[test]
            fn $test() {
                let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
                run_suite(|| suite(concat!("org.apache.lucene.search.similarities.", $name), &$parent, $make));
            }
        )*
    };
}

similarity_suites! {
    test_independence_chi_squared => "TestIndependenceChiSquared", base(), independence_chi_squared;
    test_lm_jelinek_mercer_similarity => "TestLMJelinekMercerSimilarity", base(), lm_jelinek_mercer;
    test_independence_standardized => "TestIndependenceStandardized", base(), independence_standardized;
    test_independence_saturated => "TestIndependenceSaturated", base(), independence_saturated;
    test_indri_dirichlet_similarity => "TestIndriDirichletSimilarity", base(), indri_dirichlet;
    test_lm_dirichlet_similarity => "TestLMDirichletSimilarity", base(), lm_dirichlet;
    test_axiomatic_f1exp => "TestAxiomaticF1EXP", AXIOMATIC_TEST_CASE, |r| axiomatic(r, |s, _q, k| Ok(Arc::new(AxiomaticF1EXP::with_s_and_k(s, k)?)));
    test_axiomatic_f1log => "TestAxiomaticF1LOG", AXIOMATIC_TEST_CASE, |r| axiomatic(r, |s, _q, _k| Ok(Arc::new(AxiomaticF1LOG::with_s(s)?)));
    test_axiomatic_f2exp => "TestAxiomaticF2EXP", AXIOMATIC_TEST_CASE, |r| axiomatic(r, |s, _q, k| Ok(Arc::new(AxiomaticF2EXP::with_s_and_k(s, k)?)));
    test_axiomatic_f2log => "TestAxiomaticF2LOG", AXIOMATIC_TEST_CASE, |r| axiomatic(r, |s, _q, _k| Ok(Arc::new(AxiomaticF2LOG::with_s(s)?)));
    test_axiomatic_f3exp => "TestAxiomaticF3EXP", AXIOMATIC_TEST_CASE, |r| axiomatic(r, |s, q, _k| Ok(Arc::new(AxiomaticF3EXP::with_s_and_query_len(s, q)?)));
    test_axiomatic_f3log => "TestAxiomaticF3LOG", AXIOMATIC_TEST_CASE, |r| axiomatic(r, |s, q, _k| Ok(Arc::new(AxiomaticF3LOG::with_s_and_query_len(s, q)?)));
    test_distribution_ll => "TestDistributionLL", DISTRIBUTION_TEST_CASE, |r| distribution(r, || Box::new(DistributionLL::new()));
    test_distribution_spl => "TestDistributionSPL", DISTRIBUTION_TEST_CASE, |r| distribution(r, || Box::new(DistributionSPL::new()));
    test_basic_model_g => "TestBasicModelG", BASIC_MODEL_TEST_CASE, |r| basic_model(r, || Box::new(BasicModelG::new()));
    test_basic_model_if => "TestBasicModelIF", BASIC_MODEL_TEST_CASE, |r| basic_model(r, || Box::new(BasicModelIF::new()));
    test_basic_model_in => "TestBasicModelIn", BASIC_MODEL_TEST_CASE, |r| basic_model(r, || Box::new(BasicModelIn::new()));
    test_basic_model_ine => "TestBasicModelIne", BASIC_MODEL_TEST_CASE, |r| basic_model(r, || Box::new(BasicModelIne::new()));
}

#[test]
fn test_similarity_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match("base_similarity_draws.tsv", draws_class, &DRAWS);
}
