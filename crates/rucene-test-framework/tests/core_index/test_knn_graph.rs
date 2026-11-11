//! Port of `lucene/core/src/test/org/apache/lucene/index/TestKnnGraph.java`.
//!
//! Tests indexing of a knn-graph
//!
//! * The static `M` is a process-wide atomic, as Java's static field.
//! * `(PerFieldKnnVectorsFormat.FieldsReader) codecReader.getVectorReader()`
//!   downcasts to [`KnnFieldsReader`]; a reader of another class throws
//!   `ClassCastException`, as Java's cast does.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::{After, Assert, Before};
use randomizedtesting::RandomizedTest;
use rucene::codecs::knn_vectors::KnnVectorsReader;
use rucene::codecs::lucene104::scalar_quantized_vectors::Lucene104HnswScalarQuantizedVectorsFormat;
use rucene::codecs::lucene99::hnsw_vectors_format::{
    Lucene99HnswVectorsFormat, Lucene99HnswVectorsReader,
};
use rucene::codecs::per_field::KnnFieldsReader;
use rucene::codecs::Codec;
use rucene::document::{
    Document, FieldType, KnnFloatVectorField, SortedDocValuesField, Store, StringField,
};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::terms::Term;
use rucene::index::{IndexReader, VectorEncoding, VectorSimilarityFunction};
use rucene::search::knn_float_vector_query::KnnFloatVectorQuery;
use rucene::search::searcher_factory::DefaultSearcherFactory;
use rucene::search::searcher_manager::SearcherManager;
use rucene::search::{IndexSearcher, TopDocs, NO_MORE_DOCS};
use rucene::util::hnsw::builder::{DEFAULT_BEAM_WIDTH, DEFAULT_MAX_CONN};
use rucene::util::hnsw::HnswGraphBuilder;
use rucene::util::quantization::quantized_byte_vector_values::ScalarEncoding;
use rucene::util::{vector_util, BytesRef};
use rucene_test_framework::index::base_field_info_format_test_case::{
    VECTOR_ENCODING_VALUES, VECTOR_SIMILARITY_FUNCTION_VALUES,
};
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{
    assert_array_equals_delta_msg, assert_equals, assert_equals_msg, random,
};
use crate::support::{npe, CountDownLatch};

const KNN_GRAPH_FIELD: &str = "vector";

/// `private static int M = HnswGraphBuilder.DEFAULT_MAX_CONN`.
static M: AtomicI32 = AtomicI32::new(DEFAULT_MAX_CONN);

fn m() -> i32 {
    M.load(Ordering::SeqCst)
}

/// The instance fields of `TestKnnGraph`.
struct Instance {
    codec: Option<Arc<dyn Codec>>,
    float32_codec: Option<Arc<dyn Codec>>,
    vector_encoding: VectorEncoding,
    similarity_function: VectorSimilarityFunction,
}

impl Instance {
    fn new() -> Self {
        Instance {
            codec: None,
            float32_codec: None,
            vector_encoding: VectorEncoding::FLOAT32,
            similarity_function: VectorSimilarityFunction::EUCLIDEAN,
        }
    }

    fn codec(&self) -> Arc<dyn Codec> {
        self.codec.clone().unwrap_or_else(|| npe("codec"))
    }

    fn float32_codec(&self) -> Arc<dyn Codec> {
        self.float32_codec
            .clone()
            .unwrap_or_else(|| npe("float32Codec"))
    }
}

fn t(name: &str, body: fn(&mut Instance) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body(move |inst: &mut Instance| body(inst).unwrap_or_else(|e| rethrow_java(e)))
}

/// `TestKnnGraph`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestKnnGraph")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(Instance::new))
        .method(
            Method::new("setup")
                .annotate(Before)
                .body(|inst: &mut Instance| setup(inst).unwrap_or_else(|e| rethrow_java(e))),
        )
        .method(Method::new("cleanup").annotate(After).body(cleanup))
        .method(t("testBasic", test_basic))
        .method(t("testSingleDocument", test_single_document))
        .method(t("testMerge", test_merge))
        .method(t("testMultipleVectorFields", test_multiple_vector_fields))
        .method(t("testSearch", test_search))
        .method(t("testMultiThreadedSearch", test_multi_threaded_search))
        .build()
}

/// `@Before setup()`.
fn setup(this: &mut Instance) -> Result<()> {
    HnswGraphBuilder::set_rand_seed(random().next_long() as u64);
    if random().next_boolean() {
        M.store(random().next_int_bound(256) + 3, Ordering::SeqCst);
    }

    let similarity =
        random().next_int_bound(VECTOR_SIMILARITY_FUNCTION_VALUES.len() as i32 - 1) + 1;
    this.similarity_function = VECTOR_SIMILARITY_FUNCTION_VALUES[similarity as usize];
    this.vector_encoding = random_vector_encoding();
    let quantized = RandomizedTest::random_boolean();
    this.codec = Some(if quantized {
        TestUtil::always_knn_vectors_format(Arc::new(
            Lucene104HnswScalarQuantizedVectorsFormat::with_params(
                m(),
                DEFAULT_BEAM_WIDTH,
                ScalarEncoding::UnsignedByte,
            )?,
        ))
    } else {
        TestUtil::always_knn_vectors_format(Arc::new(
            Lucene99HnswVectorsFormat::with_max_conn_beam_width(m(), DEFAULT_BEAM_WIDTH),
        ))
    });

    this.float32_codec = Some(TestUtil::always_knn_vectors_format(Arc::new(
        Lucene99HnswVectorsFormat::with_max_conn_beam_width(m(), DEFAULT_BEAM_WIDTH),
    )));
    Ok(())
}

/// `randomVectorEncoding()`.
fn random_vector_encoding() -> VectorEncoding {
    VECTOR_ENCODING_VALUES[random().next_int_bound(VECTOR_ENCODING_VALUES.len() as i32) as usize]
}

/// `@After cleanup()`.
fn cleanup(_this: &mut Instance) {
    M.store(DEFAULT_MAX_CONN, Ordering::SeqCst);
}

/// `newIndexWriterConfig(null).setCodec(codec)`.
fn null_analyzer_iwc(codec: Arc<dyn Codec>) -> Result<rucene::index::IndexWriterConfig> {
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(None);
    iwc.set_codec(codec)?;
    Ok(iwc)
}

/// `try (Directory dir = newDirectory(); IndexWriter iw = new IndexWriter(dir, iwc)) { body }`.
fn with_writer(
    iwc: rucene::index::IndexWriterConfig,
    body: impl FnOnce(&IndexWriter) -> Result<()>,
) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = match IndexWriter::new(dir.as_directory(), iwc) {
        Ok(iw) => iw,
        Err(e) => {
            let _ = dir.close();
            return Err(e);
        }
    };
    let outcome = body(&iw);
    let closed_iw = iw.close();
    drop(iw);
    let closed_dir = dir.close();
    outcome?;
    closed_iw?;
    closed_dir
}

/// Basic test of creating documents in a graph
///
/// Port of `TestKnnGraph.testBasic`.
fn test_basic(this: &mut Instance) -> Result<()> {
    with_writer(null_analyzer_iwc(this.codec())?, |iw| {
        let num_doc = LuceneTestCase::at_least(10);
        let dimension = LuceneTestCase::at_least(3);
        let mut values: Vec<Option<Vec<f32>>> = vec![None; num_doc as usize];
        for i in 0..num_doc as usize {
            if random().next_boolean() {
                values[i] = Some(random_vector(this, dimension)?);
            }
            add(this, iw, i as i32, values[i].as_deref())?;
        }
        assert_consistent_graph(iw, &values)
    })
}

/// Port of `TestKnnGraph.testSingleDocument`.
fn test_single_document(this: &mut Instance) -> Result<()> {
    with_writer(null_analyzer_iwc(this.codec())?, |iw| {
        let mut values: Vec<Option<Vec<f32>>> = vec![Some(vec![0.0, 1.0, 2.0])];
        let v0 = values[0].as_mut().expect("INVARIANT: set above");
        if this.similarity_function == VectorSimilarityFunction::DOT_PRODUCT {
            vector_util::l2normalize(v0, true)?;
        }
        if this.vector_encoding == VectorEncoding::BYTE {
            for value in v0.iter_mut().take(3) {
                *value = (*value * 127.0).floor();
            }
        }
        add(this, iw, 0, values[0].as_deref())?;
        assert_consistent_graph(iw, &values)?;
        iw.commit()?;
        assert_consistent_graph(iw, &values)
    })
}

/// Verify that the graph properties are preserved when merging
///
/// Port of `TestKnnGraph.testMerge`.
fn test_merge(this: &mut Instance) -> Result<()> {
    with_writer(null_analyzer_iwc(this.codec())?, |iw| {
        let num_doc = LuceneTestCase::at_least(100);
        let dimension = LuceneTestCase::at_least(10);
        let mut values = random_vectors(this, num_doc, dimension)?;
        for i in 0..num_doc as usize {
            if random().next_boolean() {
                values[i] = Some(random_vector(this, dimension)?);
            }
            add(this, iw, i as i32, values[i].as_deref())?;
            if random().next_int_bound(10) == 3 {
                iw.commit()?;
            }
        }
        if random().next_boolean() {
            iw.force_merge(1)?;
        }
        assert_consistent_graph(iw, &values)
    })
}

/// Test writing and reading of multiple vector fields *
///
/// Port of `TestKnnGraph.testMultipleVectorFields`.
fn test_multiple_vector_fields(this: &mut Instance) -> Result<()> {
    let num_vector_fields = RandomizedTest::random_int_between(2, 5) as usize;
    let num_doc = LuceneTestCase::at_least(100);
    let mut dims = vec![0i32; num_vector_fields];
    let mut values: Vec<Vec<Option<Vec<f32>>>> = Vec::with_capacity(num_vector_fields);
    let mut field_types: Vec<FieldType> = Vec::with_capacity(num_vector_fields);
    for field in 0..num_vector_fields {
        dims[field] = LuceneTestCase::at_least(3);
        values.push(random_vectors(this, num_doc, dims[field])?);
        field_types.push(KnnFloatVectorField::create_field_type(
            dims[field],
            this.similarity_function,
        )?);
    }

    with_writer(null_analyzer_iwc(this.codec())?, |iw| {
        for doc_id in 0..num_doc as usize {
            let mut doc = Document::new();
            for field in 0..num_vector_fields {
                if let Some(vector) = &values[field][doc_id] {
                    doc.add(Box::new(KnnFloatVectorField::with_field_type(
                        &format!("{KNN_GRAPH_FIELD}{field}"),
                        vector,
                        field_types[field].clone(),
                    )?));
                }
            }
            let id_string = doc_id.to_string();
            doc.add(Box::new(StringField::new("id", id_string, Store::YES)?));
            iw.add_document(&doc)?;
        }
        for (field, field_values) in values.iter().enumerate() {
            assert_consistent_graph_field(iw, field_values, &format!("{KNN_GRAPH_FIELD}{field}"))?;
        }
        Ok(())
    })
}

/// Port of `TestKnnGraph.randomVectors` (`private`).
fn random_vectors(this: &Instance, num_doc: i32, dimension: i32) -> Result<Vec<Option<Vec<f32>>>> {
    let mut values: Vec<Option<Vec<f32>>> = vec![None; num_doc as usize];
    for value in values.iter_mut() {
        if random().next_boolean() {
            *value = Some(random_vector(this, dimension)?);
        }
    }
    Ok(values)
}

/// Port of `TestKnnGraph.randomVector` (`private`).
fn random_vector(this: &Instance, dimension: i32) -> Result<Vec<f32>> {
    let mut value = vec![0f32; dimension as usize];
    for v in value.iter_mut() {
        *v = random().next_float();
    }
    vector_util::l2normalize(&mut value, true)?;
    if this.vector_encoding == VectorEncoding::BYTE {
        for v in value.iter_mut() {
            // `(byte) (value[j] * 127)`: the float-to-int narrowing, then
            // the byte narrowing.
            *v = ((*v * 127.0) as i32 as i8) as f32;
        }
    }
    Ok(value)
}

/// Verify that searching does something reasonable
///
/// Port of `TestKnnGraph.testSearch`.
fn test_search(this: &mut Instance) -> Result<()> {
    // We can't use dot product here since the vectors are laid out on a grid, not a sphere.
    this.similarity_function = VectorSimilarityFunction::EUCLIDEAN;
    let config = LuceneTestCase::new_index_writer_config();
    config.set_codec(this.float32_codec())?;
    with_writer(config, |iw| {
        index_data(this, iw)?;
        let dr = directory_reader::open_with_writer(iw)?;
        let searched = (|| -> Result<()> {
            // results are ordered by score (descending) and docid (ascending);
            // This is the insertion order:
            // column major, origin at upper left
            //  0 15  5 20 10
            //  3 18  8 23 13
            //  6 21 11  1 16
            //  9 24 14  4 19
            // 12  2 17  7 22

            // For this small graph the "search" is exhaustive, so this mostly tests the APIs, the
            // orientation of the various priority queues, the scoring function, but not so much the
            // approximate KNN search algorithm
            let reader: Arc<dyn IndexReader> = Arc::clone(&dr) as Arc<dyn IndexReader>;
            assert_graph_search(
                &[0, 15, 3, 18, 5],
                &[0.99, 0.55, 0.50, 0.36, 0.22],
                &[0.0, 0.1],
                &reader,
            )?;
            // Tiebreaking by docid must be done after search.
            // assertGraphSearch(new int[]{11, 1, 8, 14, 21}, new float[]{2, 2}, dr);
            assert_graph_search(
                &[15, 18, 0, 3, 5],
                &[0.88, 0.65, 0.58, 0.47, 0.40],
                &[0.3, 0.8],
                &reader,
            )
        })();
        let closed = dr.close();
        searched?;
        closed
    })
}

/// Port of `TestKnnGraph.indexData` (`private`).
fn index_data(this: &Instance, iw: &IndexWriter) -> Result<()> {
    // Add a document for every cartesian point in an NxN square so we can
    // easily know which are the nearest neighbors to every point. Insert by iterating
    // using a prime number that is not a divisor of N*N so that we will hit each point once,
    // and chosen so that points will be inserted in a deterministic
    // but somewhat distributed pattern
    let n = 5;
    let step_size = 17;
    let mut values: Vec<Option<Vec<f32>>> = vec![None; n * n];
    let mut index = 0;
    for i in 0..values.len() {
        // System.out.printf("%d: (%d, %d)\n", i, index % n, index / n);
        let x = index % n;
        let y = index / n;
        values[i] = Some(vec![x as f32, y as f32]);
        index = (index + step_size) % (n * n);
        add(this, iw, i as i32, values[i].as_deref())?;
        if i == 13 {
            // create 2 segments
            iw.commit()?;
        }
    }
    let force_merge = random().next_boolean();
    if force_merge {
        iw.force_merge(1)?;
    }
    assert_consistent_graph(iw, &values)
}

/// Port of `TestKnnGraph.testMultiThreadedSearch`.
fn test_multi_threaded_search(this: &mut Instance) -> Result<()> {
    this.similarity_function = VectorSimilarityFunction::EUCLIDEAN;
    let config = LuceneTestCase::new_index_writer_config();
    config.set_codec(this.float32_codec())?;
    let dir = LuceneTestCase::new_directory();
    let iw = IndexWriter::new(dir.as_directory(), config)?;
    index_data(this, &iw)?;

    let manager = Arc::new(SearcherManager::from_writer(
        &*iw,
        true,
        false,
        Some(Arc::new(DefaultSearcherFactory)),
        None,
    )?);
    let num_threads = RandomizedTest::random_int_between(2, 5) as usize;
    let mut threads = Vec::with_capacity(num_threads);
    let latch = Arc::new(CountDownLatch::new(1));
    for _ in 0..num_threads {
        let manager = Arc::clone(&manager);
        let latch = Arc::clone(&latch);
        let thread = Thread::unnamed(move || {
            let outcome = (|| -> Result<()> {
                latch.await_();
                let searcher = manager.acquire()?;
                let searched = assert_graph_search(
                    &[0, 15, 3, 18, 5],
                    &[0.99, 0.55, 0.50, 0.36, 0.22],
                    &[0.0, 0.1],
                    searcher.searcher().get_index_reader(),
                );
                // finally
                let released = manager.release(searcher);
                searched?;
                released
            })();
            if let Err(e) = outcome {
                Throwable::from_cause(
                    "java.lang.RuntimeException",
                    rucene_test_framework::util::java_exceptions::java_throwable(&e),
                )
                .throw();
            }
        });
        thread.start();
        threads.push(thread);
    }

    latch.count_down();
    for t in &threads {
        let _ = t.join();
    }
    // IOUtils.close(manager, iw, dir)
    let closed_manager = manager.close();
    let closed_iw = iw.close();
    drop(iw);
    let closed_dir = dir.close();
    closed_manager?;
    closed_iw?;
    closed_dir
}

/// Port of `TestKnnGraph.assertGraphSearch` (`private`).
fn assert_graph_search(
    expected: &[i32],
    expected_scores: &[f32],
    vector: &[f32],
    reader: &Arc<dyn IndexReader>,
) -> Result<()> {
    let mut results = do_knn_search(reader, vector, 5)?;
    let stored_fields = reader.stored_fields()?;
    for doc in results.score_docs.iter_mut() {
        // map docId to insertion id
        let id = stored_fields
            .document(doc.doc)?
            .get("id")
            .unwrap_or_else(|| npe("id"));
        doc.doc = parse_int(&id);
    }
    assert_results(expected, expected_scores, &results);
    Ok(())
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

/// Port of `TestKnnGraph.doKnnSearch` (`private static`).
fn do_knn_search(reader: &Arc<dyn IndexReader>, vector: &[f32], k: i32) -> Result<TopDocs> {
    let searcher = IndexSearcher::new(Arc::clone(reader))?;
    searcher.search(
        Arc::new(KnnFloatVectorQuery::new(
            KNN_GRAPH_FIELD,
            vector.to_vec(),
            k,
        )?),
        k,
    )
}

/// `Arrays.toString(results.scoreDocs)`.
fn score_docs_to_string(results: &TopDocs) -> String {
    let parts: Vec<String> = results.score_docs.iter().map(|d| d.to_string()).collect();
    format!("[{}]", parts.join(", "))
}

/// Port of `TestKnnGraph.assertResults` (`private`).
fn assert_results(expected: &[i32], expected_scores: &[f32], results: &TopDocs) {
    // `results.toString()`: `TopDocs` does not override `Object.toString()`.
    let results_string = format!(
        "org.apache.lucene.search.TopDocs@{:x}",
        results as *const TopDocs as usize
    );
    assert_equals_msg(
        Some(&results_string),
        &expected.len(),
        &results.score_docs.len(),
    );
    for i in (0..expected.len()).rev() {
        assert_equals_msg(
            Some(&format!(
                "doc mismatch at idx: {i} results: {}",
                score_docs_to_string(results)
            )),
            &expected[i],
            &results.score_docs[i].doc,
        );
        randomizedtesting::junit::Assert::assert_equals_float_msg(
            Some(&format!(
                "score mismatch at idx: {i} results: {}",
                score_docs_to_string(results)
            )),
            expected_scores[i],
            results.score_docs[i].score,
            0.01,
        );
    }
}

/// Port of `TestKnnGraph.assertConsistentGraph(IndexWriter, float[][])` (`private`).
fn assert_consistent_graph(iw: &IndexWriter, values: &[Option<Vec<f32>>]) -> Result<()> {
    assert_consistent_graph_field(iw, values, KNN_GRAPH_FIELD)
}

/// `assertArrayEquals(message, expecteds, actuals, 0)` over `float[]`.
fn assert_float_array_equals_msg(message: &str, expecteds: Option<&[f32]>, actuals: &[f32]) {
    let Some(expecteds) = expecteds else {
        Assert::fail(&format!("{message}: expected array was null"));
    };
    let e: Vec<f64> = expecteds.iter().map(|&v| f64::from(v)).collect();
    let a: Vec<f64> = actuals.iter().map(|&v| f64::from(v)).collect();
    assert_array_equals_delta_msg(Some(message), &e, &a, 0.0);
}

/// `Arrays.toString(float[])`.
fn floats_to_string(values: &[f32]) -> String {
    let parts: Vec<String> = values
        .iter()
        .map(|&v| rucene::util::java_fmt::float_to_string(v))
        .collect();
    format!("[{}]", parts.join(", "))
}

/// For each leaf, verify that its graph nodes are 1-1 with vectors, that the vectors are the
/// expected values, and that the graph is fully connected and symmetric.
/// NOTE: when we impose max-fanout on the graph it wil no longer be symmetric, but should still
/// be fully connected. Is there any other invariant we can test? Well, we can check that max
/// fanout is respected. We can test *desirable* properties of the graph like small-world
/// (the graph diameter should be tightly bounded).
///
/// Port of `TestKnnGraph.assertConsistentGraph(IndexWriter, float[][], String)` (`private`).
fn assert_consistent_graph_field(
    iw: &IndexWriter,
    values: &[Option<Vec<f32>>],
    vector_field: &str,
) -> Result<()> {
    let mut num_docs_with_vectors = 0;
    let dr = directory_reader::open_with_writer(iw)?;
    let checked = (|| -> Result<()> {
        for ctx in (Arc::clone(&dr) as Arc<dyn IndexReader>).leaves()? {
            let reader = ctx.leaf_reader();
            let codec_reader = Arc::clone(&reader).as_codec_reader().unwrap_or_else(|| {
                Throwable::new("java.lang.ClassCastException", "not a CodecReader").throw()
            });
            let Some(vectors_reader) = codec_reader.get_vector_reader()? else {
                continue;
            };
            let per_field_reader: &dyn KnnVectorsReader = &*vectors_reader;
            let per_field_reader = per_field_reader
                .as_any()
                .and_then(|a| a.downcast_ref::<KnnFieldsReader>())
                .unwrap_or_else(|| {
                    Throwable::new(
                        "java.lang.ClassCastException",
                        "not a PerFieldKnnVectorsFormat.FieldsReader",
                    )
                    .throw()
                });
            let Some(field_reader) = per_field_reader.get_field_reader(vector_field) else {
                continue;
            };
            let vector_reader = field_reader
                .as_any()
                .and_then(|a| a.downcast_ref::<Lucene99HnswVectorsReader>())
                .unwrap_or_else(|| {
                    Throwable::new(
                        "java.lang.ClassCastException",
                        "not a Lucene99HnswVectorsReader",
                    )
                    .throw()
                });
            let mut graph_values =
                rucene::codecs::hnsw::graph_provider::HnswGraphProvider::get_graph(
                    vector_reader,
                    vector_field,
                )?;
            let Some(vector_values) = reader.get_float_vector_values(vector_field)? else {
                continue;
            };

            // assert vector values:
            // stored vector values are the same as original
            let mut next_doc_with_vectors = 0;
            let stored_fields = reader.stored_fields()?;
            let mut iterator = vector_values.iterator()?;
            let max_doc = reader.max_doc();
            let mut i = 0;
            while i < max_doc {
                next_doc_with_vectors = iterator.advance(i)?;
                while i < next_doc_with_vectors && i < max_doc {
                    let id = parse_int(
                        &stored_fields
                            .document(i)?
                            .get("id")
                            .unwrap_or_else(|| npe("id")),
                    );
                    crate::support::assert_null_msg(
                        &format!("document {id}, expected to have no vector, does have one"),
                        &values[id as usize],
                    );
                    i += 1;
                }
                if next_doc_with_vectors == NO_MORE_DOCS {
                    break;
                }
                let id = parse_int(
                    &stored_fields
                        .document(i)?
                        .get("id")
                        .unwrap_or_else(|| npe("id")),
                );
                // documents with KnnGraphValues have the expected vectors
                let scratch = vector_values.vector_value(iterator.index())?;
                assert_float_array_equals_msg(
                    &format!(
                        "vector did not match for doc {i}, id={id}: {}",
                        floats_to_string(&scratch)
                    ),
                    values[id as usize].as_deref(),
                    &scratch,
                );
                num_docs_with_vectors += 1;
                i += 1;
            }
            // if IndexDisi.doc == NO_MORE_DOCS, we should not call IndexDisi.nextDoc()
            if next_doc_with_vectors != NO_MORE_DOCS {
                assert_equals(&NO_MORE_DOCS, &iterator.next_doc()?);
            } else {
                assert_equals(&NO_MORE_DOCS, &iterator.doc_id());
            }

            // assert graph values:
            // For each level of the graph assert that:
            // 1. There are no orphan nodes without any friends
            // 2. If orphans are found, than the level must contain only 0 or a single node
            // 3. If the number of nodes on the level doesn't exceed maxConnOnLevel, assert that the
            // graph is
            //   fully connected, i.e. any node is reachable from any other node.
            // 4. If the number of nodes on the level exceeds maxConnOnLevel, assert that maxConnOnLevel
            // is respected.
            for level in 0..graph_values.num_levels()? {
                let max_conn_on_level = if level == 0 { m() * 2 } else { m() };
                let mut graph_on_level: Vec<Option<Vec<i32>>> =
                    vec![None; graph_values.size() as usize];
                let mut count_on_level = 0;
                let mut found_orphan = false;
                let mut nodes_itr = graph_values.get_nodes_on_level(level)?;
                while nodes_itr.has_next() {
                    let node = nodes_itr.next_int();
                    graph_values.seek(level, node)?;
                    let mut friends: Vec<i32> = Vec::new();
                    loop {
                        let arc = graph_values.next_neighbor()?;
                        if arc == NO_MORE_DOCS {
                            break;
                        }
                        friends.push(arc);
                    }
                    if friends.is_empty() {
                        found_orphan = true;
                    } else {
                        graph_on_level[node as usize] = Some(friends);
                    }
                    count_on_level += 1;
                }
                assert_equals(&nodes_itr.size(), &count_on_level);
                Assert::assert_false_msg(
                    &format!("No nodes on level [{level}]"),
                    count_on_level == 0,
                );
                if count_on_level == 1 {
                    Assert::assert_true_msg(
                        &format!("Graph with 1 node has unexpected neighbors on level [{level}]"),
                        found_orphan,
                    );
                } else {
                    Assert::assert_false_msg(
                        &format!("Graph has orphan nodes with no friends on level [{level}]"),
                        found_orphan,
                    );
                    if max_conn_on_level > count_on_level {
                        // assert that the graph is fully connected,
                        // i.e. any node can be reached from any other node
                        assert_connected(&graph_on_level);
                    } else {
                        // assert that max-connections was respected
                        assert_max_conn(&graph_on_level, max_conn_on_level);
                    }
                }
            }
        }
        Ok(())
    })();
    let closed = dr.close();
    checked?;
    closed?;

    let expected_num_docs_with_vectors = values.iter().filter(|v| v.is_some()).count() as i32;
    assert_equals(&expected_num_docs_with_vectors, &num_docs_with_vectors);
    Ok(())
}

/// Port of `TestKnnGraph.assertMaxConn` (`public static`).
pub fn assert_max_conn(graph: &[Option<Vec<i32>>], max_conn: i32) {
    for ints in graph.iter().flatten() {
        debug_assert!(ints.len() as i32 <= max_conn);
        for &k in ints {
            crate::core_support::assert_not_null(&graph[k as usize]);
        }
    }
}

/// Assert that every node is reachable from some other node
///
/// Port of `TestKnnGraph.assertConnected` (`private static`).
fn assert_connected(graph: &[Option<Vec<i32>>]) {
    let mut nodes: Vec<i32> = Vec::new();
    let mut visited: HashSet<i32> = HashSet::new();
    let mut queue: VecDeque<i32> = VecDeque::new();
    for (i, node) in graph.iter().enumerate() {
        if node.is_some() {
            nodes.push(i as i32);
        }
    }

    // start from any node
    let start_idx = random().next_int_bound(nodes.len() as i32) as usize;
    queue.push_back(nodes[start_idx]);
    while let Some(i) = queue.pop_front() {
        crate::support::assert_not_null_msg(
            &format!("expected neighbors of {i}"),
            &graph[i as usize],
        );
        visited.insert(i);
        for &j in graph[i as usize].as_deref().unwrap_or(&[]) {
            if !visited.contains(&j) {
                queue.push_back(j);
            }
        }
    }
    // assert that every node is reachable from some other node as it was visited
    for node in &nodes {
        Assert::assert_true_msg(
            &format!("Attempted to walk entire graph but never visited node [{node}]"),
            visited.contains(node),
        );
    }
}

/// Port of `TestKnnGraph.add(IndexWriter, int, float[])` (`private`).
fn add(this: &Instance, iw: &IndexWriter, id: i32, vector: Option<&[f32]>) -> Result<()> {
    add_with_similarity(iw, id, vector, this.similarity_function)
}

/// Port of `TestKnnGraph.add(IndexWriter, int, float[], VectorSimilarityFunction)` (`private`).
fn add_with_similarity(
    iw: &IndexWriter,
    id: i32,
    vector: Option<&[f32]>,
    similarity_function: VectorSimilarityFunction,
) -> Result<()> {
    let mut doc = Document::new();
    if let Some(vector) = vector {
        let field_type =
            KnnFloatVectorField::create_field_type(vector.len() as i32, similarity_function)?;
        doc.add(Box::new(KnnFloatVectorField::with_field_type(
            KNN_GRAPH_FIELD,
            vector,
            field_type,
        )?));
    }
    let id_string = id.to_string();
    doc.add(Box::new(StringField::new(
        "id",
        id_string.clone(),
        Store::YES,
    )?));
    doc.add(Box::new(SortedDocValuesField::new(
        "id",
        BytesRef::from_char_sequence(&id_string),
    )));
    // XSSystem.out.println("add " + idString + " " + Arrays.toString(vector));
    iw.update_document(Term::from_text("id", &id_string), &doc)?;
    Ok(())
}
