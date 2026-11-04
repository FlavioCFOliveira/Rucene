//! Oracle parity of the helpers of `org.apache.lucene.tests.util` (rmp #342)
//! against Apache Lucene 10.5.0.
//!
//! The Java oracle (`org.apache.lucene.tests.util.TestUtilOracle`, in the
//! workspace harness `crates/randomizedtesting/tests/fixtures/oracle/`)
//! records:
//!
//! * `testutil_english.tsv`: `English.intToEnglish` and
//!   `English.longToEnglish` over every int in [-1000, 1000], the `int` and
//!   `long` extremes, the powers of ten around every scale word, and 500 longs
//!   of `new Random(0)` at every magnitude.

use rucene_test_framework::util::English;

fn rows(name: &str) -> Vec<Vec<String>> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {path}: {e}"))
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| l.split('\t').map(str::to_string).collect())
        .collect()
}

/// `English` spells every value of the oracle's grid as Lucene does.
#[test]
fn english_is_lucenes() {
    let rows = rows("testutil_english.tsv");
    assert_eq!(rows.len(), 2003 + 57 + 2 + 500);
    for r in &rows {
        let rust = match r[0].as_str() {
            "int" => {
                let i: i32 = r[1].parse().unwrap();
                let mut into = String::new();
                English::int_to_english_into(i, &mut into);
                assert_eq!(into, English::int_to_english(i));
                into
            }
            _ => English::long_to_english(r[1].parse().unwrap()),
        };
        // The fixture's line holds the result's trailing space, if any.
        assert_eq!(rust, r[2], "{} {}", r[0], r[1]);
    }
}
