//! Port of `lucene/core/src/test/org/apache/lucene/index/TestSegmentReader.java`.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::field_infos::FieldInfos;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::segment_reader::SegmentReader;
use rucene::index::{IndexOptions, IndexReader, LeafReader};
use rucene::search::NO_MORE_DOCS;
use rucene::store::DEFAULT_IO_CONTEXT;
use rucene::util::{BytesRef, Version};
use rucene_test_framework::index::doc_helper::{self, DocHelper, NameValue};
use rucene_test_framework::util::java_exceptions::{expect_throws, rethrow_java};
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_equals_msg, assert_not_null, assert_null, random};

/// The instance fields of `TestSegmentReader`.
#[derive(Default)]
struct TestSegmentReaderInstance {
    /// `private Directory dir`.
    dir: Option<TestDirectory>,
    /// `private Document testDoc = new Document()`.
    test_doc: Document,
    /// `private SegmentReader reader = null`.
    reader: Option<Arc<SegmentReader>>,
}

impl TestSegmentReaderInstance {
    fn reader(&self) -> &Arc<SegmentReader> {
        self.reader
            .as_ref()
            .unwrap_or_else(|| crate::support::npe("reader"))
    }

    /// `reader`, as a `LeafReader`.
    fn leaf(&self) -> Arc<dyn LeafReader> {
        Arc::clone(self.reader()) as Arc<dyn LeafReader>
    }

    /// `reader`, as the `IndexReader` the static helpers take.
    fn index_reader(&self) -> Arc<dyn IndexReader> {
        Arc::clone(self.reader()) as Arc<dyn IndexReader>
    }
}

/// A test method over the instance.
fn instance_test(name: &str, body: fn(&mut TestSegmentReaderInstance) -> Result<()>) -> Method {
    Method::new(name).throws(&["java.lang.Exception"]).body(
        move |inst: &mut TestSegmentReaderInstance| body(inst).unwrap_or_else(|e| rethrow_java(e)),
    )
}

/// `TestSegmentReader`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestSegmentReader")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(TestSegmentReaderInstance::default))
        .method(instance_test("setUp", set_up))
        .method(instance_test("tearDown", tear_down))
        .method(instance_test("test", test))
        .method(instance_test("testDocument", test_document))
        .method(instance_test(
            "testGetFieldNameVariations",
            test_get_field_name_variations,
        ))
        .method(instance_test("testTerms", test_terms))
        .method(instance_test("testNorms", test_norms))
        .method(instance_test("testTermVectors", test_term_vectors))
        .method(instance_test(
            "testOutOfBoundsAccess",
            test_out_of_bounds_access,
        ))
        .build()
}

/// TODO: Setup the reader w/ multiple documents.
///
/// `public void setUp()`.
fn set_up(inst: &mut TestSegmentReaderInstance) -> Result<()> {
    LuceneTestCase::super_set_up();
    let dir = LuceneTestCase::new_directory();
    DocHelper::setup_doc(&mut inst.test_doc);
    let info = DocHelper::write_doc(&*random(), dir.as_directory(), &inst.test_doc)?
        .unwrap_or_else(|| crate::support::npe("writeDoc"));
    inst.reader = Some(Arc::new(SegmentReader::new(
        info,
        i32::from(Version::LATEST.major),
        &*DEFAULT_IO_CONTEXT,
    )?));
    inst.dir = Some(dir);
    Ok(())
}

/// `public void tearDown()`.
fn tear_down(inst: &mut TestSegmentReaderInstance) -> Result<()> {
    if let Some(reader) = inst.reader.take() {
        reader.close()?;
    }
    if let Some(mut dir) = inst.dir.take() {
        dir.close()?;
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}

/// Port of `TestSegmentReader.test`.
fn test(inst: &mut TestSegmentReaderInstance) -> Result<()> {
    Assert::assert_true(inst.dir.is_some());
    Assert::assert_true(inst.reader.is_some());
    Assert::assert_true(!DocHelper::name_values().is_empty());
    Assert::assert_true(DocHelper::num_fields(&inst.test_doc) == DocHelper::all().len());
    Ok(())
}

/// Port of `TestSegmentReader.testDocument`.
fn test_document(inst: &mut TestSegmentReaderInstance) -> Result<()> {
    let reader = inst.leaf();
    Assert::assert_true(reader.num_docs() == 1);
    Assert::assert_true(reader.max_doc() >= 1);
    let result = reader.stored_fields()?.document(0)?;
    // There are 2 unstored fields on the document that are not preserved across writing
    Assert::assert_true(
        DocHelper::num_fields(&result)
            == DocHelper::num_fields(&inst.test_doc) - DocHelper::unstored().len(),
    );

    let name_values = DocHelper::name_values();
    for field in result.get_fields() {
        Assert::assert_true(name_values.contains_key(field.name()));
    }
    Ok(())
}

/// Port of `TestSegmentReader.testGetFieldNameVariations`.
fn test_get_field_name_variations(inst: &mut TestSegmentReaderInstance) -> Result<()> {
    let mut all_field_names: HashSet<String> = HashSet::new();
    let mut indexed_field_names: HashSet<String> = HashSet::new();
    let mut not_indexed_field_names: HashSet<String> = HashSet::new();
    let mut tv_field_names: HashSet<String> = HashSet::new();
    let mut no_tv_field_names: HashSet<String> = HashSet::new();

    for field_info in inst.reader().get_field_infos().iter() {
        let name = field_info.name.clone();
        all_field_names.insert(name.clone());
        if field_info.get_index_options() != IndexOptions::NONE {
            indexed_field_names.insert(name.clone());
        } else {
            not_indexed_field_names.insert(name.clone());
        }
        if field_info.has_term_vectors() {
            tv_field_names.insert(name);
        } else if field_info.get_index_options() != IndexOptions::NONE {
            no_tv_field_names.insert(name);
        }
    }

    let name_values = DocHelper::name_values();
    Assert::assert_true(all_field_names.len() == DocHelper::all().len());
    for s in &all_field_names {
        Assert::assert_true(name_values.contains_key(s) || s.is_empty());
    }

    let indexed = DocHelper::indexed();
    Assert::assert_true(indexed_field_names.len() == indexed.len());
    for s in &indexed_field_names {
        Assert::assert_true(indexed.contains_key(s) || s.is_empty());
    }

    Assert::assert_true(not_indexed_field_names.len() == DocHelper::unindexed().len());
    // Get all indexed fields that are storing term vectors
    Assert::assert_true(tv_field_names.len() == DocHelper::termvector().len());

    Assert::assert_true(no_tv_field_names.len() == DocHelper::notermvector().len());
    Ok(())
}

/// `(String) DocHelper.nameValues.get(field)`.
fn name_value_string(field: &str) -> String {
    match DocHelper::name_values().get(field) {
        Some(NameValue::Text(s)) => s.clone(),
        Some(NameValue::Bytes(_)) => Throwable::new(
            "java.lang.ClassCastException",
            "class [B cannot be cast to class java.lang.String",
        )
        .throw(),
        None => crate::support::npe("nameValues.get(field)"),
    }
}

/// Port of `TestSegmentReader.testTerms`.
fn test_terms(inst: &mut TestSegmentReaderInstance) -> Result<()> {
    let reader = inst.index_reader();
    let fields = FieldInfos::get_indexed_fields(Arc::clone(&reader))?;
    for field in &fields {
        let terms = MultiTerms::get_terms(&reader, field)?;
        assert_not_null(&terms);
        let mut terms_enum = terms.expect("checked by assertNotNull").iterator()?;
        while let Some(term) = terms_enum.next()? {
            let field_value = name_value_string(field);
            Assert::assert_true(field_value.contains(term.utf8_to_string()?.as_str()));
        }
    }

    let mut term_docs = TestUtil::docs(
        &*random(),
        &reader,
        doc_helper::TEXT_FIELD_1_KEY,
        &BytesRef::new(b"field".to_vec()),
        None,
        0,
    )?
    .unwrap_or_else(|| crate::support::npe("termDocs"));
    Assert::assert_true(term_docs.next_doc()? != NO_MORE_DOCS);

    let mut term_docs = TestUtil::docs(
        &*random(),
        &reader,
        doc_helper::NO_NORMS_KEY,
        &BytesRef::new(doc_helper::NO_NORMS_TEXT.as_bytes().to_vec()),
        None,
        0,
    )?
    .unwrap_or_else(|| crate::support::npe("termDocs"));

    Assert::assert_true(term_docs.next_doc()? != NO_MORE_DOCS);

    let mut positions = MultiTerms::get_term_postings_enum_all(
        &reader,
        doc_helper::TEXT_FIELD_1_KEY,
        &BytesRef::new(b"field".to_vec()),
    )?
    .unwrap_or_else(|| crate::support::npe("positions"));
    // NOTE: prior rev of this test was failing to first
    // call next here:
    Assert::assert_true(positions.next_doc()? != NO_MORE_DOCS);
    Assert::assert_true(positions.doc_id() == 0);
    Assert::assert_true(positions.next_position()? >= 0);
    Ok(())
}

/// Port of `TestSegmentReader.testNorms`.
fn test_norms(inst: &mut TestSegmentReaderInstance) -> Result<()> {
    // TODO: Not sure how these work/should be tested
    /*
        try {
          byte [] norms = reader.norms(DocHelper.TEXT_FIELD_1_KEY);
          System.out.println("Norms: " + norms);
          assertTrue(norms != null);
        } catch (IOException e) {
          e.printStackTrace();
          assertTrue(false);
        }
    */

    check_norms(&(Arc::clone(inst.reader()) as Arc<dyn LeafReader>))
}

/// `public static void checkNorms(LeafReader reader)`.
pub fn check_norms(reader: &Arc<dyn LeafReader>) -> Result<()> {
    // test omit norms
    let no_norms = DocHelper::no_norms();
    for f in DocHelper::fields() {
        if f.field_type().index_options() != IndexOptions::NONE {
            assert_equals(
                &reader.get_norm_values(f.name())?.is_some(),
                &!f.field_type().omit_norms(),
            );
            assert_equals(
                &reader.get_norm_values(f.name())?.is_some(),
                &!no_norms.contains_key(f.name()),
            );
            if reader.get_norm_values(f.name())?.is_none() {
                // test for norms of null
                let norms = MultiDocValues::get_norm_values(
                    &(Arc::clone(reader).as_index_reader()),
                    f.name(),
                )?;
                assert_null(&norms.map(|_| "NumericDocValues"));
            }
        }
    }
    Ok(())
}

/// Port of `TestSegmentReader.testTermVectors`.
fn test_term_vectors(inst: &mut TestSegmentReaderInstance) -> Result<()> {
    let reader = inst.leaf();
    let result = reader
        .term_vectors()?
        .get(0)?
        .unwrap_or_else(|| crate::support::npe("termVectors().get(0)"))
        .terms(doc_helper::TEXT_FIELD_2_KEY)?;
    assert_not_null(&result);
    let result = result.expect("checked by assertNotNull");
    assert_equals(&3i64, &result.size()?);
    let mut terms_enum = result.iterator()?;
    while let Some(term) = terms_enum.next()? {
        let term = term.utf8_to_string()?;
        let freq = terms_enum.total_term_freq()? as i32;
        Assert::assert_true(doc_helper::FIELD_2_TEXT.contains(term.as_str()));
        Assert::assert_true(freq > 0);
    }

    let results = reader.term_vectors()?.get(0)?;
    Assert::assert_true(results.is_some());
    assert_equals_msg(
        Some("We do not have 3 term freq vectors"),
        &3,
        &results.expect("checked").size(),
    );
    Ok(())
}

/// Port of `TestSegmentReader.testOutOfBoundsAccess`.
fn test_out_of_bounds_access(inst: &mut TestSegmentReaderInstance) -> Result<()> {
    let reader = inst.leaf();
    let num_docs = reader.max_doc();

    const IOOBE: &str = "java.lang.IndexOutOfBoundsException";
    expect_throws(IOOBE, || reader.stored_fields()?.document(-1));

    expect_throws(IOOBE, || reader.term_vectors()?.get(-1));

    expect_throws(IOOBE, || reader.stored_fields()?.document(num_docs));

    expect_throws(IOOBE, || reader.term_vectors()?.get(num_docs));
    Ok(())
}
