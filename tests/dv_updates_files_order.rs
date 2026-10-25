//! `segments_N` lists a segment's doc-values update files in the iteration
//! order of `SegmentCommitInfo.dvUpdatesFiles`, a `java.util.HashMap<Integer,
//! Set<String>>` (`SegmentCommitInfo.java:72`, `SegmentInfos.java:696-702`).
//! The port kept a Rust `HashMap`, whose order is randomised per process, so
//! with two or more updated fields its `segments_N` differed from Lucene's.
//!
//! `DvUpdatesFilesFixture` from `tests/fixtures/java-codec-harness`
//! (`java -cp lucene-core-10.5.0.jar DvUpdatesFilesFixture.java <dir>`)
//! updates five fields, two pairs of which share a bucket of the 16-slot map,
//! and prints the commit's random ids and the map's order. Rucene runs the same
//! operations; its `segments_N` must be Lucene's byte for byte once Lucene's
//! ids are replaced by Rucene's.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{Document, NumericDocValuesField, Store, StringField};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{SegmentInfos, Term};
use rucene::store::{Directory, FSDirectory};
use rucene::util::BytesRef;

/// `DvUpdatesFilesFixture.UPDATED`.
const UPDATED: [&str; 5] = ["f17", "f1", "f19", "f3", "f8"];

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

fn decode_hex(text: &str) -> [u8; 16] {
    let bytes: Vec<u8> = (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
        .collect();
    bytes.try_into().unwrap()
}

fn replace_all(bytes: &mut [u8], from: &[u8; 16], to: &[u8; 16]) -> usize {
    let mut count = 0;
    let mut at = 0;
    while at + 16 <= bytes.len() {
        if &bytes[at..at + 16] == from {
            bytes[at..at + 16].copy_from_slice(to);
            count += 1;
            at += 16;
        } else {
            at += 1;
        }
    }
    count
}

fn checked_body(side: &str, file: &[u8]) -> Vec<u8> {
    let body = &file[..file.len() - 8];
    let stored = u64::from_be_bytes(file[file.len() - 8..].try_into().unwrap());
    assert_eq!(stored, u64::from(crc32fast::hash(body)), "{side} footer");
    body.to_vec()
}

#[test]
fn segments_n_lists_doc_values_update_files_in_lucenes_order() {
    let java_tmp = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         DvUpdatesFilesFixture.java",
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
        "DvUpdatesFilesFixture failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value = |key: &str| {
        stdout
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}=")))
            .unwrap_or_else(|| panic!("the fixture printed {key}"))
            .to_string()
    };
    let java_dv: Vec<String> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("dv ").map(str::to_string))
        .collect();
    assert_eq!(java_dv.len(), UPDATED.len());

    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let rust_tmp = tempfile::tempdir().unwrap();
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(rust_tmp.path()).unwrap());
    {
        let mut config = IndexWriterConfig::new();
        config
            .set_merge_policy(Arc::new(NoMergePolicy::new()))
            .unwrap();
        config.set_use_compound_file(false).unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        for i in 0..10i64 {
            let mut doc = Document::new();
            doc.add(Box::new(
                StringField::new("id", format!("d{i}"), Store::NO).unwrap(),
            ));
            for f in 0..20i64 {
                doc.add(Box::new(NumericDocValuesField::new(
                    &format!("f{f}"),
                    i * 100 + f,
                )));
            }
            writer.add_document(&doc).unwrap();
        }
        writer.commit().unwrap();
        for (offset, field) in UPDATED.iter().enumerate() {
            writer
                .update_numeric_doc_value(
                    Term::new("id", BytesRef::new(b"d3".to_vec())),
                    field,
                    1000 + offset as i64,
                )
                .unwrap();
            // One commit per update, as the fixture does: Lucene applies the
            // fields of one commit in `Map.copyOf` order, which is salted per
            // JVM, so only one field per commit has a reproducible generation.
            writer.commit().unwrap();
        }
        writer.close().unwrap();
    }

    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    let segments = value("segments");
    assert_eq!(infos.segments_file_name().unwrap(), segments);
    let commit = infos.info(0);
    let rust_dv: Vec<String> = commit
        .get_doc_values_updates_files()
        .iter()
        .map(|(field, files)| {
            let names: Vec<&str> = files.iter().map(String::as_str).collect();
            format!("{field} {}", names.join(","))
        })
        .collect();
    assert_eq!(
        rust_dv, java_dv,
        "the update files must be listed in Lucene's order"
    );

    let mut expected = std::fs::read(java_tmp.path().join(&segments)).unwrap();
    let actual = std::fs::read(rust_tmp.path().join(&segments)).unwrap();
    let actual_body = checked_body("rust", &actual);
    for (key, rust_id) in [
        ("infos_id", infos.id()),
        ("si_id", commit.info.id()),
        ("sci_id", commit.id()),
    ] {
        assert!(
            replace_all(&mut expected, &decode_hex(&value(key)), &rust_id) >= 1,
            "{key} occurs in Lucene's segments_N"
        );
    }
    // `SegmentInfos.version` counts every change the writer records
    // (`IndexWriter.java:1024, 2139, 2876, 3812`); it sits right after the
    // header and the four version vInts (`SegmentInfos.java:606-616`) and is
    // compared like every other byte.
    let suffix = segments.strip_prefix("segments_").unwrap();
    let version_at = 4 + 1 + "segments".len() + 4 + 16 + 1 + suffix.len() + 4;
    let java_version: i64 = value("version").parse().unwrap();
    assert_eq!(
        expected[version_at..version_at + 8],
        java_version.to_be_bytes(),
        "the version long sits where the format puts it"
    );
    assert_eq!(infos.version(), java_version, "SegmentInfos.version");
    let expected_body = &expected[..expected.len() - 8];
    let first_difference = actual_body
        .iter()
        .zip(expected_body)
        .position(|(a, b)| a != b);
    assert!(
        actual_body == expected_body,
        "segments_N differs from Lucene's ({} vs {} bytes) at {first_difference:?}: \
         rust {:02x?} lucene {:02x?}",
        actual_body.len(),
        expected_body.len(),
        first_difference
            .map(|at| &actual_body[at.saturating_sub(8)..(at + 24).min(actual_body.len())]),
        first_difference
            .map(|at| &expected_body[at.saturating_sub(8)..(at + 24).min(expected_body.len())]),
    );
}
