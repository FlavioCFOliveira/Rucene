//! Postings contracts pinned against Apache Lucene Core 10.5.0.
//!
//! * **rmp #238.** `readLevel0PosData` reads the position and payload skip
//!   data of the next level-0 block from the **doc** file
//!   (`Lucene104PostingsReader.java:792-799`), where the skip data is
//!   interleaved with the doc blocks. The port read it from the positions and
//!   payloads files, so an `advance` that skipped a block positioned the
//!   positions stream from garbage. The test reads a Lucene-written index whose
//!   term occurs in 10000 documents and compares every posting an `advance`
//!   schedule reaches, positions, offsets and payloads included, and the hits
//!   of a phrase query.
//! * **rmp #240.** The `PostingsEnum` flag constants have Lucene's values
//!   (`PostingsEnum.java:34-64`).
//! * **rmp #241.** `ForUtil.decode` reached with a width past its 32-entry mask
//!   table throws `ArrayIndexOutOfBoundsException` (`ForUtil.java:168-170`);
//!   the port panicked. The Java side is measured on `ForUtil` directly; the
//!   Rust side on a `.doc` file whose full block was corrupted, which is the
//!   path that reaches it (`Lucene104PostingsReader.java:594-597`).
//!
//! # Where the expected values come from
//!
//! `PostingsSkipFixture` from `tests/fixtures/java-codec-harness`
//! (`java -cp lucene-core-10.5.0.jar PostingsSkipFixture.java <dir>`) writes
//! the index and prints what Lucene's own reader returns.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::index::{
    directory_reader, IndexReader, PostingsEnum, Term, POSTINGS_ENUM_ALL, POSTINGS_ENUM_FREQS,
    POSTINGS_ENUM_NONE, POSTINGS_ENUM_OFFSETS, POSTINGS_ENUM_PAYLOADS, POSTINGS_ENUM_POSITIONS,
};
use rucene::search::{IndexSearcher, PhraseQuery, Query, NO_MORE_DOCS};
use rucene::store::{Directory, NIOFSDirectory};
use rucene::util::BytesRef;

/// `PostingsSkipFixture.DOCS`.
const DOCS: i32 = 10_000;

/// `PostingsSkipFixture.TARGETS`.
const TARGETS: [i32; 16] = [
    0, 1, 255, 256, 257, 300, 600, 1024, 1025, 4097, 8191, 8192, 8193, 8600, 9000, 9999,
];

/// What the fixture printed, and the index it wrote.
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

/// Runs the fixture once per test process; every test reads the same index.
fn java_run() -> &'static JavaRun {
    static RUN: OnceLock<JavaRun> = OnceLock::new();
    RUN.get_or_init(|| {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
             PostingsSkipFixture.java",
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
            "PostingsSkipFixture failed:\n{}",
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

/// The lines the fixture printed with `prefix `, prefix removed.
fn java_lines(prefix: &str) -> Vec<String> {
    let prefix = format!("{prefix} ");
    java_run()
        .lines
        .iter()
        .filter_map(|line| line.strip_prefix(&prefix).map(str::to_string))
        .collect()
}

fn term(field: &str, text: &str) -> Term {
    Term::new(field, BytesRef::new(text.as_bytes().to_vec()))
}

/// `PostingsSkipFixture.describe`: doc, freq, then pos/start-end/payload.
fn describe(postings: &mut dyn PostingsEnum, doc: i32) -> String {
    if doc == NO_MORE_DOCS {
        return "END".to_string();
    }
    let freq = postings.freq().unwrap();
    let mut out = format!("{doc} freq={freq}");
    for _ in 0..freq {
        let position = postings.next_position().unwrap();
        let payload = match postings.get_payload().unwrap() {
            Some(bytes) => bytes.iter().map(|b| format!("{b:02x}")).collect(),
            None => "-".to_string(),
        };
        out.push_str(&format!(
            " {position}/{}-{}/{payload}",
            postings.start_offset(),
            postings.end_offset()
        ));
    }
    out
}

fn open_searcher() -> IndexSearcher {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let directory: Arc<dyn Directory> = Arc::new(NIOFSDirectory::open(&java_run().dir).unwrap());
    let reader = directory_reader::open(directory).unwrap();
    IndexSearcher::new(reader as Arc<dyn IndexReader>).unwrap()
}

#[test]
fn advance_reads_the_position_skip_data_lucene_wrote() {
    let searcher = open_searcher();
    assert!(java_run().lines.iter().any(|l| l == "leaves=1"));
    let leaves = searcher.get_leaf_contexts();
    assert_eq!(leaves.len(), 1);
    let leaf = leaves[0].leaf_reader();

    let mut advanced = Vec::new();
    let mut walked = Vec::new();
    for field in ["body", "plain"] {
        for flags in [POSTINGS_ENUM_POSITIONS, POSTINGS_ENUM_ALL] {
            let mut postings = leaf.postings(&term(field, "a"), flags).unwrap().unwrap();
            for target in TARGETS {
                let doc = postings.advance(target).unwrap();
                advanced.push(format!(
                    "{field} {flags} {target} {}",
                    describe(postings.as_mut(), doc)
                ));
            }
        }
        let mut postings = leaf
            .postings(&term(field, "a"), POSTINGS_ENUM_ALL)
            .unwrap()
            .unwrap();
        let mut doc = postings.next_doc().unwrap();
        while doc != NO_MORE_DOCS {
            walked.push(format!("{field} {}", describe(postings.as_mut(), doc)));
            doc = postings.next_doc().unwrap();
            if doc != NO_MORE_DOCS {
                doc = postings.advance(doc + 397).unwrap();
            }
        }
    }
    assert_eq!(advanced.len(), 64);
    assert_eq!(
        advanced,
        java_lines("advance"),
        "every advanced-to posting must match Lucene's"
    );
    assert_eq!(
        walked,
        java_lines("walk"),
        "the interleaved walk must match"
    );
}

/// Round 2, item 3: a docs-and-freqs enumeration walks every block through
/// the fast path of `moveToNextLevel0Block` (`Lucene104PostingsReader.java:780-786`)
/// and must still see Lucene's documents and frequencies.
#[test]
fn docs_and_freqs_walk_matches_lucene() {
    let searcher = open_searcher();
    let leaf = searcher.get_leaf_contexts()[0].leaf_reader();
    let mut rust = Vec::new();
    for field in ["body", "plain"] {
        let mut postings = leaf
            .postings(&term(field, "a"), POSTINGS_ENUM_FREQS)
            .unwrap()
            .unwrap();
        let (mut count, mut doc_sum, mut freq_sum) = (0i64, 0i64, 0i64);
        let mut doc = postings.next_doc().unwrap();
        while doc != NO_MORE_DOCS {
            count += 1;
            doc_sum += i64::from(doc);
            freq_sum += i64::from(postings.freq().unwrap()) * (i64::from(doc) + 1);
            doc = postings.next_doc().unwrap();
        }
        rust.push(format!("{field} {count} {doc_sum} {freq_sum}"));
    }
    assert_eq!(rust, java_lines("freqs"));
}

#[test]
fn phrase_query_hits_match_lucene_over_skipped_blocks() {
    let searcher = open_searcher();
    let mut rust = Vec::new();
    for field in ["body", "plain"] {
        let phrase: Arc<dyn Query> = Arc::new(
            PhraseQuery::new(0, vec![term(field, "a"), term(field, "b")], vec![0, 1]).unwrap(),
        );
        let top = searcher.search(phrase, DOCS).unwrap();
        let mut docs: Vec<i32> = top.score_docs.iter().map(|hit| hit.doc).collect();
        docs.sort_unstable();
        let docs: Vec<String> = docs.iter().map(i32::to_string).collect();
        rust.push(format!(
            "{field} {} {}",
            top.total_hits.value(),
            docs.join(",")
        ));
    }
    assert_eq!(rust, java_lines("phrase"));
}

#[test]
fn postings_flags_have_lucenes_values() {
    let rust = format!(
        "NONE={POSTINGS_ENUM_NONE} FREQS={POSTINGS_ENUM_FREQS} \
         POSITIONS={POSTINGS_ENUM_POSITIONS} OFFSETS={POSTINGS_ENUM_OFFSETS} \
         PAYLOADS={POSTINGS_ENUM_PAYLOADS} ALL={POSTINGS_ENUM_ALL}"
    );
    assert_eq!(vec![rust], java_lines("flags"));
}

#[test]
fn for_util_widths_past_the_mask_table_throw_lucenes_exception() {
    // What Lucene throws, and that it throws before reading a byte.
    let expected: Vec<String> = [32, 33, 40, 127]
        .iter()
        .map(|bits| {
            format!(
                "{bits} java.lang.ArrayIndexOutOfBoundsException \
                 Index {bits} out of bounds for length 32 fp=0"
            )
        })
        .collect();
    assert_eq!(java_lines("forutil"), expected);
}
