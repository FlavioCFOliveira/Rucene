# Task #269 block 1 — `codecs` + `store` no-production-caller verdicts

Input: `list_0b048e6.jsonl` (238 items, subsystem codecs/store). Java citations are relative to `/tmp/lucene1050/lucene/core/src/java/org/apache/lucene/` (java21 sources under `core/src/java21/`).

Final classes: **W** link ported now; **W=** the Java link already exists in Rust through the field or a declared-divergence mechanism (accessor kept, no change); **W-blocked** missing link that needs a larger or divergent change (reported, not done); **P** legitimate API, kept; **D** no Lucene counterpart, removed; **T** test-only, no Lucene counterpart, left in place and reported.

## Counts

Auto: D? 95, P? 65, W? 78  
Final: D 32, P 128, T 18, W 20, W-blocked 7, W= 33

| auto \ final | D | P | T | W | W-blocked | W= |
|---|---|---|---|---|---|---|
| D? | 32 | 41 | 17 | 2 | 0 | 3 |
| P? | 0 | 65 | 0 | 0 | 0 | 0 |
| W? | 0 | 22 | 1 | 18 | 7 | 30 |

## Items

| item | auto | final | action | Java evidence |
|---|---|---|---|---|
| `codecs::FilterCodec` | W? | **P** | none | codecs/FilterCodec.java:31 public abstract class; Lucene104Codec.java:50 names it only in javadoc |
| `codecs::FilterCodec::delegate` | W? | **W=** | none (accessor kept) | FilterCodec.java protected field `delegate` read by its own format getters (FilterCodec.java:79-131); Rust reads the field directly in `impl Codec for FilterCodec` (codecs.rs) |
| `codecs::FilterCodec::new` | P? | **P** | none | FilterCodec.java:66 protected ctor for subclasses |
| `codecs::FilterCodec::with_compound_format` | D? | **P** | none (renamed: Rust idiom for overriding `FilterCodec#compoundFormat()`) | FilterCodec.java:79-131 `compoundFormat()` is overridden by subclasses; Rust has no subclassing, the builder installs the override (declared in FilterCodec docs) |
| `codecs::FilterCodec::with_doc_values_format` | D? | **P** | none (renamed: Rust idiom for overriding `FilterCodec#docValuesFormat()`) | FilterCodec.java:79-131 `docValuesFormat()` is overridden by subclasses; Rust has no subclassing, the builder installs the override (declared in FilterCodec docs) |
| `codecs::FilterCodec::with_field_infos_format` | D? | **P** | none (renamed: Rust idiom for overriding `FilterCodec#fieldInfosFormat()`) | FilterCodec.java:79-131 `fieldInfosFormat()` is overridden by subclasses; Rust has no subclassing, the builder installs the override (declared in FilterCodec docs) |
| `codecs::FilterCodec::with_knn_vectors_format` | D? | **P** | none (renamed: Rust idiom for overriding `FilterCodec#knnVectorsFormat()`) | FilterCodec.java:79-131 `knnVectorsFormat()` is overridden by subclasses; Rust has no subclassing, the builder installs the override (declared in FilterCodec docs) |
| `codecs::FilterCodec::with_live_docs_format` | D? | **P** | none (renamed: Rust idiom for overriding `FilterCodec#liveDocsFormat()`) | FilterCodec.java:79-131 `liveDocsFormat()` is overridden by subclasses; Rust has no subclassing, the builder installs the override (declared in FilterCodec docs) |
| `codecs::FilterCodec::with_norms_format` | D? | **P** | none (renamed: Rust idiom for overriding `FilterCodec#normsFormat()`) | FilterCodec.java:79-131 `normsFormat()` is overridden by subclasses; Rust has no subclassing, the builder installs the override (declared in FilterCodec docs) |
| `codecs::FilterCodec::with_points_format` | D? | **P** | none (renamed: Rust idiom for overriding `FilterCodec#pointsFormat()`) | FilterCodec.java:79-131 `pointsFormat()` is overridden by subclasses; Rust has no subclassing, the builder installs the override (declared in FilterCodec docs) |
| `codecs::FilterCodec::with_postings_format` | D? | **P** | none (renamed: Rust idiom for overriding `FilterCodec#postingsFormat()`) | FilterCodec.java:79-131 `postingsFormat()` is overridden by subclasses; Rust has no subclassing, the builder installs the override (declared in FilterCodec docs) |
| `codecs::FilterCodec::with_segment_info_format` | D? | **P** | none (renamed: Rust idiom for overriding `FilterCodec#segmentInfoFormat()`) | FilterCodec.java:79-131 `segmentInfoFormat()` is overridden by subclasses; Rust has no subclassing, the builder installs the override (declared in FilterCodec docs) |
| `codecs::FilterCodec::with_stored_fields_format` | D? | **P** | none (renamed: Rust idiom for overriding `FilterCodec#storedFieldsFormat()`) | FilterCodec.java:79-131 `storedFieldsFormat()` is overridden by subclasses; Rust has no subclassing, the builder installs the override (declared in FilterCodec docs) |
| `codecs::FilterCodec::with_term_vectors_format` | D? | **P** | none (renamed: Rust idiom for overriding `FilterCodec#termVectorsFormat()`) | FilterCodec.java:79-131 `termVectorsFormat()` is overridden by subclasses; Rust has no subclassing, the builder installs the override (declared in FilterCodec docs) |
| `codecs::available_codecs` | P? | **P** | none | codecs/Codec.java:123 public static; no core caller |
| `codecs::get_default` | W? | **W** | wired: `LiveIndexWriterConfig::with_defaults` now calls `get_default()` (was `default_codec()`, so `set_default` had no effect) | index/LiveIndexWriterConfig.java:131 `codec = Codec.getDefault()`; Codec.java:141 |
| `codecs::reload_codecs` | P? | **P** | none | codecs/Codec.java:136 public static; no core caller |
| `codecs::set_default` | P? | **P** | none | codecs/Codec.java:151 public static; no core caller |
| `codecs::codec_util::read_footer` | D? | **P** | none (name-mapping miss) | codecs/CodecUtil.java:345 public static readFooter; no core caller |
| `codecs::codec_util::read_index_header` | D? | **P** | none (name-mapping miss) | codecs/CodecUtil.java:320 public static readIndexHeader; no core caller |
| `codecs::codec_util::verify_and_copy_index_header` | D? | **W** | wired: Lucene90 compound writer calls it; its private duplicate in lucene90/compound.rs removed | codecs/CodecUtil.java:274; caller codecs/lucene90/Lucene90CompoundFormat.java:141 |
| `codecs::compound::CompoundFileDirectory` | D? | **D** | removed (with private CompoundSliceInput and its own unit tests) | no Lucene class; the reader is Lucene90CompoundReader (codecs/lucene90/Lucene90CompoundReader.java), ported in lucene90/compound.rs |
| `codecs::compound::CompoundFileDirectory::from_input` | D? | **D** | removed (with private CompoundSliceInput and its own unit tests) | no Lucene class; the reader is Lucene90CompoundReader (codecs/lucene90/Lucene90CompoundReader.java), ported in lucene90/compound.rs |
| `codecs::compound::CompoundFileDirectory::read_entry_table` | D? | **D** | removed (with private CompoundSliceInput and its own unit tests) | no Lucene class; the reader is Lucene90CompoundReader (codecs/lucene90/Lucene90CompoundReader.java), ported in lucene90/compound.rs |
| `codecs::compound::CompoundFileDirectory::write_entry_table` | D? | **D** | removed (with private CompoundSliceInput and its own unit tests) | no Lucene class; the reader is Lucene90CompoundReader (codecs/lucene90/Lucene90CompoundReader.java), ported in lucene90/compound.rs |
| `codecs::compound::EmptyCompoundFormat` | D? | **D** | removed (with its own unit test) | no Lucene counterpart (CompoundFormat has no EMPTY) |
| `codecs::doc_values::EmptyDocValuesFormat::new` | D? | **T** | none (already #[cfg(test)] pub(crate); reported) | no Lucene counterpart; test-only fixture |
| `codecs::doc_values::available_doc_values_formats` | P? | **P** | none | codecs/DocValuesFormat.java:110 public static |
| `codecs::doc_values::register_doc_values_format` | P? | **P** | none (Rust idiom for SPI registration) | codecs/DocValuesFormat.java:46-53 NamedSPILoader service lookup; used by portability tests |
| `codecs::doc_values::reload_doc_values_formats` | P? | **P** | none | codecs/DocValuesFormat.java:125 public static |
| `codecs::doc_values::singleton_view` | P? | **P** | none | codecs/DocValuesConsumer.java:1085 public static; no core caller |
| `codecs::field_infos::EmptyFieldInfosFormat` | D? | **D** | removed (with its own unit test) | no Lucene counterpart |
| `codecs::hnsw::flat_vectors::DefaultFlatFieldVectorsWriter::add_docs_with_no_vector` | D? | **D** | removed | no member in Lucene99FlatVectorsWriter.DefaultFieldWriter (codecs/lucene99/Lucene99FlatVectorsWriter.java:362-440) |
| `codecs::hnsw::flat_vectors::DefaultFlatFieldVectorsWriter::field_info` | D? | **D** | removed (field kept) | Lucene99FlatVectorsWriter.java:367 `private final FieldInfo fieldInfo`, no getter |
| `codecs::hnsw::flat_vectors::FlatVectorScorerUtil::get_lucene99_scalar_quantized_vectors_scorer` | P? | **P** | none | codecs/hnsw/FlatVectorScorerUtil.java public static getLucene99ScalarQuantizedVectorsScorer() |
| `codecs::hnsw::flat_vectors::check_dimensions` | W? | **W** | wired: Lucene99ScalarQuantizedVectorScorer::from_vector_similarity (Java argument order), Lucene104 quantized float scorer and byte scorer, Lucene99MemorySegmentFlatVectorsScorer; private duplicates removed | codecs/hnsw/FlatVectorsScorer.java checkDimensions; callers Lucene99ScalarQuantizedVectorScorer.java:99, Lucene104ScalarQuantizedVectorScorer.java:64, :106, java21 Lucene99MemorySegmentFlatVectorsScorer |
| `codecs::hnsw::graph_provider::PrefetchableFlatVectorScorer` | P? | **P** | none | codecs/hnsw/PrefetchableFlatVectorScorer.java:40 public class |
| `codecs::hnsw::graph_provider::PrefetchableFlatVectorScorer::get_delegate` | D? | **D** | removed | PrefetchableFlatVectorScorer.java:42 `private final FlatVectorsScorer flatVectorsScorer`, no getter |
| `codecs::hnsw::graph_provider::PrefetchableFlatVectorScorer::new` | P? | **P** | none | PrefetchableFlatVectorScorer.java:49 public ctor |
| `codecs::hnsw::scalar_quantized_scorer::ScalarQuantizedVectorScorer` | W? | **P** | none | codecs/hnsw/ScalarQuantizedVectorScorer.java:37 public class; Lucene99ScalarQuantizedVectorScorer/MemorySegment scorer only reuse its static helpers |
| `codecs::hnsw::scalar_quantized_scorer::ScalarQuantizedVectorScorer::get_non_quantized_delegate` | D? | **D** | removed | ScalarQuantizedVectorScorer.java:56 private field `nonQuantizedDelegate`, no getter |
| `codecs::hnsw::scalar_quantized_scorer::ScalarQuantizedVectorScorer::new` | P? | **P** | none | ScalarQuantizedVectorScorer.java:58 public ctor |
| `codecs::knn_vectors::ByteVectorValuesSub::into_bit_set` | D? | **P** | none (faithful member; uncalled in Lucene too) | codecs/KnnVectorsWriter.java:209 ByteVectorValuesSub.intoBitSet, no caller in core |
| `codecs::knn_vectors::EmptyByteKnnFieldVectorsWriter` | D? | **T** | none (fixture of #[cfg(test)] EmptyBufferingKnnVectorsWriter; reported) | no Lucene counterpart |
| `codecs::knn_vectors::EmptyKnnCollector` | D? | **T** | none (fixture of knn_vectors reader tests; reported) | no Lucene counterpart |
| `codecs::knn_vectors::EmptyKnnFieldVectorsWriter` | D? | **T** | none (fixture of #[cfg(test)] EmptyBufferingKnnVectorsWriter; reported) | no Lucene counterpart |
| `codecs::knn_vectors::EmptyKnnVectorsFormat` | D? | **P** | none (renamed: KnnVectorsFormat.EMPTY) | codecs/KnnVectorsFormat.java:118-167 public static final EMPTY |
| `codecs::knn_vectors::EmptyKnnVectorsFormat::new` | D? | **P** | none (renamed: KnnVectorsFormat.EMPTY construction) | codecs/KnnVectorsFormat.java:118-119 `new KnnVectorsFormat("EMPTY")` |
| `codecs::knn_vectors::EmptyKnnVectorsWriter` | D? | **T** | none (fixture of codecs.rs DummyFormat test codec; reported) | no Lucene counterpart (KnnVectorsFormat.java:120-123 throws instead of returning a writer) |
| `codecs::knn_vectors::FieldVectorWriter::ram_bytes_used` | D? | **D** | removed | Rust-only enum; no Lucene counterpart, no caller |
| `codecs::knn_vectors::FloatVectorValuesSub::into_bit_set` | D? | **P** | none (faithful member; uncalled in Lucene too) | codecs/KnnVectorsWriter.java:178 FloatVectorValuesSub.intoBitSet, no caller in core |
| `codecs::knn_vectors::available_knn_vectors_formats` | P? | **P** | none | codecs/KnnVectorsFormat.java availableKnnVectorsFormats() public static |
| `codecs::knn_vectors::reload_knn_vectors_format` | P? | **P** | none | codecs/KnnVectorsFormat.java reloadKnnVectorsFormat(ClassLoader) public static |
| `codecs::live_docs::EmptyLiveDocsFormat` | D? | **D** | removed (with its own unit test) | no Lucene counterpart |
| `codecs::lucene103::blocktree::Lucene103BlockTreeTermsReader::postings_reader` | W? | **W=** | none (accessor kept) | Lucene103BlockTreeTermsReader.java field `postingsReader`; the enums/frames reach the same Arc through FieldReader/BlockTreeShared |
| `codecs::lucene103::blocktree::Lucene103BlockTreeTermsReader::segment` | W? | **W=** | none (accessor kept) | Lucene103BlockTreeTermsReader.java:102 `final String segment`; FieldReader.toString and SegmentTermsEnum.computeBlockStats (SegmentTermsEnum.java:89) read it, Rust reads `shared.segment` (field_reader.rs Display, segment_terms_enum.rs compute_block_stats) |
| `codecs::lucene103::blocktree::Lucene103BlockTreeTermsReader::segment_suffix` | D? | **D** | removed (field kept) | Lucene103BlockTreeTermsReader.java:110-152 uses `state.segmentSuffix`, no field/getter |
| `codecs::lucene103::blocktree::Lucene103BlockTreeTermsReader::shared` | D? | **D** | removed | Rust-only BlockTreeShared; no Lucene counterpart |
| `codecs::lucene103::blocktree::Lucene103BlockTreeTermsReader::version` | W? | **W=** | none (accessor kept) | Lucene103BlockTreeTermsReader.java:104 `final int version`, read only in the ctor (:118-152); Rust ctor uses the local |
| `codecs::lucene103::blocktree::TrieBuilder::get_empty_output` | W? | **W** | wired: `TermsWriter::finish` asserts `root.index.getEmptyOutput() != null` (debug_assert) | codecs/lucene103/blocktree/Lucene103BlockTreeTermsWriter.java:1079; TrieBuilder.java:167 |
| `codecs::lucene103::blocktree::TrieBuilder::new` | W? | **T** | none (no Lucene counterpart; used by blocktree visit test; reported) | TrieBuilder.java:93-97 has only private ctor(k, v) + static bytesRefToTrie; Rust `new(fp, has_terms)` is a helper over bytes_ref_to_trie |
| `codecs::lucene103::blocktree::TrieBuilder::visit` | D? | **P** | none (faithful member; Lucene calls it only from tests) | codecs/lucene103/blocktree/TrieBuilder.java:686 |
| `codecs::lucene103::field_reader::FieldReader::root_fp` | W? | **W=** | none (accessor kept) | FieldReader.java field rootFP read by newReader(); Rust field_reader.rs:184 reads the field |
| `codecs::lucene103::intersect_terms_enum::IntersectTermsEnumFrame::suffix_slice` | D? | **D** | removed | no member in IntersectTermsEnumFrame.java (fields suffixBytes/startBytePos/suffix only) |
| `codecs::lucene103::segment_terms_enum::SegmentTermsEnum::block_term_state` | D? | **D** | removed | no member in SegmentTermsEnum.java |
| `codecs::lucene103::segment_terms_enum::SegmentTermsEnum::compute_block_stats` | W? | **W** | wired: `impl Terms for FieldReader` now overrides `stats()` = `SegmentTermsEnum::new(self.clone())?.compute_block_stats()?.to_string()` | codecs/lucene103/blocktree/FieldReader.java:110-114 getStats(); caller index/CheckIndex.java:2199 |
| `codecs::lucene103::segment_terms_enum::SegmentTermsEnum::field_reader` | D? | **W=** | none (accessor kept) | SegmentTermsEnum.java:43 `final FieldReader fr`, used by its frames; Rust uses `self.field` |
| `codecs::lucene103::segment_terms_enum::SegmentTermsEnum::index_input_initialized` | D? | **T** | none (probe of blocktree lazy-clone test; reported) | no Lucene counterpart (SegmentTermsEnum.java initIndexInput has no query) |
| `codecs::lucene103::segment_terms_enum::SegmentTermsEnum::prepare_seek_exact` | W? | **W** | wired: `impl TermsEnum for SegmentTermsEnum` now overrides `prepare_seek_exact` (trait default used before) | SegmentTermsEnum.java:503-506 overrides TermsEnum.prepareSeekExact (range check :253-254); callers index/TermStates.java build/get, FilterLeafReader, FuzzyTermsEnum, CheckIndex |
| `codecs::lucene103::segment_terms_enum::SegmentTermsEnum::print_seek_state` | D? | **P** | none (faithful private debug member; uncalled in Lucene) | SegmentTermsEnum.java:757 private printSeekState, only commented-out calls |
| `codecs::lucene103::trie_reader::TrieReader::input` | W? | **W=** | none (accessor kept) | TrieReader.java:75 `final IndexInput input` read by Node.floorData (:69-70); Rust trie_reader.rs floor_data reads the field |
| `codecs::lucene104::codec::Lucene104Codec::get_doc_values_format_for_field` | W? | **W=** | none | Lucene104Codec.java:91-97 anonymous PerFieldDocValuesFormat calls it; Rust builds the per-field format from a resolver returning the same default (declared divergence per_field.rs:86-95) |
| `codecs::lucene104::codec::Lucene104Codec::get_knn_vectors_format_for_field` | W? | **W=** | none | Lucene104Codec.java:100-106; same resolver mechanism |
| `codecs::lucene104::codec::Lucene104Codec::get_postings_format_for_field` | W? | **W=** | none | Lucene104Codec.java:82-88; same resolver mechanism |
| `codecs::lucene104::codec::Lucene104Codec::mode` | D? | **D** | removed (with the field and its own unit test) | Lucene104Codec.java:115-123 does not store or expose the mode |
| `codecs::lucene104::posting_index_input::PostingIndexInput` | P? | **P** | none | codecs/lucene104/PostingIndexInput.java public class |
| `codecs::lucene104::posting_index_input::PostingIndexInput::decode` | P? | **P** | none | PostingIndexInput.java decode(int,int[]) public |
| `codecs::lucene104::posting_index_input::PostingIndexInput::new` | P? | **P** | none | PostingIndexInput.java public ctor |
| `codecs::lucene104::postings_format::Lucene104PostingsFormat::get_impacts_enum_impl` | W? | **P** | none | Lucene104PostingsFormat.java:361 public static; its caller search/ScorerUtil.java:33 is a declared divergence in search/scorer_util.rs:136-149 |
| `codecs::lucene104::postings_format::Lucene104PostingsFormat::max_term_block_size` | W? | **W=** | none (accessor kept) | Lucene104PostingsFormat.java field maxTermBlockSize read by fieldsConsumer (:410-416); Rust fields_consumer reads the field |
| `codecs::lucene104::postings_format::Lucene104PostingsFormat::min_term_block_size` | W? | **W=** | none (accessor kept) | Lucene104PostingsFormat.java field minTermBlockSize read by fieldsConsumer (:410-416) |
| `codecs::lucene104::postings_format::Lucene104PostingsFormat::version` | W? | **W=** | none (accessor kept) | Lucene104PostingsFormat.java field version read by fieldsConsumer (:411) |
| `codecs::lucene104::postings_format::Lucene104PostingsFormat::with_version` | D? | **P** | none (renamed ctor) | Lucene104PostingsFormat.java:398 public expert ctor(int,int,int) |
| `codecs::lucene104::postings_writer::Lucene104PostingsWriter::new` | W? | **P** | none | Lucene104PostingsWriter(SegmentWriteState) public ctor; fieldsConsumer uses the (state, version) ctor = Rust with_version |
| `codecs::lucene104::scalar_quantized_vectors::FieldEntry::byte_size` | D? | **D** | removed | no FieldEntry member; Lucene104ScalarQuantizedVectorsReader.java:171-172 computes the stride inline |
| `codecs::lucene104::scalar_quantized_vectors::Lucene104HnswScalarQuantizedVectorsFormat::beam_width` | W? | **W=** | none (accessor kept) | Lucene104HnswScalarQuantizedVectorsFormat.java field beamWidth read by fieldsWriter/toString; Rust reads the field |
| `codecs::lucene104::scalar_quantized_vectors::Lucene104HnswScalarQuantizedVectorsFormat::flat_format` | D? | **W=** | none (accessor kept) | Lucene104HnswScalarQuantizedVectorsFormat.java:64 field flatVectorsFormat read at :196-197, 205, 222; Rust reads `self.flat` |
| `codecs::lucene104::scalar_quantized_vectors::Lucene104HnswScalarQuantizedVectorsFormat::max_conn` | W? | **W=** | none (accessor kept) | Lucene104HnswScalarQuantizedVectorsFormat.java field maxConn read by fieldsWriter/toString |
| `codecs::lucene104::scalar_quantized_vectors::Lucene104HnswScalarQuantizedVectorsFormat::with_merge_exec` | D? | **P** | none (renamed ctor) | Lucene104HnswScalarQuantizedVectorsFormat.java:133-140, 152-188 public ctors |
| `codecs::lucene104::scalar_quantized_vectors::Lucene104ScalarQuantizedVectorsFormat::encoding` | W? | **W=** | none (accessor kept) | Lucene104ScalarQuantizedVectorsFormat.java field encoding read by fieldsWriter/toString; Rust reads the field |
| `codecs::lucene104::scalar_quantized_vectors::Lucene104ScalarQuantizedVectorsFormat::name` | P? | **P** | none (renamed: NAME constant) | Lucene104ScalarQuantizedVectorsFormat.java public static final NAME |
| `codecs::lucene104::scalar_quantized_vectors::Lucene104ScalarQuantizedVectorsFormat::quantizer` | D? | **D** | removed | no member in Lucene104ScalarQuantizedVectorsFormat.java |
| `codecs::lucene104::scalar_quantized_vectors::Lucene104ScalarQuantizedVectorsReader::field_entry` | D? | **D** | removed | Lucene104ScalarQuantizedVectorsReader.java `fields` map is private, no per-field getter |
| `codecs::lucene104::scalar_quantized_vectors::Lucene104ScalarQuantizedVectorsReader::from_parts` | D? | **D** | removed | not a Lucene ctor (doc said so); no caller |
| `codecs::lucene104::scalar_quantized_vectors::Lucene104ScalarQuantizedVectorsReader::get_centroid` | W? | **W-blocked** | blocked: needs a downcast hook (new divergence); reported | Lucene104ScalarQuantizedVectorsWriter.java:388-396 getCentroid calls reader.getCentroid(fieldName); Rust uses the declared divergence at scalar_quantized_vectors.rs (writer get_centroid, values.get_centroid()) |
| `codecs::lucene104::scalar_quantized_vectors::Lucene104ScalarQuantizedVectorsReader::scorer` | D? | **W=** | none (accessor kept) | Lucene104ScalarQuantizedVectorsReader.java private field vectorScorer read by its own methods; Rust reads the field |
| `codecs::lucene104::scalar_quantized_vectors::Lucene104ScalarQuantizedVectorsWriter::encoding` | W? | **W=** | none (accessor kept) | Lucene104ScalarQuantizedVectorsWriter.java field encoding; Rust methods read `self.encoding` |
| `codecs::lucene104::scalar_quantized_vectors::Lucene104ScalarQuantizedVectorsWriter::with_encoding` | D? | **D** | removed | not a Lucene ctor (doc said so); no caller after the wiring |
| `codecs::lucene104::scalar_quantized_vectors::Lucene104ScalarQuantizedVectorsWriter::write_meta` | W? | **W** | wired: writeField, writeSortingField and mergeOneFlatVectorField now call it; free fn write_meta_entry removed | Lucene104ScalarQuantizedVectorsWriter.java writeMeta; callers writeField, writeSortingField, mergeOneFlatVectorField |
| `codecs::lucene104::scalar_quantized_vectors::Lucene104ScalarQuantizedVectorsWriter::write_sorted_vectors` | W? | **W** | wired: write_sorting_field calls it; free fn write_sorted_quantized_vectors removed | Lucene104ScalarQuantizedVectorsWriter.java writeSortedVectors; caller writeSortingField |
| `codecs::lucene104::scalar_quantized_vectors::Lucene104ScalarQuantizedVectorsWriter::write_sorting_field` | W? | **W** | wired: flush(sortMap != null) calls it; free fn write_sorting_field_entry removed | Lucene104ScalarQuantizedVectorsWriter.java writeSortingField; caller flush(int, Sorter.DocMap) |
| `codecs::lucene104::scalar_quantized_vectors::Lucene104ScalarQuantizedVectorsWriter::write_vectors` | W? | **W** | wired: write_field calls it; free fn write_quantized_vectors removed | Lucene104ScalarQuantizedVectorsWriter.java writeVectors; caller writeField |
| `codecs::lucene104::scalar_quantized_vectors::OffHeapScalarQuantizedVectorValues::encoding` | W? | **W=** | none (accessor kept) | OffHeapScalarQuantizedVectorValues.java:53 field encoding read by getScalarEncoding/copy; Rust reads entry.scalar_encoding |
| `codecs::lucene104::scalar_quantized_vectors::OffHeapScalarQuantizedVectorValues::get_vector_byte_length` | W? | **P** | none (follow-up: reached through `QuantizedByteVectorValues::vector_byte_length`) | OffHeapScalarQuantizedVectorValues.java:176 overrides KnnVectorValues.getVectorByteLength |
| `codecs::lucene104::scalar_quantized_vectors::ScalarQuantizedVectorValues::get_quantized_vector_values` | D? | **P** | none (faithful member; uncalled in Lucene) | Lucene104ScalarQuantizedVectorsReader.java:670 package-private getQuantizedVectorValues(), no caller |
| `codecs::lucene90::indexed_disi::IndexedDISI::as_doc_index_iterator` | W? | **W** | wired: sparse iterator() of OffHeapScalarQuantizedVectorValues, OffHeapScalarQuantizedFloatVectorValues and lucene99 off-heap float/byte values | codecs/lucene90/IndexedDISI.java asDocIndexIterator; callers OffHeapScalarQuantizedVectorValues.java:423, OffHeapScalarQuantizedFloatVectorValues.java:338, lucene95/OffHeapFloatVectorValues.java:273, OffHeapByteVectorValues.java:259 |
| `codecs::lucene90::stored_fields::Lucene90CompressingStoredFieldsReader::get_num_docs` | D? | **P** | none (faithful member; uncalled in Lucene) | Lucene90CompressingStoredFieldsReader.java:759 package-private getNumDocs(), no caller |
| `codecs::lucene90::stored_fields::declare_no_fields_index_chunks` | D? | **T** | none (already #[cfg(test)]; reported) | no Lucene counterpart; corrupt-index fixture |
| `codecs::lucene90::term_vectors::Lucene90CompressingTermVectorsReader::get_num_docs` | D? | **P** | none (faithful member; uncalled in Lucene) | Lucene90CompressingTermVectorsReader.java:301 package-private getNumDocs(), no caller |
| `codecs::lucene95::OrdToDocDISIReaderConfiguration::get_direct_monotonic_reader` | W? | **W** | wired: the four off-heap sparse values ctors call it; it now returns the shared reader (declared divergence moved here); read_ord_to_doc_reader/read_ord_to_doc_data removed | codecs/lucene95/OrdToDocDISIReaderConfiguration.java:207-212; callers OffHeapScalarQuantizedVectorValues.java:377, OffHeapScalarQuantizedFloatVectorValues, lucene95/OffHeapFloatVectorValues.java:225, OffHeapByteVectorValues |
| `codecs::lucene99::hnsw_vectors_format::Lucene99HnswVectorsFormat::with_max_conn_beam_width` | D? | **P** | none (renamed ctor) | Lucene99HnswVectorsFormat.java public ctor(int maxConn, int beamWidth) |
| `codecs::lucene99::hnsw_vectors_format::Lucene99HnswVectorsFormat::with_merge_exec` | D? | **P** | none (renamed ctor) | Lucene99HnswVectorsFormat.java public ctor(int,int,int,ExecutorService) |
| `codecs::lucene99::hnsw_vectors_format::Lucene99HnswVectorsFormat::with_merge_exec_and_threshold` | D? | **P** | none (renamed ctor) | Lucene99HnswVectorsFormat.java public ctor(int,int,int,ExecutorService,int) |
| `codecs::lucene99::hnsw_vectors_format::Lucene99HnswVectorsReader::get_quantization_state` | W? | **P** | none | Lucene99HnswVectorsReader.java:438 public QuantizedVectorsReader method; only instanceof caller in core is Lucene99HnswVectorsWriter.java:436 on the flat reader, not this one |
| `codecs::lucene99::hnsw_vectors_format::Lucene99HnswVectorsReader::get_quantized_vector_values` | W? | **P** | none | Lucene99HnswVectorsReader.java:430 public; no core caller |
| `codecs::lucene99::hnsw_vectors_format::Lucene99HnswVectorsReader::get_random_vector_scorer_supplier_for_merge` | W? | **P** | none | Lucene99HnswVectorsReader.java:446 public; Lucene99HnswVectorsWriter.java:439 calls it on the flat reader (wired in Rust at hnsw_vectors_format.rs merge) |
| `codecs::norms::EmptyNormsDocValues` | D? | **D** | removed (alias; with its own unit test) | no Lucene counterpart |
| `codecs::norms::EmptyNormsFormat::new` | D? | **T** | none (already #[cfg(test)]; reported) | no Lucene counterpart |
| `codecs::per_field::PerFieldDocValuesFormat::get_doc_values_format_for_field` | W? | **W=** | none | perfield/PerFieldDocValuesFormat.java FieldsWriter.getInstance calls getDocValuesFormatForField; Rust DvFieldsWriter calls the same resolver the method calls (declared divergence per_field.rs:586-597) |
| `codecs::per_field::PerFieldPostingsFormat::get_postings_format_for_field` | W? | **W=** | none | perfield/PerFieldPostingsFormat.java FieldsWriter.buildFieldsGroupMapping calls getPostingsFormatForField; Rust FieldsWriter::build_groups calls the same resolver (declared divergence per_field.rs:86-97) |
| `codecs::points::EmptyPointsFormat` | D? | **P** | none (renamed: PointsFormat.EMPTY) | codecs/PointsFormat.java:47-70 public static final EMPTY |
| `codecs::points::EmptyPointsFormat::new` | D? | **P** | none (renamed: PointsFormat.EMPTY construction) | codecs/PointsFormat.java:47-48 |
| `codecs::points::EmptyPointsWriter` | D? | **T** | none (fixture of codecs.rs DummyFormat test codec; reported) | no Lucene counterpart (PointsFormat.java:49-52 throws instead) |
| `codecs::postings::PostingsFormatRegistry::register` | D? | **D** | removed (with its own two unit tests) | no Lucene counterpart; register_arc is the reload path (NamedSPILoader.reload) |
| `codecs::postings::available_postings_formats` | P? | **P** | none | codecs/PostingsFormat.java availablePostingsFormats() public static |
| `codecs::postings::reload_postings_formats` | P? | **P** | none | codecs/PostingsFormat.java reloadPostingsFormats(ClassLoader) public static |
| `codecs::segment_info::EmptySegmentInfoFormat` | D? | **D** | removed (with its own unit test) | no Lucene counterpart |
| `codecs::skip_list::MultiLevelSkipListReader` | P? | **P** | none | codecs/MultiLevelSkipListReader.java public class; no core caller (Lucene104 does not use it) |
| `codecs::skip_list::MultiLevelSkipListReader::doc` | P? | **P** | none | codecs/MultiLevelSkipListReader.java public getDoc(); no core caller (Lucene104 does not use it) |
| `codecs::skip_list::MultiLevelSkipListReader::init` | P? | **P** | none | codecs/MultiLevelSkipListReader.java public init(long,int); no core caller (Lucene104 does not use it) |
| `codecs::skip_list::MultiLevelSkipListReader::new_same_multiplier` | D? | **P** | none (renamed ctor) | codecs/MultiLevelSkipListReader.java ctor(IndexInput,int maxSkipLevels,int skipInterval) = this(.., skipInterval, skipInterval) |
| `codecs::skip_list::MultiLevelSkipListReader::skip_to` | P? | **P** | none | codecs/MultiLevelSkipListReader.java public skipTo(int); no core caller (Lucene104 does not use it) |
| `codecs::skip_list::MultiLevelSkipListWriter` | P? | **P** | none | codecs/MultiLevelSkipListWriter.java public class; no core caller (Lucene104 does not use it) |
| `codecs::skip_list::MultiLevelSkipListWriter::buffer_skip` | P? | **P** | none | codecs/MultiLevelSkipListWriter.java public bufferSkip(int); no core caller (Lucene104 does not use it) |
| `codecs::skip_list::MultiLevelSkipListWriter::new_same_multiplier` | D? | **P** | none (renamed ctor) | codecs/MultiLevelSkipListWriter.java ctor(int skipInterval,int maxSkipLevels,int df) = this(skipInterval, skipInterval, ..) |
| `codecs::skip_list::MultiLevelSkipListWriter::number_of_skip_levels` | W? | **W=** | none (accessor kept) | MultiLevelSkipListWriter.java field numberOfSkipLevels read by bufferSkip/init/writeSkip; Rust reads the field |
| `codecs::skip_list::MultiLevelSkipListWriter::reset_skip` | P? | **P** | none | codecs/MultiLevelSkipListWriter.java public resetSkip(); no core caller (Lucene104 does not use it) |
| `codecs::skip_list::MultiLevelSkipListWriter::write_skip` | P? | **P** | none | codecs/MultiLevelSkipListWriter.java public writeSkip(IndexOutput); no core caller (Lucene104 does not use it) |
| `codecs::stored_fields::EmptyStoredFieldsFormat` | D? | **D** | removed (with its own unit test) | no Lucene counterpart |
| `codecs::term_state::CompetitiveImpactAccumulator::copy` | P? | **P** | none | codecs/CompetitiveImpactAccumulator.java copy(CompetitiveImpactAccumulator) public |
| `codecs::term_state::TermStats` | P? | **P** | none | codecs/TermStats.java public class |
| `codecs::term_state::TermStats::new` | D? | **P** | none (name-mapping miss) | codecs/TermStats.java public ctor TermStats(int docFreq, long totalTermFreq) |
| `store::BaseDirectory` | W? | **W-blocked** | blocked: FSDirectory/ByteBuffersDirectory do not compose BaseDirectory (refactor; reported) | store/BaseDirectory.java; extended by FSDirectory and ByteBuffersDirectory |
| `store::BaseDirectory::lock_factory` | W? | **W=** | none (accessor kept) | BaseDirectory.java protected field lockFactory read by obtainLock/toString; Rust BaseDirectory reads the field |
| `store::BaseDirectory::new` | W? | **W-blocked** | blocked: same as BaseDirectory (reported) | BaseDirectory.java ctor; callers ByteBuffersDirectory/FSDirectory ctors |
| `store::BufferedChecksum::reset` | P? | **P** | none | store/BufferedChecksum.java reset() (Checksum API) |
| `store::BufferedIndexInput::buffer_size` | W? | **P** | none (follow-up: `NIOFSIndexInput` slices now pass it, as `getBufferSize()`) | BufferedIndexInput.java public getBufferSize(); caller NIOFSDirectory.java:155-160 slice |
| `store::BufferedIndexInput::wrap` | W? | **W** | wired: BufferedIndexInput::slice now returns `Self::wrap(..)` | store/BufferedIndexInput.java:355-357 slice() = wrap(...); :376-379 |
| `store::ByteArrayDataInput::eof` | P? | **P** | none | store/ByteArrayDataInput.java public eof() |
| `store::ByteArrayDataInput::rewind` | W? | **P** | none | ByteArrayDataInput.java public rewind(); its caller document/ShapeDocValues.java:595 Reader is ported over a borrowed slice (declared in document/shape_doc_values.rs:144-149) |
| `store::ByteArrayDataOutput::new` | W? | **W** | wired: Lucene103 writeBlock copies suffix lengths into `ByteArrayDataOutput::new(&mut spare_bytes)` | codecs/lucene103/blocktree/Lucene103BlockTreeTermsWriter.java:939; DV consumer (:768,:844) and FSTCompiler (:1169) already use the with_range ctor by declared divergence |
| `store::ByteBuffersDataOutput::to_writeable_buffer_list` | W? | **P** | none | ByteBuffersDataOutput.java public; its caller util/fst/ReadWriteDataOutput.java:70 freeze() is ported without a ByteBuffersDataOutput (util/fst/read_write_data_output.rs; reported) |
| `store::ByteBuffersDirectory::lock_factory` | W? | **W=** | none (accessor kept) | BaseDirectory.java field lockFactory read by obtainLock/toString; Rust ByteBuffersDirectory reads its own field |
| `store::FileSwitchDirectory` | P? | **P** | none | store/FileSwitchDirectory.java public class |
| `store::FileSwitchDirectory::new` | P? | **P** | none | FileSwitchDirectory.java public ctor |
| `store::FileSwitchDirectory::primary_dir` | W? | **W=** | none (accessor kept; also getPrimaryDir()) | FileSwitchDirectory.java field primaryDir read by its own methods; Rust reads the field |
| `store::FileSwitchDirectory::secondary_dir` | W? | **W=** | none (accessor kept; also getSecondaryDir()) | FileSwitchDirectory.java field secondaryDir read by its own methods; Rust reads the field |
| `store::FilterDirectory::unwrap` | P? | **P** | none | store/FilterDirectory.java public static unwrap(Directory) |
| `store::FilterIndexInput` | W? | **P** | none | store/FilterIndexInput.java public class; its users only call the static unwrapOnlyTest / TestSecrets hooks |
| `store::FilterIndexInput::get_delegate` | P? | **P** | none | FilterIndexInput.java public getDelegate() |
| `store::FilterIndexInput::new` | P? | **P** | none | FilterIndexInput.java public ctor |
| `store::FilterIndexInput::unwrap` | P? | **P** | none | FilterIndexInput.java public static unwrap(IndexInput) |
| `store::FilterIndexInput::unwrap_only_test` | W? | **W-blocked** | blocked: Rust MemorySegment scorers take a pre-narrowed MemorySegmentAccessInput (declared divergence); wiring needs their create() signature + a narrowing hook (reported) | FilterIndexInput.java unwrapOnlyTest; callers internal/vectorization Lucene99MemorySegment*Scorer(Supplier).create |
| `store::FilterIndexOutput::unwrap` | P? | **P** | none | store/FilterIndexOutput.java public static unwrap(IndexOutput) |
| `store::InputStreamDataInput::close` | P? | **P** | none | store/InputStreamDataInput.java close() |
| `store::InputStreamDataInput::into_inner` | D? | **D** | removed | no Lucene counterpart |
| `store::MockIndexInput::is_closed` | D? | **T** | none (probe of MockIndexInput tests; reported) | no Lucene counterpart |
| `store::MockIndexOutput` | D? | **T** | none (test fixture used by codec_util, compound, indexed_disi, skip_list, fuzz tests; reported) | no Lucene counterpart |
| `store::MockIndexOutput::as_inner` | D? | **T** | none (reported) | no Lucene counterpart; used by store tests |
| `store::MockIndexOutput::into_inner` | D? | **T** | none (reported) | no Lucene counterpart; used by many tests |
| `store::MockIndexOutput::is_closed` | D? | **T** | none (reported) | no Lucene counterpart; used by store tests |
| `store::MockIndexOutput::is_empty` | D? | **T** | none (reported) | no Lucene counterpart; used by store tests |
| `store::MockIndexOutput::len` | D? | **D** | removed | no Lucene counterpart, no caller |
| `store::MockIndexOutput::new` | D? | **T** | none (reported) | no Lucene counterpart; used by many tests |
| `store::MockIndexOutput::with_capacity` | D? | **D** | removed | no Lucene counterpart, no caller |
| `store::NIOFSDirectory::directory_path` | D? | **P** | none (renamed: inherited FSDirectory.getDirectory()) | store/FSDirectory.java getDirectory(); NIOFSDirectory composes FSDirectory |
| `store::NIOFSDirectory::open` | W? | **P** | none (renamed ctor NIOFSDirectory(Path)) | store/NIOFSDirectory.java public ctor; FSDirectory.open (FSDirectory.java:151-162) subclass selection is a pending divergence (store.rs FSDirectory::open docs) |
| `store::NIOFSIndexInput::get_delegate` | D? | **D** | removed | NIOFSDirectory.NIOFSIndexInput extends BufferedIndexInput; no getter |
| `store::NRTCachingDirectory` | P? | **P** | none | store/NRTCachingDirectory.java public class |
| `store::NRTCachingDirectory::is_cached_file` | W? | **W-blocked** | blocked: Rust NRTCachingDirectory has no cacheSize counter nor output-to-input callback (reported) | NRTCachingDirectory.java:69-83 cacheDirectory callback calls isCachedFile (:79, :307-309) |
| `store::NRTCachingDirectory::list_cached_files` | P? | **P** | none | NRTCachingDirectory.java public listCachedFiles() |
| `store::NRTCachingDirectory::new` | P? | **P** | none | NRTCachingDirectory.java public ctor |
| `store::NativeFSLockFactory::instance` | W? | **W=** | none | FSLockFactory.java:32 returns NativeFSLockFactory.INSTANCE; Rust get_default (store.rs FSLockFactory::get_default) boxes the zero-sized NativeFSLockFactory value, which is the singleton state |
| `store::NoLockFactory::instance` | W? | **W=** | none | LockStressTest.java:101 `new NIOFSDirectory(lockDirPath, NoLockFactory.INSTANCE)`; Rust lock_stress_test.rs boxes the zero-sized NoLockFactory value |
| `store::OutputStreamDataOutput::close` | P? | **P** | none | store/OutputStreamDataOutput.java close() |
| `store::PreloadHint::instance` | W? | **W** | wired: Lucene99HnswVectorsReader opens .vex with DATA, KNN_VECTORS, RANDOM, PreloadHint (it passed no hints) | codecs/lucene99/Lucene99HnswVectorsReader.java:115-127 |
| `store::SimpleFSLockFactory::instance` | P? | **P** | none | store/SimpleFSLockFactory.java public static INSTANCE |
| `store::SimpleRateLimiter` | P? | **P** | none | store/RateLimiter.java SimpleRateLimiter public static class |
| `store::SimpleRateLimiter::new` | P? | **P** | none | RateLimiter.SimpleRateLimiter(double) public ctor |
| `store::TrackingDirectoryWrapper::get_delegate` | P? | **P** | none | store/FilterDirectory.java public getDelegate() |
| `store::hints_of` | D? | **P** | none (renamed: IOContext.hints(Class), declared divergence) | store/IOContext.java default hints(Class<T>); store.rs hints_of docs |
| `store::is_test_filter_input` | D? | **T** | none (helper over TEST_FILTER_INPUTS used by tests/test_secrets.rs; reported) | FilterIndexInput.java TEST_FILTER_INPUTS.contains(...) inside unwrapOnlyTest; no member |
| `store::output_as_byte_array` | P? | **P** | none | store/ByteBuffersDirectory.java public static OUTPUT_AS_BYTE_ARRAY |
| `store::exceptions::AlreadyClosedException` | W? | **P** | none (declared divergence: namespace over LuceneError::AlreadyClosed) | store/AlreadyClosedException.java; production throws through LuceneError::already_closed, the value this type builds (store/exceptions.rs module docs) |
| `store::exceptions::AlreadyClosedException::is` | D? | **P** | none (declared Rust idiom for `catch`) | store/exceptions.rs module docs |
| `store::exceptions::AlreadyClosedException::new` | W? | **P** | none (same as type) | AlreadyClosedException.java ctor(String); production uses LuceneError::already_closed (same value) |
| `store::exceptions::AlreadyClosedException::with_cause` | D? | **P** | none (renamed ctor) | AlreadyClosedException.java ctor(String, Throwable) |
| `store::exceptions::LockObtainFailedException::new` | W? | **P** | none (same as type) | LockObtainFailedException.java ctor; NativeFSLockFactory/SimpleFSLockFactory/SingleInstanceLockFactory throw it, Rust uses LuceneError::lock_obtain_failed (store.rs:3751,3874,3881) |
| `store::exceptions::LockReleaseFailedException` | W? | **P** | none (declared divergence namespace) | LockReleaseFailedException.java; SimpleFSLock.close throws it, Rust uses LuceneError::lock_release_failed_with_source (store.rs:4069,4080) |
| `store::exceptions::LockReleaseFailedException::is` | D? | **P** | none (declared Rust idiom for `catch`) | store/exceptions.rs module docs |
| `store::exceptions::LockReleaseFailedException::new` | W? | **P** | none (same as type) | LockReleaseFailedException.java ctor(String) |
| `store::exceptions::LockReleaseFailedException::with_cause` | D? | **P** | none (renamed ctor) | LockReleaseFailedException.java ctor(String, Throwable) |
| `store::lock_stress_test::LockStressTest` | P? | **P** | none | store/LockStressTest.java public class (tool) |
| `store::lock_stress_test::LockStressTest::main` | P? | **P** | none | LockStressTest.java public static main |
| `store::lock_validating_directory_wrapper::LockValidatingDirectoryWrapper` | W? | **W-blocked** | blocked: IndexWriter keeps no directoryOrig/directory split (index refactor; reported) | index/IndexWriter.java:958-963 `directory = new LockValidatingDirectoryWrapper(d, writeLock)` |
| `store::lock_validating_directory_wrapper::LockValidatingDirectoryWrapper::get_delegate` | P? | **P** | none | store/FilterDirectory.java public getDelegate() |
| `store::lock_validating_directory_wrapper::LockValidatingDirectoryWrapper::new` | W? | **W-blocked** | blocked: same as LockValidatingDirectoryWrapper (reported) | index/IndexWriter.java:963 |
| `store::lock_validating_directory_wrapper::LockValidatingDirectoryWrapper::write_lock` | W? | **W=** | none (accessor kept) | LockValidatingDirectoryWrapper.java field writeLock read by its own methods; Rust reads the field |
| `store::lock_verify_server::LockVerifyServer` | W? | **P** | none | store/LockVerifyServer.java public class (tool); LockStressTest reaches it over TCP only |
| `store::lock_verify_server::LockVerifyServer::main` | P? | **P** | none | LockVerifyServer.java public static main |
| `store::memory_segment::RefCountedSharedArena::allocate` | D? | **P** | none (faithful member; uncalled in Lucene) | store/RefCountedSharedArena.java allocate(long,long) (Arena API); no core caller |
| `store::memory_segment::RefCountedSharedArena::get_segment_name` | D? | **P** | none (faithful member; uncalled in Lucene) | RefCountedSharedArena.java getSegmentName(); no core caller |
| `store::memory_segment::RefCountedSharedArena::new` | W? | **P** | none | RefCountedSharedArena.java ctor(String, Runnable) delegates to the 3-arg ctor; MemorySegmentIndexInputProvider.getSharedArena uses the 3-arg one = Rust with_max_permits (memory_segment_index_input_provider.rs:302) |
| `store::memory_segment_index_input::MemorySegmentIndexInput::slice_with_context` | D? | **W** | wired (follow-up): new `IndexInput::slice_with_context` (Java 4-arg `slice` overload) overridden by `MemorySegmentIndexInput`; `Lucene90CompoundReader::open_input` calls it on its handle | store/MemorySegmentIndexInput.java slice(String,long,long,IOContext); IndexInput.java:126-139; caller codecs/lucene90/Lucene90CompoundReader.java:171 |
| `store::memory_segment_index_input::MemorySegmentIndexInput::update_io_context` | W? | **W** | wired: `impl IndexInput for MemorySegmentIndexInput` overrides update_io_context; Lucene99FlatVectorsReader get_merge_instance(_flat)/finish_merge call it | store/MemorySegmentIndexInput.java updateIOContext; callers codecs/lucene99/Lucene99FlatVectorsReader.java:206, :304 |
| `store::memory_segment_index_input_provider::MemorySegmentIndexInputProvider::attachment` | W? | **W** | wired: MMapDirectory keeps `attachment` from the provider | store/MMapDirectory.java:187 |
| `store::memory_segment_index_input_provider::MemorySegmentIndexInputProvider::get_default_max_chunk_size` | W? | **W** | wired: MMapDirectory::DEFAULT_MAX_CHUNK_SIZE is the provider value (was a fixed 1 GiB const) | store/MMapDirectory.java:427-430 |
| `store::memory_segment_index_input_provider::MemorySegmentIndexInputProvider::open_input` | W? | **W** | wired: MMapDirectory::open_input goes through the provider (simplified MMapIndexInput read path removed) | store/MMapDirectory.java:311-328 |
| `store::mmap::MMapDirectory::advise_by_context` | P? | **P** | none | store/MMapDirectory.java public static ADVISE_BY_CONTEXT |
| `store::mmap::MMapDirectory::all_files` | P? | **P** | none | MMapDirectory.java public static ALL_FILES |
| `store::mmap::MMapDirectory::directory_path` | D? | **P** | none (renamed: inherited FSDirectory.getDirectory()) | store/FSDirectory.java getDirectory() |
| `store::mmap::MMapDirectory::grouping_key` | D? | **D** | removed | no member; MMapDirectory.java:326 applies groupingFunction inline (now done in Rust open_input) |
| `store::mmap::MMapDirectory::max_chunk_size` | P? | **P** | none (value now `1 << chunkSizePower` as Java) | MMapDirectory.java:307-309 public getMaxChunkSize() |
| `store::mmap::MMapDirectory::no_grouping` | P? | **P** | none | MMapDirectory.java public static NO_GROUPING |
| `store::mmap::MMapDirectory::open` | W? | **P** | none (renamed ctor MMapDirectory(Path)) | store/MMapDirectory.java public ctor; FSDirectory.open subclass selection pending (reported) |
| `store::mmap::MMapDirectory::preload_hint` | P? | **P** | none | MMapDirectory.java public static PRELOAD_HINT |
| `store::mmap::MMapDirectory::set_grouping_function` | P? | **P** | none | MMapDirectory.java public setGroupingFunction |
| `store::mmap::MMapDirectory::set_preload` | P? | **P** | none | MMapDirectory.java public setPreload |
| `store::mmap::MMapDirectory::set_read_advice` | P? | **P** | none | MMapDirectory.java public setReadAdvice |
| `store::mmap::MMapDirectory::supports_madvise` | P? | **P** | none | MMapDirectory.java:423-425 public static supportsMadvise() |
| `store::rate_limited_index_output::RateLimitedIndexOutput::get_delegate` | P? | **P** | none | store/FilterIndexOutput.java public getDelegate() |
| `store::rate_limited_index_output::RateLimitedIndexOutput::rate_limiter` | W? | **W=** | none (accessor kept) | RateLimitedIndexOutput.java field rateLimiter read by checkRate(); Rust reads the field |
| `store::sleeping_lock_wrapper::SleepingLockWrapper` | P? | **P** | none | store/SleepingLockWrapper.java public class |
| `store::sleeping_lock_wrapper::SleepingLockWrapper::get_delegate` | P? | **P** | none | store/FilterDirectory.java public getDelegate() |
| `store::sleeping_lock_wrapper::SleepingLockWrapper::lock_wait_timeout` | W? | **W=** | none (accessor kept) | SleepingLockWrapper.java field lockWaitTimeout read by obtainLock(); Rust reads the field |
| `store::sleeping_lock_wrapper::SleepingLockWrapper::new` | W? | **P** | none | SleepingLockWrapper.java public ctors (2-arg delegates to 3-arg) |
| `store::sleeping_lock_wrapper::SleepingLockWrapper::poll_interval` | W? | **W=** | none (accessor kept) | SleepingLockWrapper.java field pollInterval read by obtainLock(); Rust reads the field |
| `store::verifying_lock_factory::VerifyingLockFactory::get_delegate` | D? | **D** | removed | store/VerifyingLockFactory.java private field `lf`, no getter |
## Removed as a consequence of the W wiring (not in the input list)

- `store::mmap::MMapIndexInput` (pub), `apply_read_advice`, `preload_mapping` — the simplified MMap read path replaced by the provider; `DEFAULT_MAX_CHUNK_SIZE` became a `LazyLock<usize>` holding the provider value (was a 1 GiB `const`).
- `codecs::lucene90::compound::verify_and_copy_index_header` (private duplicate of `CodecUtil.verifyAndCopyIndexHeader`).
- `codecs::lucene99::flat_vectors_format::{read_ord_to_doc_reader, read_ord_to_doc_data}` (pub(crate); body moved into `OrdToDocDISIReaderConfiguration::get_direct_monotonic_reader`).
- `codecs::lucene104::scalar_quantized_vectors::{write_quantized_vectors, write_sorted_quantized_vectors, write_sorting_field_entry, write_meta_entry}` and `Lucene104ScalarQuantizedVectorsWriter::outputs` (private; bodies moved into the Java-named methods).
- Private `check_dimensions` duplicates in `codecs::lucene99::scalar_quantized_scorer` and `internal::vectorization::lucene99_memory_segment_flat_vectors_scorer`.
- `codecs::compound::CompoundSliceInput` (private helper of the removed `CompoundFileDirectory`).

## Follow-up (same pass): divergences closed as Lucene 10.5.0

| # | Change | Java | Test (positive control) |
|---|---|---|---|
| 1 | `Lucene90CompoundReader` opens `.cfs` once (`IOContext.DEFAULT`) as `handle`; `openInput` = `handle.slice_with_context(name, offset, length, context)`; `checkIntegrity` = `checksumEntireFile(handle clone)`; `close` closes the handle; data header checked with the entries' `version` as min and max; duplicate entry id rejected; `checkFooter(entries, priorE)`; length check kept. No whole-file read, no `MockIndexInput`. New trait method `IndexInput::slice_with_context` (default = `slice`, = Java overload) overridden by `MemorySegmentIndexInput`. | `Lucene90CompoundReader.java:58-171, 248-251`; `IndexInput.java:126-139` | `a_lucene_compound_file_is_read_through_slices_of_its_handle` (flush_files, Java-written `.cfs` via `FSDirectory.open`; fails when sub-files are copied into `MockIndexInput`); `sub_files_are_slices_of_the_data_file`, `a_truncated_data_file_is_rejected_on_open`, `a_duplicate_entry_id_is_rejected` (each fails with its check disabled) |
| 2 | `FSDirectory::open(path)` / new `open_with_lock_factory` return `FSDirectoryImpl` = `MMapDirectory` (64-bit + `mmap` feature) else `NIOFSDirectory`; `FSDirectory::with_lock_factory` is now documented as the protected base ctor. **Declared divergence:** return type is the compile-time selected implementation, not an abstract `FSDirectory` (doc on `FSDirectoryImpl`). | `FSDirectory.java:151-162` | `fs_directory_open_picks_mmap_directory_on_64_bit` (fails with the alias switched) |
| 3 | `Lucene99FlatVectorsReader::new` = `with_access_hint(.., Some(Random))`; data context hints `FileTypeHint.DATA, FileDataHint.KNN_VECTORS, accessHint`. | `Lucene99FlatVectorsReader.java:70-93` | `vector_data_is_opened_with_the_hints_lucene_gives_it` (fails with `None`) |
| 4 | `NIOFSIndexInput` keeps `path`/`off`; `slice` builds a new `NIOFSIndexInput` at `off + offset` with `getBufferSize()`, Java's bounds message; `clone` stays a `NIOFSIndexInput`; reads bounded by `end`. Whole-file buffer = new `BufferedIndexInput::buffer_size_for_context` (= `bufferSize(IOContext)`: MERGE→4096 else 1024), replacing a `totalMaxDoc*8` heuristic Java does not have. | `NIOFSDirectory.java:79-160`; `BufferedIndexInput.java:359-369` | `niofs_slice_is_a_niofs_input_with_the_parent_buffer_size` (fails with `BUFFER_SIZE`) |
| 5 | Compound writer: `SizedFileQueue` (Lucene `PriorityQueue`, `lessThan` by length) fed in `files_ordered()` (Java `HashSet`) order; `dir.fileLength` errors propagate; `data.align_file_pointer(64)` as start offset; local align helper removed. | `Lucene90CompoundFormat.java:111-171` | `portability_seal_bytes::cfs_matches_lucene_byte_for_byte` (fails with the queue order reversed) |
| 6 | `impl QuantizedVectorsReader for Lucene99HnswVectorsReader` (forwards to the inherent methods). | `Lucene99HnswVectorsReader.java:74, 428-452` | `the_reader_is_a_quantized_vectors_reader_over_its_flat_reader` (fails when the impl answers `None`) |
| 7 | `vector_byte_length` moved from `LegacyQuantizedByteVectorValues` to `QuantizedByteVectorValues` (Java inherits it from `KnnVectorValues`), overridden by `OffHeapScalarQuantizedVectorValues` (packed length). Existing users (Lucene99 SQ scorer, MemorySegment SQ scorer) unchanged. `KnnVectorValues::vector_byte_length` already existed (the block-1 report saying it was missing was wrong). | `KnnVectorValues.java:66-69`; `OffHeapScalarQuantizedVectorValues.java:176-178` | `quantized_values_report_their_packed_vector_byte_length` (fails with the override removed) |
| 8 | `EmptyKnnVectorsReader` doc: the earlier "garbled" sighting was an artefact of my viewer; the doc is rewritten to state exactly the Java reader's behaviour and cite `KnnVectorsFormat.java:125-161`. | | — |

Gate: `cargo build --all-targets` 0 warnings; `cargo fmt --check` clean; clippy 151 unique (none new; the one hit on a changed line is the pre-existing "items after a test module" on `flat_vectors_format.rs`'s test-module line, whose visibility changed to `pub(crate)`); rustdoc: no warning on changed lines. Lib `codecs:: store:: index:: util::quantization internal::` 1358 passed; 44 integration targets, 336 tests (every target touching FSDirectory/MMap/NIOFS/compound/quantized values, incl. `portability_seal_bytes`, `portability_flush_files`, `portability_index_interop`) all passed.

## Audit fixes (same pass)

| # | Change | Java | Test (positive control) |
|---|---|---|---|
| M1 | New `FlatVectorsReader::clone_flat_reader` (declared divergence: stands for Java's shared `this`); `Lucene104ScalarQuantizedVectorsReader::clone_reader` clones its raw reader with it, so the raw `.vec` is not re-advised. `Lucene99HnswVectorsReader` keeps calling the flat reader's merge instance. | `FlatVectorsReader.java:90-93`; `Lucene99HnswVectorsReader.java:145-148` | `the_merge_instance_leaves_the_raw_vectors_advice_alone` (fails with `get_merge_instance_flat`) |
| M2 | Compound readers closed: `IndexWriter::read_field_infos`, `PendingSoftDeletes::read_field_infos` (try-with-resources: body error first, then close error); `TemporalMergePolicy` closes the points reader (try-with-resources) and the compound dir (`finally`, close error replaces). Drop safety net **not** added: Java releases a shared-arena permit only on explicit `close()`. | `IndexWriter.java:1262-1264`; `TemporalMergePolicy.java:695-736` | `reading_a_compound_segment_leaves_no_shared_arena_behind` (fails with each of the three closes removed, separately) |
| M3 | `NIOFSIndexInput` shares one channel (`Arc<RwLock<Option<File>>>`) with clones and slices, reads with positional reads (`read_exact_at`/`seek_read`), `isClone` kept: only the opener closes the channel. | `NIOFSDirectory.java:95-123, 182-210` | `niofs_sub_file_clones_share_the_open_handle` (Unix; fails when a clone reopens the file by path) |
| L1 | `Lucene99HnswVectorsReader::finish_merge` forwards to the flat reader. | `Lucene99HnswVectorsReader.java:150-153` | `finish_merge_reaches_the_flat_reader` (fails without the override) |
| L2 | `DEFAULT_MAX_CHUNK_SIZE == 1 << 34` (64-bit); chunk-boundary test (16-byte chunks: unaligned long/int across chunks, 5-chunk bulk read, cross-chunk seeks, slice spanning 6 chunks, random access); `fs_directory_open` reads through the mmap input; hint assertions compare sets. | `MemorySegmentIndexInputProvider.java:93` | tests only |
| L3 | Citation `SegmentTermsEnum.java:253-254, 503-506`; rows 90 and 235 and the counts updated. | | — |
| L4 | Default-codec test serialised by `DEFAULT_CODEC_TEST_LOCK`, restores the exact prior slot (`None` included) through a drop guard (`swap_default`, test-only). | | — |
| L5 | Entries map not pre-sized from the file's count (declared in `read_mapping` docs). | | — |
| L6 | `"Duplicate cfs entry id=" + id + " in CFS "` + resource (double space); file-not-found lists the ids in `HashMap.newHashMap(n)` order filled in file order (JVM-faithful `JavaHashMap`); `fileLength` miss message is the name. | `Lucene90CompoundReader.java:131-145, 155-169, 185` | `a_duplicate_entry_id_is_rejected`, `a_missing_sub_file_lists_the_ids_in_java_order` (expected listing printed by OpenJDK 25; fails with file order) |
| L7 | `toReadAdvice` reads the directory's policy at call time (`Arc<RwLock<_>>`). | `MMapDirectory.java:317-318` | `update_io_context_reads_the_policy_set_after_opening` (fails when `set_read_advice` replaces the cell) |
| L8 | `IndexInput::as_memory_segment_access_input` gated on `mmap`; `FSDirectory` declared concrete with its own non-Java read path. `cargo check --lib --no-default-features --features std` still fails in `internal/vectorization` (8 unresolved `MemorySegment`/`MemorySegmentAccessInput` imports) — listed, not changed. The `NIOFSDirectory` branch of `FSDirectoryImpl` compiled and ran when switched in under default features (positive control of the previous round). | `FSDirectory.java:151-162` | — |
| Nit | Chunk size above `i64::MAX` rejected. | | `a_chunk_size_beyond_a_java_long_is_rejected` (fails without the check) |
| N1 | `Lucene104ScalarQuantizedVectorsReader::new` = `with_access_hint(.., Random)`; `.veq` opened with DATA, KNN_VECTORS, RANDOM; header+fields read inside `checkFooter(meta, priorE)`. | `Lucene104ScalarQuantizedVectorsReader.java:85-137` | assertion in `the_merge_instance_leaves_the_raw_vectors_advice_alone` (fails with `None`) |
| N2 | `check_footer_with_prior`: prior rethrown unchanged (type kept) when the checksum passes or verification fails with a non-corruption error; corruption thrown with prior carried. **Declared:** no `addSuppressed` in `LuceneError`; Java's suppressed notes are dropped when the prior is rethrown. | `CodecUtil.java:470-516` | `check_footer_with_prior_rethrows_the_prior_unless_the_footer_is_corrupt` (fails when a passing checksum wraps the prior) |

Gate (target dir `.rucene-kg-target/fix-target`): build 0 warnings; fmt clean; clippy 151 unique (none new; the hit on a changed line is the pre-existing "items after a test module" at `flat_vectors_format.rs`'s test module); rustdoc no warning on changed lines; lib `codecs:: store:: index:: util:: internal::` 1651 passed; 44 integration targets, 336 tests passed (incl. `portability_seal_bytes`, `portability_flush_files`, `portability_index_interop`).
