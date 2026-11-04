//! `Double.toString` / `Float.toString` (`jdk::lang`) against JDK 25
//! (`to_string.tsv`, written by `ToStringOracle.java`): subnormals, the powers
//! of ten at the formatting boundaries (1e-3, 1e7), `MIN_NORMAL`,
//! `MAX_VALUE`, and 10 000 random bit patterns per type for 3 seeds.

use super::{rows, Rec};
use crate::jdk::lang::{double_to_string, float_to_string};
use crate::jdk::random::{JdkRandom, Random};

#[test]
fn to_string_matches_jdk25() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for r in rows("to_string.tsv") {
        let got = match r[0].as_str() {
            "double" => double_to_string(f64::from_bits(u64::from_str_radix(&r[1], 16).unwrap())),
            "float" => float_to_string(f32::from_bits(u32::from_str_radix(&r[1], 16).unwrap())),
            "double-random" | "float-random" => {
                let seed: i64 = r[1].parse().unwrap();
                let rnd = JdkRandom::new(seed);
                let mut rec = Rec::default();
                for _ in 0..10_000 {
                    if r[0] == "double-random" {
                        rec.s(&double_to_string(f64::from_bits(rnd.next_long() as u64)));
                    } else {
                        rec.s(&float_to_string(f32::from_bits(rnd.next_int() as u32)));
                    }
                }
                let want = format!("{}\t{}", r[2], r[3]);
                checked += 1;
                if rec.line() != want {
                    failures.push(format!(
                        "{} seed {seed}: java={want} rust={}",
                        r[0],
                        rec.line()
                    ));
                }
                continue;
            }
            other => panic!("{other}"),
        };
        checked += 1;
        if got != r[2] {
            failures.push(format!("{} {}: java={} rust={got}", r[0], r[1], r[2]));
        }
    }
    assert!(checked > 600, "{checked} rows");
    assert!(
        failures.is_empty(),
        "{} of {checked}:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
