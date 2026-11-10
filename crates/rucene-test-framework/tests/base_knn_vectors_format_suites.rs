//! The `BaseKnnVectorsFormatTestCase` suites (rmp #354): the inherited cases
//! of `BaseIndexFileFormatTestCase` and `BaseKnnVectorsFormatTestCase` run,
//! under the ported runner and `LuceneTestCase`'s rule chain, on the codecs
//! Lucene's own subclasses bind:
//!
//! * `org.apache.lucene.codecs.lucene99.TestLucene99HnswVectorsFormat`:
//!   `TestUtil.getDefaultCodec()`;
//! * `org.apache.lucene.codecs.lucene99.TestLucene99HnswVectorsFormatV0`:
//!   `TestUtil.alwaysKnnVectorsFormat(new Lucene99HnswVectorsFormat(
//!   DEFAULT_MAX_CONN, DEFAULT_BEAM_WIDTH, DEFAULT_NUM_MERGE_WORKER, null,
//!   0))`;
//! * `org.apache.lucene.codecs.perfield.TestPerFieldKnnVectorsFormat`: its
//!   `setUp()` builds `new RandomCodec(new Random(random().nextLong()),
//!   Collections.emptySet())` before `super.setUp()`;
//! * `org.apache.lucene.codecs.lucene99.TestKnnVectorsFormatCustomWriter`:
//!   `TestUtil.alwaysKnnVectorsFormat(new PagedHnswVectorsFormat())`, an HNSW
//!   writer whose flat writer keeps its vectors in pages of ten;
//! * `org.apache.lucene.codecs.lucene104.TestLucene104HnswScalarQuantizedVectorsFormat`:
//!   its `setUp()` draws a `ScalarEncoding` and builds the format before
//!   `super.setUp()`;
//! * `org.apache.lucene.codecs.lucene104.TestLucene104ScalarQuantizedVectorsFormat`:
//!   likewise; it supports the float-vector fallback, reports the encoding's
//!   bits, empties the raw vectors for that test, and turns
//!   `testRandomWithUpdatesAndGraph` and `testSearchWithVisitedLimit` into
//!   no-ops.
//!
//! Every class answers `supportsFloatVectorFallback()` as Lucene's does. The
//! own test methods of `TestLucene99HnswVectorsFormat`,
//! `TestPerFieldKnnVectorsFormat`,
//! `TestLucene104HnswScalarQuantizedVectorsFormat` and
//! `TestLucene104ScalarQuantizedVectorsFormat` (sprint 9) are declared on their
//! bindings.
//!
//! **Divergence (`testToString`).** Java prints
//! `customCodec.knnVectorsFormat().toString()`, `Object.toString()` of the
//! format the anonymous `FilterCodec` returns; Rucene's `KnnVectorsFormat`
//! trait has no `toString()`, so the tests print the `Display` of the very
//! format that override returns.
//!
//! Each suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds of
//! `suite_seeds::SEEDS` instead, reporting the pass counts.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::codecs::codec_util;
use rucene::codecs::hnsw::flat_vectors::{
    DocsWithFieldSet, FlatFieldVectorWriter, FlatFieldVectorsWriter, FlatVectorScorerUtil,
    FlatVectorsScorer,
};
use rucene::codecs::knn_vectors::{
    FieldVectorWriter, IORunnable, KnnFieldVectorsWriter, KnnVectorsFormat, KnnVectorsReader,
    KnnVectorsWriter, SorterDocMap,
};
use rucene::codecs::lucene104::scalar_quantized_vectors::{
    Lucene104HnswScalarQuantizedVectorsFormat, Lucene104ScalarQuantizedVectorsFormat,
    DIRECT_MONOTONIC_BLOCK_SHIFT,
};
use rucene::codecs::lucene104::scalar_quantized_vectors::{
    NormalizedFloatVectorValues, OffHeapScalarQuantizedVectorValues, ScalarQuantizedVectorValues,
};
use rucene::codecs::lucene95::OrdToDocDISIReaderConfiguration;
use rucene::codecs::lucene99::flat_vectors_format::{
    Lucene99FlatVectorsFormat, Lucene99FlatVectorsWriter,
};
use rucene::codecs::lucene99::hnsw_vectors_format::Lucene99HnswVectorsReader;
use rucene::codecs::lucene99::hnsw_vectors_format::{
    Lucene99HnswVectorsFormat, Lucene99HnswVectorsWriter, DEFAULT_BEAM_WIDTH, DEFAULT_MAX_CONN,
    DEFAULT_NUM_MERGE_WORKER, HNSW_GRAPH_THRESHOLD, VERSION_CURRENT,
};
use rucene::codecs::per_field::KnnFieldsReader;
use rucene::codecs::postings::MergeState;
use rucene::codecs::state::{OwnedSegmentWriteState, SegmentReadState};
use rucene::codecs::{get_default, Codec, FilterCodec};
use rucene::document::{Document, KnnFloatVectorField, Store};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::field_infos::FieldInfo;
use rucene::index::index_writer::IndexWriter;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::VectorEncoding;
use rucene::index::{FloatVectorValues, IndexReader, LeafReader, VectorSimilarityFunction};
use rucene::search::doc_id_set_iterator::from_live_docs;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::knn_float_vector_query::KnnFloatVectorQuery;
use rucene::search::total_hits::TotalHitsRelation;
use rucene::search::{Query, NO_MORE_DOCS};
use rucene::store::{ByteBuffersDirectory, Directory, IndexInput, DEFAULT_IO_CONTEXT};
use rucene::util::quantization::OptimizedScalarQuantizer;
use rucene::util::quantization::ScalarEncoding;
use rucene::util::vector_util;
use rucene::util::RamUsageEstimator;
use rucene::util::SameThreadExecutorService;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::codecs::asserting::asserting_codec::{
    AssertingCodec, KnnVectorsFormatForField,
};
use rucene_test_framework::index::base_index_file_format_test_case::{
    rethrow, set_up, tear_down, BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::base_knn_vectors_format_test_case::{
    self, BaseKnnVectorsFormatTestCase, KnnSuiteState,
};
use rucene_test_framework::index::base_knn_vectors_format_test_case::{
    random_normalized_vector, random_vector,
};
use rucene_test_framework::index::random_codec::RandomCodec;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

mod suite_draws;
mod suite_seeds;
use suite_seeds::run_suite;

/// How a kNN suite answers `getCodec()`.
enum CodecOf {
    /// `TestUtil.getDefaultCodec()`.
    Default,
    /// `TestUtil.alwaysKnnVectorsFormat(make())`, on every call.
    Always(fn() -> Arc<dyn KnnVectorsFormat>),
    /// The `codec` field `setUp()` assigns (`TestPerFieldKnnVectorsFormat`).
    SetUpCodec(Mutex<Option<Arc<dyn Codec>>>),
    /// The `encoding` and `format` fields `setUp()` assigns, wrapped by
    /// `TestUtil.alwaysKnnVectorsFormat(format)` on every call.
    SetUpFormat(Mutex<Option<(ScalarEncoding, Arc<dyn KnnVectorsFormat>)>>),
}

/// A kNN suite.
struct KnnSuite {
    state: KnnSuiteState,
    codec: CodecOf,
    /// `supportsFloatVectorFallback()`.
    fallback: bool,
    /// `TestLucene104ScalarQuantizedVectorsFormat`: the encoding's bits and
    /// `simulateEmptyRawVectors`.
    scalar_quantized: bool,
}

impl KnnSuite {
    fn new(codec: CodecOf, fallback: bool, scalar_quantized: bool) -> Self {
        KnnSuite {
            state: KnnSuiteState::default(),
            codec,
            fallback,
            scalar_quantized,
        }
    }

    /// The `encoding` field `setUp()` assigned.
    fn encoding(&self) -> ScalarEncoding {
        match &self.codec {
            CodecOf::SetUpFormat(f) => {
                f.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .expect("setUp drew the encoding")
                    .0
            }
            _ => unreachable!("only the scalar-quantized suites have an encoding"),
        }
    }
}

impl BaseIndexFileFormatTestCase for KnnSuite {
    fn get_codec(&self) -> Arc<dyn Codec> {
        match &self.codec {
            CodecOf::Default => TestUtil::get_default_codec(),
            CodecOf::Always(make) => TestUtil::always_knn_vectors_format(make()),
            CodecOf::SetUpCodec(c) => c
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
                .expect("setUp built the codec"),
            CodecOf::SetUpFormat(f) => TestUtil::always_knn_vectors_format(Arc::clone(
                &f.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .expect("setUp built the format")
                    .1,
            )),
        }
    }

    fn add_random_fields(&self, doc: &mut Document) {
        self.knn_add_random_fields(doc);
    }

    fn merge_is_stable(&self) -> bool {
        self.knn_merge_is_stable()
    }

    fn as_knn_vectors(&self) -> Option<&dyn BaseKnnVectorsFormatTestCase> {
        Some(self)
    }
}

impl BaseKnnVectorsFormatTestCase for KnnSuite {
    fn knn_state(&self) -> &KnnSuiteState {
        &self.state
    }

    fn supports_float_vector_fallback(&self) -> bool {
        self.fallback
    }

    fn get_quantization_bits(&self) -> i32 {
        if self.scalar_quantized {
            i32::from(self.encoding().get_bits())
        } else {
            8
        }
    }

    fn simulate_empty_raw_vectors(&self, dir: &dyn Directory) -> Result<()> {
        if !self.scalar_quantized {
            return Err(LuceneError::Unchecked {
                class: "java.lang.Exception",
                message: Some(
                    "simulateEmptyRawVectors must be implemented by codecs that support FloatVector fallback"
                        .to_string(),
                ),
                cause: None,
            });
        }
        let index_files = dir.list_all()?;
        let raw_vector_extension = "vec";
        let vector_meta_extension = "vemf";

        for file in &index_files {
            if file.ends_with(&format!(".{raw_vector_extension}")) {
                replace_with_empty_vector_file(dir, file)?;
            } else if file.ends_with(&format!(".{vector_meta_extension}")) {
                update_vector_metadata_file(dir, file)?;
            }
        }
        Ok(())
    }
}

/// `replaceWithEmptyVectorFile(dir, fileName)` (`private`): replaces a raw
/// vector file with an empty one that has valid header/footer.
fn replace_with_empty_vector_file(dir: &dyn Directory, file_name: &str) -> Result<()> {
    let index_header = {
        let mut input = dir.open_input(file_name, &*DEFAULT_IO_CONTEXT)?;
        let header = codec_util::read_index_header(input.as_mut())?;
        input.close()?;
        header
    };
    dir.delete_file(file_name)?;
    let mut out = dir.create_output(file_name, &*DEFAULT_IO_CONTEXT)?;
    // Write header
    out.write_bytes(&index_header, 0, index_header.len())?;
    // Write footer (no content in between)
    codec_util::write_footer(out.as_mut())?;
    out.close()
}

/// `updateVectorMetadataFile(dir, fileName)` (`private`): updates vector
/// metadata file to indicate zero vector length.
fn update_vector_metadata_file(dir: &dyn Directory, file_name: &str) -> Result<()> {
    // Read original metadata
    let (
        index_header,
        field_number,
        vector_encoding,
        vector_similarity_function,
        vector_start_pos,
        dimension,
    ) = {
        let mut input: Box<dyn IndexInput> = dir.open_input(file_name, &*DEFAULT_IO_CONTEXT)?;
        let index_header = codec_util::read_index_header(input.as_mut())?;
        let field_number = input.read_int()?;
        let vector_encoding = input.read_int()?;
        let vector_similarity_function = input.read_int()?;
        let vector_start_pos = input.read_v_long()?;
        input.read_v_long()?; // Skip original vector length
        let dimension = input.read_v_int()?;
        input.close()?;
        (
            index_header,
            field_number,
            vector_encoding,
            vector_similarity_function,
            vector_start_pos,
            dimension,
        )
    };

    // Create updated metadata file
    dir.delete_file(file_name)?;
    let mut out = dir.create_output(file_name, &*DEFAULT_IO_CONTEXT)?;
    // Write header
    out.write_bytes(&index_header, 0, index_header.len())?;

    // Write metadata with zero vector length
    out.write_int(field_number)?;
    out.write_int(vector_encoding)?;
    out.write_int(vector_similarity_function)?;
    out.write_v_long(vector_start_pos)?;
    out.write_v_long(0)?; // Set vector length to 0
    out.write_v_int(dimension)?;
    out.write_int(0)?;

    // Write configuration: `writeStoredMeta(..., out, null, 0, 0, null)`;
    // the vector data and the documents are not read for a count of 0.
    let mut unused_data =
        ByteBuffersDirectory::default().create_output("unused", &*DEFAULT_IO_CONTEXT)?;
    OrdToDocDISIReaderConfiguration::write_stored_meta(
        DIRECT_MONOTONIC_BLOCK_SHIFT,
        out.as_mut(),
        unused_data.as_mut(),
        0,
        0,
        &DocsWithFieldSet::new(),
    )?;

    // Mark end of fields and write footer
    out.write_int(-1)?;
    codec_util::write_footer(out.as_mut())?;
    out.close()
}

// -----------------------------------------------------------------------------
// PagedHnswVectorsFormat (TestKnnVectorsFormatCustomWriter)
// -----------------------------------------------------------------------------

/// `PagedFieldVectorsWriter.VECTORS_PER_PAGE`: fixed at 10 to keep the test
/// deliberately simple.
const VECTORS_PER_PAGE: usize = 10;

/// How a paged writer turns a vector into its little-endian bytes and back.
trait PagedElement: Clone + Send + Sync + 'static {
    /// `serialize(buf, vector)`.
    fn serialize(&self, buf: &mut [u8]);
    /// `deserialize(buf)`.
    fn deserialize(buf: &[u8], dim: usize) -> Self;
}

impl PagedElement for Vec<f32> {
    fn serialize(&self, buf: &mut [u8]) {
        for (i, v) in self.iter().enumerate() {
            buf[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
    }

    fn deserialize(buf: &[u8], dim: usize) -> Self {
        (0..dim)
            .map(|i| f32::from_le_bytes(buf[i * 4..i * 4 + 4].try_into().expect("four bytes")))
            .collect()
    }
}

impl PagedElement for Vec<u8> {
    fn serialize(&self, buf: &mut [u8]) {
        buf[..self.len()].copy_from_slice(self);
    }

    fn deserialize(buf: &[u8], dim: usize) -> Self {
        buf[..dim].to_vec()
    }
}

/// `PagedFieldVectorsWriter<T>`: a `FlatFieldVectorsWriter` that accumulates
/// vectors into a list of fixed-size pages, each holding `VECTORS_PER_PAGE`
/// vectors.
struct PagedFieldVectorsWriter<T> {
    field_info: FieldInfo,
    bytes_per_vector: usize,
    dim: usize,
    docs_with_field: DocsWithFieldSet,
    pages: Vec<Vec<u8>>,
    size: usize,
    last_doc_id: i32,
    finished: bool,
    _element: std::marker::PhantomData<T>,
}

impl<T: PagedElement> PagedFieldVectorsWriter<T> {
    fn new(fi: &FieldInfo, bytes_per_vector: usize) -> Self {
        PagedFieldVectorsWriter {
            field_info: fi.clone(),
            bytes_per_vector,
            dim: fi.vector_dimension as usize,
            docs_with_field: DocsWithFieldSet::new(),
            pages: Vec::new(),
            size: 0,
            last_doc_id: -1,
            finished: false,
            _element: std::marker::PhantomData,
        }
    }

    /// `bufferFor(ord)`: the byte region holding vector `ord`.
    fn buffer_for(&mut self, ord: usize) -> &mut [u8] {
        let page = &mut self.pages[ord / VECTORS_PER_PAGE];
        let offset = (ord % VECTORS_PER_PAGE) * self.bytes_per_vector;
        &mut page[offset..offset + self.bytes_per_vector]
    }
}

/// `PagedFieldVectorsWriter.create(fi)`.
fn create_paged(fi: &FieldInfo) -> Result<FlatFieldVectorWriter> {
    Ok(match fi.vector_encoding {
        VectorEncoding::FLOAT32 => FlatFieldVectorWriter::Float(Arc::new(Mutex::new(
            PagedFieldVectorsWriter::<Vec<f32>>::new(fi, fi.vector_dimension as usize * 4),
        ))),
        VectorEncoding::BYTE => FlatFieldVectorWriter::Byte(Arc::new(Mutex::new(
            PagedFieldVectorsWriter::<Vec<u8>>::new(fi, fi.vector_dimension as usize),
        ))),
    })
}

impl<T: PagedElement> KnnFieldVectorsWriter<T> for PagedFieldVectorsWriter<T> {
    fn add_value(&mut self, doc_id: i32, vector_value: T) -> Result<()> {
        if self.finished {
            return Err(LuceneError::illegal_state(
                "already finished, cannot add more values".to_string(),
            ));
        }
        if doc_id == self.last_doc_id {
            return Err(LuceneError::IllegalArgument(format!(
                "VectorValuesField \"{}\" appears more than once in this document (only one value is allowed per field)",
                self.field_info.name
            )));
        }
        debug_assert!(doc_id > self.last_doc_id);
        self.last_doc_id = doc_id;

        if self.size % VECTORS_PER_PAGE == 0 {
            self.pages
                .push(vec![0u8; VECTORS_PER_PAGE * self.bytes_per_vector]);
        }
        let size = self.size;
        vector_value.serialize(self.buffer_for(size));
        self.docs_with_field.add(doc_id)?;
        self.size += 1;
        Ok(())
    }

    /// Unsupported: this writer owns its storage and copies vector bytes
    /// directly in `add_value`; `copyValue` is never invoked.
    fn copy_value(&self, _vector_value: T) -> T {
        randomizedtesting::jdk::exception::throw_without_message(
            "java.lang.UnsupportedOperationException",
        )
    }

    fn ram_bytes_used(&self) -> i64 {
        RamUsageEstimator::shallow_size_of_instance::<Self>()
            + self.docs_with_field.ram_bytes_used()
            + self.pages.len() as i64
                * (RamUsageEstimator::NUM_BYTES_OBJECT_REF
                    + (VECTORS_PER_PAGE * self.bytes_per_vector) as i64)
    }
}

impl<T: PagedElement> FlatFieldVectorsWriter<T> for PagedFieldVectorsWriter<T> {
    fn vectors(&self) -> Cow<'_, [T]>
    where
        T: Clone,
    {
        // `new AbstractList<>() { get(index) = deserialize(bufferFor(index)) }`
        Cow::Owned(
            (0..self.size)
                .map(|ord| {
                    let page = &self.pages[ord / VECTORS_PER_PAGE];
                    let offset = (ord % VECTORS_PER_PAGE) * self.bytes_per_vector;
                    T::deserialize(&page[offset..offset + self.bytes_per_vector], self.dim)
                })
                .collect(),
        )
    }

    fn docs_with_field_set(&self) -> Cow<'_, DocsWithFieldSet> {
        Cow::Borrowed(&self.docs_with_field)
    }

    fn finish(&mut self) -> Result<()> {
        self.finished = true;
        Ok(())
    }

    fn is_finished(&self) -> bool {
        self.finished
    }
}

/// `PagedHnswVectorsFormat`: an HNSW writer whose flat writer uses the paged
/// storage strategy; reads are delegated to a standard
/// `Lucene99HnswVectorsFormat` — the on-disk format is unchanged.
#[derive(Debug, Default)]
struct PagedHnswVectorsFormat {
    /// `READ_FORMAT`.
    read_format: Lucene99HnswVectorsFormat,
}

impl KnnVectorsFormat for PagedHnswVectorsFormat {
    /// Reuse the registered name so segments written by this format can be
    /// reopened by the standard `Lucene99HnswVectorsFormat` via SPI — the
    /// on-disk bytes are identical.
    fn name(&self) -> &str {
        "Lucene99HnswVectorsFormat"
    }

    fn fields_writer(&self, state: &OwnedSegmentWriteState) -> Result<Box<dyn KnnVectorsWriter>> {
        // Build a flat writer with our paged strategy, and wrap it in the standard HNSW writer.
        let scorer: Arc<dyn FlatVectorsScorer> =
            Arc::new(FlatVectorScorerUtil::get_lucene99_flat_vectors_scorer());
        let flat_writer = Lucene99FlatVectorsWriter::with_field_writer_factory(
            &state.borrow(),
            scorer,
            Arc::new(create_paged),
        )?;
        Ok(Box::new(Lucene99HnswVectorsWriter::with_flat_format(
            state,
            DEFAULT_MAX_CONN,
            DEFAULT_BEAM_WIDTH,
            DEFAULT_NUM_MERGE_WORKER,
            None,
            HNSW_GRAPH_THRESHOLD,
            VERSION_CURRENT,
            Box::new(Lucene99FlatVectorsFormat::new(
                FlatVectorScorerUtil::get_lucene99_flat_vectors_scorer(),
            )),
            Box::new(flat_writer),
        )?))
    }

    fn fields_reader<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn KnnVectorsReader>> {
        self.read_format.fields_reader(state)
    }

    fn get_max_dimensions(&self, _field_name: &str) -> i32 {
        rucene::codecs::knn_vectors::DEFAULT_MAX_DIMENSIONS
    }
}

// -----------------------------------------------------------------------------
// The classes
// -----------------------------------------------------------------------------

fn lucene99_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.lucene99.TestLucene99HnswVectorsFormat")
        .extends(&base_knn_vectors_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(KnnSuite::new(CodecOf::Default, false, false)))
        }))
        .method(knn_test("testToString", |_| lucene99_test_to_string()))
        .method(knn_test("testLimits", |_| lucene99_test_limits()))
        .method(knn_test("testSimpleOffHeapSize", |_| {
            lucene99_test_simple_off_heap_size()
        }))
        .build()
}

/// `new Lucene99HnswVectorsFormat(DEFAULT_MAX_CONN, DEFAULT_BEAM_WIDTH,
/// DEFAULT_NUM_MERGE_WORKER, null, 0)`.
fn v0_format() -> Arc<dyn KnnVectorsFormat> {
    Arc::new(Lucene99HnswVectorsFormat::with_merge_exec_and_threshold(
        DEFAULT_MAX_CONN,
        DEFAULT_BEAM_WIDTH,
        DEFAULT_NUM_MERGE_WORKER,
        None,
        0,
    ))
}

fn lucene99_v0_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.lucene99.TestLucene99HnswVectorsFormatV0")
        .extends(&base_knn_vectors_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(KnnSuite::new(
                CodecOf::Always(v0_format),
                false,
                false,
            )))
        }))
        .build()
}

fn per_field_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.perfield.TestPerFieldKnnVectorsFormat")
        .extends(&base_knn_vectors_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(KnnSuite::new(
                CodecOf::SetUpCodec(Mutex::new(None)),
                false,
                false,
            )))
        }))
        .method(Method::new("setUp").throws(&["java.lang.Exception"]).body(
            |inst: &mut FormatTestInstance| {
                let suite = inst.subclass::<KnnSuite>();
                let random = JdkRandom::new(LuceneTestCase::random().next_long());
                let codec: Arc<dyn Codec> =
                    Arc::new(RandomCodec::new_avoiding(&random, HashSet::new()));
                if let CodecOf::SetUpCodec(c) = &suite.codec {
                    *c.lock().unwrap_or_else(|e| e.into_inner()) = Some(codec);
                }
                set_up(inst);
            },
        ))
        .method(knn_test("testMissingFieldReturnsNoResults", |_| {
            test_missing_field_returns_no_results()
        }))
        .method(knn_test("testTwoFieldsTwoFormats", |_| {
            test_two_fields_two_formats()
        }))
        .method(knn_test("testMergeUsesNewFormat", |_| {
            test_merge_uses_new_format()
        }))
        .method(knn_test("testMaxDimensionsPerFieldFormat", |_| {
            test_max_dimensions_per_field_format()
        }))
        .build()
}

/// `new PagedHnswVectorsFormat()`.
fn paged_format() -> Arc<dyn KnnVectorsFormat> {
    Arc::new(PagedHnswVectorsFormat::default())
}

fn custom_writer_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.lucene99.TestKnnVectorsFormatCustomWriter")
        .extends(&base_knn_vectors_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(KnnSuite::new(
                CodecOf::Always(paged_format),
                false,
                false,
            )))
        }))
        .build()
}

/// The `setUp()` of the scalar-quantized classes: draws the encoding,
/// builds the format with `make`, then `super.setUp()`.
fn scalar_quantized_set_up(make: fn(ScalarEncoding) -> Arc<dyn KnnVectorsFormat>) -> Method {
    Method::new("setUp").throws(&["java.lang.Exception"]).body(
        move |inst: &mut FormatTestInstance| {
            let suite = inst.subclass::<KnnSuite>();
            let encoding_values = ScalarEncoding::VALUES;
            let encoding = encoding_values
                [LuceneTestCase::random().next_int_bound(encoding_values.len() as i32) as usize];
            let format = make(encoding);
            if let CodecOf::SetUpFormat(f) = &suite.codec {
                *f.lock().unwrap_or_else(|e| e.into_inner()) = Some((encoding, format));
            }
            set_up(inst);
        },
    )
}

fn hnsw_scalar_quantized_class() -> Arc<Class> {
    Class::builder(
        "org.apache.lucene.codecs.lucene104.TestLucene104HnswScalarQuantizedVectorsFormat",
    )
    .extends(&base_knn_vectors_format_test_case::class())
    .constructor(Constructor::no_args(|| {
        FormatTestInstance::new(Arc::new(KnnSuite::new(
            CodecOf::SetUpFormat(Mutex::new(None)),
            false,
            false,
        )))
    }))
    .method(scalar_quantized_set_up(|encoding| {
        Arc::new(
            Lucene104HnswScalarQuantizedVectorsFormat::with_merge_exec(
                encoding,
                DEFAULT_MAX_CONN,
                DEFAULT_BEAM_WIDTH,
                1,
                None,
                HNSW_GRAPH_THRESHOLD,
            )
            .unwrap_or_else(|e| rethrow(e)),
        )
    }))
    .method(knn_test("testToString", |_| hnsw_sq_test_to_string()))
    .method(knn_test("testSingleVectorCase", |_| {
        hnsw_sq_test_single_vector_case()
    }))
    .method(knn_test("testLimits", |_| hnsw_sq_test_limits()))
    .method(knn_test("testVectorSimilarityFuncs", |_| {
        hnsw_sq_test_vector_similarity_funcs()
    }))
    .method(knn_test(
        "testSimpleOffHeapSize",
        hnsw_sq_test_simple_off_heap_size,
    ))
    .build()
}

fn scalar_quantized_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.codecs.lucene104.TestLucene104ScalarQuantizedVectorsFormat")
        .extends(&base_knn_vectors_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(KnnSuite::new(
                CodecOf::SetUpFormat(Mutex::new(None)),
                true,
                true,
            )))
        }))
        .method(scalar_quantized_set_up(|encoding| {
            Arc::new(Lucene104ScalarQuantizedVectorsFormat::with_encoding(
                encoding,
            ))
        }))
        .method(Method::new("testRandomWithUpdatesAndGraph").body(
            |_inst: &mut FormatTestInstance| {
                // graph not supported
            },
        ))
        .method(
            Method::new("testSearchWithVisitedLimit").body(|_inst: &mut FormatTestInstance| {
                // visited limit is not respected, as it is brute force search
            }),
        )
        .method(knn_test("testSearch", sq_test_search))
        .method(knn_test("testToString", |_| sq_test_to_string()))
        .method(knn_test(
            "testQuantizedVectorsWriteAndRead",
            sq_test_quantized_vectors_write_and_read,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// The classes' own test methods
// -----------------------------------------------------------------------------

/// A test method a kNN class declares itself: `public void <name>() throws
/// Exception`, run on the class's instance.
fn knn_test(name: &str, body: fn(&KnnSuite) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body(move |inst: &mut FormatTestInstance| {
            let suite = inst.subclass::<KnnSuite>();
            body(&suite).unwrap_or_else(|e| rethrow(e));
        })
}

/// `assertThat(actual, is(oneOf(expected...)))`.
fn assert_is_one_of(actual: &str, expected: &[&str]) {
    if !expected.contains(&actual) {
        let quoted: Vec<String> = expected.iter().map(|e| format!("\"{e}\"")).collect();
        Assert::fail(&format!(
            "\nExpected: is one of {{{}}}\n     but: was \"{actual}\"",
            quoted.join(", ")
        ));
    }
}

/// `assertArrayEquals(expecteds, actuals, delta)` over `float[]`.
fn assert_array_equals_f32(expecteds: &[f32], actuals: &[f32], delta: f32) {
    Assert::assert_equals_msg(
        Some("array lengths differed"),
        expecteds.len(),
        actuals.len(),
    );
    for (i, (e, a)) in expecteds.iter().zip(actuals).enumerate() {
        Assert::assert_equals_float_msg(
            Some(&format!("arrays first differed at element [{i}]")),
            *e,
            *a,
            delta,
        );
    }
}

/// `assertArrayEquals(expecteds, actuals)` over `byte[]`.
fn assert_array_equals_u8(expecteds: &[u8], actuals: &[u8]) {
    Assert::assert_equals_msg(
        Some("array lengths differed"),
        expecteds.len(),
        actuals.len(),
    );
    for (i, (e, a)) in expecteds.iter().zip(actuals).enumerate() {
        Assert::assert_equals_msg(
            Some(&format!("arrays first differed at element [{i}]")),
            *e as i8,
            *a as i8,
        );
    }
}

/// `expectThrows(IllegalArgumentException.class, () -> new Lucene99HnswVectorsFormat(...))`:
/// the constructor throws (panics with) the exception.
fn expect_iae_panic(body: impl FnOnce()) {
    LuceneTestCase::expect_throws("java.lang.IllegalArgumentException", || {
        body();
        Ok(())
    });
}

/// Port of `TestLucene99HnswVectorsFormat.testToString`.
fn lucene99_test_to_string() -> Result<()> {
    let format = Lucene99HnswVectorsFormat::with_max_conn_beam_width(10, 20);
    let rendered = format.to_string();
    let default_codec = get_default().unwrap_or_else(|| {
        Throwable::new("java.lang.NullPointerException", "Codec.getDefault()").throw()
    });
    let _custom_codec = FilterCodec::new("foo", default_codec).with_knn_vectors_format(format);
    let expected_pattern = |scorer: &str| {
        format!(
            "Lucene99HnswVectorsFormat(name=Lucene99HnswVectorsFormat, maxConn=10, beamWidth=20, tinySegmentsThreshold=100, flatVectorFormat=Lucene99FlatVectorsFormat(vectorsScorer={scorer}()))"
        )
    };
    let default_scorer = expected_pattern("DefaultFlatVectorScorer");
    let mem_seg_scorer = expected_pattern("Lucene99MemorySegmentFlatVectorsScorer");
    assert_is_one_of(&rendered, &[&default_scorer, &mem_seg_scorer]);
    Ok(())
}

/// Port of `TestLucene99HnswVectorsFormat.testLimits`.
fn lucene99_test_limits() -> Result<()> {
    expect_iae_panic(|| {
        Lucene99HnswVectorsFormat::with_max_conn_beam_width(-1, 20);
    });
    expect_iae_panic(|| {
        Lucene99HnswVectorsFormat::with_max_conn_beam_width(0, 20);
    });
    expect_iae_panic(|| {
        Lucene99HnswVectorsFormat::with_max_conn_beam_width(20, 0);
    });
    expect_iae_panic(|| {
        Lucene99HnswVectorsFormat::with_max_conn_beam_width(20, -1);
    });
    expect_iae_panic(|| {
        Lucene99HnswVectorsFormat::with_max_conn_beam_width(512 + 1, 20);
    });
    expect_iae_panic(|| {
        Lucene99HnswVectorsFormat::with_max_conn_beam_width(20, 3201);
    });
    expect_iae_panic(|| {
        Lucene99HnswVectorsFormat::with_merge_exec(
            20,
            100,
            1,
            Some(Arc::new(SameThreadExecutorService::new())),
        );
    });
    Ok(())
}

/// The body of both `testSimpleOffHeapSize` methods up to the off-heap map:
/// indexes one `vector` under `DOT_PRODUCT`, and answers, when the only leaf
/// is a `CodecReader`, the field's `getOffHeapByteSize` and its dimension.
fn simple_off_heap_size(vector: &[f32]) -> Result<Option<(HashMap<String, i64>, i32)>> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let mut doc = Document::new();
    doc.add(Box::new(KnnFloatVectorField::new(
        "f",
        vector,
        VectorSimilarityFunction::DOT_PRODUCT,
    )?));
    w.add_document(&doc)?;
    w.commit()?;
    let mut result = None;
    {
        let reader = directory_reader::open_with_writer(&*w)?;
        let r = LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
        if let Some(codec_reader) = Arc::clone(&r).as_codec_reader() {
            let knn_vectors_reader = codec_reader.get_vector_reader()?.unwrap_or_else(|| {
                Throwable::new(
                    "java.lang.NullPointerException",
                    "getVectorReader() is null",
                )
                .throw()
            });
            let mut field_reader: &dyn KnnVectorsReader = &*knn_vectors_reader;
            if let Some(fields_reader) = knn_vectors_reader
                .as_any()
                .and_then(|any| any.downcast_ref::<KnnFieldsReader>())
            {
                field_reader = fields_reader.get_field_reader("f").unwrap_or_else(|| {
                    Throwable::new("java.lang.NullPointerException", "getFieldReader(\"f\")")
                        .throw()
                });
            }
            let field_infos = r.get_field_infos();
            let field_info = field_infos.field_info("f").unwrap_or_else(|| {
                Throwable::new("java.lang.NullPointerException", "fieldInfo(\"f\")").throw()
            });
            let off_heap = field_reader.get_off_heap_byte_size(field_info);
            result = Some((off_heap, field_info.vector_dimension));
        }
        reader.close()?;
    }
    w.close()?;
    dir.close()?;
    Ok(result)
}

/// `offHeap.get(key)` unboxed to `long`: the `NullPointerException` of
/// unboxing `null`.
fn off_heap_get(off_heap: &HashMap<String, i64>, key: &str) -> i64 {
    *off_heap.get(key).unwrap_or_else(|| {
        Throwable::new(
            "java.lang.NullPointerException",
            format!("offHeap.get(\"{key}\")"),
        )
        .throw()
    })
}

/// Port of `TestLucene99HnswVectorsFormat.testSimpleOffHeapSize`.
fn lucene99_test_simple_off_heap_size() -> Result<()> {
    let vector = random_vector(LuceneTestCase::random().next_int_range(12, 500));
    if let Some((off_heap, _)) = simple_off_heap_size(&vector)? {
        Assert::assert_equals(vector.len() as i64 * 4, off_heap_get(&off_heap, "vec"));
        Assert::assert_true(off_heap.contains_key("vex")); // assertNotNull(offHeap.get("vex"))
        Assert::assert_equals(2, off_heap.len());
    }
    Ok(())
}

/// Port of `TestLucene104HnswScalarQuantizedVectorsFormat.testToString`.
fn hnsw_sq_test_to_string() -> Result<()> {
    let format = Lucene104HnswScalarQuantizedVectorsFormat::with_merge_exec(
        ScalarEncoding::UnsignedByte,
        10,
        20,
        1,
        None,
        HNSW_GRAPH_THRESHOLD,
    )?;
    let rendered = format.to_string();
    let default_codec = get_default().unwrap_or_else(|| {
        Throwable::new("java.lang.NullPointerException", "Codec.getDefault()").throw()
    });
    let _custom_codec = FilterCodec::new("foo", default_codec).with_knn_vectors_format(format);
    let expected_pattern = |scorer: &str| {
        format!(
            "Lucene104HnswScalarQuantizedVectorsFormat(name=Lucene104HnswScalarQuantizedVectorsFormat, maxConn=10, beamWidth=20, tinySegmentsThreshold=100, flatVectorFormat=Lucene104ScalarQuantizedVectorsFormat(name=Lucene104ScalarQuantizedVectorsFormat, encoding=UNSIGNED_BYTE, flatVectorScorer=Lucene104ScalarQuantizedVectorScorer(nonQuantizedDelegate={scorer}()), rawVectorFormat=Lucene99FlatVectorsFormat(vectorsScorer={scorer}())))"
        )
    };
    let default_scorer = expected_pattern("DefaultFlatVectorScorer");
    let mem_seg_scorer = expected_pattern("Lucene99MemorySegmentFlatVectorsScorer");
    assert_is_one_of(&rendered, &[&default_scorer, &mem_seg_scorer]);
    Ok(())
}

/// Port of `TestLucene104HnswScalarQuantizedVectorsFormat.testSingleVectorCase`.
fn hnsw_sq_test_single_vector_case() -> Result<()> {
    let vector = random_vector(LuceneTestCase::random().next_int_range(12, 500));
    for similarity_function in VectorSimilarityFunction::values() {
        let mut dir = LuceneTestCase::new_directory();
        let w = IndexWriter::new(
            dir.as_directory(),
            LuceneTestCase::new_index_writer_config(),
        )?;
        let mut doc = Document::new();
        let doc_vector = if similarity_function == VectorSimilarityFunction::DOT_PRODUCT {
            let mut copy = vector.clone();
            vector_util::l2normalize(&mut copy, true)?;
            copy
        } else {
            vector.clone()
        };
        doc.add(Box::new(KnnFloatVectorField::new(
            "f",
            &doc_vector,
            similarity_function,
        )?));
        w.add_document(&doc)?;
        w.commit()?;
        {
            let reader = directory_reader::open_with_writer(&*w)?;
            let r =
                LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
            let vector_values = r.get_float_vector_values("f")?.unwrap_or_else(|| {
                Throwable::new(
                    "java.lang.NullPointerException",
                    "getFloatVectorValues(\"f\")",
                )
                .throw()
            });
            let mut doc_index_iterator = vector_values.iterator()?;
            debug_assert!(vector_values.size() == 1);
            while doc_index_iterator.next_doc()? != NO_MORE_DOCS {
                assert_array_equals_f32(
                    &doc_vector,
                    &vector_values.vector_value(doc_index_iterator.index())?,
                    0.00001,
                );
            }
            let random_vector = if similarity_function == VectorSimilarityFunction::DOT_PRODUCT {
                random_normalized_vector(vector.len() as i32)
            } else {
                random_vector(vector.len() as i32)
            };
            let true_score = similarity_function.compare_f32(&doc_vector, &random_vector)?;
            let mut accept_docs = from_live_docs(None, r.max_doc())?;
            let td = r.search_nearest_vectors_default(
                "f",
                &random_vector,
                1,
                &mut accept_docs,
                i32::MAX,
            )?;
            Assert::assert_equals(1, td.total_hits.value());
            Assert::assert_true(td.score_docs[0].score >= 0.0);
            // When it's the only vector in a segment, the score should be very close to the true
            // score
            Assert::assert_equals_float_msg(None, true_score, td.score_docs[0].score, 0.01);
            drop(vector_values);
            reader.close()?;
        }
        w.close()?;
        dir.close()?;
    }
    Ok(())
}

/// Port of `TestLucene104HnswScalarQuantizedVectorsFormat.testLimits`.
fn hnsw_sq_test_limits() -> Result<()> {
    let iae = "java.lang.IllegalArgumentException";
    let two = |max_conn: i32, beam_width: i32| {
        Lucene104HnswScalarQuantizedVectorsFormat::with_params(
            max_conn,
            beam_width,
            ScalarEncoding::UnsignedByte,
        )
    };
    expect_throws(iae, || two(-1, 20));
    expect_throws(iae, || two(0, 20));
    expect_throws(iae, || two(20, 0));
    expect_throws(iae, || two(20, -1));
    expect_throws(iae, || two(512 + 1, 20));
    expect_throws(iae, || two(20, 3201));
    expect_throws(iae, || {
        Lucene104HnswScalarQuantizedVectorsFormat::with_merge_exec(
            ScalarEncoding::UnsignedByte,
            20,
            100,
            1,
            Some(Arc::new(SameThreadExecutorService::new())),
            HNSW_GRAPH_THRESHOLD,
        )
    });
    Ok(())
}

// Ensures that all expected vector similarity functions are translatable in the format.
/// Port of `TestLucene104HnswScalarQuantizedVectorsFormat.testVectorSimilarityFuncs`.
fn hnsw_sq_test_vector_similarity_funcs() -> Result<()> {
    // This does not necessarily have to be all similarity functions, but
    // differences should be considered carefully.
    let expected_values = VectorSimilarityFunction::values();
    Assert::assert_true(Lucene99HnswVectorsReader::SIMILARITY_FUNCTIONS == expected_values);
    Ok(())
}

/// Port of `TestLucene104HnswScalarQuantizedVectorsFormat.testSimpleOffHeapSize`.
fn hnsw_sq_test_simple_off_heap_size(suite: &KnnSuite) -> Result<()> {
    let vector = random_vector(LuceneTestCase::random().next_int_range(12, 500));
    if let Some((off_heap, dimension)) = simple_off_heap_size(&vector)? {
        Assert::assert_equals(vector.len() as i64 * 4, off_heap_get(&off_heap, "vec"));
        Assert::assert_true(off_heap.contains_key("vex")); // assertNotNull(offHeap.get("vex"))
        let corrections: i64 = 4 + 4 + 4 + 4;
        let expected = i64::from(suite.encoding().get_doc_packed_length(dimension)) + corrections;
        Assert::assert_equals(expected, off_heap_get(&off_heap, "veq"));
        Assert::assert_equals(3, off_heap.len());
    }
    Ok(())
}

/// Port of `TestLucene104ScalarQuantizedVectorsFormat.testSearch`.
fn sq_test_search(suite: &KnnSuite) -> Result<()> {
    let field_name = "field";
    let num_vectors = LuceneTestCase::random().next_int_range(99, 500);
    let dims = LuceneTestCase::random().next_int_range(4, 65);
    let vector = random_vector(dims);
    let similarity_function = suite.random_similarity();
    let mut knn_field = KnnFloatVectorField::new(field_name, &vector, similarity_function)?;
    let iwc = LuceneTestCase::new_index_writer_config();
    let mut dir = LuceneTestCase::new_directory();
    {
        let w = IndexWriter::new(dir.as_directory(), iwc)?;
        for _ in 0..num_vectors {
            let mut doc = Document::new();
            knn_field.set_vector_value(&random_vector(dims))?;
            doc.add(Box::new(knn_field.clone()));
            w.add_document(&doc)?;
        }
        w.commit()?;

        {
            let reader = directory_reader::open_with_writer(&*w)?;
            let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
            let k = LuceneTestCase::random().next_int_range(5, 50);
            let query_vector = random_vector(dims);
            let q: Arc<dyn Query> =
                Arc::new(KnnFloatVectorQuery::new(field_name, query_vector, k)?);
            let collected_docs = searcher.search(q, k)?;
            Assert::assert_equals(i64::from(k), collected_docs.total_hits.value());
            Assert::assert_true(
                TotalHitsRelation::EQUAL_TO == collected_docs.total_hits.relation(),
            );
            reader.close()?;
        }
        w.close()?;
    }
    dir.close()
}

/// Port of `TestLucene104ScalarQuantizedVectorsFormat.testToString`.
fn sq_test_to_string() -> Result<()> {
    let format = Lucene104ScalarQuantizedVectorsFormat::new();
    let rendered = format.to_string();
    let default_codec = get_default().unwrap_or_else(|| {
        Throwable::new("java.lang.NullPointerException", "Codec.getDefault()").throw()
    });
    let _custom_codec = FilterCodec::new("foo", default_codec).with_knn_vectors_format(format);
    let expected_pattern = |scorer: &str| {
        format!(
            "Lucene104ScalarQuantizedVectorsFormat(name=Lucene104ScalarQuantizedVectorsFormat, encoding=UNSIGNED_BYTE, flatVectorScorer=Lucene104ScalarQuantizedVectorScorer(nonQuantizedDelegate={scorer}()), rawVectorFormat=Lucene99FlatVectorsFormat(vectorsScorer={scorer}()))"
        )
    };
    let default_scorer = expected_pattern("DefaultFlatVectorScorer");
    let mem_seg_scorer = expected_pattern("Lucene99MemorySegmentFlatVectorsScorer");
    assert_is_one_of(&rendered, &[&default_scorer, &mem_seg_scorer]);
    Ok(())
}

/// Port of `TestLucene104ScalarQuantizedVectorsFormat.testQuantizedVectorsWriteAndRead`.
///
/// **Divergence.** Java reads the quantized vectors through the very
/// `QuantizedByteVectorValues` `getQuantizedVectorValues()` returns; Rucene's
/// accessor lends it immutably while its reads take `&mut self`, so the test
/// reads them through a `copy()` of it, which holds the same data.
fn sq_test_quantized_vectors_write_and_read(suite: &KnnSuite) -> Result<()> {
    let encoding = suite.encoding();
    let field_name = "field";
    let num_vectors = LuceneTestCase::random().next_int_range(99, 500);
    let dims = LuceneTestCase::random().next_int_range(4, 65);

    let vector = random_vector(dims);
    let similarity_function = suite.random_similarity();
    let mut knn_field = KnnFloatVectorField::new(field_name, &vector, similarity_function)?;
    let mut dir = LuceneTestCase::new_directory();
    {
        let w = IndexWriter::new(
            dir.as_directory(),
            LuceneTestCase::new_index_writer_config(),
        )?;
        for i in 0..num_vectors {
            let mut doc = Document::new();
            knn_field.set_vector_value(&random_vector(dims))?;
            doc.add(Box::new(knn_field.clone()));
            w.add_document(&doc)?;
            if i % 101 == 0 {
                w.commit()?;
            }
        }
        w.commit()?;
        w.force_merge(1)?;

        {
            let reader = directory_reader::open_with_writer(&*w)?;
            let r =
                LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
            let mut vector_values: Box<dyn FloatVectorValues> =
                r.get_float_vector_values(field_name)?.unwrap_or_else(|| {
                    Throwable::new("java.lang.NullPointerException", "getFloatVectorValues").throw()
                });
            Assert::assert_equals(vector_values.size(), num_vectors);
            let mut qvector_values = vector_values
                .as_any()
                .and_then(|any| any.downcast_ref::<ScalarQuantizedVectorValues>())
                .unwrap_or_else(|| {
                    Throwable::new(
                        "java.lang.ClassCastException",
                        "the vector values are not ScalarQuantizedVectorValues",
                    )
                    .throw()
                })
                .get_quantized_vector_values()
                .copy()?;
            let centroid: Vec<f32> = qvector_values
                .get_centroid()
                .unwrap_or_else(|| {
                    Throwable::new("java.lang.NullPointerException", "getCentroid()").throw()
                })
                .to_vec();
            Assert::assert_equals(centroid.len() as i32, dims);

            let quantizer = OptimizedScalarQuantizer::new(similarity_function);
            let mut scratch = vec![0u8; encoding.get_discrete_dimensions(dims) as usize];
            let mut expected_vector =
                vec![0u8; encoding.get_doc_packed_length(scratch.len() as i32) as usize];
            if similarity_function == VectorSimilarityFunction::COSINE {
                vector_values = Box::new(NormalizedFloatVectorValues::new(vector_values)?);
            }
            let mut doc_index_iterator = vector_values.iterator()?;

            while doc_index_iterator.next_doc()? != NO_MORE_DOCS {
                let mut value = vector_values.vector_value(doc_index_iterator.index())?;
                let corrections = quantizer.scalar_quantize(
                    &mut value,
                    &mut scratch,
                    encoding.get_bits(),
                    &centroid,
                )?;
                match encoding {
                    ScalarEncoding::UnsignedByte | ScalarEncoding::SevenBit => {
                        expected_vector[..dims as usize].copy_from_slice(&scratch[..dims as usize])
                    }
                    ScalarEncoding::PackedNibble => {
                        OffHeapScalarQuantizedVectorValues::pack_nibbles(
                            &scratch,
                            &mut expected_vector,
                        )?
                    }
                    ScalarEncoding::SingleBitQueryNibble => {
                        OptimizedScalarQuantizer::pack_as_binary(&scratch, &mut expected_vector)
                    }
                    ScalarEncoding::DibitQueryNibble => {
                        OptimizedScalarQuantizer::transpose_dibit(&scratch, &mut expected_vector)
                    }
                }
                assert_array_equals_u8(
                    &expected_vector,
                    qvector_values.vector_value(doc_index_iterator.index())?,
                );
                let actual_corrections =
                    qvector_values.get_corrective_terms(doc_index_iterator.index())?;
                Assert::assert_equals_float_msg(
                    None,
                    corrections.lower_interval,
                    actual_corrections.lower_interval,
                    0.00001,
                );
                Assert::assert_equals_float_msg(
                    None,
                    corrections.upper_interval,
                    actual_corrections.upper_interval,
                    0.00001,
                );
                Assert::assert_equals_float_msg(
                    None,
                    corrections.additional_correction,
                    actual_corrections.additional_correction,
                    0.00001,
                );
                Assert::assert_equals(
                    corrections.quantized_component_sum,
                    actual_corrections.quantized_component_sum,
                );
            }
            drop(doc_index_iterator);
            drop(vector_values);
            reader.close()?;
        }
        w.close()?;
    }
    dir.close()
}

/// `new AssertingCodec() { getKnnVectorsFormatForField(field) { return field1
/// ? format1 : format2; } }`.
fn two_format_codec(
    format1: Arc<dyn KnnVectorsFormat>,
    format2: Arc<dyn KnnVectorsFormat>,
) -> Arc<dyn Codec> {
    let for_field: KnnVectorsFormatForField = Arc::new(move |field: &str| {
        if "field1" == field {
            Arc::clone(&format1)
        } else {
            Arc::clone(&format2)
        }
    });
    Arc::new(AssertingCodec::with_formats_for_field(
        None,
        None,
        Some(for_field),
    ))
}

/// `searchNearestVectors(field, {1, 2, 3}, 10, AcceptDocs.fromLiveDocs(...),
/// Integer.MAX_VALUE).scoreDocs.length`.
fn nearest_hits(reader: &dyn LeafReader, field: &str) -> Result<usize> {
    let mut accept_docs = from_live_docs(reader.get_live_docs(), reader.max_doc())?;
    Ok(reader
        .search_nearest_vectors_default(field, &[1.0, 2.0, 3.0], 10, &mut accept_docs, i32::MAX)?
        .score_docs
        .len())
}

/// Port of `TestPerFieldKnnVectorsFormat.testMissingFieldReturnsNoResults`.
fn test_missing_field_returns_no_results() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    let mut iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::new(&*LuceneTestCase::random()),
    )));
    let for_field: KnnVectorsFormatForField =
        Arc::new(|_field: &str| TestUtil::get_default_knn_vectors_format());
    iwc.set_codec(Arc::new(AssertingCodec::with_formats_for_field(
        None,
        None,
        Some(for_field),
    )))?;
    {
        let iwriter = IndexWriter::new(directory.as_directory(), iwc)?;
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "id",
            "1",
            Store::YES,
        )));
        iwriter.add_document(&doc)?;
        iwriter.close()?;
    }

    {
        let ireader = directory_reader::open(directory.as_directory())?;
        let reader = Arc::clone(&ireader).leaves()?[0].leaf_reader();
        Assert::assert_equals(0, nearest_hits(&*reader, "missing_field")?);
        Assert::assert_equals(0, nearest_hits(&*reader, "id")?);
        drop(reader);
        ireader.close()?;
    }
    directory.close()
}

/// Port of `TestPerFieldKnnVectorsFormat.testTwoFieldsTwoFormats`.
fn test_two_fields_two_formats() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    // we don't use RandomIndexWriter because it might add more values than we expect !!!!1
    let mut iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::new(&*LuceneTestCase::random()),
    )));
    let format1 = Arc::new(WriteRecordingKnnVectorsFormat::new(
        TestUtil::get_default_knn_vectors_format(),
    ));
    let format2 = Arc::new(WriteRecordingKnnVectorsFormat::new(
        TestUtil::get_default_knn_vectors_format(),
    ));
    iwc.set_codec(two_format_codec(
        Arc::clone(&format1) as Arc<dyn KnnVectorsFormat>,
        Arc::clone(&format2) as Arc<dyn KnnVectorsFormat>,
    ))?;

    {
        let iwriter = IndexWriter::new(directory.as_directory(), iwc)?;
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "id",
            "1",
            Store::YES,
        )));
        doc.add(Box::new(KnnFloatVectorField::with_euclidean(
            "field1",
            &[1.0, 2.0, 3.0],
        )?));
        iwriter.add_document(&doc)?;

        doc.clear();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "id",
            "2",
            Store::YES,
        )));
        doc.add(Box::new(KnnFloatVectorField::with_euclidean(
            "field2",
            &[4.0, 5.0, 6.0],
        )?));
        iwriter.add_document(&doc)?;
        iwriter.close()?;
    }

    // Check that each format was used to write the expected field
    Assert::assert_true(format1.fields_written() == HashSet::from(["field1".to_string()]));
    Assert::assert_true(format2.fields_written() == HashSet::from(["field2".to_string()]));

    // Double-check the vectors were written
    {
        let ireader = directory_reader::open(directory.as_directory())?;
        let reader = Arc::clone(&ireader).leaves()?[0].leaf_reader();
        Assert::assert_equals(1, nearest_hits(&*reader, "field1")?);
        Assert::assert_equals(1, nearest_hits(&*reader, "field2")?);
        drop(reader);
        ireader.close()?;
    }
    directory.close()
}

/// Port of `TestPerFieldKnnVectorsFormat.testMergeUsesNewFormat`.
fn test_merge_uses_new_format() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    let mut initial_config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::new(&*LuceneTestCase::random()),
    )));
    initial_config.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;

    {
        let iw = IndexWriter::new(directory.as_directory(), initial_config)?;
        for _ in 0..3 {
            let mut doc = Document::new();
            doc.add(Box::new(LuceneTestCase::new_text_field(
                "id",
                "1",
                Store::YES,
            )));
            doc.add(Box::new(KnnFloatVectorField::with_euclidean(
                "field1",
                &[1.0, 2.0, 3.0],
            )?));
            doc.add(Box::new(KnnFloatVectorField::with_euclidean(
                "field2",
                &[1.0, 2.0, 3.0],
            )?));
            iw.add_document(&doc)?;
            iw.commit()?;
        }
        iw.close()?;
    }

    let mut new_config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::new(&*LuceneTestCase::random()),
    )));
    let format1 = Arc::new(WriteRecordingKnnVectorsFormat::new(
        TestUtil::get_default_knn_vectors_format(),
    ));
    let format2 = Arc::new(WriteRecordingKnnVectorsFormat::new(
        TestUtil::get_default_knn_vectors_format(),
    ));
    new_config.set_codec(two_format_codec(
        Arc::clone(&format1) as Arc<dyn KnnVectorsFormat>,
        Arc::clone(&format2) as Arc<dyn KnnVectorsFormat>,
    ))?;

    {
        let iw = IndexWriter::new(directory.as_directory(), new_config)?;
        iw.force_merge(1)?;
        iw.close()?;
    }

    // Check that the new format was used while merging
    // `assertThat(fieldsWritten, contains("field1"))`: exactly that one item.
    Assert::assert_true(format1.fields_written() == HashSet::from(["field1".to_string()]));
    Assert::assert_true(format2.fields_written() == HashSet::from(["field2".to_string()]));
    directory.close()
}

/// Port of `TestPerFieldKnnVectorsFormat.testMaxDimensionsPerFieldFormat`.
fn test_max_dimensions_per_field_format() -> Result<()> {
    let mut directory = LuceneTestCase::new_directory();
    let mut iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::new(&*LuceneTestCase::random()),
    )));
    let format1: Arc<dyn KnnVectorsFormat> = Arc::new(KnnVectorsFormatMaxDims32::new(Arc::new(
        Lucene99HnswVectorsFormat::with_max_conn_beam_width(16, 100),
    )));
    let format2: Arc<dyn KnnVectorsFormat> =
        Arc::new(Lucene99HnswVectorsFormat::with_max_conn_beam_width(16, 100));
    iwc.set_codec(two_format_codec(format1, format2))?;
    {
        let writer = IndexWriter::new(directory.as_directory(), iwc)?;
        let mut doc1 = Document::new();
        doc1.add(Box::new(KnnFloatVectorField::with_euclidean(
            "field1", &[0.0; 33],
        )?));
        let exc = expect_throws("java.lang.IllegalArgumentException", || {
            writer.add_document(&doc1)
        });
        Assert::assert_true(
            exc.message()
                .unwrap_or_else(|| {
                    Throwable::new("java.lang.NullPointerException", "getMessage() is null").throw()
                })
                .contains("vector's dimensions must be <= [32]"),
        );

        let mut doc2 = Document::new();
        doc2.add(Box::new(KnnFloatVectorField::with_euclidean(
            "field1", &[0.0; 32],
        )?));
        doc2.add(Box::new(KnnFloatVectorField::with_euclidean(
            "field2", &[0.0; 33],
        )?));
        writer.add_document(&doc2)?;
        writer.close()?;
    }

    // Check that the vectors were written
    {
        let reader = directory_reader::open(directory.as_directory())?;
        let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
        let query1: Arc<dyn Query> =
            Arc::new(KnnFloatVectorQuery::new("field1", vec![0.0; 32], 10)?);
        let top_docs1 = searcher.search(query1, 1)?;
        Assert::assert_equals(1, top_docs1.score_docs.len());

        let query2: Arc<dyn Query> =
            Arc::new(KnnFloatVectorQuery::new("field2", vec![0.0; 33], 10)?);
        let top_docs2 = searcher.search(query2, 1)?;
        Assert::assert_equals(1, top_docs2.score_docs.len());
        reader.close()?;
    }
    directory.close()
}

/// `TestPerFieldKnnVectorsFormat.WriteRecordingKnnVectorsFormat` (`private
/// static class … extends KnnVectorsFormat`): records the fields its writers
/// add or merge.
#[derive(Debug)]
struct WriteRecordingKnnVectorsFormat {
    delegate: Arc<dyn KnnVectorsFormat>,
    fields_written: Arc<Mutex<HashSet<String>>>,
}

impl WriteRecordingKnnVectorsFormat {
    /// `WriteRecordingKnnVectorsFormat(delegate)`: `super(delegate.getName())`.
    fn new(delegate: Arc<dyn KnnVectorsFormat>) -> Self {
        Self {
            delegate,
            fields_written: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// `fieldsWritten`.
    fn fields_written(&self) -> HashSet<String> {
        self.fields_written
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl KnnVectorsFormat for WriteRecordingKnnVectorsFormat {
    fn name(&self) -> &str {
        self.delegate.name()
    }

    fn fields_writer(&self, state: &OwnedSegmentWriteState) -> Result<Box<dyn KnnVectorsWriter>> {
        let writer = self.delegate.fields_writer(state)?;
        Ok(Box::new(WriteRecordingKnnVectorsWriter {
            writer,
            fields_written: Arc::clone(&self.fields_written),
        }))
    }

    fn fields_reader<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn KnnVectorsReader>> {
        self.delegate.fields_reader(state)
    }

    fn get_max_dimensions(&self, _field_name: &str) -> i32 {
        rucene::codecs::knn_vectors::DEFAULT_MAX_DIMENSIONS
    }
}

/// The anonymous `KnnVectorsWriter` of
/// `WriteRecordingKnnVectorsFormat.fieldsWriter`.
#[derive(Debug)]
struct WriteRecordingKnnVectorsWriter {
    writer: Box<dyn KnnVectorsWriter>,
    fields_written: Arc<Mutex<HashSet<String>>>,
}

impl KnnVectorsWriter for WriteRecordingKnnVectorsWriter {
    fn add_field(&mut self, field_info: &FieldInfo) -> Result<FieldVectorWriter> {
        self.fields_written
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(field_info.name.clone());
        self.writer.add_field(field_info)
    }

    fn flush(&mut self, max_doc: i32, sort_map: Option<&SorterDocMap>) -> Result<()> {
        self.writer.flush(max_doc, sort_map)
    }

    fn merge_one_field<'m>(
        &mut self,
        field_info: &FieldInfo,
        merge_state: &'m MergeState,
    ) -> Result<Option<Box<dyn IORunnable + 'm>>> {
        self.fields_written
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(field_info.name.clone());
        self.writer.merge_one_field(field_info, merge_state)
    }

    fn finish(&mut self) -> Result<()> {
        self.writer.finish()
    }

    fn close(&mut self) -> Result<()> {
        self.writer.close()
    }

    fn ram_bytes_used(&self) -> i64 {
        self.writer.ram_bytes_used()
    }
}

/// `TestPerFieldKnnVectorsFormat.KnnVectorsFormatMaxDims32` (`private static
/// class … extends KnnVectorsFormat`): its delegate, limited to 32
/// dimensions.
#[derive(Debug)]
struct KnnVectorsFormatMaxDims32 {
    delegate: Arc<dyn KnnVectorsFormat>,
}

impl KnnVectorsFormatMaxDims32 {
    /// `KnnVectorsFormatMaxDims32(delegate)`: `super(delegate.getName())`.
    fn new(delegate: Arc<dyn KnnVectorsFormat>) -> Self {
        Self { delegate }
    }
}

impl KnnVectorsFormat for KnnVectorsFormatMaxDims32 {
    fn name(&self) -> &str {
        self.delegate.name()
    }

    fn fields_writer(&self, state: &OwnedSegmentWriteState) -> Result<Box<dyn KnnVectorsWriter>> {
        self.delegate.fields_writer(state)
    }

    fn fields_reader<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn KnnVectorsReader>> {
        self.delegate.fields_reader(state)
    }

    fn get_max_dimensions(&self, _field_name: &str) -> i32 {
        32
    }
}

// -----------------------------------------------------------------------------
// The seeded-draw oracle: TestKnnVectorsSuiteDraws
// -----------------------------------------------------------------------------

/// The rows `TestKnnVectorsSuiteDraws` records: `<method>\tnext\t<long>`.
static DRAWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `TestKnnVectorsSuiteDraws` of `BaseFormatSuitesOracle`: the binding of
/// `TestLucene99HnswVectorsFormat`, recording the next `random().nextLong()`
/// after each test and after the class.
fn draws_class() -> Arc<Class> {
    Class::builder("org.apache.lucene.tests.index.TestKnnVectorsSuiteDraws")
        .extends(&base_knn_vectors_format_test_case::class())
        .constructor(Constructor::no_args(|| {
            FormatTestInstance::new(Arc::new(KnnSuite::new(CodecOf::Default, false, false)))
        }))
        .method(
            Method::new("tearDown")
                .throws(&["java.lang.Exception"])
                .body(|inst: &mut FormatTestInstance| {
                    DRAWS.lock().unwrap().push(format!(
                        "{}\tnext\t{}",
                        LuceneTestCase::get_test_name().unwrap_or_default(),
                        LuceneTestCase::random().next_long()
                    ));
                    tear_down(inst);
                }),
        )
        .method(
            Method::new("recordClassDraw")
                .modifiers(modifier::PUBLIC | modifier::STATIC)
                .annotate(randomizedtesting::junit::AfterClass)
                .static_body(|| {
                    DRAWS.lock().unwrap().push(format!(
                        "#class\tnext\t{}",
                        LuceneTestCase::random().next_long()
                    ));
                }),
        )
        .build()
}

/// One suite of this binary at a time: the runner's state is process-wide.
static SUITES: Mutex<()> = Mutex::new(());

/// Seeded-draw parity with Lucene (rmp #354): every row of
/// `base_knn_vectors_draws.tsv` (`BaseFormatSuitesOracle`) must be equal.
#[test]
fn test_knn_vectors_suite_draws_match_lucene() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    suite_draws::assert_draws_match("base_knn_vectors_draws.tsv", draws_class, &DRAWS);
}

#[test]
fn test_lucene99_hnsw_vectors_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(lucene99_class);
}

#[test]
fn test_lucene99_hnsw_vectors_format_v0() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(lucene99_v0_class);
}

#[test]
fn test_per_field_knn_vectors_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(per_field_class);
}

#[test]
fn test_knn_vectors_format_custom_writer() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(custom_writer_class);
}

#[test]
fn test_lucene104_hnsw_scalar_quantized_vectors_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(hnsw_scalar_quantized_class);
}

#[test]
fn test_lucene104_scalar_quantized_vectors_format() {
    let _one = SUITES.lock().unwrap_or_else(|e| e.into_inner());
    run_suite(scalar_quantized_class);
}
