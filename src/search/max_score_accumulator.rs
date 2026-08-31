//! Cross-slice score sharing, ported from
//! `org.apache.lucene.search.MaxScoreAccumulator`.

#![deny(unsafe_code)]

use std::sync::atomic::{AtomicI64, Ordering};

/// Maintains the maximum score and its corresponding document ID concurrently.
///
/// Equivalent to `org.apache.lucene.search.MaxScoreAccumulator`, which is
/// package-private and `final` in Java; it is public here because Rust has no
/// package visibility and
/// [`TopScoreDocCollectorManager`](crate::search::TopScoreDocCollectorManager)
/// shares one across the collectors it creates.
///
/// The accumulated value is a `(doc, score)` pair encoded into a single `i64`
/// by the encoder that
/// [`TopScoreDocCollector`](crate::search::TopScoreDocCollector) uses, so that
/// the accumulation is a plain maximum.
///
/// **Divergence from Lucene 10.5.0.** Java uses a
/// `java.util.concurrent.atomic.LongAccumulator` seeded with `Long.MIN_VALUE`
/// and `Math::max`. Rust's standard library has no accumulator type, so this
/// port uses an [`AtomicI64`] updated with a compare-and-exchange maximum,
/// which has the same semantics.
#[derive(Debug)]
pub struct MaxScoreAccumulator {
    acc: AtomicI64,
    mod_interval: i64,
}

/// The default sampling interval: `2^10 - 1`, so that the remainder can be
/// checked with a bitwise `and`.
///
/// Equivalent to `MaxScoreAccumulator.DEFAULT_INTERVAL`.
const DEFAULT_INTERVAL: i64 = 0x3ff;

impl Default for MaxScoreAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl MaxScoreAccumulator {
    /// Creates an accumulator seeded with [`i64::MIN`] and the default
    /// sampling interval.
    ///
    /// Equivalent to `new MaxScoreAccumulator()`.
    pub fn new() -> Self {
        Self {
            // Scores are always positive, so the minimum is a safe seed.
            acc: AtomicI64::new(i64::MIN),
            mod_interval: DEFAULT_INTERVAL,
        }
    }

    /// Returns the sampling interval: a collector consults the accumulator once
    /// every `mod_interval + 1` hits.
    ///
    /// Equivalent to reading the package-private `modInterval` field, which
    /// Java documents as non-final and visible for tests.
    pub fn mod_interval(&self) -> i64 {
        self.mod_interval
    }

    /// Sets the sampling interval.
    ///
    /// Equivalent to writing the package-private `modInterval` field.
    pub fn set_mod_interval(&mut self, mod_interval: i64) {
        self.mod_interval = mod_interval;
    }

    /// Accumulates an encoded `(doc, score)` pair, keeping the maximum.
    ///
    /// Equivalent to `MaxScoreAccumulator.accumulate(long)`.
    pub fn accumulate(&self, code: i64) {
        let mut current = self.acc.load(Ordering::Relaxed);
        while code > current {
            match self
                .acc
                .compare_exchange_weak(current, code, Ordering::AcqRel, Ordering::Relaxed)
            {
                Ok(_) => return,
                Err(observed) => current = observed,
            }
        }
    }

    /// Returns the raw encoded maximum, or [`i64::MIN`] when nothing has been
    /// accumulated yet.
    ///
    /// Equivalent to `MaxScoreAccumulator.getRaw()`.
    pub fn get_raw(&self) -> i64 {
        self.acc.load(Ordering::Acquire)
    }
}
