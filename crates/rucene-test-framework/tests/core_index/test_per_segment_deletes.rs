//! Port of `lucene/core/src/test/org/apache/lucene/index/TestPerSegmentDeletes.java`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::random::JdkRandom;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::{
    CfsSettings, MergeContext, MergePolicy, MergeSpecification, MergeTrigger, OneMerge,
};
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::postings_enum::{PostingsEnum, POSTINGS_ENUM_NONE};
use rucene::index::segment_infos::SegmentInfos;
use rucene::index::terms::Term;
use rucene::index::{IndexReader, SegmentCommitInfo};
use rucene::search::NO_MORE_DOCS;
use rucene::store::{ByteBuffersDirectory, Directory};

use rucene::util::{ArrayUtil, Bits, BytesRef};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::doc_helper::DocHelper;
use rucene_test_framework::store::MockDirectoryWrapper;
use rucene_test_framework::util::TestUtil;

use crate::core_support::{assert_equals, class, random, test};
use crate::support::npe;

/// `TestPerSegmentDeletes`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestPerSegmentDeletes",
        vec![test("testDeletes1", test_deletes1)],
    )
}

/// `(RangeMergePolicy) writer.getConfig().getMergePolicy()`.
fn range_merge_policy(writer: &IndexWriter) -> Arc<dyn MergePolicy> {
    let mp = writer.get_config().merge_policy();
    if mp
        .as_any()
        .and_then(|a| a.downcast_ref::<RangeMergePolicy>())
        .is_none()
    {
        randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.ClassCastException",
            "not a RangeMergePolicy",
        )
        .throw()
    }
    mp
}

/// Port of `TestPerSegmentDeletes.testDeletes1`.
fn test_deletes1() -> Result<()> {
    // IndexWriter.debug2 = System.out;
    let mut dir = MockDirectoryWrapper::new(
        &JdkRandom::new(random().next_long()),
        Arc::new(ByteBuffersDirectory::new()),
    );
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    iwc.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
    iwc.set_max_buffered_docs(5000)?;
    iwc.set_ram_buffer_size_mb(100.0)?;
    let fsmp = RangeMergePolicy::new(false);
    iwc.set_merge_policy(Some(Arc::new(fsmp)))?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    for x in 0..5 {
        writer.add_document(&DocHelper::create_document(x, "1", 2))?;
        // System.out.println("numRamDocs(" + x + ")" + writer.numRamDocs());
    }
    // System.out.println("commit1");
    writer.commit()?;
    assert_equals(&1, &writer.clone_segment_infos()?.size());
    for x in 5..10 {
        writer.add_document(&DocHelper::create_document(x, "2", 2))?;
        // System.out.println("numRamDocs(" + x + ")" + writer.numRamDocs());
    }
    // System.out.println("commit2");
    writer.commit()?;
    assert_equals(&2, &writer.clone_segment_infos()?.size());

    for x in 10..15 {
        writer.add_document(&DocHelper::create_document(x, "3", 2))?;
        // System.out.println("numRamDocs(" + x + ")" + writer.numRamDocs());
    }

    writer.delete_documents_by_terms(vec![Term::from_text("id", "1")])?;

    writer.delete_documents_by_terms(vec![Term::from_text("id", "11")])?;

    writer.flush_with(false, false)?;

    // deletes are now resolved on flush, so there shouldn't be
    // any deletes after flush
    Assert::assert_false(writer.has_changes_in_ram());

    // get reader flushes pending deletes
    // so there should not be anymore
    let r1 = directory_reader::open_with_writer(&*writer)?;
    Assert::assert_false(writer.has_changes_in_ram());
    r1.close()?;

    // delete id:2 from the first segment
    // merge segments 0 and 1
    // which should apply the delete id:2
    writer.delete_documents_by_terms(vec![Term::from_text("id", "2")])?;
    writer.flush_with(false, false)?;
    let mp = range_merge_policy(&writer);
    let fsmp = mp
        .as_any()
        .and_then(|a| a.downcast_ref::<RangeMergePolicy>())
        .expect("checked");
    fsmp.do_merge.store(true, Ordering::SeqCst);
    fsmp.start.store(0, Ordering::SeqCst);
    fsmp.length.store(2, Ordering::SeqCst);
    writer.maybe_merge()?;

    assert_equals(&2, &writer.clone_segment_infos()?.size());

    // id:2 shouldn't exist anymore because
    // it's been applied in the merge and now it's gone
    let r2: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*writer)?;
    let id2docs = to_docs_array(&Term::from_text("id", "2"), None, &r2)?;
    Assert::assert_true(id2docs.is_none());
    r2.close()?;

    /*
    // added docs are in the ram buffer
    for (int x = 15; x < 20; x++) {
      writer.addDocument(TestIndexWriterReader.createDocument(x, "4", 2));
      System.out.println("numRamDocs(" + x + ")" + writer.numRamDocs());
    }
    assertTrue(writer.numRamDocs() > 0);
    // delete from the ram buffer
    writer.deleteDocuments(new Term("id", Integer.toString(13)));

    Term id3 = new Term("id", Integer.toString(3));

    // delete from the 1st segment
    writer.deleteDocuments(id3);

    assertTrue(writer.numRamDocs() > 0);

    //System.out
    //    .println("segdels1:" + writer.docWriter.deletesToString());

    //assertTrue(writer.docWriter.segmentDeletes.size() > 0);

    // we cause a merge to happen
    fsmp.doMerge = true;
    fsmp.start = 0;
    fsmp.length = 2;
    System.out.println("maybeMerge "+writer.segmentInfos);

    SegmentInfo info0 = writer.segmentInfos.info(0);
    SegmentInfo info1 = writer.segmentInfos.info(1);

    writer.maybeMerge();
    System.out.println("maybeMerge after "+writer.segmentInfos);
    // there should be docs in RAM
    assertTrue(writer.numRamDocs() > 0);

    // assert we've merged the 1 and 2 segments
    // and still have a segment leftover == 2
    assertEquals(2, writer.segmentInfos.size());
    assertFalse(segThere(info0, writer.segmentInfos));
    assertFalse(segThere(info1, writer.segmentInfos));

    //System.out.println("segdels2:" + writer.docWriter.deletesToString());

    //assertTrue(writer.docWriter.segmentDeletes.size() > 0);

    IndexReader r = writer.getReader();
    IndexReader r1 = r.getSequentialSubReaders()[0];
    printDelDocs(r1.getLiveDocs());
    int[] docs = toDocsArray(id3, null, r);
    System.out.println("id3 docs:"+Arrays.toString(docs));
    // there shouldn't be any docs for id:3
    assertTrue(docs == null);
    r.close();

    part2(writer, fsmp);
    */
    // System.out.println("segdels2:"+writer.docWriter.segmentDeletes.toString());
    // System.out.println("close");
    writer.close()?;
    dir.close()
}

/// `void part2(IndexWriter writer, RangeMergePolicy fsmp)`.
#[allow(dead_code)]
fn part2(writer: &IndexWriter, fsmp: &RangeMergePolicy) -> Result<()> {
    for x in 20..25 {
        writer.add_document(&DocHelper::create_document(x, "5", 2))?;
        // System.out.println("numRamDocs(" + x + ")" + writer.numRamDocs());
    }
    writer.flush_with(false, false)?;
    for x in 25..30 {
        writer.add_document(&DocHelper::create_document(x, "5", 2))?;
        // System.out.println("numRamDocs(" + x + ")" + writer.numRamDocs());
    }
    writer.flush_with(false, false)?;

    // System.out.println("infos3:"+writer.segmentInfos);

    let delterm = Term::from_text("id", "8");
    writer.delete_documents_by_terms(vec![delterm])?;
    // System.out.println("segdels3:" + writer.docWriter.deletesToString());

    fsmp.do_merge.store(true, Ordering::SeqCst);
    fsmp.start.store(1, Ordering::SeqCst);
    fsmp.length.store(2, Ordering::SeqCst);
    writer.maybe_merge()

    // deletes for info1, the newly created segment from the
    // merge should have no deletes because they were applied in
    // the merge
    // SegmentInfo info1 = writer.segmentInfos.info(1);
    // assertFalse(exists(info1, writer.docWriter.segmentDeletes));

    // System.out.println("infos4:"+writer.segmentInfos);
    // System.out.println("segdels4:" + writer.docWriter.deletesToString());
}

/// `boolean segThere(SegmentCommitInfo info, SegmentInfos infos)`.
#[allow(dead_code)]
fn seg_there(info: &SegmentCommitInfo, infos: &SegmentInfos) -> bool {
    infos.iter().any(|si| si.info.name == info.info.name)
}

/// `public static void printDelDocs(Bits bits)`.
#[allow(dead_code)]
pub fn print_del_docs(bits: Option<&dyn Bits>) {
    let Some(bits) = bits else {
        return;
    };
    for x in 0..bits.length() {
        println!("{x}:{}", bits.get(x));
    }
}

/// `public int[] toDocsArray(Term term, Bits bits, IndexReader reader)`.
fn to_docs_array(
    term: &Term,
    _bits: Option<&dyn Bits>,
    reader: &Arc<dyn IndexReader>,
) -> Result<Option<Vec<i32>>> {
    let mut cterms_enum = MultiTerms::get_terms(reader, term.field())?
        .unwrap_or_else(|| npe("MultiTerms.getTerms"))
        .iterator()?;
    if cterms_enum.seek_exact(&BytesRef::from_char_sequence(&term.text()))? {
        let mut postings_enum =
            TestUtil::docs_of(&*random(), &mut *cterms_enum, None, POSTINGS_ENUM_NONE)?;
        return Ok(Some(to_array(&mut *postings_enum)?));
    }
    Ok(None)
}

/// `public static int[] toArray(PostingsEnum postingsEnum)`, with its
/// `docs[numDocs + 1] = docID` as Java writes it.
pub fn to_array(postings_enum: &mut dyn PostingsEnum) -> Result<Vec<i32>> {
    let mut docs: Vec<i32> = Vec::new();
    let num_docs = 0usize;
    while postings_enum.next_doc()? != NO_MORE_DOCS {
        let doc_id = postings_enum.doc_id();
        // `ArrayUtil.grow(docs, numDocs + 1)`.
        if docs.len() < num_docs + 1 {
            docs.resize(ArrayUtil::oversize(num_docs + 1, 4), 0);
        }
        docs[num_docs + 1] = doc_id;
    }
    Ok(docs[0..num_docs].to_vec())
}

/// `public static class RangeMergePolicy extends MergePolicy`.
pub struct RangeMergePolicy {
    do_merge: AtomicBool,
    start: AtomicI32,
    length: AtomicI32,
    use_compound_file: bool,
    /// The `MergePolicy` base fields `noCFSRatio` and `maxCFSSegmentSize`.
    cfs: CfsSettings,
}

impl RangeMergePolicy {
    fn new(use_compound_file: bool) -> Self {
        Self {
            do_merge: AtomicBool::new(false),
            start: AtomicI32::new(0),
            length: AtomicI32::new(0),
            use_compound_file,
            cfs: CfsSettings::default(),
        }
    }
}

impl std::fmt::Display for RangeMergePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RangeMergePolicy")
    }
}

impl MergePolicy for RangeMergePolicy {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn find_merges(
        &self,
        _merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let mut ms = MergeSpecification::new();
        if self.do_merge.load(Ordering::SeqCst) {
            let start = self.start.load(Ordering::SeqCst) as usize;
            let length = self.length.load(Ordering::SeqCst) as usize;
            let om = OneMerge::new(segment_infos.as_list()[start..start + length].to_vec())?;
            ms.add(om);
            self.do_merge.store(false, Ordering::SeqCst);
            return Ok(Some(ms));
        }
        Ok(None)
    }

    fn find_forced_merges(
        &self,
        _segment_infos: &SegmentInfos,
        _max_segment_count: i32,
        _segments_to_merge: &HashMap<String, bool>,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(None)
    }

    fn find_forced_deletes_merges(
        &self,
        _segment_infos: &SegmentInfos,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(None)
    }

    fn use_compound_file(
        &self,
        _segments: &SegmentInfos,
        _new_segment: &SegmentCommitInfo,
        _merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        Ok(self.use_compound_file)
    }

    fn get_no_cfs_ratio(&self) -> f64 {
        self.cfs.get_no_cfs_ratio()
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.cfs.get_max_cfs_segment_size()
    }

    fn get_max_cfs_segment_size_mb(&self) -> f64 {
        self.cfs.get_max_cfs_segment_size_mb()
    }

    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        self.cfs.set_no_cfs_ratio(no_cfs_ratio)
    }

    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        self.cfs.set_max_cfs_segment_size_mb(v)
    }
}
