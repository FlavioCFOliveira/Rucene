//! Flush-path portability tests against Apache Lucene Core 10.5.0.
//!
//! These tests cover the step that turns the raw files an indexing chain wrote
//! into a segment a reader can open: `IndexingChain.flush`'s final
//! `fieldInfosFormat().write(...)` call and
//! `DocumentsWriterPerThread.sealFlushedSegment`, which writes the compound
//! file, the `.si` and the `.liv`.
//!
//! Every other portability suite in this directory drives one codec format in
//! isolation, bypassing [`IndexWriter`] entirely. That is why a segment that no
//! reader could open — no `.fnm`, no `.si` — passed every one of them. These
//! tests deliberately go through the public writer, so what they prove is a
//! property of the *whole* write path.
//!
//! Three things are proven, for four scripted shapes:
//!
//! 1. **Rucene writes the files Lucene writes.** The same sequence of
//!    operations is run by a real Apache Lucene Core 10.5.0 [`IndexWriter`]
//!    (`tests/fixtures/java-codec-harness`, class `FlushFilesFixture`) and by
//!    Rucene's, and the two directory listings are compared name for name, as
//!    are the per-segment file lists, `maxDoc`, `delCount` and the
//!    compound-file flag recorded in each `.si`.
//! 2. **Rucene reads back what Rucene wrote.** `SegmentInfos::read_latest_commit`
//!    and a [`DirectoryReader`] are opened on Rucene's own output.
//! 3. **Lucene reads what Rucene wrote, and Rucene reads what Lucene wrote.**
//!    The Java harness opens Rucene's directory with its own `DirectoryReader`
//!    and prints every document and field name; Rucene opens Java's directory
//!    and does the same. The two document lists must agree.
//!
//! File *names* rather than file *bytes* are compared here, because the two
//! runs cannot produce identical bytes: Lucene stamps every segment with a
//! random 16-byte id (`StringHelper.randomId()`), which the `.si` records and
//! which every codec header in the segment embeds, and it stamps the `.si`
//! diagnostics with a wall-clock timestamp. Byte parity for `.fnm`, `.si`,
//! `.cfs` and `.cfe` is proven separately, with the id and the diagnostics
//! pinned on both sides, in `tests/portability/seal_bytes.rs`.
//!
//! # Shapes covered
//!
//! * `cfs` — five documents, one commit, compound file on (Lucene's default),
//!   so the segment is `.cfe` + `.cfs` + `.si` and nothing else;
//! * `nocfs` — the same with `setUseCompoundFile(false)`, which exposes every
//!   per-format file, `.fnm` included, as a separate entry;
//! * `deletes` — eight documents with two deleted before the flush, so the
//!   segment is sealed with a `.liv` that must land *outside* the compound
//!   file;
//! * `twosegments` — two commits under `NoMergePolicy`, so two segments are
//!   sealed independently and the generation advances.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{Document, Store, StringField, TextField};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{SegmentInfos, Term};
use rucene::store::{Directory, FSDirectory};
use rucene::util::BytesRef;

// ---------------------------------------------------------------------------
// The scripted corpus, duplicated on both sides
// ---------------------------------------------------------------------------

/// The documents both writers index, in this order.
///
/// Mirrors `FlushFilesFixture.doc(int)`: a `StringField` `id` and a `TextField`
/// `body`, both stored. `body` is already lower case and separated by single
/// spaces, so Lucene's `StandardAnalyzer` and Rucene's produce the same tokens.
fn doc(d: usize) -> Document {
    let mut document = Document::new();
    document.add(Box::new(
        StringField::new("id", format!("doc-{d}"), Store::YES).expect("id field"),
    ));
    document.add(Box::new(
        TextField::new("body", format!("hello world number {d}"), Store::YES).expect("body field"),
    ));
    document
}

/// Every case this suite runs, in the order the tests declare them.
const CASES: [&str; 4] = ["cfs", "nocfs", "deletes", "twosegments"];

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
        panic!("flush-path portability tests require Maven and a JDK: {reason}");
    }
}

/// Runs `FlushFilesFixture` and returns its standard output.
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
        .arg("-Dexec.mainClass=org.apache.lucene.rucene.codec.FlushFilesFixture")
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

/// What one run of the Java harness — or of its Rucene counterpart — reports.
#[derive(Debug, Default, PartialEq, Eq)]
struct Report {
    /// The sorted directory listing after the last commit.
    files: Vec<String>,
    /// The generation of the latest commit point.
    generation: i64,
    /// One entry per segment, keyed by segment name.
    segments: BTreeMap<String, SegmentReport>,
    /// Live documents visible to a reader.
    num_docs: i32,
    /// Documents addressable by a reader, deleted ones included.
    max_doc: i32,
    /// Every field name the reader's `FieldInfos` hold, sorted.
    fields: Vec<String>,
    /// `id|body` of every document, in doc-id order.
    docs: Vec<String>,
}

/// What one segment of a commit reports about itself.
#[derive(Debug, Default, PartialEq, Eq)]
struct SegmentReport {
    max_doc: i32,
    del_count: i32,
    use_compound_file: bool,
    files: Vec<String>,
}

impl Report {
    /// Parses the `key=value` lines `FlushFilesFixture` prints.
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
            } else if let Some(value) = line.strip_prefix("fields=") {
                report.fields = split_list(value);
            } else if let Some(value) = line.strip_prefix("docs=") {
                report.docs = value
                    .trim()
                    .split(';')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
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

    /// Renders the report as the aligned block the failure messages print.
    fn render(&self, label: &str) -> String {
        let mut out = format!("{label}:\n  files={}\n", self.files.join(","));
        out.push_str(&format!("  generation={}\n", self.generation));
        for (name, segment) in &self.segments {
            out.push_str(&format!(
                "  segment={name} maxDoc={} delCount={} cfs={} files={}\n",
                segment.max_doc,
                segment.del_count,
                segment.use_compound_file,
                segment.files.join(",")
            ));
        }
        out.push_str(&format!(
            "  numDocs={} maxDoc={} fields={} docs={}\n",
            self.num_docs,
            self.max_doc,
            self.fields.join(","),
            self.docs.join(";")
        ));
        out
    }
}

impl SegmentReport {
    fn parse(value: &str) -> Result<(String, Self), String> {
        let mut name = None;
        let mut segment = Self::default();
        // `files=` is last and is the only value that may itself contain `=`
        // in principle, so it is split off first.
        let (head, files) = match value.split_once(" files=") {
            Some((head, files)) => (head, files),
            None => return Err(format!("missing files= in segment line {value:?}")),
        };
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
                Some(("cfs", v)) => {
                    segment.use_compound_file = v.parse().map_err(|e| format!("{pair:?}: {e}"))?
                }
                // The Java side also prints `id=`, which cannot match across
                // runs and is deliberately not compared.
                Some(("id", _)) => {}
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
    if case == "nocfs" {
        config
            .set_use_compound_file(false)
            .expect("set use compound file");
    }
    let writer = IndexWriter::new(Arc::clone(&dir), config).expect("open writer");

    match case {
        "cfs" | "nocfs" => {
            for d in 0..5 {
                writer.add_document(&doc(d)).expect("add document");
            }
            writer.commit().expect("commit");
        }
        "deletes" => {
            for d in 0..8 {
                writer.add_document(&doc(d)).expect("add document");
            }
            for deleted in ["doc-2", "doc-5"] {
                writer
                    .delete_documents_by_terms(vec![Term::new(
                        "id",
                        BytesRef::new(deleted.as_bytes().to_vec()),
                    )])
                    .expect("delete document");
            }
            writer.commit().expect("commit");
        }
        "twosegments" => {
            for d in 0..3 {
                writer.add_document(&doc(d)).expect("add document");
            }
            writer.commit().expect("first commit");
            for d in 3..6 {
                writer.add_document(&doc(d)).expect("add document");
            }
            writer.commit().expect("second commit");
        }
        other => panic!("unknown case {other:?}"),
    }
    writer.close().expect("close writer");
}

/// Reports on the index at `dir` using Rucene's own readers.
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
                use_compound_file: commit_info.info.get_use_compound_file(),
                files,
            },
        );
    }

    let reader = rucene::index::directory_reader::open(Arc::clone(&dir))
        .expect("Rucene must be able to open a reader over the index it wrote");
    report.num_docs = reader.num_docs();
    report.max_doc = reader.max_doc();

    let mut fields = BTreeSet::new();
    for leaf in Arc::clone(&reader).leaves() {
        for info in leaf.leaf_reader().get_field_infos().iter() {
            fields.insert(info.name.clone());
        }
    }
    report.fields = fields.into_iter().collect();

    let stored = reader.stored_fields().expect("stored fields");
    for doc_id in 0..reader.max_doc() {
        let document = stored.document(doc_id).expect("stored document");
        let get = |name: &str| -> String {
            document
                .get_field(name)
                .and_then(|f| f.string_value())
                .unwrap_or_else(|| "null".to_string())
        };
        report.docs.push(format!("{}|{}", get("id"), get("body")));
    }
    report
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Asserts that Rucene and Lucene lay out the same files for the same script,
/// and that each side reads the other's index back.
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
        java.files, rust.files,
        "[{case}] the two writers laid out different files\n{side_by_side}"
    );
    assert_eq!(
        java.generation, rust.generation,
        "[{case}] commit generation differs\n{side_by_side}"
    );
    assert_eq!(
        java.segments, rust.segments,
        "[{case}] segment metadata differs\n{side_by_side}"
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
        java.fields, rust.fields,
        "[{case}] field names differ\n{side_by_side}"
    );
    assert_eq!(
        java.docs, rust.docs,
        "[{case}] stored documents differ\n{side_by_side}"
    );

    // Direction 3: Lucene opens the index Rucene wrote.
    let read_back =
        run_java_fixture(rust_dir.path(), "read").expect("Lucene must read Rucene's index");
    let java_on_rust = Report::parse(&read_back).expect("parse java read-back");
    assert_eq!(
        java_on_rust.num_docs,
        rust.num_docs,
        "[{case}] Lucene saw a different numDocs in Rucene's index\n{}",
        java_on_rust.render("lucene-on-rucene")
    );
    assert_eq!(
        java_on_rust.max_doc,
        rust.max_doc,
        "[{case}] Lucene saw a different maxDoc in Rucene's index\n{}",
        java_on_rust.render("lucene-on-rucene")
    );
    assert_eq!(
        java_on_rust.fields,
        rust.fields,
        "[{case}] Lucene saw different fields in Rucene's index\n{}",
        java_on_rust.render("lucene-on-rucene")
    );
    assert_eq!(
        java_on_rust.docs,
        rust.docs,
        "[{case}] Lucene read different documents from Rucene's index\n{}",
        java_on_rust.render("lucene-on-rucene")
    );

    // Direction 4: Rucene opens the index Lucene wrote.
    let rust_on_java = rucene_report(java_dir.path());
    assert_eq!(
        rust_on_java.num_docs,
        java.num_docs,
        "[{case}] Rucene saw a different numDocs in Lucene's index\n{}",
        rust_on_java.render("rucene-on-lucene")
    );
    assert_eq!(
        rust_on_java.max_doc,
        java.max_doc,
        "[{case}] Rucene saw a different maxDoc in Lucene's index\n{}",
        rust_on_java.render("rucene-on-lucene")
    );
    assert_eq!(
        rust_on_java.fields,
        java.fields,
        "[{case}] Rucene saw different fields in Lucene's index\n{}",
        rust_on_java.render("rucene-on-lucene")
    );
    assert_eq!(
        rust_on_java.docs,
        java.docs,
        "[{case}] Rucene read different documents from Lucene's index\n{}",
        rust_on_java.render("rucene-on-lucene")
    );
}

#[test]
fn flush_files_match_lucene_compound() {
    assert_case("cfs");
}

#[test]
fn flush_files_match_lucene_no_compound() {
    assert_case("nocfs");
}

#[test]
fn flush_files_match_lucene_with_deletes() {
    assert_case("deletes");
}

#[test]
fn flush_files_match_lucene_two_segments() {
    assert_case("twosegments");
}

/// Prints both directory listings for every case.
///
/// Not an assertion: this is the measurement the acceptance criteria ask to be
/// shown side by side, kept in the suite so it can be re-run at any time with
/// `cargo test --test portability_flush_files -- --nocapture file_listings`.
#[test]
fn file_listings_side_by_side() {
    require_maven();
    for case in CASES {
        let java_dir = tempfile::tempdir().expect("java temp dir");
        let rust_dir = tempfile::tempdir().expect("rucene temp dir");
        let java_stdout = run_java_fixture(java_dir.path(), case).expect("java harness");
        let java = Report::parse(&java_stdout).expect("parse java report");
        run_rucene_case(rust_dir.path(), case);
        let rust_files = {
            let dir: Arc<dyn Directory> =
                Arc::new(FSDirectory::open(rust_dir.path()).expect("open directory"));
            let mut files = dir.list_all().expect("list directory");
            files.sort();
            files
        };
        println!("case={case}");
        println!("  lucene: {}", java.files.join(","));
        println!("  rucene: {}", rust_files.join(","));
    }
}
