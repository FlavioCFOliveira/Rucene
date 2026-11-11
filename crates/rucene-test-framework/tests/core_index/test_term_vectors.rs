//! Port of `lucene/core/src/test/org/apache/lucene/index/TestTermVectors.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use randomizedtesting::RandomizedTest;
use rucene::document::{Document, Field, FieldType, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::IndexReader;
use rucene::store::Directory;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::base_term_vectors_format_test_case::{
    RandomTokenStream, RandomTokenStreamData,
};
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_not_equals, class, random, test};
use crate::support::{assert_not_null_msg, npe};

/// `TestTermVectors`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestTermVectors",
        vec![
            test("testFullMergeAddDocs", test_full_merge_add_docs),
            test(
                "testFullMergeAddIndexesDir",
                test_full_merge_add_indexes_dir,
            ),
            test(
                "testFullMergeAddIndexesReader",
                test_full_merge_add_indexes_reader,
            ),
            test("testMergeWithPayloads", test_merge_with_payloads),
        ],
    )
}

/// `private IndexWriter createWriter(Directory dir)`.
fn create_writer(dir: &Arc<dyn Directory>) -> Result<Arc<IndexWriter>> {
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    iwc.set_max_buffered_docs(2)?;
    IndexWriter::new(Arc::clone(dir), iwc)
}

/// `private void createDir(Directory dir)`.
fn create_dir(dir: &Arc<dyn Directory>) -> Result<()> {
    let writer = create_writer(dir)?;
    writer.add_document(&create_doc()?)?;
    writer.close()
}

/// `private Document createDoc()`.
fn create_doc() -> Result<Document> {
    let mut doc = Document::new();
    let ft = FieldType::new_from(TextField::type_stored());
    ft.set_store_term_vectors(true)?;
    ft.set_store_term_vector_offsets(true)?;
    ft.set_store_term_vector_positions(true)?;
    doc.add(Box::new(LuceneTestCase::new_field("c", "aaa", &ft)));
    Ok(doc)
}

/// `private void verifyIndex(Directory dir)`.
fn verify_index(dir: &Arc<dyn Directory>) -> Result<()> {
    let r = directory_reader::open(Arc::clone(dir))?;
    let term_vectors = r.term_vectors()?;
    let num_docs = r.num_docs();
    for i in 0..num_docs {
        assert_not_null_msg(
            &format!("term vectors should not have been null for document {i}"),
            &term_vectors
                .get(i)?
                .unwrap_or_else(|| npe("termVectors.get(i)"))
                .terms("c")?,
        );
    }
    r.close()
}

/// Port of `TestTermVectors.testFullMergeAddDocs`.
fn test_full_merge_add_docs() -> Result<()> {
    let target = LuceneTestCase::new_directory();
    let writer = create_writer(&target.as_directory())?;
    // with maxBufferedDocs=2, this results in two segments, so that forceMerge
    // actually does something.
    for _ in 0..4 {
        writer.add_document(&create_doc()?)?;
    }
    writer.force_merge(1)?;
    writer.close()?;

    verify_index(&target.as_directory())?;
    target.close()
}

/// `IOUtils.close(target, input[0], input[1])`.
fn close_all(target: &mut TestDirectory, input: &mut [TestDirectory]) -> Result<()> {
    let mut first: Option<rucene::error::LuceneError> = None;
    for d in std::iter::once(target).chain(input.iter_mut()) {
        if let Err(e) = d.close() {
            first.get_or_insert(e);
        }
    }
    match first {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Port of `TestTermVectors.testFullMergeAddIndexesDir`.
fn test_full_merge_add_indexes_dir() -> Result<()> {
    let mut input = [
        LuceneTestCase::new_directory(),
        LuceneTestCase::new_directory(),
    ];
    let mut target = LuceneTestCase::new_directory();

    for dir in &input {
        create_dir(&dir.as_directory())?;
    }

    let writer = create_writer(&target.as_directory())?;
    let dirs: Vec<Arc<dyn Directory>> = input.iter().map(|d| d.as_directory()).collect();
    writer.add_indexes_dirs(&dirs)?;
    writer.force_merge(1)?;
    writer.close()?;
    drop(dirs);

    verify_index(&target.as_directory())?;

    close_all(&mut target, &mut input)
}

/// Port of `TestTermVectors.testFullMergeAddIndexesReader`.
fn test_full_merge_add_indexes_reader() -> Result<()> {
    let mut input = [
        LuceneTestCase::new_directory(),
        LuceneTestCase::new_directory(),
    ];
    let mut target = LuceneTestCase::new_directory();

    for dir in &input {
        create_dir(&dir.as_directory())?;
    }

    let writer = create_writer(&target.as_directory())?;
    for dir in &input {
        let r = directory_reader::open(dir.as_directory())?;
        TestUtil::add_indexes_slowly(&writer, &[Arc::clone(&r)])?;
        r.close()?;
    }
    writer.force_merge(1)?;
    writer.close()?;

    verify_index(&target.as_directory())?;
    close_all(&mut target, &mut input)
}

/// Assert that a merged segment has payloads set up in fieldInfo, if at least 1 segment has
/// payloads for this field.
///
/// Port of `TestTermVectors.testMergeWithPayloads`.
fn test_merge_with_payloads() -> Result<()> {
    let ft1 = FieldType::new_from(TextField::type_not_stored());
    ft1.set_store_term_vectors(true)?;
    ft1.set_store_term_vector_offsets(true)?;
    ft1.set_store_term_vector_positions(true)?;
    ft1.set_store_term_vector_payloads(true)?;
    ft1.freeze();
    let ft1 = Arc::new(ft1);

    let num_docs_in_segment = 10;
    for has_payloads in [false, true] {
        let dir = LuceneTestCase::new_directory();
        let index_writer_config =
            IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
        index_writer_config.set_max_buffered_docs(num_docs_in_segment)?;
        let writer = IndexWriter::new(dir.as_directory(), index_writer_config)?;
        let tkg1 = TokenStreamGenerator::new(has_payloads);
        let tkg2 = TokenStreamGenerator::new(!has_payloads);

        // create one segment with payloads, and another without payloads
        for _ in 0..num_docs_in_segment {
            let mut doc = Document::new();
            doc.add(Box::new(Field::new_with_token_stream(
                "c",
                tkg1.new_token_stream(),
                Arc::clone(&ft1),
            )?));
            writer.add_document(&doc)?;
        }
        for _ in 0..num_docs_in_segment {
            let mut doc = Document::new();
            doc.add(Box::new(Field::new_with_token_stream(
                "c",
                tkg2.new_token_stream(),
                Arc::clone(&ft1),
            )?));
            writer.add_document(&doc)?;
        }

        let reader1: Arc<dyn DirectoryReader> = directory_reader::open_with_writer(&*writer)?;
        let leaves1 = (Arc::clone(&reader1) as Arc<dyn IndexReader>).leaves()?;
        assert_equals(&2, &leaves1.len());
        let has = |i: usize| -> bool {
            leaves1[i]
                .leaf_reader()
                .get_field_infos()
                .field_info("c")
                .unwrap_or_else(|| npe("fieldInfo(\"c\")"))
                .has_payloads()
        };
        assert_equals(&has_payloads, &has(0));
        assert_not_equals(&has_payloads, &has(1));

        writer.force_merge(1)?;
        let reader2: Arc<dyn DirectoryReader> = directory_reader::open_with_writer(&*writer)?;
        let leaves2 = (Arc::clone(&reader2) as Arc<dyn IndexReader>).leaves()?;
        assert_equals(&1, &leaves2.len());
        // assert that in the merged segments payloads set up for the field
        Assert::assert_true(
            leaves2[0]
                .leaf_reader()
                .get_field_infos()
                .field_info("c")
                .unwrap_or_else(|| npe("fieldInfo(\"c\")"))
                .has_payloads(),
        );

        // IOUtils.close(writer, reader1, reader2, dir): every one is closed,
        // the first failure is rethrown.
        drop((leaves1, leaves2));
        let outcomes = [
            writer.close(),
            reader1.close(),
            reader2.close(),
            dir.close(),
        ];
        for outcome in outcomes {
            outcome?;
        }
    }
    Ok(())
}

/// A generator for token streams with optional null payloads
struct TokenStreamGenerator {
    terms: Vec<String>,
    term_bytes: Vec<BytesRef>,
    has_payloads: bool,
}

impl TokenStreamGenerator {
    fn new(has_payloads: bool) -> Self {
        let terms_count = 10;
        let mut terms = Vec::with_capacity(terms_count);
        let mut term_bytes = Vec::with_capacity(terms_count);
        for _ in 0..terms_count {
            let t = TestUtil::random_realistic_unicode_string(&*random());
            term_bytes.push(BytesRef::from_char_sequence(&t));
            terms.push(t);
        }
        Self {
            terms,
            term_bytes,
            has_payloads,
        }
    }

    /// `public TokenStream newTokenStream()`: an `OptionalNullPayloadTokenStream`.
    fn new_token_stream(&self) -> Arc<AtomicRefCell<dyn rucene::analysis::TokenStream>> {
        let len = TestUtil::next_int(&*random(), 1, 5);
        let has_payloads = self.has_payloads;
        let data =
            RandomTokenStreamData::with_payloads(len, &self.terms, &self.term_bytes, &|| {
                random_payload(has_payloads)
            });
        Arc::new(AtomicRefCell::new(RandomTokenStream::over(Arc::new(data))))
    }
}

/// `OptionalNullPayloadTokenStream.randomPayload()`.
fn random_payload(has_payloads: bool) -> Option<BytesRef> {
    if !has_payloads {
        return None;
    }
    let len = RandomizedTest::random_int_between(1, 5) as usize;
    let mut payload = BytesRef::new(vec![0u8; len]);
    random().next_bytes(&mut payload.bytes);
    payload.length = len;
    Some(payload)
}
