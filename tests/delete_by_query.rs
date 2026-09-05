//! Delete-by-query regression tests.
//!
//! `IndexWriter::delete_documents_by_queries` buffers each query and
//! `FrozenBufferedUpdates::apply_query_deletes` resolves it against every
//! segment the packet reaches — the port of
//! `org.apache.lucene.index.FrozenBufferedUpdates#applyQueryDeletes`.
//!
//! Before that pass existed the operation was a **silent no-op**: the writer
//! accepted the call, returned a sequence number and deleted nothing. Every
//! assertion here therefore states a positive outcome — which documents
//! survived, how many the segment records as deleted, which generation its
//! `.liv` carries — so that a regression to a no-op fails the suite rather than
//! passing it quietly.
//!
//! The expected values are not this port's own output. Each was measured by
//! running the identical script through a real Apache Lucene Core 10.5.0
//! `IndexWriter` (`tests/fixtures/java-codec-harness`, class
//! `DeleteByQueryFixture`); `tests/portability/delete_by_query.rs` re-runs that
//! comparison live against Java, while this suite pins the numbers so it can
//! run without Maven.
//!
//! # Why the queries are point queries
//!
//! The scripted corpus selects documents with an [`IntPoint`] range query and,
//! for the "delete everything" case, with a [`MatchAllDocsQuery`]. A
//! [`TermQuery`](rucene::search::TermQuery) would be the more obvious choice,
//! but scoring one against an on-disk segment needs
//! `SegmentTermsEnum::postings`, which is not ported yet: the block-tree terms
//! dictionary can seek to a term but cannot yet hand out its postings. That gap
//! is independent of delete-by-query — it stops delete-by-*term* against a
//! flushed segment as well — and the point and match-all paths exercise exactly
//! the same `apply_query_deletes` code.

use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{Document, IntPoint, Store, StringField, TextField};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::SegmentInfos;
use rucene::search::{MatchAllDocsQuery, Query};
use rucene::store::{Directory, FSDirectory};

// ---------------------------------------------------------------------------
// The scripted corpus, mirroring DeleteByQueryFixture.doc(int)
// ---------------------------------------------------------------------------

/// One document: a stored `id`, a stored `body` already lower case and space
/// separated, and an `IntPoint` `rank` holding the document's ordinal, so a
/// point range query selects any contiguous run of the corpus.
fn doc(d: i32) -> Document {
    let mut document = Document::new();
    document.add(Box::new(
        StringField::new("id", format!("doc-{d}"), Store::YES).expect("id field"),
    ));
    document.add(Box::new(
        TextField::new("body", format!("hello world number {d}"), Store::YES).expect("body field"),
    ));
    document.add(Box::new(IntPoint::new("rank", &[d]).expect("rank field")));
    document
}

/// `IntPoint.newRangeQuery("rank", lo, hi)`, both bounds inclusive.
fn rank(lo: i32, hi: i32) -> Arc<dyn Query> {
    IntPoint::new_range_query("rank", &[lo], &[hi]).expect("rank range query")
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// What one run reports, in the same shape `DeleteByQueryFixture` prints.
#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    /// `id` of every live document, in doc-id order.
    live: Vec<String>,
    /// `id` of every deleted document still present in a segment.
    deleted: Vec<String>,
    num_docs: i32,
    max_doc: i32,
    /// `(segment name, delCount, delGen)` for each segment of the last commit.
    segments: Vec<(String, i32, i64)>,
    /// The sorted directory listing after the last commit.
    files: Vec<String>,
}

fn ensure_codec() {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
}

/// Replays one scripted case and reports what a reader then sees.
fn run(case: &str) -> Outcome {
    ensure_codec();
    let tmp = tempfile::tempdir().expect("temp dir");
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(tmp.path()).expect("open directory"));
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .expect("set merge policy");
    let writer = IndexWriter::new(Arc::clone(&dir), config).expect("open writer");

    // Mirrors DeleteByQueryFixture.script(IndexWriter, String) exactly.
    match case {
        "byquery" => {
            for d in 0..6 {
                writer.add_document(&doc(d)).expect("add document");
            }
            writer.commit().expect("commit");
            writer
                .delete_documents_by_queries(vec![rank(0, 2)])
                .expect("delete by query");
            writer.commit().expect("commit");
        }
        "multi" => {
            for d in 0..6 {
                writer.add_document(&doc(d)).expect("add document");
            }
            writer.commit().expect("commit");
            writer
                .delete_documents_by_queries(vec![rank(1, 1), rank(3, 3)])
                .expect("delete by query");
            writer.commit().expect("commit");
        }
        "nomatch" => {
            for d in 0..5 {
                writer.add_document(&doc(d)).expect("add document");
            }
            writer.commit().expect("commit");
            writer
                .delete_documents_by_queries(vec![rank(99, 99)])
                .expect("delete by query");
            writer.commit().expect("commit");
        }
        "pending" => {
            for d in 0..4 {
                writer.add_document(&doc(d)).expect("add document");
            }
            writer
                .delete_documents_by_queries(vec![rank(0, 9)])
                .expect("delete by query");
            for d in 4..6 {
                writer.add_document(&doc(d)).expect("add document");
            }
            writer.commit().expect("commit");
        }
        "repeat" => {
            for d in 0..6 {
                writer.add_document(&doc(d)).expect("add document");
            }
            writer.commit().expect("commit");
            writer
                .delete_documents_by_queries(vec![rank(0, 2)])
                .expect("delete by query");
            writer.commit().expect("commit");
            writer
                .delete_documents_by_queries(vec![rank(0, 2)])
                .expect("repeat the same delete by query");
            writer.commit().expect("commit");
        }
        "all" => {
            for d in 0..6 {
                writer.add_document(&doc(d)).expect("add document");
            }
            writer.commit().expect("commit");
            writer
                .delete_documents_by_queries(vec![Arc::new(MatchAllDocsQuery::instance())])
                .expect("delete every document by query");
            writer.commit().expect("commit");
        }
        other => panic!("unknown case {other:?}"),
    }
    writer.close().expect("close writer");

    report(&dir)
}

/// Reports on the index at `dir` using Rucene's own readers.
fn report(dir: &Arc<dyn Directory>) -> Outcome {
    let mut files = dir.list_all().expect("list directory");
    files.sort();

    let infos = SegmentInfos::read_latest_commit(Arc::clone(dir)).expect("read latest commit");
    let segments = infos
        .iter()
        .map(|sci| {
            (
                sci.info.name.clone(),
                sci.get_del_count(),
                sci.get_del_gen(),
            )
        })
        .collect();

    let reader = rucene::index::directory_reader::open(Arc::clone(dir)).expect("open reader");
    let stored = reader.stored_fields().expect("stored fields");
    let mut live = Vec::new();
    let mut deleted = Vec::new();
    for leaf in Arc::clone(&reader).leaves() {
        let live_docs = leaf.leaf_reader().get_live_docs();
        for i in 0..leaf.leaf_reader().max_doc() {
            let id = stored
                .document(leaf.doc_base() + i)
                .expect("stored document")
                .get_field("id")
                .and_then(|f| f.string_value())
                .unwrap_or_else(|| "null".to_string());
            match &live_docs {
                Some(bits) if !bits.get(i as usize) => deleted.push(id),
                _ => live.push(id),
            }
        }
    }

    Outcome {
        live,
        deleted,
        num_docs: reader.num_docs(),
        max_doc: reader.max_doc(),
        segments,
        files,
    }
}

fn ids(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| (*s).to_string()).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// One query against an already-flushed segment deletes every document it
/// matches, and only those.
///
/// Lucene 10.5.0 reports `numDocs=3 maxDoc=6`, `live=doc-3,doc-4,doc-5`,
/// `deleted=doc-0,doc-1,doc-2` and `segment=_0 delCount=3 delGen=1` with
/// `_0_1.liv` in the directory.
#[test]
fn delete_by_query_deletes_exactly_the_matching_documents() {
    let out = run("byquery");
    assert_eq!(
        out.live,
        ids(&["doc-3", "doc-4", "doc-5"]),
        "the documents outside the range must survive; got {out:?}"
    );
    assert_eq!(
        out.deleted,
        ids(&["doc-0", "doc-1", "doc-2"]),
        "exactly the three documents in rank [0,2] must be deleted; got {out:?}"
    );
    assert_eq!((out.num_docs, out.max_doc), (3, 6), "{out:?}");
    assert_eq!(
        out.segments,
        vec![("_0".to_string(), 3, 1)],
        "the segment must record three deletions at delGen 1; got {out:?}"
    );
    assert!(
        out.files.contains(&"_0_1.liv".to_string()),
        "the deletions must be written to _0_1.liv; got {:?}",
        out.files
    );
}

/// Every query of a single `deleteDocuments(Query...)` call is applied.
#[test]
fn delete_by_query_applies_every_query_of_one_call() {
    let out = run("multi");
    assert_eq!(
        out.live,
        ids(&["doc-0", "doc-2", "doc-4", "doc-5"]),
        "{out:?}"
    );
    assert_eq!(out.deleted, ids(&["doc-1", "doc-3"]), "{out:?}");
    assert_eq!((out.num_docs, out.max_doc), (4, 6), "{out:?}");
    assert_eq!(out.segments, vec![("_0".to_string(), 2, 1)], "{out:?}");
}

/// A query that matches nothing deletes nothing and writes no `.liv`.
///
/// This is the one case a no-op implementation would also pass, which is why it
/// is never asserted on its own: it pins the *other* half of "exactly the
/// matching documents and no others".
#[test]
fn delete_by_query_matching_nothing_changes_nothing() {
    let out = run("nomatch");
    assert_eq!(
        out.live,
        ids(&["doc-0", "doc-1", "doc-2", "doc-3", "doc-4"]),
        "{out:?}"
    );
    assert!(out.deleted.is_empty(), "{out:?}");
    assert_eq!((out.num_docs, out.max_doc), (5, 5), "{out:?}");
    assert_eq!(
        out.segments,
        vec![("_0".to_string(), 0, -1)],
        "no deletion means the segment keeps delGen -1; got {out:?}"
    );
    assert!(
        !out.files.iter().any(|f| f.ends_with(".liv")),
        "no .liv may be written; got {:?}",
        out.files
    );
}

/// A delete issued while documents are still buffered in RAM becomes a
/// segment-private packet, and its `docIDUpto` limit spares the documents added
/// afterwards.
///
/// The query matches every rank in the corpus, but `doc-4` and `doc-5` were
/// added *after* the delete: Lucene keeps them, reporting `live=doc-4,doc-5`
/// and `delCount=4`. An implementation that ignored the limit would delete the
/// whole segment instead.
#[test]
fn delete_by_query_respects_the_segment_private_doc_id_limit() {
    let out = run("pending");
    assert_eq!(
        out.live,
        ids(&["doc-4", "doc-5"]),
        "the documents added after the delete must survive it; got {out:?}"
    );
    assert_eq!(
        out.deleted,
        ids(&["doc-0", "doc-1", "doc-2", "doc-3"]),
        "{out:?}"
    );
    assert_eq!((out.num_docs, out.max_doc), (2, 6), "{out:?}");
    assert_eq!(out.segments, vec![("_0".to_string(), 4, 1)], "{out:?}");
}

/// Re-issuing a delete that already ran changes nothing: a document is deleted
/// at most once.
///
/// Lucene leaves `delCount=3` and the `.liv` at generation 1, because no
/// document made the live-to-deleted transition in the second round.
#[test]
fn delete_by_query_does_not_delete_an_already_deleted_document_twice() {
    let out = run("repeat");
    assert_eq!(out.live, ids(&["doc-3", "doc-4", "doc-5"]), "{out:?}");
    assert_eq!(out.deleted, ids(&["doc-0", "doc-1", "doc-2"]), "{out:?}");
    assert_eq!((out.num_docs, out.max_doc), (3, 6), "{out:?}");
    assert_eq!(
        out.segments,
        vec![("_0".to_string(), 3, 1)],
        "the repeated delete must not advance delCount or delGen; got {out:?}"
    );
    assert!(
        out.files.contains(&"_0_1.liv".to_string()) && !out.files.contains(&"_0_2.liv".to_string()),
        "no second .liv generation may be written; got {:?}",
        out.files
    );
}

/// A query that matches everything empties the segment, and a segment that lost
/// its last live document is dropped from the commit outright.
///
/// Lucene 10.5.0 reports `numDocs=0 maxDoc=0` and a directory holding only
/// `segments_2` and `write.lock`.
#[test]
fn delete_by_query_matching_everything_drops_the_segment() {
    let out = run("all");
    assert!(out.live.is_empty(), "{out:?}");
    assert!(out.deleted.is_empty(), "{out:?}");
    assert_eq!((out.num_docs, out.max_doc), (0, 0), "{out:?}");
    assert!(
        out.segments.is_empty(),
        "a wholly deleted segment must be dropped from the commit; got {out:?}"
    );
    assert_eq!(
        out.files,
        ids(&["segments_2", "write.lock"]),
        "the segment's files must be gone; got {out:?}"
    );
}
