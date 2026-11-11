//! Port of `lucene/core/src/test/org/apache/lucene/index/TestParallelReaderEmptyIndex.java`.
//!
//! Some tests for `ParallelLeafReader`s with empty indexes.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, FieldType, Store, TextField};
use rucene::error::Result;
use rucene::index::codec_reader::CodecReader;
use rucene::index::directory_reader;
use rucene::index::filter_directory_reader::SlowCodecReaderWrapper;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::parallel_reader::{ParallelCompositeReader, ParallelLeafReader};
use rucene::index::terms::Term;
use rucene::index::{CompositeReader, IndexReader, LeafReader};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test, verbose};
use crate::support::field_mut;

/// `TestParallelReaderEmptyIndex`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestParallelReaderEmptyIndex",
        vec![
            test("testEmptyIndex", test_empty_index),
            test("testEmptyIndexWithVectors", test_empty_index_with_vectors),
        ],
    )
}

/// Creates two empty indexes and wraps a ParallelReader around. Adding this reader to a new index
/// should not throw any exception.
///
/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn new_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// Port of `TestParallelReaderEmptyIndex.testEmptyIndex`: creates two empty
/// indexes and wraps a ParallelReader around. Adding this reader to a new
/// index should not throw any exception.
fn test_empty_index() -> Result<()> {
    let rd1 = LuceneTestCase::new_directory();
    let iw = IndexWriter::new(rd1.as_directory(), new_iwc())?;
    iw.close()?;
    // create a copy:
    let rd2 = LuceneTestCase::new_directory_copy(&*rd1.as_directory())?;

    let rd_out = LuceneTestCase::new_directory();

    let iw_out = IndexWriter::new(rd_out.as_directory(), new_iwc())?;

    // add a readerless parallel reader
    iw_out.add_indexes_codec_readers(&[SlowCodecReaderWrapper::wrap(Arc::new(
        ParallelLeafReader::new(vec![])?,
    ))?])?;
    iw_out.force_merge(1)?;

    let cpr = Arc::new(ParallelCompositeReader::new(vec![
        directory_reader::open(rd1.as_directory())? as Arc<dyn CompositeReader>,
        directory_reader::open(rd2.as_directory())? as Arc<dyn CompositeReader>,
    ])?);

    // When unpatched, Lucene crashes here with a NoSuchElementException (caused by
    // ParallelTermEnum)
    let mut leaves: Vec<Arc<dyn CodecReader>> = Vec::new();
    for leaf in Arc::clone(&cpr).leaves()? {
        leaves.push(SlowCodecReaderWrapper::wrap(leaf.leaf_reader())?);
    }
    iw_out.add_indexes_codec_readers(&leaves)?;
    iw_out.force_merge(1)?;

    iw_out.close()?;
    rd_out.close()?;
    rd1.close()?;
    rd2.close()
}

/// Port of `TestParallelReaderEmptyIndex.testEmptyIndexWithVectors`: this
/// method creates an empty index (numFields=0, numDocs=0) but is marked to
/// have TermVectors. Adding this index to another index should not throw any
/// exception.
fn test_empty_index_with_vectors() -> Result<()> {
    let rd1 = LuceneTestCase::new_directory();
    {
        if verbose() {
            println!("\nTEST: make 1st writer");
        }
        let iw = IndexWriter::new(rd1.as_directory(), new_iwc())?;
        let mut doc = Document::new();
        let id_field = LuceneTestCase::new_text_field("id", "", Store::NO);
        doc.add(Box::new(id_field));
        let custom_type = FieldType::new_from(TextField::type_not_stored());
        custom_type.set_store_term_vectors(true)?;
        doc.add(Box::new(LuceneTestCase::new_field(
            "test",
            "",
            &custom_type,
        )));
        field_mut(&mut doc, "id").set_string_value("1".to_string())?;
        iw.add_document(&doc)?;
        doc.add(Box::new(LuceneTestCase::new_field(
            "test",
            "",
            &custom_type,
        )));
        field_mut(&mut doc, "id").set_string_value("2".to_string())?;
        iw.add_document(&doc)?;
        iw.close()?;

        let dont_merge_config =
            IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
        dont_merge_config.set_merge_policy(Some(NoMergePolicy::instance()))?;
        if verbose() {
            println!("\nTEST: make 2nd writer");
        }
        let writer = IndexWriter::new(rd1.as_directory(), dont_merge_config)?;

        writer.delete_documents_by_terms(vec![Term::from_text("id", "1")])?;
        writer.close()?;
        let ir = directory_reader::open(rd1.as_directory())?;
        assert_equals(&2, &ir.max_doc());
        assert_equals(&1, &ir.num_docs());
        ir.close()?;

        let conf = new_iwc();
        conf.set_open_mode(OpenMode::APPEND)?;
        let iw = IndexWriter::new(rd1.as_directory(), conf)?;
        iw.force_merge(1)?;
        iw.close()?;
    }

    let rd2 = LuceneTestCase::new_directory();
    {
        let iw = IndexWriter::new(rd2.as_directory(), new_iwc())?;
        let doc = Document::new();
        iw.add_document(&doc)?;
        iw.close()?;
    }

    let rd_out = LuceneTestCase::new_directory();

    let iw_out = IndexWriter::new(rd_out.as_directory(), new_iwc())?;
    let reader1 = directory_reader::open(rd1.as_directory())?;
    let reader2 = directory_reader::open(rd2.as_directory())?;
    let pr: Arc<dyn LeafReader> = Arc::new(ParallelLeafReader::with_close_flag(
        false,
        vec![
            LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader1) as Arc<dyn IndexReader>)?,
            LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader2) as Arc<dyn IndexReader>)?,
        ],
    )?);

    // When unpatched, Lucene crashes here with an ArrayIndexOutOfBoundsException (caused by
    // TermVectorsWriter)
    iw_out.add_indexes_codec_readers(&[SlowCodecReaderWrapper::wrap(Arc::clone(&pr))?])?;

    Arc::clone(&pr).as_index_reader().close()?;
    reader1.close()?;
    reader2.close()?;

    // assert subreaders were closed
    assert_equals(&0, &reader1.get_ref_count());
    assert_equals(&0, &reader2.get_ref_count());

    rd1.close()?;
    rd2.close()?;

    iw_out.force_merge(1)?;
    iw_out.close()?;

    rd_out.close()
}
