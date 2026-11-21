//! `search --exact-total`: the total-hits line with and without the flag.
//!
//! As in Lucene 10.5.0, `IndexSearcher.search(query, n, sort, doDocScores)`
//! counts hits exactly only up to `TOTAL_HITS_THRESHOLD` = 1 000
//! (`IndexSearcher.java:106`); past it the collector reports a lower bound
//! (`TopFieldCollector.java:79-85`). `--exact-total` replaces the total with
//! `IndexSearcher.count(query)`.

mod common;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use common::{run, temp_dir};

/// Documents in the generated index: more than the 1 000-hit threshold.
const DOCS: usize = 1_500;

/// One index of `DOCS` generated documents, shared by every test of this
/// binary (searches never write). Every document has `net=gen`; `mag` is
/// `i % 10`, so `mag=0..3` matches 600 documents.
fn index() -> &'static PathBuf {
    static INDEX: OnceLock<PathBuf> = OnceLock::new();
    INDEX.get_or_init(|| {
        let dir = temp_dir("exact-total");
        let mut csv = String::from("id,time,mag,net,place\n");
        for i in 0..DOCS {
            writeln!(
                csv,
                "gen{i:05},2024-01-01T00:{:02}:{:02}.000Z,{},gen,\"{} km N of Testville, Nowhere\"",
                i / 60 % 60,
                i % 60,
                i % 10,
                i % 97
            )
            .unwrap();
        }
        let file = dir.join("generated.csv");
        std::fs::write(&file, csv).unwrap();
        let idx = dir.join("idx");
        let idx_str = idx.to_str().unwrap();
        run(&["create", idx_str]);
        run(&["add", idx_str, "--file", file.to_str().unwrap()]);
        idx
    })
}

fn search(dir: &Path, args: &[&str]) -> String {
    let mut full = vec!["search", dir.to_str().unwrap()];
    full.extend_from_slice(args);
    run(&full)
}

fn first_line(output: &str) -> &str {
    output.lines().next().expect("total hits line")
}

#[test]
fn above_the_threshold_the_default_total_is_a_lower_bound() {
    for args in [
        &["--term", "net=gen"][..],
        &["--term", "net=gen", "--sort", "time:desc"][..],
        &[][..],
    ] {
        let line = search(index(), args).lines().next().unwrap().to_owned();
        let count: usize = line
            .strip_prefix("# total hits: ")
            .and_then(|rest| rest.strip_suffix(" (lower bound)"))
            .unwrap_or_else(|| panic!("{args:?}: expected a lower bound, got {line:?}"))
            .parse()
            .unwrap();
        assert!(
            (1_000..=DOCS).contains(&count),
            "{args:?}: lower bound {count}"
        );
    }
}

#[test]
fn exact_total_counts_every_hit_above_the_threshold() {
    for args in [
        &["--term", "net=gen", "--exact-total"][..],
        &["--exact-total", "--term", "net=gen", "--sort", "time:desc"][..],
        &["--exact-total"][..],
    ] {
        let output = search(index(), args);
        assert_eq!(
            first_line(&output),
            format!("# total hits: {DOCS}"),
            "{args:?}"
        );
    }
}

#[test]
fn exact_total_changes_only_the_total_line() {
    let args = ["--term", "net=gen", "--sort", "time:desc", "--limit", "20"];
    let default = search(index(), &args);
    let mut with_flag_args = args.to_vec();
    with_flag_args.push("--exact-total");
    let exact = search(index(), &with_flag_args);
    assert_ne!(first_line(&default), first_line(&exact));
    assert_eq!(
        default.lines().skip(1).collect::<Vec<_>>(),
        exact.lines().skip(1).collect::<Vec<_>>()
    );
}

#[test]
fn at_or_below_the_threshold_both_ways_are_identical() {
    for args in [
        &["--range", "mag=0..3"][..],
        &[
            "--range", "mag=0..3", "--sort", "time:asc", "--limit", "1000",
        ][..],
        &["--term", "net=gen", "--not", "--range", "mag=1..9"][..],
    ] {
        let default = search(index(), args);
        let mut with_flag = args.to_vec();
        with_flag.push("--exact-total");
        let exact = search(index(), &with_flag);
        assert_eq!(default, exact, "{args:?}");
        let expected = if args[0] == "--range" { 600 } else { 150 };
        assert_eq!(
            first_line(&default),
            format!("# total hits: {expected}"),
            "{args:?}"
        );
    }
}

#[test]
fn exact_total_after_a_criterion_prefix_is_a_usage_error() {
    let error = common::run_err(&[
        "search",
        index().to_str().unwrap(),
        "--not",
        "--exact-total",
    ]);
    assert!(
        matches!(error, rucene_cli_example::CliError::Usage(_)),
        "{error:?}"
    );
}
