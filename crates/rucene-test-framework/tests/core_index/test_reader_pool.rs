//! Port of `lucene/core/src/test/org/apache/lucene/index/TestReaderPool.java`.
//!
//! Java's `ReaderPool` keeps the writer's `SegmentInfos` as a field; Rucene's
//! takes the index-created major version on `get` and the live infos on
//! `release` (declared in `src/index/readers_and_updates.rs`), so the test
//! passes the `segmentInfos` clone it gave the Java constructor.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, NumericDocValuesField, Store, StringField};
use rucene::error::Result;
use rucene::index::directory_reader::{self, StandardDirectoryReader};
use rucene::index::field_infos::FieldNumbers;
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_policy::{CodecReaderSupplier, FilterMergePolicy};
use rucene::index::postings_enum::POSTINGS_ENUM_FREQS;
use rucene::index::readers_and_updates::{ReaderPool, ReadersAndUpdates};
use rucene::index::terms::Term;
use rucene::index::{DocValuesFieldUpdates, DocValuesType, IndexReader, LeafReader, SegmentInfos};
use rucene::search::NO_MORE_DOCS;
use rucene::store::{Directory, DEFAULT_IO_CONTEXT};
use rucene::util::{BytesRef, IOUtils};
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::null_info_stream::NullInfoStream;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_null, assert_null, class, random, test,
};
use crate::support::CountDownLatch;

/// `TestReaderPool`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestReaderPool",
        vec![
            test("testDrop", test_drop),
            test("testPoolReaders", test_pool_readers),
            test("testUpdate", test_update),
            test("testDeletes", test_deletes),
            test(
                "testPassReaderToMergePolicyConcurrently",
                test_pass_reader_to_merge_policy_concurrently,
            ),
            test("testGetReaderByRam", test_get_reader_by_ram),
        ],
    )
}

/// The fixture every test builds: the directory, the reader over it, a clone
/// of its `SegmentInfos`, and a pool over them.
struct Fixture {
    directory: TestDirectory,
    reader: Arc<dyn rucene::index::DirectoryReader>,
    segment_infos: SegmentInfos,
    pool: ReaderPool,
}

/// `(StandardDirectoryReader) reader`.
fn as_standard(reader: &Arc<dyn rucene::index::DirectoryReader>) -> &StandardDirectoryReader {
    reader
        .as_any()
        .downcast_ref::<StandardDirectoryReader>()
        .unwrap_or_else(|| {
            Throwable::new(
                "java.lang.ClassCastException",
                "not a StandardDirectoryReader",
            )
            .throw()
        })
}

/// `Directory directory = newDirectory(); FieldNumbers fieldNumbers =
/// buildIndex(directory); StandardDirectoryReader reader = ...; SegmentInfos
/// segmentInfos = reader.segmentInfos.clone(); ReaderPool pool = new
/// ReaderPool(directory, directory, segmentInfos, fieldNumbers, () -> 0L,
/// infoStream, null, null)`.
///
/// Java passes `null` as the info stream in `testDrop` and `testPoolReaders`,
/// where the pool never logs; Rucene's pool takes a non-null stream, so a
/// [`NullInfoStream`] stands in there, with no observable difference.
fn fixture() -> Result<Fixture> {
    let directory = LuceneTestCase::new_directory();
    let field_numbers = build_index(&directory.as_directory())?;
    let reader = directory_reader::open(directory.as_directory())?;
    let segment_infos = as_standard(&reader).segment_infos().clone();
    let pool = ReaderPool::new(
        directory.as_directory(),
        directory.as_directory(),
        &segment_infos,
        field_numbers,
        Arc::new(|| 0i64),
        Arc::new(NullInfoStream),
        None,
        None,
    )?;
    Ok(Fixture {
        directory,
        reader,
        segment_infos,
        pool,
    })
}

impl Fixture {
    /// `pool.get(info, create)`.
    fn get(
        &self,
        info: &rucene::index::SegmentCommitInfo,
        create: bool,
    ) -> Result<Option<Arc<ReadersAndUpdates>>> {
        get(&self.pool, &self.segment_infos, info, create)
    }

    /// `pool.release(rld, assertInfoLive)`.
    fn release(&self, rld: &Arc<ReadersAndUpdates>, assert_info_live: bool) -> Result<bool> {
        self.pool
            .release(rld, assert_info_live, &self.segment_infos)
    }

    /// `IOUtils.close(pool, reader, directory)`.
    fn close(self) -> Result<()> {
        let Fixture {
            directory,
            reader,
            pool,
            ..
        } = self;
        IOUtils::close([
            IOUtils::closeable(|| pool.close()),
            IOUtils::closeable(|| reader.close()),
            IOUtils::closeable(|| directory.close()),
        ])
    }
}

/// `pool.get(info, create)`, with the created version of `infos`.
fn get(
    pool: &ReaderPool,
    infos: &SegmentInfos,
    info: &rucene::index::SegmentCommitInfo,
    create: bool,
) -> Result<Option<Arc<ReadersAndUpdates>>> {
    pool.get(info, create, infos.index_created_version_major())
}

/// `pool.get(info, true)`, which never returns null.
fn get_created(
    f: &Fixture,
    info: &rucene::index::SegmentCommitInfo,
) -> Result<Arc<ReadersAndUpdates>> {
    Ok(f.get(info, true)?
        .unwrap_or_else(|| crate::support::npe("pool.get(info, true)")))
}

/// `assertSame(expected, actual)` over pooled readers.
fn assert_same_rld(
    expected: Option<&Arc<ReadersAndUpdates>>,
    actual: Option<&Arc<ReadersAndUpdates>>,
) {
    let same = match (expected, actual) {
        (Some(e), Some(a)) => Arc::ptr_eq(e, a),
        (None, None) => true,
        _ => false,
    };
    if !same {
        Assert::fail(&format!(
            "expected same:<{:?}> was not:<{:?}>",
            expected.map(|r| r.segment_name()),
            actual.map(|r| r.segment_name())
        ))
    }
}

/// `assertNotSame(-1, doc)`: both are boxed to the cached `Integer`
/// instances, so they are the same object exactly when `doc == -1`.
fn assert_not_same_minus_one(doc: i32) {
    if doc == -1 {
        Assert::fail("expected not same")
    }
}

/// Port of `TestReaderPool.testDrop`.
fn test_drop() -> Result<()> {
    let f = fixture()?;
    let commit_info = RandomPicks::random_from(&*random(), f.segment_infos.as_list()).clone();
    let readers_and_updates = get_created(&f, &commit_info)?;
    assert_same_rld(
        Some(&readers_and_updates),
        f.get(&commit_info, false)?.as_ref(),
    );
    Assert::assert_true(f.pool.drop(&commit_info)?);
    if random().next_boolean() {
        Assert::assert_false(f.pool.drop(&commit_info)?);
    }
    assert_null(&f.get(&commit_info, false)?.map(|r| r.segment_name()));
    f.release(&readers_and_updates, random().next_boolean())?;
    f.close()
}

/// Port of `TestReaderPool.testPoolReaders`.
fn test_pool_readers() -> Result<()> {
    let f = fixture()?;
    let commit_info = RandomPicks::random_from(&*random(), f.segment_infos.as_list()).clone();
    Assert::assert_false(f.pool.is_reader_pooling_enabled());
    f.release(&get_created(&f, &commit_info)?, random().next_boolean())?;
    assert_null(&f.get(&commit_info, false)?.map(|r| r.segment_name()));
    // now start pooling
    f.pool.enable_reader_pooling(true);
    Assert::assert_true(f.pool.is_reader_pooling_enabled());
    f.release(&get_created(&f, &commit_info)?, random().next_boolean())?;
    assert_not_null(&f.get(&commit_info, false)?);
    assert_same_rld(
        f.get(&commit_info, false)?.as_ref(),
        f.get(&commit_info, false)?.as_ref(),
    );
    f.pool.drop(&commit_info)?;
    let mut ram_bytes_used = 0i64;
    assert_equals(&0, &f.pool.ram_bytes_used());
    for info in f.segment_infos.iter() {
        f.release(&get_created(&f, info)?, random().next_boolean())?;
        assert_equals_msg(
            Some(&format!(
                " used: {ram_bytes_used} actual: {}",
                f.pool.ram_bytes_used()
            )),
            &0,
            &f.pool.ram_bytes_used(),
        );
        ram_bytes_used = f.pool.ram_bytes_used();
        assert_same_rld(f.get(info, false)?.as_ref(), f.get(info, false)?.as_ref());
    }
    // `assertNotSame(0, pool.ramBytesUsed())`: an `Integer` and a `Long` are
    // never the same object, so Java's assertion always holds.
    let _ = f.pool.ram_bytes_used();
    f.pool.drop_all()?;
    for info in f.segment_infos.iter() {
        assert_null(&f.get(info, false)?.map(|r| r.segment_name()));
    }
    assert_equals(&0, &f.pool.ram_bytes_used());
    f.close()
}

/// `readOnlyClone.postings(new Term("id", "" + id))`.
fn postings(
    reader: &dyn LeafReader,
    id: i32,
) -> Result<Option<Box<dyn rucene::index::postings_enum::PostingsEnum>>> {
    reader.postings(&Term::from_text("id", &id.to_string()), POSTINGS_ENUM_FREQS)
}

/// Port of `TestReaderPool.testUpdate`.
fn test_update() -> Result<()> {
    let f = fixture()?;
    let id = random().next_int_bound(10);
    if random().next_boolean() {
        f.pool.enable_reader_pooling(true);
    }
    for commit_info in f.segment_infos.iter() {
        let readers_and_updates = get_created(&f, commit_info)?;
        let read_only_clone = readers_and_updates.get_read_only_clone(&*DEFAULT_IO_CONTEXT)?;
        let mut postings = postings(&*read_only_clone, id)?;
        let mut expect_update = false;
        let mut doc = -1;
        if let Some(p) = postings.as_mut().filter(|_| true) {
            if p.next_doc()? != NO_MORE_DOCS {
                let mut number = DocValuesFieldUpdates::new(
                    commit_info.info.max_doc()?,
                    0,
                    "number",
                    DocValuesType::NUMERIC,
                );
                doc = p.doc_id();
                number.add_long(doc, 1000);
                number.finish()?;
                readers_and_updates.add_dv_update(number)?;
                expect_update = true;
                assert_equals(&NO_MORE_DOCS, &p.next_doc()?);
                Assert::assert_true(f.pool.any_doc_values_changes());
            } else {
                Assert::assert_false(f.pool.any_doc_values_changes());
            }
        } else {
            Assert::assert_false(f.pool.any_doc_values_changes());
        }
        read_only_clone.close()?;
        let written_to_disk;
        if f.pool.is_reader_pooling_enabled() {
            if random().next_boolean() {
                written_to_disk = f.pool.write_all_doc_values_updates()?;
                Assert::assert_false(readers_and_updates.is_merging());
            } else if random().next_boolean() {
                written_to_disk = f.pool.commit(f.segment_infos.as_list())?;
                Assert::assert_false(readers_and_updates.is_merging());
            } else {
                written_to_disk = f
                    .pool
                    .write_doc_values_updates_for_merge(std::slice::from_ref(commit_info))?;
                Assert::assert_true(readers_and_updates.is_merging());
            }
            Assert::assert_false(f.release(&readers_and_updates, random().next_boolean())?);
        } else if random().next_boolean() {
            written_to_disk = f.release(&readers_and_updates, random().next_boolean())?;
            Assert::assert_false(readers_and_updates.is_merging());
        } else {
            written_to_disk = f
                .pool
                .write_doc_values_updates_for_merge(std::slice::from_ref(commit_info))?;
            Assert::assert_true(readers_and_updates.is_merging());
            Assert::assert_false(f.release(&readers_and_updates, random().next_boolean())?);
        }
        Assert::assert_false(f.pool.any_doc_values_changes());
        assert_equals(&expect_update, &written_to_disk);
        if expect_update {
            let readers_and_updates = get_created(&f, commit_info)?;
            let updated_reader = readers_and_updates.get_read_only_clone(&*DEFAULT_IO_CONTEXT)?;
            assert_not_same_minus_one(doc);
            let mut number = updated_reader
                .get_numeric_doc_values("number")?
                .unwrap_or_else(|| crate::support::npe("getNumericDocValues"));
            assert_equals(&doc, &number.advance(doc)?);
            assert_equals(&1000i64, &number.long_value()?);
            readers_and_updates.release(&updated_reader)?;
            Assert::assert_false(f.release(&readers_and_updates, random().next_boolean())?);
        }
    }
    f.close()
}

/// Port of `TestReaderPool.testDeletes`.
fn test_deletes() -> Result<()> {
    let f = fixture()?;
    let id = random().next_int_bound(10);
    if random().next_boolean() {
        f.pool.enable_reader_pooling(true);
    }
    for commit_info in f.segment_infos.iter() {
        let readers_and_updates = get_created(&f, commit_info)?;
        let read_only_clone = readers_and_updates.get_read_only_clone(&*DEFAULT_IO_CONTEXT)?;
        let mut postings = postings(&*read_only_clone, id)?;
        let mut expect_update = false;
        let mut doc = -1;
        if let Some(p) = postings.as_mut() {
            if p.next_doc()? != NO_MORE_DOCS {
                doc = p.doc_id();
                Assert::assert_true(readers_and_updates.delete(doc)?);
                expect_update = true;
                assert_equals(&NO_MORE_DOCS, &p.next_doc()?);
            }
        }
        Assert::assert_false(f.pool.any_doc_values_changes()); // deletes are not accounted here
        read_only_clone.close()?;
        let written_to_disk = if f.pool.is_reader_pooling_enabled() {
            let written = f.pool.commit(f.segment_infos.as_list())?;
            Assert::assert_false(f.release(&readers_and_updates, random().next_boolean())?);
            written
        } else {
            f.release(&readers_and_updates, random().next_boolean())?
        };
        Assert::assert_false(f.pool.any_doc_values_changes());
        assert_equals(&expect_update, &written_to_disk);
        if expect_update {
            let readers_and_updates = get_created(&f, commit_info)?;
            let updated_reader = readers_and_updates.get_read_only_clone(&*DEFAULT_IO_CONTEXT)?;
            assert_not_same_minus_one(doc);
            let live_docs = updated_reader
                .get_live_docs()
                .unwrap_or_else(|| crate::support::npe("getLiveDocs"));
            Assert::assert_false(live_docs.get(doc as usize));
            readers_and_updates.release(&updated_reader)?;
            Assert::assert_false(f.release(&readers_and_updates, random().next_boolean())?);
        }
    }
    f.close()
}

/// Implements `MergePolicy` for `$ty` by delegating every method to its
/// `FilterMergePolicy` field `$field`, except the overrides in the block:
/// an anonymous `new FilterMergePolicy(in) { ... }` subclass.
macro_rules! delegate_merge_policy {
    ($ty:ty, $field:ident, { $($overrides:item)* }) => {
        impl rucene::index::merge_policy::MergePolicy for $ty {
            $($overrides)*
            fn as_any(&self) -> Option<&dyn std::any::Any> {
                rucene::index::merge_policy::MergePolicy::as_any(&self.$field)
            }
            fn find_merges(
                &self,
                merge_trigger: rucene::index::merge_policy::MergeTrigger,
                segment_infos: &rucene::index::SegmentInfos,
                merge_context: &dyn rucene::index::merge_policy::MergeContext,
            ) -> Result<Option<rucene::index::merge_policy::MergeSpecification>> {
                self.$field.find_merges(merge_trigger, segment_infos, merge_context)
            }
            fn find_full_flush_merges(
                &self,
                merge_trigger: rucene::index::merge_policy::MergeTrigger,
                segment_infos: &rucene::index::SegmentInfos,
                merge_context: &dyn rucene::index::merge_policy::MergeContext,
            ) -> Result<Option<rucene::index::merge_policy::MergeSpecification>> {
                self.$field
                    .find_full_flush_merges(merge_trigger, segment_infos, merge_context)
            }
            fn find_merges_for_readers(
                &self,
                readers: &[Arc<dyn rucene::index::codec_reader::CodecReader>],
            ) -> Result<Option<rucene::index::merge_policy::MergeSpecification>> {
                self.$field.find_merges_for_readers(readers)
            }
            fn find_forced_merges(
                &self,
                segment_infos: &rucene::index::SegmentInfos,
                max_num_segments: i32,
                segments_to_merge: &std::collections::HashMap<String, bool>,
                merge_context: &dyn rucene::index::merge_policy::MergeContext,
            ) -> Result<Option<rucene::index::merge_policy::MergeSpecification>> {
                self.$field.find_forced_merges(
                    segment_infos,
                    max_num_segments,
                    segments_to_merge,
                    merge_context,
                )
            }
            fn find_forced_deletes_merges(
                &self,
                segment_infos: &rucene::index::SegmentInfos,
                merge_context: &dyn rucene::index::merge_policy::MergeContext,
            ) -> Result<Option<rucene::index::merge_policy::MergeSpecification>> {
                self.$field
                    .find_forced_deletes_merges(segment_infos, merge_context)
            }
            fn use_compound_file(
                &self,
                infos: &rucene::index::SegmentInfos,
                merged_info: &rucene::index::SegmentCommitInfo,
                merge_context: &dyn rucene::index::merge_policy::MergeContext,
            ) -> Result<bool> {
                self.$field.use_compound_file(infos, merged_info, merge_context)
            }
            fn get_no_cfs_ratio(&self) -> f64 {
                self.$field.get_no_cfs_ratio()
            }
            fn get_max_cfs_segment_size(&self) -> i64 {
                self.$field.get_max_cfs_segment_size()
            }
            fn get_max_cfs_segment_size_mb(&self) -> f64 {
                self.$field.get_max_cfs_segment_size_mb()
            }
            fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
                self.$field.set_no_cfs_ratio(no_cfs_ratio)
            }
            fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
                self.$field.set_max_cfs_segment_size_mb(v)
            }
            fn size(
                &self,
                info: &rucene::index::SegmentCommitInfo,
                merge_context: &dyn rucene::index::merge_policy::MergeContext,
            ) -> Result<i64> {
                self.$field.size(info, merge_context)
            }
            fn is_merged(
                &self,
                infos: &rucene::index::SegmentInfos,
                info: &rucene::index::SegmentCommitInfo,
                merge_context: &dyn rucene::index::merge_policy::MergeContext,
            ) -> Result<bool> {
                self.$field.is_merged(infos, info, merge_context)
            }
            fn num_deletes_to_merge_default(
                &self,
                info: &rucene::index::SegmentCommitInfo,
                del_count: i32,
            ) -> i32 {
                self.$field.num_deletes_to_merge_default(info, del_count)
            }
            fn max_full_flush_merge_size(&self) -> i64 {
                self.$field.max_full_flush_merge_size()
            }
        }
    };
}

/// The anonymous `FilterMergePolicy` of
/// `testPassReaderToMergePolicyConcurrently`, overriding
/// `keepFullyDeletedSegment`.
struct AccessReaderMergePolicy {
    filter: FilterMergePolicy,
}

impl std::fmt::Display for AccessReaderMergePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.filter, f)
    }
}

impl std::fmt::Debug for AccessReaderMergePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.filter, f)
    }
}

delegate_merge_policy!(AccessReaderMergePolicy, filter, {
    fn keep_fully_deleted_segment(
        &self,
        reader_supplier: &mut CodecReaderSupplier<'_>,
    ) -> Result<bool> {
        let reader = reader_supplier()?;
        debug_assert!(reader.max_doc() > 0); // just try to access the reader
        Ok(true)
    }
});

/// Port of `TestReaderPool.testPassReaderToMergePolicyConcurrently`.
fn test_pass_reader_to_merge_policy_concurrently() -> Result<()> {
    let f = Arc::new(fixture()?);
    if random().next_boolean() {
        f.pool.enable_reader_pooling(true);
    }
    let is_done = Arc::new(AtomicBool::new(false));
    let latch = Arc::new(CountDownLatch::new(1));
    let refresher = {
        let (f, is_done, latch) = (Arc::clone(&f), Arc::clone(&is_done), Arc::clone(&latch));
        Thread::unnamed(move || {
            let run = || -> Result<()> {
                latch.count_down();
                while !is_done.load(Ordering::SeqCst) {
                    for commit_info in f.segment_infos.iter() {
                        let readers_and_updates = get_created(&f, commit_info)?;
                        let segment_reader =
                            readers_and_updates.get_reader(&*DEFAULT_IO_CONTEXT)?;
                        readers_and_updates.release(&segment_reader)?;
                        f.release(&readers_and_updates, random().next_boolean())?;
                    }
                }
                Ok(())
            };
            if let Err(ex) = run() {
                Throwable::from_cause("java.lang.AssertionError", java_throwable(&ex)).throw();
            }
        })
    };
    refresher.start();
    let merge_policy = AccessReaderMergePolicy {
        filter: FilterMergePolicy::new(LuceneTestCase::new_merge_policy()),
    };
    latch.await_();
    for i in 0..f.reader.max_doc() {
        for commit_info in f.segment_infos.iter() {
            let readers_and_updates = get_created(&f, commit_info)?;
            let sr = readers_and_updates.get_read_only_clone(&*DEFAULT_IO_CONTEXT)?;
            let postings = postings(&*sr, i)?;
            sr.dec_ref()?;
            if let Some(mut postings) = postings {
                let mut doc_id = postings.next_doc()?;
                while doc_id != NO_MORE_DOCS {
                    readers_and_updates.delete(doc_id)?;
                    Assert::assert_true(
                        readers_and_updates.keep_fully_deleted_segment(&merge_policy)?,
                    );
                    doc_id = postings.next_doc()?;
                }
            }
            Assert::assert_true(readers_and_updates.keep_fully_deleted_segment(&merge_policy)?);
            f.release(&readers_and_updates, random().next_boolean())?;
        }
    }
    is_done.store(true, Ordering::SeqCst);
    if let Err(p) = refresher.join() {
        std::panic::resume_unwind(p);
    }
    drop(refresher);
    match Arc::try_unwrap(f) {
        Ok(f) => f.close(),
        Err(_) => crate::support::npe("fixture still shared"),
    }
}

/// `private FieldInfos.FieldNumbers buildIndex(Directory directory)`.
fn build_index(directory: &Arc<dyn Directory>) -> Result<Arc<FieldNumbers>> {
    let writer = IndexWriter::new(
        Arc::clone(directory),
        LuceneTestCase::new_index_writer_config(),
    )?;
    for i in 0..10 {
        let mut document = Document::new();
        document.add(Box::new(StringField::new("id", i.to_string(), Store::YES)?));
        document.add(Box::new(NumericDocValuesField::new("number", i)));
        writer.add_document(&document)?;
        if random().next_boolean() {
            writer.flush()?;
        }
    }
    writer.commit()?;
    writer.close()?;
    Ok(Arc::clone(writer.global_field_number_map()))
}

/// Port of `TestReaderPool.testGetReaderByRam`.
fn test_get_reader_by_ram() -> Result<()> {
    let f = fixture()?;
    assert_equals(&0, &f.pool.get_readers_by_ram().len());

    let mut ord = 0usize;
    for commit_info in f.segment_infos.iter() {
        let readers_and_updates = get_created(&f, commit_info)?;
        let mut test = DocValuesFieldUpdates::new(
            commit_info.info.max_doc()?,
            0,
            "test",
            DocValuesType::BINARY,
        );
        test.add_binary(0, &BytesRef::new(vec![0u8; ord]));
        ord += 1;
        test.finish()?;
        readers_and_updates.add_dv_update(test)?;
    }

    let readers_by_ram = f.pool.get_readers_by_ram();
    assert_equals(&f.segment_infos.size(), &readers_by_ram.len());
    let mut previous_ram = i64::MAX;
    for rld in &readers_by_ram {
        Assert::assert_true_msg(
            &format!("previous: {previous_ram} now: {}", rld.ram_bytes_used()),
            previous_ram >= rld.ram_bytes_used(),
        );
        previous_ram = rld.ram_bytes_used();
        rld.drop_changes();
        f.pool.drop(&rld.info())?;
    }
    f.close()
}
