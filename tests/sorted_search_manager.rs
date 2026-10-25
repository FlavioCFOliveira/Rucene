//! rmp #242: sorted searches through `IndexSearcher::search_with_manager`.
//!
//! `IndexSearcher.search(Query, CollectorManager)` may hand every collector to
//! an executor's worker, so a [`TopFieldCollector`](rucene::search::TopFieldCollector)
//! must be `Send`. It holds the hit queue's comparators, and those held the
//! segment's iterators, which are not; the call did not compile. Lucene keeps
//! the per-segment state in a separate `LeafFieldComparator` returned by
//! `FieldComparator.getLeafComparator` (`FieldComparator.java`,
//! `LeafFieldComparator.java`), which the port now does too.
//!
//! `SortedSearchFixture` from `tests/fixtures/java-codec-harness`
//! (`java -cp lucene-core-10.5.0.jar SortedSearchFixture.java <dir>`) writes a
//! 12-segment index and prints the top 10 documents of
//! `searcher.search(query, new TopFieldCollectorManager(sort, 10, null, 1000))`
//! for string, reversed string, missing-last string, long, int, double, float,
//! score, doc, binary-value and multi-field sorts — with their total hits when
//! the searcher has no executor, and without them when it has one, since the
//! slices then race to raise the minimum competitive score. Rucene reads
//! Lucene's index and must return the same documents, with and without an
//! executor that runs every slice on a thread of its own.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::index::{directory_reader, IndexReader, Term};
use rucene::search::{
    Executor, IndexSearcher, MatchAllDocsQuery, MissingValue, Query, Sort, SortField,
    SortFieldType, TermQuery, TopFieldCollectorManager, TotalHitsRelation,
};
use rucene::store::{Directory, NIOFSDirectory};
use rucene::util::BytesRef;

struct JavaRun {
    dir: PathBuf,
    lines: Vec<String>,
    _tmp: tempfile::TempDir,
}

fn lucene_jar() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME is set");
    let jar = Path::new(&home)
        .join(".m2/repository/org/apache/lucene/lucene-core/10.5.0/lucene-core-10.5.0.jar");
    assert!(
        jar.is_file(),
        "the Lucene 10.5.0 jar is required: {}",
        jar.display()
    );
    jar
}

fn java_run() -> &'static JavaRun {
    static RUN: OnceLock<JavaRun> = OnceLock::new();
    RUN.get_or_init(|| {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
             SortedSearchFixture.java",
        );
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("index");
        let output = Command::new("java")
            .arg("-cp")
            .arg(lucene_jar())
            .arg(&fixture)
            .arg(&dir)
            .output()
            .expect("a JDK is required to run the Lucene oracle");
        assert!(
            output.status.success(),
            "SortedSearchFixture failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let lines = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        JavaRun {
            dir,
            lines,
            _tmp: tmp,
        }
    })
}

fn java_lines(prefix: &str) -> Vec<String> {
    let prefix = format!("{prefix} ");
    java_run()
        .lines
        .iter()
        .filter_map(|line| line.strip_prefix(&prefix).map(str::to_string))
        .collect()
}

fn field(name: &str, field_type: SortFieldType, reverse: bool) -> SortField {
    SortField::new_reverse(Some(name.to_string()), field_type, reverse).unwrap()
}

fn missing(name: &str, field_type: SortFieldType, value: MissingValue) -> SortField {
    SortField::new_with_missing(Some(name.to_string()), field_type, false, Some(value)).unwrap()
}

/// `SortedSearchFixture.sorts()`, in the same order.
fn sorts() -> Vec<(&'static str, Sort)> {
    use SortFieldType::*;
    let sort = |fields: Vec<SortField>| Sort::new_fields(fields).unwrap();
    vec![
        ("s", sort(vec![field("s", String, false)])),
        ("s_rev", sort(vec![field("s", String, true)])),
        (
            "s_last",
            sort(vec![missing("s", String, MissingValue::StringLast)]),
        ),
        ("l", sort(vec![field("l", Long, false)])),
        ("l_rev", sort(vec![field("l", Long, true)])),
        (
            "l_missing",
            sort(vec![missing("l", Long, MissingValue::Long(1000))]),
        ),
        ("i", sort(vec![field("i", Int, false)])),
        ("d_rev", sort(vec![field("d", Double, true)])),
        ("f", sort(vec![field("f", Float, false)])),
        ("score", sort(vec![SortField::FIELD_SCORE.clone()])),
        ("doc", sort(vec![SortField::FIELD_DOC.clone()])),
        ("sv", sort(vec![field("sv", StringVal, false)])),
        (
            "s_l_rev",
            sort(vec![field("s", String, false), field("l", Long, true)]),
        ),
        (
            "score_doc",
            sort(vec![
                SortField::FIELD_SCORE.clone(),
                SortField::FIELD_DOC.clone(),
            ]),
        ),
        (
            "f_i_s",
            sort(vec![
                field("f", Float, false),
                field("i", Int, true),
                field("s", String, false),
            ]),
        ),
    ]
}

/// An executor that runs every command on a thread of its own, so that each
/// slice's collector really moves to another thread.
struct ThreadPerTask;

impl Executor for ThreadPerTask {
    fn execute(&self, command: Box<dyn FnOnce() + Send + 'static>) {
        std::thread::spawn(command);
    }
}

fn run(prefix: &str, searcher: &IndexSearcher, with_totals: bool) -> Vec<String> {
    let queries: Vec<(&str, Arc<dyn Query>)> = vec![
        (
            "a",
            Arc::new(TermQuery::new(Term::new(
                "body",
                BytesRef::new(b"a".to_vec()),
            ))),
        ),
        ("all", Arc::new(MatchAllDocsQuery::instance())),
    ];
    let mut lines = vec![format!(
        "{prefix}_slices={}",
        searcher.get_slices().unwrap().len()
    )];
    for (sort_name, sort) in sorts() {
        for (query_name, query) in &queries {
            let manager = TopFieldCollectorManager::new(sort.clone(), 10, None, 1000).unwrap();
            let top = searcher
                .search_with_manager(Arc::clone(query), &manager)
                .unwrap();
            let docs: Vec<String> = top
                .score_docs
                .iter()
                .map(|hit| hit.score_doc.doc.to_string())
                .collect();
            let totals = if with_totals {
                let relation = match top.total_hits.relation() {
                    TotalHitsRelation::EQUAL_TO => "EQUAL_TO",
                    TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO => "GREATER_THAN_OR_EQUAL_TO",
                };
                format!("{} {relation} ", top.total_hits.value())
            } else {
                String::new()
            };
            lines.push(format!(
                "{prefix} {sort_name} {query_name} {totals}{}",
                docs.join(",")
            ));
        }
    }
    lines
}

fn java_output(prefix: &str) -> Vec<String> {
    let slices = java_run()
        .lines
        .iter()
        .find(|line| line.starts_with(&format!("{prefix}_slices=")))
        .unwrap_or_else(|| panic!("the fixture printed {prefix}_slices"))
        .clone();
    let mut lines = vec![slices];
    lines.extend(
        java_lines(prefix)
            .into_iter()
            .map(|line| format!("{prefix} {line}")),
    );
    lines
}

fn reader() -> Arc<dyn IndexReader> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let directory: Arc<dyn Directory> = Arc::new(NIOFSDirectory::open(&java_run().dir).unwrap());
    directory_reader::open(directory).unwrap() as Arc<dyn IndexReader>
}

#[test]
fn sorted_search_with_manager_returns_lucenes_hits() {
    assert!(java_run().lines.iter().any(|line| line == "leaves=12"));
    let searcher = IndexSearcher::new(reader()).unwrap();
    assert_eq!(run("hits", &searcher, true), java_output("hits"));
}

#[test]
fn sorted_search_with_manager_on_an_executor_returns_lucenes_hits() {
    let executor: Arc<dyn Executor> = Arc::new(ThreadPerTask);
    let searcher = IndexSearcher::with_executor(reader(), Some(executor)).unwrap();
    assert_eq!(run("xhits", &searcher, false), java_output("xhits"));
}
