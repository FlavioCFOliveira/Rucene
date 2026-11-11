//! Port of `lucene/core/src/test/org/apache/lucene/index/TestOmitNorms.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Field, FieldType, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_policy::{LogMergePolicy, MergePolicy};
use rucene::index::IndexReader;
use rucene::store::Directory;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test, IAE};
use crate::support::npe;
use rucene_test_framework::util::java_exceptions::expect_throws;

/// `TestOmitNorms`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestOmitNorms",
        vec![
            test("testMixedMergeThrowsError", test_mixed_merge_throws_error),
            test("testMixedRAM", test_mixed_ram),
            test("testNoNrmFile", test_no_nrm_file),
        ],
    )
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))` with `maxBufferedDocs`
/// and a merge policy.
///
/// The merge policy is created after the configuration, in Java's order.
fn config(
    max_buffered_docs: i32,
    merge_policy: impl FnOnce() -> Result<LogMergePolicy>,
) -> Result<rucene::index::index_writer_config::IndexWriterConfig> {
    let analyzer = Arc::new(MockAnalyzer::new(&*random()));
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    conf.set_max_buffered_docs(max_buffered_docs)?;
    conf.set_merge_policy(Some(Arc::new(merge_policy()?)))?;
    Ok(conf)
}

/// `new FieldType(TextField.TYPE_NOT_STORED)` with `omitNorms` and
/// `storeTermVectors` set.
fn text_type(omit_norms: bool, store_term_vectors: Option<bool>) -> Result<Arc<FieldType>> {
    let field_type = FieldType::new_from(TextField::type_not_stored());
    field_type.set_omit_norms(omit_norms)?;
    if let Some(store_term_vectors) = store_term_vectors {
        field_type.set_store_term_vectors(store_term_vectors)?;
    }
    Ok(Arc::new(field_type))
}

/// `reader.getFieldInfos().fieldInfo(name).omitsNorms()`.
fn omits_norms(reader: &Arc<dyn rucene::index::LeafReader>, name: &str) -> bool {
    reader
        .get_field_infos()
        .field_info(name)
        .unwrap_or_else(|| npe("fieldInfo"))
        .omits_norms()
}

/// Port of `TestOmitNorms.testMixedMergeThrowsError`.
fn test_mixed_merge_throws_error() -> Result<()> {
    let ram = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        ram.as_directory(),
        config(3, || LuceneTestCase::new_log_merge_policy_merge_factor(2))?,
    )?;
    let mut d = Document::new();

    // this field will have norms
    let field_type1 = text_type(false, Some(false))?;
    let f1 = Field::new(
        "f1",
        "This field has norms".into(),
        Arc::clone(&field_type1),
    )?;
    d.add(Box::new(f1));

    // this field will NOT have norms
    let field_type2 = text_type(true, Some(false))?;
    let f2 = Field::new(
        "f2",
        "This field has NO norms in all docs".into(),
        Arc::clone(&field_type2),
    )?;
    d.add(Box::new(f2));

    for _ in 0..30 {
        writer.add_document(&d)?;
    }

    // reverse omitNorms options for f1 and f2
    let mut d2 = Document::new();
    d2.add(Box::new(Field::new(
        "f1",
        "This field has NO norms".into(),
        Arc::clone(&field_type2),
    )?));
    d2.add(Box::new(Field::new(
        "f2",
        "This field has norms".into(),
        Arc::clone(&field_type1),
    )?));

    let exception = expect_throws(IAE, || writer.add_document(&d2));
    assert_equals(
        &Some("cannot change field \"f1\" from omitNorms=false to inconsistent omitNorms=true"),
        &exception.message(),
    );

    writer.force_merge(1)?;
    writer.close()?;

    let reader = LuceneTestCase::get_only_leaf_reader(
        directory_reader::open(ram.as_directory())? as Arc<dyn IndexReader>
    )?;
    // assert original omitNorms
    Assert::assert_true_msg(
        "OmitNorms field bit must not be set.",
        !omits_norms(&reader, "f1"),
    );
    Assert::assert_true_msg(
        "OmitNorms field bit must be set.",
        omits_norms(&reader, "f2"),
    );

    Arc::clone(&reader).as_index_reader().close()?;
    ram.close()
}

/// Port of `TestOmitNorms.testMixedRAM`.
fn test_mixed_ram() -> Result<()> {
    let ram = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        ram.as_directory(),
        config(10, || LuceneTestCase::new_log_merge_policy_merge_factor(2))?,
    )?;
    let mut d = Document::new();

    // this field will have norms
    let f1 = LuceneTestCase::new_text_field("f1", "This field has norms", Store::NO);
    d.add(Box::new(f1));

    // this field will NOT have norms

    let custom_type = text_type(true, None)?;
    let f2 = LuceneTestCase::new_field("f2", "This field has NO norms in all docs", &custom_type);
    d.add(Box::new(f2));

    for _ in 0..5 {
        writer.add_document(&d)?;
    }

    for _ in 0..20 {
        writer.add_document(&d)?;
    }

    // force merge
    writer.force_merge(1)?;

    // flush
    writer.close()?;

    let reader = LuceneTestCase::get_only_leaf_reader(
        directory_reader::open(ram.as_directory())? as Arc<dyn IndexReader>
    )?;
    Assert::assert_true_msg(
        "OmitNorms field bit should not be set.",
        !omits_norms(&reader, "f1"),
    );
    Assert::assert_true_msg(
        "OmitNorms field bit should be set.",
        omits_norms(&reader, "f2"),
    );

    Arc::clone(&reader).as_index_reader().close()?;
    ram.close()
}

/// `private void assertNoNrm(Directory dir)`.
fn assert_no_nrm(dir: &Arc<dyn Directory>) -> Result<()> {
    let files = dir.list_all()?;
    for file in &files {
        // TODO: this relies upon filenames
        Assert::assert_false(file.ends_with(".nrm") || file.ends_with(".len"));
    }
    Ok(())
}

/// Port of `TestOmitNorms.testNoNrmFile`.
fn test_no_nrm_file() -> Result<()> {
    let ram = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        ram.as_directory(),
        config(3, || Ok(LuceneTestCase::new_log_merge_policy()))?,
    )?;
    let mp = writer.get_config().merge_policy();
    let lmp = mp
        .as_any()
        .and_then(|a| a.downcast_ref::<LogMergePolicy>())
        .unwrap_or_else(|| {
            randomizedtesting::jdk::exception::Throwable::new(
                "java.lang.ClassCastException",
                "not a LogMergePolicy",
            )
            .throw()
        });
    lmp.set_merge_factor(2)?;
    lmp.set_no_cfs_ratio(0.0)?;
    let mut d = Document::new();

    let custom_type = text_type(true, None)?;
    let f1 = LuceneTestCase::new_field("f1", "This field has no norms", &custom_type);
    d.add(Box::new(f1));

    for _ in 0..30 {
        writer.add_document(&d)?;
    }

    writer.commit()?;

    assert_no_nrm(&ram.as_directory())?;

    // force merge
    writer.force_merge(1)?;
    // flush
    writer.close()?;

    assert_no_nrm(&ram.as_directory())?;
    ram.close()
}
