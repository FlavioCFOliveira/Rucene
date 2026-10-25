//! `IndexWriter::force_merge` of segments that carry an index sort.
//!
//! Pins rmp #229 against Apache Lucene Core 10.5.0: `MergeState.buildDocMaps`
//! (`MergeState.java:212-240`) merge-sorts the incoming sorted segments through
//! `MultiSorter.sort` and sets `needsIndexSort` only when that reorders
//! documents. The port built plain deletion doc maps and derived
//! `needs_index_sort` from the configuration alone, so sorted segments were
//! concatenated instead of interleaved.
//!
//! # Where the expected bytes come from
//!
//! `SortedMergeFixture` from `tests/fixtures/java-codec-harness`
//! (`java -cp lucene-core-10.5.0.jar SortedMergeFixture.java <dir>`) writes the
//! same documents as three committed segments sorted by `rank` and
//! force-merges them. A segment id is random, so every merged file is compared
//! with Lucene's after Lucene's segment id is replaced by Rucene's; the
//! footer's checksum is checked on its own.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use rucene::codecs::Lucene104Codec;
use rucene::document::{
    Document, NumericDocValuesField, Store, StoredField, StringField, TextField,
};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::{directory_reader, SegmentInfos};
use rucene::search::{Sort, SortField, SortFieldType, NO_MORE_DOCS};
use rucene::store::{Directory, RamDirectory, DEFAULT_IO_CONTEXT};

/// `SortedMergeFixture.SIZES`.
const SIZES: [i32; 3] = [300, 300, 300];

/// `SortedMergeFixture.doc(int)`, field for field and in the same order.
fn doc(i: i32) -> Document {
    let mut d = Document::new();
    d.add(Box::new(
        StringField::new("id", format!("id{i:05}"), Store::NO).unwrap(),
    ));
    d.add(Box::new(NumericDocValuesField::new(
        "rank",
        (i64::from(i) * 7919) % 1000,
    )));
    d.add(Box::new(
        StoredField::new_string("sv", format!("v{i}")).unwrap(),
    ));
    d.add(Box::new(
        TextField::new("body", format!("all w{} x{i}", i % 31), Store::NO).unwrap(),
    ));
    d
}

fn rank_sort() -> Sort {
    Sort::new_fields(vec![SortField::new(
        Some("rank".to_string()),
        SortFieldType::Long,
    )
    .unwrap()])
    .unwrap()
}

fn rust_merge() -> Arc<dyn Directory> {
    let _ = rucene::codecs::register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config.set_index_sort(rank_sort()).unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    let mut i = 0;
    for size in SIZES {
        for _ in 0..size {
            writer.add_document(&doc(i)).unwrap();
            i += 1;
        }
        writer.commit().unwrap();
    }
    assert_eq!(
        SegmentInfos::read_latest_commit(Arc::clone(&dir))
            .unwrap()
            .size(),
        SIZES.len(),
        "nothing merges before force_merge"
    );
    writer.force_merge(1).unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();
    dir
}

/// What `SortedMergeFixture` printed about Lucene's merged segment.
struct JavaMerge {
    dir: PathBuf,
    name: String,
    id: [u8; 16],
    max_doc: i32,
    files: BTreeMap<String, u64>,
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

fn java_merge() -> JavaMerge {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         SortedMergeFixture.java",
    );
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("sorted");
    let output = Command::new("java")
        .arg("-cp")
        .arg(lucene_jar())
        .arg(&fixture)
        .arg(&dir)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "SortedMergeFixture failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let (mut name, mut id, mut max_doc) = (None, None, None);
    let mut files = BTreeMap::new();
    for line in stdout.lines() {
        if let Some(value) = line.strip_prefix("name=") {
            name = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("id=") {
            let decoded: Vec<u8> = (0..value.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(&value[at..at + 2], 16).unwrap())
                .collect();
            id = Some(<[u8; 16]>::try_from(decoded).unwrap());
        } else if let Some(value) = line.strip_prefix("maxDoc=") {
            let (count, deletions) = value.split_once(" delCount=").unwrap();
            assert_eq!(deletions, "0");
            max_doc = Some(count.parse().unwrap());
        } else if let Some(value) = line.strip_prefix("file ") {
            let (file, length) = value.split_once(' ').unwrap();
            files.insert(file.to_string(), length.parse().unwrap());
        } else if let Some(value) = line.strip_prefix("segments=") {
            assert_eq!(value, "1");
        }
    }
    JavaMerge {
        dir,
        name: name.expect("the fixture printed the segment name"),
        id: id.expect("the fixture printed the segment id"),
        max_doc: max_doc.expect("the fixture printed the document count"),
        files,
        _tmp: tmp,
    }
}

fn read_file(dir: &dyn Directory, name: &str) -> Vec<u8> {
    let length = dir.file_length(name).unwrap() as usize;
    let mut input = dir.open_input(name, &*DEFAULT_IO_CONTEXT).unwrap();
    let mut bytes = vec![0u8; length];
    input.read_bytes(&mut bytes, 0, length).unwrap();
    bytes
}

fn assert_valid_footer(side: &str, name: &str, file: &[u8]) {
    let body = &file[..file.len() - 8];
    let stored = u64::from_be_bytes(file[file.len() - 8..].try_into().unwrap());
    assert_eq!(
        stored,
        u64::from(crc32fast::hash(body)),
        "{side} {name}: the footer checksum must cover the file"
    );
}

fn replace_id(bytes: &mut [u8], from: &[u8; 16], to: &[u8; 16]) -> usize {
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

#[test]
fn a_sorted_force_merge_interleaves_the_segments_as_lucene_does() {
    let java = java_merge();
    let dir = rust_merge();

    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    assert_eq!(infos.size(), 1);
    let merged = infos.info(0);
    assert_eq!(merged.info.name, java.name, "the merged segment's name");
    assert_eq!(merged.info.max_doc().unwrap(), java.max_doc, "maxDoc");
    let rust_id = merged.info.id();
    let files: BTreeSet<String> = merged.files().unwrap().into_iter().collect();
    assert_eq!(
        files,
        java.files.keys().cloned().collect::<BTreeSet<_>>(),
        "the merged segment's files"
    );
    for name in &files {
        if name.ends_with(".si") {
            continue;
        }
        let mut expected = std::fs::read(java.dir.join(name)).unwrap();
        let actual = read_file(dir.as_ref(), name);
        assert_valid_footer("Lucene", name, &expected);
        assert_valid_footer("Rucene", name, &actual);
        assert!(replace_id(&mut expected, &java.id, &rust_id) >= 1);
        let body = actual.len().min(expected.len()) - 8;
        assert!(
            actual.len() == expected.len() && actual[..body] == expected[..body],
            "{name}: Lucene has {} bytes, Rucene {}; first difference at {:?}",
            expected.len(),
            actual.len(),
            (0..body).find(|&at| actual[at] != expected[at])
        );
        println!(
            "{name} lucene={} rucene={} equal=true",
            expected.len(),
            actual.len()
        );
    }

    // The merged segment is in `rank` order.
    let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
    let leaves = Arc::clone(&reader).leaves();
    let mut ranks = leaves[0]
        .leaf_reader()
        .get_numeric_doc_values("rank")
        .unwrap()
        .unwrap();
    let mut previous = i64::MIN;
    loop {
        let doc = ranks.next_doc().unwrap();
        if doc == NO_MORE_DOCS {
            break;
        }
        let rank = ranks.long_value().unwrap();
        assert!(rank >= previous, "document {doc} breaks the rank order");
        previous = rank;
    }
}
