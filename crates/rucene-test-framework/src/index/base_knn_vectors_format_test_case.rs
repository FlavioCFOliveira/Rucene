/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `org.apache.lucene.tests.index.BaseKnnVectorsFormatTestCase`:
//! base class aiming at testing `KnnVectorsFormat` vectors formats. To test a
//! new format, all you need is to register a new `Codec` which uses it and
//! extend this class and override `getCodec()`.
//!
//! The class is declared by [`class()`] and its members are the trait
//! [`BaseKnnVectorsFormatTestCase`]; see
//! [`base_index_file_format_test_case`](super::base_index_file_format_test_case)
//! for how a suite runs. The instance fields `vectorEncoding` and
//! `similarityFunction`, which the `@Before init()` draws, live in the
//! subclass's [`KnnSuiteState`].
//!
//! **Divergences** (declared, each confined to the member named):
//! * `testFieldConstructor`, `testFieldSetValue`: `assertSame(v,
//!   field.vectorValue())` — the port's field holds its own copy, so the
//!   test compares the values.
//! * `testFieldConstructorExceptions`, `testFieldSetValue`,
//!   `testInvalidKnnVectorFieldUsage`: the `null` name, vector and
//!   similarity, `setVectorValue(null)` and `setIntValue(14)` cannot be
//!   expressed: Rust's types have no `null`, and `KnnFloatVectorField` has no
//!   `setIntValue`. The cases that can be expressed (an empty vector, a
//!   vector of another dimension) are kept.
//! * `hasQuantized`, `hasHNSW`: Java falls back on
//!   `getClass().getSimpleName()`; the port answers the simple name of the
//!   readers it can recognise through `KnnVectorsReader::as_any`.
//! * `testFloatVectorScorerIteration`, `testByteVectorScorerIteration`:
//!   `assertSame(iterator, scorer.iterator())` compares the addresses the two
//!   calls return; `assertNotSame(iterator, scorer)` holds by construction
//!   (different objects of different types).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::junit::{Assert, Before};
use rucene::codecs::hnsw::graph_provider::HnswGraphProvider;
use rucene::codecs::knn_vectors::{
    merge_off_heap_byte_size_maps, FieldVectorWriter, KnnVectorsFormat, KnnVectorsReader,
    KnnVectorsWriter,
};
use rucene::codecs::lucene104::scalar_quantized_vectors::Lucene104ScalarQuantizedVectorsReader;
use rucene::codecs::lucene99::hnsw_vectors_format::{
    Lucene99HnswVectorsReader, EXHAUSTIVE_BULK_SCORE_ORDS,
};
use rucene::codecs::per_field::PerFieldKnnVectorsFormat;
use rucene::codecs::state::{OwnedSegmentWriteState, SegmentReadState};
use rucene::codecs::{Codec, FilterCodec};
use rucene::document::{
    Document, KnnByteVectorField, KnnFloatVectorField, NumericDocValuesField, Store, StoredField,
    StringField,
};
use rucene::error::{LuceneError, Result};
use rucene::index::check_index::Level;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::documents_writer::BufferedUpdates;
use rucene::index::field_infos::{FieldInfo, FieldInfos};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::{MergeTrigger, NoMergePolicy};
use rucene::index::merge_scheduler::{MergeScheduler, MergeSource, SerialMergeScheduler};
use rucene::index::segment_info::SegmentInfo;
use rucene::index::terms::Term;
use rucene::index::{
    DocValuesSkipIndexType, DocValuesType, IndexOptions, IndexableField, LeafReader,
    VectorEncoding, VectorSimilarityFunction,
};
use rucene::search::{
    from_live_docs, DocIdSetIterator, Sort, SortField, SortFieldType, TopDocs, TotalHitsRelation,
    NO_MORE_DOCS,
};
use rucene::store::{Directory, FSDirectory};
use rucene::util::java_collections::JavaStringMap;
use rucene::util::quantization::QuantizedVectorsReader;
use rucene::util::string_helper::StringHelper;
use rucene::util::{vector_util, BytesRef, NoOutputInfoStream, SameThreadExecutorService, Version};

use super::base_field_info_format_test_case::{
    VECTOR_ENCODING_VALUES, VECTOR_SIMILARITY_FUNCTION_VALUES,
};
use super::base_index_file_format_test_case::{
    assert_array_equals, assert_not_null, assert_null, exception_message, expect_throws_lucene,
    format_test, rethrow, BaseIndexFileFormatTestCase, FormatTestInstance,
};
use super::force_merge_policy::ForceMergePolicy;
use super::mismatched_codec_reader::MismatchedCodecReader;
use super::random_index_writer::RandomIndexWriter;
use crate::codecs::asserting::asserting_knn_vectors_format::AssertingKnnVectorsReader;
use crate::util::lucene_test_case::LuceneTestCase;
use crate::util::TestUtil;

/// The instance fields of `BaseKnnVectorsFormatTestCase` that `init()`
/// draws: `vectorEncoding` and `similarityFunction`.
#[derive(Debug)]
pub struct KnnSuiteState {
    vector_encoding: Mutex<VectorEncoding>,
    similarity_function: Mutex<VectorSimilarityFunction>,
}

impl Default for KnnSuiteState {
    fn default() -> Self {
        KnnSuiteState {
            vector_encoding: Mutex::new(VectorEncoding::FLOAT32),
            similarity_function: Mutex::new(VectorSimilarityFunction::EUCLIDEAN),
        }
    }
}

impl KnnSuiteState {
    fn vector_encoding(&self) -> VectorEncoding {
        *self
            .vector_encoding
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn similarity_function(&self) -> VectorSimilarityFunction {
        *self
            .similarity_function
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }
}

/// The declaration of `org.apache.lucene.tests.index.BaseKnnVectorsFormatTestCase`
/// (`public abstract class BaseKnnVectorsFormatTestCase extends
/// BaseIndexFileFormatTestCase`), with its `@Before init()` and its tests.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        let t = |name: &str, body: fn(&dyn BaseKnnVectorsFormatTestCase) -> Result<()>| {
            format_test(name, move |c| body(knn_case(c)))
        };
        Class::builder("org.apache.lucene.tests.index.BaseKnnVectorsFormatTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&super::base_index_file_format_test_case::class())
            .method(
                Method::new("init")
                    .annotate(Before)
                    .body(|inst: &mut FormatTestInstance| {
                        let case = Arc::clone(&inst.case);
                        knn_case(&*case).init();
                    }),
            )
            .method(t("testFieldConstructor", |c| c.test_field_constructor()))
            .method(t("testFieldConstructorExceptions", |c| {
                c.test_field_constructor_exceptions()
            }))
            .method(t("testFieldSetValue", |c| c.test_field_set_value()))
            .method(t("testIllegalDimChangeTwoDocs", |c| {
                c.test_illegal_dim_change_two_docs()
            }))
            .method(t("testIllegalSimilarityFunctionChange", |c| {
                c.test_illegal_similarity_function_change()
            }))
            .method(t("testIllegalDimChangeTwoWriters", |c| {
                c.test_illegal_dim_change_two_writers()
            }))
            .method(t("testMergingWithDifferentKnnFields", |c| {
                c.test_merging_with_different_knn_fields()
            }))
            .method(t("testMergingWithDifferentByteKnnFields", |c| {
                c.test_merging_with_different_byte_knn_fields()
            }))
            .method(t("testWriterRamEstimate", |c| c.test_writer_ram_estimate()))
            .method(t("testWriterByteVectorRamEstimate", |c| {
                c.test_writer_byte_vector_ram_estimate()
            }))
            .method(t("testIllegalSimilarityFunctionChangeTwoWriters", |c| {
                c.test_illegal_similarity_function_change_two_writers()
            }))
            .method(t("testAddIndexesDirectory0", |c| {
                c.test_add_indexes_directory0()
            }))
            .method(t("testAddIndexesDirectory1", |c| {
                c.test_add_indexes_directory1()
            }))
            .method(t("testAddIndexesDirectory01", |c| {
                c.test_add_indexes_directory01()
            }))
            .method(t("testIllegalDimChangeViaAddIndexesDirectory", |c| {
                c.test_illegal_dim_change_via_add_indexes_directory()
            }))
            .method(t(
                "testIllegalSimilarityFunctionChangeViaAddIndexesDirectory",
                |c| c.test_illegal_similarity_function_change_via_add_indexes_directory(),
            ))
            .method(t("testIllegalDimChangeViaAddIndexesCodecReader", |c| {
                c.test_illegal_dim_change_via_add_indexes_codec_reader()
            }))
            .method(t(
                "testIllegalSimilarityFunctionChangeViaAddIndexesCodecReader",
                |c| c.test_illegal_similarity_function_change_via_add_indexes_codec_reader(),
            ))
            .method(t("testIllegalDimChangeViaAddIndexesSlowCodecReader", |c| {
                c.test_illegal_dim_change_via_add_indexes_slow_codec_reader()
            }))
            .method(t(
                "testIllegalSimilarityFunctionChangeViaAddIndexesSlowCodecReader",
                |c| c.test_illegal_similarity_function_change_via_add_indexes_slow_codec_reader(),
            ))
            .method(t("testIllegalMultipleValues", |c| {
                c.test_illegal_multiple_values()
            }))
            .method(t("testIllegalDimensionTooLarge", |c| {
                c.test_illegal_dimension_too_large()
            }))
            .method(t("testIllegalEmptyVector", |c| {
                c.test_illegal_empty_vector()
            }))
            .method(t("testDifferentCodecs1", |c| c.test_different_codecs1()))
            .method(t("testDifferentCodecs2", |c| c.test_different_codecs2()))
            .method(t("testInvalidKnnVectorFieldUsage", |c| {
                c.test_invalid_knn_vector_field_usage()
            }))
            .method(t("testDeleteAllVectorDocs", |c| {
                c.test_delete_all_vector_docs()
            }))
            .method(t("testKnnVectorFieldMissingFromOneSegment", |c| {
                c.test_knn_vector_field_missing_from_one_segment()
            }))
            .method(t("testSparseVectors", |c| c.test_sparse_vectors()))
            .method(t("testFloatVectorScorerIteration", |c| {
                c.test_float_vector_scorer_iteration()
            }))
            .method(t("testByteVectorScorerIteration", |c| {
                c.test_byte_vector_scorer_iteration()
            }))
            .method(t("testEmptyFloatVectorData", |c| {
                c.test_empty_float_vector_data()
            }))
            .method(t("testEmptyByteVectorData", |c| {
                c.test_empty_byte_vector_data()
            }))
            .method(t("testIndexedValueNotAliased", |c| {
                c.test_indexed_value_not_aliased()
            }))
            .method(t("testSortedIndex", |c| c.test_sorted_index()))
            .method(t("testSortedIndexBytes", |c| c.test_sorted_index_bytes()))
            .method(t("testIndexMultipleKnnVectorFields", |c| {
                c.test_index_multiple_knn_vector_fields()
            }))
            .method(t("testRandom", |c| c.test_random()))
            .method(t("testRandomBytes", |c| c.test_random_bytes()))
            .method(t("testSearchWithVisitedLimit", |c| {
                c.test_search_with_visited_limit()
            }))
            .method(t("testRandomWithUpdatesAndGraph", |c| {
                c.test_random_with_updates_and_graph()
            }))
            .method(t("testCheckIndexIncludesVectors", |c| {
                c.test_check_index_includes_vectors()
            }))
            .method(t("testSimilarityFunctionIdentifiers", |c| {
                c.test_similarity_function_identifiers()
            }))
            .method(t("testVectorEncodingOrdinals", |c| {
                c.test_vector_encoding_ordinals()
            }))
            .method(t("testAdvance", |c| c.test_advance()))
            .method(t("testVectorValuesReportCorrectDocs", |c| {
                c.test_vector_values_report_correct_docs()
            }))
            .method(t("testReadQuantizedVectorWithEmptyRawVectors", |c| {
                c.test_read_quantized_vector_with_empty_raw_vectors()
            }))
            .method(t("testMismatchedFields", |c| c.test_mismatched_fields()))
            .method(t("testMergeOffHeapByteSizeMaps", |c| {
                c.test_merge_off_heap_byte_size_maps()
            }))
            .build()
    });
    Arc::clone(&CLASS)
}

/// The instance's subclass as a `BaseKnnVectorsFormatTestCase`.
fn knn_case(c: &dyn BaseIndexFileFormatTestCase) -> &dyn BaseKnnVectorsFormatTestCase {
    c.as_knn_vectors().unwrap_or_else(|| {
        Throwable::new(
            "java.lang.IllegalArgumentException",
            "object is not an instance of org.apache.lucene.tests.index.BaseKnnVectorsFormatTestCase",
        )
        .throw()
    })
}

/// `random()`.
fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// `randomVector(dim)` (`public static`).
pub fn random_vector(dim: i32) -> Vec<f32> {
    assert!(dim > 0);
    let mut v = vec![0f32; dim as usize];
    let mut square_sum = 0.0f64;
    // keep generating until we don't get a zero-length vector
    while square_sum == 0.0 {
        square_sum = 0.0;
        for x in v.iter_mut() {
            *x = random().next_float();
            // `squareSum += v[i] * v[i]`: a float product widened to double.
            square_sum += f64::from(*x * *x);
        }
    }
    v
}

/// `randomNormalizedVector(dim)` (`public static`).
pub fn random_normalized_vector(dim: i32) -> Vec<f32> {
    let mut v = random_vector(dim);
    vector_util::l2normalize(&mut v, true).unwrap_or_else(|e| rethrow(e));
    v
}

/// `randomVector8(dim)` (`public static`): `(byte) (v[i] * 127)` of a
/// normalized vector.
pub fn random_vector8(dim: i32) -> Vec<u8> {
    assert!(dim > 0);
    let v = random_normalized_vector(dim);
    v.iter().map(|x| (*x * 127.0) as i32 as i8 as u8).collect()
}

/// `assertArrayEquals(message, expecteds, actuals, delta)` over floats, with
/// `expecteds` possibly `null`.
fn assert_float_array_equals(message: &str, expected: Option<&[f32]>, actual: &[f32], delta: f32) {
    let Some(expected) = expected else {
        Throwable::new(
            "java.lang.AssertionError",
            format!("{message}: expected array was null"),
        )
        .throw()
    };
    if expected.len() != actual.len() {
        Throwable::new(
            "org.junit.internal.ArrayComparisonFailure",
            format!(
                "{message}: array lengths differed, expected.length={} actual.length={}",
                expected.len(),
                actual.len()
            ),
        )
        .throw()
    }
    for (i, (e, a)) in expected.iter().zip(actual).enumerate() {
        if !(e.to_bits() == a.to_bits() || (e - a).abs() <= delta) {
            Throwable::new(
                "org.junit.internal.ArrayComparisonFailure",
                format!(
                    "{message}: arrays first differed at element [{i}]; expected:<{e}> but was:<{a}>"
                ),
            )
            .throw()
        }
    }
}

/// `assertEquals(expected, actual, delta)` over floats.
fn assert_float_equals(expected: f32, actual: f32, delta: f32) {
    Assert::assert_equals_float_msg(None, expected, actual, delta);
}

/// The `IllegalArgumentException` a writer answers, with its message.
fn expect_illegal_argument<T>(runnable: impl FnOnce() -> Result<T>) -> String {
    let e = expect_throws_lucene(
        "java.lang.IllegalArgumentException",
        |e| matches!(e, LuceneError::IllegalArgument(_)),
        runnable,
    );
    exception_message(&e)
}

/// `new IndexWriter(dir, newIndexWriterConfig())`.
fn new_writer(dir: Arc<dyn Directory>) -> Result<Arc<IndexWriter>> {
    IndexWriter::new(dir, LuceneTestCase::new_index_writer_config())
}

/// A document of one float vector field.
fn float_doc(name: &str, vector: &[f32], sim: VectorSimilarityFunction) -> Result<Document> {
    let mut doc = Document::new();
    doc.add(Box::new(KnnFloatVectorField::new(name, vector, sim)?));
    Ok(doc)
}

/// The `TestMergeScheduler` of the merging tests: runs every merge, and
/// keeps the first `IllegalStateException` or `IllegalArgumentException`
/// one throws.
#[derive(Default)]
pub struct TestMergeScheduler {
    ex: Mutex<Option<LuceneError>>,
    serial: SerialMergeScheduler,
}

impl std::fmt::Display for TestMergeScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TestMergeScheduler")
    }
}

impl MergeScheduler for TestMergeScheduler {
    fn merge(&self, merge_source: Arc<dyn MergeSource>, _trigger: MergeTrigger) -> Result<()> {
        loop {
            let Some(merge) = merge_source.get_next_merge()? else {
                break;
            };
            match merge_source.merge(merge) {
                Ok(()) => {}
                Err(e @ (LuceneError::IllegalState { .. } | LuceneError::IllegalArgument(_))) => {
                    *self.ex.lock().unwrap_or_else(|e| e.into_inner()) = Some(e);
                    break;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn same_thread_executor(&self) -> &Arc<SameThreadExecutorService> {
        self.serial.same_thread_executor()
    }

    fn close(&self) -> Result<()> {
        Ok(())
    }
}

/// A `KnnVectorsFormat` held behind an `Arc`, for `FilterCodec`'s override
/// (`new FilterCodec(codec.getName(), codec) { knnVectorsFormat() { return
/// format; } }`).
#[derive(Debug)]
struct SharedKnnFormat(Arc<dyn KnnVectorsFormat>);

impl KnnVectorsFormat for SharedKnnFormat {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn fields_writer(&self, state: &OwnedSegmentWriteState) -> Result<Box<dyn KnnVectorsWriter>> {
        self.0.fields_writer(state)
    }

    fn fields_reader<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn KnnVectorsReader>> {
        self.0.fields_reader(state)
    }

    fn get_max_dimensions(&self, field_name: &str) -> i32 {
        self.0.get_max_dimensions(field_name)
    }
}

/// `getNumVectors(reader, fieldInfo)` (`static`).
fn get_num_vectors(reader: &dyn KnnVectorsReader, field_info: &FieldInfo) -> Result<i32> {
    Ok(match field_info.vector_encoding {
        VectorEncoding::BYTE => reader
            .get_byte_vector_values(&field_info.name)?
            .expect("the field has byte vectors")
            .size(),
        VectorEncoding::FLOAT32 => reader
            .get_float_vector_values(&field_info.name)?
            .expect("the field has float vectors")
            .size(),
    })
}

/// `knnVectorsReader.getClass().getSimpleName().toLowerCase(Locale.ROOT)`
/// of the readers the port can recognise (see the module docs).
fn simple_name(reader: &dyn KnnVectorsReader) -> String {
    let Some(any) = reader.as_any() else {
        return String::new();
    };
    if any.is::<Lucene99HnswVectorsReader>() {
        "lucene99hnswvectorsreader".to_string()
    } else if any.is::<Lucene104ScalarQuantizedVectorsReader>() {
        "lucene104scalarquantizedvectorsreader".to_string()
    } else if any.is::<rucene_codecs::simpletext::knn_vectors_reader::SimpleTextKnnVectorsReader>()
    {
        "simpletextknnvectorsreader".to_string()
    } else {
        String::new()
    }
}

/// `knnVectorsReader instanceof AssertingKnnVectorsReader assertingReader`:
/// its delegate, else the reader.
fn unwrap_asserting(reader: &dyn KnnVectorsReader) -> &dyn KnnVectorsReader {
    match reader
        .as_any()
        .and_then(|a| a.downcast_ref::<AssertingKnnVectorsReader>())
    {
        Some(asserting) => &*asserting.delegate,
        None => reader,
    }
}

/// `hasQuantized(knnVectorsReader, fieldInfo)` (`static`).
fn has_quantized(reader: &dyn KnnVectorsReader, field_info: &FieldInfo) -> Result<bool> {
    if field_info.vector_encoding == VectorEncoding::BYTE {
        return Ok(false); // byte vectors are never auto-quantized
    }
    let reader = unwrap_asserting(reader);

    if let Some(any) = reader.as_any() {
        if let Some(r) = any.downcast_ref::<Lucene99HnswVectorsReader>() {
            return Ok(
                QuantizedVectorsReader::get_quantized_vector_values(r, &field_info.name)?.is_some(),
            );
        }
        if let Some(r) = any.downcast_ref::<Lucene104ScalarQuantizedVectorsReader>() {
            return Ok(
                QuantizedVectorsReader::get_quantized_vector_values(r, &field_info.name)?.is_some(),
            );
        }
    }

    Ok(simple_name(reader).contains("quantized"))
}

/// `hasHNSW(knnVectorsReader, fieldInfo)` (`static`).
fn has_hnsw(reader: &dyn KnnVectorsReader, field_info: &FieldInfo) -> Result<bool> {
    let reader = unwrap_asserting(reader);
    if let Some(provider) = reader
        .as_any()
        .and_then(|a| a.downcast_ref::<Lucene99HnswVectorsReader>())
    {
        return Ok(provider.get_graph(&field_info.name)?.size() > 0);
    }
    Ok(simple_name(reader).contains("hnsw"))
}

/// `storedFields.document(docId).get(name)`.
fn stored_get(leaf: &dyn LeafReader, doc_id: i32, name: &str) -> Result<Option<String>> {
    Ok(leaf
        .stored_fields()?
        .document(doc_id)?
        .get_values(name)
        .into_iter()
        .next())
}

/// `Integer.parseInt(s)`.
fn parse_int(s: &str) -> i32 {
    s.parse().unwrap_or_else(|_| {
        Throwable::new(
            "java.lang.NumberFormatException",
            format!("For input string: \"{s}\""),
        )
        .throw()
    })
}

/// `new FieldInfo("fieldA", 0, …, dim, encoding, DOT_PRODUCT, false,
/// false)` of the ram-estimate tests.
fn ram_estimate_field(dim: i32, encoding: VectorEncoding) -> FieldInfo {
    FieldInfo::new_full(
        "fieldA",
        0,
        false,
        false,
        false,
        IndexOptions::NONE,
        DocValuesType::NONE,
        DocValuesSkipIndexType::NONE,
        -1,
        JavaStringMap::default(),
        0,
        0,
        0,
        dim,
        encoding,
        VectorSimilarityFunction::DOT_PRODUCT,
        false,
        false,
    )
    .unwrap_or_else(|e| rethrow(e))
}

/// The `SegmentWriteState` of the ram-estimate tests.
fn ram_estimate_state(dir: Arc<dyn Directory>) -> Result<OwnedSegmentWriteState> {
    let field_infos = FieldInfos::new(Vec::new())?;
    let codec = rucene::codecs::get_default().expect("Codec.getDefault()");
    let si = SegmentInfo::new(
        Arc::clone(&dir),
        Version::LATEST,
        Some(Version::LATEST),
        "0".to_string(),
        10000,
        false,
        false,
        codec,
        HashMap::new(),
        StringHelper::random_id(),
        HashMap::new(),
        Sort::default(),
    )?;
    Ok(OwnedSegmentWriteState::new(
        Arc::new(NoOutputInfoStream),
        dir,
        si,
        field_infos,
        BufferedUpdates::new("0"),
        Arc::from(LuceneTestCase::new_io_context(&*random())),
    ))
}

/// The suite's `add(iw, field, id, sortkey, vector, similarityFunction)`
/// over float vectors (`private`).
fn add_float(
    iw: &IndexWriter,
    field: &str,
    id: i32,
    sortkey: i32,
    vector: Option<&[f32]>,
    similarity_function: VectorSimilarityFunction,
) -> Result<()> {
    let mut doc = Document::new();
    if let Some(vector) = vector {
        doc.add(Box::new(KnnFloatVectorField::new(
            field,
            vector,
            similarity_function,
        )?));
    }
    doc.add(Box::new(NumericDocValuesField::new(
        "sortkey",
        i64::from(sortkey),
    )));
    let id_string = id.to_string();
    doc.add(Box::new(StringField::new(
        "id",
        id_string.clone(),
        Store::YES,
    )?));
    let id_term = Term::new("id", BytesRef::new(id_string.into_bytes()));
    iw.update_document(id_term, &doc)?;
    Ok(())
}

/// The suite's `add(iw, field, id, sortKey, vector, similarityFunction)`
/// over byte vectors (`private`).
fn add_byte(
    iw: &IndexWriter,
    field: &str,
    id: i32,
    sort_key: i32,
    vector: Option<&[u8]>,
    similarity_function: VectorSimilarityFunction,
) -> Result<()> {
    let mut doc = Document::new();
    if let Some(vector) = vector {
        doc.add(Box::new(KnnByteVectorField::new(
            field,
            vector,
            similarity_function,
        )?));
    }
    doc.add(Box::new(NumericDocValuesField::new(
        "sortkey",
        i64::from(sort_key),
    )));
    let id_string = id.to_string();
    doc.add(Box::new(StringField::new(
        "id",
        id_string.clone(),
        Store::YES,
    )?));
    let id_term = Term::new("id", BytesRef::new(id_string.into_bytes()));
    iw.update_document(id_term, &doc)?;
    Ok(())
}

/// `add(iw, field, id, vector, similarityFunction)`: a random sort key.
fn add_float_random_key(
    iw: &IndexWriter,
    field: &str,
    id: i32,
    vector: Option<&[f32]>,
    similarity_function: VectorSimilarityFunction,
) -> Result<()> {
    let sortkey = random().next_int_bound(100);
    add_float(iw, field, id, sortkey, vector, similarity_function)
}

/// `add(iw, field, id, vector, similarity)` over bytes: a random sort key.
fn add_byte_random_key(
    iw: &IndexWriter,
    field: &str,
    id: i32,
    vector: Option<&[u8]>,
    similarity: VectorSimilarityFunction,
) -> Result<()> {
    let sort_key = random().next_int_bound(100);
    add_byte(iw, field, id, sort_key, vector, similarity)
}

/// `new Sort(new SortField("sortkey", SortField.Type.INT))`.
fn sortkey_sort() -> Result<Sort> {
    Sort::new_fields(vec![SortField::new(
        Some("sortkey".to_string()),
        SortFieldType::Int,
    )?])
}

/// `getOnlyLeafReader(reader)`.
fn only_leaf(reader: &Arc<dyn DirectoryReader>) -> Result<Arc<dyn LeafReader>> {
    LuceneTestCase::get_only_leaf_reader(Arc::clone(reader) as _)
}

/// `AcceptDocs.fromLiveDocs(liveDocs, maxDoc)` for `leaf`.
fn search_nearest(
    leaf: &dyn LeafReader,
    field: &str,
    target: &[f32],
    k: i32,
    visited_limit: i32,
) -> Result<TopDocs> {
    let mut accept_docs = from_live_docs(leaf.get_live_docs(), leaf.max_doc())?;
    leaf.search_nearest_vectors_default(field, target, k, &mut accept_docs, visited_limit)
}

/// Port of the members of `BaseKnnVectorsFormatTestCase`.
pub trait BaseKnnVectorsFormatTestCase: BaseIndexFileFormatTestCase {
    /// The instance fields `vectorEncoding` and `similarityFunction`.
    fn knn_state(&self) -> &KnnSuiteState;

    /// `supportsFloatVectorFallback()` (`protected abstract`).
    fn supports_float_vector_fallback(&self) -> bool;

    /// `getQuantizationBits()`: returns the number of bits used for
    /// quantization to compute epsilon tolerance of float quantization errors
    /// in test cases. Default is 8 bits, override in subclasses if needed.
    fn get_quantization_bits(&self) -> i32 {
        8
    }

    /// `getCodecForFloatVectorFallbackTest()`.
    fn get_codec_for_float_vector_fallback_test(&self) -> Arc<dyn Codec> {
        self.get_codec() // Default implementation
    }

    /// `randomSimilarity()` (`protected`).
    fn random_similarity(&self) -> VectorSimilarityFunction {
        VECTOR_SIMILARITY_FUNCTION_VALUES
            [random().next_int_bound(VECTOR_SIMILARITY_FUNCTION_VALUES.len() as i32) as usize]
    }

    /// `randomVectorEncoding()` (`protected`): this method is overrideable
    /// since old codec versions only support `VectorEncoding#FLOAT32`.
    fn random_vector_encoding(&self) -> VectorEncoding {
        VECTOR_ENCODING_VALUES
            [random().next_int_bound(VECTOR_ENCODING_VALUES.len() as i32) as usize]
    }

    /// `simulateEmptyRawVectors(dir)` (`protected`): simulates empty raw
    /// vectors by modifying index files. Override in codecs that support
    /// FloatVector fallback.
    ///
    /// # Errors
    ///
    /// The base throws `Exception`.
    fn simulate_empty_raw_vectors(&self, _dir: &dyn Directory) -> Result<()> {
        Err(LuceneError::Unchecked {
            class: "java.lang.Exception",
            message: Some(
                "simulateEmptyRawVectors must be implemented by codecs that support FloatVector fallback"
                    .to_string(),
            ),
            cause: None,
        })
    }

    /// `@Before init()`.
    fn init(&self) {
        let vector_encoding = self.random_vector_encoding();
        *self
            .knn_state()
            .vector_encoding
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = vector_encoding;
        let similarity_function = self.random_similarity();
        *self
            .knn_state()
            .similarity_function
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = similarity_function;
    }

    /// `addRandomFields(Document)`: the override a subclass's
    /// `add_random_fields` calls.
    fn knn_add_random_fields(&self, doc: &mut Document) {
        let similarity_function = self.knn_state().similarity_function();
        match self.knn_state().vector_encoding() {
            VectorEncoding::BYTE => doc.add(Box::new(
                KnnByteVectorField::new("v2", &random_vector8(30), similarity_function)
                    .unwrap_or_else(|e| rethrow(e)),
            )),
            VectorEncoding::FLOAT32 => doc.add(Box::new(
                KnnFloatVectorField::new("v2", &random_normalized_vector(30), similarity_function)
                    .unwrap_or_else(|e| rethrow(e)),
            )),
        }
    }

    /// `mergeIsStable()`: suppress this test from base class: merges for knn
    /// graphs are not stable due to connected components logic.
    fn knn_merge_is_stable(&self) -> bool {
        false
    }

    /// `getVectorsMaxDimensions(fieldName)` (`private`).
    fn get_vectors_max_dimensions(&self, field_name: &str) -> i32 {
        rucene::codecs::get_default()
            .expect("Codec.getDefault()")
            .knn_vectors_format()
            .get_max_dimensions(field_name)
    }

    /// `testFieldConstructor()`.
    fn test_field_constructor(&self) -> Result<()> {
        let v = [0f32; 1];
        let field = KnnFloatVectorField::with_euclidean("f", &v)?;
        Assert::assert_equals(1i64, i64::from(field.field_type().vector_dimension()));
        Assert::assert_true(
            field.field_type().vector_similarity_function() == VectorSimilarityFunction::EUCLIDEAN,
        );
        Assert::assert_true(field.vector_value() == v);
        Ok(())
    }

    /// `testFieldConstructorExceptions()` (see the module docs for the
    /// `null` cases).
    fn test_field_constructor_exceptions(&self) -> Result<()> {
        expect_illegal_argument(|| KnnFloatVectorField::with_euclidean("f", &[]));
        Ok(())
    }

    /// `testFieldSetValue()` (see the module docs for the `null` case).
    fn test_field_set_value(&self) -> Result<()> {
        let mut field = KnnFloatVectorField::with_euclidean("f", &[0f32; 1])?;
        let v1 = [0f32; 1];
        field.set_vector_value(&v1)?;
        Assert::assert_true(field.vector_value() == v1);
        expect_illegal_argument(|| field.set_vector_value(&[0f32; 2]));
        Ok(())
    }

    // Illegal schema change tests:

    /// `testIllegalDimChangeTwoDocs()`.
    fn test_illegal_dim_change_two_docs(&self) -> Result<()> {
        // illegal change in the same segment
        {
            let mut dir = LuceneTestCase::new_directory();
            let w = new_writer(dir.as_directory())?;
            w.add_document(&float_doc(
                "f",
                &[0f32; 4],
                VectorSimilarityFunction::DOT_PRODUCT,
            )?)?;

            let doc2 = float_doc("f", &[0f32; 6], VectorSimilarityFunction::DOT_PRODUCT)?;
            let expected = expect_illegal_argument(|| w.add_document(&doc2));
            let err_msg = "Inconsistency of field data structures across documents for field [f] of doc [1]. vector dimension: expected '4', but it has '6'.";
            Assert::assert_equals_str(None, Some(err_msg), Some(&expected));
            w.close()?;
            dir.close()?;
        }

        // illegal change in a different segment
        {
            let mut dir = LuceneTestCase::new_directory();
            let w = new_writer(dir.as_directory())?;
            w.add_document(&float_doc(
                "f",
                &[0f32; 4],
                VectorSimilarityFunction::DOT_PRODUCT,
            )?)?;
            w.commit()?;

            let doc2 = float_doc("f", &[0f32; 6], VectorSimilarityFunction::DOT_PRODUCT)?;
            let expected = expect_illegal_argument(|| w.add_document(&doc2));
            let err_msg = "cannot change field \"f\" from vector dimension=4, vector encoding=FLOAT32, vector similarity function=DOT_PRODUCT to inconsistent vector dimension=6, vector encoding=FLOAT32, vector similarity function=DOT_PRODUCT";
            Assert::assert_equals_str(None, Some(err_msg), Some(&expected));
            w.close()?;
            dir.close()?;
        }
        Ok(())
    }

    /// `testIllegalSimilarityFunctionChange()`.
    fn test_illegal_similarity_function_change(&self) -> Result<()> {
        // illegal change in the same segment
        {
            let mut dir = LuceneTestCase::new_directory();
            let w = new_writer(dir.as_directory())?;
            w.add_document(&float_doc(
                "f",
                &[0f32; 4],
                VectorSimilarityFunction::DOT_PRODUCT,
            )?)?;

            let doc2 = float_doc("f", &[0f32; 4], VectorSimilarityFunction::EUCLIDEAN)?;
            let expected = expect_illegal_argument(|| w.add_document(&doc2));
            let err_msg = "Inconsistency of field data structures across documents for field [f] of doc [1]. vector similarity function: expected 'DOT_PRODUCT', but it has 'EUCLIDEAN'.";
            Assert::assert_equals_str(None, Some(err_msg), Some(&expected));
            w.close()?;
            dir.close()?;
        }

        // illegal change a different segment
        {
            let mut dir = LuceneTestCase::new_directory();
            let w = new_writer(dir.as_directory())?;
            w.add_document(&float_doc(
                "f",
                &[0f32; 4],
                VectorSimilarityFunction::DOT_PRODUCT,
            )?)?;
            w.commit()?;

            let doc2 = float_doc("f", &[0f32; 4], VectorSimilarityFunction::EUCLIDEAN)?;
            let expected = expect_illegal_argument(|| w.add_document(&doc2));
            let err_msg = "cannot change field \"f\" from vector dimension=4, vector encoding=FLOAT32, vector similarity function=DOT_PRODUCT to inconsistent vector dimension=4, vector encoding=FLOAT32, vector similarity function=EUCLIDEAN";
            Assert::assert_equals_str(None, Some(err_msg), Some(&expected));
            w.close()?;
            dir.close()?;
        }
        Ok(())
    }

    /// The body of the two-writers tests: the first writer adds `first`, the
    /// second's `second` must fail with `message`.
    fn two_writers(
        &self,
        first: &[f32],
        first_sim: VectorSimilarityFunction,
        second: &[f32],
        second_sim: VectorSimilarityFunction,
        message: &str,
    ) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let w = new_writer(dir.as_directory())?;
            w.add_document(&float_doc("f", first, first_sim)?)?;
            w.close()?;
        }

        {
            let w2 = new_writer(dir.as_directory())?;
            let doc2 = float_doc("f", second, second_sim)?;
            let expected = expect_illegal_argument(|| w2.add_document(&doc2));
            Assert::assert_equals_str(None, Some(message), Some(&expected));
            w2.close()?;
        }
        dir.close()
    }

    /// `testIllegalDimChangeTwoWriters()`.
    fn test_illegal_dim_change_two_writers(&self) -> Result<()> {
        self.two_writers(
            &[0f32; 4],
            VectorSimilarityFunction::DOT_PRODUCT,
            &[0f32; 2],
            VectorSimilarityFunction::DOT_PRODUCT,
            "cannot change field \"f\" from vector dimension=4, vector encoding=FLOAT32, vector similarity function=DOT_PRODUCT to inconsistent vector dimension=2, vector encoding=FLOAT32, vector similarity function=DOT_PRODUCT",
        )
    }

    /// The `IndexWriterConfig` of the merging tests: `new
    /// IndexWriterConfig()`, the per-field format of `"field"` when the
    /// codec's format is a `PerFieldKnnVectorsFormat`, the scheduler and a
    /// `ForceMergePolicy`.
    fn merging_config(
        &self,
        merge_scheduler: Arc<TestMergeScheduler>,
    ) -> Result<IndexWriterConfig> {
        let mut iwc = IndexWriterConfig::new();
        let codec = self.get_codec();
        if let Some(per_field_knn_vectors_format) = codec
            .knn_vectors_format()
            .as_any()
            .and_then(|a| a.downcast_ref::<PerFieldKnnVectorsFormat>())
        {
            let format = per_field_knn_vectors_format.get_knn_vectors_format_for_field("field");
            iwc.set_codec(Arc::new(
                FilterCodec::new(codec.name().to_string(), Arc::clone(&codec))
                    .with_knn_vectors_format(SharedKnnFormat(format)),
            ))?;
        }
        iwc.set_merge_scheduler(merge_scheduler)?;
        let mp = iwc.merge_policy();
        iwc.set_merge_policy(Arc::new(ForceMergePolicy::new(mp)))?;
        Ok(iwc)
    }

    /// `testMergingWithDifferentKnnFields()`.
    fn test_merging_with_different_knn_fields(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let merge_scheduler = Arc::new(TestMergeScheduler::default());
        let iwc = self.merging_config(Arc::clone(&merge_scheduler))?;
        let writer = IndexWriter::new(dir.as_directory(), iwc)?;
        for i in 0..10 {
            let i = i as f32;
            let mut doc = Document::new();
            doc.add(Box::new(KnnFloatVectorField::with_euclidean(
                "field",
                &[i, i + 1.0, i + 2.0, i + 3.0],
            )?));
            writer.add_document(&doc)?;
        }
        writer.commit()?;
        for i in 0..10 {
            let i = i as f32;
            let mut doc = Document::new();
            doc.add(Box::new(KnnFloatVectorField::with_euclidean(
                "otherVector",
                &[i, i, i, i],
            )?));
            writer.add_document(&doc)?;
        }
        writer.commit()?;
        writer.force_merge(1)?;
        assert_null(
            merge_scheduler
                .ex
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .map(|e| format!("{e}")),
        );
        writer.close()?;
        dir.close()
    }

    /// `testMergingWithDifferentByteKnnFields()`.
    fn test_merging_with_different_byte_knn_fields(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let merge_scheduler = Arc::new(TestMergeScheduler::default());
        let iwc = self.merging_config(Arc::clone(&merge_scheduler))?;
        let writer = IndexWriter::new(dir.as_directory(), iwc)?;
        for i in 0..10u8 {
            let mut doc = Document::new();
            doc.add(Box::new(KnnByteVectorField::with_euclidean(
                "field",
                &[i, i, i, i],
            )?));
            writer.add_document(&doc)?;
        }
        writer.commit()?;
        for i in 0..10u8 {
            let mut doc = Document::new();
            doc.add(Box::new(KnnByteVectorField::with_euclidean(
                "otherVector",
                &[i, i, i, i],
            )?));
            writer.add_document(&doc)?;
        }
        writer.commit()?;
        writer.force_merge(1)?;
        assert_null(
            merge_scheduler
                .ex
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .map(|e| format!("{e}")),
        );
        writer.close()?;
        dir.close()
    }

    /// `testWriterRamEstimate()`.
    fn test_writer_ram_estimate(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let codec = rucene::codecs::get_default().expect("Codec.getDefault()");
        let state = ram_estimate_state(dir.as_directory())?;
        let format = codec.knn_vectors_format();
        let mut writer = format.fields_writer(&state)?;
        let outcome = (|| -> Result<()> {
            let ram_bytes_used = writer.ram_bytes_used();
            let mut dim = random().next_int_bound(64) + 1;
            if dim % 2 == 1 {
                dim += 1;
            }
            let num_docs = LuceneTestCase::at_least(100);
            let FieldVectorWriter::Float(mut field_writer) =
                writer.add_field(&ram_estimate_field(dim, VectorEncoding::FLOAT32))?
            else {
                Throwable::new("java.lang.ClassCastException", "not a float writer").throw()
            };
            for i in 0..num_docs {
                field_writer.add_value(i, random_vector(dim))?;
            }
            let ram_bytes_used2 = writer.ram_bytes_used();
            Assert::assert_true(ram_bytes_used2 > ram_bytes_used);
            Assert::assert_true(ram_bytes_used2 > i64::from(dim) * i64::from(num_docs) * 4);
            Ok(())
        })();
        let closed = writer.close();
        outcome?;
        closed?;
        dir.close()
    }

    /// `testWriterByteVectorRamEstimate()`.
    fn test_writer_byte_vector_ram_estimate(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let codec = rucene::codecs::get_default().expect("Codec.getDefault()");
        let state = ram_estimate_state(dir.as_directory())?;
        let format = codec.knn_vectors_format();
        let mut writer = format.fields_writer(&state)?;
        let outcome = (|| -> Result<()> {
            let mut dim = random().next_int_bound(64) + 1;
            if dim % 2 == 1 {
                dim += 1;
            }
            let num_docs = LuceneTestCase::at_least(100);
            let FieldVectorWriter::Byte(mut field_writer) =
                writer.add_field(&ram_estimate_field(dim, VectorEncoding::BYTE))?
            else {
                Throwable::new("java.lang.ClassCastException", "not a byte writer").throw()
            };
            for i in 0..num_docs {
                field_writer.add_value(i, random_vector8(dim))?;
            }
            // Validate the field-level RAM accounting uses correct byte sizes.
            // The reported RAM must be at least the raw byte vector data.
            let field_ram_bytes_used = field_writer.ram_bytes_used();
            let raw_byte_vector_data = i64::from(dim) * i64::from(num_docs);
            Assert::assert_true_msg(
                &format!(
                    "Expected field ramBytesUsed ({field_ram_bytes_used}) >= raw byte vector data size ({raw_byte_vector_data})"
                ),
                field_ram_bytes_used >= raw_byte_vector_data,
            );
            Ok(())
        })();
        let closed = writer.close();
        outcome?;
        closed?;
        dir.close()
    }

    /// `testIllegalSimilarityFunctionChangeTwoWriters()`.
    fn test_illegal_similarity_function_change_two_writers(&self) -> Result<()> {
        self.two_writers(
            &[0f32; 4],
            VectorSimilarityFunction::DOT_PRODUCT,
            &[0f32; 4],
            VectorSimilarityFunction::EUCLIDEAN,
            "cannot change field \"f\" from vector dimension=4, vector encoding=FLOAT32, vector similarity function=DOT_PRODUCT to inconsistent vector dimension=4, vector encoding=FLOAT32, vector similarity function=EUCLIDEAN",
        )
    }

    /// `testAddIndexesDirectory0()`.
    fn test_add_indexes_directory0(&self) -> Result<()> {
        let field_name = "field";
        let doc = float_doc(
            field_name,
            &[0f32; 4],
            VectorSimilarityFunction::DOT_PRODUCT,
        )?;
        let mut dir = LuceneTestCase::new_directory();
        let mut dir2 = LuceneTestCase::new_directory();
        {
            let w = new_writer(dir.as_directory())?;
            w.add_document(&doc)?;
            w.close()?;
        }
        {
            let w2 = new_writer(dir2.as_directory())?;
            w2.add_indexes_dirs(&[dir.as_directory()])?;
            w2.force_merge(1)?;
            let reader = directory_reader::open_with_writer(&*w2)?;
            let r = only_leaf(&reader)?;
            let vector_values = r.get_float_vector_values(field_name)?.expect("vectors");
            let mut iterator = vector_values.iterator()?;
            Assert::assert_equals(0i64, i64::from(iterator.next_doc()?));
            assert_float_equals(0.0, vector_values.vector_value(0)?[0], 0.0);
            Assert::assert_equals(i64::from(NO_MORE_DOCS), i64::from(iterator.next_doc()?));
            self.assert_off_heap_byte_size(&r, field_name)?;
            drop(r);
            reader.close()?;
            w2.close()?;
        }
        dir.close()?;
        dir2.close()
    }

    /// `testAddIndexesDirectory1()`.
    fn test_add_indexes_directory1(&self) -> Result<()> {
        let field_name = "field";
        let mut doc = Document::new();
        let mut dir = LuceneTestCase::new_directory();
        let mut dir2 = LuceneTestCase::new_directory();
        {
            let w = new_writer(dir.as_directory())?;
            w.add_document(&doc)?;
            w.close()?;
        }
        doc.add(Box::new(KnnFloatVectorField::new(
            field_name,
            &[0f32; 4],
            VectorSimilarityFunction::DOT_PRODUCT,
        )?));
        {
            let w2 = new_writer(dir2.as_directory())?;
            w2.add_document(&doc)?;
            w2.add_indexes_dirs(&[dir.as_directory()])?;
            w2.force_merge(1)?;
            let reader = directory_reader::open_with_writer(&*w2)?;
            let r = only_leaf(&reader)?;
            let vector_values = r.get_float_vector_values(field_name)?.expect("vectors");
            let mut iterator = vector_values.iterator()?;
            Assert::assert_true(iterator.next_doc()? != NO_MORE_DOCS);
            assert_float_equals(0.0, vector_values.vector_value(iterator.index())?[0], 0.0);
            Assert::assert_equals(i64::from(NO_MORE_DOCS), i64::from(iterator.next_doc()?));
            drop(r);
            reader.close()?;
            w2.close()?;
        }
        dir.close()?;
        dir2.close()
    }

    /// `testAddIndexesDirectory01()`.
    fn test_add_indexes_directory01(&self) -> Result<()> {
        let field_name = "field";
        let mut vector = [0f32; 2];
        let mut dir = LuceneTestCase::new_directory();
        let mut dir2 = LuceneTestCase::new_directory();
        {
            let w = new_writer(dir.as_directory())?;
            w.add_document(&float_doc(
                field_name,
                &vector,
                VectorSimilarityFunction::DOT_PRODUCT,
            )?)?;
            w.close()?;
        }
        {
            let w2 = new_writer(dir2.as_directory())?;
            vector[0] = 1.0;
            vector[1] = 1.0;
            // Java's document holds the array it was built over, which now
            // reads [1, 1].
            w2.add_document(&float_doc(
                field_name,
                &vector,
                VectorSimilarityFunction::DOT_PRODUCT,
            )?)?;
            w2.add_indexes_dirs(&[dir.as_directory()])?;
            w2.force_merge(1)?;
            let reader = directory_reader::open_with_writer(&*w2)?;
            let r = only_leaf(&reader)?;
            let vector_values = r.get_float_vector_values(field_name)?.expect("vectors");
            let mut iterator = vector_values.iterator()?;
            Assert::assert_equals(0i64, i64::from(iterator.next_doc()?));
            // The merge order is randomized, we might get 0 first, or 1
            let mut value = vector_values.vector_value(0)?[0];
            Assert::assert_true(value == 0.0 || value == 1.0);
            Assert::assert_equals(1i64, i64::from(iterator.next_doc()?));
            value += vector_values.vector_value(1)?[0];
            assert_float_equals(1.0, value, 0.0);
            drop(r);
            reader.close()?;
            w2.close()?;
        }
        dir.close()?;
        dir2.close()
    }

    /// The body of the add-indexes illegal-change tests: `dir` gets
    /// (`first`, `first_sim`), `dir2`'s writer (`second`, `second_sim`), and
    /// adding `dir` to it through `how` fails with `message`.
    #[allow(clippy::too_many_arguments)]
    fn illegal_change_via_add_indexes(
        &self,
        first: &[f32],
        first_sim: VectorSimilarityFunction,
        second: &[f32],
        second_sim: VectorSimilarityFunction,
        how: AddIndexesHow,
        message: &str,
    ) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let mut dir2 = LuceneTestCase::new_directory();
        {
            let w = new_writer(dir.as_directory())?;
            w.add_document(&float_doc("f", first, first_sim)?)?;
            w.close()?;
        }
        {
            let w2 = new_writer(dir2.as_directory())?;
            w2.add_document(&float_doc("f", second, second_sim)?)?;
            match how {
                AddIndexesHow::Directory => {
                    let expected =
                        expect_illegal_argument(|| w2.add_indexes_dirs(&[dir.as_directory()]));
                    Assert::assert_equals_str(None, Some(message), Some(&expected));
                }
                AddIndexesHow::CodecReader => {
                    let r = directory_reader::open(dir.as_directory())?;
                    let codec_reader = only_leaf(&r)?
                        .as_codec_reader()
                        .expect("a segment reader is a CodecReader");
                    let expected =
                        expect_illegal_argument(|| w2.add_indexes_codec_readers(&[codec_reader]));
                    Assert::assert_equals_str(None, Some(message), Some(&expected));
                    r.close()?;
                }
                AddIndexesHow::SlowCodecReader => {
                    let r = directory_reader::open(dir.as_directory())?;
                    let expected = expect_illegal_argument(|| {
                        TestUtil::add_indexes_slowly(&w2, &[Arc::clone(&r)])
                    });
                    Assert::assert_equals_str(None, Some(message), Some(&expected));
                    r.close()?;
                }
            }
            w2.close()?;
        }
        dir.close()?;
        dir2.close()
    }

    /// `testIllegalDimChangeViaAddIndexesDirectory()`.
    fn test_illegal_dim_change_via_add_indexes_directory(&self) -> Result<()> {
        self.illegal_change_via_add_indexes(
            &[0f32; 4],
            VectorSimilarityFunction::DOT_PRODUCT,
            &[0f32; 6],
            VectorSimilarityFunction::DOT_PRODUCT,
            AddIndexesHow::Directory,
            "cannot change field \"f\" from vector dimension=6, vector encoding=FLOAT32, vector similarity function=DOT_PRODUCT to inconsistent vector dimension=4, vector encoding=FLOAT32, vector similarity function=DOT_PRODUCT",
        )
    }

    /// `testIllegalSimilarityFunctionChangeViaAddIndexesDirectory()`.
    fn test_illegal_similarity_function_change_via_add_indexes_directory(&self) -> Result<()> {
        self.illegal_change_via_add_indexes(
            &[0f32; 4],
            VectorSimilarityFunction::DOT_PRODUCT,
            &[0f32; 4],
            VectorSimilarityFunction::EUCLIDEAN,
            AddIndexesHow::Directory,
            "cannot change field \"f\" from vector dimension=4, vector encoding=FLOAT32, vector similarity function=EUCLIDEAN to inconsistent vector dimension=4, vector encoding=FLOAT32, vector similarity function=DOT_PRODUCT",
        )
    }

    /// `testIllegalDimChangeViaAddIndexesCodecReader()`.
    fn test_illegal_dim_change_via_add_indexes_codec_reader(&self) -> Result<()> {
        self.illegal_change_via_add_indexes(
            &[0f32; 4],
            VectorSimilarityFunction::DOT_PRODUCT,
            &[0f32; 6],
            VectorSimilarityFunction::DOT_PRODUCT,
            AddIndexesHow::CodecReader,
            "cannot change field \"f\" from vector dimension=6, vector encoding=FLOAT32, vector similarity function=DOT_PRODUCT to inconsistent vector dimension=4, vector encoding=FLOAT32, vector similarity function=DOT_PRODUCT",
        )
    }

    /// `testIllegalSimilarityFunctionChangeViaAddIndexesCodecReader()`.
    fn test_illegal_similarity_function_change_via_add_indexes_codec_reader(&self) -> Result<()> {
        self.illegal_change_via_add_indexes(
            &[0f32; 4],
            VectorSimilarityFunction::DOT_PRODUCT,
            &[0f32; 4],
            VectorSimilarityFunction::EUCLIDEAN,
            AddIndexesHow::CodecReader,
            "cannot change field \"f\" from vector dimension=4, vector encoding=FLOAT32, vector similarity function=EUCLIDEAN to inconsistent vector dimension=4, vector encoding=FLOAT32, vector similarity function=DOT_PRODUCT",
        )
    }

    /// `testIllegalDimChangeViaAddIndexesSlowCodecReader()`.
    fn test_illegal_dim_change_via_add_indexes_slow_codec_reader(&self) -> Result<()> {
        self.illegal_change_via_add_indexes(
            &[0f32; 4],
            VectorSimilarityFunction::DOT_PRODUCT,
            &[0f32; 6],
            VectorSimilarityFunction::DOT_PRODUCT,
            AddIndexesHow::SlowCodecReader,
            "cannot change field \"f\" from vector dimension=6, vector encoding=FLOAT32, vector similarity function=DOT_PRODUCT to inconsistent vector dimension=4, vector encoding=FLOAT32, vector similarity function=DOT_PRODUCT",
        )
    }

    /// `testIllegalSimilarityFunctionChangeViaAddIndexesSlowCodecReader()`.
    fn test_illegal_similarity_function_change_via_add_indexes_slow_codec_reader(
        &self,
    ) -> Result<()> {
        self.illegal_change_via_add_indexes(
            &[0f32; 4],
            VectorSimilarityFunction::DOT_PRODUCT,
            &[0f32; 4],
            VectorSimilarityFunction::EUCLIDEAN,
            AddIndexesHow::SlowCodecReader,
            "cannot change field \"f\" from vector dimension=4, vector encoding=FLOAT32, vector similarity function=EUCLIDEAN to inconsistent vector dimension=4, vector encoding=FLOAT32, vector similarity function=DOT_PRODUCT",
        )
    }

    /// `testIllegalMultipleValues()`.
    fn test_illegal_multiple_values(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let w = new_writer(dir.as_directory())?;
        let mut doc = Document::new();
        doc.add(Box::new(KnnFloatVectorField::new(
            "f",
            &[0f32; 4],
            VectorSimilarityFunction::DOT_PRODUCT,
        )?));
        doc.add(Box::new(KnnFloatVectorField::new(
            "f",
            &[0f32; 4],
            VectorSimilarityFunction::DOT_PRODUCT,
        )?));
        let expected = expect_illegal_argument(|| w.add_document(&doc));
        Assert::assert_equals_str(
            None,
            Some("VectorValuesField \"f\" appears more than once in this document (only one value is allowed per field)"),
            Some(&expected),
        );
        w.close()?;
        dir.close()
    }

    /// `testIllegalDimensionTooLarge()`.
    fn test_illegal_dimension_too_large(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let w = new_writer(dir.as_directory())?;
        let max = self.get_vectors_max_dimensions("f");
        let too_large = vec![0f32; (max + 1) as usize];
        let doc = float_doc("f", &too_large, VectorSimilarityFunction::DOT_PRODUCT)?;
        let exc = expect_illegal_argument(|| w.add_document(&doc));
        Assert::assert_true(exc.contains(&format!("vector's dimensions must be <= [{max}]")));

        w.add_document(&float_doc(
            "f",
            &[0f32; 2],
            VectorSimilarityFunction::DOT_PRODUCT,
        )?)?;

        let doc3 = float_doc("f", &too_large, VectorSimilarityFunction::DOT_PRODUCT)?;
        let exc = expect_illegal_argument(|| w.add_document(&doc3));
        Assert::assert_true(
            exc.contains("Inconsistency of field data structures across documents for field [f]")
                || exc.contains(&format!("vector's dimensions must be <= [{max}]")),
        );
        w.flush()?;

        let doc4 = float_doc("f", &too_large, VectorSimilarityFunction::DOT_PRODUCT)?;
        let exc = expect_illegal_argument(|| w.add_document(&doc4));
        Assert::assert_true(exc.contains(&format!("vector's dimensions must be <= [{max}]")));
        w.close()?;
        dir.close()
    }

    /// `testIllegalEmptyVector()`.
    fn test_illegal_empty_vector(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let w = new_writer(dir.as_directory())?;
        let e = expect_illegal_argument(|| {
            KnnFloatVectorField::new("f", &[], VectorSimilarityFunction::EUCLIDEAN)
        });
        Assert::assert_equals_str(None, Some("cannot index an empty vector"), Some(&e));

        w.add_document(&float_doc(
            "f",
            &[0f32; 2],
            VectorSimilarityFunction::EUCLIDEAN,
        )?)?;
        w.close()?;
        dir.close()
    }

    /// `testDifferentCodecs1()`: write vectors, one segment with default
    /// codec, another with SimpleText, then forceMerge.
    fn test_different_codecs1(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let w = new_writer(dir.as_directory())?;
            w.add_document(&float_doc(
                "f",
                &[0f32; 4],
                VectorSimilarityFunction::DOT_PRODUCT,
            )?)?;
            w.close()?;
        }
        let mut iwc = LuceneTestCase::new_index_writer_config();
        iwc.set_codec(codec_for_name("SimpleText")?)?;
        {
            let w = IndexWriter::new(dir.as_directory(), iwc)?;
            w.add_document(&float_doc(
                "f",
                &[0f32; 4],
                VectorSimilarityFunction::DOT_PRODUCT,
            )?)?;
            w.force_merge(1)?;
            w.close()?;
        }
        dir.close()
    }

    /// `testDifferentCodecs2()`: write vectors, one segment with SimpleText,
    /// another with default codec, then forceMerge.
    fn test_different_codecs2(&self) -> Result<()> {
        let mut iwc = LuceneTestCase::new_index_writer_config();
        iwc.set_codec(codec_for_name("SimpleText")?)?;
        let mut dir = LuceneTestCase::new_directory();
        {
            let w = IndexWriter::new(dir.as_directory(), iwc)?;
            w.add_document(&float_doc(
                "f",
                &[0f32; 4],
                VectorSimilarityFunction::DOT_PRODUCT,
            )?)?;
            w.close()?;
        }
        {
            let w = new_writer(dir.as_directory())?;
            w.add_document(&float_doc(
                "f",
                &[0f32; 4],
                VectorSimilarityFunction::DOT_PRODUCT,
            )?)?;
            w.force_merge(1)?;
            w.close()?;
        }
        dir.close()
    }

    /// `testInvalidKnnVectorFieldUsage()` (see the module docs for
    /// `setIntValue`).
    fn test_invalid_knn_vector_field_usage(&self) -> Result<()> {
        let mut field =
            KnnFloatVectorField::new("field", &[0f32; 2], VectorSimilarityFunction::EUCLIDEAN)?;

        expect_illegal_argument(|| field.set_vector_value(&[0f32; 1]));

        Assert::assert_true(field.numeric_value().is_none());
        Ok(())
    }

    /// `testDeleteAllVectorDocs()`.
    fn test_delete_all_vector_docs(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let w = new_writer(dir.as_directory())?;
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            "0".to_string(),
            Store::NO,
        )?));
        doc.add(Box::new(KnnFloatVectorField::new(
            "v",
            &[2.0, 3.0, 5.0, 6.0],
            VectorSimilarityFunction::DOT_PRODUCT,
        )?));
        w.add_document(&doc)?;
        w.add_document(&Document::new())?;
        w.commit()?;

        {
            let r = directory_reader::open_with_writer(&*w)?;
            let values = only_leaf(&r)?.get_float_vector_values("v")?;
            assert_not_null(values.is_some());
            Assert::assert_equals(1i64, i64::from(values.expect("checked").size()));
            r.close()?;
        }
        w.delete_documents_by_terms(vec![Term::new("id", BytesRef::new(b"0".to_vec()))])?;
        w.force_merge(1)?;
        {
            let r = directory_reader::open_with_writer(&*w)?;
            let leaf_reader = only_leaf(&r)?;
            let values = leaf_reader.get_float_vector_values("v")?;
            assert_not_null(values.is_some());
            Assert::assert_equals(0i64, i64::from(values.expect("checked").size()));

            // assert that knn search doesn't fail on a field with all deleted docs
            let results = search_nearest(
                &*leaf_reader,
                "v",
                &random_normalized_vector(4),
                1,
                i32::MAX,
            )?;
            Assert::assert_equals(0i64, results.score_docs.len() as i64);
            drop(leaf_reader);
            r.close()?;
        }
        w.close()?;
        dir.close()
    }

    /// `testKnnVectorFieldMissingFromOneSegment()`.
    fn test_knn_vector_field_missing_from_one_segment(&self) -> Result<()> {
        let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(
            LuceneTestCase::create_temp_dir().as_std_path(),
        )?);
        let w = new_writer(Arc::clone(&dir))?;
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "id",
            "0".to_string(),
            Store::NO,
        )?));
        doc.add(Box::new(KnnFloatVectorField::new(
            "v0",
            &[2.0, 3.0, 5.0, 6.0],
            VectorSimilarityFunction::DOT_PRODUCT,
        )?));
        w.add_document(&doc)?;
        w.commit()?;

        let mut doc = Document::new();
        doc.add(Box::new(KnnFloatVectorField::new(
            "v1",
            &[2.0, 3.0, 5.0, 6.0],
            VectorSimilarityFunction::DOT_PRODUCT,
        )?));
        w.add_document(&doc)?;
        w.force_merge(1)?;
        w.close()
    }

    /// `testSparseVectors()`.
    fn test_sparse_vectors(&self) -> Result<()> {
        let num_docs = LuceneTestCase::at_least(1000);
        let num_fields = TestUtil::next_int(&*random(), 1, 10) as usize;
        let mut field_doc_counts = vec![0i32; num_fields];
        let mut field_totals = vec![0f64; num_fields];
        let mut field_dims = vec![0i32; num_fields];
        let mut field_similarity_functions = vec![VectorSimilarityFunction::EUCLIDEAN; num_fields];
        let mut field_vector_encodings = vec![VectorEncoding::FLOAT32; num_fields];
        for i in 0..num_fields {
            field_dims[i] = random().next_int_bound(20) + 1;
            if field_dims[i] % 2 != 0 {
                field_dims[i] += 1;
            }
            field_similarity_functions[i] = self.random_similarity();
            field_vector_encodings[i] = self.random_vector_encoding();
        }
        let mut dir = LuceneTestCase::new_directory();
        let w = RandomIndexWriter::new_with_config(
            &*random(),
            dir.as_directory(),
            LuceneTestCase::new_index_writer_config(),
        )?;
        for _ in 0..num_docs {
            let mut doc = Document::new();
            for field in 0..num_fields {
                let field_name = format!("int{field}");
                if random().next_int_bound(100) == 17 {
                    match field_vector_encodings[field] {
                        VectorEncoding::BYTE => {
                            let b = random_vector8(field_dims[field]);
                            doc.add(Box::new(KnnByteVectorField::new(
                                &field_name,
                                &b,
                                field_similarity_functions[field],
                            )?));
                            field_totals[field] += f64::from(b[0] as i8);
                        }
                        VectorEncoding::FLOAT32 => {
                            let v = random_normalized_vector(field_dims[field]);
                            doc.add(Box::new(KnnFloatVectorField::new(
                                &field_name,
                                &v,
                                field_similarity_functions[field],
                            )?));
                            field_totals[field] += f64::from(v[0]);
                        }
                    }
                    field_doc_counts[field] += 1;
                }
            }
            w.add_document(&doc)?;
        }
        let r = w.get_reader()?;
        for field in 0..num_fields {
            let mut doc_count = 0i32;
            let mut checksum = 0f64;
            let field_name = format!("int{field}");
            match field_vector_encodings[field] {
                VectorEncoding::BYTE => {
                    for ctx in Arc::clone(&r).leaves()? {
                        if let Some(byte_vector_values) =
                            ctx.leaf_reader().get_byte_vector_values(&field_name)?
                        {
                            doc_count += byte_vector_values.size();
                            let mut iterator = byte_vector_values.iterator()?;
                            while iterator.next_doc()? != NO_MORE_DOCS {
                                checksum += f64::from(
                                    byte_vector_values.vector_value(iterator.index())?[0] as i8,
                                );
                            }
                        }
                    }
                }
                VectorEncoding::FLOAT32 => {
                    for ctx in Arc::clone(&r).leaves()? {
                        if let Some(vector_values) =
                            ctx.leaf_reader().get_float_vector_values(&field_name)?
                        {
                            doc_count += vector_values.size();
                            let mut iterator = vector_values.iterator()?;
                            while iterator.next_doc()? != NO_MORE_DOCS {
                                checksum +=
                                    f64::from(vector_values.vector_value(iterator.index())?[0]);
                            }
                        }
                    }
                }
            }
            Assert::assert_equals(i64::from(field_doc_counts[field]), i64::from(doc_count));
            // Account for quantization done when indexing fields w/BYTE encoding
            let delta = if field_vector_encodings[field] == VectorEncoding::BYTE {
                f64::from(num_docs) * 0.01
            } else {
                1e-5
            };
            Assert::assert_equals_double_msg(None, field_totals[field], checksum, delta);
        }
        r.close()?;
        w.close()?;
        dir.close()
    }

    /// `testFloatVectorScorerIteration()`.
    fn test_float_vector_scorer_iteration(&self) -> Result<()> {
        let mut iwc = LuceneTestCase::new_index_writer_config();
        if random().next_boolean() {
            iwc.set_index_sort(sortkey_sort()?)?;
        }
        let field_name = "field";
        let similarity_function = self.knn_state().similarity_function();
        let mut dir = LuceneTestCase::new_directory();
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;
        let num_doc = LuceneTestCase::at_least(100);
        let mut dimension = LuceneTestCase::at_least(10);
        if dimension % 2 != 0 {
            dimension += 1;
        }
        let mut values: Vec<Option<Vec<f32>>> = vec![None; num_doc as usize];
        for i in 0..num_doc {
            if random().next_int_bound(7) != 3 {
                // usually index a vector value for a doc
                values[i as usize] = Some(random_normalized_vector(dimension));
            }
            add_float_random_key(
                &iw,
                field_name,
                i,
                values[i as usize].as_deref(),
                similarity_function,
            )?;
            if random().next_int_bound(10) == 2 {
                let id = random().next_int_bound(i + 1).to_string();
                iw.delete_documents_by_terms(vec![Term::new(
                    "id",
                    BytesRef::new(id.into_bytes()),
                )])?;
            }
            if random().next_int_bound(10) == 3 {
                iw.commit()?;
            }
        }
        let vector_to_score = random_normalized_vector(dimension);
        let reader = directory_reader::open_with_writer(&*iw)?;
        for ctx in Arc::clone(&reader).leaves()? {
            let Some(vector_values) = ctx.leaf_reader().get_float_vector_values(field_name)? else {
                continue;
            };
            if vector_values.size() == 0 {
                Assert::assert_true(vector_values.scorer(&vector_to_score)?.is_none());
                continue;
            }
            let scorer = vector_values.scorer(&vector_to_score)?;
            assert_not_null(scorer.is_some());
            let mut scorer = scorer.expect("checked");
            let first = scorer.iterator() as *mut dyn DocIdSetIterator as *const ();
            let second = scorer.iterator() as *mut dyn DocIdSetIterator as *const ();
            Assert::assert_true(std::ptr::eq(first, second));
            // verify scorer iteration scores are valid & iteration with vectorValues is consistent
            let mut values_iterator = vector_values.iterator()?;
            while scorer.iterator().next_doc()? != NO_MORE_DOCS {
                if values_iterator.next_doc()? == NO_MORE_DOCS {
                    break;
                }
                let score = scorer.score()?;
                Assert::assert_true(score >= 0.0);
                Assert::assert_equals(
                    i64::from(scorer.iterator().doc_id()),
                    i64::from(values_iterator.doc_id()),
                );
            }
            // verify that a new scorer can be obtained after iteration
            let new_scorer = vector_values.scorer(&vector_to_score)?;
            assert_not_null(new_scorer.is_some());
            let mut new_scorer = new_scorer.expect("checked");
            let old = scorer.iterator() as *mut dyn DocIdSetIterator as *const ();
            let new = new_scorer.iterator() as *mut dyn DocIdSetIterator as *const ();
            Assert::assert_false(std::ptr::eq(old, new));
        }
        reader.close()?;
        iw.close()?;
        dir.close()
    }

    /// `testByteVectorScorerIteration()`.
    fn test_byte_vector_scorer_iteration(&self) -> Result<()> {
        let mut iwc = LuceneTestCase::new_index_writer_config();
        if random().next_boolean() {
            iwc.set_index_sort(sortkey_sort()?)?;
        }
        let field_name = "field";
        let similarity_function = self.knn_state().similarity_function();
        let mut dir = LuceneTestCase::new_directory();
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;
        let num_doc = LuceneTestCase::at_least(100);
        let mut dimension = LuceneTestCase::at_least(10);
        if dimension % 2 != 0 {
            dimension += 1;
        }
        let mut values: Vec<Option<Vec<u8>>> = vec![None; num_doc as usize];
        for i in 0..num_doc {
            if random().next_int_bound(7) != 3 {
                // usually index a vector value for a doc
                values[i as usize] = Some(random_vector8(dimension));
            }
            add_byte_random_key(
                &iw,
                field_name,
                i,
                values[i as usize].as_deref(),
                similarity_function,
            )?;
            if random().next_int_bound(10) == 2 {
                let id = random().next_int_bound(i + 1).to_string();
                iw.delete_documents_by_terms(vec![Term::new(
                    "id",
                    BytesRef::new(id.into_bytes()),
                )])?;
            }
            if random().next_int_bound(10) == 3 {
                iw.commit()?;
            }
        }
        let vector_to_score = random_vector8(dimension);
        let reader = directory_reader::open_with_writer(&*iw)?;
        for ctx in Arc::clone(&reader).leaves()? {
            let Some(vector_values) = ctx.leaf_reader().get_byte_vector_values(field_name)? else {
                continue;
            };
            if vector_values.size() == 0 {
                Assert::assert_true(vector_values.scorer(&vector_to_score)?.is_none());
                continue;
            }
            let scorer = vector_values.scorer(&vector_to_score)?;
            assert_not_null(scorer.is_some());
            let mut scorer = scorer.expect("checked");
            let first = scorer.iterator() as *mut dyn DocIdSetIterator as *const ();
            let second = scorer.iterator() as *mut dyn DocIdSetIterator as *const ();
            Assert::assert_true(std::ptr::eq(first, second));
            // verify scorer iteration scores are valid & iteration with vectorValues is consistent
            let mut values_iterator = vector_values.iterator()?;
            while scorer.iterator().next_doc()? != NO_MORE_DOCS {
                if values_iterator.next_doc()? == NO_MORE_DOCS {
                    break;
                }
                let score = scorer.score()?;
                Assert::assert_true(score >= 0.0);
                Assert::assert_equals(
                    i64::from(scorer.iterator().doc_id()),
                    i64::from(values_iterator.doc_id()),
                );
            }
            // verify that a new scorer can be obtained after iteration
            let new_scorer = vector_values.scorer(&vector_to_score)?;
            assert_not_null(new_scorer.is_some());
            let mut new_scorer = new_scorer.expect("checked");
            let old = scorer.iterator() as *mut dyn DocIdSetIterator as *const ();
            let new = new_scorer.iterator() as *mut dyn DocIdSetIterator as *const ();
            Assert::assert_false(std::ptr::eq(old, new));
        }
        reader.close()?;
        iw.close()?;
        dir.close()
    }

    /// The body of `testEmptyFloatVectorData` and `testEmptyByteVectorData`.
    fn empty_vector_data(&self, bytes: bool) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let w = new_writer(dir.as_directory())?;
        let mut doc1 = Document::new();
        doc1.add(Box::new(StringField::new(
            "id",
            "0".to_string(),
            Store::NO,
        )?));
        if bytes {
            doc1.add(Box::new(KnnByteVectorField::new(
                "v",
                &[2, 3, 5, 6],
                VectorSimilarityFunction::DOT_PRODUCT,
            )?));
        } else {
            doc1.add(Box::new(KnnFloatVectorField::new(
                "v",
                &[2.0, 3.0, 5.0, 6.0],
                VectorSimilarityFunction::DOT_PRODUCT,
            )?));
        }
        w.add_document(&doc1)?;

        let mut doc2 = Document::new();
        doc2.add(Box::new(StringField::new(
            "id",
            "1".to_string(),
            Store::NO,
        )?));
        w.add_document(&doc2)?;

        w.delete_documents_by_terms(vec![Term::new("id", BytesRef::new(b"0".to_vec()))])?;
        w.commit()?;
        w.force_merge(1)?;

        let reader = directory_reader::open_with_writer(&*w)?;
        let r = only_leaf(&reader)?;
        if bytes {
            let values = r.get_byte_vector_values("v")?;
            assert_not_null(values.is_some());
            let values = values.expect("checked");
            Assert::assert_equals(0i64, i64::from(values.size()));
            Assert::assert_true(values.scorer(&[2, 3, 5, 6])?.is_none());
        } else {
            let values = r.get_float_vector_values("v")?;
            assert_not_null(values.is_some());
            let values = values.expect("checked");
            Assert::assert_equals(0i64, i64::from(values.size()));
            Assert::assert_true(values.scorer(&[2.0, 3.0, 5.0, 6.0])?.is_none());
        }
        self.assert_off_heap_byte_size(&r, "v")?;
        drop(r);
        reader.close()?;
        w.close()?;
        dir.close()
    }

    /// `testEmptyFloatVectorData()`.
    fn test_empty_float_vector_data(&self) -> Result<()> {
        self.empty_vector_data(false)
    }

    /// `testEmptyByteVectorData()`.
    fn test_empty_byte_vector_data(&self) -> Result<()> {
        self.empty_vector_data(true)
    }

    /// `testIndexedValueNotAliased()`: we copy indexed values (as for
    /// BinaryDocValues) so the input float[] can be reused across calls to
    /// IndexWriter.addDocument.
    fn test_indexed_value_not_aliased(&self) -> Result<()> {
        let field_name = "field";
        let mut v = [0f32, 0.0];
        let mut dir = LuceneTestCase::new_directory();
        let mut iwc = LuceneTestCase::new_index_writer_config();
        iwc.set_merge_policy(Arc::new(NoMergePolicy::new()))?;
        iwc.set_max_buffered_docs(3)?;
        iwc.set_ram_buffer_size_mb(-1.0)?;
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;
        // Java's `doc1` and `doc2` hold the same array, which reads [1, 0]
        // when they are added.
        v[0] = 1.0;
        let doc1 = float_doc(field_name, &v, VectorSimilarityFunction::EUCLIDEAN)?;
        let doc2 = float_doc(field_name, &v, VectorSimilarityFunction::EUCLIDEAN)?;
        iw.add_document(&doc1)?;
        iw.add_document(&doc2)?;
        v[0] = 2.0;
        let doc3 = float_doc(field_name, &v, VectorSimilarityFunction::EUCLIDEAN)?;
        iw.add_document(&doc3)?;
        iw.force_merge(1)?;
        let reader = directory_reader::open_with_writer(&*iw)?;
        let r = only_leaf(&reader)?;
        let vector_values = r.get_float_vector_values(field_name)?.expect("vectors");
        Assert::assert_equals(3i64, i64::from(vector_values.size()));
        let mut iterator = vector_values.iterator()?;
        iterator.next_doc()?;
        Assert::assert_equals(0i64, i64::from(iterator.index()));
        assert_float_equals(1.0, vector_values.vector_value(0)?[0], 0.0);
        iterator.next_doc()?;
        Assert::assert_equals(1i64, i64::from(iterator.index()));
        assert_float_equals(1.0, vector_values.vector_value(1)?[0], 0.0);
        iterator.next_doc()?;
        Assert::assert_equals(2i64, i64::from(iterator.index()));
        assert_float_equals(2.0, vector_values.vector_value(2)?[0], 0.0);
        drop(r);
        reader.close()?;
        iw.close()?;
        dir.close()
    }

    /// `testSortedIndex()`.
    fn test_sorted_index(&self) -> Result<()> {
        let mut iwc = LuceneTestCase::new_index_writer_config();
        iwc.set_index_sort(sortkey_sort()?)?;
        let field_name = "field";
        let mut dir = LuceneTestCase::new_directory();
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;
        let euclidean = VectorSimilarityFunction::EUCLIDEAN;
        add_float(&iw, field_name, 1, 1, Some(&[-1.0, 0.0]), euclidean)?;
        add_float(&iw, field_name, 4, 4, Some(&[0.0, 1.0]), euclidean)?;
        add_float(&iw, field_name, 3, 3, None, euclidean)?;
        add_float(&iw, field_name, 2, 2, Some(&[1.0, 0.0]), euclidean)?;
        iw.force_merge(1)?;
        let reader = directory_reader::open_with_writer(&*iw)?;
        let leaf = only_leaf(&reader)?;

        let vector_values = leaf.get_float_vector_values(field_name)?.expect("vectors");
        Assert::assert_equals(2i64, i64::from(vector_values.dimension()));
        Assert::assert_equals(3i64, i64::from(vector_values.size()));
        let mut iterator = vector_values.iterator()?;
        Assert::assert_equals_str(
            None,
            Some("1"),
            stored_get(&*leaf, iterator.next_doc()?, "id")?.as_deref(),
        );
        assert_float_equals(-1.0, vector_values.vector_value(0)?[0], 0.0);
        Assert::assert_equals_str(
            None,
            Some("2"),
            stored_get(&*leaf, iterator.next_doc()?, "id")?.as_deref(),
        );
        assert_float_equals(1.0, vector_values.vector_value(1)?[0], 0.0);
        Assert::assert_equals_str(
            None,
            Some("4"),
            stored_get(&*leaf, iterator.next_doc()?, "id")?.as_deref(),
        );
        assert_float_equals(0.0, vector_values.vector_value(2)?[0], 0.0);
        Assert::assert_equals(i64::from(NO_MORE_DOCS), i64::from(iterator.next_doc()?));
        drop(leaf);
        reader.close()?;
        iw.close()?;
        dir.close()
    }

    /// `testSortedIndexBytes()`.
    fn test_sorted_index_bytes(&self) -> Result<()> {
        let mut iwc = LuceneTestCase::new_index_writer_config();
        iwc.set_index_sort(sortkey_sort()?)?;
        let field_name = "field";
        let mut dir = LuceneTestCase::new_directory();
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;
        let euclidean = VectorSimilarityFunction::EUCLIDEAN;
        add_byte(&iw, field_name, 1, 1, Some(&[(-1i8) as u8, 0]), euclidean)?;
        add_byte(&iw, field_name, 4, 4, Some(&[0, 1]), euclidean)?;
        add_byte(&iw, field_name, 3, 3, None, euclidean)?;
        add_byte(&iw, field_name, 2, 2, Some(&[1, 0]), euclidean)?;
        iw.force_merge(1)?;
        let reader = directory_reader::open_with_writer(&*iw)?;
        let leaf = only_leaf(&reader)?;

        let vector_values = leaf.get_byte_vector_values(field_name)?.expect("vectors");
        Assert::assert_equals(2i64, i64::from(vector_values.dimension()));
        Assert::assert_equals(3i64, i64::from(vector_values.size()));
        Assert::assert_equals_str(
            None,
            Some("1"),
            stored_get(&*leaf, vector_values.iterator()?.next_doc()?, "id")?.as_deref(),
        );
        assert_float_equals(
            -1.0,
            f32::from(vector_values.vector_value(0)?[0] as i8),
            0.0,
        );
        Assert::assert_equals_str(
            None,
            Some("2"),
            stored_get(&*leaf, vector_values.iterator()?.next_doc()?, "id")?.as_deref(),
        );
        assert_float_equals(1.0, f32::from(vector_values.vector_value(1)?[0] as i8), 0.0);
        Assert::assert_equals_str(
            None,
            Some("4"),
            stored_get(&*leaf, vector_values.iterator()?.next_doc()?, "id")?.as_deref(),
        );
        assert_float_equals(0.0, f32::from(vector_values.vector_value(2)?[0] as i8), 0.0);
        Assert::assert_equals(
            i64::from(NO_MORE_DOCS),
            i64::from(vector_values.iterator()?.next_doc()?),
        );
        drop(leaf);
        reader.close()?;
        iw.close()?;
        dir.close()
    }

    /// `testIndexMultipleKnnVectorFields()`.
    fn test_index_multiple_knn_vector_fields(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let mut iwc = LuceneTestCase::new_index_writer_config();
        iwc.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;
        let mut v = [1.0f32, 2.0];
        let doc_with = |v: &[f32]| -> Result<Document> {
            let mut doc = Document::new();
            doc.add(Box::new(KnnFloatVectorField::new(
                "field1",
                v,
                VectorSimilarityFunction::EUCLIDEAN,
            )?));
            doc.add(Box::new(KnnFloatVectorField::new(
                "field2",
                &[1.0, 2.0, 3.0, 4.0],
                VectorSimilarityFunction::EUCLIDEAN,
            )?));
            Ok(doc)
        };
        iw.add_document(&doc_with(&v)?)?;
        v[0] = 2.0;
        iw.add_document(&doc_with(&v)?)?;
        let mut doc = Document::new();
        doc.add(Box::new(KnnFloatVectorField::new(
            "field3",
            &[1.0, 2.0, 3.0, 4.0],
            VectorSimilarityFunction::DOT_PRODUCT,
        )?));
        iw.add_document(&doc)?;
        iw.force_merge(1)?;
        let reader = directory_reader::open_with_writer(&*iw)?;
        let leaf = Arc::clone(&reader).leaves()?[0].leaf_reader();

        let vector_values = leaf.get_float_vector_values("field1")?.expect("field1");
        Assert::assert_equals(2i64, i64::from(vector_values.dimension()));
        Assert::assert_equals(2i64, i64::from(vector_values.size()));
        let mut iterator = vector_values.iterator()?;
        iterator.next_doc()?;
        assert_float_equals(1.0, vector_values.vector_value(0)?[0], 0.0);
        iterator.next_doc()?;
        assert_float_equals(2.0, vector_values.vector_value(1)?[0], 0.0);
        Assert::assert_equals(i64::from(NO_MORE_DOCS), i64::from(iterator.next_doc()?));

        let vector_values2 = leaf.get_float_vector_values("field2")?.expect("field2");
        let mut it2 = vector_values2.iterator()?;
        Assert::assert_equals(4i64, i64::from(vector_values2.dimension()));
        Assert::assert_equals(2i64, i64::from(vector_values2.size()));
        it2.next_doc()?;
        assert_float_equals(2.0, vector_values2.vector_value(0)?[1], 0.0);
        it2.next_doc()?;
        assert_float_equals(2.0, vector_values2.vector_value(1)?[1], 0.0);
        Assert::assert_equals(i64::from(NO_MORE_DOCS), i64::from(it2.next_doc()?));

        let vector_values3 = leaf.get_float_vector_values("field3")?.expect("field3");
        Assert::assert_equals(4i64, i64::from(vector_values3.dimension()));
        Assert::assert_equals(1i64, i64::from(vector_values3.size()));
        let mut it3 = vector_values3.iterator()?;
        it3.next_doc()?;
        Assert::assert_equals_double_msg(
            None,
            1.0,
            f64::from(vector_values3.vector_value(0)?[0]),
            0.1,
        );
        Assert::assert_equals(i64::from(NO_MORE_DOCS), i64::from(it3.next_doc()?));
        drop(leaf);
        reader.close()?;
        iw.close()?;
        dir.close()
    }

    /// `testRandom()`: index random vectors, sometimes skipping documents,
    /// sometimes deleting a document, sometimes merging, sometimes sorting
    /// the index, and verify that the expected values can be read back
    /// consistently.
    fn test_random(&self) -> Result<()> {
        let mut iwc = LuceneTestCase::new_index_writer_config();
        if random().next_boolean() {
            iwc.set_index_sort(sortkey_sort()?)?;
        }
        let field_name = "field";
        let similarity_function = self.knn_state().similarity_function();
        let mut dir = LuceneTestCase::new_directory();
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;
        let num_doc = LuceneTestCase::at_least(100);
        let mut dimension = LuceneTestCase::at_least(10);
        if dimension % 2 != 0 {
            dimension += 1;
        }
        let mut scratch = vec![0f32; dimension as usize];
        let mut num_values = 0;
        let mut values: Vec<Option<Vec<f32>>> = vec![None; num_doc as usize];
        for i in 0..num_doc {
            if random().next_int_bound(7) != 3 {
                // usually index a vector value for a doc
                values[i as usize] = Some(random_normalized_vector(dimension));
                num_values += 1;
            }
            if random().next_boolean() && values[i as usize].is_some() {
                // sometimes use a shared scratch array
                scratch.copy_from_slice(values[i as usize].as_ref().expect("checked"));
                add_float_random_key(&iw, field_name, i, Some(&scratch), similarity_function)?;
            } else {
                add_float_random_key(
                    &iw,
                    field_name,
                    i,
                    values[i as usize].as_deref(),
                    similarity_function,
                )?;
            }
            if random().next_int_bound(10) == 2 {
                // sometimes delete a random document
                let id_to_delete = random().next_int_bound(i + 1);
                iw.delete_documents_by_terms(vec![Term::new(
                    "id",
                    BytesRef::new(id_to_delete.to_string().into_bytes()),
                )])?;
                // and remember that it was deleted
                if values[id_to_delete as usize].is_some() {
                    values[id_to_delete as usize] = None;
                    num_values -= 1;
                }
            }
            if random().next_int_bound(10) == 3 {
                iw.commit()?;
            }
        }
        let mut num_deletes = 0;
        let reader = directory_reader::open_with_writer(&*iw)?;
        let mut value_count = 0;
        let mut total_size = 0;
        for ctx in Arc::clone(&reader).leaves()? {
            let leaf = ctx.leaf_reader();
            let Some(vector_values) = leaf.get_float_vector_values(field_name)? else {
                continue;
            };
            total_size += vector_values.size();
            let mut iterator = vector_values.iterator()?;
            loop {
                let doc_id = iterator.next_doc()?;
                if doc_id == NO_MORE_DOCS {
                    break;
                }
                let v = vector_values.vector_value(iterator.index())?;
                Assert::assert_equals(i64::from(dimension), v.len() as i64);
                let id_string = stored_get(&*leaf, doc_id, "id")?.expect("id is stored");
                let id = parse_int(&id_string);
                let live_docs = leaf.get_live_docs();
                if live_docs.as_ref().is_none_or(|l| l.get(doc_id as usize)) {
                    assert_float_array_equals(
                        &format!("{id_string} {doc_id}"),
                        values[id as usize].as_deref(),
                        &v,
                        0.0,
                    );
                    value_count += 1;
                } else {
                    num_deletes += 1;
                    assert_null(values[id as usize].as_ref().map(|v| format!("{v:?}")));
                }
            }
        }
        Assert::assert_equals(i64::from(num_values), i64::from(value_count));
        Assert::assert_equals(i64::from(num_values), i64::from(total_size - num_deletes));
        reader.close()?;
        iw.close()?;
        dir.close()
    }

    /// `testRandomBytes()`: index random vectors as bytes, sometimes skipping
    /// documents, sometimes deleting a document, sometimes merging,
    /// sometimes sorting the index, and verify that the expected values can
    /// be read back consistently.
    fn test_random_bytes(&self) -> Result<()> {
        let mut iwc = LuceneTestCase::new_index_writer_config();
        if random().next_boolean() {
            iwc.set_index_sort(sortkey_sort()?)?;
        }
        let field_name = "field";
        let similarity_function = self.knn_state().similarity_function();
        let mut dir = LuceneTestCase::new_directory();
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;
        let num_doc = LuceneTestCase::at_least(100);
        let mut dimension = LuceneTestCase::at_least(10);
        if dimension % 2 != 0 {
            dimension += 1;
        }
        let mut scratch = vec![0u8; dimension as usize];
        let mut num_values = 0;
        let mut values: Vec<Option<Vec<u8>>> = vec![None; num_doc as usize];
        for i in 0..num_doc {
            if random().next_int_bound(7) != 3 {
                // usually index a vector value for a doc
                values[i as usize] = Some(random_vector8(dimension));
                num_values += 1;
            }
            if random().next_boolean() && values[i as usize].is_some() {
                // sometimes use a shared scratch array
                scratch.copy_from_slice(values[i as usize].as_ref().expect("checked"));
                add_byte_random_key(&iw, field_name, i, Some(&scratch), similarity_function)?;
            } else {
                add_byte_random_key(
                    &iw,
                    field_name,
                    i,
                    values[i as usize].as_deref(),
                    similarity_function,
                )?;
            }
            if random().next_int_bound(10) == 2 {
                // sometimes delete a random document
                let id_to_delete = random().next_int_bound(i + 1);
                iw.delete_documents_by_terms(vec![Term::new(
                    "id",
                    BytesRef::new(id_to_delete.to_string().into_bytes()),
                )])?;
                // and remember that it was deleted
                if values[id_to_delete as usize].is_some() {
                    values[id_to_delete as usize] = None;
                    num_values -= 1;
                }
            }
            if random().next_int_bound(10) == 3 {
                iw.commit()?;
            }
        }
        let mut num_deletes = 0;
        let reader = directory_reader::open_with_writer(&*iw)?;
        let mut value_count = 0;
        let mut total_size = 0;
        for ctx in Arc::clone(&reader).leaves()? {
            let leaf = ctx.leaf_reader();
            let Some(vector_values) = leaf.get_byte_vector_values(field_name)? else {
                continue;
            };
            total_size += vector_values.size();
            let mut iterator = vector_values.iterator()?;
            loop {
                let doc_id = iterator.next_doc()?;
                if doc_id == NO_MORE_DOCS {
                    break;
                }
                let v = vector_values.vector_value(iterator.index())?;
                Assert::assert_equals(i64::from(dimension), v.len() as i64);
                let id_string = stored_get(&*leaf, doc_id, "id")?.expect("id is stored");
                let id = parse_int(&id_string);
                let live_docs = leaf.get_live_docs();
                if live_docs.as_ref().is_none_or(|l| l.get(doc_id as usize)) {
                    // `values[id].compareTo(new BytesRef(v))`: a `null`
                    // `values[id]` is Java's NullPointerException.
                    let expected = values[id as usize].as_ref().unwrap_or_else(|| {
                        randomizedtesting::jdk::exception::throw_without_message(
                            "java.lang.NullPointerException",
                        )
                    });
                    Assert::assert_equals_msg(
                        Some(&id_string),
                        0i64,
                        i64::from(expected.as_slice().cmp(v.as_slice()) as i32),
                    );
                    value_count += 1;
                } else {
                    num_deletes += 1;
                    assert_null(values[id as usize].as_ref().map(|v| format!("{v:?}")));
                }
            }
        }
        Assert::assert_equals(i64::from(num_values), i64::from(value_count));
        Assert::assert_equals(i64::from(num_values), i64::from(total_size - num_deletes));
        reader.close()?;
        iw.close()?;
        dir.close()
    }

    /// `testSearchWithVisitedLimit()`: tests whether `KnnVectorsReader#search`
    /// implementations obey the limit on the number of visited vectors.
    /// This test is a best-effort attempt to capture the right behavior, and
    /// isn't meant to define a strict requirement on behavior.
    fn test_search_with_visited_limit(&self) -> Result<()> {
        let iwc = LuceneTestCase::new_index_writer_config();
        let field_name = "field";
        let mut dir = LuceneTestCase::new_directory();
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;
        let num_doc = 300;
        let dimension = 10;
        for i in 0..num_doc {
            let value = if random().next_int_bound(7) != 3 {
                // usually index a vector value for a doc
                Some(random_normalized_vector(dimension))
            } else {
                None
            };
            add_float_random_key(
                &iw,
                field_name,
                i,
                value.as_deref(),
                VectorSimilarityFunction::EUCLIDEAN,
            )?;
        }
        iw.force_merge(1)?;

        // randomly delete some documents
        for _ in 0..30 {
            let id_to_delete = random().next_int_bound(num_doc);
            iw.delete_documents_by_terms(vec![Term::new(
                "id",
                BytesRef::new(id_to_delete.to_string().into_bytes()),
            )])?;
        }

        let reader = directory_reader::open_with_writer(&*iw)?;
        for ctx in Arc::clone(&reader).leaves()? {
            let leaf = ctx.leaf_reader();
            let Some(vector_values) = leaf.get_float_vector_values(field_name)? else {
                continue;
            };

            // check the limit is hit when it's very small
            let k = 5 + random().next_int_bound(45);
            let visited_limit = k + random().next_int_bound(5);
            let results = search_nearest(
                &*leaf,
                field_name,
                &random_normalized_vector(dimension),
                k,
                visited_limit,
            )?;
            Assert::assert_true(
                results.total_hits.relation() == TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO,
            );
            let size = EXHAUSTIVE_BULK_SCORE_ORDS as i64;
            // visit limit is a "best effort" limit given our bulk scoring logic; assert that we
            // are within reasonable bounds
            Assert::assert_true(
                results.total_hits.value() == i64::from(visited_limit)
                    || results.total_hits.value()
                        <= ((i64::from(visited_limit) + size - 1) / size) * size,
            );

            // check the limit is not hit when it clearly exceeds the number of vectors
            let k = vector_values.size();
            let visited_limit = k + 30;
            let results = search_nearest(
                &*leaf,
                field_name,
                &random_normalized_vector(dimension),
                k,
                visited_limit,
            )?;
            Assert::assert_true(results.total_hits.relation() == TotalHitsRelation::EQUAL_TO);
            Assert::assert_true(results.total_hits.value() <= i64::from(visited_limit));
            self.assert_off_heap_byte_size(&leaf, field_name)?;
        }
        reader.close()?;
        iw.close()?;
        dir.close()
    }

    /// `testRandomWithUpdatesAndGraph()`: index random vectors, sometimes
    /// skipping documents, sometimes updating a document, sometimes merging,
    /// sometimes sorting the index, using an HNSW similarity function to
    /// also produce a graph, and verify that the expected values can be read
    /// back consistently.
    fn test_random_with_updates_and_graph(&self) -> Result<()> {
        let iwc = LuceneTestCase::new_index_writer_config();
        let field_name = "field";
        let mut dir = LuceneTestCase::new_directory();
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;
        let num_doc = LuceneTestCase::at_least(100);
        let mut dimension = LuceneTestCase::at_least(10);
        if dimension % 2 != 0 {
            dimension += 1;
        }
        let mut id2value: Vec<Option<Vec<f32>>> = vec![None; num_doc as usize];
        for _ in 0..num_doc {
            let id = random().next_int_bound(num_doc);
            let value = if random().next_int_bound(7) != 3 {
                // usually index a vector value for a doc
                Some(random_normalized_vector(dimension))
            } else {
                None
            };
            id2value[id as usize] = value.clone();
            add_float_random_key(
                &iw,
                field_name,
                id,
                value.as_deref(),
                VectorSimilarityFunction::EUCLIDEAN,
            )?;
        }
        let reader = directory_reader::open_with_writer(&*iw)?;
        for ctx in Arc::clone(&reader).leaves()? {
            let leaf = ctx.leaf_reader();
            let live_docs = leaf.get_live_docs();
            let Some(vector_values) = leaf.get_float_vector_values(field_name)? else {
                continue;
            };
            let mut num_live_docs_with_vectors = 0;
            let mut iterator = vector_values.iterator()?;
            loop {
                let doc_id = iterator.next_doc()?;
                if doc_id == NO_MORE_DOCS {
                    break;
                }
                let v = vector_values.vector_value(iterator.index())?;
                Assert::assert_equals(i64::from(dimension), v.len() as i64);
                let id_string = stored_get(&*leaf, doc_id, "id")?.expect("id is stored");
                let id = parse_int(&id_string);
                if live_docs.as_ref().is_none_or(|l| l.get(doc_id as usize)) {
                    assert_float_array_equals(
                        &format!(
                            "values differ for id={id_string}, docid={doc_id} leaf={}",
                            ctx.ord()
                        ),
                        id2value[id as usize].as_deref(),
                        &v,
                        0.0,
                    );
                    num_live_docs_with_vectors += 1;
                } else if let Some(expected) = &id2value[id as usize] {
                    Assert::assert_false(*expected == v);
                }
            }

            if num_live_docs_with_vectors == 0 {
                continue;
            }

            // assert that searchNearestVectors returns the expected number of documents,
            // in descending score order
            let size = leaf
                .get_float_vector_values(field_name)?
                .expect("vectors")
                .size();
            let mut k = random().next_int_bound(size / 10 + 1) + 1;
            if k > num_live_docs_with_vectors {
                k = num_live_docs_with_vectors;
            }
            let results = search_nearest(
                &*leaf,
                field_name,
                &random_normalized_vector(dimension),
                k,
                i32::MAX,
            )?;
            Assert::assert_equals(i64::from(k.min(size)), results.score_docs.len() as i64);
            for i in 0..(k - 1) as usize {
                Assert::assert_true(results.score_docs[i].score >= results.score_docs[i + 1].score);
            }
            self.assert_off_heap_byte_size(&leaf, field_name)?;
        }
        reader.close()?;
        iw.close()?;
        dir.close()
    }

    /// `testCheckIndexIncludesVectors()`.
    fn test_check_index_includes_vectors(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let w = new_writer(dir.as_directory())?;
            let mut doc = Document::new();
            doc.add(Box::new(KnnFloatVectorField::new(
                "v1",
                &random_normalized_vector(4),
                VectorSimilarityFunction::EUCLIDEAN,
            )?));
            w.add_document(&doc)?;

            doc.add(Box::new(KnnFloatVectorField::new(
                "v2",
                &random_normalized_vector(4),
                VectorSimilarityFunction::EUCLIDEAN,
            )?));
            w.add_document(&doc)?;
            w.close()?;
        }

        let mut output = Vec::new();
        let status = TestUtil::check_index_with(
            dir.as_directory(),
            Level::MIN_LEVEL_FOR_INTEGRITY_CHECKS,
            true,
            true,
            Some(&mut output),
        )?;
        Assert::assert_equals(1i64, status.segment_infos.len() as i64);
        let seg_status = &status.segment_infos[0];
        let vector_values_status = seg_status
            .vector_values_status
            .as_ref()
            .expect("vectors were checked");
        // total 3 vector values were indexed:
        Assert::assert_equals(3i64, vector_values_status.total_vector_values);
        // ... across 2 fields:
        Assert::assert_equals(
            2i64,
            i64::from(vector_values_status.total_knn_vector_fields),
        );

        // Make sure CheckIndex in fact declares that it is testing vectors!
        Assert::assert_true(String::from_utf8_lossy(&output).contains("test: vectors..."));
        dir.close()
    }

    /// `testSimilarityFunctionIdentifiers()`: make sure we don't
    /// accidentally mess up similarity function identifiers by re-ordering
    /// their enumerators.
    fn test_similarity_function_identifiers(&self) -> Result<()> {
        Assert::assert_equals(0i64, VectorSimilarityFunction::EUCLIDEAN as i64);
        Assert::assert_equals(1i64, VectorSimilarityFunction::DOT_PRODUCT as i64);
        Assert::assert_equals(2i64, VectorSimilarityFunction::COSINE as i64);
        Assert::assert_equals(3i64, VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT as i64);
        Assert::assert_equals(4i64, VECTOR_SIMILARITY_FUNCTION_VALUES.len() as i64);
        Ok(())
    }

    /// `testVectorEncodingOrdinals()`: make sure we don't accidentally mess up
    /// vector encoding identifiers by re-ordering their enumerators.
    fn test_vector_encoding_ordinals(&self) -> Result<()> {
        Assert::assert_equals(0i64, VectorEncoding::BYTE as i64);
        Assert::assert_equals(1i64, VectorEncoding::FLOAT32 as i64);
        Assert::assert_equals(2i64, VECTOR_ENCODING_VALUES.len() as i64);
        Ok(())
    }

    /// `testAdvance()`.
    fn test_advance(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        let w = new_writer(dir.as_directory())?;
        let numdocs = LuceneTestCase::at_least(1500);
        let field_name = "field";
        for _ in 0..numdocs {
            let mut doc = Document::new();
            // randomly add a vector field
            if random().next_int_bound(4) == 3 {
                doc.add(Box::new(KnnFloatVectorField::new(
                    field_name,
                    &[0f32; 4],
                    VectorSimilarityFunction::EUCLIDEAN,
                )?));
            }
            w.add_document(&doc)?;
        }
        w.force_merge(1)?;
        let reader = directory_reader::open_with_writer(&*w)?;
        let r = only_leaf(&reader)?;
        let vector_values = r.get_float_vector_values(field_name)?.expect("vectors");
        let mut vector_docs = vec![0i32; vector_values.size() as usize + 1];
        let mut cur: i32 = -1;
        let mut iterator = vector_values.iterator()?;
        loop {
            cur += 1;
            if cur >= vector_values.size() + 1 {
                break;
            }
            vector_docs[cur as usize] = iterator.next_doc()?;
            if cur != 0 {
                Assert::assert_true(vector_docs[cur as usize] > vector_docs[cur as usize - 1]);
            }
        }
        let vector_values = r.get_float_vector_values(field_name)?.expect("vectors");
        let mut iter = vector_values.iterator()?;
        cur = -1;
        let mut i = 0;
        while i < numdocs {
            // randomly advance to i
            if random().next_int_bound(4) == 3 {
                loop {
                    cur += 1;
                    if vector_docs[cur as usize] >= i {
                        break;
                    }
                }
                Assert::assert_equals(
                    i64::from(vector_docs[cur as usize]),
                    i64::from(iter.advance(i)?),
                );
                Assert::assert_equals(
                    i64::from(vector_docs[cur as usize]),
                    i64::from(iter.doc_id()),
                );
                if iter.doc_id() == NO_MORE_DOCS {
                    break;
                }
                // make i equal to docid so that it is greater than docId in the next loop iteration
                i = iter.doc_id();
            }
            i += 1;
        }
        self.assert_off_heap_byte_size(&r, field_name)?;
        drop(r);
        reader.close()?;
        w.close()?;
        dir.close()
    }

    /// `testVectorValuesReportCorrectDocs()`.
    fn test_vector_values_report_correct_docs(&self) -> Result<()> {
        let num_docs = LuceneTestCase::at_least(1000);
        let mut dim = random().next_int_bound(20) + 1;
        if dim % 2 != 0 {
            dim += 1;
        }
        let mut field_values_check_sum = 0f64;
        let mut field_doc_count = 0i32;
        let mut field_sum_doc_ids = 0i64;
        let vector_encoding = self.knn_state().vector_encoding();
        let similarity_function = self.knn_state().similarity_function();

        let mut dir = LuceneTestCase::new_directory();
        let w = RandomIndexWriter::new_with_config(
            &*random(),
            dir.as_directory(),
            LuceneTestCase::new_index_writer_config(),
        )?;
        for _ in 0..num_docs {
            let mut doc = Document::new();
            let doc_id = random().next_int_bound(num_docs);
            doc.add(Box::new(StoredField::new_number(
                "id",
                rucene::document::NumericValue::Int(doc_id),
            )?));
            if random().next_int_bound(4) == 3 {
                match vector_encoding {
                    VectorEncoding::BYTE => {
                        let b = random_vector8(dim);
                        field_values_check_sum += f64::from(b[0] as i8);
                        doc.add(Box::new(KnnByteVectorField::new(
                            "knn_vector",
                            &b,
                            similarity_function,
                        )?));
                    }
                    VectorEncoding::FLOAT32 => {
                        let v = random_normalized_vector(dim);
                        field_values_check_sum += f64::from(v[0]);
                        doc.add(Box::new(KnnFloatVectorField::new(
                            "knn_vector",
                            &v,
                            similarity_function,
                        )?));
                    }
                }
                field_doc_count += 1;
                field_sum_doc_ids += i64::from(doc_id);
            }
            w.add_document(&doc)?;
        }

        if random().next_boolean() {
            w.force_merge(1)?;
        }

        let r = w.get_reader()?;
        let mut checksum = 0f64;
        let mut doc_count = 0i32;
        let mut sum_doc_ids = 0i64;
        let mut sum_ord_to_doc_ids = 0i64;
        let stored_id = |leaf: &dyn LeafReader, doc: i32| -> Result<i64> {
            let id = leaf
                .stored_fields()?
                .document_fields(doc, &["id".to_string()].into_iter().collect())?
                .get_values("id")
                .into_iter()
                .next()
                .expect("id is stored");
            Ok(i64::from(parse_int(&id)))
        };
        match vector_encoding {
            VectorEncoding::BYTE => {
                for ctx in Arc::clone(&r).leaves()? {
                    let leaf = ctx.leaf_reader();
                    if let Some(byte_vector_values) = leaf.get_byte_vector_values("knn_vector")? {
                        doc_count += byte_vector_values.size();
                        let mut iter = byte_vector_values.iterator()?;
                        iter.next_doc()?;
                        while iter.doc_id() != NO_MORE_DOCS {
                            let ord = iter.index();
                            checksum += f64::from(byte_vector_values.vector_value(ord)?[0] as i8);
                            sum_doc_ids += stored_id(&*leaf, iter.doc_id())?;
                            iter.next_doc()?;
                        }
                        for ord in 0..byte_vector_values.size() {
                            sum_ord_to_doc_ids +=
                                stored_id(&*leaf, byte_vector_values.ord_to_doc(ord))?;
                        }
                        self.assert_off_heap_byte_size(&leaf, "knn_vector")?;
                    }
                }
            }
            VectorEncoding::FLOAT32 => {
                for ctx in Arc::clone(&r).leaves()? {
                    let leaf = ctx.leaf_reader();
                    if let Some(vector_values) = leaf.get_float_vector_values("knn_vector")? {
                        doc_count += vector_values.size();
                        let mut iter = vector_values.iterator()?;
                        iter.next_doc()?;
                        while iter.doc_id() != NO_MORE_DOCS {
                            let ord = iter.index();
                            checksum += f64::from(vector_values.vector_value(ord)?[0]);
                            sum_doc_ids += stored_id(&*leaf, iter.doc_id())?;
                            iter.next_doc()?;
                        }
                        for ord in 0..vector_values.size() {
                            sum_ord_to_doc_ids += stored_id(&*leaf, vector_values.ord_to_doc(ord))?;
                        }
                        self.assert_off_heap_byte_size(&leaf, "knn_vector")?;
                    }
                }
            }
        }
        Assert::assert_equals_double_msg(
            Some(&format!("encoding={vector_encoding:?}")),
            field_values_check_sum,
            checksum,
            if vector_encoding == VectorEncoding::BYTE {
                f64::from(num_docs) * 0.2
            } else {
                1e-5
            },
        );
        Assert::assert_equals(i64::from(field_doc_count), i64::from(doc_count));
        Assert::assert_equals(field_sum_doc_ids, sum_doc_ids);
        Assert::assert_equals(field_sum_doc_ids, sum_ord_to_doc_ids);
        r.close()?;
        w.close()?;
        dir.close()
    }

    /// `getRandomFloatVector(numVectors, dim, normalize)` (`private`).
    fn get_random_float_vector(
        &self,
        num_vectors: i32,
        dim: i32,
        normalize: bool,
    ) -> Vec<Vec<f32>> {
        let mut vectors = Vec::with_capacity(num_vectors as usize);
        for _ in 0..num_vectors {
            let mut vec = random_vector(dim);
            if normalize {
                vector_util::l2normalize(&mut vec, true).unwrap_or_else(|e| rethrow(e));
            }
            vectors.push(vec);
        }
        vectors
    }

    /// `testReadQuantizedVectorWithEmptyRawVectors()`: tests reading
    /// quantized vectors when raw vector data is empty. Verifies that scalar
    /// quantized formats can properly dequantize vectors and maintain
    /// accuracy within expected error bounds even when the original raw
    /// vector file is empty or corrupted.
    fn test_read_quantized_vector_with_empty_raw_vectors(&self) -> Result<()> {
        LuceneTestCase::assume_true(
            "Test only applies to scalar quantized formats",
            self.supports_float_vector_fallback(),
        );

        let vector_field_name = "vec1";
        let num_vectors = 1 + random().next_int_bound(50);
        let mut dim = random().next_int_bound(64) + 1;
        if dim % 2 == 1 {
            dim += 1;
        }
        let eps = 1.0f32 / (1i32 << self.get_quantization_bits()) as f32;
        let similarity_function = self.random_similarity();
        let vectors = self.get_random_float_vector(
            num_vectors,
            dim,
            similarity_function == VectorSimilarityFunction::COSINE,
        );

        let mut dir = LuceneTestCase::new_directory();
        if let Some(mock) = dir.as_mock() {
            mock.set_check_index_on_close(false);
        } else if let crate::util::lucene_test_case::TestDirectory::Raw(raw) = &dir {
            raw.base().set_check_index_on_close(false);
        }

        {
            let mut iwc = IndexWriterConfig::new();
            iwc.set_max_buffered_docs(num_vectors + 1)?;
            iwc.set_ram_buffer_size_mb(f64::from(
                rucene::index::index_writer_config::LiveIndexWriterConfig::DISABLE_AUTO_FLUSH,
            ))?;
            iwc.set_merge_policy(Arc::new(NoMergePolicy::new()))?;
            iwc.set_use_compound_file(false)?;
            iwc.set_codec(self.get_codec_for_float_vector_fallback_test())?;
            let w = IndexWriter::new(dir.as_directory(), iwc)?;
            for vector in &vectors {
                let mut doc = Document::new();
                doc.add(Box::new(KnnFloatVectorField::new(
                    vector_field_name,
                    vector,
                    similarity_function,
                )?));
                w.add_document(&doc)?;
            }
            w.close()?;
        }
        self.simulate_empty_raw_vectors(&*dir.as_directory())?;

        let reader = directory_reader::open(dir.as_directory())?;
        let r = only_leaf(&reader)?;
        if let Some(codec_reader) = Arc::clone(&r).as_codec_reader() {
            let vectors_reader = codec_reader
                .get_vector_reader()?
                .expect("the segment has vectors");
            let knn_vectors_reader = vectors_reader
                .get_field_reader(vector_field_name)
                .unwrap_or_else(|| {
                    Throwable::new(
                        "java.lang.ClassCastException",
                        "not a PerFieldKnnVectorsFormat.FieldsReader",
                    )
                    .throw()
                });
            let float_vector_values = knn_vectors_reader
                .get_float_vector_values(vector_field_name)?
                .expect("float vectors");
            if float_vector_values.size() > 0 {
                let mut iter = float_vector_values.iterator()?;
                let mut doc_id = iter.next_doc()?;
                while doc_id != NO_MORE_DOCS {
                    let dequantized_vector = float_vector_values.vector_value(iter.index())?;
                    let mut mae = 0f32;
                    for i in 0..dim as usize {
                        mae += (dequantized_vector[i] - vectors[doc_id as usize][i]).abs();
                    }
                    mae /= dim as f32;
                    Assert::assert_true_msg(
                        &format!(
                            "bits: {} mae: {mae} > eps: {eps}",
                            self.get_quantization_bits()
                        ),
                        mae <= eps,
                    );
                    doc_id = iter.next_doc()?;
                }
            } else {
                Assert::fail("floatVectorValues size should be non zero");
            }
        } else {
            Assert::fail("reader is not CodecReader");
        }
        drop(r);
        reader.close()?;
        dir.close()
    }

    /// `testMismatchedFields()`.
    fn test_mismatched_fields(&self) -> Result<()> {
        let mut dir1 = LuceneTestCase::new_directory();
        let w1 = new_writer(dir1.as_directory())?;
        let mut doc = Document::new();
        doc.add(Box::new(KnnFloatVectorField::with_euclidean(
            "float",
            &[1.0, 2.0],
        )?));
        doc.add(Box::new(KnnByteVectorField::with_euclidean("byte", &[42])?));
        w1.add_document(&doc)?;

        let mut dir2 = LuceneTestCase::new_directory();
        let mut iwc2 = LuceneTestCase::new_index_writer_config();
        iwc2.set_merge_scheduler(Arc::new(SerialMergeScheduler::default()))?;
        let w2 = IndexWriter::new(dir2.as_directory(), iwc2)?;
        w2.add_document(&doc)?;
        w2.commit()?;

        let reader = directory_reader::open_with_writer(&*w1)?;
        w1.close()?;
        let codec_reader = only_leaf(&reader)?
            .as_codec_reader()
            .expect("a segment reader is a CodecReader");
        w2.add_indexes_codec_readers(&[Arc::new(MismatchedCodecReader::new(
            codec_reader,
            &*random(),
        )?)])?;
        reader.close()?;
        w2.force_merge(1)?;
        let reader = directory_reader::open_with_writer(&*w2)?;
        w2.close()?;

        let leaf_reader = only_leaf(&reader)?;

        let byte_vectors = leaf_reader.get_byte_vector_values("byte")?;
        assert_not_null(byte_vectors.is_some());
        let byte_vectors = byte_vectors.expect("checked");
        let mut iter = byte_vectors.iterator()?;
        Assert::assert_equals(0i64, i64::from(iter.next_doc()?));
        assert_array_equals(&[42u8], &byte_vectors.vector_value(0)?);
        Assert::assert_equals(1i64, i64::from(iter.next_doc()?));
        assert_array_equals(&[42u8], &byte_vectors.vector_value(1)?);
        Assert::assert_equals(i64::from(NO_MORE_DOCS), i64::from(iter.next_doc()?));

        let float_vectors = leaf_reader.get_float_vector_values("float")?;
        assert_not_null(float_vectors.is_some());
        let float_vectors = float_vectors.expect("checked");
        let mut iter = float_vectors.iterator()?;
        Assert::assert_equals(0i64, i64::from(iter.next_doc()?));
        let vector = float_vectors.vector_value(0)?;
        Assert::assert_equals(2i64, vector.len() as i64);
        assert_float_equals(1.0, vector[0], 0.0);
        assert_float_equals(2.0, vector[1], 0.0);
        Assert::assert_equals(1i64, i64::from(iter.next_doc()?));
        let vector = float_vectors.vector_value(1)?;
        Assert::assert_equals(2i64, vector.len() as i64);
        assert_float_equals(1.0, vector[0], 0.0);
        assert_float_equals(2.0, vector[1], 0.0);
        Assert::assert_equals(i64::from(NO_MORE_DOCS), i64::from(iter.next_doc()?));

        drop(leaf_reader);
        reader.close()?;
        w2.close()?;
        dir1.close()?;
        dir2.close()
    }

    /// `assertOffHeapByteSize(r, fieldName)` (`protected`).
    fn assert_off_heap_byte_size(&self, r: &Arc<dyn LeafReader>, field_name: &str) -> Result<()> {
        let field_infos = r.get_field_infos();
        let field_info = field_infos
            .field_info(field_name)
            .expect("the field exists")
            .clone();

        let Some(codec_reader) = Arc::clone(r).as_codec_reader() else {
            Throwable::new("java.lang.AssertionError", "unexpected: not a CodecReader").throw()
        };
        let vectors_reader = codec_reader
            .get_vector_reader()?
            .expect("the segment has vectors");
        let knn_vectors_reader: &dyn KnnVectorsReader = match vectors_reader
            .as_any()
            .and_then(|a| a.downcast_ref::<rucene::codecs::per_field::KnnFieldsReader>())
        {
            Some(fields_reader) => fields_reader
                .get_field_reader(field_name)
                .expect("the field has a reader"),
            None => &*vectors_reader,
        };
        let off_heap = knn_vectors_reader.get_off_heap_byte_size(&field_info);
        let total_byte_size: i64 = off_heap.values().sum();
        let is_simple_text = knn_vectors_reader.as_any().is_some_and(|a| {
            a.is::<rucene_codecs::simpletext::knn_vectors_reader::SimpleTextKnnVectorsReader>()
        });
        if is_simple_text {
            Assert::assert_equals(0i64, off_heap.len() as i64); // all vectors are in memory
            Assert::assert_equals(0i64, total_byte_size);
        } else if get_num_vectors(knn_vectors_reader, &field_info)? == 0 {
            Assert::assert_equals(0i64, total_byte_size);
        } else {
            Assert::assert_true(total_byte_size > 0);
            Assert::assert_true(*off_heap.get("vec").expect("vec") > 0);

            if has_hnsw(knn_vectors_reader, &field_info)? {
                Assert::assert_true(*off_heap.get("vex").expect("vex") > 0);
                let quant = off_heap.get("veq").or_else(|| off_heap.get("veb"));
                Assert::assert_true(quant.is_none_or(|q| *q > 0));
            } else {
                Assert::assert_true(off_heap.get("vex").is_none_or(|v| *v == 0));
            }

            if has_quantized(knn_vectors_reader, &field_info)? {
                Assert::assert_true(
                    *off_heap
                        .get("veq")
                        .or_else(|| off_heap.get("veb"))
                        .expect("quantized bytes")
                        > 0,
                );
            }
        }
        Ok(())
    }

    /// `testMergeOffHeapByteSizeMaps()`.
    fn test_merge_off_heap_byte_size_maps(&self) -> Result<()> {
        let map1: HashMap<String, i64> = [("a", 0i64), ("b", 1), ("c", 100)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        let map2: HashMap<String, i64> = [("b", 2i64), ("c", 5), ("d", 101), ("e", 6)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        let r = merge_off_heap_byte_size_maps(map1, map2);
        Assert::assert_equals(r.len() as i64, 5i64);
        Assert::assert_equals(0i64, r["a"]);
        Assert::assert_equals(3i64, r["b"]);
        Assert::assert_equals(105i64, r["c"]);
        Assert::assert_equals(101i64, r["d"]);
        Assert::assert_equals(6i64, r["e"]);
        Ok(())
    }
}

/// How the add-indexes illegal-change tests add `dir`.
#[derive(Debug, Clone, Copy)]
pub enum AddIndexesHow {
    /// `w2.addIndexes(Directory...)`.
    Directory,
    /// `w2.addIndexes(CodecReader...)`.
    CodecReader,
    /// `TestUtil.addIndexesSlowly(w2, r)`.
    SlowCodecReader,
}

/// `Codec.forName(name)`.
fn codec_for_name(name: &str) -> Result<Arc<dyn Codec>> {
    rucene::codecs::for_name(name).ok_or_else(|| {
        LuceneError::IllegalArgument(format!(
            "An SPI class of type org.apache.lucene.codecs.Codec with name '{name}' does not exist."
        ))
    })
}
