//! `RandomizedTest.systemPropertyAs*` over the JDK parsers, `String.hashCode`,
//! `MurmurHash3` (`java_lang.tsv`), and `SeedUtils`, `Randomness` and
//! `MixWithSuiteName` (`seed_utils.tsv`).

use std::sync::Arc;

use super::{catch, from_units, java_error, rows, Rec};
use crate::jdk::lang::string_hash_code;
use crate::jdk::random::Random;
use crate::jdk::system;
use crate::murmur_hash3::MurmurHash3;
use crate::random_supplier::default_supplier;
use crate::seed_decorator::SeedDecorator;
use crate::{MixWithSuiteName, RandomizedTest, Randomness, SeedUtils};

const PROP: &str = "rucene.oracle.prop";

fn outcome(kind: &str) -> String {
    let r = catch(|| match kind {
        "double" => format!(
            "{:x}",
            RandomizedTest::system_property_as_double(PROP, -2.5).to_bits()
        ),
        "float" => format!(
            "{:x}",
            RandomizedTest::system_property_as_float(PROP, -2.5).to_bits()
        ),
        "int" => RandomizedTest::system_property_as_int(PROP, -3).to_string(),
        "long" => format!(
            "{:x}",
            RandomizedTest::system_property_as_long(PROP, -3).to_bits()
        ),
        "boolean" => RandomizedTest::system_property_as_boolean(PROP, true).to_string(),
        other => panic!("{other}"),
    });
    match r {
        Ok(v) => format!("OK\t{v}"),
        Err(p) => java_error(&p),
    }
}

#[test]
fn system_properties_parse_as_jdk25() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for r in rows("java_lang.tsv") {
        if !["double", "float", "int", "long", "boolean"].contains(&r[0].as_str()) {
            continue;
        }
        if r[1] == "unset" {
            system::clear_property(PROP);
        } else {
            system::set_property(PROP, &from_units(&r[1]));
        }
        let got = outcome(&r[0]);
        let want = format!("{}\t{}", r[2], r[3]);
        checked += 1;
        if got != want {
            failures.push(format!(
                "{} {:?}: java={want} rust={got}",
                r[0],
                from_units(&r[1])
            ));
        }
    }
    system::clear_property(PROP);
    assert!(checked > 500, "{checked}");
    assert!(
        failures.is_empty(),
        "{} of {checked} differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn hash_codes_equal_java() {
    for r in rows("java_lang.tsv") {
        match r[0].as_str() {
            "hashCode" => {
                let s = from_units(&r[1]);
                let h = string_hash_code(&s);
                assert_eq!(h.to_string(), r[2], "hashCode {s:?}");
                assert_eq!(MurmurHash3::hash_long(i64::from(h)).to_string(), r[3]);
            }
            "murmur64" => {
                let v: i64 = r[1].parse().unwrap();
                assert_eq!(MurmurHash3::hash_long(v).to_string(), r[2]);
            }
            "murmur32" => {
                let v: i32 = r[1].parse().unwrap();
                assert_eq!(MurmurHash3::hash_int(v).to_string(), r[2]);
            }
            _ => {}
        }
    }
}

fn mixed_suite() -> MixWithSuiteName {
    let mut mix = MixWithSuiteName::new();
    mix.initialize("com.carrotsearch.randomizedtesting.RandomizedTestingOracle$MixedSuite");
    mix
}

#[test]
fn seed_utils_and_randomness_equal_randomizedtesting() {
    let mut checked = 0;
    for r in rows("seed_utils.tsv") {
        checked += 1;
        match r[0].as_str() {
            "formatSeed" => {
                assert_eq!(SeedUtils::format_seed(r[1].parse().unwrap()), r[2]);
            }
            "parseSeed" => {
                let input = from_units(&r[1]);
                let got = match catch(|| SeedUtils::parse_seed(&input)) {
                    Ok(v) => format!("OK\t{v}"),
                    Err(p) => java_error(&p),
                };
                assert_eq!(got, format!("{}\t{}", r[2], r[3]), "parseSeed {input:?}");
            }
            "parseSeedChain" => {
                let input = from_units(&r[1]);
                let got = match catch(|| SeedUtils::parse_seed_chain(&input)) {
                    Ok(v) => format!(
                        "OK\t[{}]",
                        v.iter()
                            .map(|x| x.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    Err(p) => java_error(&p),
                };
                // A Java message may end in an empty chain: the row then has
                // no trailing field.
                let want = format!("{}\t{}", r[2], r.get(3).map(String::as_str).unwrap_or(""));
                assert_eq!(got, want, "parseSeedChain {input:?}");
            }
            "formatSeedChain" => {
                let s: Vec<i64> = r[1].split(',').map(|x| x.parse().unwrap()).collect();
                let rs: Vec<Randomness> = s
                    .iter()
                    .map(|&x| Randomness::new(x, default_supplier(), vec![]))
                    .collect();
                assert_eq!(
                    SeedUtils::format_seed_chain(&[&rs[0], &rs[1], &rs[2]]),
                    r[2]
                );
                assert_eq!(SeedUtils::format_seed_chain(&[&rs[0]]), r[3]);
                assert_eq!(SeedUtils::format_seed_chain(&[]), r[4]);
                assert_eq!(rs[0].to_string(), r[5]);
            }
            "decorated" => {
                let seed: i64 = r[1].parse().unwrap();
                let mix = mixed_suite();
                let decorators: Vec<Arc<dyn SeedDecorator>> = vec![Arc::new(mix), Arc::new(mix)];
                let rnd = Randomness::new(seed, default_supplier(), decorators);
                let mut rec = Rec::default();
                for _ in 0..100 {
                    rec.l(rnd.get_random().next_long());
                }
                assert_eq!(rnd.get_seed().to_string(), r[2]);
                assert_eq!(mix.decorate(seed).to_string(), r[3]);
                assert_eq!(
                    rec.line(),
                    format!("{}\t{}", r[4], r[5]),
                    "decorated {seed}"
                );
            }
            other => panic!("{other}"),
        }
    }
    assert!(checked > 70);
}

/// `String.equalsIgnoreCase` on the pairs probed on JDK 25 (the expected
/// booleans are JDK 25's): simple case mappings (U+0130, the ypogegrammeni
/// letters), surrogate pairs compared as code points, Georgian, title case.
#[test]
fn equals_ignore_case_equals_jdk25() {
    use crate::jdk::lang::equals_ignore_case;
    let cases: [(&str, &str, bool); 18] = [
        ("\u{130}", "i", true),
        ("\u{1F80}", "\u{1F88}", true),
        ("\u{10400}", "\u{10428}", true),
        ("a\u{10400}", "A\u{10428}", true),
        ("\u{131}", "I", true),
        ("\u{3C2}", "\u{3A3}", true),
        ("\u{10400}x", "\u{10428}X", true),
        ("\u{10D0}", "\u{1C90}", true),
        ("\u{1C5}", "\u{1C6}", true),
        ("\u{1FB3}", "\u{1FBC}", true),
        ("\u{1FB3}", "\u{391}\u{399}", false),
        ("\u{212A}", "K", true),
        ("\u{B5}", "\u{39C}", true),
        ("\u{1E9E}", "\u{DF}", true),
        ("\u{10400}", "\u{10401}", false),
        ("\u{1E900}", "\u{1E922}", true),
        ("nightly", "NIGHTLY", true),
        ("\u{130}x", "IX", true),
    ];
    for (a, b, expected) in cases {
        assert_eq!(equals_ignore_case(a, b), expected, "{a:?} vs {b:?}");
    }
}
