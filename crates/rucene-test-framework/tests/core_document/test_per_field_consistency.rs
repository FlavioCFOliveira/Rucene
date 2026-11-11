//! Port of `lucene/core/src/test/org/apache/lucene/document/TestPerFieldConsistency.java`.

use std::sync::Arc;

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use randomizedtesting::RandomizedTest;
use rucene::document::{
    BinaryDocValuesField, Document, DoubleDocValuesField, DoublePoint, Field, FieldType,
    FloatPoint, IntPoint, KnnFloatVectorField, LongPoint, NumericDocValuesField,
    SortedSetDocValuesField,
};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{
    IndexOptions, IndexReader, IndexableField, IndexableFieldType, VectorSimilarityFunction,
};
use rucene::util::BytesRef;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test, IAE};

/// `TestPerFieldConsistency`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestPerFieldConsistency",
        vec![
            test(
                "testDocWithMissingSchemaOptionsThrowsError",
                test_doc_with_missing_schema_options_throws_error,
            ),
            test(
                "testDocWithExtraSchemaOptionsThrowsError",
                test_doc_with_extra_schema_options_throws_error,
            ),
        ],
    )
}

/// A random field of the test: Java builds each once and adds the same
/// instance to several documents ([`Document::add_shared`]).
type SharedField = Arc<dyn IndexableField>;

const INDEX_OPTIONS: [IndexOptions; 5] = [
    IndexOptions::NONE,
    IndexOptions::DOCS,
    IndexOptions::DOCS_AND_FREQS,
    IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
    IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
];

const SIMILARITIES: [VectorSimilarityFunction; 4] = [
    VectorSimilarityFunction::EUCLIDEAN,
    VectorSimilarityFunction::DOT_PRODUCT,
    VectorSimilarityFunction::COSINE,
    VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
];

/// `randomIndexedField(random, fieldName)`.
fn random_indexed_field(random: &dyn Random, field_name: &str) -> Result<SharedField> {
    let field_type = FieldType::new();
    let mut index_options = *RandomPicks::random_from(random, &INDEX_OPTIONS);
    while index_options == IndexOptions::NONE {
        index_options = *RandomPicks::random_from(random, &INDEX_OPTIONS);
    }
    field_type.set_index_options(index_options)?;
    field_type.set_store_term_vectors(random.next_boolean())?;
    if field_type.store_term_vectors() {
        field_type.set_store_term_vector_positions(random.next_boolean())?;
        if field_type.store_term_vector_positions() {
            field_type.set_store_term_vector_payloads(random.next_boolean())?;
            field_type.set_store_term_vector_offsets(random.next_boolean())?;
        }
    }
    field_type.set_omit_norms(random.next_boolean())?;
    field_type.set_stored(random.next_boolean())?;
    field_type.freeze();
    Ok(Arc::new(Field::new(
        field_name,
        "randomValue".to_string(),
        field_type,
    )?))
}

/// `randomPointField(random, fieldName)`.
fn random_point_field(random: &dyn Random, field_name: &str) -> Result<SharedField> {
    Ok(match random.next_int_bound(4) {
        0 => Arc::new(LongPoint::new(
            field_name,
            &[RandomizedTest::random_long()],
        )?),
        1 => Arc::new(IntPoint::new(field_name, &[RandomizedTest::random_int()])?),
        2 => Arc::new(DoublePoint::new(
            field_name,
            &[RandomizedTest::random_double()],
        )?),
        _ => Arc::new(FloatPoint::new(
            field_name,
            &[RandomizedTest::random_float()],
        )?),
    })
}

/// `randomDocValuesField(random, fieldName)`.
fn random_doc_values_field(random: &dyn Random, field_name: &str) -> SharedField {
    match random.next_int_bound(4) {
        0 => Arc::new(BinaryDocValuesField::new(
            field_name,
            BytesRef::from_char_sequence("randomValue"),
        )),
        1 => Arc::new(NumericDocValuesField::new(
            field_name,
            RandomizedTest::random_long(),
        )),
        2 => Arc::new(DoubleDocValuesField::new(
            field_name,
            RandomizedTest::random_double(),
        )),
        _ => Arc::new(SortedSetDocValuesField::new(
            field_name,
            BytesRef::from_char_sequence("randomValue"),
        )),
    }
}

/// `randomKnnVectorField(random, fieldName)`.
fn random_knn_vector_field(random: &dyn Random, field_name: &str) -> Result<SharedField> {
    let similarity_function = *RandomPicks::random_from(random, &SIMILARITIES);
    let mut values = vec![0f32; RandomizedTest::random_int_between(1, 10) as usize];
    for v in values.iter_mut() {
        *v = RandomizedTest::random_float();
    }
    Ok(Arc::new(KnnFloatVectorField::new(
        field_name,
        &values,
        similarity_function,
    )?))
}

/// `randomFieldsWithTheSameName(fieldName)`.
fn random_fields_with_the_same_name(field_name: &str) -> Result<Vec<SharedField>> {
    let text_field = random_indexed_field(&*random(), field_name)?;
    let doc_values_field = random_doc_values_field(&*random(), field_name);
    let point_field = random_point_field(&*random(), field_name)?;
    let vector_field = random_knn_vector_field(&*random(), field_name)?;
    Ok(vec![
        text_field,
        doc_values_field,
        point_field,
        vector_field,
    ])
}

/// `assertTrue("'" + errorMsg + "' not found in '" + message + "'", message.contains(errorMsg))`.
fn assert_message_contains(e: randomizedtesting::jdk::exception::Throwable, error_msg: &str) {
    let message = e.message().unwrap_or_default().to_string();
    Assert::assert_true_msg(
        &format!("'{error_msg}' not found in '{message}'"),
        message.contains(error_msg),
    );
}

/// `doTestDocWithMissingSchemaOptionsThrowsError(fields, missing, writer, errorMsg)`.
fn do_test_doc_with_missing_schema_options_throws_error(
    fields: &[SharedField],
    missing: usize,
    writer: &IndexWriter,
    error_msg: &str,
) -> Result<()> {
    let mut doc = Document::new();
    for (i, f) in fields.iter().enumerate() {
        if i != missing {
            doc.add_shared(Arc::clone(f));
        }
    }
    let exception = expect_throws(IAE, || writer.add_document(&doc));
    assert_message_contains(exception, error_msg);
    Ok(())
}

/// `doTestDocWithExtraSchemaOptionsThrowsError(existing, extra, writer, errorMsg)`.
fn do_test_doc_with_extra_schema_options_throws_error(
    existing: &SharedField,
    extra: &SharedField,
    writer: &IndexWriter,
    error_msg: &str,
) -> Result<()> {
    let mut doc = Document::new();
    doc.add_shared(Arc::clone(existing));
    doc.add_shared(Arc::clone(extra));
    let exception = expect_throws(IAE, || writer.add_document(&doc));
    assert_message_contains(exception, error_msg);
    Ok(())
}

/// `try (Directory dir = newDirectory(); IndexWriter writer = new IndexWriter(dir,
/// new IndexWriterConfig().setMergePolicy(NoMergePolicy.INSTANCE))) { body }`.
fn with_writer(body: impl FnOnce(&Arc<IndexWriter>) -> Result<()>) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let config = IndexWriterConfig::new();
    config.set_merge_policy(Some(NoMergePolicy::instance()))?;
    let writer = IndexWriter::new(dir.as_directory(), config)?;
    let outcome = body(&writer);
    let closed = writer.close();
    let dir_closed = dir.close();
    outcome?;
    closed?;
    dir_closed
}

/// `try (IndexReader reader = DirectoryReader.open(writer))`: checks the
/// leaf count, and the live and deleted docs of leaf `leaf`.
fn check_reader(
    writer: &IndexWriter,
    leaves: usize,
    leaf: usize,
    num_not_indexed_docs: i32,
) -> Result<()> {
    let reader: Arc<dyn IndexReader> = rucene::index::directory_reader::open_with_writer(writer)?;
    let checked = (|| -> Result<()> {
        let ls = Arc::clone(&reader).leaves()?;
        assert_equals(&leaves, &ls.len());
        let leaf_reader = ls[leaf].leaf_reader();
        assert_equals(&1, &leaf_reader.num_docs());
        assert_equals(
            &num_not_indexed_docs,
            &leaf_reader.as_index_reader().num_deleted_docs(),
        );
        Ok(())
    })();
    let closed = reader.close();
    checked?;
    closed
}

/// Port of `testDocWithMissingSchemaOptionsThrowsError`.
fn test_doc_with_missing_schema_options_throws_error() -> Result<()> {
    with_writer(|writer| {
        let fields = random_fields_with_the_same_name("myfield")?;
        let mut doc0 = Document::new();
        for field in &fields {
            doc0.add_shared(Arc::clone(field));
        }
        writer.add_document(&doc0)?;

        // the same segment: indexing a doc with a missing field throws error
        let mut num_not_indexed_docs = 0;
        for missing_field_idx in 0..fields.len() {
            num_not_indexed_docs += 1;
            do_test_doc_with_missing_schema_options_throws_error(
                &fields,
                missing_field_idx,
                writer,
                &format!(
                    "Inconsistency of field data structures across documents for field [myfield] of doc [{num_not_indexed_docs}]."
                ),
            )?;
        }
        writer.flush()?;
        check_reader(writer, 1, 0, num_not_indexed_docs)?;

        // diff segment, same index: indexing a doc with a missing field throws error
        num_not_indexed_docs = 0;
        for missing_field_idx in 0..fields.len() {
            num_not_indexed_docs += 1;
            do_test_doc_with_missing_schema_options_throws_error(
                &fields,
                missing_field_idx,
                writer,
                "cannot change field \"myfield\" from ",
            )?;
        }
        writer.add_document(&doc0)?; // add document with correct data structures
        writer.flush()?;
        check_reader(writer, 2, 1, num_not_indexed_docs)
    })
}

/// Port of `testDocWithExtraSchemaOptionsThrowsError`.
fn test_doc_with_extra_schema_options_throws_error() -> Result<()> {
    with_writer(|writer| {
        let fields = random_fields_with_the_same_name("myfield")?;
        let mut doc0 = Document::new();
        let existing_field_idx =
            RandomizedTest::random_int_between(0, fields.len() as i32 - 1) as usize;
        doc0.add_shared(Arc::clone(&fields[existing_field_idx]));
        writer.add_document(&doc0)?;

        // the same segment: indexing a field with extra field indexing options returns error
        let mut num_not_indexed_docs = 0;
        for extra_field_index in 0..fields.len() {
            if extra_field_index == existing_field_idx {
                continue;
            }
            num_not_indexed_docs += 1;
            do_test_doc_with_extra_schema_options_throws_error(
                &fields[existing_field_idx],
                &fields[extra_field_index],
                writer,
                &format!(
                    "Inconsistency of field data structures across documents for field [myfield] of doc [{num_not_indexed_docs}]."
                ),
            )?;
        }
        writer.flush()?;
        check_reader(writer, 1, 0, num_not_indexed_docs)?;

        // diff segment, same index: indexing a field with extra field indexing options returns error
        num_not_indexed_docs = 0;
        for extra_field_index in 0..fields.len() {
            if extra_field_index == existing_field_idx {
                continue;
            }
            num_not_indexed_docs += 1;
            do_test_doc_with_extra_schema_options_throws_error(
                &fields[existing_field_idx],
                &fields[extra_field_index],
                writer,
                "cannot change field \"myfield\" from ",
            )?;
        }
        writer.add_document(&doc0)?; // add document with correct data structures
        writer.flush()?;
        check_reader(writer, 2, 1, num_not_indexed_docs)
    })
}
