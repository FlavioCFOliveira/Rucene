//! Doc values written through `IndexWriter` must survive the round trip.
//!
//! Regression guards for two silent write-path defects found while fixing rmp
//! task #183:
//!
//! 1. `PerFieldDocValuesFormat` recorded `PerFieldDocValuesFormat.format` on
//!    the `FieldInfo` the doc-values writer owned, which in this port is a
//!    *copy* of the one the `.fnm` is written from. Java has a single object
//!    (`IndexingChain`'s `PerField.fieldInfo` is the very object
//!    `FieldInfos.Builder.finish()` hands to the segment), so the attribute
//!    reached the file. Without it, **any** doc-values field written by
//!    `IndexWriter` produced a segment that could not be reopened — the write
//!    reported success and the failure only surfaced on `DirectoryReader.open`.
//!
//! 2. The flush hardcoded `softDelCount = 0` on the new `SegmentCommitInfo`,
//!    where Java computes `flushState.softDelCountOnFlush` from
//!    `IndexingChain.getHasDocValues(softDeletesField)`
//!    (`DocumentsWriterPerThread.java:486-500`). The committed segment then
//!    claimed no soft deletes at all.

use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{Document, NumericDocValuesField, Store, StringField};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{directory_reader, IndexReader, SegmentInfos};
use rucene::search::NO_MORE_DOCS;
use rucene::store::{Directory, FSDirectory};

const SOFT_DELETES_FIELD: &str = "__soft_deletes";

fn fresh_directory(name: &str) -> Arc<dyn Directory> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let tmp = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create the index directory");
    Arc::new(FSDirectory::open(&tmp).expect("open the directory"))
}

#[test]
fn numeric_doc_values_written_by_the_index_writer_are_read_back() {
    let dir = fresh_directory("rucene-183-dv-round-trip");
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .expect("merge policy");
    let writer = IndexWriter::new(Arc::clone(&dir), config).expect("index writer");
    for d in 0..4i64 {
        let mut document = Document::new();
        document.add(Box::new(
            StringField::new("id", format!("doc-{d}"), Store::YES).expect("id"),
        ));
        document.add(Box::new(NumericDocValuesField::new("num", d * 10)));
        writer.add_document(&document).expect("add document");
    }
    writer.commit().expect("commit");

    // Before the fix this call failed with
    // `missing attribute: PerFieldDocValuesFormat.format for field: num`.
    let reader = directory_reader::open(Arc::clone(&dir)).expect("reopen the index");
    assert_eq!(IndexReader::num_docs(reader.as_ref()), 4);

    let leaves = Arc::clone(&reader).leaves();
    let leaf = leaves[0].leaf_reader();
    let mut values = leaf
        .get_numeric_doc_values("num")
        .expect("numeric doc values")
        .expect("the field has doc values");
    let mut read = Vec::new();
    loop {
        let doc = values.next_doc().expect("advance");
        if doc == NO_MORE_DOCS {
            break;
        }
        read.push((doc, values.long_value().expect("value")));
    }
    println!("numeric doc values read back = {read:?}");
    assert_eq!(read, vec![(0, 0), (1, 10), (2, 20), (3, 30)]);
}

#[test]
fn a_flushed_segment_records_how_many_documents_it_soft_deletes() {
    let dir = fresh_directory("rucene-183-soft-del-count");
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .expect("merge policy");
    config.set_soft_deletes_field(Some(SOFT_DELETES_FIELD.to_string()));
    let writer = IndexWriter::new(Arc::clone(&dir), config).expect("index writer");
    for d in 0..6 {
        let mut document = Document::new();
        document.add(Box::new(
            StringField::new("id", format!("doc-{d}"), Store::YES).expect("id"),
        ));
        if d % 2 == 1 {
            document.add(Box::new(NumericDocValuesField::new(SOFT_DELETES_FIELD, 1)));
        }
        writer.add_document(&document).expect("add document");
    }
    writer.commit().expect("commit");

    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).expect("read the commit");
    assert_eq!(infos.size(), 1, "one segment was written");
    let info = infos.info(0);
    println!(
        "segment {} maxDoc={} delCount={} softDelCount={}",
        info.info.name,
        info.info.max_doc().expect("maxDoc"),
        info.get_del_count(),
        info.get_soft_del_count()
    );
    assert_eq!(
        info.get_soft_del_count(),
        3,
        "documents 1, 3 and 5 carry the soft-deletes field"
    );
    assert_eq!(info.get_del_count(), 0, "nothing is hard-deleted");
}

#[test]
fn a_segment_with_no_soft_deletes_field_configured_records_none() {
    let dir = fresh_directory("rucene-183-soft-del-count-off");
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .expect("merge policy");
    let writer = IndexWriter::new(Arc::clone(&dir), config).expect("index writer");
    for d in 0..4 {
        let mut document = Document::new();
        document.add(Box::new(
            StringField::new("id", format!("doc-{d}"), Store::YES).expect("id"),
        ));
        document.add(Box::new(NumericDocValuesField::new(SOFT_DELETES_FIELD, 1)));
        writer.add_document(&document).expect("add document");
    }
    writer.commit().expect("commit");

    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).expect("read the commit");
    let info = infos.info(0);
    assert_eq!(
        info.get_soft_del_count(),
        0,
        "the field is just a doc-values field until the config names it, \
         exactly as `DocumentsWriterPerThread.flush` decides"
    );
}
