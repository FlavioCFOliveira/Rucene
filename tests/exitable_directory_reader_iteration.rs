//! `ExitableDirectoryReader` must keep checking the timeout **during** an
//! iteration, not only when the iteration is handed out.
//!
//! Regression guard for rmp task #199. Lucene's
//! `ExitableDirectoryReader.ExitableFilterAtomicReader` checks no timeout in
//! its accessors: it wraps what it returns — `ExitableTerms`,
//! `ExitableTermsEnum`, `ExitablePointValues`, `ExitablePointTree`,
//! `ExitableIntersectVisitor`, `ExitableAcceptDocs`,
//! `ExitableFloatVectorValues`, `ExitableByteVectorValues` and the doc-values
//! wrappers — so that the check recurs as the caller iterates. This port used
//! to check once, in the accessor, and hand back the underlying object
//! unwrapped, so a long iteration ran to completion however long it took.
//!
//! These tests fail if an iteration goes back to being uninterruptible.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{Document, NumericDocValuesField, Store, StringField, TextField};
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, ExitableDirectoryReaderWrapper,
};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{directory_reader, DirectoryReader, QueryTimeout};
use rucene::search::DocIdSetIterator;
use rucene::store::{Directory, FSDirectory};

/// A timeout that fires from its `fires_after`-th interrogation onwards, so
/// that an iteration crosses it part way through instead of before it starts.
#[derive(Debug)]
struct TimeoutAfter {
    calls: AtomicUsize,
    fires_after: usize,
}

impl TimeoutAfter {
    fn new(fires_after: usize) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            fires_after,
        }
    }
}

impl QueryTimeout for TimeoutAfter {
    fn should_exit(&self) -> bool {
        self.calls.fetch_add(1, Ordering::Relaxed) + 1 >= self.fires_after
    }
}

/// A timeout that never fires.
#[derive(Debug)]
struct NeverTimesOut;

impl QueryTimeout for NeverTimesOut {
    fn should_exit(&self) -> bool {
        false
    }
}

/// A timeout that has always already fired.
#[derive(Debug)]
struct AlwaysTimesOut;

impl QueryTimeout for AlwaysTimesOut {
    fn should_exit(&self) -> bool {
        true
    }
}

const DOC_COUNT: i32 = 2_500;
const COUNT_FIELD: &str = "count";

fn doc(d: i32) -> Document {
    let mut document = Document::new();
    document.add(Box::new(
        StringField::new("id", format!("doc-{d}"), Store::NO).expect("id"),
    ));
    // One distinct term per document, so a terms enumeration is long.
    document.add(Box::new(
        TextField::new("body", format!("t{d:06}"), Store::NO).expect("body"),
    ));
    document.add(Box::new(NumericDocValuesField::new(
        COUNT_FIELD,
        i64::from(d),
    )));
    document
}

/// Writes `DOC_COUNT` documents into a fresh directory and returns a reader
/// over the commit.
fn build_index(name: &str) -> Arc<dyn DirectoryReader> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let tmp = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create the index directory");

    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(&tmp).expect("open the directory"));
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .expect("no merges, so the segment layout is the one written");
    let writer = IndexWriter::new(Arc::clone(&dir), config).expect("index writer");
    for d in 0..DOC_COUNT {
        writer.add_document(&doc(d)).expect("add document");
    }
    writer.commit().expect("commit");

    directory_reader::open(dir).expect("open a directory reader")
}

#[test]
fn a_terms_enumeration_is_interrupted_mid_iteration() {
    let reader = build_index("rucene-199-terms");
    // The timeout has not fired when the enumeration is built (the first
    // interrogation), and fires from the second one onwards — which
    // `ExitableTermsEnum` makes on its sixteenth `next()`.
    let exitable = ExitableDirectoryReaderWrapper::new(Arc::new(TimeoutAfter::new(2)))
        .wrap(Arc::clone(&reader))
        .expect("wrap as an exitable reader");
    let leaves = Arc::clone(&exitable).leaves();
    let leaf = leaves[0].leaf_reader();

    let terms = leaf
        .terms("body")
        .expect("reading the terms of a field must not itself check the timeout")
        .expect("the field has terms");
    let mut enumeration = terms
        .iterator()
        .expect("the timeout has not fired yet, so the enumeration is handed out");

    let mut seen = 0usize;
    let outcome = loop {
        match enumeration.next() {
            Ok(Some(_)) => seen += 1,
            Ok(None) => break Ok(()),
            Err(error) => break Err(error),
        }
    };

    let error = outcome.expect_err(
        "the timeout fires during the enumeration, so it must be interrupted rather than run to \
         completion",
    );
    println!("terms enumeration stopped after {seen} terms with: {error}");
    assert!(
        error
            .to_string()
            .contains("The request took too long to iterate over terms"),
        "the error must carry the ExitingReaderException wording for terms, got: {error}"
    );
    assert!(
        seen > 0,
        "the enumeration must have started before the timeout fired"
    );
    assert!(
        seen < DOC_COUNT as usize,
        "the enumeration must stop mid-iteration, but it read all {seen} terms"
    );
}

#[test]
fn a_terms_enumeration_runs_to_completion_when_the_timeout_never_fires() {
    let reader = build_index("rucene-199-terms-ok");
    let exitable = ExitableDirectoryReaderWrapper::new(Arc::new(NeverTimesOut))
        .wrap(Arc::clone(&reader))
        .expect("wrap as an exitable reader");
    let leaves = Arc::clone(&exitable).leaves();
    let terms = leaf_terms(&leaves[0].leaf_reader());

    let mut enumeration = terms.iterator().expect("an enumeration");
    let mut seen = 0usize;
    while enumeration.next().expect("no timeout fires").is_some() {
        seen += 1;
    }
    assert_eq!(
        seen, DOC_COUNT as usize,
        "with no timeout the wrapping hides no term"
    );
}

fn leaf_terms(leaf: &Arc<dyn rucene::index::LeafReader>) -> Box<dyn rucene::index::Terms> {
    leaf.terms("body").expect("terms").expect("the field")
}

#[test]
fn numeric_doc_values_are_interrupted_mid_iteration() {
    let reader = build_index("rucene-199-doc-values");
    // The first check happens on the first document, the second once the
    // iteration has crossed `DOCS_BETWEEN_TIMEOUT_CHECK` (1000) documents.
    let exitable = ExitableDirectoryReaderWrapper::new(Arc::new(TimeoutAfter::new(2)))
        .wrap(Arc::clone(&reader))
        .expect("wrap as an exitable reader");
    let leaves = Arc::clone(&exitable).leaves();
    let leaf = leaves[0].leaf_reader();

    let mut values = leaf
        .get_numeric_doc_values(COUNT_FIELD)
        .expect("reading doc values must not itself check the timeout")
        .expect("the field has doc values");

    let mut seen = 0usize;
    let outcome = loop {
        match values.next_doc() {
            Ok(doc) if doc == rucene::search::NO_MORE_DOCS => break Ok(()),
            Ok(_) => seen += 1,
            Err(error) => break Err(error),
        }
    };

    let error = outcome.expect_err(
        "the timeout fires during the iteration, so it must be interrupted rather than run to \
         completion",
    );
    println!("doc-values iteration stopped after {seen} documents with: {error}");
    assert!(
        error
            .to_string()
            .contains("The request took too long to iterate over doc values"),
        "the error must carry the ExitingReaderException wording for doc values, got: {error}"
    );
    assert!(
        (1..DOC_COUNT as usize).contains(&seen),
        "the iteration must stop mid-iteration, having read {seen} of {DOC_COUNT} documents"
    );
}

#[test]
fn the_accessors_check_no_timeout_of_their_own() {
    let reader = build_index("rucene-199-accessors");
    let exitable = ExitableDirectoryReaderWrapper::new(Arc::new(AlwaysTimesOut))
        .wrap(Arc::clone(&reader))
        .expect("wrap as an exitable reader");
    let leaves = Arc::clone(&exitable).leaves();
    let leaf = leaves[0].leaf_reader();

    // Lucene's `ExitableFilterAtomicReader` overrides none of these to check
    // the timeout: `terms(String)` wraps, and the wrapper is what checks.
    assert!(
        leaf.terms("body").expect("terms").is_some(),
        "terms(String) hands out an ExitableTerms even once the timeout has fired"
    );
    assert!(
        leaf.get_numeric_doc_values(COUNT_FIELD)
            .expect("doc values")
            .is_some(),
        "getNumericDocValues hands out a wrapper even once the timeout has fired"
    );
    assert!(
        leaf.stored_fields().is_ok(),
        "storedFields() is not overridden by ExitableFilterAtomicReader"
    );
    assert!(
        leaf.term_vectors().is_ok(),
        "termVectors() is not overridden by ExitableFilterAtomicReader"
    );

    // The enumeration, on the other hand, checks the timeout as it is built.
    let terms = leaf.terms("body").expect("terms").expect("the field");
    let error = terms
        .iterator()
        .err()
        .expect("ExitableTermsEnum checks the timeout in its constructor");
    assert!(
        error
            .to_string()
            .contains("The request took too long to iterate over terms"),
        "got: {error}"
    );
}
