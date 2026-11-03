//! `IndexWriter` opened with `OpenMode::CREATE` over an existing index.
//!
//! Pins rmp #265 against Apache Lucene Core 10.5.0: Java's constructor reads
//! the latest commit of an existing index and continues its generation,
//! version and segment counter (`sis.updateGenerationVersionAndCounter(previous)`,
//! `IndexWriter.java:1013-1016`), so the new commit neither overwrites
//! `segments_N` nor reuses a segment name a reader may still have open. The
//! port started from a fresh `SegmentInfos`, named the first new segment `_0`
//! again and failed with "file already exists".
//!
//! # Where the expected values come from
//!
//! `CreateOverFixture` from `tests/fixtures/java-codec-harness`
//! (`java -cp lucene-core-10.5.0.jar CreateOverFixture.java <dir>`) writes two
//! one-document commits, reopens the index with `OpenMode.CREATE`, adds one
//! document and commits, and prints the generation, version, counter and
//! segments of the commit before and after, and the files left behind.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{Document, Store, StringField};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::SegmentInfos;
use rucene::store::{Directory, FSDirectory};

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

/// `CreateOverFixture.doc(int)`.
fn doc(i: i32) -> Document {
    let mut d = Document::new();
    d.add(Box::new(
        StringField::new("id", format!("doc{i}"), Store::YES).unwrap(),
    ));
    d
}

/// The same `key=value` report `CreateOverFixture.report` prints.
fn report(prefix: &str, dir: &Arc<dyn Directory>) -> Vec<String> {
    let infos = SegmentInfos::read_latest_commit(Arc::clone(dir)).unwrap();
    let names: Vec<String> = infos.iter().map(|info| info.info.name.clone()).collect();
    vec![
        format!("{prefix}_gen={}", infos.generation()),
        format!("{prefix}_version={}", infos.version()),
        format!("{prefix}_counter={}", infos.counter),
        format!("{prefix}_segments={}", names.join(",")),
    ]
}

fn config(open_mode: OpenMode) -> IndexWriterConfig {
    let mut config = IndexWriterConfig::new();
    config.set_use_compound_file(false).unwrap();
    config.set_open_mode(open_mode).unwrap();
    config
}

#[test]
fn create_over_an_existing_index_continues_its_commit_as_lucene_does() {
    let java_tmp = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         CreateOverFixture.java",
    );
    let output = Command::new("java")
        .arg("-cp")
        .arg(lucene_jar())
        .arg(&fixture)
        .arg(java_tmp.path())
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "CreateOverFixture failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    // The write lock is left behind by Java's `NativeFSLockFactory` and is not
    // part of the index; it is dropped from both listings.
    let without_lock = |line: &str| -> String {
        match line.strip_prefix("files=") {
            Some(files) => format!(
                "files={}",
                files
                    .split(',')
                    .filter(|file| *file != "write.lock")
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            None => line.to_string(),
        }
    };
    let java: Vec<String> = stdout
        .lines()
        .filter(|line| {
            line.starts_with("before_") || line.starts_with("after_") || line.starts_with("files=")
        })
        .map(without_lock)
        .collect();

    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let rust_tmp = tempfile::tempdir().unwrap();
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(rust_tmp.path()).unwrap());
    let mut rust = Vec::new();
    {
        let writer =
            IndexWriter::new(Arc::clone(&dir), config(OpenMode::CREATE_OR_APPEND)).unwrap();
        writer.add_document(&doc(0)).unwrap();
        writer.commit().unwrap();
        writer.add_document(&doc(1)).unwrap();
        writer.commit().unwrap();
        writer.close().unwrap();
    }
    rust.extend(report("before", &dir));
    {
        let writer = IndexWriter::new(Arc::clone(&dir), config(OpenMode::CREATE))
            .expect("CREATE over an existing index must open");
        writer.add_document(&doc(2)).unwrap();
        writer
            .commit()
            .expect("the commit must not collide with the previous index's files");
        writer.close().unwrap();
    }
    rust.extend(report("after", &dir));
    let mut files = dir.list_all().unwrap();
    files.sort();
    rust.push(without_lock(&format!("files={}", files.join(","))));

    assert_eq!(
        rust, java,
        "Rucene must continue the previous commit as Lucene does"
    );
}
