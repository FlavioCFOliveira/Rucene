//! Delete-by-query portability tests against Apache Lucene Core 10.5.0.
//!
//! `IndexWriter.deleteDocuments(Query...)` buffers each query and
//! `FrozenBufferedUpdates.applyQueryDeletes` resolves it against every segment
//! the packet reaches. This suite runs the same scripted sequence of additions
//! and delete-by-query calls through a real Apache Lucene Core 10.5.0
//! `IndexWriter` (`tests/fixtures/java-codec-harness`, class
//! `DeleteByQueryFixture`) and through Rucene's, and compares what the two runs
//! leave behind:
//!
//! * the **live documents**, by `id`, and the deleted ones — so a document
//!   deleted on one side and kept on the other fails the test;
//! * `numDocs` and `maxDoc`;
//! * each segment's `delCount` and `delGen` — the generation counter that names
//!   the `.liv`;
//! * the **`.liv` files** produced, and the whole directory listing.
//!
//! Both directions of interoperability are also checked, exactly as
//! `flush_files.rs` does: Lucene opens the index Rucene wrote and reports the
//! same live documents, and Rucene opens the index Lucene wrote and reports the
//! same live documents. A deletion that one side records and the other cannot
//! see is a file-format divergence, not merely a behavioural one.
//!
//! # Why the queries are point queries
//!
//! The corpus selects documents with an `IntPoint` range query and, for the
//! "delete everything" case, with a `MatchAllDocsQuery`. A `TermQuery` would be
//! the more obvious choice, but scoring one against an on-disk segment needs
//! `SegmentTermsEnum::postings`, which is not ported yet. That gap is
//! independent of delete-by-query — it stops delete-by-*term* against a flushed
//! segment as well — and the point and match-all paths drive exactly the same
//! `apply_query_deletes` code.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{Document, IntPoint, Store, StringField, TextField};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::SegmentInfos;
use rucene::search::{MatchAllDocsQuery, Query};
use rucene::store::{Directory, FSDirectory};

/// Every case this suite runs.
const CASES: [&str; 6] = ["byquery", "multi", "nomatch", "pending", "repeat", "all"];

// ---------------------------------------------------------------------------
// The scripted corpus, duplicated on both sides
// ---------------------------------------------------------------------------

/// Mirrors `DeleteByQueryFixture.doc(int)`.
fn doc(d: i32) -> Document {
    let mut document = Document::new();
    document.add(Box::new(
        StringField::new("id", format!("doc-{d}"), Store::YES).expect("id field"),
    ));
    document.add(Box::new(
        TextField::new("body", format!("hello world number {d}"), Store::YES).expect("body field"),
    ));
    document.add(Box::new(IntPoint::new("rank", &[d]).expect("rank field")));
    document
}

fn rank(lo: i32, hi: i32) -> Arc<dyn Query> {
    IntPoint::new_range_query("rank", &[lo], &[hi]).expect("rank range query")
}

// ---------------------------------------------------------------------------
// Java harness driver
// ---------------------------------------------------------------------------

fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/java-codec-harness")
}

/// Serialises Maven invocations, which all share one `target/` directory.
static HARNESS_LOCK: Mutex<()> = Mutex::new(());

fn which_mvn() -> Result<String, String> {
    for candidate in ["mvn", "mvnw"] {
        if Command::new(candidate)
            .arg("-v")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
        {
            return Ok(candidate.to_string());
        }
    }
    Err("Maven is not available on PATH".to_string())
}

/// Fails the test when Maven is unavailable.
///
/// A portability test proves compatibility against the reference Java
/// implementation, so it has nothing to assert without the harness: skipping
/// would report success while proving nothing.
fn require_maven() {
    if let Err(reason) = which_mvn() {
        panic!("delete-by-query portability tests require Maven and a JDK: {reason}");
    }
}

/// Runs `DeleteByQueryFixture` and returns its standard output.
fn run_java_fixture(out_dir: &Path, case: &str) -> Result<String, String> {
    let _guard = HARNESS_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let harness = harness_dir();
    if !harness.join("pom.xml").exists() {
        return Err(format!("pom.xml not found in {}", harness.display()));
    }
    let mvn = which_mvn()?;
    let output = Command::new(mvn)
        .arg("-q")
        .arg("compile")
        .arg("exec:java")
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.DeleteByQueryFixture")
        .arg(format!("-Dexec.args={} {}", out_dir.display(), case))
        .current_dir(&harness)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("failed to spawn Maven: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(format!(
            "Java harness failed for case {case}:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        ));
    }
    Ok(stdout)
}

// ---------------------------------------------------------------------------
// The comparable report
// ---------------------------------------------------------------------------

/// What one run of the Java harness — or of its Rucene counterpart — reports.
#[derive(Debug, Default, PartialEq, Eq)]
struct Report {
    /// The sorted directory listing after the last commit.
    files: Vec<String>,
    /// The generation of the latest commit point.
    generation: i64,
    /// One entry per segment, keyed by segment name.
    segments: BTreeMap<String, SegmentReport>,
    num_docs: i32,
    max_doc: i32,
    /// `id` of every live document, in doc-id order.
    live: Vec<String>,
    /// `id` of every deleted document still present in a segment.
    deleted: Vec<String>,
}

/// What one segment of a commit reports about its deletions.
#[derive(Debug, Default, PartialEq, Eq)]
struct SegmentReport {
    max_doc: i32,
    del_count: i32,
    del_gen: i64,
    files: Vec<String>,
}

impl Report {
    /// Parses the `key=value` lines `DeleteByQueryFixture` prints.
    fn parse(stdout: &str) -> Result<Self, String> {
        let mut report = Self::default();
        for line in stdout.lines() {
            if let Some(value) = line.strip_prefix("files=") {
                report.files = split_list(value);
            } else if let Some(value) = line.strip_prefix("generation=") {
                report.generation = value
                    .trim()
                    .parse()
                    .map_err(|e| format!("bad generation in {line:?}: {e}"))?;
            } else if let Some(value) = line.strip_prefix("segment=") {
                let (name, segment) = SegmentReport::parse(value)?;
                report.segments.insert(name, segment);
            } else if let Some(value) = line.strip_prefix("live=") {
                report.live = split_ids(value);
            } else if let Some(value) = line.strip_prefix("deleted=") {
                report.deleted = split_ids(value);
            } else if let Some(value) = line.strip_prefix("numDocs=") {
                let mut parts = value.split_whitespace();
                report.num_docs = parts
                    .next()
                    .ok_or_else(|| format!("missing numDocs in {line:?}"))?
                    .parse()
                    .map_err(|e| format!("bad numDocs in {line:?}: {e}"))?;
                report.max_doc = parts
                    .next()
                    .and_then(|p| p.strip_prefix("maxDoc="))
                    .ok_or_else(|| format!("missing maxDoc in {line:?}"))?
                    .parse()
                    .map_err(|e| format!("bad maxDoc in {line:?}: {e}"))?;
            }
        }
        Ok(report)
    }

    /// Every `.liv` file the directory holds, sorted.
    fn liv_files(&self) -> Vec<String> {
        self.files
            .iter()
            .filter(|f| f.ends_with(".liv"))
            .cloned()
            .collect()
    }

    /// Renders the report as the aligned block the failure messages print.
    fn render(&self, label: &str) -> String {
        let mut out = format!("{label}:\n  files={}\n", self.files.join(","));
        out.push_str(&format!("  generation={}\n", self.generation));
        for (name, segment) in &self.segments {
            out.push_str(&format!(
                "  segment={name} maxDoc={} delCount={} delGen={} files={}\n",
                segment.max_doc,
                segment.del_count,
                segment.del_gen,
                segment.files.join(",")
            ));
        }
        out.push_str(&format!(
            "  numDocs={} maxDoc={} live={} deleted={}\n",
            self.num_docs,
            self.max_doc,
            self.live.join(","),
            self.deleted.join(",")
        ));
        out
    }
}

impl SegmentReport {
    fn parse(value: &str) -> Result<(String, Self), String> {
        let mut name = None;
        let mut segment = Self::default();
        let (head, files) = value
            .split_once(" files=")
            .ok_or_else(|| format!("missing files= in segment line {value:?}"))?;
        segment.files = split_list(files);
        for pair in head.split_whitespace() {
            match pair.split_once('=') {
                None => name = Some(pair.to_string()),
                Some(("maxDoc", v)) => {
                    segment.max_doc = v.parse().map_err(|e| format!("{pair:?}: {e}"))?
                }
                Some(("delCount", v)) => {
                    segment.del_count = v.parse().map_err(|e| format!("{pair:?}: {e}"))?
                }
                Some(("delGen", v)) => {
                    segment.del_gen = v.parse().map_err(|e| format!("{pair:?}: {e}"))?
                }
                Some((other, _)) => return Err(format!("unexpected key {other:?}")),
            }
        }
        let name = name.ok_or_else(|| format!("missing segment name in {value:?}"))?;
        Ok((name, segment))
    }
}

fn split_list(value: &str) -> Vec<String> {
    let mut items: Vec<String> = value
        .trim()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    items.sort();
    items
}

/// Document ids keep the order they were printed in, which is doc-id order.
fn split_ids(value: &str) -> Vec<String> {
    value
        .trim()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

// ---------------------------------------------------------------------------
// Rucene side
// ---------------------------------------------------------------------------

fn ensure_codec() {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
}

/// Replays `case` with Rucene's [`IndexWriter`] into `out_dir`.
fn run_rucene_case(out_dir: &Path, case: &str) {
    ensure_codec();
    std::fs::create_dir_all(out_dir).expect("create output directory");
    for entry in std::fs::read_dir(out_dir).expect("read output directory") {
        let path = entry.expect("directory entry").path();
        if path.is_file() {
            std::fs::remove_file(path).expect("clear output directory");
        }
    }

    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(out_dir).expect("open directory"));
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .expect("set merge policy");
    let writer = IndexWriter::new(Arc::clone(&dir), config).expect("open writer");

    // Mirrors DeleteByQueryFixture.script(IndexWriter, String) exactly.
    match case {
        "byquery" => {
            for d in 0..6 {
                writer.add_document(&doc(d)).expect("add document");
            }
            writer.commit().expect("commit");
            writer
                .delete_documents_by_queries(vec![rank(0, 2)])
                .expect("delete by query");
            writer.commit().expect("commit");
        }
        "multi" => {
            for d in 0..6 {
                writer.add_document(&doc(d)).expect("add document");
            }
            writer.commit().expect("commit");
            writer
                .delete_documents_by_queries(vec![rank(1, 1), rank(3, 3)])
                .expect("delete by query");
            writer.commit().expect("commit");
        }
        "nomatch" => {
            for d in 0..5 {
                writer.add_document(&doc(d)).expect("add document");
            }
            writer.commit().expect("commit");
            writer
                .delete_documents_by_queries(vec![rank(99, 99)])
                .expect("delete by query");
            writer.commit().expect("commit");
        }
        "pending" => {
            for d in 0..4 {
                writer.add_document(&doc(d)).expect("add document");
            }
            writer
                .delete_documents_by_queries(vec![rank(0, 9)])
                .expect("delete by query");
            for d in 4..6 {
                writer.add_document(&doc(d)).expect("add document");
            }
            writer.commit().expect("commit");
        }
        "repeat" => {
            for d in 0..6 {
                writer.add_document(&doc(d)).expect("add document");
            }
            writer.commit().expect("commit");
            writer
                .delete_documents_by_queries(vec![rank(0, 2)])
                .expect("delete by query");
            writer.commit().expect("commit");
            writer
                .delete_documents_by_queries(vec![rank(0, 2)])
                .expect("repeat the same delete by query");
            writer.commit().expect("commit");
        }
        "all" => {
            for d in 0..6 {
                writer.add_document(&doc(d)).expect("add document");
            }
            writer.commit().expect("commit");
            writer
                .delete_documents_by_queries(vec![Arc::new(MatchAllDocsQuery::instance())])
                .expect("delete every document by query");
            writer.commit().expect("commit");
        }
        other => panic!("unknown case {other:?}"),
    }
    writer.close().expect("close writer");
}

/// Reports on the index at `out_dir` using Rucene's own readers.
fn rucene_report(out_dir: &Path) -> Report {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(out_dir).expect("open directory"));

    let mut report = Report {
        files: {
            let mut files = dir.list_all().expect("list directory");
            files.sort();
            files
        },
        ..Report::default()
    };

    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir))
        .expect("Rucene must be able to reopen the index it wrote");
    report.generation = infos.generation();
    for commit_info in infos.iter() {
        let mut files: Vec<String> = commit_info
            .files()
            .expect("segment files")
            .into_iter()
            .collect();
        files.sort();
        report.segments.insert(
            commit_info.info.name.clone(),
            SegmentReport {
                max_doc: commit_info.info.max_doc().expect("maxDoc"),
                del_count: commit_info.get_del_count(),
                del_gen: commit_info.get_del_gen(),
                files,
            },
        );
    }

    let reader = rucene::index::directory_reader::open(Arc::clone(&dir))
        .expect("Rucene must be able to open a reader over the index it wrote");
    report.num_docs = reader.num_docs();
    report.max_doc = reader.max_doc();

    let stored = reader.stored_fields().expect("stored fields");
    for leaf in Arc::clone(&reader).leaves() {
        let live_docs = leaf.leaf_reader().get_live_docs();
        for i in 0..leaf.leaf_reader().max_doc() {
            let id = stored
                .document(leaf.doc_base() + i)
                .expect("stored document")
                .get_field("id")
                .and_then(|f| f.string_value())
                .unwrap_or_else(|| "null".to_string());
            match &live_docs {
                Some(bits) if !bits.get(i as usize) => report.deleted.push(id),
                _ => report.live.push(id),
            }
        }
    }
    report
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Asserts that Rucene and Lucene delete the same documents for the same
/// script, record the same deletion metadata, and read each other's result.
fn assert_case(case: &str) {
    require_maven();

    let java_dir = tempfile::tempdir().expect("java temp dir");
    let rust_dir = tempfile::tempdir().expect("rucene temp dir");

    let java_stdout = run_java_fixture(java_dir.path(), case).expect("java harness");
    let java = Report::parse(&java_stdout).expect("parse java report");

    run_rucene_case(rust_dir.path(), case);
    let rust = rucene_report(rust_dir.path());

    let side_by_side = format!(
        "[{case}]\n{}{}",
        java.render("lucene"),
        rust.render("rucene")
    );

    assert_eq!(
        java.live, rust.live,
        "[{case}] the two writers left different documents alive\n{side_by_side}"
    );
    assert_eq!(
        java.deleted, rust.deleted,
        "[{case}] the two writers deleted different documents\n{side_by_side}"
    );
    assert_eq!(
        java.num_docs, rust.num_docs,
        "[{case}] numDocs differs\n{side_by_side}"
    );
    assert_eq!(
        java.max_doc, rust.max_doc,
        "[{case}] maxDoc differs\n{side_by_side}"
    );
    assert_eq!(
        java.liv_files(),
        rust.liv_files(),
        "[{case}] the two writers produced different .liv files\n{side_by_side}"
    );
    assert_eq!(
        java.segments, rust.segments,
        "[{case}] segment deletion metadata differs\n{side_by_side}"
    );
    assert_eq!(
        java.files, rust.files,
        "[{case}] the two writers laid out different files\n{side_by_side}"
    );
    assert_eq!(
        java.generation, rust.generation,
        "[{case}] commit generation differs\n{side_by_side}"
    );

    // Lucene opens the index Rucene wrote, and must see the same live documents.
    let read_back =
        run_java_fixture(rust_dir.path(), "read").expect("Lucene must read Rucene's index");
    let java_on_rust = Report::parse(&read_back).expect("parse java read-back");
    assert_eq!(
        java_on_rust.live,
        rust.live,
        "[{case}] Lucene saw different live documents in Rucene's index\n{}",
        java_on_rust.render("lucene-on-rucene")
    );
    assert_eq!(
        java_on_rust.deleted,
        rust.deleted,
        "[{case}] Lucene saw different deleted documents in Rucene's index\n{}",
        java_on_rust.render("lucene-on-rucene")
    );
    assert_eq!(
        java_on_rust.num_docs,
        rust.num_docs,
        "[{case}] Lucene saw a different numDocs in Rucene's index\n{}",
        java_on_rust.render("lucene-on-rucene")
    );

    // Rucene opens the index Lucene wrote, and must see the same live documents.
    let rust_on_java = rucene_report(java_dir.path());
    assert_eq!(
        rust_on_java.live,
        java.live,
        "[{case}] Rucene saw different live documents in Lucene's index\n{}",
        rust_on_java.render("rucene-on-lucene")
    );
    assert_eq!(
        rust_on_java.deleted,
        java.deleted,
        "[{case}] Rucene saw different deleted documents in Lucene's index\n{}",
        rust_on_java.render("rucene-on-lucene")
    );
    assert_eq!(
        rust_on_java.num_docs,
        java.num_docs,
        "[{case}] Rucene saw a different numDocs in Lucene's index\n{}",
        rust_on_java.render("rucene-on-lucene")
    );
}

#[test]
fn delete_by_query_matches_lucene() {
    assert_case("byquery");
}

#[test]
fn delete_by_query_multiple_queries_match_lucene() {
    assert_case("multi");
}

#[test]
fn delete_by_query_matching_nothing_matches_lucene() {
    assert_case("nomatch");
}

#[test]
fn delete_by_query_segment_private_limit_matches_lucene() {
    assert_case("pending");
}

#[test]
fn delete_by_query_repeated_matches_lucene() {
    assert_case("repeat");
}

#[test]
fn delete_by_query_matching_everything_matches_lucene() {
    assert_case("all");
}

/// Prints both reports for every case, side by side.
///
/// Not an assertion: this is the measurement the acceptance criteria ask to be
/// shown, kept in the suite so it can be re-run at any time with
/// `cargo test --test portability_delete_by_query -- --nocapture side_by_side`.
#[test]
fn live_documents_side_by_side() {
    require_maven();
    for case in CASES {
        let java_dir = tempfile::tempdir().expect("java temp dir");
        let rust_dir = tempfile::tempdir().expect("rucene temp dir");
        let java_stdout = run_java_fixture(java_dir.path(), case).expect("java harness");
        let java = Report::parse(&java_stdout).expect("parse java report");
        run_rucene_case(rust_dir.path(), case);
        let rust = rucene_report(rust_dir.path());
        println!("case={case}");
        println!(
            "  lucene: numDocs={} live=[{}] deleted=[{}] liv={:?}",
            java.num_docs,
            java.live.join(","),
            java.deleted.join(","),
            java.liv_files()
        );
        println!(
            "  rucene: numDocs={} live=[{}] deleted=[{}] liv={:?}",
            rust.num_docs,
            rust.live.join(","),
            rust.deleted.join(","),
            rust.liv_files()
        );
    }
}
