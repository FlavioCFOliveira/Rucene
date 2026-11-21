//! Port of `lucene/core/src/test/org/apache/lucene/codecs/TestCompetitiveFreqNormAccumulator.java`.
//!
//! Java's `TreeSet<Impact>` ordered by `Comparator.comparingInt(i -> i.freq)`
//! is a [`FreqSet`]: a `BTreeMap` keyed by `freq`, whose `add` keeps the
//! element already there and whose `remove` removes by `freq`, as the
//! comparator decides equality.

use std::collections::BTreeMap;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::codecs::{CompetitiveImpactAccumulator, Impact};
use rucene::error::Result;

use crate::core_support::{assert_equals, class, test};

/// `TestCompetitiveFreqNormAccumulator`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.codecs.TestCompetitiveFreqNormAccumulator",
        vec![
            test("testBasics", test_basics),
            test("testExtremeNorms", test_extreme_norms),
            test("testCopy", test_copy),
            test("testOmitFreqs", test_omit_freqs),
            test("testOmitNorms", test_omit_norms),
        ],
    )
}

/// `new TreeSet<>(Comparator.comparingInt(i -> i.freq))`.
#[derive(Default)]
struct FreqSet(BTreeMap<i32, Impact>);

impl FreqSet {
    /// `add(e)`: no change when an element of the same `freq` is present.
    fn add(&mut self, impact: Impact) {
        self.0.entry(impact.freq).or_insert(impact);
    }

    /// `remove(o)`: the element of the same `freq`.
    fn remove(&mut self, impact: Impact) {
        self.0.remove(&impact.freq);
    }

    /// `clear()`.
    fn clear(&mut self) {
        self.0.clear();
    }

    /// `List.copyOf(set)`, in `freq` order.
    fn list(&self) -> Vec<Impact> {
        self.0.values().copied().collect()
    }
}

/// Port of `TestCompetitiveFreqNormAccumulator.testBasics`.
fn test_basics() -> Result<()> {
    let mut acc = CompetitiveImpactAccumulator::new();
    let mut expected = FreqSet::default();

    acc.add(3, 5);
    expected.add(Impact::new(3, 5));
    assert_equals(&expected.list(), &acc.get_competitive_freq_norm_pairs());

    acc.add(6, 11);
    expected.add(Impact::new(6, 11));
    assert_equals(&expected.list(), &acc.get_competitive_freq_norm_pairs());

    acc.add(10, 13);
    expected.add(Impact::new(10, 13));
    assert_equals(&expected.list(), &acc.get_competitive_freq_norm_pairs());

    acc.add(1, 2);
    expected.add(Impact::new(1, 2));
    assert_equals(&expected.list(), &acc.get_competitive_freq_norm_pairs());

    acc.add(7, 9);
    expected.remove(Impact::new(6, 11));
    expected.add(Impact::new(7, 9));
    assert_equals(&expected.list(), &acc.get_competitive_freq_norm_pairs());

    acc.add(8, 2);
    expected.clear();
    expected.add(Impact::new(10, 13));
    expected.add(Impact::new(8, 2));
    assert_equals(&expected.list(), &acc.get_competitive_freq_norm_pairs());
    Ok(())
}

/// Port of `TestCompetitiveFreqNormAccumulator.testExtremeNorms`.
fn test_extreme_norms() -> Result<()> {
    let mut acc = CompetitiveImpactAccumulator::new();
    let mut expected = FreqSet::default();

    acc.add(3, 5);
    expected.add(Impact::new(3, 5));
    assert_equals(&expected.list(), &acc.get_competitive_freq_norm_pairs());

    acc.add(10, 10000);
    expected.add(Impact::new(10, 10000));
    assert_equals(&expected.list(), &acc.get_competitive_freq_norm_pairs());

    acc.add(5, 200);
    expected.add(Impact::new(5, 200));
    assert_equals(&expected.list(), &acc.get_competitive_freq_norm_pairs());

    acc.add(20, -100);
    expected.add(Impact::new(20, -100));
    assert_equals(&expected.list(), &acc.get_competitive_freq_norm_pairs());

    acc.add(30, -3);
    expected.add(Impact::new(30, -3));
    assert_equals(&expected.list(), &acc.get_competitive_freq_norm_pairs());
    Ok(())
}

/// Port of `TestCompetitiveFreqNormAccumulator.testCopy`.
fn test_copy() -> Result<()> {
    let mut acc = CompetitiveImpactAccumulator::new();
    let mut copied_acc = CompetitiveImpactAccumulator::new();
    let mut merged_acc = CompetitiveImpactAccumulator::new();

    acc.add(3, 5);
    copied_acc.copy(&acc);
    assert_equals(
        &copied_acc.get_competitive_freq_norm_pairs(),
        &acc.get_competitive_freq_norm_pairs(),
    );

    merged_acc.add_all(&acc);
    assert_equals(
        &copied_acc.get_competitive_freq_norm_pairs(),
        &merged_acc.get_competitive_freq_norm_pairs(),
    );

    acc.add(10, 10000);
    copied_acc.copy(&acc);
    assert_equals(
        &copied_acc.get_competitive_freq_norm_pairs(),
        &acc.get_competitive_freq_norm_pairs(),
    );

    merged_acc.clear();
    merged_acc.add_all(&acc);
    assert_equals(
        &copied_acc.get_competitive_freq_norm_pairs(),
        &merged_acc.get_competitive_freq_norm_pairs(),
    );

    acc.add(5, 200);
    copied_acc.copy(&acc);
    assert_equals(
        &copied_acc.get_competitive_freq_norm_pairs(),
        &acc.get_competitive_freq_norm_pairs(),
    );

    merged_acc.clear();
    merged_acc.add_all(&acc);
    assert_equals(
        &copied_acc.get_competitive_freq_norm_pairs(),
        &merged_acc.get_competitive_freq_norm_pairs(),
    );

    acc.add(20, -100);
    copied_acc.copy(&acc);
    assert_equals(
        &copied_acc.get_competitive_freq_norm_pairs(),
        &acc.get_competitive_freq_norm_pairs(),
    );

    merged_acc.clear();
    merged_acc.add_all(&acc);
    assert_equals(
        &copied_acc.get_competitive_freq_norm_pairs(),
        &merged_acc.get_competitive_freq_norm_pairs(),
    );

    acc.add(30, -3);
    copied_acc.copy(&acc);
    assert_equals(
        &copied_acc.get_competitive_freq_norm_pairs(),
        &acc.get_competitive_freq_norm_pairs(),
    );

    merged_acc.clear();
    merged_acc.add_all(&acc);
    assert_equals(
        &copied_acc.get_competitive_freq_norm_pairs(),
        &merged_acc.get_competitive_freq_norm_pairs(),
    );
    Ok(())
}

/// Port of `TestCompetitiveFreqNormAccumulator.testOmitFreqs`.
fn test_omit_freqs() -> Result<()> {
    let mut acc = CompetitiveImpactAccumulator::new();

    acc.add(1, 5);
    acc.add(1, 7);
    acc.add(1, 4);

    assert_equals(
        &vec![Impact::new(1, 4)],
        &acc.get_competitive_freq_norm_pairs(),
    );
    Ok(())
}

/// Port of `TestCompetitiveFreqNormAccumulator.testOmitNorms`.
fn test_omit_norms() -> Result<()> {
    let mut acc = CompetitiveImpactAccumulator::new();

    acc.add(5, 1);
    acc.add(7, 1);
    acc.add(4, 1);

    assert_equals(
        &vec![Impact::new(7, 1)],
        &acc.get_competitive_freq_norm_pairs(),
    );
    Ok(())
}
