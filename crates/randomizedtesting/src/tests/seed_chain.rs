//! The seed chain under `RandomizedRunner` (`seed_chain.tsv`): master seed,
//! suite randomness (with and without `MixWithSuiteName`), method seeds and
//! the formatted chains, driven through `RandomizedContext` the way
//! `RandomizedRunner` drives it.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::{assert_same_lines, rows, Rec};
use crate::group_evaluator::GroupEvaluator;
use crate::jdk::lang::string_hash_code;
use crate::jdk::random::Random;
use crate::jdk::thread::{Thread, ThreadGroup};
use crate::murmur_hash3::MurmurHash3;
use crate::random_supplier::default_supplier;
use crate::seed_decorator::SeedDecorator;
use crate::{MixWithSuiteName, RandomizedContext, Randomness, SeedUtils};

const ORACLE: &str = "com.carrotsearch.randomizedtesting.RandomizedTestingOracle";

/// The oracle's `recordContext`.
fn record(master: &str, suite: &str, wherein: &str, log: &Mutex<Vec<(String, String)>>) {
    let ctx = RandomizedContext::current();
    let r = ctx.get_random();
    let mut rec = Rec::default();
    rec.l(r.next_long());
    rec.i(r.next_int_bound(100));
    rec.d(r.next_double());
    rec.d(r.next_gaussian());
    for _ in 0..96 {
        rec.l(r.next_long());
    }
    let chain = ctx.get_randomnesses();
    let chain_refs: Vec<&Randomness> = chain.iter().collect();
    let key = format!("{master}\t{suite}\t{wherein}");
    let value = format!(
        "{}\t{}\t{}\t{}\t{}\t{}",
        ctx.get_runner_seed_as_string(),
        ctx.get_randomness().expect("randomness"),
        SeedUtils::format_seed_chain(&chain_refs),
        ctx.get_target_class(),
        ctx.get_target_method().unwrap_or_else(|| "-".into()),
        rec.line()
    );
    log.lock().unwrap().push((key, value));
}

/// What `RandomizedRunner` does with `tests.seed`, a suite's decorators and
/// its test methods (`RandomizedRunner.java:328-367`, `:681-750`,
/// `:820-845`, and the method seed of `determineMethodSeeds`, `:1685-1686`).
fn run_suite(
    master: &str,
    suite: &str,
    mixed: bool,
    methods: &[&str],
    after_class: bool,
    log: Arc<Mutex<Vec<(String, String)>>>,
) {
    let suite_class = format!("{ORACLE}${suite}");
    let (master, suite) = (master.to_string(), suite.to_string());
    let methods: Vec<String> = methods.iter().map(|m| m.to_string()).collect();
    let group = ThreadGroup::new(ThreadGroup::main(), format!("TGRP-{suite}"));
    let body = move || {
        let mut decorators: Vec<Arc<dyn SeedDecorator>> = Vec::new();
        if mixed {
            let mut mix = MixWithSuiteName::new();
            mix.initialize(&suite_class);
            decorators.push(Arc::new(mix));
        }
        let chain = SeedUtils::parse_seed_chain(&master);
        let override_seed = chain.get(1).copied();
        let runner = Randomness::new(chain[0], default_supplier(), decorators);
        let me = Thread::current();
        let ctx = RandomizedContext::create(
            &me.thread_group().unwrap(),
            &suite_class,
            runner.clone(),
            GroupEvaluator::new(),
        );
        ctx.push(runner.clone_for(&me));
        record(&master, &suite, "beforeClass", &log);
        for m in &methods {
            let seed = override_seed.unwrap_or_else(|| {
                runner.get_seed() ^ MurmurHash3::hash_long(i64::from(string_hash_code(m)))
            });
            ctx.push(Randomness::new(seed, default_supplier(), vec![]));
            ctx.set_target_method(Some(m));
            record(&master, &suite, m, &log);
            ctx.set_target_method(None);
            ctx.pop_and_destroy();
        }
        if after_class {
            record(&master, &suite, "afterClass", &log);
        }
        ctx.pop_and_destroy();
        ctx.dispose();
    };
    let t = Thread::with_group(&group, format!("SUITE-{}", "x"), body);
    t.start();
    t.join().expect("suite thread");
}

#[test]
fn seed_chain_equals_randomized_runner() {
    let expected: BTreeMap<String, String> = rows("seed_chain.tsv")
        .into_iter()
        .map(|r| (r[..3].join("\t"), r[3..].join("\t")))
        .collect();
    let masters: Vec<String> = expected
        .keys()
        .map(|k| k.split('\t').next().unwrap().to_string())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    assert_eq!(masters.len(), 9);
    let log: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    for m in &masters {
        run_suite(
            m,
            "PlainSuite",
            false,
            &["testAlpha", "testBeta", "test"],
            true,
            log.clone(),
        );
        run_suite(
            m,
            "MixedSuite",
            true,
            &["testAlpha", "testBeta"],
            false,
            log.clone(),
        );
    }
    let actual: BTreeMap<String, String> = log.lock().unwrap().iter().cloned().collect();
    assert_same_lines("seed chain", &expected, &actual);
}
