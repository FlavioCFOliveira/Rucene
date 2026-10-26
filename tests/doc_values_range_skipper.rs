//! The doc-values range queries narrow a sorted leaf to one document range.
//!
//! Regression guard for rmp #47. When the leaf is index-sorted by the queried
//! field first, the skip index covers every document, and the field is
//! single-valued, `SortedNumericDocValuesRangeQuery` and
//! `SortedSetDocValuesRangeQuery` hand out a `SortedSkipperScorerSupplier`
//! (`SortedNumericDocValuesRangeQuery.java:138-142, 193-233`,
//! `SortedSetDocValuesRangeQuery.java:122-130, 198-234`), whose scorer
//! iterates one contiguous `[min, max)` document range with no two-phase
//! check. Without an index sort, the same queries keep the two-phase
//! doc-values iterator; that unsorted leaf is the positive control.

use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::doc_values_queries::{
    SortedNumericDocValuesRangeQuery, SortedSetDocValuesRangeQuery,
};
use rucene::document::{Document, NumericDocValuesField, SortedDocValuesField};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::IndexReader;
use rucene::search::{IndexSearcher, Query, ScoreMode, Sort, SortField, SortFieldType};
use rucene::store::{Directory, RamDirectory};
use rucene::util::BytesRef;

const DOC_COUNT: i32 = 3000;

fn num(i: i32) -> i64 {
    i64::from((i * 37) % 500)
}

fn tag(i: i32) -> String {
    format!("t{:03}", (i * 13) % 200)
}

fn searcher(sort: Option<Sort>) -> IndexSearcher {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    if let Some(sort) = sort {
        config.set_index_sort(sort).expect("index sort");
    }
    let writer = IndexWriter::new(Arc::clone(&dir), config).expect("writer");
    for i in 0..DOC_COUNT {
        let mut d = Document::new();
        d.add(Box::new(NumericDocValuesField::indexed_field(
            "num",
            num(i),
        )));
        d.add(Box::new(SortedDocValuesField::indexed_field(
            "tag",
            BytesRef::new(tag(i).into_bytes()),
        )));
        writer.add_document(&d).expect("add");
    }
    writer.force_merge(1).expect("force merge");
    writer.commit().expect("commit");
    writer.close().expect("close");
    let reader = directory_reader::open(dir).expect("reader");
    IndexSearcher::new(reader as Arc<dyn IndexReader>).expect("searcher")
}

fn sort_by(field: &str, kind: SortFieldType) -> Sort {
    Sort::new_fields(vec![
        SortField::new(Some(field.to_string()), kind).expect("sort field")
    ])
    .expect("sort")
}

/// Returns, for the one leaf, whether the scorer is two-phase and how many
/// documents it matches.
fn probe(searcher: &IndexSearcher, query: Arc<dyn Query>) -> (bool, i32) {
    let weight = searcher
        .create_weight(Arc::clone(&query), ScoreMode::COMPLETE_NO_SCORES, 1.0)
        .expect("weight");
    let leaves = searcher.get_leaf_contexts();
    assert_eq!(leaves.len(), 1);
    let mut supplier = weight
        .scorer_supplier(&leaves[0])
        .expect("scorer supplier")
        .expect("the range matches documents");
    let mut scorer = supplier.get(i64::MAX).expect("scorer");
    let two_phase = scorer.two_phase_iterator().is_some();
    (two_phase, searcher.count(query).expect("count"))
}

fn expected_num(lo: i64, hi: i64) -> i32 {
    (0..DOC_COUNT)
        .filter(|&i| (lo..=hi).contains(&num(i)))
        .count() as i32
}

fn expected_tag(lo: &str, hi: &str) -> i32 {
    (0..DOC_COUNT)
        .filter(|&i| {
            let t = tag(i);
            t.as_str() >= lo && t.as_str() <= hi
        })
        .count() as i32
}

fn numeric_query() -> Arc<dyn Query> {
    Arc::new(SortedNumericDocValuesRangeQuery::new("num", 100, 250))
}

fn ordinal_query() -> Arc<dyn Query> {
    Arc::new(SortedSetDocValuesRangeQuery::new(
        "tag",
        Some(BytesRef::new(b"t050".to_vec())),
        Some(BytesRef::new(b"t120".to_vec())),
        true,
        true,
    ))
}

#[test]
fn a_numeric_range_over_a_leaf_sorted_by_its_field_uses_the_skipper_range() {
    let sorted = searcher(Some(sort_by("num", SortFieldType::Long)));
    let (two_phase, count) = probe(&sorted, numeric_query());
    assert!(
        !two_phase,
        "the sorted leaf must be served by one document range"
    );
    assert_eq!(count, expected_num(100, 250));

    // Positive control: the unsorted leaf keeps the two-phase iterator, and
    // finds the same documents.
    let unsorted = searcher(None);
    let (two_phase, count) = probe(&unsorted, numeric_query());
    assert!(two_phase, "without an index sort the query stays two-phase");
    assert_eq!(count, expected_num(100, 250));
}

#[test]
fn an_ordinal_range_over_a_leaf_sorted_by_its_field_uses_the_skipper_range() {
    let sorted = searcher(Some(sort_by("tag", SortFieldType::String)));
    let (two_phase, count) = probe(&sorted, ordinal_query());
    assert!(
        !two_phase,
        "the sorted leaf must be served by one document range"
    );
    assert_eq!(count, expected_tag("t050", "t120"));

    let unsorted = searcher(None);
    let (two_phase, count) = probe(&unsorted, ordinal_query());
    assert!(two_phase, "without an index sort the query stays two-phase");
    assert_eq!(count, expected_tag("t050", "t120"));
}

#[test]
fn a_reverse_sorted_leaf_still_finds_every_document_in_range() {
    let sort = Sort::new_fields(vec![SortField::new_reverse(
        Some("num".to_string()),
        SortFieldType::Long,
        true,
    )
    .expect("sort field")])
    .expect("sort");
    let sorted = searcher(Some(sort));
    let (two_phase, count) = probe(&sorted, numeric_query());
    assert!(!two_phase);
    assert_eq!(count, expected_num(100, 250));
}
