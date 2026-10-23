//! `IndexWriter::force_merge` of indexed terms, points, per-field doc values
//! and committed deletions, against Apache Lucene Core 10.5.0.
//!
//! Four merge-path defects kept a real index from surviving a merge, and these
//! tests pin the repaired path:
//!
//! * **rmp #217.** The postings merge was a no-op, and the block-tree writer's
//!   `close` then wrote the postings header of `.tmd` with an empty segment
//!   suffix, so a merged segment with an indexed field could not be reopened
//!   ("file mismatch, expected suffix=\"Lucene104_0\", got=\"\""). The default
//!   `FieldsConsumer.merge` (`FieldsConsumer.java:72-96`) is now ported, over
//!   `MappedMultiFields`, and the writer has Lucene's lifecycle.
//! * **rmp #218.** The points merge was a no-op. The default
//!   `PointsWriter.merge`/`mergeOneField` (`PointsWriter.java:42-236`) and
//!   `Lucene90PointsWriter.merge` (`Lucene90PointsWriter.java:197-299`), with
//!   its `BKDWriter.merge` path, are now ported.
//! * **rmp #219.** `PerFieldMergeState::restrict_fields` dropped the doc maps,
//!   the live docs, every reader but the postings producers, the index-sort
//!   flag and the info stream, so a per-field doc-values merge read nothing.
//!   It now restricts only what `PerFieldMergeState.java:43-74` restricts.
//! * **rmp #220.** `PendingDeletes::get_live_docs` moved the live docs out
//!   instead of sharing them (`PendingDeletes.java:108-112`), so a merge of a
//!   segment with committed deletions failed in `verify_doc_counts`.
//!
//! # The oracle
//!
//! Each test runs `MergedFieldsFixture` from `tests/fixtures/java-codec-harness`
//! on a real Lucene 10.5.0 (`java -cp lucene-core-10.5.0.jar
//! MergedFieldsFixture.java <dir> <case>`): the same documents, committed as the
//! same segments, deleted the same way and force-merged into one segment. Every
//! file of the two merged segments is then compared **byte for byte**, except
//! the `.si`, whose diagnostics carry the JVM, the OS and a timestamp. A segment
//! id is random, so Lucene's id is replaced by this port's wherever it occurs —
//! once in most headers, twice in `.tmd`, whose postings header carries it too
//! — and the footer checksum, which covers the id, is left out of the
//! comparison and verified on its own on both sides.
//!
//! A missing JDK or Lucene jar is a hard failure, not a skip: these tests have
//! nothing to assert without the reference implementation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use rucene::codecs::Lucene104Codec;
use rucene::document::{
    BinaryDocValuesField, Document, Field, FieldType, IntPoint, LongPoint, NumericDocValuesField,
    SortedDocValuesField, SortedNumericDocValuesField, SortedSetDocValuesField, Store, StoredField,
    StringField, TextField,
};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::terms::Term;
use rucene::index::{directory_reader, DirectoryReader, IndexOptions, SegmentInfos};
use rucene::search::{IndexSearcher, Query, TermQuery, NO_MORE_DOCS};
use rucene::store::{Directory, RamDirectory, DEFAULT_IO_CONTEXT};
use rucene::util::BytesRef;

// ---------------------------------------------------------------------------
// The documents: `MergedFieldsFixture.doc(int, int, int)`, field for field
// ---------------------------------------------------------------------------

const POSTINGS: u32 = 1;
const POINTS: u32 = 2;
const DOC_VALUES: u32 = 4;
const DELETES: u32 = 8;

/// One case of `MergedFieldsFixture`: its name, its features and the number
/// of documents of each segment.
#[derive(Debug, Clone, Copy)]
struct Case {
    name: &'static str,
    features: u32,
    sizes: &'static [i32],
}

const CASE_POSTINGS: Case = Case {
    name: "postings",
    features: POSTINGS,
    sizes: &[4200, 4200],
};
const CASE_POINTS: Case = Case {
    name: "points",
    features: POINTS,
    sizes: &[700, 700],
};
const CASE_POINTS_DEFAULT: Case = Case {
    name: "points-default",
    features: POINTS,
    sizes: &[700, 700, 50],
};
const CASE_DOC_VALUES: Case = Case {
    name: "docvalues",
    features: DOC_VALUES,
    sizes: &[9000, 9000],
};
const CASE_DELETES: Case = Case {
    name: "deletes",
    features: POSTINGS | DELETES,
    sizes: &[500, 500, 50],
};
const CASE_COMBINED: Case = Case {
    name: "combined",
    features: POSTINGS | POINTS | DOC_VALUES | DELETES,
    sizes: &[6500, 6500, 50],
};

/// The field type of `off`: `MergedFieldsFixture.OFFSETS_TYPE`.
fn offsets_type() -> FieldType {
    let mut field_type = FieldType::new();
    field_type.set_tokenized(true).unwrap();
    field_type
        .set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
        .unwrap();
    field_type.freeze();
    field_type
}

/// Whether document `i` of segment `seg` is deleted in the cases with
/// deletions.
fn is_deleted(seg: usize, i: i32) -> bool {
    i % 6 == 0 || seg == 2
}

fn bytes(value: &str) -> BytesRef {
    BytesRef::new(value.as_bytes().to_vec())
}

/// The value of numeric doc-values field `n` for document `i` of segment `seg`.
fn n_value(seg: usize, i: i32) -> i64 {
    if seg == 0 {
        i64::from(i % 8)
    } else {
        (i64::from(i) * 1_000_003) % 1_000_000_000
    }
}

/// Document `i` of segment `seg`: `MergedFieldsFixture.doc(int, int, int)`,
/// field for field and in the same order, so that field numbers agree.
fn doc(features: u32, seg: usize, i: i32) -> Document {
    let mut d = Document::new();
    if features & POSTINGS != 0 {
        d.add(Box::new(
            StringField::new("id", format!("id{i:05}"), Store::NO).unwrap(),
        ));
        let mut body = format!("all m{} w{} x{i}", i % 3, i % 97);
        for _ in 0..=(i % 5) {
            body.push_str(" rep");
        }
        d.add(Box::new(TextField::new("body", body, Store::NO).unwrap()));
        d.add(Box::new(
            StringField::new("tag", format!("t{}", i % 50), Store::NO).unwrap(),
        ));
        d.add(Box::new(
            Field::new("off", format!("alpha beta{} alpha", i % 13), offsets_type()).unwrap(),
        ));
    }
    if features & POINTS != 0 && seg < 2 {
        d.add(Box::new(
            IntPoint::new("i1", &[((i * 7919) % 2003) - 1000]).unwrap(),
        ));
        d.add(Box::new(
            LongPoint::new(
                "l1",
                &[(i64::from(i) * 1_000_000_007) % 4_000_000_000 - 2_000_000_000],
            )
            .unwrap(),
        ));
        d.add(Box::new(
            IntPoint::new("p2", &[(i * 31) % 257, (i * 17) % 263]).unwrap(),
        ));
        if i % 2 == 0 {
            d.add(Box::new(
                IntPoint::new("p2", &[(i * 13) % 251, (i * 7) % 241]).unwrap(),
            ));
        }
        if i % 10 == 0 {
            d.add(Box::new(IntPoint::new("sparse", &[i]).unwrap()));
        }
    }
    if features & DOC_VALUES != 0 {
        d.add(Box::new(NumericDocValuesField::new("n", n_value(seg, i))));
        if i % 7 == 0 {
            d.add(Box::new(NumericDocValuesField::new(
                "nsparse",
                i64::from(i) * 3,
            )));
        }
        d.add(Box::new(BinaryDocValuesField::new(
            "b",
            bytes(&format!("b{i}")),
        )));
        d.add(Box::new(SortedDocValuesField::new(
            "s",
            bytes(&format!("s{}", i % 300)),
        )));
        d.add(Box::new(SortedSetDocValuesField::new(
            "ss",
            bytes(&format!("a{}", i % 40)),
        )));
        d.add(Box::new(SortedSetDocValuesField::new(
            "ss",
            bytes(&format!("b{}", i % 1100)),
        )));
        d.add(Box::new(SortedNumericDocValuesField::new(
            "sn",
            i64::from(i % 5),
        )));
        d.add(Box::new(SortedNumericDocValuesField::new(
            "sn",
            -i64::from(i),
        )));
    }
    if features & DELETES != 0 {
        let deleted = is_deleted(seg, i);
        d.add(Box::new(
            StringField::new("del", if deleted { "yes" } else { "no" }.into(), Store::NO).unwrap(),
        ));
        d.add(Box::new(
            StoredField::new_string("sv", format!("v{i}")).unwrap(),
        ));
        if deleted && seg < 2 {
            d.add(Box::new(
                StringField::new("qt", format!("qt{}", i % 4), Store::NO).unwrap(),
            ));
            d.add(Box::new(IntPoint::new("q", &[i]).unwrap()));
            d.add(Box::new(NumericDocValuesField::new("qdv", i64::from(i))));
            d.add(Box::new(BinaryDocValuesField::new(
                "qb",
                bytes(&format!("qb{i}")),
            )));
            d.add(Box::new(SortedDocValuesField::new(
                "qs",
                bytes(&format!("qs{i}")),
            )));
            d.add(Box::new(SortedSetDocValuesField::new(
                "qss",
                bytes(&format!("qss{i}")),
            )));
            d.add(Box::new(SortedNumericDocValuesField::new(
                "qsn",
                i64::from(i),
            )));
        }
    }
    if features == POINTS && seg == 2 {
        d.add(Box::new(
            StringField::new("id", format!("id{i:05}"), Store::NO).unwrap(),
        ));
    }
    d
}

/// The `(segment, document)` of every document that survives the case's
/// deletions, in the order the documents were added.
///
/// A merge orders its segments as the merge policy lists them —
/// `TieredMergePolicy` lists the larger first — so the merged order is not
/// necessarily this one; the byte comparison pins Lucene's, and the checks
/// below identify each merged document by its own values.
fn live_docs(case: Case) -> Vec<(usize, i32)> {
    let mut out = Vec::new();
    let mut i = 0;
    for (seg, &size) in case.sizes.iter().enumerate() {
        for _ in 0..size {
            if case.features & DELETES == 0 || !is_deleted(seg, i) {
                out.push((seg, i));
            }
            i += 1;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The two merges
// ---------------------------------------------------------------------------

fn ensure_codec() {
    let _ = rucene::codecs::register_codec("Lucene104", Lucene104Codec::new());
}

/// Writes the case's segments with this port's `IndexWriter`, commits its
/// deletions, and force-merges them into one segment, as the fixture does.
fn rust_merge(case: Case) -> Arc<dyn Directory> {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    let mut i = 0;
    for (seg, &size) in case.sizes.iter().enumerate() {
        for _ in 0..size {
            writer.add_document(&doc(case.features, seg, i)).unwrap();
            i += 1;
        }
        writer.commit().unwrap();
    }
    if case.features & DELETES != 0 {
        writer
            .delete_documents_by_terms(vec![Term::new("del", bytes("yes"))])
            .unwrap();
        writer.commit().unwrap();
    }
    let before = SegmentInfos::read_latest_commit(Arc::clone(&dir))
        .unwrap()
        .size();
    let expected_before = if case.features & DELETES != 0 {
        // The third segment is fully deleted, and dropped.
        2
    } else {
        case.sizes.len()
    };
    assert_eq!(
        before, expected_before,
        "a fully deleted segment is dropped when its deletions are committed, and nothing \
         merges before force_merge"
    );
    writer.force_merge(1).unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();
    dir
}

/// What `MergedFieldsFixture` printed about Lucene's merged segment.
#[derive(Debug)]
struct JavaMerge {
    dir: PathBuf,
    name: String,
    id: [u8; 16],
    max_doc: i32,
    files: BTreeMap<String, u64>,
    /// Keeps the output directory alive as long as this value.
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

/// Runs `MergedFieldsFixture <dir> <case>` and parses what it prints.
fn java_merge(case: Case) -> JavaMerge {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         MergedFieldsFixture.java",
    );
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join(case.name);
    let output = Command::new("java")
        .arg("-cp")
        .arg(lucene_jar())
        .arg(&fixture)
        .arg(&dir)
        .arg(case.name)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "MergedFieldsFixture {} failed:\n{}",
        case.name,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let mut name = None;
    let mut id = None;
    let mut max_doc = None;
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
            assert_eq!(deletions, "0", "the merged segment has no deletion");
            max_doc = Some(count.parse().unwrap());
        } else if let Some(value) = line.strip_prefix("file ") {
            let (file, length) = value.split_once(' ').unwrap();
            files.insert(file.to_string(), length.parse().unwrap());
        } else if let Some(value) = line.strip_prefix("compound=") {
            assert_eq!(value, "false", "Lucene does not compound a full merge");
        } else if let Some(value) = line.strip_prefix("segments=") {
            assert_eq!(value, "1");
        }
    }
    JavaMerge {
        dir,
        name: name.expect("the fixture printed the segment name"),
        id: id.expect("the fixture printed the segment id"),
        max_doc: max_doc.expect("the fixture printed the document counts"),
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

/// Checks that the footer's checksum is the CRC-32 of everything before it,
/// as `CodecUtil.writeFooter` writes it.
fn assert_valid_footer(side: &str, name: &str, file: &[u8]) {
    assert!(file.len() >= 16, "{side} {name}: shorter than a footer");
    let footer = file.len() - 16;
    assert_eq!(
        &file[footer..footer + 4],
        &[0xc0, 0x28, 0x93, 0xe8],
        "{side} {name}: footer magic"
    );
    let body = &file[..file.len() - 8];
    let stored = u64::from_be_bytes(file[file.len() - 8..].try_into().unwrap());
    assert_eq!(
        stored,
        u64::from(crc32fast::hash(body)),
        "{side} {name}: the footer checksum must cover the file"
    );
}

/// Replaces every occurrence of `from` in `bytes` with `to`, returning how
/// many there were.
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

/// The byte comparison of one merged file, for the report.
#[derive(Debug)]
struct Compared {
    name: String,
    lucene: usize,
    rucene: usize,
}

/// Asserts that the merged segment `dir` holds holds exactly the files of
/// Lucene's, and that each one but the `.si` has Lucene's bytes, segment id
/// and checksum aside.
fn assert_same_merged_files(dir: &Arc<dyn Directory>, java: &JavaMerge) -> Vec<Compared> {
    let infos = SegmentInfos::read_latest_commit(Arc::clone(dir)).unwrap();
    assert_eq!(infos.size(), 1, "one segment after force_merge(1)");
    let merged = infos.info(0);
    assert_eq!(merged.info.name, java.name, "the merged segment's name");
    assert_eq!(merged.info.max_doc().unwrap(), java.max_doc, "maxDoc");
    assert_eq!(merged.get_del_count(), 0, "delCount");
    assert!(!merged.info.get_use_compound_file(), "compound file");
    let rust_id = merged.info.id();
    let files: BTreeSet<String> = merged.files().unwrap().into_iter().collect();
    assert_eq!(
        files,
        java.files.keys().cloned().collect::<BTreeSet<_>>(),
        "the merged segment's files"
    );

    let mut compared = Vec::new();
    for name in &files {
        if name.ends_with(".si") {
            continue;
        }
        let mut expected = std::fs::read(java.dir.join(name)).unwrap();
        let actual = read_file(dir.as_ref(), name);
        assert_valid_footer("Lucene", name, &expected);
        assert_valid_footer("Rucene", name, &actual);
        let ids = replace_id(&mut expected, &java.id, &rust_id);
        assert!(ids >= 1, "{name}: Lucene's file carries the segment id");
        let report = Compared {
            name: name.clone(),
            lucene: expected.len(),
            rucene: actual.len(),
        };
        let body = actual.len().min(expected.len()) - 8;
        if actual.len() != expected.len() || actual[..body] != expected[..body] {
            let first = (0..body)
                .find(|&at| actual[at] != expected[at])
                .unwrap_or(body);
            panic!(
                "{name}: Lucene has {} bytes, Rucene {}; they differ from offset {first}: \
                 Lucene {:02x?}, Rucene {:02x?}",
                expected.len(),
                actual.len(),
                &expected[first..(first + 16).min(expected.len())],
                &actual[first..(first + 16).min(actual.len())]
            );
        }
        compared.push(report);
    }
    compared
}

fn print_comparison(case: Case, compared: &[Compared]) {
    for file in compared {
        println!(
            "{} {} lucene={} rucene={} equal=true",
            case.name, file.name, file.lucene, file.rucene
        );
    }
}

// ---------------------------------------------------------------------------
// What a reader sees in the merged segment
// ---------------------------------------------------------------------------

fn searcher(dir: &Arc<dyn Directory>) -> (Arc<dyn DirectoryReader>, IndexSearcher) {
    let reader = directory_reader::open(Arc::clone(dir)).unwrap();
    assert_eq!(Arc::clone(&reader).leaves().len(), 1, "one segment");
    let searcher =
        IndexSearcher::new(Arc::clone(&reader) as Arc<dyn rucene::index::IndexReader>).unwrap();
    (reader, searcher)
}

fn term_query(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(Term::new(field, bytes(text))))
}

/// Every live document's `id` term finds exactly one document, every merged
/// document is found by exactly one, and the terms every document has find
/// every document. Returns the document number of each merged document.
fn assert_every_term_and_document(dir: &Arc<dyn Directory>, case: Case) -> Vec<i32> {
    let live = live_docs(case);
    let (reader, searcher) = searcher(dir);
    assert_eq!(
        (reader.num_docs(), reader.max_doc()),
        (live.len() as i32, live.len() as i32)
    );
    let mut ids = vec![-1; live.len()];
    for &(_, i) in &live {
        let hits = searcher
            .search(term_query("id", &format!("id{i:05}")), 2)
            .unwrap();
        let docs: Vec<i32> = hits.score_docs.iter().map(|hit| hit.doc).collect();
        assert_eq!(docs.len(), 1, "id{i:05} finds one document: {docs:?}");
        assert_eq!(
            ids[docs[0] as usize], -1,
            "two ids find document {}",
            docs[0]
        );
        ids[docs[0] as usize] = i;
    }
    assert_eq!(
        searcher.count(term_query("body", "all")).unwrap(),
        live.len() as i32
    );
    for k in 0..50 {
        let expected = live.iter().filter(|&&(_, i)| i % 50 == k).count() as i32;
        assert_eq!(
            searcher.count(term_query("tag", &format!("t{k}"))).unwrap(),
            expected,
            "t{k}"
        );
    }
    assert_eq!(
        searcher.count(term_query("off", "alpha")).unwrap(),
        live.len() as i32
    );
    // Frequencies and positions survive the merge: document `i` has `rep`
    // `i % 5 + 1` times, at positions 4 onwards.
    let leaf = Arc::clone(&reader).leaves()[0].leaf_reader();
    let terms = leaf.terms("body").unwrap().expect("body is indexed");
    let mut terms_enum = terms.iterator().unwrap();
    assert!(terms_enum.seek_exact(&bytes("rep")).unwrap());
    assert_eq!(terms_enum.doc_freq().unwrap(), live.len() as i32);
    let mut postings = terms_enum
        .postings(None, rucene::index::POSTINGS_ENUM_POSITIONS)
        .unwrap();
    for (doc, &i) in ids.iter().enumerate() {
        assert_eq!(postings.next_doc().unwrap(), doc as i32);
        let freq = postings.freq().unwrap();
        assert_eq!(freq, i % 5 + 1, "the frequency of rep in document {i}");
        let positions: Vec<i32> = (0..freq)
            .map(|_| postings.next_position().unwrap())
            .collect();
        assert_eq!(positions, (4..4 + freq).collect::<Vec<_>>(), "document {i}");
    }
    assert_eq!(postings.next_doc().unwrap(), NO_MORE_DOCS);

    if case.features & DELETES != 0 {
        assert_eq!(searcher.count(term_query("del", "yes")).unwrap(), 0);
        assert!(
            leaf.terms("qt").unwrap().is_none(),
            "every document indexing qt is deleted"
        );
        let stored = reader.stored_fields().unwrap();
        for (doc, &i) in ids.iter().enumerate() {
            let value = stored
                .document(doc as i32)
                .unwrap()
                .get_field("sv")
                .and_then(|field| field.string_value())
                .unwrap();
            assert_eq!(value, format!("v{i}"), "the stored value of document {doc}");
        }
    }
    ids
}

/// The segment document `i` was added to.
fn segment_of(case: Case, i: i32) -> usize {
    let mut end = 0;
    for (seg, &size) in case.sizes.iter().enumerate() {
        end += size;
        if i < end {
            return seg;
        }
    }
    panic!("document {i} is not in case {}", case.name)
}

/// Range queries over every point field find exactly the documents that hold
/// a matching point.
fn assert_every_point(dir: &Arc<dyn Directory>, case: Case) {
    let live = live_docs(case);
    let points: Vec<(usize, i32)> = live.iter().copied().filter(|&(seg, _)| seg < 2).collect();
    let (_reader, searcher) = searcher(dir);
    let count = |query: Arc<dyn Query>| searcher.count(query).unwrap() as usize;

    let i1 = |i: i32| ((i * 7919) % 2003) - 1000;
    assert_eq!(
        count(IntPoint::new_range_query("i1", &[-1000], &[1002]).unwrap()),
        points.len()
    );
    assert_eq!(
        count(IntPoint::new_range_query("i1", &[-100], &[250]).unwrap()),
        points
            .iter()
            .filter(|&&(_, i)| (-100..=250).contains(&i1(i)))
            .count()
    );
    let l1 = |i: i32| (i64::from(i) * 1_000_000_007) % 4_000_000_000 - 2_000_000_000;
    assert_eq!(
        count(LongPoint::new_range_query("l1", &[-500_000_000], &[1_500_000_000]).unwrap()),
        points
            .iter()
            .filter(|&&(_, i)| (-500_000_000..=1_500_000_000).contains(&l1(i)))
            .count()
    );
    let in_box = |x: i32, y: i32| (40..=200).contains(&x) && (10..=120).contains(&y);
    assert_eq!(
        count(IntPoint::new_range_query("p2", &[40, 10], &[200, 120]).unwrap()),
        points
            .iter()
            .filter(|&&(_, i)| {
                in_box((i * 31) % 257, (i * 17) % 263)
                    || (i % 2 == 0 && in_box((i * 13) % 251, (i * 7) % 241))
            })
            .count()
    );
    assert_eq!(
        count(IntPoint::new_range_query("sparse", &[i32::MIN], &[i32::MAX]).unwrap()),
        points.iter().filter(|&&(_, i)| i % 10 == 0).count()
    );
    if case.features & DELETES != 0 {
        assert_eq!(
            count(IntPoint::new_range_query("q", &[i32::MIN], &[i32::MAX]).unwrap()),
            0,
            "every document with a point in q is deleted"
        );
    }
}

/// Every doc-values field of the merged segment holds every live document's
/// values, from both source segments; each merged document is identified by
/// its binary value, `b<i>`.
fn assert_every_doc_value(dir: &Arc<dyn Directory>, case: Case) {
    let live = live_docs(case);
    let (reader, _searcher) = searcher(dir);
    let leaf = Arc::clone(&reader).leaves()[0].leaf_reader();

    let mut ids = Vec::with_capacity(live.len());
    let mut binary = leaf.get_binary_doc_values("b").unwrap().unwrap();
    for doc in 0..live.len() as i32 {
        assert_eq!(binary.next_doc().unwrap(), doc, "every document has b");
        let value = binary.binary_value().unwrap();
        let text = std::str::from_utf8(&value.bytes[value.offset..value.offset + value.length])
            .unwrap()
            .to_string();
        ids.push(text.strip_prefix('b').unwrap().parse::<i32>().unwrap());
    }
    assert_eq!(binary.next_doc().unwrap(), NO_MORE_DOCS);
    let mut sorted_ids = ids.clone();
    sorted_ids.sort_unstable();
    assert_eq!(
        sorted_ids,
        live.iter().map(|&(_, i)| i).collect::<Vec<_>>(),
        "the merged segment holds every live document once"
    );

    let mut numeric = leaf.get_numeric_doc_values("n").unwrap().unwrap();
    let mut sparse = leaf.get_numeric_doc_values("nsparse").unwrap().unwrap();
    let mut sorted = leaf.get_sorted_doc_values("s").unwrap().unwrap();
    let mut sorted_set = leaf.get_sorted_set_doc_values("ss").unwrap().unwrap();
    let mut sorted_numeric = leaf.get_sorted_numeric_doc_values("sn").unwrap().unwrap();
    for (doc, &i) in ids.iter().enumerate() {
        let doc = doc as i32;
        let seg = segment_of(case, i);
        assert_eq!(numeric.next_doc().unwrap(), doc);
        assert_eq!(numeric.long_value().unwrap(), n_value(seg, i), "n of {i}");
        if i % 7 == 0 {
            assert_eq!(sparse.advance(doc).unwrap(), doc);
            assert_eq!(sparse.long_value().unwrap(), i64::from(i) * 3);
        }
        assert_eq!(sorted.next_doc().unwrap(), doc);
        assert_eq!(
            sorted.lookup_ord(sorted.ord_value().unwrap()).unwrap(),
            bytes(&format!("s{}", i % 300))
        );
        assert_eq!(sorted_set.next_doc().unwrap(), doc);
        assert_eq!(sorted_set.doc_value_count().unwrap(), 2);
        let mut terms = Vec::new();
        for _ in 0..2 {
            let ord = sorted_set.next_ord().unwrap();
            terms.push(sorted_set.lookup_ord(ord).unwrap());
        }
        assert_eq!(
            terms,
            vec![
                bytes(&format!("a{}", i % 40)),
                bytes(&format!("b{}", i % 1100))
            ]
        );
        assert_eq!(sorted_numeric.next_doc().unwrap(), doc);
        assert_eq!(sorted_numeric.doc_value_count().unwrap(), 2);
        let values = [
            sorted_numeric.next_value().unwrap(),
            sorted_numeric.next_value().unwrap(),
        ];
        assert_eq!(values, [-i64::from(i), i64::from(i % 5)]);
    }
    assert_eq!(numeric.next_doc().unwrap(), NO_MORE_DOCS);
    if case.features & DELETES != 0 {
        // Every document with a value in these fields is deleted.
        assert_eq!(
            leaf.get_numeric_doc_values("qdv")
                .unwrap()
                .unwrap()
                .next_doc()
                .unwrap(),
            NO_MORE_DOCS
        );
        assert_eq!(
            leaf.get_binary_doc_values("qb")
                .unwrap()
                .unwrap()
                .next_doc()
                .unwrap(),
            NO_MORE_DOCS
        );
        assert_eq!(
            leaf.get_sorted_doc_values("qs")
                .unwrap()
                .unwrap()
                .next_doc()
                .unwrap(),
            NO_MORE_DOCS
        );
        assert_eq!(
            leaf.get_sorted_set_doc_values("qss")
                .unwrap()
                .unwrap()
                .next_doc()
                .unwrap(),
            NO_MORE_DOCS
        );
        assert_eq!(
            leaf.get_sorted_numeric_doc_values("qsn")
                .unwrap()
                .unwrap()
                .next_doc()
                .unwrap(),
            NO_MORE_DOCS
        );
    }
}

/// Runs one case on both sides and compares the merged segments.
/// Runs one case on both sides, checks what a reader finds in this port's
/// merged segment with `check`, and then compares the two merged segments.
///
/// The reader's checks come first so that a merge that loses data says which
/// data; the byte comparison then pins everything else.
fn merge_check_and_compare(case: Case, check: impl FnOnce(&Arc<dyn Directory>)) {
    let java = java_merge(case);
    let dir = rust_merge(case);
    check(&dir);
    let compared = assert_same_merged_files(&dir, &java);
    print_comparison(case, &compared);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Two commits of indexed text, string and offsets fields merge into a segment
/// that reopens and finds every term and document, and whose terms, postings
/// and norms files are Lucene's (rmp #217).
#[test]
fn merged_postings_are_lucenes_and_find_every_term() {
    merge_check_and_compare(CASE_POSTINGS, |dir| {
        assert_every_term_and_document(dir, CASE_POSTINGS);
    });
}

/// Points of one and two dimensions, multi-valued and sparse, merge through
/// `BKDWriter.merge` and the default `mergeOneField` into Lucene's
/// `.kdm`/`.kdi`/`.kdd`, and range queries find every document (rmp #218).
#[test]
fn merged_points_are_lucenes_and_found_by_range_queries() {
    merge_check_and_compare(CASE_POINTS, |dir| assert_every_point(dir, CASE_POINTS));
}

/// A source segment without points has no points reader, which sends
/// `Lucene90PointsWriter.merge` down the default `PointsWriter.merge` for
/// every field, one-dimensional ones included; the files are still Lucene's
/// (rmp #218).
#[test]
fn a_source_segment_without_points_takes_the_default_points_merge() {
    merge_check_and_compare(CASE_POINTS_DEFAULT, |dir| {
        assert_every_point(dir, CASE_POINTS_DEFAULT)
    });
}

/// Per-field doc values of every type merge through `PerFieldDocValuesFormat`
/// into Lucene's files, and the merged segment holds both segments' values
/// (rmp #219).
///
/// Field `n` holds 18000 values, more than one numeric block (16384), but
/// per-block widths would not save the 10% Lucene requires
/// (`Lucene90DocValuesConsumer.java:486-497`), so Lucene writes it as one
/// block: the multi-block numeric encoding is not exercised here.
#[test]
fn merged_per_field_doc_values_are_lucenes_and_hold_every_value() {
    merge_check_and_compare(CASE_DOC_VALUES, |dir| {
        assert_every_doc_value(dir, CASE_DOC_VALUES)
    });
}

/// Committed deletions, and a third segment whose every document is deleted,
/// merge into a segment that holds exactly the live documents and is Lucene's
/// byte for byte (rmp #220).
///
/// Field `qt` is indexed only in deleted documents: its terms all lose their
/// documents, so the block-tree writer writes no metadata for it
/// (`TermsWriter.finish`, `Lucene103BlockTreeTermsWriter.java:1060-1113`) and
/// the merged segment answers no terms for it.
#[test]
fn force_merge_with_committed_deletions_keeps_the_live_documents() {
    merge_check_and_compare(CASE_DELETES, |dir| {
        assert_every_term_and_document(dir, CASE_DELETES);
    });
}

/// Every field of the four tasks in one index, with deletions: the merged
/// segment is Lucene's, file for file.
#[test]
fn a_merge_of_every_kind_of_field_with_deletions_is_lucenes() {
    merge_check_and_compare(CASE_COMBINED, |dir| {
        assert_every_term_and_document(dir, CASE_COMBINED);
        assert_every_point(dir, CASE_COMBINED);
        assert_every_doc_value(dir, CASE_COMBINED);
    });
}

// ---------------------------------------------------------------------------
// A corrupt bit-set postings block
// ---------------------------------------------------------------------------

/// Writes 3000 documents whose `del` term `no` skips every sixth one, so its
/// first full postings block is a bit set of five words, and returns a copy of
/// the index in which `patch` has rewritten those five words.
///
/// The block is found by its first word — documents 1 to 63 but the
/// multiples of six — preceded by its negative word count.
fn index_with_patched_bit_set_block(patch: impl Fn(&mut [u8])) -> Arc<dyn Directory> {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    for i in 0..3000 {
        let mut doc = Document::new();
        let value = if i % 6 == 0 { "yes" } else { "no" };
        doc.add(Box::new(
            StringField::new("del", value.to_string(), Store::NO).unwrap(),
        ));
        writer.add_document(&doc).unwrap();
    }
    writer.commit().unwrap();
    writer.close().unwrap();

    let mut first_word = 0u64;
    for bit in (0..64).filter(|bit| bit % 6 != 0) {
        first_word |= 1 << bit;
    }
    let pattern = first_word.to_le_bytes();
    let copy: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut patched = 0;
    for name in dir.list_all().unwrap() {
        let mut bytes = read_file(dir.as_ref(), &name);
        if patched == 0 && (name.ends_with(".cfs") || name.ends_with(".doc")) {
            if let Some(at) = (1..bytes.len().saturating_sub(8))
                .find(|&at| bytes[at..at + 8] == pattern && (bytes[at - 1] as i8) == -5)
            {
                patch(&mut bytes[at..at + 5 * 8]);
                patched += 1;
            }
        }
        let mut out = copy.create_output(&name, &*DEFAULT_IO_CONTEXT).unwrap();
        out.write_bytes(&bytes, 0, bytes.len()).unwrap();
        out.close().unwrap();
    }
    assert_eq!(patched, 1, "the bit-set block was found");
    copy
}

/// The postings of `del:no` in `dir`, with `flags`.
fn postings_of_no(
    dir: &Arc<dyn Directory>,
    flags: i32,
) -> Box<dyn rucene::index::postings_enum::PostingsEnum> {
    let reader = directory_reader::open(Arc::clone(dir)).unwrap();
    let leaf = Arc::clone(&reader).leaves()[0].leaf_reader();
    let terms = leaf.terms("del").unwrap().unwrap();
    let mut terms_enum = terms.iterator().unwrap();
    assert!(terms_enum.seek_exact(&bytes("no")).unwrap());
    terms_enum.postings(None, flags).unwrap()
}

/// A bit-set postings block whose words no longer hold the documents its
/// skip data promises is a corrupt-index error, never a panic. Java only
/// asserts that the bit set has a next document
/// (`Lucene104PostingsReader.java:916, 946`), and indexes `freqBuffer` with a
/// position such a block can push past the block
/// (`Lucene104PostingsReader.java:590`).
#[test]
fn a_corrupt_bit_set_postings_block_is_an_error_not_a_panic() {
    // Every word cleared: the block has no document at all.
    let cleared = index_with_patched_bit_set_block(|words| words.fill(0));
    let mut postings = postings_of_no(&cleared, rucene::index::POSTINGS_ENUM_NONE);
    let error = loop {
        match postings.next_doc() {
            Ok(NO_MORE_DOCS) => panic!("a cleared block cannot be iterated to the end"),
            Ok(_) => continue,
            Err(error) => break error,
        }
    };
    assert!(
        matches!(error, rucene::error::LuceneError::CorruptIndex(_)),
        "{error:?}"
    );
    let mut postings = postings_of_no(&cleared, rucene::index::POSTINGS_ENUM_FREQS);
    let error = postings.advance(100).unwrap_err();
    assert!(
        matches!(error, rucene::error::LuceneError::CorruptIndex(_)),
        "{error:?}"
    );

    // Every word set: more documents than a block holds.
    let filled = index_with_patched_bit_set_block(|words| words.fill(0xff));
    let mut postings = postings_of_no(&filled, rucene::index::POSTINGS_ENUM_FREQS);
    let error = loop {
        match postings.next_doc() {
            Ok(NO_MORE_DOCS) => panic!("the frequency of a 257th document was answered"),
            Ok(_) => {
                if let Err(error) = postings.freq() {
                    break error;
                }
            }
            Err(error) => break error,
        }
    };
    assert!(
        matches!(error, rucene::error::LuceneError::CorruptIndex(_)),
        "{error:?}"
    );
}

/// `advance` into a bit-set postings block finds a document's frequency
/// through the cumulative pop counts of the block's words, which Java keeps in
/// `docBuffer` (`Lucene104PostingsReader.java:312, 614-622, 949-955`).
#[test]
fn advance_into_a_bit_set_block_reads_each_documents_frequency() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    for i in 0..3000 {
        let mut doc = Document::new();
        // Every sixth document lacks `no`, so its first full block is a bit set.
        let body = if i % 6 == 0 {
            "yes".to_string()
        } else {
            vec!["no"; (i % 3) as usize + 1].join(" ")
        };
        doc.add(Box::new(TextField::new("body", body, Store::NO).unwrap()));
        writer.add_document(&doc).unwrap();
    }
    writer.commit().unwrap();
    writer.close().unwrap();

    let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
    let leaf = Arc::clone(&reader).leaves()[0].leaf_reader();
    let terms = leaf.terms("body").unwrap().unwrap();
    let mut terms_enum = terms.iterator().unwrap();
    assert!(terms_enum.seek_exact(&bytes("no")).unwrap());
    let mut postings = terms_enum
        .postings(None, rucene::index::POSTINGS_ENUM_FREQS)
        .unwrap();
    for target in [10, 77, 150, 251, 299, 700, 1500, 2999] {
        let doc = postings.advance(target).unwrap();
        let expected = if target % 6 == 0 { target + 1 } else { target };
        assert_eq!(doc, expected, "advance({target})");
        assert_eq!(
            postings.freq().unwrap(),
            doc % 3 + 1,
            "the frequency of {doc}"
        );
    }
}
