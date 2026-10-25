//! `IndexWriter` over an existing index, and soft updates through the writer.
//!
//! Pins two writer defects against Apache Lucene Core 10.5.0:
//!
//! * **rmp #236.** The constructor started from an empty `FieldNumbers`
//!   instead of loading the field numbers of the index it opened
//!   (`IndexWriter.java:1129`, `getFieldNumberMap`, `:1275-1286`), so a doc-values
//!   update on a reopened index failed with "field doesn't exist".
//! * **rmp #237.** `soft_update_document` failed at commit with "cannot
//!   configure [soft] as soft-deletes; this index uses [soft] as
//!   non-soft-deletes already".

use std::sync::Arc;

use rucene::codecs::Lucene104Codec;
use rucene::document::{
    Document, Field, FieldType, NumericDocValuesField, NumericValue, Store, StringField,
};
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, SoftDeletesDirectoryReaderWrapper,
};
use rucene::index::index_reader::IndexReader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::terms::Term;
use rucene::index::{
    directory_reader, DirectoryReader, DocValuesType, SegmentCommitInfo, SegmentInfos,
};
use rucene::search::NO_MORE_DOCS;
use rucene::store::{Directory, RamDirectory};
use rucene::util::BytesRef;

fn ensure_codec() {
    let _ = rucene::codecs::register_codec("Lucene104", Lucene104Codec::new());
}

fn id_term(id: &str) -> Term {
    Term::new("id", BytesRef::new(id.as_bytes().to_vec()))
}

fn doc(id: &str, price: i64) -> Document {
    let mut d = Document::new();
    d.add(Box::new(
        StringField::new("id", id.to_string(), Store::NO).unwrap(),
    ));
    d.add(Box::new(NumericDocValuesField::new("price", price)));
    d
}

/// Every `(doc, value)` of the numeric doc-values field `field`, leaf by leaf.
fn numeric_values(reader: &Arc<dyn DirectoryReader>, field: &str) -> Vec<i64> {
    let mut out = Vec::new();
    for leaf in Arc::clone(reader).leaves() {
        let leaf = leaf.leaf_reader();
        let Some(mut values) = leaf.get_numeric_doc_values(field).unwrap() else {
            continue;
        };
        loop {
            let doc = values.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                break;
            }
            out.push(values.long_value().unwrap());
        }
    }
    out
}

#[test]
fn a_reopened_writer_updates_doc_values_of_an_existing_field() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    writer.add_document(&doc("a", 1)).unwrap();
    writer.add_document(&doc("b", 2)).unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();

    let mut config = IndexWriterConfig::new();
    config.set_open_mode(OpenMode::APPEND).unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    // The field numbers come from the index, so the update finds the field.
    writer
        .update_numeric_doc_value(id_term("b"), "price", 42)
        .expect("the reopened writer knows the field");
    let names = writer.get_field_number_map().get_field_names();
    assert!(names.contains("id") && names.contains("price"));
    writer.commit().unwrap();
    writer.close().unwrap();

    let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
    assert_eq!(numeric_values(&reader, "price"), vec![1, 42]);
    IndexReader::close(reader.as_ref()).unwrap();
}

#[test]
fn soft_update_document_commits_and_leaves_one_live_document() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config.set_soft_deletes_field(Some("soft".to_string()));
    // No merge, so the soft-deleted version cannot be merged away before the
    // reader looks for it.
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    writer.add_document(&doc("a", 1)).unwrap();
    // `new NumericDocValuesField("soft", 1)`, as the `Field` the writer takes.
    let mut numeric = FieldType::new();
    numeric.set_doc_values_type(DocValuesType::NUMERIC).unwrap();
    numeric.freeze();
    let soft = Field::new_with_number("soft", NumericValue::Long(1), numeric).unwrap();
    writer
        .soft_update_document(id_term("a"), &doc("a", 2), &[soft])
        .unwrap();
    writer.commit().expect("the soft update commits");
    writer.close().unwrap();

    // The commit records the soft delete in the segment's counts.
    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    assert_eq!(infos.size(), 1);
    assert_eq!(
        infos.info(0).get_soft_del_count(),
        1,
        "one soft-deleted document"
    );
    assert_eq!(infos.info(0).get_del_count(), 0, "and no hard delete");

    let inner = directory_reader::open(Arc::clone(&dir)).unwrap();
    assert_eq!(inner.max_doc(), 2, "the old version stays in the index");
    let reader = SoftDeletesDirectoryReaderWrapper::new("soft")
        .wrap(Arc::clone(&inner))
        .unwrap();
    assert_eq!(reader.num_docs(), 1, "only the new version is live");
    IndexReader::close(reader.as_ref()).unwrap();
}

/// A flushed segment whose documents all soft-deleted themselves at indexing
/// time is dropped when it is published.
///
/// `IndexWriter.publishFlushedSegment` checks such a segment — the soft-deletes
/// field is present with no doc-values generation — for being fully deleted
/// and drops it (`IndexWriter.java:2971-2999`).
#[test]
fn a_flushed_segment_of_self_soft_deleted_documents_is_dropped() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = IndexWriterConfig::new();
    config.set_soft_deletes_field(Some("soft".to_string()));
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    writer.add_document(&doc("live", 1)).unwrap();
    writer.commit().unwrap();
    for id in ["t1", "t2"] {
        let mut tombstone = doc(id, 0);
        tombstone.add(Box::new(NumericDocValuesField::new("soft", 1)));
        writer.add_document(&tombstone).unwrap();
    }
    writer.commit().unwrap();
    writer.close().unwrap();

    let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
    assert_eq!(
        Arc::clone(&reader).leaves().len(),
        1,
        "the segment of tombstones was dropped"
    );
    assert_eq!(reader.max_doc(), 1);
    IndexReader::close(reader.as_ref()).unwrap();
}

// ---------------------------------------------------------------------------
// rmp #262: numDeletesToMerge asks the merge policy, with a reader
// ---------------------------------------------------------------------------

/// `SoftDeletesToMergeFixture.doc(int, int)`.
fn retention_doc(i: i32, version: i64) -> Document {
    let mut d = Document::new();
    d.add(Box::new(
        StringField::new("id", format!("id{i}"), Store::NO).unwrap(),
    ));
    d.add(Box::new(
        StringField::new(
            "keep",
            if i % 2 == 0 { "yes" } else { "no" }.to_string(),
            Store::NO,
        )
        .unwrap(),
    ));
    d.add(Box::new(NumericDocValuesField::new("version", version)));
    d
}

/// A policy that records, from inside `find_merges`, what the writer answers
/// for each live segment, then delegates to the retention policy.
struct AskingPolicy {
    inner: Arc<dyn rucene::index::merge_policy::MergePolicy>,
    answers: Arc<std::sync::Mutex<Vec<(String, i32)>>>,
}

impl std::fmt::Display for AskingPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AskingPolicy({})", self.inner)
    }
}

impl rucene::index::merge_policy::MergePolicy for AskingPolicy {
    fn find_merges(
        &self,
        _trigger: rucene::index::merge_policy::MergeTrigger,
        infos: &SegmentInfos,
        context: &dyn rucene::index::merge_policy::MergeContext,
    ) -> rucene::Result<Option<rucene::index::merge_policy::MergeSpecification>> {
        let mut answers = self.answers.lock().unwrap();
        for info in infos.iter() {
            answers.push((info.info.name.clone(), context.num_deletes_to_merge(info)?));
        }
        Ok(None)
    }

    fn find_forced_merges(
        &self,
        infos: &SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &std::collections::HashMap<String, bool>,
        context: &dyn rucene::index::merge_policy::MergeContext,
    ) -> rucene::Result<Option<rucene::index::merge_policy::MergeSpecification>> {
        self.inner
            .find_forced_merges(infos, max_num_segments, segments_to_merge, context)
    }

    fn find_forced_deletes_merges(
        &self,
        infos: &SegmentInfos,
        context: &dyn rucene::index::merge_policy::MergeContext,
    ) -> rucene::Result<Option<rucene::index::merge_policy::MergeSpecification>> {
        self.inner.find_forced_deletes_merges(infos, context)
    }

    fn num_deletes_to_merge_default(&self, info: &SegmentCommitInfo, del_count: i32) -> i32 {
        self.inner.num_deletes_to_merge_default(info, del_count)
    }

    fn num_deletes_to_merge_with_reader(
        &self,
        info: &SegmentCommitInfo,
        del_count: i32,
        reader_supplier: &mut dyn FnMut() -> rucene::Result<Arc<dyn rucene::index::CodecReader>>,
    ) -> rucene::Result<i32> {
        self.inner
            .num_deletes_to_merge_with_reader(info, del_count, reader_supplier)
    }
}

/// What a merge of each segment would reclaim, as `SoftDeletesToMergeFixture`
/// prints it the last time its policy is asked.
fn lucene_num_deletes_to_merge() -> Vec<(String, i32)> {
    let home = std::env::var("HOME").expect("HOME is set");
    let jar = std::path::Path::new(&home)
        .join(".m2/repository/org/apache/lucene/lucene-core/10.5.0/lucene-core-10.5.0.jar");
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         SoftDeletesToMergeFixture.java",
    );
    let output = std::process::Command::new("java")
        .arg("-cp")
        .arg(jar)
        .arg(fixture)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let answers: Vec<(String, i32)> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("numDeletesToMerge "))
        .map(|rest| {
            let mut parts = rest.split(' ');
            let name = parts.next().unwrap().to_string();
            (name, parts.next().unwrap().parse().unwrap())
        })
        .collect();
    let segments = 2;
    answers[answers.len() - segments..].to_vec()
}

/// The writer answers how many deletes a merge would reclaim by asking the
/// merge policy with a reader (`IndexWriter.numDeletesToMerge`,
/// `IndexWriter.java:6246-6262`), so the soft-deleted documents a
/// `SoftDeletesRetentionMergePolicy` retains are not counted, as in Lucene.
#[test]
fn num_deletes_to_merge_excludes_the_soft_deletes_the_retention_query_keeps() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let answers = Arc::new(std::sync::Mutex::new(Vec::new()));
    let retention = Arc::new(
        rucene::index::merge_policy::SoftDeletesRetentionMergePolicy::new(
            "soft",
            Arc::new(|| {
                Arc::new(rucene::search::TermQuery::new(Term::new(
                    "keep",
                    BytesRef::new(b"yes".to_vec()),
                ))) as Arc<dyn rucene::search::Query>
            }),
            Arc::new(NoMergePolicy::new()),
        ),
    );
    let mut config = IndexWriterConfig::new();
    config.set_soft_deletes_field(Some("soft".to_string()));
    config
        .set_merge_policy(Arc::new(AskingPolicy {
            inner: retention,
            answers: Arc::clone(&answers),
        }))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for i in 0..10 {
        writer.add_document(&retention_doc(i, 1)).unwrap();
    }
    writer.commit().unwrap();
    let mut numeric = FieldType::new();
    numeric.set_doc_values_type(DocValuesType::NUMERIC).unwrap();
    numeric.freeze();
    for i in 0..5 {
        let soft = Field::new_with_number("soft", NumericValue::Long(1), numeric.clone()).unwrap();
        writer
            .soft_update_document(
                Term::new("id", BytesRef::new(format!("id{i}").into_bytes())),
                &retention_doc(i, 2),
                &[soft],
            )
            .unwrap();
    }
    writer.commit().unwrap();
    writer.maybe_merge().unwrap();
    let rucene: Vec<(String, i32)> = {
        let answers = answers.lock().unwrap();
        answers[answers.len() - 2..].to_vec()
    };
    writer.close().unwrap();
    assert_eq!(rucene, lucene_num_deletes_to_merge());
    assert_eq!(rucene, vec![("_0".to_string(), 2), ("_1".to_string(), 0)]);
}
