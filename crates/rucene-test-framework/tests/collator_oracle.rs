//! Parity of `rucene_test_framework::jdk::collator` (the port of
//! `java.text.RuleBasedCollator`) and `LuceneTestCase.collate` with JDK 25,
//! on the grid `CollatorOracle` writes to `tests/fixtures/collator.tsv` (see
//! its documentation): the root collator's rules; every grid string compared
//! with every other by each collator (the root collator, the rule sets of
//! `RuleBasedCollator`'s javadoc and one set per rule feature) at every
//! strength and decomposition; their collation keys; the rules that do not
//! parse; and 4000 random pairs of `TestUtil.randomUnicodeString` and
//! `TestUtil.randomRealisticUnicodeString`, drawn here from the same seed.

mod oracle_support;

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};

use oracle_support::rows;
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::JdkRandom;
use rucene_test_framework::jdk::collator::{CollationKey, RuleBasedCollator};
use rucene_test_framework::util::lucene_test_case::LuceneTestCase;
use rucene_test_framework::util::TestUtil;

fn from_units(s: &str) -> Vec<u16> {
    if s == "-" {
        return Vec::new();
    }
    s.split('.')
        .map(|u| u16::from_str_radix(u, 16).expect("hex code unit"))
        .collect()
}

fn units(s: &[u16]) -> String {
    if s.is_empty() {
        return "-".to_string();
    }
    s.iter()
        .map(|u| format!("{u:04x}"))
        .collect::<Vec<_>>()
        .join(".")
}

fn string(s: &str) -> String {
    String::from_utf16(&from_units(s)).expect("the grid holds valid UTF-16")
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// `CollatorOracle.fnvInt`.
fn fnv_int(mut h: u64, v: i32) -> u64 {
    for k in 0..4 {
        h ^= u64::from((v >> (8 * k)) as u8);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// `CollatorOracle.fnv`.
fn fnv(mut h: u64, s: &[u16]) -> u64 {
    h = fnv_int(h, s.len() as i32);
    for &c in s {
        h ^= u64::from(c as u8);
        h = h.wrapping_mul(0x100_0000_01b3);
        h ^= u64::from((c >> 8) as u8);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// `CollatorOracle.keyString`: the key's code units, from `toByteArray()`.
fn key_units(k: &CollationKey) -> Vec<u16> {
    k.to_byte_array()
        .chunks(2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .collect()
}

fn sign(v: i32) -> char {
    match v.signum() {
        -1 => '<',
        1 => '>',
        _ => '=',
    }
}

/// `CollatorOracle.signs`: every decomposition, then every strength.
fn signs(base: &RuleBasedCollator, a: &str, b: &str) -> String {
    let mut s = String::new();
    for d in 0..=2 {
        for st in 0..=3 {
            let mut c = base.clone();
            c.set_decomposition(d);
            c.set_strength(st);
            s.push(sign(c.compare(a, b)));
        }
    }
    s
}

fn throwable(payload: Box<dyn std::any::Any + Send>) -> Throwable {
    Throwable::from_panic(payload)
}

#[test]
fn collator_matches_jdk25() {
    let rows = rows("collator.tsv");
    let mut grid: Vec<String> = Vec::new();
    let mut collators: HashMap<String, RuleBasedCollator> = HashMap::new();
    collators.insert("root".to_string(), RuleBasedCollator::root());
    let mut failures: Vec<String> = Vec::new();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut fail = |what: String| failures.push(what);
    let random = JdkRandom::new(20_261_106);
    for row in &rows {
        *counts.entry(row[0].as_str()).or_default() += 1;
        match row[0].as_str() {
            "root-rules" => {
                let root = RuleBasedCollator::root();
                let rules: Vec<u16> = root.get_rules().encode_utf16().collect();
                let got = format!(
                    "{}\t{}\t{}\t{}",
                    units(&rules),
                    root.hash_code(),
                    root.get_strength(),
                    root.get_decomposition()
                );
                if got != row[1..].join("\t") {
                    fail(format!("root-rules: {got}"));
                }
            }
            "str" => {
                assert_eq!(row[1].parse::<usize>().unwrap(), grid.len());
                grid.push(string(&row[2]));
            }
            "rules" => {
                let rules = string(&row[2]);
                let c = RuleBasedCollator::new(&rules)
                    .unwrap_or_else(|e| panic!("rules {}: {e}", row[1]));
                collators.insert(row[1].clone(), c);
            }
            "error" => {
                let rules = string(&row[2]);
                let got = match catch_unwind(AssertUnwindSafe(|| RuleBasedCollator::new(&rules))) {
                    Ok(Ok(_)) => "ok\t-\t-".to_string(),
                    Ok(Err(e)) => format!(
                        "java.text.ParseException\t{}\t{}",
                        e.get_message(),
                        e.get_error_offset()
                    ),
                    Err(p) => {
                        let t = throwable(p);
                        format!("{}\t{}\t-", t.class_name(), t.message().unwrap_or("<null>"))
                    }
                };
                if got != row[3..].join("\t") {
                    fail(format!(
                        "error {}: got {got:?}, want {:?}",
                        row[1],
                        row[3..].join("\t")
                    ));
                }
            }
            "cmp" => {
                let mut c = collators[&row[1]].clone();
                c.set_strength(row[2].parse().unwrap());
                c.set_decomposition(row[3].parse().unwrap());
                let i: usize = row[4].parse().unwrap();
                let got: String = grid.iter().map(|g| sign(c.compare(&grid[i], g))).collect();
                if got != row[5] {
                    let j = got
                        .chars()
                        .zip(row[5].chars())
                        .position(|(x, y)| x != y)
                        .unwrap_or(0);
                    fail(format!(
                        "cmp {} s={} d={} {:?} vs {:?}: got {}, want {}",
                        row[1],
                        row[2],
                        row[3],
                        grid[i],
                        grid[j],
                        got.chars().nth(j).unwrap_or(' '),
                        row[5].chars().nth(j).unwrap_or(' ')
                    ));
                }
            }
            "keys" => {
                let mut c = collators[&row[1]].clone();
                c.set_strength(row[2].parse().unwrap());
                c.set_decomposition(row[3].parse().unwrap());
                let h = grid.iter().fold(FNV_OFFSET, |h, g| {
                    fnv(h, &key_units(&c.get_collation_key(g)))
                });
                if format!("{h:016x}") != row[4] {
                    fail(format!("keys {} s={} d={}", row[1], row[2], row[3]));
                }
            }
            "key" => {
                let i: usize = row[1].parse().unwrap();
                let got = units(&key_units(&collators["root"].get_collation_key(&grid[i])));
                if got != row[2] {
                    fail(format!("key {:?}: got {got}, want {}", grid[i], row[2]));
                }
            }
            "random" => {
                let k: usize = row[1].parse().unwrap();
                let (a, b) = if k.is_multiple_of(2) {
                    let a = TestUtil::random_unicode_string(&random);
                    (a, TestUtil::random_unicode_string(&random))
                } else {
                    let a = TestUtil::random_realistic_unicode_string(&random);
                    (a, TestUtil::random_realistic_unicode_string(&random))
                };
                let (ua, ub): (Vec<u16>, Vec<u16>) =
                    (a.encode_utf16().collect(), b.encode_utf16().collect());
                let pair = fnv(fnv(FNV_OFFSET, &ua), &ub);
                assert_eq!(
                    format!("{pair:016x}"),
                    row[2],
                    "random {k}: the generators drew another pair"
                );
                let root = &collators["root"];
                let (ka, kb) = (root.get_collation_key(&a), root.get_collation_key(&b));
                let collate = match catch_unwind(AssertUnwindSafe(|| {
                    LuceneTestCase::collate(root, &a, &b)
                })) {
                    Ok(v) => v.signum().to_string(),
                    Err(p) => format!("assume:{}", throwable(p).message().unwrap_or("")),
                };
                let keys = fnv(fnv(FNV_OFFSET, &key_units(&ka)), &key_units(&kb));
                let got = format!(
                    "{}\t{}\t{}\t{keys:016x}",
                    signs(root, &a, &b),
                    sign(ka.compare_to(&kb)),
                    collate
                );
                if got != row[3..].join("\t") {
                    fail(format!(
                        "random {k} {a:?} vs {b:?}: got {got}, want {}",
                        row[3..].join("\t")
                    ));
                }
            }
            other => panic!("unknown row {other}"),
        }
    }
    // The grid is the oracle's: every kind of row was read.
    assert_eq!(counts.get("root-rules"), Some(&1));
    assert_eq!(counts.get("str"), Some(&grid.len()));
    assert_eq!(counts.get("rules"), Some(&10));
    assert_eq!(counts.get("error"), Some(&9));
    assert_eq!(counts.get("cmp"), Some(&(11 * 12 * grid.len())));
    assert_eq!(counts.get("keys"), Some(&(11 * 12)));
    assert_eq!(counts.get("key"), Some(&grid.len()));
    assert_eq!(counts.get("random"), Some(&4000));
    let shown: Vec<&String> = failures.iter().take(40).collect();
    assert!(
        failures.is_empty(),
        "{} rows differ from JDK 25:\n{:#?}",
        failures.len(),
        shown
    );
}
