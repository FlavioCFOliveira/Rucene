//! A `FilterDirectoryReader` must expose its **wrapped** leaves.
//!
//! Regression guard for rmp task #183. Java's constructor passes
//! `wrapper.wrap(in.getSequentialSubReaders())` to `super`
//! (`FilterDirectoryReader.java:91-93`), so `getSequentialSubReaders()` — and
//! therefore `leaves()`, and therefore everything that searches through the
//! filtered reader — sees the wrapped leaves. This port used to answer with the
//! *delegate's* unwrapped leaves, which made two public features silent
//! no-ops: `ExitableDirectoryReader` checked no timeout, and
//! `SoftDeletesDirectoryReaderWrapper` filtered no soft deletes.
//!
//! These tests fail if the leaves go back to being returned unwrapped.

use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{Document, NumericDocValuesField, Store, StringField, TextField};
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, ExitableDirectoryReaderWrapper, SoftDeletesDirectoryReaderWrapper,
};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{directory_reader, DirectoryReader, IndexReader, QueryTimeout, Term};
use rucene::search::{IndexSearcher, Query, TermQuery};
use rucene::store::{Directory, FSDirectory};
use rucene::util::BytesRef;

/// A timeout that has always already fired.
#[derive(Debug)]
struct AlwaysTimesOut;

impl QueryTimeout for AlwaysTimesOut {
    fn should_exit(&self) -> bool {
        true
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

const SOFT_DELETES_FIELD: &str = "__soft_deletes";

fn doc(d: i32, soft_deleted: bool) -> Document {
    let mut document = Document::new();
    document.add(Box::new(
        StringField::new("id", format!("doc-{d}"), Store::YES).expect("id"),
    ));
    document.add(Box::new(
        TextField::new("body", format!("thing number {d}"), Store::YES).expect("body"),
    ));
    if soft_deleted {
        document.add(Box::new(NumericDocValuesField::new(SOFT_DELETES_FIELD, 1)));
    }
    document
}

/// Writes `count` documents into a fresh directory, soft-deleting the odd ones
/// when `soft_delete_odd` is set, and returns a reader over the commit.
fn build_index(name: &str, count: i32, soft_delete_odd: bool) -> Arc<dyn DirectoryReader> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let tmp = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create the index directory");

    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(&tmp).expect("open the directory"));
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .expect("no merges, so the segment layout is the one written");
    if soft_delete_odd {
        // Naming the field is what makes the flush record the segment's
        // `softDelCount`, which `SoftDeletesDirectoryReaderWrapper` checks its
        // own arithmetic against.
        config.set_soft_deletes_field(Some(SOFT_DELETES_FIELD.to_string()));
    }
    let writer = IndexWriter::new(Arc::clone(&dir), config).expect("index writer");
    for d in 0..count {
        writer
            .add_document(&doc(d, soft_delete_odd && d % 2 == 1))
            .expect("add document");
    }
    writer.commit().expect("commit");

    directory_reader::open(dir).expect("open a directory reader")
}

#[test]
fn exitable_directory_reader_times_out_through_its_leaves() {
    let reader = build_index("rucene-183-exitable", 6, false);

    // The bare reader answers normally: the timeout is what must make the
    // difference, not the index.
    let bare_leaves = Arc::clone(&reader).leaves();
    assert_eq!(bare_leaves.len(), 1, "one segment was written");
    assert!(
        bare_leaves[0].leaf_reader().terms("body").is_ok(),
        "the unwrapped leaf reads its terms"
    );

    let exitable = ExitableDirectoryReaderWrapper::new(Arc::new(AlwaysTimesOut))
        .wrap(Arc::clone(&reader))
        .expect("wrap as an exitable reader");

    let leaves = Arc::clone(&exitable).leaves();
    assert_eq!(leaves.len(), 1, "the wrapping preserves the leaf count");

    let leaf = leaves[0].leaf_reader();
    println!("exitable leaf = {leaf}");
    // `ExitableFilterAtomicReader.terms(String)` checks no timeout of its own:
    // it hands out an `ExitableTerms`, and the enumeration that wrapper builds
    // is what checks — see `tests/exitable_directory_reader_iteration.rs`.
    let terms = leaf
        .terms("body")
        .expect("the accessor checks no timeout")
        .expect("the field has terms");
    let enumeration = terms.iterator();
    println!(
        "exitable leaf.terms(\"body\").iterator() = {}",
        match &enumeration {
            Ok(_) => "Ok(enumeration)".to_string(),
            Err(error) => format!("Err({error})"),
        }
    );
    assert!(
        enumeration.is_err(),
        "the timeout has fired, so enumerating the terms of a leaf of the \
         exitable reader must raise; got Ok, which means leaves() handed back \
         the delegate's unwrapped leaf"
    );
    let message = enumeration.err().expect("an error").to_string();
    assert!(
        message.contains("The request took too long to iterate over"),
        "the error must be the ExitingReaderException wording, got: {message}"
    );
}

#[test]
fn exitable_directory_reader_lets_work_through_when_the_timeout_has_not_fired() {
    let reader = build_index("rucene-183-exitable-ok", 6, false);
    let exitable = ExitableDirectoryReaderWrapper::new(Arc::new(NeverTimesOut))
        .wrap(Arc::clone(&reader))
        .expect("wrap as an exitable reader");

    let leaves = Arc::clone(&exitable).leaves();
    let leaf = leaves[0].leaf_reader();
    assert!(
        leaf.terms("body")
            .expect("terms")
            .expect("the field has terms")
            .iterator()
            .is_ok(),
        "with no timeout fired the wrapped leaf reads exactly as the bare one"
    );
    assert_eq!(
        IndexReader::num_docs(exitable.as_ref()),
        6,
        "the exitable wrapping hides no document"
    );
}

#[test]
fn soft_deletes_directory_reader_wrapper_filters_the_soft_deleted_documents() {
    let reader = build_index("rucene-183-soft-deletes", 6, true);

    assert_eq!(
        IndexReader::num_docs(reader.as_ref()),
        6,
        "nothing is hard-deleted, so the bare reader sees every document"
    );

    let wrapped = SoftDeletesDirectoryReaderWrapper::new(SOFT_DELETES_FIELD)
        .wrap(Arc::clone(&reader))
        .expect("wrap as a soft-deletes reader");

    let num_docs = IndexReader::num_docs(wrapped.as_ref());
    println!("soft-deletes wrapped num_docs = {num_docs}");
    assert_eq!(
        num_docs, 3,
        "documents 1, 3 and 5 carry the soft-deletes field and must be hidden"
    );
    assert_eq!(
        IndexReader::max_doc(wrapped.as_ref()),
        6,
        "soft deletes hide documents, they do not remove them"
    );

    let leaves = Arc::clone(&wrapped).leaves();
    assert_eq!(leaves.len(), 1, "one segment was written");
    let leaf = leaves[0].leaf_reader();
    println!("soft-deletes leaf = {leaf}");
    let live_docs = leaf
        .get_live_docs()
        .expect("the wrapped leaf must carry live docs that hide the soft deletes");
    let live: Vec<usize> = (0..6).filter(|d| live_docs.get(*d)).collect();
    println!("live docs = {live:?}");
    assert_eq!(
        live,
        vec![0, 2, 4],
        "only the documents without the soft-deletes field stay live"
    );
    assert_eq!(
        leaf.num_docs(),
        3,
        "the leaf's own live count must agree with the reader's"
    );

    // The consequence a user actually sees: a search through the wrapped
    // reader must not return the soft-deleted documents.
    let term: Arc<dyn Query> = Arc::new(TermQuery::new(Term::new(
        "body",
        BytesRef::new(b"thing".to_vec()),
    )));
    let bare = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)
        .expect("searcher over the bare reader");
    let filtered = IndexSearcher::new(Arc::clone(&wrapped) as Arc<dyn IndexReader>)
        .expect("searcher over the wrapped reader");
    let bare_hits = bare.count(Arc::clone(&term)).expect("count");
    let filtered_hits = filtered.count(Arc::clone(&term)).expect("count");
    println!("count over the bare reader = {bare_hits}, over the wrapped = {filtered_hits}");
    assert_eq!(bare_hits, 6, "every document carries the term");
    assert_eq!(
        filtered_hits, 3,
        "the soft-deleted documents must not be searchable through the wrapper"
    );
}
