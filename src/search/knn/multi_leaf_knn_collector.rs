//! Cross-segment kNN collection, ported from
//! `org.apache.lucene.search.knn.MultiLeafKnnCollector`.

#![deny(unsafe_code)]

use std::fmt;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::search::abstract_knn_collector::AbstractKnnCollector;
use crate::search::knn::{KnnCollector, KnnSearchStrategy, TopDocs};
use crate::util::hnsw::{BlockingFloatHeap, FloatHeap};

/// The greediness of a globally non-competitive search, in `(0, 1]`.
///
/// Equivalent to `MultiLeafKnnCollector.DEFAULT_GREEDINESS`.
const DEFAULT_GREEDINESS: f32 = 0.9;

/// The default interval, as a mask over the visited count.
///
/// Equivalent to `MultiLeafKnnCollector.DEFAULT_INTERVAL`.
const DEFAULT_INTERVAL: i32 = 0xff;

/// A kNN collector that exchanges the top collected results across segments
/// through a shared global queue.
///
/// Equivalent to the `final` class
/// `org.apache.lucene.search.knn.MultiLeafKnnCollector`, which extends
/// `KnnCollector.Decorator`; Rust has no implementation inheritance, so the
/// decorator's delegation is written out.
pub struct MultiLeafKnnCollector {
    /// The global queue of the highest similarities collected so far across all
    /// segments.
    global_similarity_queue: Arc<BlockingFloatHeap>,
    /// The local queue of the highest similarities if we are not competitive
    /// globally; its size is defined by the greediness.
    non_competitive_queue: FloatHeap,
    /// The queue of local similarities, periodically flushed into the global
    /// queue.
    updates_queue: FloatHeap,
    updates_scratch: Vec<f32>,
    /// The interval, as a number of visited vectors, at which the local and
    /// global queues are synchronised.
    interval: i32,
    k_results_collected: bool,
    cached_global_min_sim: f32,
    sub_collector: Box<dyn AbstractKnnCollector>,
}

impl fmt::Debug for MultiLeafKnnCollector {
    /// Renders the collector exactly as `MultiLeafKnnCollector.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MultiLeafKnnCollector[subCollector=..]")
    }
}

impl MultiLeafKnnCollector {
    /// Creates a collector with the default greediness and interval.
    ///
    /// Equivalent to
    /// `new MultiLeafKnnCollector(int, BlockingFloatHeap, AbstractKnnCollector)`.
    ///
    /// # Errors
    ///
    /// As [`with_greediness`](Self::with_greediness).
    pub fn new(
        k: i32,
        global_similarity_queue: Arc<BlockingFloatHeap>,
        sub_collector: Box<dyn AbstractKnnCollector>,
    ) -> Result<Self> {
        Self::with_greediness(
            k,
            DEFAULT_GREEDINESS,
            DEFAULT_INTERVAL,
            global_similarity_queue,
            sub_collector,
        )
    }

    /// Creates a collector with an explicit greediness and synchronisation
    /// interval.
    ///
    /// Equivalent to
    /// `new MultiLeafKnnCollector(int, float, int, BlockingFloatHeap, AbstractKnnCollector)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with Java's messages — when
    /// `greediness` is outside `[0, 1]` or `interval` is not positive.
    pub fn with_greediness(
        k: i32,
        greediness: f32,
        interval: i32,
        global_similarity_queue: Arc<BlockingFloatHeap>,
        sub_collector: Box<dyn AbstractKnnCollector>,
    ) -> Result<Self> {
        if !(0.0..=1.0).contains(&greediness) {
            return Err(LuceneError::IllegalArgument(
                "greediness must be in [0,1]".to_string(),
            ));
        }
        if interval <= 0 {
            return Err(LuceneError::IllegalArgument(
                "interval must be positive".to_string(),
            ));
        }
        let non_competitive_size = 1.max(java_round_f32((1.0 - greediness) * k as f32));
        Ok(Self {
            global_similarity_queue,
            non_competitive_queue: FloatHeap::new(non_competitive_size as usize),
            updates_queue: FloatHeap::new(k.max(0) as usize),
            updates_scratch: vec![0.0; k.max(0) as usize],
            interval,
            k_results_collected: false,
            cached_global_min_sim: f32::NEG_INFINITY,
            sub_collector,
        })
    }
}

impl KnnCollector for MultiLeafKnnCollector {
    fn early_terminated(&self) -> bool {
        self.sub_collector.early_terminated()
    }

    fn inc_visited_count(&mut self, count: i32) {
        self.sub_collector.inc_visited_count(count);
    }

    fn visited_count(&self) -> i64 {
        self.sub_collector.visited_count()
    }

    fn visit_limit(&self) -> i64 {
        self.sub_collector.visit_limit()
    }

    fn k(&self) -> i32 {
        self.sub_collector.k()
    }

    fn collect(&mut self, doc_id: i32, similarity: f32) -> bool {
        let local_sim_updated = self.sub_collector.collect(doc_id, similarity);
        let first_k_results_collected =
            !self.k_results_collected && self.sub_collector.num_collected() == self.k();
        if first_k_results_collected {
            self.k_results_collected = true;
        }
        self.updates_queue.offer(similarity);
        let mut global_sim_updated = self.non_competitive_queue.offer(similarity);

        if self.k_results_collected {
            // As we have collected k results, we can start doing periodic
            // updates with the global queue.
            if first_k_results_collected
                || (self.sub_collector.visited_count() & self.interval as i64) == 0
            {
                // BlockingFloatHeap::offer_all requires its input to be sorted
                // in ascending order, so we cannot pass the underlying
                // updates_queue array as-is: it is only partially ordered (see
                // GH#13462).
                let len = self.updates_queue.size();
                if len > 0 {
                    for slot in self.updates_scratch.iter_mut().take(len) {
                        *slot = self.updates_queue.poll();
                    }
                    debug_assert_eq!(self.updates_queue.size(), 0);
                    self.cached_global_min_sim = self
                        .global_similarity_queue
                        .offer_all(&self.updates_scratch, len);
                    global_sim_updated = true;
                }
            }
        }
        local_sim_updated || global_sim_updated
    }

    fn min_competitive_similarity(&self) -> f32 {
        if !self.k_results_collected {
            return f32::NEG_INFINITY;
        }
        java_max_f32(
            self.sub_collector.min_competitive_similarity(),
            java_min_f32(
                self.non_competitive_queue.peek(),
                self.cached_global_min_sim,
            ),
        )
    }

    fn top_docs(&mut self) -> TopDocs {
        self.sub_collector.top_docs()
    }

    fn get_search_strategy(&self) -> Option<&KnnSearchStrategy> {
        self.sub_collector.get_search_strategy()
    }
}

/// `java.lang.Math.round(float)`.
///
/// The JDK does not compute `floor(x + 0.5f)`, which would round twice; it
/// reads the significand directly. This is a transcription of that algorithm,
/// so that the non-competitive queue is sized exactly as in Java.
fn java_round_f32(value: f32) -> i32 {
    const SIGNIFICAND_WIDTH: i32 = 24;
    const EXP_BIAS: i32 = 127;
    const EXP_BIT_MASK: i32 = 0x7F80_0000u32 as i32;
    const SIGNIF_BIT_MASK: i32 = 0x007F_FFFF;

    let int_bits = value.to_bits() as i32;
    let biased_exp = (int_bits & EXP_BIT_MASK) >> (SIGNIFICAND_WIDTH - 1);
    let shift = (SIGNIFICAND_WIDTH - 2 + EXP_BIAS) - biased_exp;
    if (shift & -32) == 0 {
        // shift >= 0 && shift < 32
        let mut r = (int_bits & SIGNIF_BIT_MASK) | (SIGNIF_BIT_MASK + 1);
        if int_bits < 0 {
            r = -r;
        }
        ((r >> shift) + 1) >> 1
    } else {
        // The exponent is so small or large that the value is already an
        // integer, or is NaN; a narrowing primitive conversion applies.
        java_f32_to_i32(value)
    }
}

/// Java's narrowing primitive conversion from `float` to `int`.
fn java_f32_to_i32(value: f32) -> i32 {
    if value.is_nan() {
        0
    } else if value >= i32::MAX as f32 {
        i32::MAX
    } else if value <= i32::MIN as f32 {
        i32::MIN
    } else {
        value as i32
    }
}

/// `java.lang.Math.max(float, float)`, which propagates `NaN` where
/// [`f32::max`] would discard it.
fn java_max_f32(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a > b {
        a
    } else if a < b {
        b
    } else if a == 0.0 && b == 0.0 {
        // Java resolves +0.0 vs -0.0 through the sign bit.
        if a.is_sign_positive() {
            a
        } else {
            b
        }
    } else {
        a
    }
}

/// `java.lang.Math.min(float, float)`, which propagates `NaN` where
/// [`f32::min`] would discard it.
fn java_min_f32(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a < b {
        a
    } else if a > b {
        b
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_negative() {
            a
        } else {
            b
        }
    } else {
        a
    }
}
