//! The ten nested Lucene types of Sprint 8's last port effort, driven through
//! the public paths that reach them and compared with Apache Lucene Core
//! 10.5.0.
//!
//! | Test | Lucene type | Public path |
//! | --- | --- | --- |
//! | `binary_index_sort_*` | `IndexSorter.BinarySorter`, `IndexSorter.BinaryDocValuesProvider`, `FreqProxTermsWriter.SortingTerms` | `IndexWriterConfig.setIndexSort(new Sort(new BinarySortField(..)))`, then a flush |
//! | `feature_sort_*` | `FeatureSortField.FeatureComparator` | a search sorted by `FeatureField.newFeatureSort` |
//! | `tiered_merge_score_*` | `TieredMergePolicy.MergeScore` | `TieredMergePolicy.findMerges`, whose info stream prints each score |
//! | `single_value_dv_update_*` | `NumericDocValuesFieldUpdates.SingleValueNumericDocValuesFieldUpdates` | `IndexWriter.updateNumericDocValue` / `updateDocValues`, then a commit |
//! | `merge_observer_*` | `MergePolicy.MergeObserver` | `IndexWriter.forceMergeDeletes(boolean)` |
//! | `delegating_cache_helper_*` | `FilterDirectoryReader.DelegatingCacheHelper` | `SoftDeletesDirectoryReaderWrapper` |
//! | `filter_fields_*`, `filter_postings_enum_*` | `FilterLeafReader.FilterFields`, `FilterLeafReader.FilterPostingsEnum` | a filter over a real segment |
//!
//! # The oracle
//!
//! Each Lucene-backed test runs `NestedTypesFixture` from
//! `tests/fixtures/java-codec-harness` on a real Lucene 10.5.0 (`java -cp
//! lucene-core-10.5.0.jar NestedTypesFixture.java <dir> <case> [<variant>]`)
//! and compares what it prints, or the files it leaves, with what this port
//! does with the same input. Files are compared **byte for byte**; a segment id
//! is random, so Lucene's id is replaced by this port's wherever it occurs, and
//! the footer checksum, which covers the id, is left out of the comparison and
//! verified on its own on both sides.
//!
//! A missing JDK or Lucene jar is a hard failure, not a skip: these tests have
//! nothing to assert without the reference implementation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rucene::codecs::{Codec, Lucene104Codec};
use rucene::document::feature_field::FeatureField;
use rucene::document::{
    BinaryDocValuesField, Document, Field, FieldType, IntPoint, NumericDocValuesField,
    NumericValue, SortedDocValuesField, SortedNumericDocValuesField, SortedSetDocValuesField,
    Store, StoredField, StringField, TextField,
};
use rucene::index::filter_directory_reader::{
    DelegatingCacheHelper, DirectoryReaderWrapper, SoftDeletesDirectoryReaderWrapper,
};
use rucene::index::filter_reader::{FilterFields, FilterPostingsEnum};
use rucene::index::index_reader::{CacheHelper, CacheKey, ClosedListener, IndexReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::{
    MergeContext, MergePolicy, MergeTrigger, NoMergePolicy, TieredMergePolicy,
};
use rucene::index::merge_scheduler::{NoMergeScheduler, SerialMergeScheduler};
use rucene::index::point_values::{IntersectVisitor, Relation};
use rucene::index::terms::Term;
use rucene::index::{
    directory_reader, DirectoryReader, DocValuesType, FieldInfo, Fields, IndexOptions,
    PostingsEnum, SegmentCommitInfo, SegmentInfo, SegmentInfos, StoredFieldVisitor,
    StoredFieldVisitorStatus, POSTINGS_ENUM_ALL, POSTINGS_ENUM_NONE,
};
use rucene::search::{
    BinarySortField, CollectorManager, DocIdSetIterator, IndexSearcher, MatchAllDocsQuery,
    MissingValue, Query, Sort, SortField, SortFieldType, SortValue, TopFieldCollector,
    TopFieldCollectorManager, TopFieldDocs, NO_MORE_DOCS,
};
use rucene::store::{Directory, RamDirectory, DEFAULT_IO_CONTEXT};
use rucene::util::{BytesRef, InfoStream, StringHelper, Version};

// ---------------------------------------------------------------------------
// The oracle
// ---------------------------------------------------------------------------

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

/// What one run of `NestedTypesFixture` printed, and the index it left.
struct JavaRun {
    dir: PathBuf,
    stdout: String,
    _tmp: tempfile::TempDir,
}

impl JavaRun {
    /// The value of the first `key=value` line.
    fn value(&self, key: &str) -> &str {
        let prefix = format!("{key}=");
        self.stdout
            .lines()
            .find_map(|line| line.strip_prefix(&prefix))
            .unwrap_or_else(|| panic!("the fixture printed no `{key}=` line:\n{}", self.stdout))
    }

    /// Every line that starts with `prefix`, the prefix stripped.
    fn lines(&self, prefix: &str) -> Vec<String> {
        self.stdout
            .lines()
            .filter_map(|line| line.strip_prefix(prefix).map(str::to_string))
            .collect()
    }

    /// The segment id the fixture printed.
    fn id(&self) -> [u8; 16] {
        let value = self.value("id");
        let decoded: Vec<u8> = (0..value.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&value[at..at + 2], 16).unwrap())
            .collect();
        <[u8; 16]>::try_from(decoded).unwrap()
    }

    /// The files of the segment the fixture printed, with their lengths.
    fn files(&self) -> BTreeMap<String, u64> {
        self.lines("file ")
            .into_iter()
            .map(|line| {
                let (name, length) = line.split_once(' ').unwrap();
                (name.to_string(), length.parse().unwrap())
            })
            .collect()
    }
}

/// Runs `NestedTypesFixture <dir> <case> [<variant>]`.
fn run_java(case: &str, variant: &str) -> JavaRun {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         NestedTypesFixture.java",
    );
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("index");
    let mut command = Command::new("java");
    command
        .arg("-cp")
        .arg(lucene_jar())
        .arg(&fixture)
        .arg(&dir)
        .arg(case);
    if !variant.is_empty() {
        command.arg(variant);
    }
    let output = command
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "NestedTypesFixture {case} {variant} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    JavaRun {
        dir,
        stdout: String::from_utf8(output.stdout).unwrap(),
        _tmp: tmp,
    }
}

fn ensure_codec() {
    let _ = rucene::codecs::register_codec("Lucene104", Lucene104Codec::new());
}

fn bytes(value: &str) -> BytesRef {
    BytesRef::new(value.as_bytes().to_vec())
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

/// Compares Lucene's `name` with this port's, the segment id and the footer
/// checksum aside; returns both lengths, or the first differing offset.
fn compare_file(
    java: &JavaRun,
    dir: &dyn Directory,
    rust_id: &[u8; 16],
    name: &str,
) -> std::result::Result<(usize, usize), String> {
    let mut expected = std::fs::read(java.dir.join(name)).unwrap();
    let actual = read_file(dir, name);
    assert_valid_footer("Lucene", name, &expected);
    assert_valid_footer("Rucene", name, &actual);
    let ids = replace_id(&mut expected, &java.id(), rust_id);
    assert!(ids >= 1, "{name}: Lucene's file carries the segment id");
    let body = actual.len().min(expected.len()) - 8;
    if actual.len() != expected.len() || actual[..body] != expected[..body] {
        let first = (0..body)
            .find(|&at| actual[at] != expected[at])
            .unwrap_or(body);
        return Err(format!(
            "{name}: Lucene has {} bytes, Rucene {}; they differ from offset {first}: Lucene \
             {:02x?}, Rucene {:02x?}",
            expected.len(),
            actual.len(),
            &expected[first..(first + 16).min(expected.len())],
            &actual[first..(first + 16).min(actual.len())]
        ));
    }
    Ok((expected.len(), actual.len()))
}

fn open_reader(dir: &Arc<dyn Directory>) -> Arc<dyn DirectoryReader> {
    directory_reader::open(Arc::clone(dir)).unwrap()
}

// ---------------------------------------------------------------------------
// #203 and #204: a binary index sort, and the postings written through
// `SortingTerms`
// ---------------------------------------------------------------------------

const SORTED_DOCS: i32 = 600;

/// `NestedTypesFixture.key(int)`.
fn key(i: i32) -> Option<Vec<u8>> {
    if i % 7 == 3 {
        return None;
    }
    if i % 11 == 5 {
        return Some(Vec::new());
    }
    let a = ((i * 131) % 256) as u8;
    Some(match i % 3 {
        0 => vec![a],
        1 => vec![a, 0],
        _ => vec![a, 0xFF, (i % 2) as u8],
    })
}

/// `NestedTypesFixture.fieldType(IndexOptions)`: tokenized, no norms.
fn no_norms_type(options: IndexOptions) -> FieldType {
    let mut field_type = FieldType::new();
    field_type.set_tokenized(true).unwrap();
    field_type.set_omit_norms(true).unwrap();
    field_type.set_index_options(options).unwrap();
    field_type.freeze();
    field_type
}

/// `NestedTypesFixture.sortedDoc(int)`, field for field and in the same order.
fn sorted_doc(i: i32) -> Document {
    let mut d = Document::new();
    d.add(Box::new(
        StringField::new("id", format!("id{i:04}"), Store::NO).unwrap(),
    ));
    if let Some(key) = key(i) {
        d.add(Box::new(BinaryDocValuesField::new(
            "key",
            BytesRef::new(key),
        )));
    }
    d.add(Box::new(
        StringField::new("tag", format!("t{}", i % 17), Store::NO).unwrap(),
    ));
    let mut body = format!("all w{} x{i}", i % 31);
    for _ in 0..=(i % 4) {
        body.push_str(" rep");
    }
    d.add(Box::new(
        Field::new(
            "body",
            body,
            no_norms_type(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS),
        )
        .unwrap(),
    ));
    d.add(Box::new(
        Field::new(
            "freq",
            format!("f{} f{} g{}", i % 5, i % 5, i % 9),
            no_norms_type(IndexOptions::DOCS_AND_FREQS),
        )
        .unwrap(),
    ));
    d.add(Box::new(
        Field::new(
            "pos",
            format!("p{} q p{}", i % 3, i % 3),
            no_norms_type(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS),
        )
        .unwrap(),
    ));
    d
}

/// `NestedTypesFixture.binarySortField(String)`.
fn binary_sort_field(variant: &str) -> SortField {
    match variant {
        "asc" => BinarySortField::new("key", false),
        "desc" => BinarySortField::new("key", true),
        "last" => BinarySortField::with_missing_value("key", false, Some(MissingValue::StringLast)),
        "desc-last" => {
            BinarySortField::with_missing_value("key", true, Some(MissingValue::StringLast))
        }
        other => panic!("unknown variant {other}"),
    }
    .unwrap()
    .into_sort_field()
}

/// Flushes the 600 documents into one segment under the binary index sort.
fn rust_sorted_flush(variant: &str) -> Arc<dyn Directory> {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config.set_use_compound_file(false).unwrap();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .unwrap();
    config
        .set_index_sort(Sort::new_fields(vec![binary_sort_field(variant)]).unwrap())
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for i in 0..SORTED_DOCS {
        writer.add_document(&sorted_doc(i)).unwrap();
    }
    writer.commit().unwrap();
    writer.close().unwrap();
    dir
}

/// The document each `id` term landed on, read through the postings.
fn id_order(dir: &Arc<dyn Directory>) -> Vec<String> {
    let reader = open_reader(dir);
    let leaves = Arc::clone(&reader).leaves();
    assert_eq!(leaves.len(), 1, "one segment");
    let leaf = leaves[0].leaf_reader();
    let terms = leaf.terms("id").unwrap().expect("id is indexed");
    let mut terms_enum = terms.iterator().unwrap();
    let mut order = BTreeMap::new();
    while let Some(term) = terms_enum.next().unwrap() {
        let mut postings = terms_enum.postings(None, POSTINGS_ENUM_NONE).unwrap();
        let doc = postings.next_doc().unwrap();
        order.insert(doc, String::from_utf8(term.slice().to_vec()).unwrap());
    }
    order
        .into_iter()
        .map(|(doc, id)| format!("{doc} {id}"))
        .collect()
}

/// The postings files of a segment, which the sorted flush writes through
/// `SortingTerms`.
fn is_postings_file(name: &str) -> bool {
    ["doc", "pos", "pay", "psm", "tim", "tip", "tmd"]
        .iter()
        .any(|extension| name.ends_with(&format!("_Lucene104_0.{extension}")))
}

fn assert_binary_index_sort_flush(variant: &str) {
    let java = run_java("sorted-flush", variant);
    let dir = rust_sorted_flush(variant);
    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    assert_eq!(infos.size(), 1, "one flushed segment");
    let info = infos.info(0);
    assert_eq!(info.info.name, java.value("name"));
    assert_eq!(
        info.info.index_sort().to_string(),
        java.value("indexSort"),
        "the segment records the binary index sort"
    );

    // The postings files, byte for byte.
    let rust_id = info.info.id();
    let rust_files: BTreeSet<String> = info.files().unwrap().into_iter().collect();
    let java_files = java.files();
    let postings: Vec<&String> = java_files.keys().filter(|f| is_postings_file(f)).collect();
    assert_eq!(postings.len(), 7, "Lucene wrote seven postings files");
    for name in postings {
        assert!(rust_files.contains(name), "Rucene wrote {name}");
        let (lucene, rucene) = compare_file(&java, dir.as_ref(), &rust_id, name)
            .unwrap_or_else(|difference| panic!("{variant}: {difference}"));
        println!("sorted-flush {variant} {name} lucene={lucene} rucene={rucene} equal=true");
    }

    // The document order: `BinarySorter`'s comparator decides it, and the
    // postings carry it.
    assert_eq!(
        id_order(&dir),
        java.lines("doc "),
        "the {variant} binary sort orders the documents as Lucene does"
    );
}

#[test]
fn binary_index_sort_ascending_matches_lucene() {
    assert_binary_index_sort_flush("asc");
}

#[test]
fn binary_index_sort_descending_matches_lucene() {
    assert_binary_index_sort_flush("desc");
}

#[test]
fn binary_index_sort_missing_last_matches_lucene() {
    assert_binary_index_sort_flush("last");
}

#[test]
fn binary_index_sort_descending_missing_last_matches_lucene() {
    assert_binary_index_sort_flush("desc-last");
}

/// A delete-by-query buffered while the segment is still in memory is
/// private to it and limited to the documents added before it. The segment is
/// then sorted on flush, so the limit is in the pre-sort doc space and every
/// match is mapped back through the sort map before the comparison
/// (`FrozenBufferedUpdates.java:397-409`, with `ReadersAndUpdates.sortMap` set
/// by `IndexWriter.publishFlushedSegment`).
#[test]
fn binary_index_sort_segment_private_delete_matches_lucene() {
    let java = run_java("sorted-private-delete", "");
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config.set_use_compound_file(false).unwrap();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .unwrap();
    config
        .set_index_sort(Sort::new_fields(vec![binary_sort_field("asc")]).unwrap())
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for i in 0..120 {
        writer.add_document(&sorted_doc(i)).unwrap();
        if i == 40 {
            let query: Arc<dyn Query> = Arc::new(rucene::search::TermQuery::new(Term::new(
                "tag",
                bytes("t3"),
            )));
            writer.delete_documents_by_queries(vec![query]).unwrap();
        }
    }
    writer.commit().unwrap();
    writer.close().unwrap();

    let reader = open_reader(&dir);
    let leaves = Arc::clone(&reader).leaves();
    assert_eq!(leaves.len(), 1, "one segment");
    let leaf = leaves[0].leaf_reader();
    let live_docs = leaf.get_live_docs();
    let terms = leaf.terms("id").unwrap().expect("id is indexed");
    let mut terms_enum = terms.iterator().unwrap();
    let mut order = BTreeMap::new();
    while let Some(term) = terms_enum.next().unwrap() {
        let mut postings = terms_enum.postings(None, POSTINGS_ENUM_NONE).unwrap();
        let doc = postings.next_doc().unwrap();
        let live = live_docs.as_ref().is_none_or(|bits| bits.get(doc as usize));
        order.insert(
            doc,
            format!(
                "{} {}",
                String::from_utf8(term.slice().to_vec()).unwrap(),
                if live { "live" } else { "deleted" }
            ),
        );
    }
    assert_eq!(leaf.num_docs().to_string(), java.value("numDocs"));
    let lines: Vec<String> = order
        .into_iter()
        .map(|(doc, id)| format!("{doc} {id}"))
        .collect();
    assert_eq!(
        lines,
        java.lines("doc "),
        "the documents the private delete removed"
    );
}

/// `IndexWriterConfig.setIndexSort` accepts a binary sort because it has an
/// index sorter, and still refuses a feature sort, which has none.
#[test]
fn binary_index_sort_is_accepted_and_feature_sort_refused() {
    ensure_codec();
    let mut config = IndexWriterConfig::new();
    assert!(config
        .set_index_sort(Sort::new_fields(vec![binary_sort_field("asc")]).unwrap())
        .is_ok());
    let feature = FeatureField::new_feature_sort("features", "pagerank");
    let error = config
        .set_index_sort(Sort::new_fields(vec![feature]).unwrap())
        .expect_err("a feature sort cannot sort an index");
    assert_eq!(
        error.to_string(),
        rucene::error::LuceneError::IllegalArgument(
            "Cannot sort index with sort field <feature:\"features\" featureName=pagerank>"
                .to_string()
        )
        .to_string()
    );
}

/// A binary index sort over a field indexed with another doc-values type is
/// refused when the document is added, with Lucene's message.
#[test]
fn binary_index_sort_refuses_a_numeric_key() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config
        .set_index_sort(Sort::new_fields(vec![binary_sort_field("asc")]).unwrap())
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    let mut doc = Document::new();
    doc.add(Box::new(NumericDocValuesField::new("key", 3)));
    let error = writer
        .add_document(&doc)
        .expect_err("a numeric key cannot feed a binary sort");
    assert!(
        error.to_string().contains(
            "SortField <binary: \"key\"> expected field [key] to be BINARY but it is [NUMERIC]"
        ),
        "{error}"
    );
    writer.rollback().unwrap();
}

/// `BinarySorter` inherits `getComparableProviders`, which throws with the
/// sorter's class name, and implements `getComparableValues` instead.
#[test]
fn binary_sorter_has_no_comparable_providers() {
    let sorter = binary_sort_field("asc")
        .get_index_sorter()
        .expect("a binary sort has an index sorter");
    assert_eq!(sorter.get_provider_name(), "BinarySortField");
    let error = sorter
        .get_comparable_providers(&[])
        .err()
        .expect("BinarySorter does not implement getComparableProviders");
    assert!(
        matches!(error, rucene::error::LuceneError::UnsupportedOperation(ref message)
            if message == "org.apache.lucene.index.IndexSorter$BinarySorter does not implement \
                           getComparableProviders"),
        "{error:?}"
    );
    assert!(sorter.get_comparable_values(&[]).is_ok());
}

// ---------------------------------------------------------------------------
// #204: FeatureSortField.FeatureComparator
// ---------------------------------------------------------------------------

/// `NestedTypesFixture.featureValue(int)`.
fn feature_value(i: i32) -> f32 {
    let values = [
        1.5f32, 0.25, 8.0, 0.0, 8.0, 3.75, 100.0, 0.001, 8.000001, 42.0,
    ];
    values[(i % values.len() as i32) as usize]
}

/// `NestedTypesFixture.featureDoc(int, int)`.
fn feature_doc(i: i32, segment: usize) -> Document {
    let mut d = Document::new();
    d.add(Box::new(
        StringField::new("id", format!("f{i}"), Store::NO).unwrap(),
    ));
    if segment == 0 {
        if i == 3 {
            d.add(Box::new(
                FeatureField::new("features", "other", 2.0).unwrap(),
            ));
        } else {
            d.add(Box::new(
                FeatureField::new("features", "pagerank", feature_value(i)).unwrap(),
            ));
        }
    } else if segment == 2 {
        d.add(Box::new(
            FeatureField::new("features", "other", 1.0 + i as f32).unwrap(),
        ));
        if i % 3 == 0 {
            d.add(Box::new(
                FeatureField::new("features", "pagerank", 2.5 + i as f32).unwrap(),
            ));
        }
    }
    d
}

fn hit_lines(page: &str, top: &TopFieldDocs) -> Vec<String> {
    top.score_docs
        .iter()
        .map(|hit| {
            let value = match &hit.fields.as_ref().expect("sort values")[0] {
                SortValue::Float(value) => value.to_bits(),
                other => panic!("a feature sort value is a Float, got {other:?}"),
            };
            format!(
                "{page} doc={} value={value:x} score={:x}",
                hit.score_doc.doc,
                hit.score_doc.score.to_bits()
            )
        })
        .collect()
}

fn search_page(
    searcher: &IndexSearcher,
    sort: &Sort,
    num_hits: i32,
    after: Option<rucene::search::FieldDoc>,
) -> TopFieldDocs {
    let query: Arc<dyn Query> = Arc::new(MatchAllDocsQuery);
    let manager = TopFieldCollectorManager::new(sort.clone(), num_hits, after, i32::MAX).unwrap();
    // `IndexSearcher.search(Query, CollectorManager)` by hand — one collector,
    // as the reader is searched in one slice, then `reduce` — because this
    // port's `search_with_manager` requires a `Send` collector, which a
    // `TopFieldCollector` holding its comparators is not.
    let mut collector = CollectorManager::new_collector(&manager).unwrap();
    searcher
        .search_with_collector(Arc::clone(&query), &mut collector)
        .unwrap();
    let mut top = manager.reduce(vec![collector]).unwrap();
    let mut score_docs: Vec<_> = top.score_docs.iter().map(|hit| hit.score_doc).collect();
    TopFieldCollector::populate_scores(&mut score_docs, searcher, query).unwrap();
    for (hit, scored) in top.score_docs.iter_mut().zip(score_docs) {
        hit.score_doc.score = scored.score;
    }
    top
}

#[test]
fn feature_sort_orders_and_pages_as_lucene() {
    let java = run_java("feature-sort", "");
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    let mut i = 0;
    for (segment, size) in [10, 5, 7].into_iter().enumerate() {
        for _ in 0..size {
            writer.add_document(&feature_doc(i, segment)).unwrap();
            i += 1;
        }
        writer.commit().unwrap();
    }
    writer.close().unwrap();

    let reader = open_reader(&dir);
    assert_eq!(
        Arc::clone(&reader).leaves().len().to_string(),
        java.value("leaves")
    );
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>).unwrap();
    let sort_field = FeatureField::new_feature_sort("features", "pagerank");
    assert_eq!(sort_field.to_string(), java.value("sortField"));
    let sort = Sort::new_fields(vec![sort_field]).unwrap();

    let first = search_page(&searcher, &sort, 6, None);
    let mut lines = hit_lines("first", &first);
    let after = first.score_docs.last().cloned();
    let second = search_page(&searcher, &sort, 30, after);
    lines.extend(hit_lines("second", &second));
    let expected: Vec<String> = java
        .stdout
        .lines()
        .filter(|line| line.starts_with("first ") || line.starts_with("second "))
        .map(str::to_string)
        .collect();
    assert_eq!(lines, expected, "the feature sort's hits");
}

/// `FeatureSortField` refuses a missing value and has no index sorter, as
/// Lucene's does.
#[test]
fn feature_sort_field_contract() {
    let mut sort_field = FeatureField::new_feature_sort("features", "pagerank");
    assert!(sort_field.reverse(), "a feature sort is descending");
    assert!(sort_field.get_index_sorter().is_none());
    let error = sort_field
        .set_missing_value(Some(MissingValue::Float(1.0)))
        .expect_err("a feature sort has no missing value");
    assert!(
        error
            .to_string()
            .contains("Missing value not supported for FeatureSortField"),
        "{error}"
    );
    assert_ne!(
        sort_field,
        FeatureField::new_feature_sort("features", "other"),
        "the feature name takes part in equality"
    );
    assert_eq!(
        sort_field,
        FeatureField::new_feature_sort("features", "pagerank")
    );
}

// ---------------------------------------------------------------------------
// #204: TieredMergePolicy.MergeScore
// ---------------------------------------------------------------------------

/// `NestedTypesFixture.SCORE_SEGMENTS`: maxDoc, delCount and byte size.
const SCORE_SEGMENTS: [(i32, i32, usize); 12] = [
    (300, 0, 30000),
    (280, 28, 28000),
    (250, 0, 25000),
    (90, 9, 9000),
    (80, 0, 8000),
    (75, 30, 7500),
    (70, 21, 7000),
    (30, 0, 3000),
    (29, 3, 2900),
    (28, 0, 2800),
    (12, 6, 1200),
    (11, 0, 1100),
];

/// An info stream that keeps the `MP` messages.
#[derive(Default)]
struct CapturingInfoStream {
    messages: Mutex<Vec<String>>,
}

impl InfoStream for CapturingInfoStream {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.rucene.codec.NestedTypesFixture$1"
    }

    fn message(&self, component: &str, message: &str) {
        if component == "MP" {
            self.messages.lock().unwrap().push(message.to_string());
        }
    }

    fn is_enabled(&self, component: &str) -> bool {
        component == "MP"
    }

    fn close(&self) {}
}

/// `NestedTypesFixture`'s anonymous `MergeContext`.
struct ScoreContext {
    info_stream: Arc<CapturingInfoStream>,
}

impl MergeContext for ScoreContext {
    fn num_deletes_to_merge(&self, info: &SegmentCommitInfo) -> rucene::error::Result<i32> {
        Ok(info.get_del_count())
    }

    fn num_deleted_docs(&self, info: &SegmentCommitInfo) -> i32 {
        info.get_del_count()
    }

    fn get_merging_segments(&self) -> std::collections::HashSet<String> {
        std::collections::HashSet::new()
    }

    fn get_info_stream(&self) -> Arc<dyn InfoStream> {
        Arc::clone(&self.info_stream) as Arc<dyn InfoStream>
    }
}

#[test]
fn tiered_merge_score_and_explanation_match_lucene() {
    let java = run_java("merge-score", "");
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut infos = SegmentInfos::new(i32::from(Version::LATEST.major)).unwrap();
    let codec: Arc<dyn Codec> = Arc::new(Lucene104Codec::new());
    for (k, &(max_doc, del_count, size)) in SCORE_SEGMENTS.iter().enumerate() {
        let name = format!("_{}", radix_36(k));
        let info = SegmentInfo::new(
            Arc::clone(&dir),
            Version::LATEST,
            Some(Version::LATEST),
            name.clone(),
            max_doc,
            false,
            false,
            Arc::clone(&codec),
            std::collections::HashMap::new(),
            StringHelper::random_id(),
            std::collections::HashMap::new(),
            Sort::default(),
        )
        .unwrap();
        let mut output = dir
            .create_output(&format!("{name}.bin"), &*DEFAULT_IO_CONTEXT)
            .unwrap();
        output.write_bytes(&vec![0u8; size], 0, size).unwrap();
        output.close().unwrap();
        info.set_files([format!("{name}.bin")]).unwrap();
        infos
            .add(
                SegmentCommitInfo::new(info, del_count, 0, -1, -1, -1, StringHelper::random_id())
                    .unwrap(),
            )
            .unwrap();
    }
    let mut policy = TieredMergePolicy::new();
    policy.set_floor_segment_mb(0.002).unwrap();
    policy.set_max_merged_segment_mb(0.06).unwrap();
    policy.set_segments_per_tier(3.0).unwrap();
    policy.set_max_merge_at_once(4).unwrap();
    let context = ScoreContext {
        info_stream: Arc::new(CapturingInfoStream::default()),
    };
    let spec = policy
        .find_merges(MergeTrigger::Explicit, &infos, &context)
        .unwrap();

    let maybe: Vec<String> = context
        .info_stream
        .messages
        .lock()
        .unwrap()
        .iter()
        .filter(|line| line.starts_with("  maybe="))
        .cloned()
        .collect();
    assert_eq!(
        maybe,
        java.lines("maybe "),
        "every candidate's score and explanation"
    );
    let spec = match spec {
        None => "null".to_string(),
        Some(spec) => spec.to_string().replace('\n', "|"),
    };
    assert_eq!(vec![spec], java.lines("spec "), "the merges chosen");
}

/// `Integer.toString(k, Character.MAX_RADIX)` for the small `k` used here.
fn radix_36(k: usize) -> String {
    std::char::from_digit(k as u32, 36).unwrap().to_string()
}

// ---------------------------------------------------------------------------
// #204: NumericDocValuesFieldUpdates.SingleValueNumericDocValuesFieldUpdates
// ---------------------------------------------------------------------------

/// The field type of a numeric doc-values field.
fn numeric_dv_type() -> FieldType {
    let mut field_type = FieldType::new();
    field_type
        .set_doc_values_type(DocValuesType::NUMERIC)
        .unwrap();
    field_type.freeze();
    field_type
}

/// A numeric doc-values field with no value: the reset `updateDocValues`
/// applies, which Java writes as `new NumericDocValuesField(name, (Long) null)`.
/// This port's `Field` has no valueless constructor, so the reset is a field
/// of the numeric doc-values type carrying no number — which is what
/// `IndexWriter::update_doc_values` reads as "no value".
fn numeric_reset(name: &str) -> Field {
    Field::new_with_bytes(name, BytesRef::default(), numeric_dv_type()).unwrap()
}

fn rust_dv_update(variant: &str) -> Arc<dyn Directory> {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config.set_use_compound_file(false).unwrap();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for i in 0..50 {
        let mut d = Document::new();
        d.add(Box::new(
            StringField::new("id", format!("d{i}"), Store::NO).unwrap(),
        ));
        d.add(Box::new(
            StringField::new("group", format!("g{}", i % 5), Store::NO).unwrap(),
        ));
        d.add(Box::new(NumericDocValuesField::new(
            "price",
            i64::from(i) * 3,
        )));
        writer.add_document(&d).unwrap();
    }
    writer.commit().unwrap();
    let group = |g: &str| Term::new("group", bytes(g));
    match variant {
        "single" => {
            writer
                .update_numeric_doc_value(group("g1"), "price", 42)
                .unwrap();
            writer
                .update_numeric_doc_value(group("g3"), "price", 42)
                .unwrap();
        }
        "single-reset" => {
            writer
                .update_numeric_doc_value(group("g1"), "price", 42)
                .unwrap();
            writer
                .update_doc_values(group("g3"), &[numeric_reset("price")])
                .unwrap();
        }
        "reset-first" => {
            writer
                .update_doc_values(group("g1"), &[numeric_reset("price")])
                .unwrap();
            writer
                .update_numeric_doc_value(group("g3"), "price", 42)
                .unwrap();
        }
        other => panic!("unknown variant {other}"),
    }
    writer.commit().unwrap();
    writer.close().unwrap();
    dir
}

fn assert_dv_update(variant: &str) {
    let java = run_java("dv-update", variant);
    let dir = rust_dv_update(variant);
    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    let info = infos.info(0);
    assert_eq!(
        info.get_doc_values_gen().to_string(),
        java.value("dvGen"),
        "the update was written as doc-values generation 1"
    );
    let rust_files: BTreeSet<String> = info.files().unwrap().into_iter().collect();
    let java_files = java.files();
    assert_eq!(
        rust_files,
        java_files.keys().cloned().collect::<BTreeSet<_>>(),
        "the segment's files after the update"
    );

    // The generation files the update wrote, byte for byte.
    let rust_id = info.info.id();
    let generation: Vec<&String> = java_files
        .keys()
        .filter(|f| f.starts_with("_0_1"))
        .collect();
    assert_eq!(generation.len(), 4, "the .fnm and three doc-values files");
    for name in generation {
        let (lucene, rucene) = compare_file(&java, dir.as_ref(), &rust_id, name)
            .unwrap_or_else(|difference| panic!("{variant}: {difference}"));
        println!("dv-update {variant} {name} lucene={lucene} rucene={rucene} equal=true");
    }

    // The values a reader sees.
    let reader = open_reader(&dir);
    let leaf = Arc::clone(&reader).leaves()[0].leaf_reader();
    let mut values = leaf
        .get_numeric_doc_values("price")
        .unwrap()
        .expect("price has doc values");
    let mut seen = String::from("values");
    loop {
        let doc = values.next_doc().unwrap();
        if doc == NO_MORE_DOCS {
            break;
        }
        seen.push_str(&format!(" {doc}={}", values.long_value().unwrap()));
    }
    let expected_values = java
        .stdout
        .lines()
        .find(|line| line.starts_with("values"))
        .unwrap();
    assert_eq!(seen, expected_values, "the updated values");
}

#[test]
fn single_value_dv_update_matches_lucene() {
    assert_dv_update("single");
}

#[test]
fn single_value_dv_update_with_a_later_reset_matches_lucene() {
    assert_dv_update("single-reset");
}

#[test]
fn dv_update_with_a_reset_first_matches_lucene() {
    assert_dv_update("reset-first");
}

/// `NestedTypesFixture.MULTI_NUMERIC`: the numeric fields of the multi-field
/// update, in the order they are updated.
const MULTI_NUMERIC: [&str; 5] = ["weight", "views", "stock", "rating", "price"];

fn rust_dv_update_multi_field() -> Arc<dyn Directory> {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config.set_use_compound_file(false).unwrap();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for i in 0..50i64 {
        let mut d = Document::new();
        d.add(Box::new(
            StringField::new("id", format!("d{i}"), Store::NO).unwrap(),
        ));
        d.add(Box::new(
            StringField::new("group", format!("g{}", i % 5), Store::NO).unwrap(),
        ));
        for (f, field) in MULTI_NUMERIC.iter().enumerate() {
            d.add(Box::new(NumericDocValuesField::new(
                field,
                i * (f as i64 + 2),
            )));
        }
        d.add(Box::new(BinaryDocValuesField::new(
            "label",
            bytes(&format!("l{i}")),
        )));
        writer.add_document(&d).unwrap();
    }
    writer.commit().unwrap();
    for (f, field) in MULTI_NUMERIC.iter().enumerate() {
        writer
            .update_numeric_doc_value(
                Term::new("group", bytes(&format!("g{f}"))),
                field,
                100 + f as i64,
            )
            .unwrap();
    }
    writer
        .update_binary_doc_value(Term::new("group", bytes("g2")), "label", bytes("updated"))
        .unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();
    dir
}

/// Five numeric fields and one binary field updated in one commit: each field
/// is written as its own doc-values generation, in the iteration order of
/// `ReadersAndUpdates.pendingDVUpdates`, a `java.util.HashMap`
/// (`ReadersAndUpdates.java:78, 293, 456`). The field names are chosen so that
/// Java's order — price, rating, weight, label, stock, views — is neither the
/// update order nor the sorted order, and no two share a bucket (so the order
/// does not depend on the insertion order `Map.copyOf` randomises in
/// `FrozenBufferedUpdates`). The run is repeated, because a seeded hash order
/// would pass by chance on some runs.
#[test]
fn dv_update_of_several_fields_follows_java_hash_map_order() {
    let java = run_java("dv-update", "multi-field");
    let java_files = java.files();
    let generation_files: Vec<&String> = java_files
        .keys()
        .filter(|f| f.starts_with("_0_") && !f.starts_with("_0_Lucene"))
        .collect();
    assert_eq!(
        generation_files.len(),
        19,
        "one .fnm and six times three doc-values files"
    );
    for run in 0..6 {
        let dir = rust_dv_update_multi_field();
        let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
        let info = infos.info(0);
        assert_eq!(info.get_doc_values_gen().to_string(), java.value("dvGen"));
        let rust_files: BTreeSet<String> = info.files().unwrap().into_iter().collect();
        assert_eq!(
            rust_files,
            java_files.keys().cloned().collect::<BTreeSet<_>>(),
            "run {run}: the segment's files after the update"
        );
        let rust_id = info.info.id();
        for name in &generation_files {
            let (lucene, rucene) = compare_file(&java, dir.as_ref(), &rust_id, name)
                .unwrap_or_else(|difference| panic!("run {run}: {difference}"));
            if run == 0 {
                println!("dv-update multi-field {name} lucene={lucene} rucene={rucene} equal=true");
            }
        }
        let reader = open_reader(&dir);
        let leaf = Arc::clone(&reader).leaves()[0].leaf_reader();
        let gens: Vec<String> = leaf
            .get_field_infos()
            .iter()
            .map(|fi| format!("{} {}", fi.name, fi.get_doc_values_gen()))
            .collect();
        assert_eq!(
            gens,
            java.lines("gen "),
            "run {run}: each field's generation"
        );
    }
}

// ---------------------------------------------------------------------------
// F1/F2: every file of an index-sorted flush, and the documents a reader sees
// ---------------------------------------------------------------------------

/// `NestedTypesFixture.FULL_DOCS`.
const FULL_DOCS: i32 = 300;

/// `NestedTypesFixture.TV_TYPE`: `TextField.TYPE_NOT_STORED` with term vectors,
/// positions and offsets.
fn term_vector_type() -> FieldType {
    let mut field_type = FieldType::new();
    field_type.set_tokenized(true).unwrap();
    field_type
        .set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
        .unwrap();
    field_type.set_store_term_vectors(true).unwrap();
    field_type.set_store_term_vector_positions(true).unwrap();
    field_type.set_store_term_vector_offsets(true).unwrap();
    field_type.freeze();
    field_type
}

/// `NestedTypesFixture.fullDoc(int)`, field for field and in the same order.
fn full_doc(i: i32) -> Document {
    let mut d = Document::new();
    d.add(Box::new(
        StringField::new("id", format!("id{i:04}"), Store::YES).unwrap(),
    ));
    d.add(Box::new(
        StringField::new("tag", format!("t{}", i % 17), Store::NO).unwrap(),
    ));
    let mut body = format!("all w{}", i % 31);
    for r in 0..=((i * 7) % 9) {
        body.push_str(&format!(" rep{}", r % 3));
    }
    d.add(Box::new(TextField::new("body", body, Store::NO).unwrap()));
    if i % 4 != 1 {
        d.add(Box::new(
            StoredField::new_string("sv", format!("value-{}", i * 13 % 97)).unwrap(),
        ));
    }
    d.add(Box::new(
        StoredField::new_number("sn", NumericValue::Int(i * 3)).unwrap(),
    ));
    d.add(Box::new(IntPoint::new("ip", &[(i * 37) % 101]).unwrap()));
    d.add(Box::new(
        IntPoint::new("p2", &[i % 23, (i * 11) % 29]).unwrap(),
    ));
    if i % 3 != 2 {
        d.add(Box::new(
            Field::new(
                "tv",
                format!("tv{} common tv{}", i % 5, i % 5),
                term_vector_type(),
            )
            .unwrap(),
        ));
    }
    if i % 13 != 4 {
        d.add(Box::new(NumericDocValuesField::new(
            "rank",
            i64::from((i * 37) % 101),
        )));
    }
    if i % 7 != 3 {
        d.add(Box::new(SortedDocValuesField::new(
            "skey",
            bytes(&format!("s{}", (i * 53) % 71)),
        )));
    }
    if let Some(key) = key(i) {
        d.add(Box::new(BinaryDocValuesField::new(
            "key",
            BytesRef::new(key),
        )));
    }
    for v in 0..(i % 3) {
        d.add(Box::new(SortedNumericDocValuesField::new(
            "snum",
            i64::from((i * (v + 5)) % 19 - 7),
        )));
    }
    for v in 0..(i % 4) {
        d.add(Box::new(SortedSetDocValuesField::new(
            "sset",
            bytes(&format!("ss{}", (i + v * 7) % 11)),
        )));
    }
    d
}

/// `NestedTypesFixture.fullSortField(String)`.
fn full_sort_field(variant: &str) -> SortField {
    match variant {
        "numeric" => SortField::new(Some("rank".to_string()), SortFieldType::Long).unwrap(),
        "string" => SortField::new(Some("skey".to_string()), SortFieldType::String).unwrap(),
        "binary" => binary_sort_field("asc"),
        other => panic!("unknown variant {other}"),
    }
}

/// `NestedTypesFixture.addFullDocs(IndexWriter, int, int)`.
fn add_full_docs(writer: &IndexWriter, from: i32, to: i32) {
    for i in from..to {
        writer.add_document(&full_doc(i)).unwrap();
        if i == from + (to - from) / 2 {
            writer
                .delete_documents_by_terms(vec![Term::new("tag", bytes("t3"))])
                .unwrap();
        }
    }
    writer
        .delete_documents_by_terms(vec![Term::new("tag", bytes("t5"))])
        .unwrap();
}

/// `NestedTypesFixture.sortedFull(Path, String, boolean)`.
fn rust_sorted_full(variant: &str, merge: bool) -> Arc<dyn Directory> {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config.set_use_compound_file(false).unwrap();
    if merge {
        config
            .set_merge_policy(Arc::new(TieredMergePolicy::new()))
            .unwrap();
    } else {
        config
            .set_merge_policy(Arc::new(NoMergePolicy::new()))
            .unwrap();
    }
    config
        .set_index_sort(Sort::new_fields(vec![full_sort_field(variant)]).unwrap())
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    if merge {
        add_full_docs(&writer, 0, FULL_DOCS / 2);
        writer.commit().unwrap();
        add_full_docs(&writer, FULL_DOCS / 2, FULL_DOCS);
        writer.commit().unwrap();
        writer.force_merge(1).unwrap();
    } else {
        add_full_docs(&writer, 0, FULL_DOCS);
    }
    writer.commit().unwrap();
    writer.close().unwrap();
    dir
}

/// Collects a document's stored fields as `name=value`, as Java's
/// `IndexableField.numericValue()`/`stringValue()` render them.
struct StoredLines(Vec<String>);

impl StoredFieldVisitor for StoredLines {
    fn string_field(&mut self, field_info: &FieldInfo, value: &str) -> rucene::error::Result<()> {
        self.0.push(format!("{}={value}", field_info.name));
        Ok(())
    }

    fn int_field(&mut self, field_info: &FieldInfo, value: i32) -> rucene::error::Result<()> {
        self.0.push(format!("{}={value}", field_info.name));
        Ok(())
    }

    fn needs_field(
        &mut self,
        _field_info: &FieldInfo,
    ) -> rucene::error::Result<StoredFieldVisitorStatus> {
        Ok(StoredFieldVisitorStatus::Yes)
    }
}

/// Collects every point of a field by document, as `NestedTypesFixture.points`.
struct PointLines {
    dims: usize,
    by_doc: BTreeMap<i32, String>,
}

impl IntersectVisitor for PointLines {
    fn visit(&mut self, _doc_id: i32) -> rucene::error::Result<()> {
        panic!("every cell crosses");
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> rucene::error::Result<()> {
        let value = (0..self.dims)
            .map(|dim| IntPoint::decode_dimension(packed_value, dim * 4).to_string())
            .collect::<Vec<_>>()
            .join(",");
        self.by_doc
            .entry(doc_id)
            .and_modify(|existing| {
                existing.push(';');
                existing.push_str(&value);
            })
            .or_insert(value);
        Ok(())
    }

    fn compare(&self, _min: &[u8], _max: &[u8]) -> rucene::error::Result<Relation> {
        Ok(Relation::CellCrossesQuery)
    }
}

fn points_by_doc(leaf: &Arc<dyn rucene::index::LeafReader>, field: &str) -> BTreeMap<i32, String> {
    let values = leaf.get_point_values(field).unwrap().expect("points");
    let mut visitor = PointLines {
        dims: values.num_dimensions().unwrap() as usize,
        by_doc: BTreeMap::new(),
    };
    values.intersect(&mut visitor).unwrap();
    visitor.by_doc
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `NestedTypesFixture.printDocuments(LeafReader)`, line for line.
fn document_lines(leaf: &Arc<dyn rucene::index::LeafReader>) -> Vec<String> {
    let mut ids: BTreeMap<i32, String> = BTreeMap::new();
    let terms = leaf.terms("id").unwrap().expect("id is indexed");
    let mut terms_enum = terms.iterator().unwrap();
    while let Some(term) = terms_enum.next().unwrap() {
        let text = String::from_utf8(term.slice().to_vec()).unwrap();
        let mut postings = terms_enum.postings(None, POSTINGS_ENUM_NONE).unwrap();
        loop {
            let doc = postings.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                break;
            }
            ids.entry(doc)
                .and_modify(|existing| {
                    existing.push(';');
                    existing.push_str(&text);
                })
                .or_insert_with(|| text.clone());
        }
    }
    let ip = points_by_doc(leaf, "ip");
    let p2 = points_by_doc(leaf, "p2");
    let live = leaf.get_live_docs();
    let stored = leaf.stored_fields().unwrap();
    let vectors = leaf.term_vectors().unwrap();
    let mut rank = leaf.get_numeric_doc_values("rank").unwrap().unwrap();
    let mut skey = leaf.get_sorted_doc_values("skey").unwrap().unwrap();
    let mut key = leaf.get_binary_doc_values("key").unwrap().unwrap();
    let mut snum = leaf.get_sorted_numeric_doc_values("snum").unwrap().unwrap();
    let mut sset = leaf.get_sorted_set_doc_values("sset").unwrap().unwrap();
    let mut norms = leaf.get_norm_values("body").unwrap().unwrap();
    let mut lines = Vec::new();
    for doc in 0..leaf.max_doc() {
        let mut b = format!("doc {doc}");
        let is_live = live.as_ref().is_none_or(|bits| bits.get(doc as usize));
        b.push_str(&format!(" live={is_live}"));
        b.push_str(&format!(
            " id={}",
            ids.get(&doc).map_or("-", String::as_str)
        ));
        let mut visitor = StoredLines(Vec::new());
        stored.document_with_visitor(doc, &mut visitor).unwrap();
        b.push_str(&format!(" stored=[{}]", visitor.0.join(",")));
        b.push_str(" rank=");
        if rank.advance_exact(doc).unwrap() {
            b.push_str(&rank.long_value().unwrap().to_string());
        } else {
            b.push('-');
        }
        b.push_str(" skey=");
        if skey.advance_exact(doc).unwrap() {
            let term = skey.lookup_ord(skey.ord_value().unwrap()).unwrap();
            b.push_str(std::str::from_utf8(term.slice()).unwrap());
        } else {
            b.push('-');
        }
        b.push_str(" key=");
        if key.advance_exact(doc).unwrap() {
            b.push_str(&hex(key.binary_value().unwrap().slice()));
        } else {
            b.push('-');
        }
        b.push_str(" snum=[");
        if snum.advance_exact(doc).unwrap() {
            let values: Vec<String> = (0..snum.doc_value_count().unwrap())
                .map(|_| snum.next_value().unwrap().to_string())
                .collect();
            b.push_str(&values.join(","));
        }
        b.push(']');
        b.push_str(" sset=[");
        if sset.advance_exact(doc).unwrap() {
            let values: Vec<String> = (0..sset.doc_value_count().unwrap())
                .map(|_| {
                    let ord = sset.next_ord().unwrap();
                    String::from_utf8(sset.lookup_ord(ord).unwrap().slice().to_vec()).unwrap()
                })
                .collect();
            b.push_str(&values.join(","));
        }
        b.push(']');
        b.push_str(" norm=");
        if norms.advance_exact(doc).unwrap() {
            b.push_str(&norms.long_value().unwrap().to_string());
        } else {
            b.push('-');
        }
        b.push_str(&format!(" ip={}", ip.get(&doc).map_or("-", String::as_str)));
        b.push_str(&format!(" p2={}", p2.get(&doc).map_or("-", String::as_str)));
        b.push_str(" tv=[");
        if let Some(tv) = vectors.get_field(doc, "tv").unwrap() {
            let mut te = tv.iterator().unwrap();
            let mut entries = Vec::new();
            while let Some(term) = te.next().unwrap() {
                let mut pe = te.postings(None, POSTINGS_ENUM_ALL).unwrap();
                pe.next_doc().unwrap();
                let freq = pe.freq().unwrap();
                let mut entry = format!("{}:{freq}", std::str::from_utf8(term.slice()).unwrap());
                for _ in 0..freq {
                    let position = pe.next_position().unwrap();
                    entry.push_str(&format!(
                        "@{position}[{}-{}]",
                        pe.start_offset(),
                        pe.end_offset()
                    ));
                }
                entries.push(entry);
            }
            b.push_str(&entries.join(","));
        }
        b.push(']');
        lines.push(b);
    }
    lines
}

/// Asserts what makes a line self-consistent: the `id` term that leads to the
/// document is the `id` its stored fields hold, and the doc values, norm and
/// points are those of the same source document.
fn assert_self_consistent(lines: &[String]) {
    for line in lines {
        let field = |name: &str| -> &str {
            let start = line.find(&format!(" {name}=")).expect(name) + name.len() + 2;
            let rest = &line[start..];
            let end = if rest.starts_with('[') {
                rest.find(']').unwrap() + 1
            } else {
                rest.find(' ').unwrap_or(rest.len())
            };
            &rest[..end]
        };
        let id = field("id");
        let stored = field("stored");
        assert!(
            stored.starts_with(&format!("[id={id},")),
            "the id term and the stored id disagree: {line}"
        );
        let i: i32 = id[2..].parse().unwrap();
        let rank = if i % 13 != 4 {
            ((i * 37) % 101).to_string()
        } else {
            "-".to_string()
        };
        assert_eq!(field("rank"), rank, "rank of {id}: {line}");
        assert_eq!(
            field("ip"),
            ((i * 37) % 101).to_string(),
            "ip of {id}: {line}"
        );
        assert_eq!(
            field("p2"),
            format!("{},{}", i % 23, (i * 11) % 29),
            "p2 of {id}: {line}"
        );
        assert_eq!(
            field("key"),
            key(i).map_or("-".to_string(), |k| hex(&k)),
            "key of {id}: {line}"
        );
    }
}

fn assert_sorted_full_flush(variant: &str) {
    let java = run_java("sorted-full", variant);
    let dir = rust_sorted_full(variant, false);
    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    assert_eq!(infos.size().to_string(), java.value("segments"));
    let info = infos.info(0);
    assert_eq!(info.info.name, java.value("name"));
    assert_eq!(info.info.index_sort().to_string(), java.value("indexSort"));
    assert_eq!(info.get_del_count().to_string(), java.value("delCount"));

    let rust_files: BTreeSet<String> = info.files().unwrap().into_iter().collect();
    let java_files = java.files();
    assert_eq!(
        rust_files,
        java_files.keys().cloned().collect::<BTreeSet<_>>(),
        "{variant}: the segment's files"
    );
    // The sorting consumers' temporary files are deleted after the copy
    // (`SortingStoredFieldsConsumer.java:135-138`,
    // `SortingTermVectorsConsumer.java:80-82`).
    let leftovers: Vec<String> = dir
        .list_all()
        .unwrap()
        .into_iter()
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "{variant}: temporary files left: {leftovers:?}"
    );
    // Every file but the `.si`, whose diagnostics name the JVM and the time.
    let rust_id = info.info.id();
    let mut differences = Vec::new();
    for name in java_files.keys().filter(|f| !f.ends_with(".si")) {
        match compare_file(&java, dir.as_ref(), &rust_id, name) {
            Ok((lucene, rucene)) => {
                println!("sorted-full {variant} {name} lucene={lucene} rucene={rucene} equal=true")
            }
            Err(difference) => differences.push(difference),
        }
    }
    assert!(
        differences.is_empty(),
        "{variant}: files differ:\n{}",
        differences.join("\n")
    );

    let reader = open_reader(&dir);
    let leaf = Arc::clone(&reader).leaves()[0].leaf_reader();
    let lines = document_lines(&leaf);
    assert_self_consistent(&lines);
    let expected: Vec<String> = java
        .stdout
        .lines()
        .filter(|line| line.starts_with("doc "))
        .map(str::to_string)
        .collect();
    assert_eq!(
        lines, expected,
        "{variant}: what a reader sees, document by document"
    );
}

/// Two sorted segments merged into one: every document stays self-consistent —
/// its `id` term, stored fields, doc values, norm, points and term vectors all
/// belong to the same source document — and the merged segment holds exactly
/// Lucene's documents, live and deleted, each at Lucene's doc id: the two
/// segments are interleaved through `MultiSorter.sort`
/// (`MergeState.java:212-240`), with an `OrdinalMap` across the segments for
/// the string sort (rmp #229).
fn assert_sorted_merge(variant: &str) {
    let java = run_java("sorted-merge", variant);
    let dir = rust_sorted_full(variant, true);
    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    assert_eq!(infos.size().to_string(), java.value("segments"));
    let info = infos.info(0);
    assert_eq!(info.info.index_sort().to_string(), java.value("indexSort"));
    assert_eq!(info.get_del_count().to_string(), java.value("delCount"));
    let reader = open_reader(&dir);
    let leaf = Arc::clone(&reader).leaves()[0].leaf_reader();
    let lines = document_lines(&leaf);
    assert_self_consistent(&lines);
    let expected: Vec<String> = java
        .stdout
        .lines()
        .filter(|line| line.starts_with("doc "))
        .map(str::to_string)
        .collect();
    let without_doc_id = |lines: &[String]| -> BTreeSet<String> {
        lines
            .iter()
            .map(|line| line.splitn(3, ' ').nth(2).unwrap().to_string())
            .collect()
    };
    assert_eq!(
        lines.len(),
        expected.len(),
        "{variant}: the merged segment's maxDoc"
    );
    assert_eq!(
        without_doc_id(&lines),
        without_doc_id(&expected),
        "{variant}: the merged segment's documents"
    );
    // rmp #229: `MergeState.buildDocMaps` merge-sorts the incoming segments
    // through `MultiSorter`, so every document lands at Lucene's doc id.
    assert_eq!(
        lines, expected,
        "{variant}: the merged segment's document order"
    );
}

#[test]
fn numeric_index_sort_flush_writes_every_file_as_lucene() {
    assert_sorted_full_flush("numeric");
}

#[test]
fn string_index_sort_flush_writes_every_file_as_lucene() {
    assert_sorted_full_flush("string");
}

#[test]
fn binary_index_sort_flush_writes_every_file_as_lucene() {
    assert_sorted_full_flush("binary");
}

#[test]
fn numeric_index_sort_merge_keeps_every_document_consistent() {
    assert_sorted_merge("numeric");
}

#[test]
fn string_index_sort_merge_keeps_every_document_consistent() {
    assert_sorted_merge("string");
}

#[test]
fn binary_index_sort_merge_keeps_every_document_consistent() {
    assert_sorted_merge("binary");
}

// ---------------------------------------------------------------------------
// #204: MergePolicy.MergeObserver
// ---------------------------------------------------------------------------

fn rust_merge_observer(variant: &str) -> Vec<String> {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    if variant == "pending" {
        config
            .set_merge_scheduler(Arc::new(NoMergeScheduler))
            .unwrap();
    } else {
        config
            .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
            .unwrap();
    }
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for segment in 0..2 {
        for k in 0..20 {
            let mut d = Document::new();
            d.add(Box::new(
                StringField::new("id", format!("s{segment}d{k}"), Store::NO).unwrap(),
            ));
            let deleted = variant != "none" && k < 3 - segment;
            d.add(Box::new(
                StringField::new(
                    "del",
                    if deleted { "yes" } else { "no" }.to_string(),
                    Store::NO,
                )
                .unwrap(),
            ));
            writer.add_document(&d).unwrap();
        }
        writer.commit().unwrap();
    }
    writer
        .delete_documents_by_terms(vec![Term::new("del", bytes("yes"))])
        .unwrap();
    let observer = writer.force_merge_deletes_with(variant == "wait").unwrap();
    let mut lines = vec![
        format!("numMerges={}", observer.num_merges()),
        format!("numCompletedMerges={}", observer.num_completed_merges()),
        format!(
            "awaitTimeout={}",
            observer.await_timeout(Duration::from_millis(10))
        ),
        format!("awaitAsyncDone={}", observer.await_async().is_done()),
    ];
    if variant == "wait" {
        lines.push(format!("await={}", observer.r#await()));
    }
    // After the wait, each merge renders the merged segment's name too
    // (`OneMerge.segString`, `MergePolicy.java:357-359`).
    lines.push(format!(
        "toString={}",
        observer.to_string().replace('\n', "|")
    ));
    writer.rollback().unwrap();
    lines
}

fn assert_merge_observer(variant: &str) {
    let java = run_java("merge-observer", variant);
    let expected: Vec<String> = java
        .stdout
        .lines()
        .filter(|line| {
            ["numMerges=", "numCompletedMerges=", "await", "toString="]
                .iter()
                .any(|prefix| line.starts_with(prefix))
        })
        .map(str::to_string)
        .collect();
    assert_eq!(
        rust_merge_observer(variant),
        expected,
        "the {variant} observer"
    );
}

#[test]
fn merge_observer_without_merges_matches_lucene() {
    assert_merge_observer("none");
}

#[test]
fn merge_observer_of_pending_merges_matches_lucene() {
    assert_merge_observer("pending");
}

#[test]
fn merge_observer_of_finished_merges_matches_lucene() {
    assert_merge_observer("wait");
}

// ---------------------------------------------------------------------------
// #204: FilterDirectoryReader.DelegatingCacheHelper
// ---------------------------------------------------------------------------

fn key_address(helper: &dyn CacheHelper) -> usize {
    helper.get_key() as *const CacheKey as usize
}

/// Counts how often it is called.
struct CountingListener(Arc<AtomicUsize>);

impl ClosedListener for CountingListener {
    fn on_close(&self, _key: CacheKey) -> rucene::error::Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn delegating_cache_helper_matches_lucene() {
    let java = run_java("cache-helper", "");
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config.set_soft_deletes_field(Some("soft".to_string()));
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    // Document c3 carries the soft-deletes field, so the first segment has one
    // soft deletion; the second segment has none.
    for i in 0..10 {
        let mut d = Document::new();
        d.add(Box::new(
            StringField::new("id", format!("c{i}"), Store::NO).unwrap(),
        ));
        if i == 3 {
            d.add(Box::new(NumericDocValuesField::new("soft", 1)));
        }
        writer.add_document(&d).unwrap();
    }
    writer.commit().unwrap();
    let mut last = Document::new();
    last.add(Box::new(
        StringField::new("id", "c10".to_string(), Store::NO).unwrap(),
    ));
    writer.add_document(&last).unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();

    let inner = open_reader(&dir);
    let wrapped = SoftDeletesDirectoryReaderWrapper::new("soft")
        .wrap(Arc::clone(&inner))
        .unwrap();
    let inner_key = key_address(inner.get_reader_cache_helper().unwrap().as_ref());
    let wrapped_helper = wrapped.get_reader_cache_helper().unwrap();
    let wrapped_key = key_address(wrapped_helper.as_ref());
    let mut lines = vec![
        format!("dirDistinct={}", inner_key != wrapped_key),
        format!(
            "dirStable={}",
            key_address(wrapped.get_reader_cache_helper().unwrap().as_ref()) == wrapped_key
        ),
    ];
    let wrapped_leaves = Arc::clone(&wrapped).leaves();
    let inner_leaves = Arc::clone(&inner).leaves();
    for (i, (w, u)) in wrapped_leaves.iter().zip(inner_leaves.iter()).enumerate() {
        let w = w.leaf_reader();
        let u = u.leaf_reader();
        let wk = key_address(w.get_reader_cache_helper().unwrap().as_ref());
        let uk = key_address(u.get_reader_cache_helper().unwrap().as_ref());
        lines.push(format!(
            "leaf {i} numDocs={} readerDistinct={} readerStable={} coreShared={}",
            w.num_docs(),
            wk != uk,
            key_address(w.get_reader_cache_helper().unwrap().as_ref()) == wk,
            key_address(w.get_core_cache_helper().unwrap().as_ref())
                == key_address(u.get_core_cache_helper().unwrap().as_ref())
        ));
    }

    let dir_calls = Arc::new(AtomicUsize::new(0));
    wrapped_helper.add_closed_listener(Box::new(CountingListener(Arc::clone(&dir_calls))));
    let leaf_calls = Arc::new(AtomicUsize::new(0));
    wrapped_leaves[0]
        .leaf_reader()
        .get_reader_cache_helper()
        .unwrap()
        .add_closed_listener(Box::new(CountingListener(Arc::clone(&leaf_calls))));
    drop(wrapped_leaves);
    drop(inner_leaves);
    wrapped.close().unwrap();
    lines.push(format!(
        "dirListenerCalls={}",
        dir_calls.load(Ordering::SeqCst)
    ));
    lines.push(format!(
        "leafListenerCalls={}",
        leaf_calls.load(Ordering::SeqCst)
    ));

    // `dirListenerKeyIsWrapper` is not compared: this port's
    // `ClosedListener::on_close` takes the key by value, so the identity Java
    // checks there cannot be observed (the divergence declared on
    // `DelegatingCacheHelper`).
    let expected: Vec<String> = java
        .stdout
        .lines()
        .filter(|line| {
            (line.starts_with("dir") || line.starts_with("leaf"))
                && !line.starts_with("dirListenerKeyIsWrapper")
        })
        .map(str::to_string)
        .collect();
    assert_eq!(lines, expected);
    assert_eq!(java.value("dirListenerKeyIsWrapper"), "true");
}

/// A delegate with its own key, which a `DelegatingCacheHelper` must never
/// answer with.
#[derive(Debug, Default)]
struct PlainHelper {
    key: CacheKey,
    _non_empty: u8,
}

impl CacheHelper for PlainHelper {
    fn get_key(&self) -> &CacheKey {
        &self.key
    }

    fn add_closed_listener(&self, _listener: Box<dyn ClosedListener>) {}
}

/// Java's `cacheKey` is an object: moving the reference that holds the helper
/// leaves the key's identity unchanged. The port's key must not move with the
/// helper either, whether the helper is moved by value or boxed.
#[test]
fn delegating_cache_helper_key_does_not_move_with_the_helper() {
    let helper = DelegatingCacheHelper::new(Box::<PlainHelper>::default());
    let before = key_address(&helper);
    let delegate_key = key_address(&PlainHelper::default());
    assert_ne!(before, delegate_key);

    let moved = helper;
    assert_eq!(key_address(&moved), before, "moved by value");
    let boxed: Box<dyn CacheHelper> = Box::new(moved);
    assert_eq!(key_address(boxed.as_ref()), before, "moved into a box");
    let mut holder = vec![boxed];
    holder.reserve(64);
    assert_eq!(
        key_address(holder[0].as_ref()),
        before,
        "moved by a vector growth"
    );
}

/// A Java cache that still references a helper's key keeps that identity from
/// being given to any other key. Here the helper is dropped while a handle on
/// its key is held: no helper created afterwards may answer with the same
/// address, and the handle still answers with it.
#[test]
fn delegating_cache_helper_key_is_not_reused_while_a_holder_is_alive() {
    let helper = DelegatingCacheHelper::new(Box::<PlainHelper>::default());
    let handle = helper.cache_key();
    let address = key_address(&helper);
    drop(helper);

    // Same-sized allocations right after the free are where an allocator
    // hands the freed block back out.
    let later: Vec<DelegatingCacheHelper> = (0..1024)
        .map(|_| DelegatingCacheHelper::new(Box::<PlainHelper>::default()))
        .collect();
    for (i, other) in later.iter().enumerate() {
        assert_ne!(
            key_address(other),
            address,
            "helper {i} took the key of a helper whose key is still held"
        );
    }
    assert_eq!(
        handle.key() as *const CacheKey as usize,
        address,
        "the handle answers with the dropped helper's key"
    );
}

// ---------------------------------------------------------------------------
// #202: FilterLeafReader.FilterFields and FilterPostingsEnum
// ---------------------------------------------------------------------------

/// Every `(field, term, doc, freq, positions, offsets, payload)` a `Fields`
/// exposes, read through `PostingsEnum::next_doc`.
fn dump_fields(fields: &dyn Fields, wrap: bool) -> Vec<String> {
    let mut out = vec![format!("size={}", fields.size())];
    let names: Vec<String> = fields.iterator().collect();
    for name in names {
        let terms = fields.terms(&name).unwrap().expect("a listed field");
        let mut terms_enum = terms.iterator().unwrap();
        while let Some(term) = terms_enum.next().unwrap() {
            let postings = terms_enum.postings(None, POSTINGS_ENUM_ALL).unwrap();
            let mut postings: Box<dyn PostingsEnum> = if wrap {
                Box::new(FilterPostingsEnum::new(postings))
            } else {
                postings
            };
            out.push(format!("{name}:{term} cost={}", postings.cost()));
            loop {
                let doc = postings.next_doc().unwrap();
                assert_eq!(postings.doc_id(), doc);
                if doc == NO_MORE_DOCS {
                    break;
                }
                let freq = postings.freq().unwrap();
                let mut line = format!("  doc={doc} freq={freq}");
                if terms.has_positions() {
                    for _ in 0..freq {
                        let position = postings.next_position().unwrap();
                        line.push_str(&format!(
                            " {position}[{}-{}]{:?}",
                            postings.start_offset(),
                            postings.end_offset(),
                            postings.get_payload().unwrap()
                        ));
                    }
                }
                out.push(line);
            }
        }
    }
    out
}

/// A segment whose postings carry every feature a `PostingsEnum` reports.
fn postings_segment() -> Arc<dyn DirectoryReader> {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    for i in 0..300 {
        writer.add_document(&sorted_doc(i)).unwrap();
    }
    writer.commit().unwrap();
    writer.close().unwrap();
    open_reader(&dir)
}

/// `FilterFields` and `FilterPostingsEnum` forward every method they declare,
/// so a filter that overrides nothing exposes exactly what the wrapped reader
/// does.
#[test]
fn filter_fields_and_filter_postings_enum_forward_everything() {
    let reader = postings_segment();
    let leaf = Arc::clone(&reader).leaves()[0].leaf_reader();
    let codec_fields = leaf_fields(&leaf);
    let unwrapped = dump_fields(&codec_fields, false);
    let filtered = FilterFields::new(Box::new(leaf_fields(&leaf)) as Box<dyn Fields>);
    assert_eq!(dump_fields(&filtered, true), unwrapped);
    assert!(unwrapped.len() > 2000, "the dump covers the whole segment");

    // `advance` is forwarded too, and `unwrap()` hands back the delegate.
    let terms = leaf.terms("body").unwrap().unwrap();
    let mut terms_enum = terms.iterator().unwrap();
    assert!(terms_enum.seek_exact(&bytes("rep")).unwrap());
    let mut plain = terms_enum.postings(None, POSTINGS_ENUM_NONE).unwrap();
    assert!(terms_enum.seek_exact(&bytes("rep")).unwrap());
    let mut filter =
        FilterPostingsEnum::new(terms_enum.postings(None, POSTINGS_ENUM_NONE).unwrap());
    for target in [0, 7, 128, 129, 250, 299, 300] {
        assert_eq!(
            filter.advance(target).unwrap(),
            plain.advance(target).unwrap()
        );
    }
    let delegate = rucene::util::Unwrappable::unwrap(&filter);
    assert_eq!(delegate.doc_id(), plain.doc_id());
    assert!(filter
        .to_string()
        .starts_with("org.apache.lucene.index.FilterLeafReader$FilterPostingsEnum@"));
}

/// The `Fields` view of a leaf: every indexed field, through
/// `LeafReader::terms`.
fn leaf_fields(leaf: &Arc<dyn rucene::index::LeafReader>) -> LeafFieldsView {
    let mut names: Vec<String> = leaf
        .get_field_infos()
        .iter()
        .filter(|info| info.index_options != IndexOptions::NONE)
        .map(|info| info.name.clone())
        .collect();
    names.sort();
    LeafFieldsView {
        leaf: Arc::clone(leaf),
        names,
    }
}

struct LeafFieldsView {
    leaf: Arc<dyn rucene::index::LeafReader>,
    names: Vec<String>,
}

impl Fields for LeafFieldsView {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(self.names.iter().cloned())
    }

    fn terms(&self, field: &str) -> rucene::error::Result<Option<Box<dyn rucene::index::Terms>>> {
        self.leaf.terms(field)
    }

    fn size(&self) -> i32 {
        self.names.len() as i32
    }
}
