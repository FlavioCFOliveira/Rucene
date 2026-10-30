//! The prefetching and quantized vector scorers of `org.apache.lucene.codecs.hnsw`.
//!
//! Regression guard for rmp task #201. Two defects, both of them silent:
//!
//! 1. `PrefetchableFlatVectorScorer` forwarded every call to the scorer it
//!    wraps, so the prefetch the class exists to issue never reached the vector
//!    values. Lucene's three nested classes
//!    (`PrefetchableFlatVectorScorer.java:89-254`) each call
//!    `values().prefetch(nodes, numNodes)` before scoring a batch.
//! 2. `ScalarQuantizedVectorScorer` had no supplier of its own, so the only
//!    scorers reachable from it compared the *raw bytes* of a quantized field.
//!    Lucene's `ScalarQuantizedRandomVectorScorerSupplier`
//!    (`ScalarQuantizedVectorScorer.java:121-184`) scores through
//!    `ScalarQuantizedVectorSimilarity`, which applies the constant multiplier
//!    and both corrective offsets. The two disagree numerically, so a search
//!    over a quantized field returned scores from the wrong representation.

use std::sync::{Arc, Mutex};

use rucene::codecs::hnsw::graph_provider::PrefetchableFlatVectorScorer;
use rucene::codecs::hnsw::scalar_quantized_scorer::{
    ScalarQuantizedRandomVectorScorerSupplier, ScalarQuantizedVectorScorer,
};
use rucene::codecs::hnsw::{DefaultFlatVectorScorer, FlatVectorsScorer};
use rucene::error::Result;
use rucene::index::{FloatVectorValues, KnnVectorValues, VectorEncoding, VectorSimilarityFunction};
use rucene::util::hnsw::scorer::{
    RandomVectorScorer, RandomVectorScorerSupplier, UpdateableRandomVectorScorer,
};
use rucene::util::quantization::{
    LegacyQuantizedByteVectorValues, QuantizedByteVectorValues, ScalarQuantizedVectorSimilarity,
    ScalarQuantizer,
};

// -----------------------------------------------------------------------------
// 1. Prefetching
// -----------------------------------------------------------------------------

/// Float vectors that write down every `prefetch` they are asked for.
///
/// The log is shared with every copy, because Lucene's supplier hands the same
/// `KnnVectorValues` to the scorers it creates and copies it in `copy()`; a
/// prefetch issued through any of those handles is a prefetch that reached
/// this field.
#[derive(Clone)]
struct RecordingFloatVectorValues {
    vectors: Arc<Vec<Vec<f32>>>,
    prefetched: Arc<Mutex<Vec<Vec<i32>>>>,
}

impl RecordingFloatVectorValues {
    fn new() -> (Self, Arc<Mutex<Vec<Vec<i32>>>>) {
        let prefetched = Arc::new(Mutex::new(Vec::new()));
        let values = Self {
            vectors: Arc::new(vec![
                vec![1.0, 0.0],
                vec![0.0, 1.0],
                vec![1.0, 1.0],
                vec![2.0, 2.0],
            ]),
            prefetched: Arc::clone(&prefetched),
        };
        (values, prefetched)
    }
}

impl KnnVectorValues for RecordingFloatVectorValues {
    fn dimension(&self) -> i32 {
        2
    }

    fn size(&self) -> i32 {
        self.vectors.len() as i32
    }

    fn prefetch(&self, ords_to_prefetch: &[i32], num_ords: i32) -> Result<()> {
        self.prefetched
            .lock()
            .expect("the prefetch log is not poisoned")
            .push(ords_to_prefetch[..num_ords as usize].to_vec());
        Ok(())
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(self.clone()))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::FLOAT32
    }
}

impl FloatVectorValues for RecordingFloatVectorValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        Ok(self.vectors[ord as usize].clone())
    }

    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Ok(Box::new(self.clone()))
    }
}

fn prefetchable() -> PrefetchableFlatVectorScorer {
    PrefetchableFlatVectorScorer::new(Box::new(DefaultFlatVectorScorer::INSTANCE))
}

/// `PrefetchableRandomVectorScorer.bulkScore` prefetches the batch it is about
/// to score before it delegates (`PrefetchableFlatVectorScorer.java:159-162`).
#[test]
fn query_scorer_prefetches_the_batch_before_scoring_it() {
    let (values, prefetched) = RecordingFloatVectorValues::new();
    let scorer = prefetchable();

    let mut query_scorer = scorer
        .get_random_vector_scorer_float(
            VectorSimilarityFunction::EUCLIDEAN,
            Box::new(values),
            &[1.0, 1.0],
        )
        .expect("the query dimension matches the field");

    let nodes = [2i32, 0, 3];
    let mut scores = [0.0f32; 3];
    let max = query_scorer
        .bulk_score(&nodes, &mut scores, 3)
        .expect("scoring three ordinals succeeds");

    assert_eq!(
        &*prefetched.lock().expect("the prefetch log is not poisoned"),
        &[vec![2i32, 0, 3]],
        "the wrapper must prefetch exactly the nodes it is about to score"
    );
    // The wrapper changes no score: ordinal 2 is the query itself.
    assert_eq!(scores[0], 1.0);
    assert_eq!(max, 1.0);
}

/// `PrefetchableUpdatableRandomVectorScorer.bulkScore` does the same for the
/// scorers a supplier hands out (`PrefetchableFlatVectorScorer.java:230-233`).
#[test]
fn supplier_scorer_prefetches_the_batch_before_scoring_it() {
    let (values, prefetched) = RecordingFloatVectorValues::new();
    let scorer = prefetchable();

    let supplier = scorer
        .get_random_vector_scorer_supplier_float(
            VectorSimilarityFunction::EUCLIDEAN,
            Box::new(values),
        )
        .expect("a supplier over float vectors");
    let mut updateable = supplier.scorer().expect("a scorer from the supplier");
    updateable
        .set_scoring_ordinal(2)
        .expect("ordinal 2 is in range");

    let nodes = [0i32, 1];
    let mut scores = [0.0f32; 2];
    updateable
        .bulk_score(&nodes, &mut scores, 2)
        .expect("scoring two ordinals succeeds");

    assert_eq!(
        &*prefetched.lock().expect("the prefetch log is not poisoned"),
        &[vec![0i32, 1]],
        "the supplier's scorer must prefetch the nodes it is about to score"
    );
}

/// A scorer that writes down how it was asked to score.
struct RecordingScorer {
    calls: Arc<Mutex<Vec<String>>>,
}

impl RandomVectorScorer for RecordingScorer {
    fn score(&mut self, node: i32) -> Result<f32> {
        self.calls
            .lock()
            .expect("the call log is not poisoned")
            .push(format!("score({node})"));
        Ok(node as f32)
    }

    fn bulk_score(&mut self, nodes: &[i32], scores: &mut [f32], num_nodes: i32) -> Result<f32> {
        self.calls
            .lock()
            .expect("the call log is not poisoned")
            .push(format!("bulk_score({num_nodes})"));
        let mut max = f32::NEG_INFINITY;
        for i in 0..num_nodes as usize {
            scores[i] = nodes[i] as f32;
            max = max.max(scores[i]);
        }
        Ok(max)
    }

    fn max_ord(&self) -> i32 {
        4
    }
}

impl UpdateableRandomVectorScorer for RecordingScorer {
    fn set_scoring_ordinal(&mut self, _node: i32) -> Result<()> {
        Ok(())
    }
}

/// A [`FlatVectorsScorer`] that hands out [`RecordingScorer`]s.
#[derive(Debug)]
struct RecordingFlatVectorScorer {
    calls: Arc<Mutex<Vec<String>>>,
}

impl std::fmt::Display for RecordingFlatVectorScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RecordingFlatVectorScorer()")
    }
}

impl RandomVectorScorerSupplier for RecordingFlatVectorScorer {
    fn scorer(&self) -> Result<Box<dyn UpdateableRandomVectorScorer>> {
        Ok(Box::new(RecordingScorer {
            calls: Arc::clone(&self.calls),
        }))
    }

    fn copy(&self) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Ok(Box::new(Self {
            calls: Arc::clone(&self.calls),
        }))
    }
}

impl FlatVectorsScorer for RecordingFlatVectorScorer {
    fn get_random_vector_scorer_supplier_float(
        &self,
        _similarity_function: VectorSimilarityFunction,
        _vector_values: Box<dyn FloatVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Ok(Box::new(Self {
            calls: Arc::clone(&self.calls),
        }))
    }

    fn get_random_vector_scorer_supplier_byte(
        &self,
        _similarity_function: VectorSimilarityFunction,
        _vector_values: Box<dyn rucene::index::ByteVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Ok(Box::new(Self {
            calls: Arc::clone(&self.calls),
        }))
    }

    fn get_random_vector_scorer_float(
        &self,
        _similarity_function: VectorSimilarityFunction,
        _vector_values: Box<dyn FloatVectorValues>,
        _target: &[f32],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        Ok(Box::new(RecordingScorer {
            calls: Arc::clone(&self.calls),
        }))
    }

    fn get_random_vector_scorer_byte(
        &self,
        _similarity_function: VectorSimilarityFunction,
        _vector_values: Box<dyn rucene::index::ByteVectorValues>,
        _target: &[u8],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        Ok(Box::new(RecordingScorer {
            calls: Arc::clone(&self.calls),
        }))
    }
}

/// Lucene's two prefetching scorers reach the delegate differently, and the
/// asymmetry is deliberate: `PrefetchableRandomVectorScorer.bulkScore` calls
/// `delegate.bulkScore(...)` (`PrefetchableFlatVectorScorer.java:161`), while
/// `PrefetchableUpdatableRandomVectorScorer.bulkScore` calls `super.bulkScore`
/// (`PrefetchableFlatVectorScorer.java:232`) — the `RandomVectorScorer`
/// default, which loops over `score(node)`. A port that used the delegate's
/// `bulkScore` in both places would silently change which code path a
/// bulk-scoring implementation takes.
#[test]
fn the_two_wrappers_reach_the_delegate_the_way_lucene_does() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let scorer = PrefetchableFlatVectorScorer::new(Box::new(RecordingFlatVectorScorer {
        calls: Arc::clone(&calls),
    }));
    let (values, _) = RecordingFloatVectorValues::new();

    let mut query_scorer = scorer
        .get_random_vector_scorer_float(
            VectorSimilarityFunction::EUCLIDEAN,
            Box::new(values.clone()),
            &[1.0, 1.0],
        )
        .expect("a query scorer");
    let mut scores = [0.0f32; 2];
    query_scorer
        .bulk_score(&[1, 3], &mut scores, 2)
        .expect("bulk scoring succeeds");
    assert_eq!(
        &*calls.lock().expect("the call log is not poisoned"),
        &["bulk_score(2)".to_string()],
        "the query scorer must delegate the whole batch"
    );
    assert_eq!(scores, [1.0, 3.0]);

    calls.lock().expect("the call log is not poisoned").clear();
    let supplier = scorer
        .get_random_vector_scorer_supplier_float(
            VectorSimilarityFunction::EUCLIDEAN,
            Box::new(values),
        )
        .expect("a supplier");
    let mut updateable = supplier.scorer().expect("a scorer");
    updateable
        .bulk_score(&[1, 3], &mut scores, 2)
        .expect("bulk scoring succeeds");
    assert_eq!(
        &*calls.lock().expect("the call log is not poisoned"),
        &["score(1)".to_string(), "score(3)".to_string()],
        "the supplier's scorer must score node by node, as super.bulkScore does"
    );
    assert_eq!(scores, [1.0, 3.0]);
}

// -----------------------------------------------------------------------------
// 2. Quantized scoring
// -----------------------------------------------------------------------------

/// Four quantized vectors and their corrective offsets, held in memory.
#[derive(Clone)]
struct MemoryQuantizedVectorValues {
    vectors: Vec<Vec<u8>>,
    offsets: Vec<f32>,
    quantizer: ScalarQuantizer,
}

impl MemoryQuantizedVectorValues {
    fn new() -> Self {
        Self {
            vectors: vec![
                vec![10, 20, 30, 40],
                vec![40, 30, 20, 10],
                vec![5, 5, 5, 5],
                vec![100, 90, 80, 70],
            ],
            offsets: vec![0.25, -0.5, 0.125, 1.5],
            quantizer: ScalarQuantizer::new(-1.0, 1.0, 7).expect("a valid quantizer"),
        }
    }
}

impl QuantizedByteVectorValues for MemoryQuantizedVectorValues {
    fn vector_value(&mut self, ord: i32) -> Result<&[u8]> {
        Ok(&self.vectors[ord as usize])
    }

    fn get_score_correction_constant(&mut self, ord: i32) -> Result<f32> {
        Ok(self.offsets[ord as usize])
    }

    fn size(&self) -> i32 {
        self.vectors.len() as i32
    }

    fn dimension(&self) -> i32 {
        4
    }

    fn copy(&self) -> Result<Box<dyn QuantizedByteVectorValues>> {
        Ok(Box::new(self.clone()))
    }
}

impl LegacyQuantizedByteVectorValues for MemoryQuantizedVectorValues {
    fn scalar_quantizer(&self) -> &ScalarQuantizer {
        &self.quantizer
    }

    fn copy_legacy(&self) -> Result<Box<dyn LegacyQuantizedByteVectorValues>> {
        Ok(Box::new(self.clone()))
    }
}

/// The supplier scores through `ScalarQuantizedVectorSimilarity`, not through
/// the raw byte comparison the unquantized scorer would perform
/// (`ScalarQuantizedVectorScorer.java:151-170`).
#[test]
fn quantized_supplier_scores_through_the_quantized_representation() {
    let values = MemoryQuantizedVectorValues::new();
    let quantizer = values.quantizer;
    let similarity_function = VectorSimilarityFunction::DOT_PRODUCT;

    let supplier = ScalarQuantizedRandomVectorScorerSupplier::new(
        similarity_function,
        &quantizer,
        Box::new(values.clone()),
    );
    let mut scorer = supplier
        .scorer()
        .expect("a scorer over the quantized field");
    scorer
        .set_scoring_ordinal(0)
        .expect("ordinal 0 is in range");
    let scored = scorer.score(1).expect("ordinal 1 is in range");

    let expected = <dyn ScalarQuantizedVectorSimilarity>::from_vector_similarity(
        similarity_function,
        quantizer.get_constant_multiplier(),
        quantizer.get_bits(),
    )
    .score(
        &values.vectors[0],
        values.offsets[0],
        &values.vectors[1],
        values.offsets[1],
    )
    .expect("the quantized similarity scores");
    assert_eq!(
        scored, expected,
        "the score must come from the quantized representation"
    );

    let unquantized = similarity_function
        .compare_bytes(&values.vectors[0], &values.vectors[1])
        .expect("the raw byte comparison scores");
    assert_ne!(
        scored, unquantized,
        "a quantized score that equals the raw byte score proves nothing; \
         pick data where the two representations disagree"
    );
}

/// `copy()` keeps the similarity and the function, and its scorers agree with
/// the original's (`ScalarQuantizedVectorScorer.java:172-176`).
#[test]
fn quantized_supplier_copy_scores_identically() {
    let values = MemoryQuantizedVectorValues::new();
    let quantizer = values.quantizer;
    let supplier = ScalarQuantizedRandomVectorScorerSupplier::new(
        VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
        &quantizer,
        Box::new(values),
    );

    let copy = supplier.copy().expect("the supplier copies");
    let mut original = supplier.scorer().expect("a scorer");
    let mut copied = copy.scorer().expect("a scorer from the copy");
    original
        .set_scoring_ordinal(3)
        .expect("ordinal 3 is in range");
    copied
        .set_scoring_ordinal(3)
        .expect("ordinal 3 is in range");

    for ord in 0..4 {
        assert_eq!(
            original.score(ord).expect("the original scores"),
            copied.score(ord).expect("the copy scores"),
            "ordinal {ord} scores the same through the copy"
        );
    }
    assert_eq!(original.max_ord(), 4);
}

/// `toString()` names the similarity function, as Java's does
/// (`ScalarQuantizedVectorScorer.java:178-183`).
#[test]
fn quantized_supplier_to_string_matches_lucene() {
    let values = MemoryQuantizedVectorValues::new();
    let quantizer = values.quantizer;
    let supplier = ScalarQuantizedRandomVectorScorerSupplier::new(
        VectorSimilarityFunction::COSINE,
        &quantizer,
        Box::new(values),
    );
    assert_eq!(
        supplier.to_string(),
        "ScalarQuantizedRandomVectorScorerSupplier(vectorSimilarityFunction=COSINE)"
    );
}

// -----------------------------------------------------------------------------
// 3. Prefetching on the legacy-quantized path
// -----------------------------------------------------------------------------

/// Legacy quantized vectors that write down every `prefetch` they are asked
/// for, in a log every copy shares.
#[derive(Clone)]
struct RecordingLegacyValues {
    inner: MemoryQuantizedVectorValues,
    prefetched: Arc<Mutex<Vec<Vec<i32>>>>,
}

impl RecordingLegacyValues {
    fn new() -> (Self, Arc<Mutex<Vec<Vec<i32>>>>) {
        let prefetched = Arc::new(Mutex::new(Vec::new()));
        let values = Self {
            inner: MemoryQuantizedVectorValues::new(),
            prefetched: Arc::clone(&prefetched),
        };
        (values, prefetched)
    }
}

impl QuantizedByteVectorValues for RecordingLegacyValues {
    fn vector_value(&mut self, ord: i32) -> Result<&[u8]> {
        self.inner.vector_value(ord)
    }

    fn get_score_correction_constant(&mut self, ord: i32) -> Result<f32> {
        self.inner.get_score_correction_constant(ord)
    }

    fn size(&self) -> i32 {
        self.inner.size()
    }

    fn dimension(&self) -> i32 {
        self.inner.dimension()
    }

    fn prefetch(&self, ords_to_prefetch: &[i32], num_ords: i32) -> Result<()> {
        self.prefetched
            .lock()
            .expect("the prefetch log is not poisoned")
            .push(ords_to_prefetch[..num_ords as usize].to_vec());
        Ok(())
    }

    fn copy(&self) -> Result<Box<dyn QuantizedByteVectorValues>> {
        Ok(Box::new(self.clone()))
    }
}

impl LegacyQuantizedByteVectorValues for RecordingLegacyValues {
    fn scalar_quantizer(&self) -> &ScalarQuantizer {
        self.inner.scalar_quantizer()
    }

    fn copy_legacy(&self) -> Result<Box<dyn LegacyQuantizedByteVectorValues>> {
        Ok(Box::new(self.clone()))
    }
}

fn legacy_scorer() -> ScalarQuantizedVectorScorer {
    ScalarQuantizedVectorScorer::new(Box::new(DefaultFlatVectorScorer::INSTANCE))
}

/// `PrefetchableFlatVectorScorer.getRandomVectorScorerSupplier` wraps whatever
/// the delegate returns, legacy quantized values included
/// (`PrefetchableFlatVectorScorer.java:53-59`), and the supplier's scorers
/// prefetch each batch (`PrefetchableFlatVectorScorer.java:230-233`). The
/// scores are the unwrapped delegate's.
#[test]
fn legacy_quantized_supplier_prefetches_the_batch_and_keeps_scores() {
    let (values, prefetched) = RecordingLegacyValues::new();
    let similarity = VectorSimilarityFunction::DOT_PRODUCT;
    let wrapped = PrefetchableFlatVectorScorer::new(Box::new(legacy_scorer()))
        .get_random_vector_scorer_supplier_legacy_quantized(similarity, Box::new(values.clone()))
        .expect("the wrapped supplier");
    let plain = legacy_scorer()
        .get_random_vector_scorer_supplier_legacy_quantized(similarity, Box::new(values))
        .expect("the plain supplier");

    let nodes = [3i32, 1, 2];
    let mut wrapped_scores = [0.0f32; 3];
    let mut plain_scores = [0.0f32; 3];
    let mut scorer = wrapped.scorer().expect("a wrapped scorer");
    scorer
        .set_scoring_ordinal(0)
        .expect("ordinal 0 is in range");
    scorer
        .bulk_score(&nodes, &mut wrapped_scores, 3)
        .expect("the wrapped scorer scores");
    let mut scorer = plain.scorer().expect("a plain scorer");
    scorer
        .set_scoring_ordinal(0)
        .expect("ordinal 0 is in range");
    scorer
        .bulk_score(&nodes, &mut plain_scores, 3)
        .expect("the plain scorer scores");

    assert_eq!(
        &*prefetched.lock().expect("the prefetch log is not poisoned"),
        &[vec![3i32, 1, 2]],
        "the wrapper must prefetch exactly the nodes it is about to score"
    );
    assert_eq!(wrapped_scores, plain_scores, "the wrapper changes no score");
}

/// `PrefetchableFlatVectorScorer.getRandomVectorScorer(…, float[])` wraps the
/// delegate's legacy-quantized query scorer too
/// (`PrefetchableFlatVectorScorer.java:61-67,159-162`).
#[test]
fn legacy_quantized_query_scorer_prefetches_the_batch_and_keeps_scores() {
    let (values, prefetched) = RecordingLegacyValues::new();
    let similarity = VectorSimilarityFunction::DOT_PRODUCT;
    let target = [0.5f32, -0.25, 0.75, 0.125];
    let mut wrapped = PrefetchableFlatVectorScorer::new(Box::new(legacy_scorer()))
        .get_random_vector_scorer_legacy_quantized(similarity, Box::new(values.clone()), &target)
        .expect("the wrapped scorer");
    let mut plain = legacy_scorer()
        .get_random_vector_scorer_legacy_quantized(similarity, Box::new(values), &target)
        .expect("the plain scorer");

    let nodes = [2i32, 0];
    let mut wrapped_scores = [0.0f32; 2];
    let mut plain_scores = [0.0f32; 2];
    let wrapped_max = wrapped
        .bulk_score(&nodes, &mut wrapped_scores, 2)
        .expect("the wrapped scorer scores");
    let plain_max = plain
        .bulk_score(&nodes, &mut plain_scores, 2)
        .expect("the plain scorer scores");

    assert_eq!(
        &*prefetched.lock().expect("the prefetch log is not poisoned"),
        &[vec![2i32, 0]],
        "the wrapper must prefetch exactly the nodes it is about to score"
    );
    assert_eq!(wrapped_scores, plain_scores, "the wrapper changes no score");
    assert_eq!(wrapped_max, plain_max);
}
