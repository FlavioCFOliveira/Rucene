//! The flush of a segment that holds document blocks and carries an index
//! sort.
//!
//! Pins rmp #268 against Apache Lucene Core 10.5.0: with a parent field
//! declared (`IndexWriterConfig.setParentField`), `IndexingChain.maybeSortSegment`
//! (`IndexingChain.java:248-296`) wraps every comparator so that it compares
//! the parent (last) document of each block, which keeps every block
//! contiguous and in its original order while the blocks are sorted. The port
//! refused such a segment.
//!
//! The documents also carry one value — or two — of every other doc-values
//! kind, so the five doc-values writers are flushed through the sort map, and
//! their buffers (`PackedLongValues`, `PagedBytes`, the shared byte pool) are
//! exercised end to end.
//!
//! # Where the expected bytes come from
//!
//! `SortedBlocksFlushFixture` from `tests/fixtures/java-codec-harness`
//! (`java -cp lucene-core-10.5.0.jar SortedBlocksFlushFixture.java <dir>`)
//! writes the same blocks with a real `IndexWriter` and commits one segment.
//! Every file but the `.si` (whose diagnostics name the JVM) is compared with
//! Lucene's after Lucene's segment id is replaced by Rucene's; the footer
//! checksum is checked on its own. The `.si` is compared field by field
//! without its segment id and its diagnostics — version, minimum version,
//! `maxDoc`, compound flag, blocks flag, file set, attributes, and the
//! index-sort block (`Lucene99SegmentInfoFormat.java:185-237`), byte for byte.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use rucene::codecs::Lucene104Codec;
use rucene::document::{
    BinaryDocValuesField, Document, NumericDocValuesField, SortedDocValuesField,
    SortedNumericDocValuesField, SortedSetDocValuesField, Store, StringField, TextField,
};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::SegmentInfos;
use rucene::search::{Sort, SortField, SortFieldType};
use rucene::store::{ByteArrayDataInput, DataInput, Directory, RamDirectory, DEFAULT_IO_CONTEXT};
use rucene::util::BytesRef;

/// `SortedBlocksFlushFixture.BLOCKS`.
const BLOCKS: i32 = 40;

fn bytes(text: String) -> BytesRef {
    BytesRef::new(text.into_bytes())
}

/// `SortedBlocksFlushFixture.block(int)`, field for field and in the same
/// order.
fn block(b: i32) -> Vec<Document> {
    let size = (b * 3) % 4 + 1;
    let mut docs = Vec::new();
    for k in 0..size {
        let i = b * 10 + k;
        let mut d = Document::new();
        d.add(Box::new(
            StringField::new("id", format!("id{i}"), Store::YES).unwrap(),
        ));
        d.add(Box::new(
            TextField::new("body", format!("w{} x{}", i % 7, b % 5), Store::NO).unwrap(),
        ));
        d.add(Box::new(SortedDocValuesField::new(
            "kind",
            bytes(format!("k{}", i % 6)),
        )));
        d.add(Box::new(BinaryDocValuesField::new(
            "blob",
            bytes(format!("b{}", i * 31 % 97)),
        )));
        d.add(Box::new(SortedNumericDocValuesField::new(
            "multi",
            i64::from((i * 13) % 50),
        )));
        d.add(Box::new(SortedNumericDocValuesField::new(
            "multi",
            i64::from((i * 7) % 50),
        )));
        d.add(Box::new(SortedSetDocValuesField::new(
            "tags",
            bytes(format!("t{}", i % 4)),
        )));
        d.add(Box::new(SortedSetDocValuesField::new(
            "tags",
            bytes(format!("t{}", i % 3)),
        )));
        if k == size - 1 {
            d.add(Box::new(NumericDocValuesField::new(
                "rank",
                (i64::from(b) * 7919) % 101,
            )));
        }
        docs.push(d);
    }
    docs
}

fn rust_flush() -> Arc<dyn Directory> {
    let _ = rucene::codecs::register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config
        .set_index_sort(
            Sort::new_fields(vec![SortField::new(
                Some("rank".to_string()),
                SortFieldType::Long,
            )
            .unwrap()])
            .unwrap(),
        )
        .unwrap();
    config.set_parent_field(Some("_parent".to_string()));
    config.set_use_compound_file(false).unwrap();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for b in 0..BLOCKS {
        writer.add_documents(&block(b)).unwrap();
    }
    writer.commit().unwrap();
    writer.close().unwrap();
    dir
}

/// What `SortedBlocksFlushFixture` printed about Lucene's segment.
struct JavaSegment {
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

fn java_flush() -> JavaSegment {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         SortedBlocksFlushFixture.java",
    );
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("sorted-blocks");
    let output = Command::new("java")
        .arg("-cp")
        .arg(lucene_jar())
        .arg(&fixture)
        .arg(&dir)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "SortedBlocksFlushFixture failed:\n{}",
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
            max_doc = Some(value.parse().unwrap());
        } else if let Some(value) = line.strip_prefix("file ") {
            let (file, length) = value.split_once(' ').unwrap();
            files.insert(file.to_string(), length.parse().unwrap());
        } else if let Some(value) = line.strip_prefix("segments=") {
            assert_eq!(value, "1");
        }
    }
    JavaSegment {
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

/// The bytes of a `.si` file without the segment id of its index header and
/// without its diagnostics map, which names the JVM and the time; everything
/// else, from the format version to the index-sort block, is kept in file
/// order (`Lucene99SegmentInfoFormat.java:174-237`).
fn si_without_id_and_diagnostics(file: &[u8]) -> Vec<u8> {
    let body = &file[..file.len() - 16];
    let mut input = ByteArrayDataInput::new(body);
    // `CodecUtil.writeIndexHeader`: magic, codec name, version, id, suffix.
    input.read_int().unwrap();
    input.read_string().unwrap();
    input.read_int().unwrap();
    let before_id = input.position();
    let mut id = [0u8; 16];
    input.read_bytes(&mut id, 0, 16).unwrap();
    let after_id = input.position();
    let suffix_length = input.read_byte().unwrap() as usize;
    let mut suffix = vec![0u8; suffix_length];
    input.read_bytes(&mut suffix, 0, suffix_length).unwrap();
    // Version, optional minimum version, maxDoc, compound flag, blocks flag.
    for _ in 0..3 {
        input.read_int().unwrap();
    }
    if input.read_byte().unwrap() == 1 {
        for _ in 0..3 {
            input.read_int().unwrap();
        }
    }
    input.read_int().unwrap();
    input.read_byte().unwrap();
    input.read_byte().unwrap();
    let before_diagnostics = input.position();
    input.read_map_of_strings().unwrap();
    let after_diagnostics = input.position();
    let mut kept = body[..before_id].to_vec();
    kept.extend_from_slice(&body[after_id..before_diagnostics]);
    kept.extend_from_slice(&body[after_diagnostics..]);
    kept
}

#[test]
fn the_segment_info_matches_lucene_but_for_id_and_diagnostics() {
    let java = java_flush();
    let dir = rust_flush();
    let name = format!("{}.si", java.name);
    let expected = std::fs::read(java.dir.join(&name)).unwrap();
    let actual = read_file(dir.as_ref(), &name);
    assert_valid_footer("Lucene", &name, &expected);
    assert_valid_footer("Rucene", &name, &actual);
    let expected = si_without_id_and_diagnostics(&expected);
    let actual = si_without_id_and_diagnostics(&actual);
    assert_eq!(
        actual,
        expected,
        "the .si differs from Lucene's at {:?}",
        (0..actual.len().min(expected.len())).find(|&at| actual[at] != expected[at])
    );
}

#[test]
fn a_blocked_sorted_segment_flushes_byte_identical_to_lucene() {
    let java = java_flush();
    let dir = rust_flush();

    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    assert_eq!(infos.size(), 1);
    let segment = infos.info(0);
    assert_eq!(segment.info.name, java.name, "the segment's name");
    assert_eq!(segment.info.max_doc().unwrap(), java.max_doc, "maxDoc");
    assert!(
        segment.info.get_has_blocks(),
        "the segment records its blocks"
    );
    let rust_id = segment.info.id();
    let files: BTreeSet<String> = segment.files().unwrap().into_iter().collect();
    assert_eq!(
        files,
        java.files.keys().cloned().collect::<BTreeSet<_>>(),
        "the segment's files"
    );
    let mut differing = Vec::new();
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
        if !(actual.len() == expected.len() && actual[..body] == expected[..body]) {
            differing.push(format!(
                "{name}: Lucene has {} bytes, Rucene {}; first difference at {:?}",
                expected.len(),
                actual.len(),
                (0..body).find(|&at| actual[at] != expected[at])
            ));
        }
    }
    assert!(
        differing.is_empty(),
        "files differing from Lucene:\n{}",
        differing.join("\n")
    );
}
