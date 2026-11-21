//! Shared helpers of the integration tests.
//!
//! The expected results of every test are computed here from the fixture CSV
//! with plain Rust (string splitting, `str::parse`, `Vec::sort_by`), never
//! from the crate's own parser, schema or output, so a defect in the crate
//! cannot make its own oracle agree with it.

#![allow(dead_code)]

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::Arc;

use rucene::document::Document;
use rucene::index::{directory_reader, IndexReader, Term};
use rucene::search::{IndexSearcher, Query, TermQuery};
use rucene::store::{Directory, FSDirectory};
use rucene::util::BytesRef;

use rucene_cli_example::cli;

/// The fixture file committed with the crate.
pub fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join("usgs-2024-01-01-to-2024-01-04-m2.5.csv")
}

/// One fixture row: its 0-based position (= its Lucene document number in an
/// index built by adding the whole fixture at once) and its raw columns.
#[derive(Clone, Debug)]
pub struct Row {
    pub index: usize,
    pub columns: HashMap<String, String>,
}

impl Row {
    /// The raw value of `column` (empty when the CSV value is empty).
    pub fn raw(&self, column: &str) -> &str {
        self.columns
            .get(column)
            .map(String::as_str)
            .unwrap_or_else(|| panic!("no column {column}"))
    }

    pub fn id(&self) -> &str {
        self.raw("id")
    }

    /// The value parsed as `f64`, `None` when empty.
    pub fn f64(&self, column: &str) -> Option<f64> {
        let raw = self.raw(column);
        (!raw.is_empty()).then(|| raw.parse().expect("fixture number"))
    }

    /// The value parsed as `i64`, `None` when empty.
    pub fn i64(&self, column: &str) -> Option<i64> {
        let raw = self.raw(column);
        (!raw.is_empty()).then(|| raw.parse().expect("fixture integer"))
    }

    /// The lower-cased words of `place`, split on whitespace, commas and
    /// hyphens (how `StandardAnalyzer` splits the fixture's place names for
    /// the words the tests query).
    pub fn place_words(&self) -> Vec<String> {
        self.raw("place")
            .split(|c: char| c.is_whitespace() || c == ',' || c == '-')
            .filter(|word| !word.is_empty())
            .map(str::to_lowercase)
            .collect()
    }
}

/// Splits one CSV line whose only quoted fields contain no line break (true
/// of the whole fixture, which `fixture_has_no_multiline_fields` checks).
fn split_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            ('"', _) => quoted = !quoted,
            (',', false) => fields.push(std::mem::take(&mut field)),
            (other, _) => field.push(other),
        }
    }
    fields.push(field);
    fields
}

/// The fixture's data lines, as rows.
pub fn fixture_rows() -> Vec<Row> {
    let text = std::fs::read_to_string(fixture_path()).expect("fixture");
    let mut lines = text.lines();
    let header = split_line(lines.next().expect("header"));
    lines
        .enumerate()
        .map(|(index, line)| {
            let fields = split_line(line);
            assert_eq!(fields.len(), header.len(), "fixture line {}", index + 2);
            Row {
                index,
                columns: header.iter().cloned().zip(fields).collect(),
            }
        })
        .collect()
}

/// A fresh, empty, unique directory under the test temporary directory.
pub fn temp_dir(name: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rucene-cli-{name}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, AtomicOrdering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Runs a CLI command line through the library entry point and returns its
/// standard output, panicking on failure.
pub fn run(args: &[&str]) -> String {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    let mut out = Vec::new();
    if let Err(error) = cli::run(&args, &mut out) {
        panic!("{args:?} failed: {error}");
    }
    String::from_utf8(out).expect("UTF-8 output")
}

/// Runs a CLI command line and returns its error message, panicking when it
/// succeeds.
pub fn run_err(args: &[&str]) -> rucene_cli_example::CliError {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    let mut out = Vec::new();
    match cli::run(&args, &mut out) {
        Ok(()) => panic!("{args:?} unexpectedly succeeded"),
        Err(error) => error,
    }
}

/// Creates an index at a fresh directory and adds the whole fixture.
pub fn fixture_index(name: &str) -> PathBuf {
    let dir = temp_dir(name);
    let dir_str = dir.to_str().expect("UTF-8 path");
    run(&["create", dir_str]);
    run(&[
        "add",
        dir_str,
        "--file",
        fixture_path().to_str().expect("UTF-8 path"),
    ]);
    dir
}

/// Opens a fresh reader on the index at `dir` (the reopen that verifies a
/// write).
pub fn open_reader(dir: &Path) -> Arc<dyn IndexReader> {
    let directory: Arc<dyn Directory> = Arc::new(FSDirectory::open(dir).expect("directory"));
    directory_reader::open(directory).expect("reader") as Arc<dyn IndexReader>
}

/// The stored documents whose `id` term is `id`, read with plain `rucene`
/// calls (`TermQuery` + `StoredFields.document`).
pub fn stored_by_id(dir: &Path, id: &str) -> Vec<Document> {
    let searcher = IndexSearcher::new(open_reader(dir)).expect("searcher");
    let query: Arc<dyn Query> = Arc::new(TermQuery::new(Term::new(
        "id",
        BytesRef::new(id.as_bytes().to_vec()),
    )));
    let top = searcher.search_after(None, query, 100).expect("search");
    let stored = searcher.stored_fields().expect("stored fields");
    top.score_docs
        .iter()
        .map(|hit| stored.document(hit.doc).expect("document"))
        .collect()
}

/// A parsed `search` TSV output: total hits, header, and rows.
#[derive(Debug)]
pub struct Tsv {
    pub total_hits: i64,
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Tsv {
    pub fn parse(output: &str) -> Self {
        let mut lines = output.lines();
        let total_hits = lines
            .next()
            .and_then(|line| line.strip_prefix("# total hits: "))
            .expect("total hits line")
            .parse()
            .expect("total hits number");
        let header = lines
            .next()
            .expect("header")
            .split('\t')
            .map(str::to_owned)
            .collect();
        let rows = lines
            .map(|line| line.split('\t').map(str::to_owned).collect())
            .collect();
        Self {
            total_hits,
            header,
            rows,
        }
    }

    /// The values of column `name`, in hit order.
    pub fn column(&self, name: &str) -> Vec<String> {
        let position = self
            .header
            .iter()
            .position(|column| column == name)
            .unwrap_or_else(|| panic!("no column {name} in {:?}", self.header));
        self.rows.iter().map(|row| row[position].clone()).collect()
    }
}

/// Runs `search <dir> args... --fields id --limit 1000` and returns the hit
/// ids in order and the total hit count.
pub fn search_ids(dir: &Path, args: &[&str]) -> (Vec<String>, i64) {
    let mut full = vec!["search", dir.to_str().expect("UTF-8 path")];
    full.extend_from_slice(args);
    full.extend_from_slice(&["--fields", "id", "--limit", "1000"]);
    let tsv = Tsv::parse(&run(&full));
    (tsv.column("id"), tsv.total_hits)
}

/// The ids of `rows`, in order.
pub fn ids(rows: &[&Row]) -> Vec<String> {
    rows.iter().map(|row| row.id().to_owned()).collect()
}

/// Sorts ids as an unordered set, for comparing hit sets.
pub fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

/// Compares two optional keys so that `None` (missing) sorts after every
/// value in both directions, mirroring the example's missing-last sort.
pub fn missing_last<T: PartialOrd>(a: Option<T>, b: Option<T>, descending: bool) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(a), Some(b)) => {
            let order = a.partial_cmp(&b).expect("comparable");
            if descending {
                order.reverse()
            } else {
                order
            }
        }
    }
}

/// Haversine distance in metres on Lucene's mean earth radius
/// (`GeoUtils.EARTH_MEAN_RADIUS_METERS` = 6 371 008.7714).
pub fn haversine_meters(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const RADIUS: f64 = 6_371_008.771_4;
    let (phi1, phi2) = (lat1.to_radians(), lat2.to_radians());
    let d_phi = (lat2 - lat1).to_radians();
    let d_lambda = (lon2 - lon1).to_radians();
    let h = (d_phi / 2.0).sin().powi(2) + phi1.cos() * phi2.cos() * (d_lambda / 2.0).sin().powi(2);
    2.0 * RADIUS * h.sqrt().asin()
}
