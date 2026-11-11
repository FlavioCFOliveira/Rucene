//! Port of `lucene/core/src/test/org/apache/lucene/index/TestDirectoryReader.java`.
//!
//! * `Executors.newFixedThreadPool(n, new NamedThreadFactory(...))` is the
//!   `ThreadPoolExecutor` it returns ([`new_fixed_thread_pool`]); an
//!   `ExecutorService` is an [`Executor`] in Rucene.
//! * `createExecutorServiceThatFailsOneTask()`'s anonymous `ExecutorService`
//!   is [`FailsOneTask`]: it overrides `submit` as Java does, and delegates
//!   the other methods.

use std::collections::{BTreeSet, HashSet};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::{Assert, Assume};
use rucene::document::{Document, Field, FieldType, Store, StoredField, StringField, TextField};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::{self, DirectoryReader, StandardDirectoryReader};
use rucene::index::field_infos::FieldInfos;
use rucene::index::index_deletion_policy::{
    KeepOnlyLastCommitDeletionPolicy, SnapshotDeletionPolicy,
};
use rucene::index::index_reader::{CacheKey, ClosedListener};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_policy::{LogMergePolicy, NoMergePolicy};
use rucene::index::multi_bits::MultiBits;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::multi_reader::MultiReader;
use rucene::index::postings_enum::POSTINGS_ENUM_ALL;
use rucene::index::terms::Term;
use rucene::index::{IndexOptions, IndexReader, LeafReader, SegmentInfos};
use rucene::search::NO_MORE_DOCS;
use rucene::search::{Callable, Command, Executor, Future, RejectedExecutionException};
use rucene::store::{ByteArrayDataInput, Directory};
use rucene::util::{BytesRef, IOUtils, NamedThreadFactory, Version};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::doc_helper::{self, DocHelper};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::jdk::executor_service::ExecutorService;
use rucene_test_framework::jdk::thread_pool_executor::ThreadPoolExecutor;
use rucene_test_framework::util::java_exceptions::{
    expect_throws, expect_throws_any_of, java_throwable,
};
use rucene_test_framework::util::lucene_test_case::SuppressCodecs;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_equals, assert_not_null, assert_null,
    class_annotated, random, test,
};
use crate::support::{npe, set_check_index_on_close};

/// `TestDirectoryReader`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.TestDirectoryReader",
        vec![
            test("testDocument", test_document),
            test("testMultiTermDocs", test_multi_term_docs),
            test("testIsCurrent", test_is_current),
            test("testGetFieldNames", test_get_field_names),
            test("testTermVectors", test_term_vectors),
            test("testBinaryFields", test_binary_fields),
            test("testFilesOpenClose", test_files_open_close),
            test("testOpenReaderAfterDelete", test_open_reader_after_delete),
            test("testGetIndexCommit", test_get_index_commit),
            test(
                "testGetIndexCommitWithExecutorService",
                test_get_index_commit_with_executor_service,
            ),
            test("testNoDir", test_no_dir),
            test("testNoDirWithExecutor", test_no_dir_with_executor),
            test("testNoDupCommitFileNames", test_no_dup_commit_file_names),
            test("testUniqueTermCount", test_unique_term_count),
            test("testPrepareCommitIsCurrent", test_prepare_commit_is_current),
            test("testListCommits", test_list_commits),
            test("testTotalTermFreqCached", test_total_term_freq_cached),
            test("testGetSumDocFreq", test_get_sum_doc_freq),
            test("testGetDocCount", test_get_doc_count),
            test("testGetSumTotalTermFreq", test_get_sum_total_term_freq),
            test("testReaderFinishedListener", test_reader_finished_listener),
            test("testOOBDocID", test_oob_doc_id),
            test("testTryIncRef", test_try_inc_ref),
            // @SuppressForbidden(reason = "Thread sleep")
            test("testStressTryIncRef", test_stress_try_inc_ref),
            test("testLoadCertainFields", test_load_certain_fields),
            test(
                "testIndexExistsOnNonExistentDirectory",
                test_index_exists_on_non_existent_directory,
            ),
            test(
                "testOpenWithInvalidMinCompatVersion",
                test_open_with_invalid_min_compat_version,
            ),
            test(
                "testOpenWithExecutorService",
                test_open_with_executor_service,
            ),
            test(
                "testOpenWithLeafSorterAndExecutor",
                test_open_with_leaf_sorter_and_executor,
            ),
            test("testOpenCommitWithExecutor", test_open_commit_with_executor),
            test(
                "testOpenCommitWithMinVersionAndExecutor",
                test_open_commit_with_min_version_and_executor,
            ),
            test("testNullExecutorService", test_null_executor_service),
            test(
                "testExecutorServiceExceptionHandling",
                test_executor_service_exception_handling,
            ),
            test(
                "testExecutorServicePartialFailure",
                test_executor_service_partial_failure,
            ),
        ],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string()],
            })
        },
    )
}

/// `assertEquals(message, expected, actual)`.
fn aem<T: PartialEq + std::fmt::Debug + ?Sized>(message: &str, expected: &T, actual: &T) {
    assert_equals_msg(Some(message), expected, actual);
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `(LogMergePolicy) writer.getConfig().getMergePolicy()`.
fn log_merge_policy(writer: &IndexWriter) -> Arc<dyn rucene::index::merge_policy::MergePolicy> {
    let mp = writer.get_config().merge_policy();
    if mp
        .as_any()
        .and_then(|a| a.downcast_ref::<LogMergePolicy>())
        .is_none()
    {
        randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.ClassCastException",
            "not a LogMergePolicy",
        )
        .throw()
    }
    mp
}

/// The [`LogMergePolicy`] [`log_merge_policy`] returned.
fn as_lmp(mp: &Arc<dyn rucene::index::merge_policy::MergePolicy>) -> &LogMergePolicy {
    mp.as_any()
        .and_then(|a| a.downcast_ref::<LogMergePolicy>())
        .expect("checked by log_merge_policy")
}

/// `new FieldType(TextField.TYPE_STORED)` with term vectors, and term-vector
/// offsets and positions as asked.
fn tv_type(offsets: bool, positions: bool) -> Result<FieldType> {
    let ft = FieldType::new_from(TextField::type_stored());
    ft.set_store_term_vectors(true)?;
    if offsets {
        ft.set_store_term_vector_offsets(true)?;
    }
    if positions {
        ft.set_store_term_vector_positions(true)?;
    }
    Ok(ft)
}

/// `FieldType customType3 = new FieldType(); customType3.setStored(true);`.
fn stored_only_type() -> Result<FieldType> {
    let ft = FieldType::new();
    ft.set_stored(true)?;
    Ok(ft)
}

/// `Executors.newFixedThreadPool(n, new NamedThreadFactory("TestDirectoryReader"))`:
/// `new ThreadPoolExecutor(n, n, 0L, TimeUnit.MILLISECONDS, new
/// LinkedBlockingQueue<Runnable>(), threadFactory)`.
fn new_fixed_thread_pool(n: i32) -> Arc<ThreadPoolExecutor> {
    Arc::new(ThreadPoolExecutor::new(
        n,
        n,
        Duration::ZERO,
        NamedThreadFactory::new("TestDirectoryReader"),
    ))
}

/// The executor as the `ExecutorService` argument of a `DirectoryReader`
/// call.
fn exec(executor: &Arc<ThreadPoolExecutor>) -> Option<Arc<dyn Executor>> {
    Some(Arc::clone(executor) as Arc<dyn Executor>)
}

/// `(Comparator<LeafReader>) (r1, r2) -> Integer.compare(r1.numDocs(),
/// r2.numDocs())`.
#[derive(Debug)]
struct NumDocsSorter;

impl std::fmt::Display for NumDocsSorter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TestDirectoryReader$$Lambda")
    }
}

impl rucene::index::index_writer_config::LeafComparator for NumDocsSorter {
    fn compare(&self, r1: &dyn LeafReader, r2: &dyn LeafReader) -> std::cmp::Ordering {
        r1.num_docs().cmp(&r2.num_docs())
    }
}

/// Port of `TestDirectoryReader.testDocument`.
fn test_document() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let mut doc1 = Document::new();
    let mut doc2 = Document::new();
    DocHelper::setup_doc(&mut doc1);
    DocHelper::setup_doc(&mut doc2);
    DocHelper::write_doc(&*random(), dir.as_directory(), &doc1)?;
    DocHelper::write_doc(&*random(), dir.as_directory(), &doc2)?;
    let reader = directory_reader::open(dir.as_directory())?;
    Assert::assert_true(
        reader
            .as_any()
            .downcast_ref::<StandardDirectoryReader>()
            .is_some(),
    );
    let stored_fields = reader.stored_fields()?;

    let new_doc1 = stored_fields.document(0)?;
    Assert::assert_true(
        DocHelper::num_fields(&new_doc1)
            == DocHelper::num_fields(&doc1) - DocHelper::unstored().len(),
    );
    let new_doc2 = stored_fields.document(1)?;
    Assert::assert_true(
        DocHelper::num_fields(&new_doc2)
            == DocHelper::num_fields(&doc2) - DocHelper::unstored().len(),
    );
    let vector = reader
        .term_vectors()?
        .get(0)?
        .unwrap_or_else(|| npe("termVectors().get(0)"))
        .terms(doc_helper::TEXT_FIELD_2_KEY)?;
    assert_not_null(&vector);

    reader.close()?;
    dir.close()
}

/// Port of `TestDirectoryReader.testMultiTermDocs`.
fn test_multi_term_docs() -> Result<()> {
    let mut ram_dir1 = LuceneTestCase::new_directory();
    add_doc_to_dir(&*random(), &ram_dir1.as_directory(), "test foo", true)?;
    let mut ram_dir2 = LuceneTestCase::new_directory();
    add_doc_to_dir(&*random(), &ram_dir2.as_directory(), "test blah", true)?;
    let mut ram_dir3 = LuceneTestCase::new_directory();
    add_doc_to_dir(&*random(), &ram_dir3.as_directory(), "test wow", true)?;

    let readers1: Vec<Arc<dyn IndexReader>> = vec![
        directory_reader::open(ram_dir1.as_directory())?,
        directory_reader::open(ram_dir3.as_directory())?,
    ];
    let readers2: Vec<Arc<dyn IndexReader>> = vec![
        directory_reader::open(ram_dir1.as_directory())?,
        directory_reader::open(ram_dir2.as_directory())?,
        directory_reader::open(ram_dir3.as_directory())?,
    ];
    let mr2: Arc<dyn IndexReader> = Arc::new(MultiReader::new(readers1.clone(), false)?);
    let mr3: Arc<dyn IndexReader> = Arc::new(MultiReader::new(readers2.clone(), false)?);

    // test mixing up TermDocs and TermEnums from different readers.
    let mut te2 = MultiTerms::get_terms(&mr2, "body")?
        .unwrap_or_else(|| npe("getTerms(mr2, \"body\")"))
        .iterator()?;
    te2.seek_ceil(&BytesRef::from_char_sequence("wow"))?;
    let td = TestUtil::docs(&*random(), &mr2, "body", &te2.term()?, None, 0)?;

    let mut te3 = MultiTerms::get_terms(&mr3, "body")?
        .unwrap_or_else(|| npe("getTerms(mr3, \"body\")"))
        .iterator()?;
    te3.seek_ceil(&BytesRef::from_char_sequence("wow"))?;
    let mut td = TestUtil::docs_of(&*random(), &mut *te3, td, 0)?;

    let mut ret = 0;

    // This should blow up if we forget to check that the TermEnum is from the same
    // reader as the TermDocs.
    while td.next_doc()? != NO_MORE_DOCS {
        ret += td.doc_id();
    }

    // really a dummy assert to ensure that we got some docs and to ensure that
    // nothing is eliminated by hotspot
    Assert::assert_true(ret > 0);
    readers1[0].close()?;
    readers1[1].close()?;
    readers2[0].close()?;
    readers2[1].close()?;
    readers2[2].close()?;
    ram_dir1.close()?;
    ram_dir2.close()?;
    ram_dir3.close()
}

/// `private void addDoc(Random random, Directory ramDir1, String s, boolean
/// create)`.
fn add_doc_to_dir(
    random: &dyn randomizedtesting::jdk::random::Random,
    ram_dir1: &Arc<dyn Directory>,
    s: &str,
    create: bool,
) -> Result<()> {
    let iwc =
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(random))));
    iwc.set_open_mode(if create {
        OpenMode::CREATE
    } else {
        OpenMode::APPEND
    })?;
    let iw = IndexWriter::new(Arc::clone(ram_dir1), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "body",
        s,
        Store::NO,
    )));
    iw.add_document(&doc)?;
    iw.close()
}

/// Port of `TestDirectoryReader.testIsCurrent`.
fn test_is_current() -> Result<()> {
    let mut d = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(d.as_directory(), mock_iwc())?;
    add_document_with_fields(&writer)?;
    writer.close()?;
    // set up reader:
    let reader = directory_reader::open(d.as_directory())?;
    Assert::assert_true(reader.is_current()?);
    // modify index by adding another document:
    let iwc = mock_iwc();
    iwc.set_open_mode(OpenMode::APPEND)?;
    let writer = IndexWriter::new(d.as_directory(), iwc)?;
    add_document_with_fields(&writer)?;
    writer.close()?;
    Assert::assert_false(reader.is_current()?);
    // re-create index:
    let iwc = mock_iwc();
    iwc.set_open_mode(OpenMode::CREATE)?;
    let writer = IndexWriter::new(d.as_directory(), iwc)?;
    add_document_with_fields(&writer)?;
    writer.close()?;
    Assert::assert_false(reader.is_current()?);
    reader.close()?;
    d.close()
}

/// Tests the IndexReader.getFieldNames implementation
///
/// @throws Exception on error
///
/// Port of `TestDirectoryReader.testGetFieldNames`.
fn test_get_field_names() -> Result<()> {
    let mut d = LuceneTestCase::new_directory();
    // set up writer
    let writer = IndexWriter::new(d.as_directory(), mock_iwc())?;

    let mut doc = Document::new();

    let custom_type3 = stored_only_type()?;

    doc.add(Box::new(StringField::new(
        "keyword",
        "test1".to_string(),
        Store::YES,
    )?));
    doc.add(Box::new(TextField::new(
        "text",
        "test1".to_string(),
        Store::YES,
    )?));
    doc.add(Box::new(Field::new(
        "unindexed",
        "test1".to_string(),
        custom_type3.clone(),
    )?));
    doc.add(Box::new(TextField::new(
        "unstored",
        "test1".to_string(),
        Store::NO,
    )?));
    writer.add_document(&doc)?;

    writer.close()?;
    // set up reader
    let reader = directory_reader::open(d.as_directory())?;
    let field_infos =
        FieldInfos::get_merged_field_infos(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    assert_not_null(&field_infos.field_info("keyword"));
    assert_not_null(&field_infos.field_info("text"));
    assert_not_null(&field_infos.field_info("unindexed"));
    assert_not_null(&field_infos.field_info("unstored"));
    reader.close()?;
    // add more documents
    let iwc = mock_iwc();
    iwc.set_open_mode(OpenMode::APPEND)?;
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = IndexWriter::new(d.as_directory(), iwc)?;
    // want to get some more segments here
    let merge_factor = as_lmp(&log_merge_policy(&writer)).get_merge_factor();
    for _ in 0..5 * merge_factor {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "keyword",
            "test1".to_string(),
            Store::YES,
        )?));
        doc.add(Box::new(TextField::new(
            "text",
            "test1".to_string(),
            Store::YES,
        )?));
        doc.add(Box::new(Field::new(
            "unindexed",
            "test1".to_string(),
            custom_type3.clone(),
        )?));
        doc.add(Box::new(TextField::new(
            "unstored",
            "test1".to_string(),
            Store::NO,
        )?));
        writer.add_document(&doc)?;
    }
    // new fields are in some different segments (we hope)
    for _ in 0..5 * merge_factor {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "keyword2",
            "test1".to_string(),
            Store::YES,
        )?));
        doc.add(Box::new(TextField::new(
            "text2",
            "test1".to_string(),
            Store::YES,
        )?));
        doc.add(Box::new(Field::new(
            "unindexed2",
            "test1".to_string(),
            custom_type3.clone(),
        )?));
        doc.add(Box::new(TextField::new(
            "unstored2",
            "test1".to_string(),
            Store::NO,
        )?));
        writer.add_document(&doc)?;
    }
    // new termvector fields

    let custom_type5 = tv_type(false, false)?;
    let custom_type6 = tv_type(true, false)?;
    let custom_type7 = tv_type(false, true)?;
    let custom_type8 = tv_type(true, true)?;

    for _ in 0..5 * merge_factor {
        let mut doc = Document::new();
        doc.add(Box::new(TextField::new(
            "tvnot",
            "tvnot".to_string(),
            Store::YES,
        )?));
        doc.add(Box::new(Field::new(
            "termvector",
            "termvector".to_string(),
            custom_type5.clone(),
        )?));
        doc.add(Box::new(Field::new(
            "tvoffset",
            "tvoffset".to_string(),
            custom_type6.clone(),
        )?));
        doc.add(Box::new(Field::new(
            "tvposition",
            "tvposition".to_string(),
            custom_type7.clone(),
        )?));
        doc.add(Box::new(Field::new(
            "tvpositionoffset",
            "tvpositionoffset".to_string(),
            custom_type8.clone(),
        )?));
        writer.add_document(&doc)?;
    }

    writer.close()?;

    // verify fields again
    let reader = directory_reader::open(d.as_directory())?;
    let field_infos =
        FieldInfos::get_merged_field_infos(Arc::clone(&reader) as Arc<dyn IndexReader>)?;

    let mut all_field_names = HashSet::new();
    let mut indexed_field_names = HashSet::new();
    let mut not_indexed_field_names = HashSet::new();
    let mut tv_field_names: Vec<String> = Vec::new();

    for field_info in field_infos.iter() {
        let name = field_info.name.clone();
        all_field_names.insert(name.clone());
        if field_info.get_index_options() != IndexOptions::NONE {
            indexed_field_names.insert(name.clone());
        } else {
            not_indexed_field_names.insert(name.clone());
        }
        if field_info.has_term_vectors() && !tv_field_names.contains(&name) {
            tv_field_names.push(name);
        }
    }

    for name in [
        "keyword",
        "text",
        "unindexed",
        "unstored",
        "keyword2",
        "text2",
        "unindexed2",
        "unstored2",
        "tvnot",
        "termvector",
        "tvposition",
        "tvoffset",
        "tvpositionoffset",
    ] {
        Assert::assert_true(all_field_names.contains(name));
    }

    // verify that only indexed fields were returned
    assert_equals(&11, &indexed_field_names.len()); // 6 original + the 5 termvector fields
    for name in [
        "keyword",
        "text",
        "unstored",
        "keyword2",
        "text2",
        "unstored2",
        "tvnot",
        "termvector",
        "tvposition",
        "tvoffset",
        "tvpositionoffset",
    ] {
        Assert::assert_true(indexed_field_names.contains(name));
    }

    // verify that only unindexed fields were returned
    assert_equals(&2, &not_indexed_field_names.len()); // the following fields
    Assert::assert_true(not_indexed_field_names.contains("unindexed"));
    Assert::assert_true(not_indexed_field_names.contains("unindexed2"));

    // verify index term vector fields
    // `tvFieldNames.toString()`: a `HashSet`'s rendering, `[a, b]`.
    aem(
        &format!("[{}]", tv_field_names.join(", ")),
        &4,
        &tv_field_names.len(),
    ); // 4 field has term vector only
    Assert::assert_true(tv_field_names.iter().any(|n| n == "termvector"));

    reader.close()?;
    d.close()
}

/// Port of `TestDirectoryReader.testTermVectors`.
fn test_term_vectors() -> Result<()> {
    let mut d = LuceneTestCase::new_directory();
    // set up writer
    let iwc = mock_iwc();
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = IndexWriter::new(d.as_directory(), iwc)?;
    // want to get some more segments here
    // new termvector fields
    let merge_factor = as_lmp(&log_merge_policy(&writer)).get_merge_factor();
    let custom_type5 = tv_type(false, false)?;
    let custom_type6 = tv_type(true, false)?;
    let custom_type7 = tv_type(false, true)?;
    let custom_type8 = tv_type(true, true)?;
    for _ in 0..5 * merge_factor {
        let mut doc = Document::new();
        let text = "one two two three three three";
        doc.add(Box::new(TextField::new(
            "tvnot",
            text.to_string(),
            Store::YES,
        )?));
        doc.add(Box::new(Field::new(
            "termvector",
            text.to_string(),
            custom_type5.clone(),
        )?));
        doc.add(Box::new(Field::new(
            "tvoffset",
            text.to_string(),
            custom_type6.clone(),
        )?));
        doc.add(Box::new(Field::new(
            "tvposition",
            text.to_string(),
            custom_type7.clone(),
        )?));
        doc.add(Box::new(Field::new(
            "tvpositionoffset",
            text.to_string(),
            custom_type8.clone(),
        )?));

        writer.add_document(&doc)?;
    }
    writer.close()?;
    d.close()
}

/// `void assertTermDocsCount(String msg, IndexReader reader, Term term, int
/// expected)`.
#[allow(dead_code)]
fn assert_term_docs_count(
    msg: &str,
    reader: &Arc<dyn IndexReader>,
    term: &Term,
    expected: i32,
) -> Result<()> {
    let tdocs = TestUtil::docs(
        &*random(),
        reader,
        term.field(),
        &BytesRef::from_char_sequence(term.text().as_str()),
        None,
        0,
    )?;
    let mut count = 0;
    if let Some(mut tdocs) = tdocs {
        while tdocs.next_doc()? != NO_MORE_DOCS {
            count += 1;
        }
    }
    aem(&format!("{msg}, count mismatch"), &expected, &count);
    Ok(())
}

/// Checks the stored binary fields `bin1` and `bin2` of the last document.
fn check_binary_fields(dir: &Arc<dyn Directory>, bin1: &[u8], bin2: &[u8]) -> Result<()> {
    let reader = directory_reader::open(Arc::clone(dir))?;
    let doc2 = reader.stored_fields()?.document(reader.max_doc() - 1)?;
    for (name, bin) in [("bin1", bin1), ("bin2", bin2)] {
        let fields = doc2.get_fields_by_name(name);
        assert_equals(&1, &fields.len());
        let b = fields[0];
        Assert::assert_true(b.binary_value().is_some());
        let bytes_ref = b.binary_value().unwrap_or_else(|| npe("binaryValue()"));
        assert_equals(&(bin.len() as i32), &(bytes_ref.length as i32));
        for (i, expected) in bin.iter().enumerate() {
            assert_equals(expected, &bytes_ref.bytes[i + bytes_ref.offset as usize]);
        }
    }
    reader.close()
}

/// Port of `TestDirectoryReader.testBinaryFields`.
fn test_binary_fields() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let bin1: &'static [u8] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
    let bin2: &'static [u8] = &[10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0];

    let iwc = mock_iwc();
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;

    for i in 0..10 {
        add_doc(&writer, &format!("document number {}", i + 1))?;
        add_document_with_fields(&writer)?;
        add_document_with_different_fields(&writer)?;
        add_document_with_term_vector_fields(&writer)?;
    }
    writer.close()?;
    let iwc = mock_iwc();
    iwc.set_open_mode(OpenMode::APPEND)?;
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(StoredField::new_bytes(
        "bin1",
        BytesRef::new(bin1.to_vec()),
    )?));
    // `new StoredFieldDataInput(new ByteArrayDataInput(bin2))`: the whole input.
    doc.add(Box::new(StoredField::new_stored_input(
        "bin2",
        Some(Box::new(ByteArrayDataInput::new(bin2))),
        bin2.len() as i32,
    )?));
    doc.add(Box::new(TextField::new(
        "junk",
        "junk text".to_string(),
        Store::NO,
    )?));
    writer.add_document(&doc)?;
    writer.close()?;
    check_binary_fields(&dir.as_directory(), bin1, bin2)?;
    // force merge

    let iwc = mock_iwc();
    iwc.set_open_mode(OpenMode::APPEND)?;
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    writer.force_merge(1)?;
    writer.close()?;
    check_binary_fields(&dir.as_directory(), bin1, bin2)?;
    dir.close()
}

/// Port of `TestDirectoryReader.testFilesOpenClose`.
fn test_files_open_close() -> Result<()> {
    // Create initial data set
    let dir_file = LuceneTestCase::create_temp_dir_prefix("TestIndexReader.testFilesOpenClose");
    let mut dir = LuceneTestCase::new_fs_directory(&dir_file);

    let writer = IndexWriter::new(dir.as_directory(), mock_iwc())?;
    add_doc(&writer, "test")?;
    writer.close()?;
    drop(writer);
    dir.close()?;

    // Try to erase the data - this ensures that the writer closed all files
    IOUtils::rm([&dir_file])?;
    let mut dir = LuceneTestCase::new_fs_directory(&dir_file);

    // Now create the data set again, just as before
    let iwc = mock_iwc();
    iwc.set_open_mode(OpenMode::CREATE)?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    add_doc(&writer, "test")?;
    writer.close()?;
    drop(writer);
    dir.close()?;

    // Now open existing directory and test that reader closes all files
    let mut dir = LuceneTestCase::new_fs_directory(&dir_file);
    let reader1 = directory_reader::open(dir.as_directory())?;
    reader1.close()?;
    drop(reader1);
    dir.close()?;

    // The following will fail if reader did not close
    // all files
    IOUtils::rm([&dir_file])
}

/// Port of `TestDirectoryReader.testOpenReaderAfterDelete`.
fn test_open_reader_after_delete() -> Result<()> {
    let dir_file = LuceneTestCase::create_temp_dir_prefix("deletetest");
    let mut dir = LuceneTestCase::new_fs_directory(&dir_file);
    set_check_index_on_close(&dir, false); // we will hit NoSuchFileException in MDW since we nuked it!
    expect_throws_any_of(
        &[
            "java.io.FileNotFoundException",
            "java.nio.file.NoSuchFileException",
        ],
        || directory_reader::open(dir.as_directory()),
    );

    IOUtils::rm([&dir_file])?;

    // Make sure we still get a CorruptIndexException (not NPE):
    expect_throws_any_of(
        &[
            "java.io.FileNotFoundException",
            "java.nio.file.NoSuchFileException",
        ],
        || directory_reader::open(dir.as_directory()),
    );

    dir.close()
}

/// `static void addDocumentWithFields(IndexWriter writer)`.
pub fn add_document_with_fields(writer: &IndexWriter) -> Result<()> {
    let mut doc = Document::new();

    let custom_type3 = stored_only_type()?;
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "keyword",
        "test1",
        Store::YES,
    )));
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "text",
        "test1",
        Store::YES,
    )));
    doc.add(Box::new(LuceneTestCase::new_field(
        "unindexed",
        "test1",
        &custom_type3,
    )));
    doc.add(Box::new(TextField::new(
        "unstored",
        "test1".to_string(),
        Store::NO,
    )?));
    writer.add_document(&doc)?;
    Ok(())
}

/// `static void addDocumentWithDifferentFields(IndexWriter writer)`.
pub fn add_document_with_different_fields(writer: &IndexWriter) -> Result<()> {
    let mut doc = Document::new();

    let custom_type3 = stored_only_type()?;
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "keyword2",
        "test1",
        Store::YES,
    )));
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "text2",
        "test1",
        Store::YES,
    )));
    doc.add(Box::new(LuceneTestCase::new_field(
        "unindexed2",
        "test1",
        &custom_type3,
    )));
    doc.add(Box::new(TextField::new(
        "unstored2",
        "test1".to_string(),
        Store::NO,
    )?));
    writer.add_document(&doc)?;
    Ok(())
}

/// `static void addDocumentWithTermVectorFields(IndexWriter writer)`.
pub fn add_document_with_term_vector_fields(writer: &IndexWriter) -> Result<()> {
    let mut doc = Document::new();
    let custom_type5 = tv_type(false, false)?;
    let custom_type6 = tv_type(true, false)?;
    let custom_type7 = tv_type(false, true)?;
    let custom_type8 = tv_type(true, true)?;
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "tvnot",
        "tvnot",
        Store::YES,
    )));
    doc.add(Box::new(LuceneTestCase::new_field(
        "termvector",
        "termvector",
        &custom_type5,
    )));
    doc.add(Box::new(LuceneTestCase::new_field(
        "tvoffset",
        "tvoffset",
        &custom_type6,
    )));
    doc.add(Box::new(LuceneTestCase::new_field(
        "tvposition",
        "tvposition",
        &custom_type7,
    )));
    doc.add(Box::new(LuceneTestCase::new_field(
        "tvpositionoffset",
        "tvpositionoffset",
        &custom_type8,
    )));

    writer.add_document(&doc)?;
    Ok(())
}

/// `static void addDoc(IndexWriter writer, String value)`.
pub fn add_doc(writer: &IndexWriter, value: &str) -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "content",
        value,
        Store::NO,
    )));
    writer.add_document(&doc)?;
    Ok(())
}

// TODO: maybe this can reuse the logic of test dueling codecs?
/// `public static void assertIndexEquals(DirectoryReader index1,
/// DirectoryReader index2)`.
pub fn assert_index_equals(
    index1: &Arc<dyn DirectoryReader>,
    index2: &Arc<dyn DirectoryReader>,
) -> Result<()> {
    let r1: Arc<dyn IndexReader> = Arc::clone(index1) as Arc<dyn IndexReader>;
    let r2: Arc<dyn IndexReader> = Arc::clone(index2) as Arc<dyn IndexReader>;
    aem(
        "IndexReaders have different values for numDocs.",
        &index1.num_docs(),
        &index2.num_docs(),
    );
    aem(
        "IndexReaders have different values for maxDoc.",
        &index1.max_doc(),
        &index2.max_doc(),
    );
    aem(
        "Only one IndexReader has deletions.",
        &index1.has_deletions(),
        &index2.has_deletions(),
    );
    aem(
        "Single segment test differs.",
        &(Arc::clone(&r1).leaves()?.len() == 1),
        &(Arc::clone(&r2).leaves()?.len() == 1),
    );

    // check field names
    let field_infos1 = FieldInfos::get_merged_field_infos(Arc::clone(&r1))?;
    let field_infos2 = FieldInfos::get_merged_field_infos(Arc::clone(&r2))?;
    aem(
        "IndexReaders have different numbers of fields.",
        &field_infos1.size(),
        &field_infos2.size(),
    );
    let num_fields = field_infos1.size();
    for field_id in 0..num_fields as i32 {
        let field_info1 = field_infos1
            .field_info_by_number(field_id)
            .unwrap_or_else(|| npe("fieldInfo1"));
        let field_info2 = field_infos2
            .field_info_by_number(field_id)
            .unwrap_or_else(|| npe("fieldInfo2"));
        aem(
            "Different field names.",
            &field_info1.name,
            &field_info2.name,
        );
    }

    // check norms
    for field_info in field_infos1.iter() {
        let cur_field = &field_info.name;
        let norms1 = MultiDocValues::get_norm_values(&r1, cur_field)?;
        let norms2 = MultiDocValues::get_norm_values(&r2, cur_field)?;
        match (norms1, norms2) {
            (Some(mut norms1), Some(mut norms2)) => {
                // todo: generalize this (like TestDuelingCodecs assert)
                loop {
                    let doc_id = norms1.next_doc()?;
                    assert_equals(&doc_id, &norms2.next_doc()?);
                    if doc_id == NO_MORE_DOCS {
                        break;
                    }
                    aem(
                        &format!("Norm different for doc {doc_id} and field '{cur_field}'."),
                        &norms1.long_value()?,
                        &norms2.long_value()?,
                    );
                }
            }
            (norms1, norms2) => {
                assert_null(&norms1.map(|_| ()));
                assert_null(&norms2.map(|_| ()));
            }
        }
    }

    // check deletions
    let live_docs1 = MultiBits::get_live_docs(&r1)?;
    let live_docs2 = MultiBits::get_live_docs(&r2)?;
    for i in 0..index1.max_doc() {
        aem(
            &format!("Doc {i} only deleted in one index."),
            &(live_docs1.as_ref().is_none_or(|l| !l.get(i as usize))),
            &(live_docs2.as_ref().is_none_or(|l| !l.get(i as usize))),
        );
    }

    // check stored fields
    let stored_fields1 = index1.stored_fields()?;
    let stored_fields2 = index2.stored_fields()?;
    for i in 0..index1.max_doc() {
        if live_docs1.as_ref().is_none_or(|l| l.get(i as usize)) {
            let doc1 = stored_fields1.document(i)?;
            let doc2 = stored_fields2.document(i)?;
            let field1 = doc1.get_fields();
            let field2 = doc2.get_fields();
            aem(
                &format!("Different numbers of fields for doc {i}."),
                &field1.len(),
                &field2.len(),
            );
            for (cur_field1, cur_field2) in field1.iter().zip(field2.iter()) {
                aem(
                    &format!("Different fields names for doc {i}."),
                    &cur_field1.name(),
                    &cur_field2.name(),
                );
                aem(
                    &format!("Different field values for doc {i}."),
                    &cur_field1.string_value(),
                    &cur_field2.string_value(),
                );
            }
        }
    }

    // check dictionary and posting lists
    let fields1: BTreeSet<String> = FieldInfos::get_indexed_fields(Arc::clone(&r1))?
        .into_iter()
        .collect();
    let fields2: BTreeSet<String> = FieldInfos::get_indexed_fields(Arc::clone(&r2))?
        .into_iter()
        .collect();
    let mut fenum2 = fields2.iter();
    for field1 in &fields1 {
        aem(
            "Different fields",
            field1,
            fenum2.next().unwrap_or_else(|| {
                randomizedtesting::jdk::exception::Throwable::new(
                    "java.util.NoSuchElementException",
                    "",
                )
                .throw()
            }),
        );
        let Some(terms1) = MultiTerms::get_terms(&r1, field1)? else {
            assert_null(&MultiTerms::get_terms(&r2, field1)?.map(|_| ()));
            continue;
        };
        let mut enum1 = terms1.iterator()?;

        let terms2 = MultiTerms::get_terms(&r2, field1)?;
        assert_not_null(&terms2);
        let mut enum2 = terms2.unwrap_or_else(|| npe("terms2")).iterator()?;

        while enum1.next()?.is_some() {
            aem("Different terms", &Some(enum1.term()?), &enum2.next()?);
            let mut tp1 = enum1.postings(None, POSTINGS_ENUM_ALL)?;
            let mut tp2 = enum2.postings(None, POSTINGS_ENUM_ALL)?;

            while tp1.next_doc()? != NO_MORE_DOCS {
                Assert::assert_true(tp2.next_doc()? != NO_MORE_DOCS);
                let term = enum1.term()?;
                aem(
                    &format!("Different doc id in postinglist of term {term}."),
                    &tp1.doc_id(),
                    &tp2.doc_id(),
                );
                aem(
                    &format!("Different term frequence in postinglist of term {term}."),
                    &tp1.freq()?,
                    &tp2.freq()?,
                );
                for _ in 0..tp1.freq()? {
                    aem(
                        &format!("Different positions in postinglist of term {term}."),
                        &tp1.next_position()?,
                        &tp2.next_position()?,
                    );
                }
            }
        }
    }
    Assert::assert_false(fenum2.next().is_some());
    Ok(())
}

/// The writer config `testGetIndexCommit` builds: `setMaxBufferedDocs(2)`,
/// `setMergePolicy(newLogMergePolicy(10))`, and `APPEND` when asked.
fn get_index_commit_iwc(append: bool) -> Result<IndexWriterConfig> {
    let iwc = mock_iwc();
    if append {
        iwc.set_open_mode(OpenMode::APPEND)?;
    }
    iwc.set_max_buffered_docs(2)?;
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(10)?,
    )))?;
    Ok(iwc)
}

/// The body `testGetIndexCommit` and `testGetIndexCommitWithExecutorService`
/// share; `executor` is the `ExecutorService` of the second.
fn get_index_commit_body(
    d: &Arc<dyn Directory>,
    executor: Option<Arc<dyn Executor>>,
) -> Result<()> {
    let open_if_changed = |r: &Arc<dyn DirectoryReader>| match &executor {
        None => directory_reader::open_if_changed(Arc::clone(r)),
        Some(e) => {
            directory_reader::open_if_changed_with_executor(Arc::clone(r), Some(Arc::clone(e)))
        }
    };
    // set up writer
    let writer = IndexWriter::new(Arc::clone(d), get_index_commit_iwc(false)?)?;
    for _ in 0..27 {
        add_document_with_fields(&writer)?;
    }
    writer.close()?;

    let sis = SegmentInfos::read_latest_commit(Arc::clone(d))?;
    let r = directory_reader::open(Arc::clone(d))?;
    let c = r.index_commit()?;

    assert_equals(&sis.segments_file_name(), &Some(c.get_segments_file_name()));

    Assert::assert_true(c.commit_equals(r.index_commit()?.as_ref()));

    // Change the index
    let writer = IndexWriter::new(Arc::clone(d), get_index_commit_iwc(true)?)?;
    for _ in 0..7 {
        add_document_with_fields(&writer)?;
    }
    writer.close()?;

    let r2 = open_if_changed(&r)?;
    assert_not_null(&r2);
    let r2 = r2.unwrap_or_else(|| npe("r2"));
    Assert::assert_false(c.commit_equals(r2.index_commit()?.as_ref()));
    assert_not_equals(&1, &r2.index_commit()?.get_segment_count());
    r2.close()?;

    let iwc = mock_iwc();
    iwc.set_open_mode(OpenMode::APPEND)?;
    let writer = IndexWriter::new(Arc::clone(d), iwc)?;
    writer.force_merge(1)?;
    writer.close()?;

    let r2 = open_if_changed(&r)?;
    assert_not_null(&r2);
    let r2 = r2.unwrap_or_else(|| npe("r2"));
    assert_null(&open_if_changed(&r2)?.map(|_| ()));
    assert_equals(&1, &r2.index_commit()?.get_segment_count());

    r.close()?;
    r2.close()
}

/// Port of `TestDirectoryReader.testGetIndexCommit`.
fn test_get_index_commit() -> Result<()> {
    let mut d = LuceneTestCase::new_directory();
    get_index_commit_body(&d.as_directory(), None)?;
    d.close()
}

/// Port of `TestDirectoryReader.testGetIndexCommitWithExecutorService`.
fn test_get_index_commit_with_executor_service() -> Result<()> {
    let mut d = LuceneTestCase::new_directory();
    let executor_service = new_fixed_thread_pool(4);

    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        get_index_commit_body(&d.as_directory(), exec(&executor_service))
    }));
    // finally
    executor_service.shutdown();
    let closed = d.close();
    finish(result)?;
    closed
}

/// Rethrows the outcome of a `try` body once its `finally` ran.
fn finish(result: std::thread::Result<Result<()>>) -> Result<()> {
    match result {
        Ok(result) => result,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// `static Document createDocument(String id)`.
pub fn create_document(id: &str) -> Result<Document> {
    let mut doc = Document::new();
    let custom_type = FieldType::new_from(TextField::type_stored());
    custom_type.set_tokenized(false)?;
    custom_type.set_omit_norms(true)?;

    doc.add(Box::new(LuceneTestCase::new_field("id", id, &custom_type)));
    Ok(doc)
}

// LUCENE-1468 -- make sure on attempting to open an
// DirectoryReader on a non-existent directory, you get a
// good exception
/// Port of `TestDirectoryReader.testNoDir`.
fn test_no_dir() -> Result<()> {
    let temp_dir = LuceneTestCase::create_temp_dir_prefix("doesnotexist");
    let mut dir = LuceneTestCase::new_fs_directory(&temp_dir);
    expect_throws("org.apache.lucene.index.IndexNotFoundException", || {
        directory_reader::open(dir.as_directory())
    });
    dir.close()
}

/// Port of `TestDirectoryReader.testNoDirWithExecutor`.
fn test_no_dir_with_executor() -> Result<()> {
    let temp_dir = LuceneTestCase::create_temp_dir_prefix("doesnotexist");
    let mut dir = LuceneTestCase::new_fs_directory(&temp_dir);
    let executor_service = new_fixed_thread_pool(4);
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        expect_throws("org.apache.lucene.index.IndexNotFoundException", || {
            directory_reader::open_with_executor(dir.as_directory(), exec(&executor_service))
        });
    }));
    // finally
    let closed = dir.close();
    executor_service.shutdown();
    finish(result.map(Ok))?;
    closed
}

// LUCENE-1509
/// Port of `TestDirectoryReader.testNoDupCommitFileNames`.
fn test_no_dup_commit_file_names() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();

    let iwc = mock_iwc();
    iwc.set_max_buffered_docs(2)?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    writer.add_document(&create_document("a")?)?;
    writer.add_document(&create_document("a")?)?;
    writer.add_document(&create_document("a")?)?;
    writer.close()?;

    let commits = directory_reader::list_commits(dir.as_directory())?;
    for commit in &commits {
        // `commit.getFileNames()` is a `Collection`; a duplicate would be a
        // second entry of the same name.
        let files = commit.get_file_names()?;
        let mut seen = HashSet::new();
        for file_name in &files {
            Assert::assert_false_msg(
                &format!("file {file_name} was duplicated"),
                seen.contains(file_name),
            );
            seen.insert(file_name.clone());
        }
    }

    dir.close()
}

// LUCENE-1586: getUniqueTermCount
/// Port of `TestDirectoryReader.testUniqueTermCount`.
fn test_unique_term_count() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), mock_iwc())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "a b c d e f g h i j k l m n o p q r s t u v w x y z",
        Store::NO,
    )));
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "number",
        "0 1 2 3 4 5 6 7 8 9",
        Store::NO,
    )));
    writer.add_document(&doc)?;
    writer.add_document(&doc)?;
    writer.commit()?;

    let r = directory_reader::open(dir.as_directory())?;
    let r1 = LuceneTestCase::get_only_leaf_reader(Arc::clone(&r) as Arc<dyn IndexReader>)?;
    assert_equals(&26, &terms_size(r1.as_ref(), "field")?);
    assert_equals(&10, &terms_size(r1.as_ref(), "number")?);
    writer.add_document(&doc)?;
    writer.commit()?;
    let r2 = directory_reader::open_if_changed(Arc::clone(&r))?;
    assert_not_null(&r2);
    let r2 = r2.unwrap_or_else(|| npe("r2"));
    r.close()?;

    for s in (Arc::clone(&r2) as Arc<dyn IndexReader>).leaves()? {
        assert_equals(&26, &terms_size(s.leaf_reader().as_ref(), "field")?);
        assert_equals(&10, &terms_size(s.leaf_reader().as_ref(), "number")?);
    }
    r2.close()?;
    writer.close()?;
    dir.close()
}

/// `reader.terms(field).size()`.
fn terms_size(reader: &dyn LeafReader, field: &str) -> Result<i64> {
    reader
        .terms(field)?
        .unwrap_or_else(|| npe("terms(field)"))
        .size()
}

// LUCENE-2046
/// Port of `TestDirectoryReader.testPrepareCommitIsCurrent`.
fn test_prepare_commit_is_current() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), mock_iwc())?;
    writer.commit()?;
    let doc = Document::new();
    writer.add_document(&doc)?;
    let r = directory_reader::open(dir.as_directory())?;
    Assert::assert_true(r.is_current()?);
    writer.add_document(&doc)?;
    writer.prepare_commit()?;
    Assert::assert_true(r.is_current()?);
    let r2 = directory_reader::open_if_changed(Arc::clone(&r))?;
    assert_null(&r2.map(|_| ()));
    writer.commit()?;
    Assert::assert_false(r.is_current()?);
    writer.close()?;
    r.close()?;
    dir.close()
}

// LUCENE-2753
/// Port of `TestDirectoryReader.testListCommits`.
fn test_list_commits() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(None);
    let sdp = Arc::new(SnapshotDeletionPolicy::new(Some(Arc::new(
        KeepOnlyLastCommitDeletionPolicy,
    ))));
    iwc.set_index_deletion_policy(Some(Arc::clone(&sdp) as _))?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    // `(SnapshotDeletionPolicy) writer.getConfig().getIndexDeletionPolicy()`:
    // the policy installed above.
    writer.add_document(&Document::new())?;
    writer.commit()?;
    sdp.snapshot()?;
    writer.add_document(&Document::new())?;
    writer.commit()?;
    sdp.snapshot()?;
    writer.add_document(&Document::new())?;
    writer.commit()?;
    sdp.snapshot()?;
    writer.close()?;
    let mut current_gen = 0i64;
    for ic in directory_reader::list_commits(dir.as_directory())? {
        Assert::assert_true_msg(
            &format!("currentGen={current_gen} commitGen={}", ic.get_generation()),
            current_gen < ic.get_generation(),
        );
        current_gen = ic.get_generation();
    }
    dir.close()
}

/// `DirectoryReader r = DirectoryReader.open(writer)` over a writer holding
/// `docs`, the writer then closed.
fn nrt_reader_over(dir: &Arc<dyn Directory>, docs: &[&str]) -> Result<Arc<dyn DirectoryReader>> {
    let writer = IndexWriter::new(Arc::clone(dir), mock_iwc())?;
    for text in docs {
        let mut d = Document::new();
        d.add(Box::new(LuceneTestCase::new_text_field(
            "f",
            text,
            Store::NO,
        )));
        writer.add_document(&d)?;
    }
    let r = directory_reader::open_with_writer(writer.as_ref())?;
    writer.close()?;
    Ok(r)
}

// Make sure totalTermFreq works correctly in the terms
// dict cache
/// Port of `TestDirectoryReader.testTotalTermFreqCached`.
fn test_total_term_freq_cached() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let r = nrt_reader_over(&dir.as_directory(), &["a a b"])?;
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<()> {
        // Make sure codec impls totalTermFreq (eg PreFlex doesn't)
        Assume::assume_true(
            r.total_term_freq(&Term::new("f", BytesRef::from_char_sequence("b")))? != -1,
        );
        assert_equals(
            &1,
            &r.total_term_freq(&Term::new("f", BytesRef::from_char_sequence("b")))?,
        );
        assert_equals(
            &2,
            &r.total_term_freq(&Term::new("f", BytesRef::from_char_sequence("a")))?,
        );
        assert_equals(
            &1,
            &r.total_term_freq(&Term::new("f", BytesRef::from_char_sequence("b")))?,
        );
        Ok(())
    }));
    // finally
    r.close()?;
    dir.close()?;
    finish(result)
}

/// Port of `TestDirectoryReader.testGetSumDocFreq`.
fn test_get_sum_doc_freq() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let r = nrt_reader_over(&dir.as_directory(), &["a", "b"])?;
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<()> {
        // Make sure codec impls getSumDocFreq (eg PreFlex doesn't)
        Assume::assume_true(r.get_sum_doc_freq("f")? != -1);
        assert_equals(&2, &r.get_sum_doc_freq("f")?);
        Ok(())
    }));
    // finally
    r.close()?;
    dir.close()?;
    finish(result)
}

/// Port of `TestDirectoryReader.testGetDocCount`.
fn test_get_doc_count() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let r = nrt_reader_over(&dir.as_directory(), &["a", "a"])?;
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<()> {
        // Make sure codec impls getSumDocFreq (eg PreFlex doesn't)
        Assume::assume_true(r.get_doc_count("f")? != -1);
        assert_equals(&2, &r.get_doc_count("f")?);
        Ok(())
    }));
    // finally
    r.close()?;
    dir.close()?;
    finish(result)
}

/// Port of `TestDirectoryReader.testGetSumTotalTermFreq`.
fn test_get_sum_total_term_freq() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let r = nrt_reader_over(&dir.as_directory(), &["a b b", "a a b"])?;
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<()> {
        // Make sure codec impls getSumDocFreq (eg PreFlex doesn't)
        Assume::assume_true(r.get_sum_total_term_freq("f")? != -1);
        assert_equals(&6, &r.get_sum_total_term_freq("f")?);
        Ok(())
    }));
    // finally
    r.close()?;
    dir.close()?;
    finish(result)
}

/// `final IndexReader.ClosedListener listener = key -> closeCount[0]++;`.
struct CountingListener(Arc<AtomicI32>);

impl ClosedListener for CountingListener {
    fn on_close(&self, _key: CacheKey) -> Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

// LUCENE-2474
/// Port of `TestDirectoryReader.testReaderFinishedListener`.
fn test_reader_finished_listener() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    as_lmp(&log_merge_policy(&writer)).set_merge_factor(3)?;
    writer.add_document(&Document::new())?;
    writer.commit()?;
    writer.add_document(&Document::new())?;
    writer.commit()?;
    let reader = directory_reader::open_with_writer(writer.as_ref())?;
    let close_count = Arc::new(AtomicI32::new(0));

    reader
        .get_reader_cache_helper()
        .unwrap_or_else(|| npe("getReaderCacheHelper()"))
        .add_closed_listener(Box::new(CountingListener(Arc::clone(&close_count))));

    reader.close()?;

    // Close the top reader, it's the only one that should be closed
    assert_equals(&1, &close_count.load(Ordering::SeqCst));
    writer.close()?;

    let reader2 = directory_reader::open(dir.as_directory())?;
    reader2
        .get_reader_cache_helper()
        .unwrap_or_else(|| npe("getReaderCacheHelper()"))
        .add_closed_listener(Box::new(CountingListener(Arc::clone(&close_count))));

    close_count.store(0, Ordering::SeqCst);
    reader2.close()?;
    assert_equals(&1, &close_count.load(Ordering::SeqCst));
    dir.close()
}

/// Port of `TestDirectoryReader.testOOBDocID`.
fn test_oob_doc_id() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), mock_iwc())?;
    writer.add_document(&Document::new())?;
    let r = directory_reader::open_with_writer(writer.as_ref())?;
    writer.close()?;
    r.stored_fields()?.document(0)?;
    expect_throws("java.lang.IllegalArgumentException", || {
        r.stored_fields()?.document(1)
    });
    r.close()?;
    dir.close()
}

/// Port of `TestDirectoryReader.testTryIncRef`.
fn test_try_inc_ref() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), mock_iwc())?;
    writer.add_document(&Document::new())?;
    writer.commit()?;
    let r = directory_reader::open(dir.as_directory())?;
    Assert::assert_true(r.try_inc_ref());
    r.dec_ref()?;
    r.close()?;
    Assert::assert_false(r.try_inc_ref());
    writer.close()?;
    dir.close()
}

/// Port of `TestDirectoryReader.testStressTryIncRef`.
fn test_stress_try_inc_ref() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), mock_iwc())?;
    writer.add_document(&Document::new())?;
    writer.commit()?;
    let r = directory_reader::open(dir.as_directory())?;
    let num_threads = LuceneTestCase::at_least(2);

    let mut threads = Vec::with_capacity(num_threads as usize);
    for _ in 0..num_threads {
        let thread = IncThread::new(Arc::clone(&r) as Arc<dyn IndexReader>, random());
        thread.thread.start();
        threads.push(thread);
    }
    Thread::sleep(100);

    Assert::assert_true(r.try_inc_ref());
    r.dec_ref()?;
    r.close()?;

    for thread in &threads {
        if let Err(payload) = thread.thread.join() {
            std::panic::resume_unwind(payload);
        }
        assert_null(&*thread.failed.lock().unwrap_or_else(|e| e.into_inner()));
    }
    Assert::assert_false(r.try_inc_ref());
    writer.close()?;
    dir.close()
}

/// `static class IncThread extends Thread`.
struct IncThread {
    thread: Thread,
    /// `Throwable failed`.
    failed: Arc<Mutex<Option<String>>>,
}

impl IncThread {
    /// `IncThread(IndexReader toInc, Random random)`.
    fn new(
        to_inc: Arc<dyn IndexReader>,
        random: Arc<dyn randomizedtesting::jdk::random::Random>,
    ) -> Self {
        let failed: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let thread = {
            let failed = Arc::clone(&failed);
            Thread::unnamed(move || {
                let _random = random;
                let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<()> {
                    while to_inc.try_inc_ref() {
                        Assert::assert_false(to_inc.has_deletions());
                        to_inc.dec_ref()?;
                    }
                    Assert::assert_false(to_inc.try_inc_ref());
                    Ok(())
                }));
                let failure = match outcome {
                    Ok(Ok(())) => None,
                    Ok(Err(e)) => Some(java_throwable(&e).to_string()),
                    Err(payload) => Some(rucene::util::concurrent::panic_text(payload.as_ref())),
                };
                if failure.is_some() {
                    *failed.lock().unwrap_or_else(|e| e.into_inner()) = failure;
                }
            })
        };
        Self { thread, failed }
    }
}

/// Port of `TestDirectoryReader.testLoadCertainFields`.
fn test_load_certain_fields() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "field1",
        "foobar",
        Store::YES,
    )));
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "field2",
        "foobaz",
        Store::YES,
    )));
    writer.add_document(&doc)?;
    let r = writer.get_reader()?;
    writer.close()?;
    let mut fields_to_load = HashSet::new();
    assert_equals(
        &0,
        &r.stored_fields()?
            .document_fields(0, &fields_to_load)?
            .get_fields()
            .len(),
    );
    fields_to_load.insert("field1".to_string());
    let doc2 = r.stored_fields()?.document_fields(0, &fields_to_load)?;
    assert_equals(&1, &doc2.get_fields().len());
    assert_equals(&Some("foobar".to_string()), &doc2.get("field1"));
    r.close()?;
    dir.close()
}

/// Port of `TestDirectoryReader.testIndexExistsOnNonExistentDirectory`.
fn test_index_exists_on_non_existent_directory() -> Result<()> {
    let temp_dir = LuceneTestCase::create_temp_dir_prefix("testIndexExistsOnNonExistentDirectory");
    let mut dir = LuceneTestCase::new_fs_directory(&temp_dir);
    Assert::assert_false(directory_reader::index_exists(dir.as_directory().as_ref())?);
    dir.close()
}

/// Port of `TestDirectoryReader.testOpenWithInvalidMinCompatVersion`.
fn test_open_with_invalid_min_compat_version() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let body = (|| -> Result<()> {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "field1",
            "foobar",
            Store::YES,
        )));
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "field2",
            "foobaz",
            Store::YES,
        )));
        writer.add_document(&doc)?;
        writer.commit()?;
        let commit = Arc::clone(&directory_reader::list_commits(dir.as_directory())?[0]);
        expect_throws("java.lang.IllegalArgumentException", || {
            directory_reader::open_with_commit_min_version(commit.as_ref(), -1, None)
        });
        directory_reader::open_with_commit_min_version(
            commit.as_ref(),
            random().next_int_bound(i32::from(Version::LATEST.major) + 1),
            None,
        )?
        .close()?;
        Ok(())
    })();
    // try-with-resources: the writer, then the directory.
    let closed_writer = writer.close();
    drop(writer);
    let closed_dir = dir.close();
    body?;
    closed_writer?;
    closed_dir
}

/// Port of `TestDirectoryReader.testOpenWithExecutorService`.
fn test_open_with_executor_service() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    create_multi_segment_index(&dir.as_directory(), 5)?;

    let executor = new_fixed_thread_pool(1);
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<()> {
        let reader = directory_reader::open_with_executor(dir.as_directory(), exec(&executor))?;
        Assert::assert_true(reader.num_docs() > 0);
        reader.close()
    }));
    // finally
    executor.shutdown();
    dir.close()?;
    finish(result)
}

/// Port of `TestDirectoryReader.testOpenWithLeafSorterAndExecutor`.
fn test_open_with_leaf_sorter_and_executor() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    create_multi_segment_index(&dir.as_directory(), 3)?;

    let executor = new_fixed_thread_pool(1);
    let sorter = Arc::new(NumDocsSorter);

    let result = std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<()> {
        let reader = directory_reader::open_with_leaf_sorter_and_executor(
            dir.as_directory(),
            Some(sorter),
            exec(&executor),
        )?;
        reader.close()
    }));
    // finally
    executor.shutdown();
    dir.close()?;
    finish(result)
}

/// Port of `TestDirectoryReader.testOpenCommitWithExecutor`.
fn test_open_commit_with_executor() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        "1",
        Store::YES,
    )));
    writer.add_document(&doc)?;
    writer.commit()?;
    writer.close()?;

    let commit = Arc::clone(&directory_reader::list_commits(dir.as_directory())?[0]);
    let executor = new_fixed_thread_pool(1);
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<()> {
        let reader =
            directory_reader::open_with_commit_and_executor(commit.as_ref(), exec(&executor))?;
        assert_equals(&1, &reader.num_docs());
        reader.close()
    }));
    // finally
    executor.shutdown();
    drop(commit);
    dir.close()?;
    finish(result)
}

/// Port of `TestDirectoryReader.testOpenCommitWithMinVersionAndExecutor`.
fn test_open_commit_with_min_version_and_executor() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    create_multi_segment_index(&dir.as_directory(), 3)?;

    let executor = new_fixed_thread_pool(1);
    let sorter = Arc::new(NumDocsSorter);

    let result = std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<()> {
        let base_reader = directory_reader::open(dir.as_directory())?;
        let body = (|| -> Result<()> {
            let commit = base_reader.index_commit()?;
            let reader = directory_reader::open_with_commit_min_version_and_executor(
                commit.as_ref(),
                Version::MIN_SUPPORTED_MAJOR,
                Some(sorter),
                exec(&executor),
            )?;
            assert_equals(&base_reader.num_docs(), &reader.num_docs());
            reader.close()
        })();
        // try-with-resources
        let closed = base_reader.close();
        body?;
        closed
    }));
    // finally
    executor.shutdown();
    dir.close()?;
    finish(result)
}

/// Port of `TestDirectoryReader.testNullExecutorService`.
fn test_null_executor_service() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    create_multi_segment_index(&dir.as_directory(), 3)?;

    // Should work fine with null executor (fallback to sequential)
    let reader = directory_reader::open_with_executor(dir.as_directory(), None)?;
    Assert::assert_true(reader.num_docs() > 0);
    reader.close()?;
    dir.close()
}

/// Port of `TestDirectoryReader.testExecutorServiceExceptionHandling`.
fn test_executor_service_exception_handling() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    create_multi_segment_index(&dir.as_directory(), 3)?;

    // Test with shutdown executor
    let executor = new_fixed_thread_pool(1);
    executor.shutdown();

    expect_throws("java.lang.RuntimeException", || {
        directory_reader::open_with_executor(dir.as_directory(), exec(&executor))
    });

    dir.close()
}

/// Port of `TestDirectoryReader.testExecutorServicePartialFailure`.
fn test_executor_service_partial_failure() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    create_multi_segment_index(&dir.as_directory(), 3)?;

    // Create an executor that fails on the second task but succeeds on others
    let executor = create_executor_service_that_fails_one_task();

    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        expect_throws("java.lang.RuntimeException", || {
            directory_reader::open_with_executor(
                dir.as_directory(),
                Some(Arc::clone(&executor) as Arc<dyn Executor>),
            )
        });
    }));
    // finally
    executor.shutdown();
    dir.close()?;
    finish(result.map(Ok))
}

/// The anonymous `ExecutorService` of `createExecutorServiceThatFailsOneTask()`.
struct FailsOneTask {
    /// `private int taskCount = 0;`.
    task_count: Mutex<i32>,
    /// `private final ExecutorService delegate`.
    delegate: Arc<ThreadPoolExecutor>,
}

impl Executor for FailsOneTask {
    fn submit(&self, task: Callable) -> std::result::Result<Future, RejectedExecutionException> {
        let mut task_count = self.task_count.lock().unwrap_or_else(|e| e.into_inner());
        *task_count += 1;
        if *task_count == 2 {
            // Second task fails
            Ok(Future::failed(LuceneError::runtime_exception(
                LuceneError::Io(std::io::Error::other("Simulated failure on segment")),
            )))
        } else {
            drop(task_count);
            // Other tasks succeed normally
            self.delegate.submit(task)
        }
    }

    fn execute(&self, command: Command) -> std::result::Result<(), RejectedExecutionException> {
        self.delegate.execute(command)
    }
}

impl ExecutorService for FailsOneTask {
    fn shutdown(&self) {
        self.delegate.shutdown();
    }

    fn await_termination(
        &self,
        timeout: Duration,
    ) -> std::result::Result<bool, rucene::util::InterruptedException> {
        self.delegate.await_termination(timeout)
    }
}

/// `private ExecutorService createExecutorServiceThatFailsOneTask()`.
fn create_executor_service_that_fails_one_task() -> Arc<FailsOneTask> {
    Arc::new(FailsOneTask {
        task_count: Mutex::new(0),
        delegate: new_fixed_thread_pool(2),
    })
}

/// `private void createMultiSegmentIndex(Directory dir, int numSegments)`.
fn create_multi_segment_index(dir: &Arc<dyn Directory>, num_segments: i32) -> Result<()> {
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_max_buffered_docs(2)?;
    iwc.set_merge_policy(Some(NoMergePolicy::instance()))?;
    let writer = IndexWriter::new(Arc::clone(dir), iwc)?;
    for i in 0..num_segments {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &i.to_string(),
            Store::YES,
        )));
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "content",
            &format!("segment {i} content"),
            Store::YES,
        )));
        writer.add_document(&doc)?;
        writer.commit()?; // Force new segment
    }
    writer.close()
}
