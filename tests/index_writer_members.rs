//! The `IndexWriter`-side members task #186 closed, exercised end to end.
//!
//! Each of these is a package-private or test-only member of
//! `org.apache.lucene.index.IndexWriter`, `IndexFileDeleter` or `SegmentInfos`
//! that the port did not have. They are grouped here because they only mean
//! anything against a live writer: `isDeleterClosed()` is a question about a
//! writer's deleter, `hasChangesInRam()` about its buffers, `waitForMerges()`
//! about its merge queue, and `addEstimatedBytesToMerge(OneMerge)` about the
//! segments a live index holds.
//!
//! What each one must do is taken from Apache Lucene Core 10.5.0:
//!
//! * `IndexWriter.hasChangesInRam()` — `IndexWriter.java:4211-4214`
//! * `IndexWriter.isDeleterClosed()` — `IndexWriter.java:5944-5946`
//! * `IndexWriter.getDocsWriter()` — `IndexWriter.java:4439-4442`
//! * `IndexWriter.addEstimatedBytesToMerge(OneMerge)` — `IndexWriter.java:4916-4930`
//! * `IndexWriter.waitForMerges()` — `IndexWriter.java:2845-2869`
//! * `IndexFileDeleter.ensureOpen()` / `isClosed()` — `IndexFileDeleter.java:382-403`
//! * `SegmentInfos.createBackupSegmentInfos()` — `SegmentInfos.java:1123-1130`

use std::sync::Arc;

use rucene::document::{Document, Store, StringField};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::OneMerge;
use rucene::index::SegmentInfos;
use rucene::store::{Directory, RamDirectory};

/// The integration crate does not share the library's test-time codec registry,
/// so every entry point registers the default codec before writing an index.
fn ensure_codec() {
    let _ = rucene::codecs::register_codec("Lucene104", rucene::codecs::Lucene104Codec::new());
}

fn add_doc(writer: &IndexWriter, id: &str) {
    let mut doc = Document::new();
    doc.add(Box::new(
        StringField::new("id", id.to_string(), Store::YES).unwrap(),
    ));
    writer.add_document(&doc).unwrap();
}

fn writer_on_ram() -> (Arc<dyn Directory>, IndexWriter) {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    (dir, writer)
}

#[test]
fn has_changes_in_ram_follows_the_buffers_and_not_the_commit() {
    let (_dir, writer) = writer_on_ram();
    assert!(
        !writer.has_changes_in_ram(),
        "a fresh writer has buffered nothing"
    );

    add_doc(&writer, "a");
    assert!(
        writer.has_changes_in_ram(),
        "a buffered document is a change in RAM"
    );

    writer.commit().unwrap();
    assert!(
        !writer.has_changes_in_ram(),
        "committing flushes the buffers, so nothing is left in RAM"
    );

    // `hasUncommittedChanges()` is `changeCount != lastCommitChangeCount ||
    // hasChangesInRam()`, so the two must not be the same question: after a
    // commit both are false, but a delete that has not been applied leaves
    // only the RAM half true.
    writer
        .delete_documents_by_terms(vec![rucene::index::Term::new(
            "id",
            rucene::util::BytesRef::new(b"a".to_vec()),
        )])
        .unwrap();
    assert!(
        writer.has_changes_in_ram(),
        "a buffered delete is a change in RAM even though no document was added"
    );
    writer.close().unwrap();
}

#[test]
fn the_deleter_is_open_until_the_writer_closes() {
    let (_dir, writer) = writer_on_ram();
    assert!(
        !writer.is_deleter_closed().unwrap(),
        "a live writer's deleter is open"
    );

    add_doc(&writer, "a");
    writer.commit().unwrap();
    assert!(
        !writer.is_deleter_closed().unwrap(),
        "committing does not close the deleter"
    );

    writer.close().unwrap();
    assert!(
        writer.is_deleter_closed().unwrap(),
        "closing the writer closes its deleter: `IndexFileDeleter.ensureOpen()` \
         calls `writer.ensureOpen(false)`, which fails once `closed` is set"
    );
}

#[test]
fn the_docs_writer_is_the_writers_own_buffering_half() {
    let (_dir, writer) = writer_on_ram();
    let docs_writer = writer.get_docs_writer();
    assert!(
        !docs_writer.any_changes(),
        "the writer and its DocumentsWriter must agree that nothing is buffered"
    );

    add_doc(&writer, "a");
    assert!(
        writer.get_docs_writer().any_changes(),
        "the accessor hands back the live DocumentsWriter, not a copy"
    );
    writer.close().unwrap();
}

#[test]
fn the_merge_estimate_discounts_the_deleted_fraction() {
    let (dir, writer) = writer_on_ram();
    for i in 0..8 {
        add_doc(&writer, &format!("doc-{i}"));
    }
    writer.commit().unwrap();
    writer.close().unwrap();

    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    assert!(infos.size() > 0, "the index must hold a segment to size");

    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    let mut merge = OneMerge::new(infos.as_slice().to_vec()).unwrap();
    assert_eq!(merge.estimated_merge_bytes, 0);
    assert_eq!(merge.total_merge_bytes, 0);

    writer.add_estimated_bytes_to_merge(&mut merge).unwrap();

    let expected_total: i64 = infos
        .as_slice()
        .iter()
        .map(|info| info.clone().size_in_bytes().unwrap())
        .sum();
    assert_eq!(
        merge.total_merge_bytes, expected_total,
        "every segment contributes its whole size to the total"
    );
    assert_eq!(
        merge.estimated_merge_bytes, expected_total,
        "with no deletions the estimate is the total: `delRatio` is 0"
    );
    writer.close().unwrap();
}

#[test]
fn waiting_for_merges_returns_once_nothing_is_queued() {
    let (_dir, writer) = writer_on_ram();
    for i in 0..4 {
        add_doc(&writer, &format!("doc-{i}"));
    }
    writer.commit().unwrap();

    // Nothing is pending, so this must return rather than poll for ever.
    writer.wait_for_merges().unwrap();

    writer.close().unwrap();
    assert!(
        writer.wait_for_merges().is_err(),
        "`waitForMerges` calls `ensureOpen(false)`, so a closed writer is refused"
    );
}

#[test]
fn the_backup_segment_list_is_independent_of_the_live_one() {
    let (dir, writer) = writer_on_ram();
    add_doc(&writer, "a");
    writer.commit().unwrap();
    writer.close().unwrap();

    let mut infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    let backup = infos.create_backup_segment_infos();
    assert_eq!(backup.len(), infos.size());
    assert_eq!(backup[0].info.name, infos.info(0).info.name);

    // Java clones each `SegmentCommitInfo` so that a later mutation of the live
    // list cannot reach the backup. Advancing the live commit's delete
    // generation must leave the backup where it was.
    let before = backup[0].get_next_del_gen();
    infos.iter_mut().next().unwrap().advance_del_gen();
    assert_eq!(
        backup[0].get_next_del_gen(),
        before,
        "the backup holds its own copies, not borrows of the live commits"
    );
    assert_ne!(
        infos.info(0).get_next_del_gen(),
        before,
        "the live commit did advance, so the check above is not vacuous"
    );
}
