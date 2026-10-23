//! A kNN search must honour the documents it is allowed to return.
//!
//! Regression guard for rmp task #205. `Lucene99HnswVectorsReader.search`
//! (`Lucene99HnswVectorsReader.java:331-361`) puts its `AcceptDocs` to three
//! uses: it caps the filtered document count at the graph size, it turns the
//! accepted documents into the ordinals the graph search may visit, and it
//! compares that filtered count against
//! `HnswGraphSearcher.expectedVisitedNodes` to decide between searching the
//! graph and scanning every vector. This port ignored the parameter outright,
//! so a kNN search returned documents that had been deleted and documents the
//! caller's filter excluded.
//!
//! Because the reader now reads the accept docs, `ExitableDirectoryReader`'s
//! `ExitableAcceptDocs` is reachable from inside the crate for the first time,
//! and the last two tests cover it end to end.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{Document, IntPoint, KnnFloatVectorField, Store, StringField};
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, ExitableDirectoryReaderWrapper,
};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{directory_reader, DirectoryReader, LeafReader, QueryTimeout};
use rucene::search::knn::{KnnCollector, TopKnnCollector};
use rucene::search::{from_iterator_supplier, from_live_docs, range, DocIdSetIterator, Query};
use rucene::store::{Directory, FSDirectory};

/// Enough documents that `Lucene99HnswVectorsWriter.shouldCreateGraph` writes a
/// graph: it needs `numNodes > log(numNodes) * 100`, which 1000 clears and 600
/// does not. The searches below therefore run on the graph path.
const DOC_COUNT: i32 = 1_000;

const VECTOR_FIELD: &str = "vector";

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
struct AlwaysTimesOut {
    calls: AtomicUsize,
}

impl AlwaysTimesOut {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }
}

impl QueryTimeout for AlwaysTimesOut {
    fn should_exit(&self) -> bool {
        self.calls.fetch_add(1, Ordering::Relaxed);
        true
    }
}

/// Document `d` holds the vector `[d, 0]`, so the neighbours of `[0, 0]` are
/// documents 0, 1, 2, … in that order.
fn doc(d: i32) -> Document {
    let mut document = Document::new();
    document.add(Box::new(
        StringField::new("id", format!("doc-{d}"), Store::NO).expect("id"),
    ));
    document.add(Box::new(IntPoint::new("rank", &[d]).expect("rank")));
    document.add(Box::new(
        KnnFloatVectorField::with_euclidean(VECTOR_FIELD, &[d as f32, 0.0]).expect("vector"),
    ));
    document
}

/// Writes `DOC_COUNT` documents into a fresh directory, deletes the documents
/// whose rank falls in `deleted`, and returns a reader over the commit.
fn build_index(name: &str, deleted: Option<(i32, i32)>) -> Arc<dyn DirectoryReader> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let tmp = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create the index directory");

    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(&tmp).expect("open the directory"));
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .expect("no merges, so the documents keep the ids they were written with");
    let writer = IndexWriter::new(Arc::clone(&dir), config).expect("index writer");
    for d in 0..DOC_COUNT {
        writer.add_document(&doc(d)).expect("add document");
    }
    if let Some((lo, hi)) = deleted {
        writer
            .commit()
            .expect("commit the documents before deleting");
        let query: Arc<dyn Query> =
            IntPoint::new_range_query("rank", &[lo], &[hi]).expect("rank range query");
        writer
            .delete_documents_by_queries(vec![query])
            .expect("delete by query");
    }
    writer.commit().expect("commit");

    directory_reader::open(dir).expect("open a directory reader")
}

/// Runs a kNN search for `[0, 0]` over the one leaf of `reader`, returning the
/// documents it collects, best first.
fn search(leaf: &Arc<dyn LeafReader>, k: i32) -> Vec<i32> {
    let mut collector = TopKnnCollector::new(k, i64::MAX);
    let mut accept_docs =
        from_live_docs(leaf.get_live_docs(), leaf.max_doc()).expect("accept docs");
    leaf.search_nearest_vectors(VECTOR_FIELD, &[0.0, 0.0], &mut collector, &mut accept_docs)
        .expect("the search must succeed");
    collector
        .top_docs()
        .score_docs
        .iter()
        .map(|score_doc| score_doc.doc)
        .collect()
}

#[test]
fn a_deleted_document_is_not_returned_by_a_knn_search() {
    let reader = build_index("rucene-205-deletes", Some((0, 1)));
    let leaves = Arc::clone(&reader).leaves();
    assert_eq!(
        leaves.len(),
        1,
        "one segment, so the doc ids are the leaf's"
    );
    let leaf = leaves[0].leaf_reader();
    assert_eq!(
        leaf.max_doc() - leaf.num_docs(),
        2,
        "documents 0 and 1 must have been deleted"
    );

    let docs = search(&leaf, 3);
    assert!(
        !docs.contains(&0) && !docs.contains(&1),
        "a deleted document came back from the kNN search: {docs:?}"
    );
    assert_eq!(
        docs.first().copied(),
        Some(2),
        "the nearest surviving vector is document 2, got {docs:?}"
    );
}

#[test]
fn a_knn_search_over_a_live_index_still_returns_the_nearest_document() {
    let reader = build_index("rucene-205-no-deletes", None);
    let leaves = Arc::clone(&reader).leaves();
    let leaf = leaves[0].leaf_reader();
    assert_eq!(leaf.max_doc(), leaf.num_docs());

    let docs = search(&leaf, 3);
    assert_eq!(
        docs.first().copied(),
        Some(0),
        "with nothing deleted the nearest vector is document 0, got {docs:?}"
    );
}

#[test]
fn a_caller_filter_is_honoured_by_a_knn_search() {
    let reader = build_index("rucene-205-filter", None);
    let leaves = Arc::clone(&reader).leaves();
    let leaf = leaves[0].leaf_reader();

    // Half the index passes, which keeps the filtered count well above the
    // nodes an unfiltered graph search is expected to visit, so the search
    // stays on the graph path.
    let mut accept_docs = from_iterator_supplier(
        || Ok(Box::new(range(500, DOC_COUNT)?) as Box<dyn DocIdSetIterator>),
        None,
        leaf.max_doc(),
    )
    .expect("accept docs");
    let mut collector = TopKnnCollector::new(3, i64::MAX);
    leaf.search_nearest_vectors(VECTOR_FIELD, &[0.0, 0.0], &mut collector, &mut accept_docs)
        .expect("the search must succeed");
    let docs: Vec<i32> = collector
        .top_docs()
        .score_docs
        .iter()
        .map(|score_doc| score_doc.doc)
        .collect();

    assert!(
        docs.iter().all(|doc| *doc >= 500),
        "the filter excludes every document below 500, but the search returned {docs:?}"
    );
    assert_eq!(
        docs.first().copied(),
        Some(500),
        "the nearest accepted vector is document 500, got {docs:?}"
    );
}

#[test]
fn an_exitable_reader_interrupts_a_knn_search_through_the_accept_docs() {
    let reader = build_index("rucene-205-exitable", None);
    let timeout = Arc::new(AlwaysTimesOut::new());
    let exitable =
        ExitableDirectoryReaderWrapper::new(Arc::clone(&timeout) as Arc<dyn QueryTimeout>)
            .wrap(Arc::clone(&reader))
            .expect("wrap as an exitable reader");
    let leaves = Arc::clone(&exitable).leaves();
    let leaf = leaves[0].leaf_reader();

    let mut collector = TopKnnCollector::new(3, i64::MAX);
    let mut accept_docs =
        from_live_docs(leaf.get_live_docs(), leaf.max_doc()).expect("accept docs");
    let error = leaf
        .search_nearest_vectors(VECTOR_FIELD, &[0.0, 0.0], &mut collector, &mut accept_docs)
        .expect_err("the timeout has fired, so the search must be interrupted");

    assert!(
        error
            .to_string()
            .contains("The request took too long to search nearest vectors"),
        "the error must carry the ExitableAcceptDocs wording, got: {error}"
    );
    assert!(
        timeout.calls() > 0,
        "the search must have consulted the accept docs, which is what checks the timeout"
    );
}

#[test]
fn an_exitable_reader_hides_nothing_when_the_timeout_never_fires() {
    let reader = build_index("rucene-205-exitable-ok", None);
    let exitable = ExitableDirectoryReaderWrapper::new(Arc::new(NeverTimesOut))
        .wrap(Arc::clone(&reader))
        .expect("wrap as an exitable reader");
    let leaves = Arc::clone(&exitable).leaves();
    let leaf = leaves[0].leaf_reader();

    let docs = search(&leaf, 3);
    assert_eq!(
        docs.first().copied(),
        Some(0),
        "wrapping must not change what the search finds, got {docs:?}"
    );
}
