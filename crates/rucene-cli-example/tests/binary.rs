//! The built `rucene-cli` binary end to end: exit codes, standard output and
//! standard error.

mod common;

use std::process::{Command, Output};

use common::{fixture_path, fixture_rows, ids, missing_last, temp_dir, Row, Tsv};

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rucene-cli"))
        .args(args)
        .output()
        .expect("spawn rucene-cli")
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "exit {:?}, stderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn create_add_update_delete_search_through_the_binary() {
    let dir = temp_dir("binary");
    let dir = dir.to_str().unwrap();
    let fixture = fixture_path();
    let rows = fixture_rows();

    stdout(&cli(&["create", dir]));
    let added = stdout(&cli(&["add", dir, "--file", fixture.to_str().unwrap()]));
    assert_eq!(
        added.trim_end(),
        format!("added {} document(s) in {dir}", rows.len())
    );

    // Puerto Rico network, newest first.
    let mut expected: Vec<&Row> = rows.iter().filter(|row| row.raw("net") == "pr").collect();
    expected.sort_by(|a, b| missing_last(Some(a.raw("time")), Some(b.raw("time")), true));
    let tsv = Tsv::parse(&stdout(&cli(&[
        "search",
        dir,
        "--term",
        "net=pr",
        "--sort",
        "time:desc",
        "--fields",
        "id,time",
        "--limit",
        "100",
    ])));
    assert_eq!(tsv.total_hits, expected.len() as i64);
    assert_eq!(tsv.column("id"), ids(&expected));

    let target = expected[0].id();
    stdout(&cli(&[
        "update",
        dir,
        "--set",
        &format!("id={target}"),
        "--set",
        "net=zz",
    ]));
    stdout(&cli(&["delete", dir, "--id", expected[1].id()]));
    let tsv = Tsv::parse(&stdout(&cli(&[
        "search",
        dir,
        "--terms",
        "net=pr,zz",
        "--limit",
        "100",
    ])));
    assert_eq!(tsv.total_hits, expected.len() as i64 - 1);
    let tsv = Tsv::parse(&stdout(&cli(&[
        "search", dir, "--term", "net=zz", "--fields", "id",
    ])));
    assert_eq!(tsv.column("id"), vec![target.to_owned()]);
}

#[test]
fn failures_exit_non_zero_with_a_message_on_stderr() {
    let usage = cli(&["frobnicate"]);
    assert_eq!(usage.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&usage.stderr).contains("unknown command \"frobnicate\""));
    assert!(usage.stdout.is_empty());

    let missing = temp_dir("binary-missing").join("nothing");
    let failure = cli(&["search", missing.to_str().unwrap()]);
    assert_eq!(failure.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&failure.stderr).contains("no index found at"));

    let help = cli(&["help"]);
    assert!(String::from_utf8_lossy(&stdout(&help).into_bytes()).contains("USAGE:"));
}
