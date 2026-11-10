//! The `lucene/codecs` test class that extends `BaseIndexFileFormatTestCase`
//! directly (rmp #361), ported in full with the cases it inherits:
//! `org.apache.lucene.codecs.bitvectors.TestHnswBitVectorsFormat`, whose
//! codec is `TestUtil.alwaysKnnVectorsFormat(new HnswBitVectorsFormat())`
//! and whose `addRandomFields` adds a 30-dimension byte vector, with its own
//! tests `testFloatVectorFails`, `testIndexAndSearchBitVectors`,
//! `testToString` and `testLimits`.
//!
//! The suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::codecs::{self, Codec, FilterCodec, KnnVectorsFormat};
use rucene::document::{Document, KnnByteVectorField, KnnFloatVectorField, Store, StringField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{IndexReader, VectorSimilarityFunction};
use rucene::search::knn::{KnnCollector, TopKnnCollector};
use rucene::search::BitsAcceptDocs;
use rucene_codecs::bitvectors::HnswBitVectorsFormat;
use rucene_test_framework::index::base_index_file_format_test_case::{
    self, rethrow, BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::base_knn_vectors_format_test_case::random_vector8;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

#[path = "../../rucene-test-framework/tests/suite_seeds/mod.rs"]
mod suite_seeds;
use suite_seeds::run_suite;

/// `TestHnswBitVectorsFormat`.
struct TestHnswBitVectorsFormat;

impl BaseIndexFileFormatTestCase for TestHnswBitVectorsFormat {
    fn get_codec(&self) -> Arc<dyn Codec> {
        TestUtil::always_knn_vectors_format(Arc::new(HnswBitVectorsFormat::new()))
    }

    fn add_random_fields(&self, doc: &mut Document) {
        doc.add(Box::new(
            KnnByteVectorField::new(
                "v2",
                &random_vector8(30),
                VectorSimilarityFunction::DOT_PRODUCT,
            )
            .unwrap_or_else(|e| rethrow(e)),
        ));
    }
}

/// `testFloatVectorFails()`.
fn test_float_vector_fails() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let w = IndexWriter::new(
            dir.as_directory(),
            LuceneTestCase::new_index_writer_config(),
        )?;
        let mut doc = Document::new();
        doc.add(Box::new(KnnFloatVectorField::new(
            "f",
            &[0f32; 4],
            VectorSimilarityFunction::DOT_PRODUCT,
        )?));
        let e = expect_throws("java.lang.IllegalArgumentException", || {
            w.add_document(&doc)
        });
        // Java discards the result of `e.getMessage().contains(...)`.
        let _ = e
            .message()
            .unwrap_or("")
            .contains("HnswBitVectorsFormat only supports BYTE encoding");
        w.close()
    })();
    result.and(dir.close())
}

/// `testIndexAndSearchBitVectors()`.
fn test_index_and_search_bit_vectors() -> Result<()> {
    let vectors: [[u8; 2]; 5] = [
        [0b10101110, 0b01010111],
        [0b11111000, 0b00001111],
        [0b11001100, 0b00110011],
        [0b11111111, 0b00000000],
        [0b00000000, 0b00000000],
    ];
    let mut dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let w = IndexWriter::new(
            dir.as_directory(),
            LuceneTestCase::new_index_writer_config(),
        )?;
        let searched = (|| -> Result<()> {
            let mut id = 0;
            for vector in &vectors {
                let mut doc = Document::new();
                doc.add(Box::new(KnnByteVectorField::new(
                    "v1",
                    vector,
                    VectorSimilarityFunction::DOT_PRODUCT,
                )?));
                doc.add(Box::new(StringField::new(
                    "id",
                    id.to_string(),
                    Store::YES,
                )?));
                id += 1;
                w.add_document(&doc)?;
            }
            w.commit()?;
            w.force_merge(1)?;
            let reader = directory_reader::open_with_writer(&*w)?;
            let read = (|| -> Result<()> {
                let r = LuceneTestCase::get_only_leaf_reader(
                    Arc::clone(&reader) as Arc<dyn IndexReader>
                )?;
                let mut collector = TopKnnCollector::new(3, i64::from(i32::MAX));
                let mut accept = BitsAcceptDocs::from_live_docs(None, r.max_doc())?;
                r.search_nearest_vectors_byte("v1", &vectors[0], &mut collector, &mut accept)?;
                let top_docs = collector.top_docs();
                Assert::assert_equals(3usize, top_docs.score_docs.len());

                let fields = r.stored_fields()?;
                let id_of =
                    |doc: i32| -> Result<Option<String>> { Ok(fields.document(doc)?.get("id")) };
                Assert::assert_equals_str(
                    None,
                    Some("0"),
                    id_of(top_docs.score_docs[0].doc)?.as_deref(),
                );
                Assert::assert_equals_double_msg(
                    None,
                    1.0,
                    f64::from(top_docs.score_docs[0].score),
                    1e-12,
                );
                Assert::assert_equals_str(
                    None,
                    Some("2"),
                    id_of(top_docs.score_docs[1].doc)?.as_deref(),
                );
                Assert::assert_equals_double_msg(
                    None,
                    0.625,
                    f64::from(top_docs.score_docs[1].score),
                    1e-12,
                );
                Assert::assert_equals_str(
                    None,
                    Some("1"),
                    id_of(top_docs.score_docs[2].doc)?.as_deref(),
                );
                Assert::assert_equals_double_msg(
                    None,
                    0.5625,
                    f64::from(top_docs.score_docs[2].score),
                    1e-12,
                );
                Ok(())
            })();
            read.and((Arc::clone(&reader) as Arc<dyn IndexReader>).close())
        })();
        searched.and(w.close())
    })();
    result.and(dir.close())
}

/// `testToString()`: the anonymous `FilterCodec("foo", Codec.getDefault())`
/// overriding `knnVectorsFormat()` is a `FilterCodec` with that format.
fn test_to_string() -> Result<()> {
    let default = codecs::get_default().unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    });
    let custom_codec = FilterCodec::new("foo", default)
        .with_knn_vectors_format(HnswBitVectorsFormat::with_params(10, 20)?);
    let expected_string = "HnswBitVectorsFormat(name=HnswBitVectorsFormat, maxConn=10, beamWidth=20, flatVectorFormat=Lucene99FlatVectorsFormat(vectorsScorer=FlatBitVectorsScorer()))";
    // `knnVectorsFormat().toString()`: the format's `Display`.
    let format: &dyn KnnVectorsFormat = custom_codec.knn_vectors_format();
    let actual = format
        .as_any()
        .and_then(|a| a.downcast_ref::<HnswBitVectorsFormat>())
        .map(|f| f.to_string());
    Assert::assert_equals_str(None, Some(expected_string), actual.as_deref());
    Ok(())
}

/// `testLimits()`.
fn test_limits() -> Result<()> {
    const IAE: &str = "java.lang.IllegalArgumentException";
    expect_throws(IAE, || HnswBitVectorsFormat::with_params(-1, 20));
    expect_throws(IAE, || HnswBitVectorsFormat::with_params(0, 20));
    expect_throws(IAE, || HnswBitVectorsFormat::with_params(20, 0));
    expect_throws(IAE, || HnswBitVectorsFormat::with_params(20, -1));
    expect_throws(IAE, || HnswBitVectorsFormat::with_params(512 + 1, 20));
    expect_throws(IAE, || HnswBitVectorsFormat::with_params(20, 3201));
    Ok(())
}

/// A test method `public void <name>() throws IOException`.
fn own(name: &str, body: fn() -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.io.IOException"])
        .body(move |_: &mut FormatTestInstance| body().unwrap_or_else(|e| rethrow(e)))
}

fn hnsw_bit_vectors_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.bitvectors.TestHnswBitVectorsFormat")
        .extends(&base_index_file_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(TestHnswBitVectorsFormat))
        }))
        .method(own("testFloatVectorFails", test_float_vector_fails))
        .method(own(
            "testIndexAndSearchBitVectors",
            test_index_and_search_bit_vectors,
        ))
        .method(own("testToString", test_to_string))
        .method(own("testLimits", test_limits))
        .build()
}

#[test]
fn test_hnsw_bit_vectors_format() {
    run_suite(hnsw_bit_vectors_class);
}
