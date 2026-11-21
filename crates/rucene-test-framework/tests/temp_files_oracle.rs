//! `LuceneTestCase.createTempDir` / `createTempFile` and the filesystem chain
//! `TestRuleTemporaryFilesCleanup.before()` installs, against Lucene 10.5.0
//! under the same seeds and annotations (`tests/fixtures/temp_files.tsv`,
//! written by `TempFilesOracle.java`): the chain and its random parameters,
//! the paths, and the removal of the suite's folder after a successful suite
//! (a failed suite leaves it).

mod oracle_support;
mod temp_files_support;

use std::collections::BTreeMap;

use oracle_support::{assert_same, rows};
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{JUnitCore, Request};
use temp_files_support::{left, suite, ROWS};

#[test]
fn temp_files_rule_matches_lucene() {
    let mut expected = BTreeMap::new();
    let mut runs: Vec<(String, String)> = Vec::new();
    for r in rows("temp_files.tsv") {
        if r[0] == "leave" || r[0] == "leave-row" {
            continue;
        }
        let key = format!("{}\t{}", r[0], r[1]);
        expected.insert(key, r[2..].join("\t"));
        runs.push((r[0].clone(), r[1].clone()));
    }
    let mut actual = BTreeMap::new();
    for (simple, seed) in runs {
        ROWS.lock().unwrap().clear();
        system::set_property("tests.seed", &seed);
        let class = suite(&simple);
        let quiet = randomizedtesting::jdk::exception::QuietPanics::new();
        let result = JUnitCore::new().run(&Request::a_class(&class));
        drop(quiet);
        let prefix = format!(
            "{}_{seed}-",
            class.name().replacen("org.apache.lucene.", "lucene.", 1)
        );
        let left_now = left(&prefix);
        for row in ROWS.lock().unwrap().iter() {
            actual.insert(
                format!("{simple}\t{seed}"),
                format!(
                    "{row}\t{}\t[{}]",
                    result.failure_count(),
                    left_now.join(", ")
                ),
            );
        }
        for l in left_now {
            let _ = std::fs::remove_dir_all(std::env::temp_dir().join(l));
        }
    }
    system::clear_property("tests.seed");
    assert_same("temp files", &expected, &actual);
}
