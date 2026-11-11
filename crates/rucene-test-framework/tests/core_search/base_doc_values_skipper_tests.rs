//! Port of `lucene/core/src/test/org/apache/lucene/search/BaseDocValuesSkipperTests.java`
//! (`public abstract class BaseDocValuesSkipperTests extends LuceneTestCase`):
//! the fake doc values and skipper its subclasses share. The abstract class
//! declares no test method, so its subclasses' classes extend
//! `LuceneTestCase` directly.
//!
//! The `DocValuesRangeIterator` tests also share here the cast Java writes
//! as `(SkipBlockRangeIterator) iter.approximation()`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{DocValuesIterator, DocValuesSkipper, NumericDocValues};
use rucene::search::{
    DocIdSetIterator, DocValuesRangeIterator, SkipBlockRangeIterator, TwoPhaseIterator,
    NO_MORE_DOCS,
};
use rucene_test_framework::index::asserting_leaf_reader::AssertingNumericDocValues;

// Dense up to 1024+64=1088, sparse (even docs only) from 1088 onward.
// This means block [1024,1087] is dense YES while [1088,1151] is sparse YES_IF_PRESENT,
// both in the same level-1 block [1024,1151] — exercising the docIDRunEnd density check.
pub const DENSE_END: i32 = 1088;

/// Number of docs in the fake segment; docs at or beyond this bound do not exist.
pub const MAX_DOC: i32 = 2048;

/// Single definition of the fake field's data shape: the first doc that has
/// a value and is greater than or equal to `target`, or `NO_MORE_DOCS`. Pure
/// function, does not touch iteration state.
fn first_doc_with_value_on_or_after(target: i32) -> i32 {
    if target >= MAX_DOC {
        NO_MORE_DOCS
    } else if target < DENSE_END {
        target
    } else {
        // sparse: round up to even
        let d = target + (target & 1);
        if d >= MAX_DOC {
            NO_MORE_DOCS
        } else {
            d
        }
    }
}

/// The anonymous `NumericDocValues` of `docValues`.
struct FakeNumericDocValues {
    doc: Arc<AtomicI32>,
    query_min: i64,
    query_max: i64,
}

impl DocIdSetIterator for FakeNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.doc.load(Ordering::Relaxed)
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc_id() + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = first_doc_with_value_on_or_after(target);
        self.doc.store(doc, Ordering::Relaxed);
        Ok(doc)
    }

    fn cost(&self) -> i64 {
        42
    }
}

impl DocValuesIterator for FakeNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        // Per the advanceExact contract, position on target whether or not it has a value
        // (the asserting wrapper checks this).
        self.doc.store(target, Ordering::Relaxed);
        Ok(first_doc_with_value_on_or_after(target) == target)
    }
}

impl NumericDocValues for FakeNumericDocValues {
    fn long_value(&self) -> Result<i64> {
        let (query_min, query_max) = (self.query_min, self.query_max);
        let d = self.doc_id() % 1024;
        Ok(if d < 128 {
            (query_min + query_max) >> 1
        } else if d < 256 {
            query_max + 1
        } else if d < 512 {
            query_min - 1
        } else {
            match (d / 2) % 3 {
                0 => query_min - 1,
                1 => query_max + 1,
                2 => (query_min + query_max) >> 1,
                _ => randomizedtesting::jdk::exception::throw_without_message(
                    "java.lang.AssertionError",
                ),
            }
        })
    }
}

/// Fake numeric doc values: value pattern repeats every 1024 docs (d%1024 in
/// [0,128) matches, [128,256) is above queryMax, [256,512) is below
/// queryMin, [512,1024) is a mix). Docs 0 through `DENSE_END`-1 are dense
/// (all have values); docs from `DENSE_END` onward are sparse (only even
/// docs have a value).
///
/// The returned instance is wrapped in `AssertingNumericDocValues`, which
/// enforces the forward-only iteration contracts like real sparse
/// implementations rely on.
pub fn doc_values(query_min: i64, query_max: i64) -> Result<Box<dyn NumericDocValues>> {
    Ok(doc_values_shared(query_min, query_max)?.0)
}

/// [`doc_values`] with a handle on the current doc of the returned values,
/// which stands for the reference to them a Java test keeps after handing
/// them to an iterator that owns them in Rucene (the asserting wrapper's
/// `docID()` is the wrapped values' one).
pub fn doc_values_shared(
    query_min: i64,
    query_max: i64,
) -> Result<(Box<dyn NumericDocValues>, Arc<AtomicI32>)> {
    let doc = Arc::new(AtomicI32::new(-1));
    let values = FakeNumericDocValues {
        doc: Arc::clone(&doc),
        query_min,
        query_max,
    };
    Ok((
        Box::new(AssertingNumericDocValues::new(Box::new(values), MAX_DOC)?),
        doc,
    ))
}

/// The anonymous `DocValuesSkipper` of `docValuesSkipper`.
struct FakeSkipper {
    doc: i32,
    query_min: i64,
    query_max: i64,
    do_levels: bool,
}

impl DocValuesSkipper for FakeSkipper {
    fn advance(&mut self, target: i32) -> Result<()> {
        self.doc = target;
        Ok(())
    }

    fn num_levels(&self) -> i32 {
        if self.do_levels {
            3
        } else {
            1
        }
    }

    fn min_doc_id(&self, level: i32) -> i32 {
        let range_log = 9 - self.num_levels() + level;

        // the level is the log2 of the interval
        if self.doc < 0 {
            -1
        } else if self.doc >= MAX_DOC {
            NO_MORE_DOCS
        } else {
            let mask = (1 << range_log) - 1;
            // prior multiple of 2^level
            self.doc & !mask
        }
    }

    fn max_doc_id(&self, level: i32) -> i32 {
        let range_log = 9 - self.num_levels() + level;

        let min_doc_id = self.min_doc_id(level);
        match min_doc_id {
            -1 => -1,
            NO_MORE_DOCS => NO_MORE_DOCS,
            _ => min_doc_id + (1 << range_log) - 1,
        }
    }

    fn min_value(&self, level: i32) -> i64 {
        let (query_min, query_max) = (self.query_min, self.query_max);
        let d_start = self.min_doc_id(level) % 1024;
        let d_end = self.max_doc_id(level) % 1024;
        let mut min = i64::MAX;
        if d_start <= 127 && d_end >= 0 {
            min = min.min(query_min);
        }
        if d_start <= 255 && d_end >= 128 {
            min = min.min(query_max + 1);
        }
        if d_start <= 511 && d_end >= 256 {
            min = min.min(query_min - 1);
        }
        if d_end >= 512 {
            min = min.min(query_min - 1);
        }
        min
    }

    fn max_value(&self, level: i32) -> i64 {
        let (query_min, query_max) = (self.query_min, self.query_max);
        let d_start = self.min_doc_id(level) % 1024;
        let d_end = self.max_doc_id(level) % 1024;
        let mut max = i64::MIN;
        if d_start <= 127 && d_end >= 0 {
            max = max.max(query_max);
        }
        if d_start <= 255 && d_end >= 128 {
            max = max.max(query_max + 1);
        }
        if d_start <= 511 && d_end >= 256 {
            max = max.max(query_min - 1);
        }
        if d_end >= 512 {
            max = max.max(query_max + 1);
        }
        max
    }

    fn doc_count(&self, level: i32) -> i32 {
        let range_log = 9 - self.num_levels() + level;
        let block_size = 1 << range_log;
        let min_doc = self.min_doc_id(level);
        if !(0..MAX_DOC).contains(&min_doc) {
            return 0;
        }
        if min_doc >= DENSE_END {
            return block_size / 2;
        }
        // Block starts in dense region — count dense + sparse portions
        let dense_count = block_size.min(DENSE_END - min_doc);
        let sparse_count = (block_size - dense_count) / 2;
        dense_count + sparse_count
    }

    fn global_min_value(&self) -> i64 {
        i64::MIN
    }

    fn global_max_value(&self) -> i64 {
        i64::MAX
    }

    fn global_doc_count(&self) -> i32 {
        DENSE_END + (MAX_DOC - DENSE_END) / 2 // 1088 + 480 = 1568
    }

    fn max_value_count(&self) -> i32 {
        1
    }
}

/// Fake skipper over a NumericDocValues field built by an equivalent call to
/// [`doc_values`]. Dense up to doc `DENSE_END`, sparse (even docs only)
/// after that. This means block [1024,1087] is dense YES while [1088,1151]
/// is sparse YES_IF_PRESENT, both in the same level-1 block — exercising the
/// docIDRunEnd density check.
pub fn doc_values_skipper(
    query_min: i64,
    query_max: i64,
    do_levels: bool,
) -> Box<dyn DocValuesSkipper> {
    Box::new(FakeSkipper {
        doc: -1,
        query_min,
        query_max,
        do_levels,
    })
}

/// `(SkipBlockRangeIterator) iter.approximation()`, re-borrowed at each use
/// (Java holds the cast reference while it also calls `iter.matches()`).
pub fn approx(iter: &mut DocValuesRangeIterator) -> &mut SkipBlockRangeIterator {
    iter.approximation()
        .disi_as_any_mut()
        .and_then(|a| a.downcast_mut::<SkipBlockRangeIterator>())
        .unwrap_or_else(|| {
            randomizedtesting::jdk::exception::throw_without_message("java.lang.ClassCastException")
        })
}

/// The `collectMatches(DocValuesRangeIterator)` helper the subclasses
/// declare.
pub fn collect_matches(iter: &mut DocValuesRangeIterator) -> Result<Vec<i32>> {
    let mut matches = Vec::new();
    while iter.approximation().next_doc()? != NO_MORE_DOCS {
        if iter.matches()? {
            matches.push(iter.approximation().doc_id());
        }
    }
    Ok(matches)
}
