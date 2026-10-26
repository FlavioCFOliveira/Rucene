//! How `IndexWriter` resolves, writes and carries deletes and doc-values
//! updates, pinned against Apache Lucene Core 10.5.0.
//!
//! * **Lost deletes (defect A of rmp #234).** `SegmentInfos.remove` matched a
//!   segment by value. The pool's copy of a segment carried other delete counts
//!   than the writer's, so dropping a fully deleted segment left it in the
//!   index while its pooled reader — and the deletes it held — were thrown
//!   away. Java matches the very instance (`SegmentCommitInfo` does not
//!   override `equals`).
//! * **rmp #257.** `ReaderPool.release` writes a segment's live docs when
//!   readers are not pooled (`ReaderPool.java:183-230`); `writeReaderPool`
//!   drops the segments it finds fully deleted (`IndexWriter.java:4112-4127`);
//!   `writeSomeDocValuesUpdates` writes updates only once the pool holds more
//!   than half of the RAM buffer (`IndexWriter.java:810-885`).
//! * **rmp #264.** The doc-values updates a merge carries over arrive in hash
//!   order; the value a document ends with is the one of the highest
//!   generation, as `DocValuesFieldUpdates.mergedIterator` picks it
//!   (`ReadersAndUpdates.java:333-345`), not the last one in the list.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use rucene::codecs::Lucene104Codec;
use rucene::document::{Document, NumericDocValuesField, Store, StringField};
use rucene::index::index_reader::IndexReader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::{
    MergeContext, MergePolicy, MergeSpecification, MergeTrigger, NoMergePolicy, TieredMergePolicy,
};
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::terms::Term;
use rucene::index::{directory_reader, DirectoryReader, SegmentCommitInfo, SegmentInfos};
use rucene::search::NO_MORE_DOCS;
use rucene::store::{Directory, RamDirectory};
use rucene::util::BytesRef;

fn ensure_codec() {
    let _ = rucene::codecs::register_codec("Lucene104", Lucene104Codec::new());
}

fn id(value: &str) -> Term {
    Term::new("id", BytesRef::new(value.as_bytes().to_vec()))
}

fn doc(value: &str, price: i64) -> Document {
    let mut d = Document::new();
    d.add(Box::new(
        StringField::new("id", value.to_string(), Store::NO).unwrap(),
    ));
    d.add(Box::new(NumericDocValuesField::new("price", price)));
    d
}

fn no_merges() -> IndexWriterConfig {
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .unwrap();
    config
}

fn prices(reader: &Arc<dyn DirectoryReader>) -> Vec<i64> {
    let mut out = Vec::new();
    for leaf in Arc::clone(reader).leaves() {
        let leaf = leaf.leaf_reader();
        let live = leaf.get_live_docs();
        let Some(mut values) = leaf.get_numeric_doc_values("price").unwrap() else {
            continue;
        };
        loop {
            let doc = values.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                break;
            }
            if live.as_ref().is_none_or(|bits| bits.get(doc as usize)) {
                out.push(values.long_value().unwrap());
            }
        }
    }
    out
}

/// A segment emptied over two commits is dropped, and its last delete is not
/// lost.
///
/// The first delete is written at the first commit, which updates the
/// writer's copy of the segment; the pooled reader that resolves the second
/// delete still carries the copy it was opened with. Matched by value, the
/// drop missed the writer's copy: the segment stayed in the index with one
/// delete recorded, its pooled reader — and the second delete — gone.
#[test]
fn a_segment_emptied_over_two_commits_is_dropped_with_both_deletes() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let writer = IndexWriter::new(Arc::clone(&dir), no_merges())
        .unwrap()
        .shared();
    writer.add_document(&doc("a", 1)).unwrap();
    writer.add_document(&doc("b", 2)).unwrap();
    writer.commit().unwrap();
    writer.delete_documents_by_terms(vec![id("a")]).unwrap();
    writer.commit().unwrap();
    writer.delete_documents_by_terms(vec![id("b")]).unwrap();
    writer.commit().unwrap();

    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    assert_eq!(infos.size(), 0, "the fully deleted segment was dropped");
    let reader = directory_reader::open_with_writer(writer.as_ref()).unwrap();
    assert_eq!(reader.num_docs(), 0, "no deleted document comes back");
    IndexReader::close(reader.as_ref()).unwrap();
    writer.close().unwrap();
}

/// Without reader pooling, resolving a delete writes the live docs at once.
///
/// `ReaderPool.release` writes the segment's live docs and drops its entry
/// when it releases the last reference besides the pool's own and readers are
/// not pooled (`ReaderPool.java:198-221`).
#[test]
fn without_reader_pooling_a_resolved_delete_is_written_when_released() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = no_merges();
    config.set_reader_pooling(false);
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    writer.add_document(&doc("a", 1)).unwrap();
    writer.add_document(&doc("b", 2)).unwrap();
    writer.commit().unwrap();
    writer.delete_documents_by_terms(vec![id("a")]).unwrap();
    // Flush and apply all deletes, without committing.
    writer.flush_with(false, true).unwrap();
    let segment: SegmentCommitInfo = writer.newest_segment().unwrap().unwrap();
    assert_eq!(segment.get_del_gen(), 1, "the live docs were written");
    assert!(dir.list_all().unwrap().iter().any(|f| f == "_0_1.liv"));
    assert!(
        writer
            .get_pooled_instance(&segment, false)
            .unwrap()
            .is_none(),
        "releasing the last reference wrote the segment and dropped it from the pool"
    );
    writer.close().unwrap();
}

/// A segment that the soft deletes of an update empty is dropped when the
/// reader pool is written.
///
/// The soft delete only reaches the segment's counts when its doc-values
/// update is written, so it is `writeReaderPool`'s own check that finds the
/// segment fully deleted (`IndexWriter.java:4112-4127`).
#[test]
fn write_reader_pool_drops_a_segment_its_writes_emptied() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = no_merges();
    config.set_soft_deletes_field(Some("soft".to_string()));
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    writer.add_document(&doc("a", 1)).unwrap();
    writer.commit().unwrap();
    let mut numeric = rucene::document::FieldType::new();
    numeric
        .set_doc_values_type(rucene::index::DocValuesType::NUMERIC)
        .unwrap();
    numeric.freeze();
    let soft = rucene::document::Field::new_with_number(
        "soft",
        rucene::document::NumericValue::Long(1),
        numeric,
    )
    .unwrap();
    writer
        .soft_update_document(id("a"), &doc("a", 2), &[soft])
        .unwrap();
    writer.commit().unwrap();

    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    let names: Vec<String> = infos.iter().map(|info| info.info.name.clone()).collect();
    assert_eq!(names, vec!["_1".to_string()], "the emptied _0 was dropped");
    writer.close().unwrap();
}

/// The doc-values update of `writer`'s segment `_0`, made in place through an
/// NRT reader, and then offered to `write_some_doc_values_updates`.
///
/// `tryUpdateDocValue` adds the update to the pooled `ReadersAndUpdates`
/// without writing it (`IndexWriter.java:1733-1760`), which leaves
/// `writeSomeDocValuesUpdates` to decide.
fn doc_values_gen_after_write_some(ram_buffer_size_mb: f64) -> i64 {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let mut config = no_merges();
    config.set_ram_buffer_size_mb(ram_buffer_size_mb).unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap().shared();
    writer.add_document(&doc("a", 1)).unwrap();
    writer.commit().unwrap();
    let reader = directory_reader::open_with_writer(writer.as_ref()).unwrap();
    let field = rucene::document::Field::new_with_number(
        "price",
        rucene::document::NumericValue::Long(5),
        {
            let mut numeric = rucene::document::FieldType::new();
            numeric
                .set_doc_values_type(rucene::index::DocValuesType::NUMERIC)
                .unwrap();
            numeric.freeze();
            numeric
        },
    )
    .unwrap();
    let as_index_reader: Arc<dyn IndexReader> = Arc::clone(&reader) as Arc<dyn IndexReader>;
    assert!(
        writer
            .try_update_doc_value(&as_index_reader, 0, &[field])
            .unwrap()
            >= 0
    );
    writer.write_some_doc_values_updates().unwrap();
    let gen = writer
        .newest_segment()
        .unwrap()
        .unwrap()
        .get_doc_values_gen();
    IndexReader::close(reader.as_ref()).unwrap();
    writer.close().unwrap();
    gen
}

/// Resolved doc-values updates stay in RAM while the pool holds no more than
/// half of the RAM buffer, and are written — the writer's copy of the segment
/// recording the new generation — once it holds more.
#[test]
fn write_some_doc_values_updates_writes_only_past_half_the_ram_buffer() {
    assert_eq!(
        doc_values_gen_after_write_some(16.0),
        -1,
        "16 MB of RAM buffer: the update stays in RAM"
    );
    assert_eq!(
        doc_values_gen_after_write_some(0.0001),
        1,
        "a 100-byte RAM buffer: the update is written"
    );
}

/// `TieredMergePolicy`, whose force-merge runs `hook` when the merge is about
/// to be committed — after the readers were merged, before the deletes and
/// updates resolved meanwhile are carried over.
struct HookPolicy {
    inner: TieredMergePolicy,
    hook: Arc<dyn Fn() + Send + Sync>,
}

impl fmt::Display for HookPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HookPolicy({})", self.inner)
    }
}

impl MergePolicy for HookPolicy {
    fn find_merges(
        &self,
        trigger: MergeTrigger,
        infos: &SegmentInfos,
        context: &dyn MergeContext,
    ) -> rucene::Result<Option<MergeSpecification>> {
        self.inner.find_merges(trigger, infos, context)
    }

    fn find_forced_merges(
        &self,
        infos: &SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &HashMap<String, bool>,
        context: &dyn MergeContext,
    ) -> rucene::Result<Option<MergeSpecification>> {
        let mut spec =
            self.inner
                .find_forced_merges(infos, max_num_segments, segments_to_merge, context)?;
        if let Some(spec) = spec.as_mut() {
            for merge in &mut spec.merges {
                let hook = Arc::clone(&self.hook);
                merge.set_on_merge_complete(Arc::new(move |_merge| {
                    hook();
                    Ok(())
                }));
            }
        }
        Ok(spec)
    }

    fn find_forced_deletes_merges(
        &self,
        infos: &SegmentInfos,
        context: &dyn MergeContext,
    ) -> rucene::Result<Option<MergeSpecification>> {
        self.inner.find_forced_deletes_merges(infos, context)
    }
}

/// Updates resolved against the merging segments in several generations are
/// carried over, and each document ends with the value of the highest one.
#[test]
fn updates_carried_over_by_a_merge_keep_the_value_of_the_highest_generation() {
    ensure_codec();
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let writer_slot: Arc<OnceLock<Weak<IndexWriter>>> = Arc::new(OnceLock::new());
    let ran = Arc::new(Mutex::new(false));
    let hook: Arc<dyn Fn() + Send + Sync> = {
        let writer_slot = Arc::clone(&writer_slot);
        let ran = Arc::clone(&ran);
        Arc::new(move || {
            let writer = writer_slot.get().and_then(Weak::upgrade).unwrap();
            // Several packets, each resolved against the merging segments:
            // generations g, g+1, ... with the last value the highest.
            for value in [10, 20, 30, 40, 50] {
                writer
                    .update_numeric_doc_value(id("a"), "price", value)
                    .unwrap();
                writer
                    .update_numeric_doc_value(id("b"), "price", value + 1)
                    .unwrap();
                writer.flush_with(false, true).unwrap();
            }
            *ran.lock().unwrap() = true;
        })
    };
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_policy(Arc::new(HookPolicy {
            inner: TieredMergePolicy::new(),
            hook,
        }))
        .unwrap();
    config
        .set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap().shared();
    let _ = writer_slot.set(Arc::downgrade(&writer));
    writer.add_document(&doc("a", 1)).unwrap();
    writer.commit().unwrap();
    writer.add_document(&doc("b", 2)).unwrap();
    writer.commit().unwrap();
    writer.force_merge(1).unwrap();
    assert!(
        *ran.lock().unwrap(),
        "the updates ran while the merge was open"
    );
    writer.commit().unwrap();

    let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
    assert_eq!(infos.size(), 1, "one merged segment");
    let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
    let mut values = prices(&reader);
    values.sort_unstable();
    assert_eq!(
        values,
        vec![50, 51],
        "the last update of each document wins"
    );
    IndexReader::close(reader.as_ref()).unwrap();
    writer.close().unwrap();
}
