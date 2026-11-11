//! Port of `lucene/core/src/test/org/apache/lucene/index/TestSegmentMerger.java`.

use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::codec_reader::CodecReader;
use rucene::index::field_infos::FieldNumbers;
use rucene::index::merge::MergeState;
use rucene::index::segment_merger::SegmentMerger;
use rucene::index::segment_reader::SegmentReader;
use rucene::index::{IndexReader, LeafReader, SegmentCommitInfo, SegmentInfo};
use rucene::search::{Sort, NO_MORE_DOCS};
use rucene::store::{merge_io_context, IOContext, MergeInfo};
use rucene::util::concurrent::SameThreadExecutorService;
use rucene::util::string_helper::StringHelper;
use rucene::util::{default_info_stream, BytesRef, FixedBitSet, Version};
use rucene_test_framework::index::doc_helper::{self, DocHelper};
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_equals_msg, assert_not_null, random};
use crate::test_segment_reader::check_norms;

/// The instance fields of `TestSegmentMerger`.
struct TestSegmentMergerInstance {
    // The variables for the new merged segment
    /// `private Directory mergedDir`.
    merged_dir: Option<TestDirectory>,
    /// `private String mergedSegment = "test"`.
    merged_segment: String,
    // First segment to be merged
    /// `private Directory merge1Dir`.
    merge1_dir: Option<TestDirectory>,
    /// `private Document doc1 = new Document()`.
    doc1: Document,
    /// `private SegmentReader reader1 = null`.
    reader1: Option<Arc<SegmentReader>>,
    // Second Segment to be merged
    /// `private Directory merge2Dir`.
    merge2_dir: Option<TestDirectory>,
    /// `private Document doc2 = new Document()`.
    doc2: Document,
    /// `private SegmentReader reader2 = null`.
    reader2: Option<Arc<SegmentReader>>,
}

impl Default for TestSegmentMergerInstance {
    fn default() -> Self {
        TestSegmentMergerInstance {
            merged_dir: None,
            merged_segment: "test".to_string(),
            merge1_dir: None,
            doc1: Document::new(),
            reader1: None,
            merge2_dir: None,
            doc2: Document::new(),
            reader2: None,
        }
    }
}

/// A test method over the instance.
fn instance_test(name: &str, body: fn(&mut TestSegmentMergerInstance) -> Result<()>) -> Method {
    Method::new(name).throws(&["java.lang.Exception"]).body(
        move |inst: &mut TestSegmentMergerInstance| body(inst).unwrap_or_else(|e| rethrow_java(e)),
    )
}

/// `TestSegmentMerger`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestSegmentMerger")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(TestSegmentMergerInstance::default))
        .method(instance_test("setUp", set_up))
        .method(instance_test("tearDown", tear_down))
        .method(instance_test("test", test))
        .method(instance_test("testMerge", test_merge))
        .method(instance_test("testBuildDocMap", test_build_doc_map))
        .build()
}

/// `new SegmentReader(info, Version.LATEST.major, newIOContext(random()))`.
fn open_segment(info: SegmentCommitInfo) -> Result<Arc<SegmentReader>> {
    let context = LuceneTestCase::new_io_context(&*random());
    Ok(Arc::new(SegmentReader::new(
        info,
        i32::from(Version::LATEST.major),
        &*context,
    )?))
}

/// `public void setUp()`.
fn set_up(inst: &mut TestSegmentMergerInstance) -> Result<()> {
    LuceneTestCase::super_set_up();
    inst.merged_dir = Some(LuceneTestCase::new_directory());
    let merge1_dir = LuceneTestCase::new_directory();
    let merge2_dir = LuceneTestCase::new_directory();
    DocHelper::setup_doc(&mut inst.doc1);
    let info1 = DocHelper::write_doc(&*random(), merge1_dir.as_directory(), &inst.doc1)?
        .unwrap_or_else(|| crate::support::npe("writeDoc"));
    DocHelper::setup_doc(&mut inst.doc2);
    let info2 = DocHelper::write_doc(&*random(), merge2_dir.as_directory(), &inst.doc2)?
        .unwrap_or_else(|| crate::support::npe("writeDoc"));
    inst.merge1_dir = Some(merge1_dir);
    inst.merge2_dir = Some(merge2_dir);
    inst.reader1 = Some(open_segment(info1)?);
    inst.reader2 = Some(open_segment(info2)?);
    Ok(())
}

/// `public void tearDown()`.
fn tear_down(inst: &mut TestSegmentMergerInstance) -> Result<()> {
    if let Some(r) = inst.reader1.take() {
        r.close()?;
    }
    if let Some(r) = inst.reader2.take() {
        r.close()?;
    }
    for dir in [
        inst.merged_dir.take(),
        inst.merge1_dir.take(),
        inst.merge2_dir.take(),
    ]
    .into_iter()
    .flatten()
    {
        let mut dir = dir;
        dir.close()?;
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}

/// Port of `TestSegmentMerger.test`.
fn test(inst: &mut TestSegmentMergerInstance) -> Result<()> {
    Assert::assert_true(inst.merged_dir.is_some());
    Assert::assert_true(inst.merge1_dir.is_some());
    Assert::assert_true(inst.merge2_dir.is_some());
    Assert::assert_true(inst.reader1.is_some());
    Assert::assert_true(inst.reader2.is_some());
    Ok(())
}

/// Port of `TestSegmentMerger.testMerge`.
///
/// Java's `SegmentMerger.merge()` returns the `MergeState`, whose
/// `segmentInfo` the test reads; Rucene's returns nothing and hands the merged
/// `SegmentInfo` back through `into_segment_info`.
fn test_merge(inst: &mut TestSegmentMergerInstance) -> Result<()> {
    let merged_dir = inst
        .merged_dir
        .as_ref()
        .unwrap_or_else(|| crate::support::npe("mergedDir"))
        .as_directory();
    let codec =
        rucene::codecs::get_default().unwrap_or_else(|| crate::support::npe("Codec.getDefault()"));
    let si = SegmentInfo::new(
        Arc::clone(&merged_dir),
        Version::LATEST,
        None,
        inst.merged_segment.clone(),
        -1,
        false,
        false,
        Arc::clone(&codec),
        HashMap::new(),
        StringHelper::random_id(),
        HashMap::new(),
        Sort::default(),
    )?;

    let reader1 = Arc::clone(
        inst.reader1
            .as_ref()
            .unwrap_or_else(|| crate::support::npe("reader1")),
    );
    let reader2 = Arc::clone(
        inst.reader2
            .as_ref()
            .unwrap_or_else(|| crate::support::npe("reader2")),
    );
    let context: Arc<dyn IOContext> = Arc::from(LuceneTestCase::new_io_context_from(
        &*random(),
        &*merge_io_context(MergeInfo::new(-1, -1, false, -1)),
    ));
    let mut merger = SegmentMerger::new(
        &[
            Arc::clone(&reader1) as Arc<dyn CodecReader>,
            Arc::clone(&reader2) as Arc<dyn CodecReader>,
        ],
        si,
        Arc::new(FieldNumbers::new(None, None)?),
        Arc::clone(&codec),
        Arc::clone(&merged_dir),
        context,
        default_info_stream(),
        Some(Arc::new(SameThreadExecutorService::new())),
        None,
    )?;
    merger.merge()?;
    let segment_info = merger.into_segment_info();
    let docs_merged = segment_info.max_doc()?;
    Assert::assert_true(docs_merged == 2);
    // Should be able to open a new SegmentReader against the new directory
    let merged_reader = open_segment(SegmentCommitInfo::new(
        segment_info,
        0,
        0,
        -1,
        -1,
        -1,
        StringHelper::random_id(),
    )?)?;
    let merged_leaf: Arc<dyn LeafReader> = Arc::clone(&merged_reader) as _;
    Assert::assert_true(merged_leaf.num_docs() == 2);
    let new_doc1 = merged_leaf.stored_fields()?.document(0)?;
    // There are 2 unstored fields on the document
    Assert::assert_true(
        DocHelper::num_fields(&new_doc1)
            == DocHelper::num_fields(&inst.doc1) - DocHelper::unstored().len(),
    );
    let new_doc2 = merged_leaf.stored_fields()?.document(1)?;
    Assert::assert_true(
        DocHelper::num_fields(&new_doc2)
            == DocHelper::num_fields(&inst.doc2) - DocHelper::unstored().len(),
    );

    let mut term_docs = TestUtil::docs(
        &*random(),
        &(Arc::clone(&merged_reader) as Arc<dyn IndexReader>),
        doc_helper::TEXT_FIELD_2_KEY,
        &BytesRef::new(b"field".to_vec()),
        None,
        0,
    )?;
    Assert::assert_true(term_docs.is_some());
    Assert::assert_true(term_docs.as_mut().expect("checked").next_doc()? != NO_MORE_DOCS);

    let mut tv_count = 0;
    for field_info in merged_reader.get_field_infos().iter() {
        if field_info.has_term_vectors() {
            tv_count += 1;
        }
    }

    // System.out.println("stored size: " + stored.size());
    assert_equals_msg(
        Some("We do not have 3 fields that were indexed with term vector"),
        &3,
        &tv_count,
    );

    let vector = merged_leaf
        .term_vectors()?
        .get(0)?
        .unwrap_or_else(|| crate::support::npe("termVectors().get(0)"))
        .terms(doc_helper::TEXT_FIELD_2_KEY)?;
    assert_not_null(&vector);
    let vector = vector.expect("checked by assertNotNull");
    assert_equals(&3i64, &vector.size()?);
    let mut terms_enum = vector.iterator()?;

    let mut i = 0;
    while let Some(term) = terms_enum.next()? {
        let term = term.utf8_to_string()?;
        let freq = terms_enum.total_term_freq()? as i32;
        // System.out.println("Term: " + term + " Freq: " + freq);
        Assert::assert_true(doc_helper::FIELD_2_TEXT.contains(term.as_str()));
        Assert::assert_true(doc_helper::FIELD_2_FREQS[i] == freq);
        i += 1;
    }

    check_norms(&(Arc::clone(&merged_reader) as Arc<dyn LeafReader>))?;
    merged_reader.close()
}

/// Port of `TestSegmentMerger.testBuildDocMap`.
fn test_build_doc_map(_inst: &mut TestSegmentMergerInstance) -> Result<()> {
    let max_doc = TestUtil::next_int(&*random(), 1, 128);
    let num_docs = TestUtil::next_int(&*random(), 0, max_doc);
    let mut live_docs = FixedBitSet::new(max_doc as usize);
    for _i in 0..num_docs {
        loop {
            let doc_id = random().next_int_bound(max_doc);
            if !live_docs.get(doc_id as usize) {
                live_docs.set(doc_id as usize);
                break;
            }
        }
    }

    let doc_map = MergeState::remove_deletes(max_doc, &live_docs);

    // assert the mapping is compact
    let mut del = 0;
    for i in 0..max_doc {
        if !live_docs.get(i as usize) {
            del += 1;
        } else {
            assert_equals(&((i - del) as i64), &(doc_map[i as usize] as i64));
        }
    }
    Ok(())
}
