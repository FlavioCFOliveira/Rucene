//! Port of the tests of `lucene/test-framework/src/test/org/apache/lucene/tests/index/`
//! (task #359 part 2): `TestAssertingLeafReader`, `TestForceMergePolicy` and
//! `TestMockRandomMergePolicy`, each run as a suite (`extends
//! LuceneTestCase`) by the ported runner, and each reporting what Lucene's
//! own class reported on the JVM (`framework_tests.tsv`, written by
//! `FrameworkTestsOracle`).

mod oracle_support;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{Class, ClassBuilder, Constructor, Method};
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::LuceneError;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::{MergeContext, MergePolicy, MergeTrigger, NoMergePolicy};
use rucene::index::segment_info::SegmentCommitInfo;
use rucene::index::segment_infos::SegmentInfos;
use rucene::index::terms::Term;
use rucene::index::IndexReader;
use rucene::util::{BytesRef, InfoStream, Version};
use rucene_test_framework::index::{
    AssertingDirectoryReader, ForceMergePolicy, MockRandomMergePolicy,
};
use rucene_test_framework::util::LuceneTestCase;

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

fn run_suite(builder: ClassBuilder) {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    oracle_support::assert_framework_outcomes(&builder.build());
}

/// Throws a Rucene error out of a test method, as Java's `throws Exception`.
fn rethrow(e: LuceneError) -> ! {
    let (class, message) =
        rucene_test_framework::analysis::base_token_stream_test_case::java_class_and_message(&e);
    Throwable::new(class, message).throw()
}

/// `TestAssertingLeafReader.testAssertBits`: the live docs of an
/// `AssertingDirectoryReader` may be read from another thread.
#[test]
fn test_asserting_leaf_reader() {
    run_suite(
        Class::builder("org.apache.lucene.tests.index.TestAssertingLeafReader")
            .extends(&LuceneTestCase::class())
            .constructor(Constructor::no_args(|| ()))
            .method(
                Method::new("testAssertBits")
                    .throws(&["java.lang.Exception"])
                    .body_any(|| {
                        let dir = LuceneTestCase::new_directory();
                        let config = LuceneTestCase::new_index_writer_config();
                        config
                            .set_merge_policy(Some(NoMergePolicy::instance()))
                            .unwrap_or_else(|e| rethrow(e));
                        let w = IndexWriter::new(dir.as_directory(), config)
                            .unwrap_or_else(|e| rethrow(e));
                        // Not deleted:
                        w.add_document(&Document::new())
                            .unwrap_or_else(|e| rethrow(e));

                        // Does get deleted:
                        let mut doc = Document::new();
                        doc.add(Box::new(LuceneTestCase::new_string_field(
                            "id",
                            "0",
                            Store::NO,
                        )));
                        w.add_document(&doc).unwrap_or_else(|e| rethrow(e));
                        w.commit().unwrap_or_else(|e| rethrow(e));

                        w.delete_documents_by_terms(vec![Term::new(
                            "id",
                            BytesRef::from_char_sequence("0"),
                        )])
                        .unwrap_or_else(|e| rethrow(e));
                        w.close().unwrap_or_else(|e| rethrow(e));
                        // The writer's handle on the directory (see `MockDirectoryWrapper`'s
                        // module docs: `close()` needs the last one).
                        drop(w);

                        // Now we have index with 1 segment with 2 docs one of which
                        // is marked deleted

                        let r: Arc<dyn DirectoryReader> =
                            directory_reader::open(dir.as_directory())
                                .unwrap_or_else(|e| rethrow(e));
                        let leaves = Arc::clone(&r).leaves().unwrap_or_else(|e| rethrow(e));
                        Assert::assert_equals(1, leaves.len());
                        Assert::assert_equals(2, r.max_doc());
                        Assert::assert_equals(1, r.num_docs());

                        let r: Arc<dyn DirectoryReader> = Arc::new(
                            AssertingDirectoryReader::new(r).unwrap_or_else(|e| rethrow(e)),
                        );
                        let r2 = Arc::clone(&r);

                        let thread = Thread::unnamed(move || {
                            for context in Arc::clone(&r2).leaves().unwrap_or_else(|e| rethrow(e)) {
                                let live_docs = context
                                    .leaf_reader()
                                    .get_live_docs()
                                    .expect("one document is deleted");
                                live_docs.get(0);
                            }
                        });
                        thread.start();
                        let _ = thread.join();

                        // `IOUtils.close(r, dir)`
                        r.close().unwrap_or_else(|e| rethrow(e));
                        drop((r, leaves));
                        dir.close().unwrap_or_else(|e| rethrow(e));
                    }),
            ),
    );
}

/// The `null` merge context of `TestForceMergePolicy`: Rust references are
/// never null, so the port passes a context no method of it is called on.
struct NoContext;

impl MergeContext for NoContext {
    fn num_deletes_to_merge(&self, _info: &SegmentCommitInfo) -> rucene::error::Result<i32> {
        unreachable!("ForceMergePolicy.findMerges reads no argument")
    }
    fn num_deleted_docs(&self, _info: &SegmentCommitInfo) -> rucene::error::Result<i32> {
        unreachable!("ForceMergePolicy.findMerges reads no argument")
    }
    fn get_merging_segments(&self) -> HashSet<String> {
        unreachable!("ForceMergePolicy.findMerges reads no argument")
    }
    fn get_info_stream(&self) -> Arc<dyn InfoStream> {
        unreachable!("ForceMergePolicy.findMerges reads no argument")
    }
}

/// `TestForceMergePolicy.testForceMergePolicy`: `findMerges` never merges.
///
/// **Divergence.** Java builds `new ForceMergePolicy(null)` and calls
/// `findMerges(null, (SegmentInfos) null, null)`; Rust has no null
/// references, so the port wraps a `NoMergePolicy`, and passes
/// `MergeTrigger::Explicit`, an empty `SegmentInfos` and a context that
/// fails if read. `findMerges` reads none of them, so the outcome is Java's.
#[test]
fn test_force_merge_policy() {
    run_suite(
        Class::builder("org.apache.lucene.tests.index.TestForceMergePolicy")
            .extends(&LuceneTestCase::class())
            .constructor(Constructor::no_args(|| ()))
            .method(
                Method::new("testForceMergePolicy")
                    .throws(&["java.lang.Exception"])
                    .body_any(|| {
                        let mp = ForceMergePolicy::new(NoMergePolicy::instance());
                        let infos = SegmentInfos::new(Version::LATEST.major as i32)
                            .unwrap_or_else(|e| rethrow(e));
                        let merges = mp
                            .find_merges(MergeTrigger::Explicit, &infos, &NoContext)
                            .unwrap_or_else(|e| rethrow(e));
                        Assert::assert_true(merges.is_none());
                    }),
            ),
    );
}

/// `TestMockRandomMergePolicy.testReverseWithParents`: the reversing doc map
/// keeps each parent block together.
#[test]
fn test_mock_random_merge_policy() {
    run_suite(
        Class::builder("org.apache.lucene.tests.index.TestMockRandomMergePolicy")
            .extends(&LuceneTestCase::class())
            .constructor(Constructor::no_args(|| ()))
            .method(
                Method::new("testReverseWithParents")
                    .throws(&["java.lang.Exception"])
                    .body_any(|| {
                        let dir = LuceneTestCase::new_directory();
                        let config = IndexWriterConfig::new();
                        config.set_parent_field(Some("_parent".to_string()));
                        let w = IndexWriter::new(dir.as_directory(), config)
                            .unwrap_or_else(|e| rethrow(e));
                        let docs: Vec<Document> = (0..5).map(|_| Document::new()).collect();
                        w.add_documents(&docs[0..2]).unwrap_or_else(|e| rethrow(e));
                        w.add_documents(&docs[0..4]).unwrap_or_else(|e| rethrow(e));
                        w.add_documents(&docs[0..3]).unwrap_or_else(|e| rethrow(e));
                        w.force_merge(1).unwrap_or_else(|e| rethrow(e));
                        w.close().unwrap_or_else(|e| rethrow(e));
                        drop(w);
                        let reader = directory_reader::open(dir.as_directory())
                            .unwrap_or_else(|e| rethrow(e));
                        // `(CodecReader) getOnlyLeafReader(reader)`
                        let codec_reader = LuceneTestCase::get_only_leaf_reader(
                            Arc::clone(&reader) as Arc<dyn IndexReader>,
                        )
                        .unwrap_or_else(|e| rethrow(e))
                        .as_codec_reader()
                        .expect("a segment reader is a CodecReader");
                        let doc_map = MockRandomMergePolicy::reverse(&*codec_reader)
                            .unwrap_or_else(|e| rethrow(e));

                        let old_to_new = [7, 8, 3, 4, 5, 6, 0, 1, 2];
                        for (old, new) in old_to_new.into_iter().enumerate() {
                            Assert::assert_equals(new, doc_map.old_to_new(old as i32));
                        }
                        let new_to_old = [6, 7, 8, 2, 3, 4, 5, 0, 1];
                        for (new, old) in new_to_old.into_iter().enumerate() {
                            Assert::assert_equals(old, doc_map.new_to_old(new as i32));
                        }

                        reader.close().unwrap_or_else(|e| rethrow(e));
                        drop((reader, codec_reader));
                        dir.close().unwrap_or_else(|e| rethrow(e));
                    }),
            ),
    );
}
