//! `tests.leavetmpdir=true` (`LuceneTestCase.LEAVE_TEMPORARY`): the
//! temporary-files rule leaves every file, and reports each on `System.err`,
//! as the oracle's child JVM does (`temp_files.tsv`, rows `leave`). A binary
//! of its own: the constant is read once per process.

mod oracle_support;
mod temp_files_support;

use std::sync::{Arc, Mutex};

use oracle_support::rows;
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{JUnitCore, Request};
use temp_files_support::{left, suite, tmp, ROWS};

#[test]
fn leave_temporary_keeps_and_reports_every_file() {
    system::set_property("tests.leavetmpdir", "true");
    system::set_property("tests.seed", "DEADBEEF");
    assert!(*rucene_test_framework::util::lucene_test_case::LEAVE_TEMPORARY);
    let err: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let sink = Arc::clone(&err);
    let saved = system::set_err(Some(Arc::new(move |s: &str| {
        sink.lock().unwrap().push_str(s)
    })));
    let class = suite("TestAll");
    let result = JUnitCore::new().run(&Request::a_class(&class));
    system::set_err(saved);
    let prefix = "lucene.tests.util.TestTempFilesSuite$TestAll_DEADBEEF";
    let left_now = left(prefix);
    let tmp = tmp();
    let info: Vec<String> = err
        .lock()
        .unwrap()
        .lines()
        .filter(|l| l.starts_with("INFO: Will leave temporary file: "))
        .map(|l| l.replace(&tmp, "$T"))
        .collect();
    let actual = format!(
        "{}\t[{}]\t{}",
        result.failure_count(),
        left_now.join(", "),
        info.join(" | ")
    );
    let actual_row = ROWS.lock().unwrap().join("");
    for l in &left_now {
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join(l));
    }
    let fixture = rows("temp_files.tsv");
    let leave = fixture.iter().find(|r| r[0] == "leave").unwrap();
    let leave_row = fixture.iter().find(|r| r[0] == "leave-row").unwrap();
    assert_eq!(actual, leave[1..].join("\t"));
    assert_eq!(actual_row, leave_row[1..].join("\t"));
}
