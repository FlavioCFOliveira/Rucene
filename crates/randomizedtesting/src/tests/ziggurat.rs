//! `RandomGenerator.nextExponential()` and `nextGaussian(mean, stddev)`
//! (JDK 25) on `java.util.Random`, `Xoroshiro128PlusRandom` and
//! `AssertingRandom`: 20 seeds × 10 000 draws per method and parameter
//! (`ziggurat_draws.tsv`) and the argument errors (`ziggurat_errors.tsv`),
//! both written by the oracle's `ZigguratOracle`.

use std::collections::BTreeMap;

use super::random_draws::generator;
use super::{assert_same_lines, catch, java_error, rows, seeds, Rec};
use crate::jdk::random::Random;

const DRAWS: usize = 10_000;
const INF: f64 = f64::INFINITY;
const NAN: f64 = f64::NAN;

/// The oracle's `gaussianParams()`, pair for pair.
fn gaussian_params() -> Vec<(f64, f64)> {
    vec![
        (0.0, 1.0),
        (5.0, 2.0),
        (-3.0, 0.5),
        (0.0, 0.0),
        (7.0, 0.0),
        (1.0, -0.0),
        (1e300, 1e10),
        (0.0, f64::MAX),
        (0.0, f64::from_bits(1)),
        (0.0, INF),
        (INF, 1.0),
        (-INF, 1.0),
        (INF, 0.0),
        (-INF, INF),
        (NAN, 1.0),
        (0.0, NAN),
        (NAN, NAN),
    ]
}

/// `Long.toHexString(Double.doubleToLongBits(v))` (canonical NaN).
fn hex(v: f64) -> String {
    let bits = if v.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        v.to_bits()
    };
    format!("{bits:x}")
}

fn record(kind: &str, seed: i64, draw: impl Fn(&dyn Random, &mut Rec)) -> String {
    let r = generator(kind, seed);
    let mut rec = Rec::default();
    for _ in 0..DRAWS {
        draw(&*r, &mut rec);
    }
    rec.line()
}

fn check_generator(kind: &str) {
    let expected: BTreeMap<String, String> = rows("ziggurat_draws.tsv")
        .into_iter()
        .filter(|r| r[0] == kind)
        .map(|r| (format!("{}\t{}", r[1], r[2]), format!("{}\t{}", r[3], r[4])))
        .collect();
    let mut actual = BTreeMap::new();
    for seed in seeds() {
        let k = format!("{seed:x}");
        actual.insert(
            format!("{k}\tnextExponential()"),
            record(kind, seed, |r, rec| rec.d(r.next_exponential())),
        );
        actual.insert(
            format!("{k}\tnextGaussian()"),
            record(kind, seed, |r, rec| rec.d(r.next_gaussian())),
        );
        for (mean, stddev) in gaussian_params() {
            actual.insert(
                format!("{k}\tnextGaussian({},{})", hex(mean), hex(stddev)),
                record(kind, seed, |r, rec| {
                    rec.d(r.next_gaussian_mean_stddev(mean, stddev))
                }),
            );
        }
        actual.insert(
            format!("{k}\tinterleaved"),
            record(kind, seed, |r, rec| {
                rec.d(r.next_gaussian());
                rec.d(r.next_exponential());
                rec.d(r.next_gaussian_mean_stddev(2.0, 3.0));
                rec.l(r.next_long());
                rec.d(r.next_gaussian());
            }),
        );
    }
    assert_eq!(expected.len(), 20 * 20, "{kind}: oracle rows");
    assert_same_lines(&format!("{kind} ziggurat draws"), &expected, &actual);
}

#[test]
fn jdk_random_ziggurat_draws_equal_jdk25() {
    check_generator("jdk");
}

#[test]
fn xoroshiro_ziggurat_draws_equal_jdk25() {
    check_generator("xoro");
}

#[test]
fn asserting_random_ziggurat_draws_equal_jdk25() {
    check_generator("asserting");
}

#[test]
fn ziggurat_errors_equal_jdk25() {
    let expected: BTreeMap<String, String> = rows("ziggurat_errors.tsv")
        .into_iter()
        .map(|r| (format!("{}\t{}", r[0], r[1]), format!("{}\t{}", r[2], r[3])))
        .collect();
    let calls: [(&str, f64, f64); 6] = [
        ("nextGaussian(0,-1)", 0.0, -1.0),
        ("nextGaussian(0,-inf)", 0.0, -INF),
        ("nextGaussian(0,-min)", 0.0, -f64::from_bits(1)),
        ("nextGaussian(NaN,-1)", NAN, -1.0),
        ("nextGaussian(0,-0)", 0.0, -0.0),
        ("nextGaussian(0,NaN)", 0.0, NAN),
    ];
    let mut actual = BTreeMap::new();
    for kind in ["jdk", "xoro", "asserting"] {
        let r = generator(kind, 7);
        for (label, mean, stddev) in calls {
            let result = match catch(|| r.next_gaussian_mean_stddev(mean, stddev)) {
                Ok(_) => "OK\t-".to_string(),
                Err(p) => java_error(&p),
            };
            actual.insert(format!("{kind}\t{label}"), result);
        }
        let a = generator(kind, 11);
        assert!(catch(|| a.next_gaussian_mean_stddev(0.0, -1.0)).is_err());
        let b = generator(kind, 11);
        actual.insert(
            format!("{kind}\tnoDrawOnError"),
            format!("{}\t-", a.next_long() == b.next_long()),
        );
    }
    assert_same_lines("ziggurat errors", &expected, &actual);
}
