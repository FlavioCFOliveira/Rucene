//! Port of `lucene/core/src/test/org/apache/lucene/search/TestVectorSimilarityValuesSource.java`.
//!
//! Java's `dv.doubleValue() == SIM.compare(...)` widens the `float` the
//! similarity returns to `double`; the port compares `f64::from` of it. Java's
//! two-argument `assertNotEquals(double, float)` binds to
//! `assertNotEquals(Object, Object)` over a `Double` and a `Float`, which are
//! never equal; the port compares the boxed values as the
//! [`SortValue::Double`] and [`SortValue::Float`] they are.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::analysis::Analyzer;
use rucene::document::{
    Document, KnnByteVectorField, KnnFloatVectorField, SortedDocValuesField, Store, StringField,
};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::{LeafReaderContext, VectorSimilarityFunction};
use rucene::search::double_values::DoubleValues;
use rucene::search::double_values_source::{
    similarity_to_byte_query_vector, similarity_to_float_query_vector,
};
use rucene::search::field_comparator::SortValue;
use rucene::search::index_searcher::IndexSearcher;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::static_hook;
use crate::core_support::{assert_equals_delta, assert_not_equals, class, random, test};

use VectorSimilarityFunction::{COSINE, DOT_PRODUCT, EUCLIDEAN, MAXIMUM_INNER_PRODUCT};

/// `TestVectorSimilarityValuesSource`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestVectorSimilarityValuesSource",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test(
                "testEuclideanSimilarityValuesSource",
                test_euclidean_similarity_values_source,
            ),
            test(
                "testDotSimilarityValuesSource",
                test_dot_similarity_values_source,
            ),
            test(
                "testCosineSimilarityValuesSource",
                test_cosine_similarity_values_source,
            ),
            test(
                "testMaximumProductSimilarityValuesSource",
                test_maximum_product_similarity_values_source,
            ),
            test(
                "testFailuresWithSimilarityValuesSource",
                test_failures_with_similarity_values_source,
            ),
        ],
    )
}

/// The static fields `dir`, `analyzer`, `reader` and `searcher`.
struct Statics {
    dir: TestDirectory,
    analyzer: Arc<dyn Analyzer>,
    reader: Arc<dyn DirectoryReader>,
    searcher: Option<IndexSearcher>,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

/// `searcher.reader.leaves().get(0)`.
fn leaf0() -> Result<Arc<LeafReaderContext>> {
    let guard = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    let searcher = guard
        .as_ref()
        .and_then(|s| s.searcher.as_ref())
        .expect("beforeClass ran");
    Ok(Arc::clone(searcher.get_index_reader()).leaves()?[0].clone())
}

/// `new byte[] {...}`.
fn b(v: &[i8]) -> Vec<u8> {
    v.iter().map(|&x| x as u8).collect()
}

/// `new KnnFloatVectorField(field, vector, similarity)`.
fn kf(
    field: &str,
    vector: &[f32],
    similarity: VectorSimilarityFunction,
) -> Result<Box<KnnFloatVectorField>> {
    Ok(Box::new(KnnFloatVectorField::new(
        field, vector, similarity,
    )?))
}

/// `new KnnByteVectorField(field, vector, similarity)`.
fn kb(
    field: &str,
    vector: &[i8],
    similarity: VectorSimilarityFunction,
) -> Result<Box<KnnByteVectorField>> {
    Ok(Box::new(KnnByteVectorField::new(
        field,
        &b(vector),
        similarity,
    )?))
}

/// The `id` fields of each document.
fn id_fields(doc: &mut Document, id: &str) -> Result<()> {
    doc.add(Box::new(StringField::new("id", id.to_string(), Store::NO)?));
    doc.add(Box::new(SortedDocValuesField::new(
        "id",
        BytesRef::from_char_sequence(id),
    )));
    Ok(())
}

/// `@BeforeClass public static void beforeClass()`.
fn before_class() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let mut iw_config =
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&analyzer)));
    iw_config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iw_config)?;

    // `new KnnFloatVectorField(field, vector)` and `new KnnByteVectorField(field,
    // vector)` use EUCLIDEAN.
    let mut document = Document::new();
    id_fields(&mut document, "1")?;
    document.add(kf("knnFloatField1", &[1.0, 2.0, 3.0], EUCLIDEAN)?);
    document.add(kf("knnFloatField2", &[2.2, -3.2, -3.1], DOT_PRODUCT)?);
    document.add(kf("knnFloatField3", &[4.5, 10.3, -7.0], COSINE)?);
    document.add(kf(
        "knnFloatField4",
        &[-1.3, 1.0, 1.0],
        MAXIMUM_INNER_PRODUCT,
    )?);
    document.add(kf("knnFloatField5", &[-6.7, -1.0, -0.9], EUCLIDEAN)?);
    document.add(kb("knnByteField1", &[106, 80, 127], EUCLIDEAN)?);
    document.add(kb("knnByteField2", &[4, 2, 3], DOT_PRODUCT)?);
    document.add(kb("knnByteField3", &[-121, -64, -1], COSINE)?);
    document.add(kb(
        "knnByteField4",
        &[-127, 127, 127],
        MAXIMUM_INNER_PRODUCT,
    )?);
    iw.add_document(&document)?;

    let mut document2 = Document::new();
    id_fields(&mut document2, "2")?;
    document2.add(kf("knnFloatField1", &[1.0, 2.0, 3.0], EUCLIDEAN)?);
    document2.add(kf("knnFloatField2", &[-5.2, 8.7, 3.1], DOT_PRODUCT)?);
    document2.add(kf("knnFloatField3", &[0.2, -3.2, 3.1], COSINE)?);
    document2.add(kf("knnFloatField5", &[2.0, 13.2, 9.1], EUCLIDEAN)?);
    document2.add(kb("knnByteField1", &[1, -2, -30], EUCLIDEAN)?);
    document2.add(kb("knnByteField2", &[40, 21, 3], DOT_PRODUCT)?);
    document2.add(kb("knnByteField3", &[9, 2, 3], COSINE)?);
    document2.add(kb("knnByteField4", &[14, 29, 31], MAXIMUM_INNER_PRODUCT)?);
    iw.add_document(&document2)?;

    let mut document3 = Document::new();
    id_fields(&mut document3, "3")?;
    document3.add(kf("knnFloatField1", &[1.0, 2.0, 3.0], EUCLIDEAN)?);
    document3.add(kf("knnFloatField2", &[-8.0, 7.0, -6.0], DOT_PRODUCT)?);
    document3.add(kf("knnFloatField5", &[5.2, 3.2, 3.1], EUCLIDEAN)?);
    document3.add(kb("knnByteField1", &[-128, 0, 127], EUCLIDEAN)?);
    document3.add(kb("knnByteField2", &[-1, -2, -3], DOT_PRODUCT)?);
    document3.add(kb("knnByteField3", &[4, 2, 3], COSINE)?);
    document3.add(kb("knnByteField4", &[-4, -2, -128], MAXIMUM_INNER_PRODUCT)?);
    document3.add(kb("knnByteField5", &[-120, -2, 3], EUCLIDEAN)?);
    iw.add_document(&document3)?;
    iw.commit()?;
    iw.force_merge(1)?;

    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    iw.close()?;
    *STATICS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics {
        dir,
        analyzer,
        reader,
        searcher: Some(searcher),
    });
    Ok(())
}

/// `@AfterClass public static void afterClass()`.
fn after_class() -> Result<()> {
    let statics = STATICS.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(mut statics) = statics {
        statics.searcher = None;
        // `IOUtils.close(reader, dir, analyzer)`.
        let r1 = statics.reader.close();
        let r2 = statics.dir.close();
        drop(statics.analyzer);
        r1.and(r2)?;
    }
    Ok(())
}

/// `DoubleValuesSource.similarityToQueryVector(ctx, float[], field)`.
fn float_dv(query: &[f32], field: &str) -> Result<Box<dyn DoubleValues>> {
    let ctx = leaf0()?;
    similarity_to_float_query_vector(&ctx, query.to_vec(), field)
}

/// `DoubleValuesSource.similarityToQueryVector(ctx, byte[], field)`.
fn byte_dv(query: &[u8], field: &str) -> Result<Box<dyn DoubleValues>> {
    let ctx = leaf0()?;
    similarity_to_byte_query_vector(&ctx, query.to_vec(), field)
}

/// `dv.advanceExact(doc) && dv.doubleValue() == expected`.
fn matches(dv: &mut dyn DoubleValues, doc: i32, expected: f32) -> Result<bool> {
    Ok(dv.advance_exact(doc)? && dv.double_value()? == f64::from(expected))
}

/// Port of `TestVectorSimilarityValuesSource.testEuclideanSimilarityValuesSource`.
fn test_euclidean_similarity_values_source() -> Result<()> {
    let float_query_vector = [9.0f32, 1.0, -10.0];

    // Checks the computed similarity score between indexed vectors and query vector
    // using DVS is correct by passing indexed and query vector in #compare
    let mut dv = float_dv(&float_query_vector, "knnFloatField1")?;
    for doc in 0..3 {
        Assert::assert_true(matches(
            &mut *dv,
            doc,
            EUCLIDEAN.compare_f32(&[1.0, 2.0, 3.0], &float_query_vector)?,
        )?);
    }

    let mut dv = float_dv(&float_query_vector, "knnFloatField5")?;
    Assert::assert_true(matches(
        &mut *dv,
        0,
        EUCLIDEAN.compare_f32(&[-6.7, -1.0, -0.9], &float_query_vector)?,
    )?);
    Assert::assert_true(matches(
        &mut *dv,
        1,
        EUCLIDEAN.compare_f32(&[2.0, 13.2, 9.1], &float_query_vector)?,
    )?);
    Assert::assert_true(matches(
        &mut *dv,
        2,
        EUCLIDEAN.compare_f32(&[5.2, 3.2, 3.1], &float_query_vector)?,
    )?);

    let byte_query_vector = b(&[-128, 2, 127]);

    let mut dv = byte_dv(&byte_query_vector, "knnByteField1")?;
    Assert::assert_true(matches(
        &mut *dv,
        0,
        EUCLIDEAN.compare_bytes(&b(&[106, 80, 127]), &byte_query_vector)?,
    )?);
    Assert::assert_true(matches(
        &mut *dv,
        1,
        EUCLIDEAN.compare_bytes(&b(&[1, -2, -30]), &byte_query_vector)?,
    )?);
    Assert::assert_true(matches(
        &mut *dv,
        2,
        EUCLIDEAN.compare_bytes(&b(&[-128, 0, 127]), &byte_query_vector)?,
    )?);

    let mut dv = byte_dv(&byte_query_vector, "knnByteField5")?;
    Assert::assert_false(dv.advance_exact(0)?);
    Assert::assert_false(dv.advance_exact(1)?);
    Assert::assert_true(matches(
        &mut *dv,
        2,
        EUCLIDEAN.compare_bytes(&b(&[-120, -2, 3]), &byte_query_vector)?,
    )?);
    Ok(())
}

/// Port of `TestVectorSimilarityValuesSource.testDotSimilarityValuesSource`.
fn test_dot_similarity_values_source() -> Result<()> {
    let float_query_vector = [10.0f32, 1.0, -8.5];

    // Checks the computed similarity score between indexed vectors and query vector
    // using DVS is correct by passing indexed and query vector in #compare
    let mut dv = float_dv(&float_query_vector, "knnFloatField2")?;
    Assert::assert_true(matches(
        &mut *dv,
        0,
        DOT_PRODUCT.compare_f32(&[2.2, -3.2, -3.1], &float_query_vector)?,
    )?);
    Assert::assert_true(matches(
        &mut *dv,
        1,
        DOT_PRODUCT.compare_f32(&[-5.2, 8.7, 3.1], &float_query_vector)?,
    )?);
    Assert::assert_true(matches(
        &mut *dv,
        2,
        DOT_PRODUCT.compare_f32(&[-8.0, 7.0, -6.0], &float_query_vector)?,
    )?);

    let byte_query_vector = b(&[-128, 2, 127]);

    let mut dv = byte_dv(&byte_query_vector, "knnByteField2")?;
    Assert::assert_true(matches(
        &mut *dv,
        0,
        DOT_PRODUCT.compare_bytes(&b(&[4, 2, 3]), &byte_query_vector)?,
    )?);
    Assert::assert_true(matches(
        &mut *dv,
        1,
        DOT_PRODUCT.compare_bytes(&b(&[40, 21, 3]), &byte_query_vector)?,
    )?);
    Assert::assert_true(matches(
        &mut *dv,
        2,
        DOT_PRODUCT.compare_bytes(&b(&[-1, -2, -3]), &byte_query_vector)?,
    )?);
    Ok(())
}

/// Port of `TestVectorSimilarityValuesSource.testCosineSimilarityValuesSource`.
fn test_cosine_similarity_values_source() -> Result<()> {
    let float_query_vector = [0.6f32, -1.6, 38.0];

    // Checks the computed similarity score between indexed vectors and query vector
    // using DVS is correct by passing indexed and query vector in #compare
    let mut dv = float_dv(&float_query_vector, "knnFloatField3")?;
    Assert::assert_true(matches(
        &mut *dv,
        0,
        COSINE.compare_f32(&[4.5, 10.3, -7.0], &float_query_vector)?,
    )?);
    Assert::assert_true(matches(
        &mut *dv,
        1,
        COSINE.compare_f32(&[0.2, -3.2, 3.1], &float_query_vector)?,
    )?);
    Assert::assert_false(dv.advance_exact(2)?);

    let byte_query_vector = b(&[-10, 8, 0]);

    let mut dv = byte_dv(&byte_query_vector, "knnByteField3")?;
    Assert::assert_true(matches(
        &mut *dv,
        0,
        COSINE.compare_bytes(&b(&[-121, -64, -1]), &byte_query_vector)?,
    )?);
    Assert::assert_true(matches(
        &mut *dv,
        1,
        COSINE.compare_bytes(&b(&[9, 2, 3]), &byte_query_vector)?,
    )?);
    Assert::assert_true(matches(
        &mut *dv,
        2,
        COSINE.compare_bytes(&b(&[4, 2, 3]), &byte_query_vector)?,
    )?);
    Ok(())
}

/// Port of `TestVectorSimilarityValuesSource.testMaximumProductSimilarityValuesSource`.
fn test_maximum_product_similarity_values_source() -> Result<()> {
    let float_query_vector = [1.0f32, -6.0, -10.0];

    // Checks the computed similarity score between indexed vectors and query vector
    // using DVS is correct by passing indexed and query vector in #compare
    let mut dv = float_dv(&float_query_vector, "knnFloatField4")?;
    Assert::assert_true(dv.advance_exact(0)?);
    assert_equals_delta(
        f64::from(MAXIMUM_INNER_PRODUCT.compare_f32(&[-1.3, 1.0, 1.0], &float_query_vector)?),
        dv.double_value()?,
        0.0001,
    );
    Assert::assert_false(dv.advance_exact(1)?);
    Assert::assert_false(dv.advance_exact(2)?);

    let byte_query_vector = b(&[-127, 127, 127]);

    let mut dv = byte_dv(&byte_query_vector, "knnByteField4")?;
    Assert::assert_true(dv.advance_exact(0)?);
    assert_equals_delta(
        f64::from(MAXIMUM_INNER_PRODUCT.compare_bytes(&b(&[-127, 127, 127]), &byte_query_vector)?),
        dv.double_value()?,
        0.0001,
    );
    Assert::assert_true(dv.advance_exact(1)?);
    assert_equals_delta(
        f64::from(MAXIMUM_INNER_PRODUCT.compare_bytes(&b(&[14, 29, 31]), &byte_query_vector)?),
        dv.double_value()?,
        0.0001,
    );
    Assert::assert_true(dv.advance_exact(2)?);
    assert_equals_delta(
        f64::from(MAXIMUM_INNER_PRODUCT.compare_bytes(&b(&[-4, -2, -128]), &byte_query_vector)?),
        dv.double_value()?,
        0.0001,
    );
    Ok(())
}

/// Port of `TestVectorSimilarityValuesSource.testFailuresWithSimilarityValuesSource`.
fn test_failures_with_similarity_values_source() -> Result<()> {
    let float_query_vector = [1.1f32, 2.2, 3.3];
    let byte_query_vector = b(&[-10, 20, 30]);

    expect_throws("java.lang.IllegalStateException", || {
        float_dv(&float_query_vector, "knnByteField1")
    });
    expect_throws("java.lang.IllegalStateException", || {
        byte_dv(&byte_query_vector, "knnFloatField1")
    });

    let mut dv = float_dv(&float_query_vector, "knnFloatField1")?;
    Assert::assert_true(dv.advance_exact(0)?);
    assert_equals_delta(
        dv.double_value()?,
        f64::from(EUCLIDEAN.compare_f32(&[1.0, 2.0, 3.0], &float_query_vector)?),
        0.0,
    );
    assert_not_equals(
        &SortValue::Double(dv.double_value()?),
        &SortValue::Float(DOT_PRODUCT.compare_f32(&[1.0, 2.0, 3.0], &float_query_vector)?),
    );
    assert_not_equals(
        &SortValue::Double(dv.double_value()?),
        &SortValue::Float(COSINE.compare_f32(&[1.0, 2.0, 3.0], &float_query_vector)?),
    );
    assert_not_equals(
        &SortValue::Double(dv.double_value()?),
        &SortValue::Float(
            MAXIMUM_INNER_PRODUCT.compare_f32(&[1.0, 2.0, 3.0], &float_query_vector)?,
        ),
    );
    Ok(())
}
