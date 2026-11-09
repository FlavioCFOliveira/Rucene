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
//! classes' own test methods (`testToString`, `testLimits`, …) belong to the
//! core-test port (sprint 7) and are not declared here.
//!
//! Each suite runs once on the seed `tests.seed` names (a random one when it
//! is unset); `RUCENE_SUITE_SEEDS=<n>` runs it on the first `n` seeds of
//! `suite_seeds::SEEDS` instead, reporting the pass counts.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::random::JdkRandom;
use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use rucene::codecs::codec_util;
use rucene::codecs::hnsw::flat_vectors::{
    DocsWithFieldSet, FlatFieldVectorWriter, FlatFieldVectorsWriter, FlatVectorScorerUtil,
    FlatVectorsScorer,
};
use rucene::codecs::knn_vectors::{
    KnnFieldVectorsWriter, KnnVectorsFormat, KnnVectorsReader, KnnVectorsWriter,
};
use rucene::codecs::lucene104::scalar_quantized_vectors::{
    Lucene104HnswScalarQuantizedVectorsFormat, Lucene104ScalarQuantizedVectorsFormat,
    DIRECT_MONOTONIC_BLOCK_SHIFT,
};
use rucene::codecs::lucene95::OrdToDocDISIReaderConfiguration;
use rucene::codecs::lucene99::flat_vectors_format::{
    Lucene99FlatVectorsFormat, Lucene99FlatVectorsWriter,
};
use rucene::codecs::lucene99::hnsw_vectors_format::{
    Lucene99HnswVectorsFormat, Lucene99HnswVectorsWriter, DEFAULT_BEAM_WIDTH, DEFAULT_MAX_CONN,
    DEFAULT_NUM_MERGE_WORKER, HNSW_GRAPH_THRESHOLD, VERSION_CURRENT,
};
use rucene::codecs::state::{OwnedSegmentWriteState, SegmentReadState};
use rucene::codecs::Codec;
use rucene::document::Document;
use rucene::error::{LuceneError, Result};
use rucene::index::field_infos::FieldInfo;
use rucene::index::VectorEncoding;
use rucene::store::{ByteBuffersDirectory, Directory, IndexInput, DEFAULT_IO_CONTEXT};
use rucene::util::quantization::ScalarEncoding;
use rucene::util::RamUsageEstimator;
use rucene_test_framework::index::base_index_file_format_test_case::{
    rethrow, set_up, tear_down, BaseIndexFileFormatTestCase, FormatTestInstance,
};
use rucene_test_framework::index::base_knn_vectors_format_test_case::{
    self, BaseKnnVectorsFormatTestCase, KnnSuiteState,
};
use rucene_test_framework::index::random_codec::RandomCodec;
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
        .build()
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
