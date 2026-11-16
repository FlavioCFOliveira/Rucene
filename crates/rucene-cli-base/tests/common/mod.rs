//! Shared helpers of the integration tests.
//!
//! Expected results are computed here from the fixture records with plain
//! Rust (string comparison, a local accent-folding table, `str::parse`),
//! never from the crate's analyzer, schema, index or output, so a defect in
//! those cannot make its own oracle agree with it. Fixture lines are read
//! with the crate's JSON reader, which its own unit tests cover against
//! RFC 8259.
//!
//! Every test works in its own directory under `CARGO_TARGET_TMPDIR`, so the
//! tests are parallel-safe.

#![allow(dead_code)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rucene_cli_base::json::{self, JsonValue};
use rucene_cli_base::{cli, CliError};

/// A fresh, empty directory for one test.
pub fn temp_dir(label: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "rucene-cli-base-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs the command line through the library and returns its standard
/// output; panics with the error when it fails.
pub fn run(args: &[&str]) -> String {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    let mut out = Vec::new();
    if let Err(error) = cli::run(&args, &mut out) {
        panic!("{args:?} failed: {error}");
    }
    String::from_utf8(out).unwrap()
}

/// Runs the command line and returns its error; panics when it succeeds.
pub fn run_err(args: &[&str]) -> CliError {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    let mut out = Vec::new();
    match cli::run(&args, &mut out) {
        Ok(()) => panic!("{args:?} succeeded: {}", String::from_utf8_lossy(&out)),
        Err(error) => error,
    }
}

/// Runs the command line and returns its output and its error, if any (for
/// `get`, which prints what it found before reporting missing keys).
pub fn run_both(args: &[&str]) -> (String, Option<CliError>) {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    let mut out = Vec::new();
    let result = cli::run(&args, &mut out);
    (String::from_utf8(out).unwrap(), result.err())
}

/// The path text of a directory.
pub fn path_str(dir: &Path) -> &str {
    dir.to_str().unwrap()
}

/// The fixture file of an index.
pub fn fixture_path(entity: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(format!("{entity}.jsonl"))
}

/// One fixture record: its members, as parsed.
#[derive(Clone, Debug)]
pub struct Row {
    pub members: Vec<(String, JsonValue)>,
}

impl Row {
    /// The non-empty string forms of member `name` (one per array element;
    /// numbers as written; `null` and blank strings skipped).
    pub fn strings(&self, name: &str) -> Vec<String> {
        let Some((_, value)) = self.members.iter().find(|(member, _)| member == name) else {
            return Vec::new();
        };
        let scalar = |value: &JsonValue| match value {
            JsonValue::String(text) if !text.trim().is_empty() => Some(text.trim().to_owned()),
            JsonValue::Number(text) => Some(text.clone()),
            JsonValue::Bool(value) => Some(value.to_string()),
            _ => None,
        };
        match value {
            JsonValue::Array(items) => items.iter().filter_map(scalar).collect(),
            other => scalar(other).into_iter().collect(),
        }
    }

    /// The first string form of member `name`, or `""`.
    pub fn one(&self, name: &str) -> String {
        self.strings(name).into_iter().next().unwrap_or_default()
    }
}

/// Every record of an index's fixture, in file order.
pub fn fixture_rows(entity: &str) -> Vec<Row> {
    std::fs::read_to_string(fixture_path(entity))
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| match json::parse(line).unwrap() {
            JsonValue::Object(members) => Row { members },
            other => panic!("fixture line is not an object: {other:?}"),
        })
        .collect()
}

/// Lower-cases `text` and strips the Portuguese diacritics (a local table,
/// independent of the crate's analysis).
pub fn fold_pt(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            other => other,
        })
        .collect()
}

/// The lower-cased alphanumeric words of `text`.
pub fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// The keys (member `key`) of the rows for which any word of any of
/// `fields` is in `targets` (compared lower-cased, as written).
pub fn keys_with_word(
    rows: &[Row],
    key: &str,
    fields: &[&str],
    targets: &[&str],
) -> BTreeSet<String> {
    rows.iter()
        .filter(|row| {
            fields.iter().any(|field| {
                row.strings(field).iter().any(|text| {
                    words(text)
                        .iter()
                        .any(|word| targets.contains(&word.as_str()))
                })
            })
        })
        .map(|row| row.one(key))
        .collect()
}

/// A parsed `search` output.
#[derive(Debug)]
pub struct Tsv {
    pub total_hits: i64,
    pub exact: bool,
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Tsv {
    pub fn parse(output: &str) -> Self {
        let mut lines = output.lines();
        let total = lines
            .next()
            .and_then(|line| line.strip_prefix("# total hits: "))
            .expect("total line");
        let (number, exact) = match total.strip_suffix(" (lower bound)") {
            Some(number) => (number, false),
            None => (total, true),
        };
        let header: Vec<String> = lines
            .next()
            .expect("header line")
            .split('\t')
            .map(str::to_owned)
            .collect();
        let rows = lines
            .map(|line| line.split('\t').map(str::to_owned).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        for row in &rows {
            assert_eq!(row.len(), header.len(), "row {row:?} vs header {header:?}");
        }
        Self {
            total_hits: number.parse().unwrap(),
            exact,
            header,
            rows,
        }
    }

    pub fn column(&self, name: &str) -> Vec<String> {
        let index = self
            .header
            .iter()
            .position(|column| column == name)
            .unwrap_or_else(|| panic!("no column {name} in {:?}", self.header));
        self.rows.iter().map(|row| row[index].clone()).collect()
    }

    pub fn column_set(&self, name: &str) -> BTreeSet<String> {
        self.column(name).into_iter().collect()
    }
}

/// Creates `<root>/<entity>` and loads the entity's whole fixture with
/// `update`; returns the root.
pub fn loaded_root(label: &str, entity: &str) -> PathBuf {
    let root = temp_dir(label);
    run(&[entity, "create", path_str(&root)]);
    run(&[
        entity,
        "update",
        path_str(&root),
        "--file",
        fixture_path(entity).to_str().unwrap(),
    ]);
    root
}

/// Runs `search` with `--limit 1000` and the given extra arguments, and
/// returns the parsed output.
pub fn search(entity: &str, root: &Path, args: &[&str]) -> Tsv {
    let mut all = vec![entity, "search", path_str(root), "--limit", "1000"];
    all.extend_from_slice(args);
    Tsv::parse(&run(&all))
}

/// The set of a set of `&str`.
pub fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}
