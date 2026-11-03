# Task #269 block 3 — `util` no-production-caller verdicts

Input: `list_d9cdf45.jsonl`, rows with `"subsystem": "util"` (711 items in 94 files), checked against the working tree (HEAD `d9cdf45` plus pass A of this effort). Java citations are relative to `/tmp/lucene1050/lucene/core/src/java/org/apache/lucene/` (10.5.0 tag); `src/java21` sources are named as such.

Final classes, as in block 2: **W** link ported now; **W=** the Java link already exists in Rust (through the field, a declared mechanism, or a std call with the same result — `copyArray` → `Vec::clone`); **W-blocked** missing link whose Java caller is not ported or that needs a larger change (the Java caller is named); **P** legitimate API, or a faithful member uncalled in Lucene, kept; **D** no Lucene counterpart, removed; **T** test-only, no Lucene counterpart, left.

Positive controls follow block 2: a W wiring is proven by mutating the callee (or reverting the change) and watching a caller-level test fail.

## Counts

Auto (all 711): D? 204, P? 236, W? 271  
Rows written: 711 of 711. Final so far: D 35, P 413, T 18, W 47, W-blocked 61, W= 137

| auto \ final | D | P | T | W | W-blocked | W= |
|---|---|---|---|---|---|---|
| D? | 35 | 140 | 18 | 10 | 1 | 0 |
| P? | 0 | 235 | 0 | 1 | 0 | 0 |
| W? | 0 | 38 | 0 | 36 | 60 | 137 |

## Items

| item | auto | final | action | Java evidence |
|---|---|---|---|---|
| `util::ArrayUtil::copy_array` | W? | **W=** | none (std equivalent) | ArrayUtil.java:643-780 copyArray; callers KnnFloatVectorQuery.java:165, KnnByteVectorQuery.java:162 getTargetCopy, ported as `self.target.clone()` (knn_float_vector_query.rs:111, knn_byte_vector_query.rs:108); ScalarQuantizedVectorScorer.java:48, BlendedTermQuery.java:272 likewise copy with std |
| `util::ArrayUtil::copy_of_sub_array_generic` | D? | **P** | none (renamed overload) | ArrayUtil.java:780 copyOfSubArray(T[], int, int) |
| `util::ArrayUtil::copy_of_sub_array_int` | D? | **P** | none (renamed overload) | ArrayUtil.java:708 copyOfSubArray(int[], int, int) |
| `util::ArrayUtil::copy_of_sub_array_long` | D? | **P** | none (renamed overload) | ArrayUtil.java:726 copyOfSubArray(long[], int, int) |
| `util::ArrayUtil::get_unsigned_comparator` | W? | **W=** | fixed: the generic width now compares exactly `num_bytes` (`ByteArrayComparator` is a boxed closure so it can capture the width, as Java's lambda does); `compare_unsigned_generic` (compared to the slice ends, no Java counterpart) removed. Test `unsigned_comparator_of_other_widths_compares_num_bytes_only` | ArrayUtil.java:804-816 getUnsignedComparator (lambda over Arrays.compareUnsigned(a, aOffset, aOffset + numBytes, ...)); Rust callers compare through `ArrayUtil::compare_unsigned_len` (bkd.rs:5228) or their own helpers (see "Noticed") |
| `util::ArrayUtil::grow_generic` | D? | **P** | none (renamed overload) | ArrayUtil.java:231 grow(T[], int) |
| `util::ArrayUtil::grow_in_range_float` | D? | **P** | none (renamed overload) | ArrayUtil.java:353 growInRange(float[], int, int) |
| `util::ArrayUtil::grow_in_range_int` | D? | **P** | none (renamed overload) | ArrayUtil.java:329 growInRange(int[], int, int) |
| `util::ArrayUtil::grow_no_copy_generic` | D? | **P** | none (renamed overload) | ArrayUtil.java:385,421,457 growNoCopy(int[]/long[]/byte[], int) |
| `util::ArrayUtil::intro_sort_all` | D? | **P** | none (renamed overload) | ArrayUtil.java:529 introSort(T[], Comparator) |
| `util::ArrayUtil::intro_sort_natural` | D? | **P** | none (renamed overload) | ArrayUtil.java:541 introSort(T[], int, int) |
| `util::ArrayUtil::parse_int` | W? | **P** | fixed: signature is Java's `parseInt(char[], int offset, int len)` (the Rust form took the whole slice; Java has no one-argument overload). Test `parse_int_reads_the_given_range` | ArrayUtil.java:52 parseInt(char[], int, int); only caller is itself (delegates to :66) |
| `util::ArrayUtil::select` | W? | **W** | wired: `ComponentTree::create_tree` selects through `ArrayUtil::select` (introselect) instead of `slice::select_nth_unstable_by`, a different algorithm. ShapeDocValues already runs `intro_select` with the same pivot operations (W=) | ArrayUtil.java:619 select; callers ComponentTree.java:382,384, ShapeDocValues.java:313 (shape_doc_values.rs:601), QueryRescorer.java:121, DoubleValuesSourceRescorer.java:80 |
| `util::ArrayUtil::swap` | W? | **W=** | none (std equivalent) | ArrayUtil.java:502 swap; callers ArrayIntroSorter.java:45, ArrayTimSorter.java:52, ArrayInPlaceMergeSorter.java:43 swap; Rust sorters swap with `slice::swap` |
| `util::ArrayUtil::tim_sort_all` | D? | **P** | none (renamed overload) | ArrayUtil.java:578 timSort(T[], Comparator) |
| `util::ArrayUtil::tim_sort_natural` | D? | **P** | none (renamed overload) | ArrayUtil.java:590 timSort(T[], int, int) |
| `util::BitUtil::deinterleave` | P? | **P** | none | BitUtil.java:267 deinterleave |
| `util::BitUtil::flip_flop` | P? | **P** | none | BitUtil.java:278 flipFlop |
| `util::BitUtil::interleave` | W? | **W** | wired: the Tessellator's private copy (`tessellator.rs` `interleave`, documented as waiting for `BitUtil` to expose it) removed; the Morton code is `BitUtil::interleave` | BitUtil.java:249 interleave; callers Tessellator.java:746-747, 981-982, 1097-1098 and Tessellator.Node ctor |
| `util::BitUtil::native_byte_order` | W? | **P** | none | BitUtil.java:39 NATIVE_BYTE_ORDER; read only by BitUtil's VH_NATIVE_* initialisers (BitUtil.java:101-145), which have no Rust counterpart (no VarHandles) |
| `util::BitUtil::read_be_long` | D? | **P** | doc names the Java member it stands in for | BitUtil.java:174 VH_BE_LONG (`get(byte[], int)`); Java readers ArrayUtil.java:820 (Rust `compare_unsigned8` reads `u64::from_be_bytes`, same bytes) |
| `util::BitUtil::write_be_long` | D? | **P** | doc names the Java member it stands in for | BitUtil.java:174 VH_BE_LONG (`set(byte[], int, long)`) |
| `util::BytesRef::hash_code` | W? | **W=** | none (declared: Rust `Hash` feeds the same bytes) | BytesRef.java:118 hashCode; callers Term.java:150, TermRangeQuery.java:185, CompiledAutomaton.java:512 hashCode: Rust implements `Hash`, whose doc declares the mapping (util.rs `impl Hash for BytesRef`) |
| `util::BytesRefBuilder::to_bytes_ref` | W? | **W=** | none (Rust `ScoreTerm` keeps a `BytesRef` copy instead of a builder, top_terms_rewrite.rs:96,220,310) | BytesRefBuilder.java:161 toBytesRef; callers TopTermsRewrite.java:165, Term.java:65 (the `Term(String, BytesRefBuilder)` constructor, not ported) |
| `util::Constants::free_bsd` | P? | **P** | none | Constants.java:54 FREE_BSD |
| `util::Constants::has_fast_compress_mask_cast` | W? | **W-blocked** | blocked: the vector branch it gates is not ported; the Panama support runs the scalar loop (panama_vector_util_support.rs:321-332) | Constants.java:115; caller PanamaVectorUtilSupport.java:1421 (src/java21) |
| `util::Constants::has_fast_scalar_fma` | W? | **W-blocked** | blocked: declared divergence, `vector_util::USE_FMA` is pinned `false` so scores match `-Dlucene.useScalarFMA=false` (vector_util.rs:39-53; default_vector_util_support.rs:42-65) | Constants.java:112; caller DefaultVectorUtilSupport.java:31 fma |
| `util::Constants::has_fast_vector_fma` | W? | **W-blocked** | blocked: vector FMA (PanamaVectorUtilSupport.java:91-97) is not ported, and the Rust Panama provider logs its own scalar-kernel message (panama_vectorization_provider.rs:87-97) | Constants.java:109; callers PanamaVectorUtilSupport.java:92, PanamaVectorizationProvider.java:75 |
| `util::Constants::is_hotspot_vm` | W? | **W=** | none (lookup reads `HotspotVMOptions::is_hotspot_vm`, the function this one returns) | Constants.java:70; caller VectorizationProvider.java:152 (vectorization_provider.rs:288) |
| `util::Constants::is_jvmci_vm` | W? | **W-blocked** | blocked: the JVMCI gate follows the HotSpot gate, which is always false here (jvm.rs:53), so it is unreachable; declared in `lookup`'s docs | Constants.java:73; caller VectorizationProvider.java:158 |
| `util::Constants::java_vendor` | W? | **W-blocked** | blocked: declared divergence, `IndexWriter::set_diagnostics_with_details` omits the JVM keys (index_writer.rs:6651-6655; documents_writer.rs `set_flush_diagnostics`) | Constants.java:63; caller IndexWriter.java:5144 |
| `util::Constants::jre_is_64bit` | W? | **W=** | none (compile-time `target_pointer_width`, declared at the callers) | Constants.java:77; callers FSDirectory.java:157 (store.rs:8998-9005), ArrayUtil.java:175 (util.rs oversize), FST.java:413 (fst.rs:137) |
| `util::Constants::jvm_name` | P? | **P** | none | Constants.java:36 JVM_NAME |
| `util::Constants::jvm_vendor` | P? | **P** | none | Constants.java:33 JVM_VENDOR |
| `util::Constants::jvm_version` | D? | **D** | removed | no Lucene member: Constants.java declares JVM_VENDOR, JVM_NAME, OS_*, JAVA_VENDOR, IS_*, JRE_IS_64BIT (Constants.java:33-115), no JVM_VERSION; the Rust body returned the crate's rust-version |
| `util::Constants::linux` | W? | **W** | wired: `IOUtils::fsync` asserts, as Java does, that a directory fsync on Linux/macOS does not fail | Constants.java:42; callers IOUtils.java:474, MMapDirectory.java:373, NativeAccess |
| `util::Constants::os_name` | W? | **W=** | none (set_diagnostics reads `env::consts::OS`, which `os_name` returns) | Constants.java:39; caller IndexWriter.java:5140 (index_writer.rs:6664) |
| `util::Constants::os_version` | W? | **W-blocked** | blocked: as `java_vendor` | Constants.java:60; caller IndexWriter.java:5142 |
| `util::Constants::sun_os` | P? | **P** | none | Constants.java:48 SUN_OS |
| `util::Constants::windows` | W? | **W** | wired: `IOUtils::fsync` takes Java's Windows directory branch on `Constants::windows()` | Constants.java:45; callers IOUtils.java:460, FSDirectory.java:347,369, FileDeleter.java:236 (Rust uses `cfg!(windows)` there) |
| `util::FixedBitSet::and` | W? | **W** | wired: `DenseConjunctionBulkScorer` intersects with `FixedBitSet::and`; the private copy `search::bit_set_util::and_in_place` removed (with the whole `bit_set_util` module, see "Removed as a consequence") | FixedBitSet.java:633-655 and(FixedBitSet); caller DenseConjunctionBulkScorer.java:315 (dense_conjunction_bulk_scorer.rs `score_window`) |
| `util::FixedBitSet::and_not` | W? | **W** | wired: `SpatialQuery`'s two `andNot(FixedBitSet)` sites call `FixedBitSet::and_not`; the private copy in spatial_query.rs removed. The `andNot(DocIdSetIterator)` sites stay on spatial_query.rs `and_not_iterator`, now documented as the stand-in for that unported overload | FixedBitSet.java:682-684 andNot(FixedBitSet); callers SpatialQuery.java:356,376; andNot(DocIdSetIterator) (FixedBitSet.java:664-679, not ported) at SpatialQuery.java:682,727, LatLonPointDistanceQuery.java:288 |
| `util::FixedBitSet::and_not_count` | P? | **P** | none | FixedBitSet.java andNotCount(FixedBitSet, FixedBitSet) |
| `util::FixedBitSet::apply_mask` | W? | **W** | wired: `Bits` gained Java's default `apply_mask` (reads through `try_get`), overridden by `FixedBitSet` and `FixedBits` as in Java; the four bulk scorers call `accept_docs.apply_mask(..)` instead of `bit_set_util::apply_mask`, and `CheckIndex::bits_cardinality` and `Lucene90LiveDocsFormat::write_bits` batch 1024 bits through it instead of reading bit by bit. Tests `bits_apply_mask_default_clears_and_propagates_get_errors` + callee-mutation controls | FixedBitSet.java:897-909 applyMask; Bits.java:55-63 default; FixedBits.java:33-36; callers ConstantScoreBulkScorer.java:142, BooleanScorer.java:178, DenseConjunctionBulkScorer.java:277, MaxScoreBulkScorer.java:227, CheckIndex.java:1312-1326, Lucene90LiveDocsFormat.java:195-214 |
| `util::FixedBitSet::approximate_cardinality` | W? | **W** | wired: `impl BitSet for FixedBitSet` delegates `approximate_cardinality` to this method instead of a second copy of its body (bit_set.rs), so `BitDocIdSet::new` reaches it. The 4-argument `HnswGraphSearcher.search` that also reads it is not ported (declared on searcher.rs `search`) | FixedBitSet.java:261 approximateCardinality; callers BitDocIdSet.java:49 (doc_id_set.rs:103 through the BitSet trait), HnswGraphSearcher.java:102 |
| `util::FixedBitSet::as_read_only_bits` | W? | **W-blocked** | blocked: Java's view shares the words; Rust `FixedBits` copies them, so the pending-deletes port drops the stale snapshot instead (readers_and_updates.rs:146-151, declared). `ScorerUtil`'s class-identity check has no Rust counterpart | FixedBitSet.java:893 asReadOnlyBits; callers PendingDeletes.java:78, ScorerUtil.java:36 |
| `util::FixedBitSet::cardinality_range` | D? | **W** | wired: `BitSetDocIdStream::count_up_to` counts with `FixedBitSet::cardinality_range` instead of `bit_set_util::cardinality_range` | FixedBitSet.java cardinality(int, int); caller BitSetDocIdStream.java:54 |
| `util::FixedBitSet::clear_range` | D? | **W** | wired: `MaxScoreBulkScorer` clears `[0, innerWindowSize)` with it as Java does (the Rust cleared the whole set), and `impl BitSet for FixedBitSet` delegates `clear_range` to it instead of clearing bit by bit (bit_set.rs divergence note removed); `CheckIndex::bits_cardinality`/`write_bits` clear the ghost bits with it | FixedBitSet.java clear(int, int); callers MaxScoreBulkScorer.java:323, CheckIndex.java:1319, Lucene90LiveDocsFormat.java:204 |
| `util::FixedBitSet::ensure_capacity_and_clear` | W? | **W-blocked** | blocked: the searcher's `visited` is a `Box<dyn BitSet>` and Java casts it to `FixedBitSet` (HnswGraphSearcher.java:372); the trait has no downcast, so the Rust allocates `FixedBitSet::new(cap)` (same cleared content, exact rather than oversized length; searcher.rs `prepare_scratch_state`). ReadersAndUpdates.MergedDocValues.intoBitSet is not ported | FixedBitSet.java:50-60; callers HnswGraphSearcher.java:372, ReadersAndUpdates.java:556 |
| `util::FixedBitSet::flip` | W? | **W-blocked** | blocked: Java's caller `RoaringDocIdSet.Builder.flush` flips the dense buffer to build a `NotDocIdSet`; the Rust Roaring port stores blocks as a `RoaringBlock` enum and collects the excluded docs directly (bit_sets.rs `flush`), a structural change. `FixedBitSet.xor(DocIdSetIterator)` is not ported | FixedBitSet.java:726 flip(int); callers RoaringDocIdSet.java:92, FixedBitSet.java:611-629 xor(DocIdSetIterator) |
| `util::FixedBitSet::flip_range` | D? | **W** | wired: `PointRangeQuery`'s inverse-visitor branch flips with `flip_range(0, maxDoc)`; the private copy `flip_all` (documented as missing from `FixedBitSet`) removed | FixedBitSet.java:739 flip(int, int); caller PointRangeQuery.java:317 |
| `util::FixedBitSet::for_each` | W? | **W** | wired: `BitSetDocIdStream::for_each_up_to` and `MaxScoreBulkScorer::flush_window_to_doc_and_score_acc_buffer` iterate with it; it is now generic over the consumer's error type, as Java's `CheckedIntConsumer<E>` | FixedBitSet.java:916-946 forEach; callers BitSetDocIdStream.java:45, MaxScoreBulkScorer.java:313 |
| `util::FixedBitSet::intersection_count` | P? | **P** | none | FixedBitSet.java intersectionCount |
| `util::FixedBitSet::intersects` | W? | **W-blocked** | blocked: `SloppyPhraseMatcher` uses a private `TermBits` (`Vec<bool>`, declared at sloppy_phrase_matcher.rs:23-32) everywhere Java uses `FixedBitSet` — the term groups (SloppyPhraseMatcher.java:636-690) and the collision re-queue set with `ensureCapacity`/`cardinality` (SloppyPhraseMatcher.java:306-324); replacing the matcher's set type is a larger change than this wiring | FixedBitSet.java:639 intersects; caller SloppyPhraseMatcher.java:662 |
| `util::FixedBitSet::into_array` | W? | **W** | wired: `BitSetDocIdStream::into_array_up_to` copies with it instead of `bit_set_util::into_array`. Lucene104PostingsReader's bit-set `nextPostings` branch is not ported | FixedBitSet.java:963 intoArray; callers BitSetDocIdStream.java:66, Lucene104PostingsReader.java:1051,1068 |
| `util::FixedBitSet::or_mask` | W? | **W-blocked** | blocked: its caller is the Panama `rangeIntoBitSet` vector path, not ported (panama_doc_values_range_support.rs runs the scalar path) | FixedBitSet.java:313 orMask; caller PanamaDocValuesRangeSupport (src/java21) |
| `util::FixedBitSet::or_set` | D? | **P** | none (renamed overload) | FixedBitSet.java or(FixedBitSet) |
| `util::FixedBitSet::scan_is_empty` | W? | **W** | wired: the `assert windowMatches.scanIsEmpty()` sites of the constant-score and dense-conjunction scorers call it | FixedBitSet.java:705 scanIsEmpty; callers ConstantScoreBulkScorer.java:134, DenseConjunctionBulkScorer.java:261-262 |
| `util::FixedBitSet::union_count` | P? | **P** | none | FixedBitSet.java unionCount |
| `util::FixedBitSet::verify_ghost_bits_clear` | W? | **W** | wired: `FixedBitSet::from_bits` asserts it, Java's `assert verifyGhostBitsClear()`. Test `fixed_bit_set_from_bits_asserts_the_ghost_bits_are_clear` | FixedBitSet.java:185 verifyGhostBitsClear; caller FixedBitSet.java:171 ctor |
| `util::FixedBitSet::xor` | W? | **W-blocked** | blocked: its only Java caller, `FixedBitSet.xor(DocIdSetIterator)`, is not ported | FixedBitSet.java:616 xor(FixedBitSet); caller FixedBitSet.java:611-612 |
| `util::GroupVIntUtil::read_group_vints` | D? | **D** | removed (slice-decoding form, with its private slice helpers) | no Lucene member: GroupVIntUtil reads only from a DataInput (GroupVIntUtil.java:43,65,100,225,245) |
| `util::GroupVIntUtil::read_group_vints_baseline` | D? | **P** | fixed: reads from a `DataInput` as Java's `readGroupVInts$Baseline` does (the Rust form decoded a byte slice) | GroupVIntUtil.java:100-124 readGroupVInts$Baseline (benchmark-only, uncalled in core) |
| `util::GroupVIntUtil::read_group_vints_input` | D? | **W** | wired: `PostingsUtil::read_v_int_block` and the HNSW graph reader decode with it; the private copies in postings_util.rs and hnsw_vectors_format.rs removed. `PostingsUtil::write_v_int_block` writes through `DataOutput::write_group_vints` (DataOutput.java:343) instead of a third private copy | GroupVIntUtil.java:43 readGroupVInts(DataInput, int[], int); callers PostingsUtil.java:39, Lucene99HnswVectorsReader.java:583 |
| `util::GroupVIntUtil::read_group_vints_input_long` | D? | **P** | none (renamed overload) | GroupVIntUtil.java:225 readGroupVInts(DataInput, long[], int) |
| `util::GroupVIntUtil::write_group_vints` | D? | **D** | removed (slice-encoding form) | no Lucene member: GroupVIntUtil writes only to a DataOutput (GroupVIntUtil.java:185,264) |
| `util::IOUtils::close` | W? | **W-blocked** | blocked: the closure API is typed over `io::Result` while Rust closeables return `LuceneError`; the ~60 Java call sites close through `?` and `Drop` in Rust. Fixed on the way (finding 2): `use_or_suppress` keeps the first error. Test `ioutils_close_reports_the_first_failure` | IOUtils.java:69-91 close; callers e.g. Lucene90DocValuesConsumer.java:127, Lucene104PostingsReader.java:210 |
| `util::IOUtils::close_while_handling_exception` | W? | **W-blocked** | blocked: as `close` | IOUtils.java:103-113; callers e.g. Lucene104PostingsFormat.java:421 |
| `util::IOUtils::close_while_suppressing_exceptions` | W? | **W-blocked** | blocked: as `close` | IOUtils.java:145-156; callers StandardDirectoryReader.java:104, Lucene90DocValuesProducer.java:177 |
| `util::IOUtils::delete_files_if_exist` | W? | **P** | none | IOUtils.java:299 deleteFilesIfExist(Path...); only caller is its own overload (IOUtils.java:310) |
| `util::IOUtils::delete_files_ignoring_exceptions` | W? | **P** | none | IOUtils.java:278 deleteFilesIgnoringExceptions(Path...); core callers use the Directory overload (IOUtils.java:223), not this one |
| `util::IOUtils::fsync` | W? | **W** | wired: `FSDirectory`'s `sync` and `sync_metadata` fsync through it (store.rs `fsync_file`/`fsync_directory`), which opened files read-only and synced data only. Fixed to Java's body: a regular file is opened for writing and `force(true)`-synced; a directory is opened read-only and its fsync failure ignored (with Java's Linux/macOS assertion); on Windows only the directory's existence is checked. Test `ioutils_fsync_opens_a_regular_file_for_writing` (read-only file → PermissionDenied, Java oracle AccessDeniedException) | IOUtils.java:452-484 fsync; callers FSDirectory.java:275, 300-302 |
| `util::IOUtils::require_resource_non_null` | P? | **P** | none | IOUtils.java requireResourceNonNull |
| `util::IOUtils::rethrow_always` | W? | **W=** | none (Rust callers propagate with `return Err`/`?`) | IOUtils.java:419-436 rethrowAlways; callers e.g. CheckIndex.java:640, IndexWriter.java:3371 |
| `util::IOUtils::rm` | W? | **P** | none | IOUtils.java:333 rm(Path...); only caller is its own private overload (IOUtils.java:349) |
| `util::MatchNoBits` | P? | **P** | none | Bits.java MatchNoBits |
| `util::MatchNoBits::new` | P? | **P** | none | Bits.java MatchNoBits(int) |
| `util::NumericUtils::add` | W? | **W-blocked** | blocked: its caller `InetAddressPoint` is not ported. Fixed on the way: returns Java's `IllegalArgumentException("a + b overflows bytesPerDim=N")` instead of panicking. Test `numeric_utils_subtract_and_add_report_javas_illegal_argument` | NumericUtils.java:134-169 add; caller InetAddressPoint.java:97 |
| `util::NumericUtils::big_int_to_sortable_bytes` | P? | **P** | none | NumericUtils.java bigIntToSortableBytes |
| `util::NumericUtils::sortable_bytes_to_big_int` | W? | **P** | none | NumericUtils.java:249 sortableBytesToBigInt; the "caller" at NumericUtils.java:223 is a javadoc `@see` |
| `util::NumericUtils::subtract` | W? | **W** | wired: `BKDWriter::split` computes the span with it; the private copy `BKDUtil::subtract_unsigned_bytes` (message `subtract_unsigned_bytes: a < b`, no Java counterpart) removed. Fixed: returns `IllegalArgumentException("a < b")` instead of panicking. Same test. InetAddressPoint (second caller) not ported | NumericUtils.java:93-128 subtract; callers BKDWriter.java:1611, InetAddressPoint.java:115 |
| `util::RamUsageEstimator::adjust_for_field` | W? | **P** | none | RamUsageEstimator.java:592 adjustForField; its caller `shallowSizeOfInstance` walks fields by reflection, which the Rust `shallow_size_of_instance` replaces with `size_of::<T>()` (declared) |
| `util::RamUsageEstimator::human_readable_units` | W? | **W** | wired: `Accountables::to_string` renders through it; the private copy `Accountables::human_readable_units` (declared as missing from `RamUsageEstimator`, rounding half away from zero) removed. Fixed: rounds half to even as `DecimalFormat("0.#")` does (1280 bytes → `1.2 KB`, Java oracle). Test `human_readable_units_rounds_half_to_even_as_decimal_format` | RamUsageEstimator.java:600-617 humanReadableUnits; caller Accountables.java:58 |
| `util::RamUsageEstimator::size_of_accountable` | D? | **P** | none (renamed overload) | RamUsageEstimator.java sizeOf(Accountable) |
| `util::RamUsageEstimator::size_of_accountables` | D? | **P** | none (renamed overload) | RamUsageEstimator.java:624 sizeOf(Accountable[]) |
| `util::RamUsageEstimator::size_of_bool` | D? | **P** | none (renamed overload) | RamUsageEstimator.java sizeOf(boolean[]) |
| `util::RamUsageEstimator::size_of_char` | D? | **P** | none (renamed overload) | RamUsageEstimator.java sizeOf(char[]) |
| `util::RamUsageEstimator::size_of_collection` | W? | **P** | none | RamUsageEstimator.java:432 sizeOfCollection; its callers are its own overload and `sizeOfObject`'s reflective recursion (declared divergence on `size_of_object`) |
| `util::RamUsageEstimator::size_of_double` | D? | **P** | none (renamed overload) | RamUsageEstimator.java sizeOf(double[]) |
| `util::RamUsageEstimator::size_of_float` | D? | **P** | none (renamed overload) | RamUsageEstimator.java sizeOf(float[]) |
| `util::RamUsageEstimator::size_of_object` | W? | **W-blocked** | blocked: Java dispatches on the runtime class (String, boxed primitives, arrays, collections, maps); the Rust form takes an `Accountable` (declared on the item), so the 18 Java callers compute their sizes with typed `size_of_*` calls | RamUsageEstimator.java:382-432 sizeOfObject; callers e.g. Term.java:191, BytesRefHash.java:508, CompiledAutomaton.java:540 |
| `util::RamUsageEstimator::size_of_short` | D? | **P** | none (renamed overload) | RamUsageEstimator.java sizeOf(short[]) |
| `util::RefCount` | W? | **W-blocked** | blocked: `SegmentDocValues` holds its producers in `DocValuesProducerHolder`, because Java's anonymous `RefCount.release()` reaches back into the map the caller holds locked (segment_reader.rs:450-455, declared). Java FileDeleter uses its own nested `RefCount`, ported as the private one in file_deleter.rs | RefCount.java:26; caller SegmentDocValues.java:35-82 |
| `util::RefCount::dec_ref` | W? | **W-blocked** | blocked: as `RefCount` | RefCount.java decRef; caller SegmentDocValues.java:82 |
| `util::RefCount::get` | W? | **W-blocked** | blocked: as `RefCount` | RefCount.java get; caller SegmentDocValues.java:66 |
| `util::RefCount::inc_ref` | W? | **W-blocked** | blocked: as `RefCount` | RefCount.java incRef; caller SegmentDocValues.java:66 |
| `util::RefCount::new` | W? | **W-blocked** | blocked: as `RefCount` | RefCount.java:32; caller SegmentDocValues.java:51 |
| `util::RefCount::ref_count` | W? | **W=** | none (field accessor; `inc_ref`/`dec_ref` read the count) | RefCount.java refCount field / getRefCount |
| `util::RefCount::with_release` | D? | **P** | none (declared stand-in for a subclass overriding `release()`) | RefCount.java:40 protected release(); the type doc declares the mechanism |
| `util::ThreadInterruptedException` | W? | **W-blocked** | blocked: Rust threads cannot be interrupted, so no Rust caller has Java's `InterruptedException` path to wrap | ThreadInterruptedException.java; callers ConcurrentMergeScheduler.doStall, DocumentsWriterFlushControl.waitForFlush, IndexWriter.doWait |
| `util::ThreadInterruptedException::new` | W? | **W-blocked** | blocked: as `ThreadInterruptedException` | ThreadInterruptedException.java ctor |
| `util::default_info_stream` | W? | **W** | wired: `LiveIndexWriterConfig`, `HnswGraphBuilder` and `HnswConcurrentMergeBuilder` start from it (they used `NoOutputInfoStream` directly); it now returns the mutable default as an `Arc`. Test `a_new_writer_config_reads_the_default_info_stream` | InfoStream.java:64-66 getDefault; callers LiveIndexWriterConfig.java:135, HnswGraphBuilder.java:85, HnswConcurrentMergeBuilder.java:48 |
| `util::set_default_info_stream` | D? | **P** | fixed: it was a no-op stub; it now replaces the default under the lock, as `InfoStream.setDefault` does. Same test | InfoStream.java:74-81 setDefault |
| `util::accountables::Accountables` | W? | **W-blocked** | blocked: `Accountable::child_resources` returns `&dyn Accountable`, which carries no description; the described tree is the `AccountableTree` sub-trait (declared, accountables.rs:10-21), which neither `OrdinalMap` (doc_values.rs:2010-2015, returns the bare segment map) nor `LRUQueryCache` (no child resources) implements | Accountables.java; callers OrdinalMap.java:432-438, LRUQueryCache.java:513-520 |
| `util::accountables::Accountables::named_accountable` | W? | **W-blocked** | blocked: as `Accountables` | Accountables.java namedAccountable(String, Accountable); caller OrdinalMap.java:434 |
| `util::accountables::Accountables::named_accountables` | W? | **W-blocked** | blocked: as `Accountables` | Accountables.java namedAccountables(String, Map); caller LRUQueryCache.java:516 |
| `util::accountables::Accountables::to_string` | W? | **P** | none (rendering wired to `RamUsageEstimator::human_readable_units`, see that row) | Accountables.java:40-44 toString(Accountable); only caller is its private overload |
| `util::annotations::IgnoreRandomChains` | W? | **P** | none (declared marker: applying a Java annotation has no Rust counterpart, annotations.rs module doc) | IgnoreRandomChains.java (annotation, RUNTIME); applied to CachingTokenFilter |
| `util::annotations::IgnoreRandomChains::new` | D? | **P** | none (stands in for writing `@IgnoreRandomChains(reason = ...)`) | IgnoreRandomChains.java reason() element |
| `util::annotations::IgnoreRandomChains::reason` | D? | **P** | none | IgnoreRandomChains.java reason() |
| `util::annotations::SuppressForbidden` | W? | **P** | none (declared marker, as `IgnoreRandomChains`) | SuppressForbidden.java (annotation, CLASS); applied to CheckIndex, IndexUpgrader, the vectorization providers |
| `util::annotations::SuppressForbidden::new` | D? | **P** | none (stands in for writing `@SuppressForbidden(reason = ...)`) | SuppressForbidden.java reason() element |
| `util::annotations::SuppressForbidden::reason` | D? | **P** | none | SuppressForbidden.java reason() |
| `util::attribute::AttributeSource::add_attribute_by_id` | D? | **P** | none (renamed: Java's `addAttribute(Class)` with the class as a run-time value, here a `TypeId`) | AttributeSource.java addAttribute(Class<T>) |
| `util::attribute::AttributeSource::attribute_classes` | D? | **P** | none (renamed) | AttributeSource.java getAttributeClassesIterator() |
| `util::attribute::AttributeSource::attribute_impls_iter` | D? | **P** | none (renamed) | AttributeSource.java getAttributeImplsIterator() |
| `util::attribute::AttributeSource::factory` | W? | **W=** | none (accessor kept; the source's own methods read the field) | AttributeSource.java getAttributeFactory()/field `factory`, read by addAttribute/cloneAttributes |
| `util::attribute::AttributeSource::has_attribute_by_id` | D? | **P** | none (renamed: `hasAttribute(Class)` with a run-time `TypeId`) | AttributeSource.java hasAttribute(Class) |
| `util::attribute::AttributeSource::hash_code` | P? | **P** | none | AttributeSource.java hashCode() |
| `util::attribute::AttributeSource::remove_all_attributes` | P? | **P** | none | AttributeSource.java removeAllAttributes() |
| `util::attribute::DefaultAttributeFactory::register_self` | D? | **T** | none (used only by tokenattributes.rs `all_defaults_reflection` test) | no Lucene member; Java's default factory finds implementations by reflection (AttributeFactory.java DEFAULT_ATTRIBUTE_FACTORY) |
| `util::attribute::get_static_implementation` | W? | **W** | wired: `analysis::default_token_attribute_factory` is `get_static_implementation::<PackedTokenAttributeImpl>` over a default factory, as `TokenStream.DEFAULT_TOKEN_ATTRIBUTE_FACTORY` is (it registered the implementation into a `DefaultAttributeFactory`) | AttributeFactory.java getStaticImplementation; caller TokenStream.java:81-83 |
| `util::attribute::unwrap_all` | W? | **P** | none | Unwrappable.java unwrapAll; its core caller MemorySegmentIndexInputProvider.java:60 unwraps the test-only `FilterPath` layers (JDK-8259028), which a Rust `Path` never has |
| `util::automaton::automata::Automata::append_any_char` | P? | **P** | none | Automata.java appendAnyChar |
| `util::automaton::automata::Automata::append_char` | P? | **P** | none | Automata.java appendChar |
| `util::automaton::automata::Automata::make_binary_string_union_from_iter` | D? | **P** | none (renamed overload) | Automata.java makeBinaryStringUnion(BytesRefIterator) |
| `util::automaton::automata::Automata::make_string_from_code_points` | D? | **P** | none (renamed overload) | Automata.java makeString(int[], int, int) |
| `util::automaton::automata::Automata::make_string_union` | P? | **P** | none | Automata.java makeStringUnion(Iterable) |
| `util::automaton::automata::Automata::make_string_union_from_iter` | D? | **P** | none (renamed overload) | Automata.java makeStringUnion(BytesRefIterator) |
| `util::automaton::automaton::Automaton::finish` | D? | **T** | none (first removed; the compiler showed `automaton_terms_enum` tests call it; restored with its stale doc corrected) | no Lucene member: Automaton.java has finishState() (:326), no finish() |
| `util::automaton::byte_run_automaton::ByteRunAutomaton::get_char_intervals` | P? | **P** | none | RunAutomaton.java getCharIntervals (inherited) |
| `util::automaton::byte_run_automaton::ByteRunAutomaton::run_automaton` | D? | **P** | none (declared composition: the accessor stands for the inherited `RunAutomaton` members) | ByteRunAutomaton.java extends RunAutomaton |
| `util::automaton::character_run_automaton::CharacterRunAutomaton` | P? | **P** | none | CharacterRunAutomaton.java |
| `util::automaton::character_run_automaton::CharacterRunAutomaton::automaton` | W? | **W=** | none (accessor kept over the composed run table's field) | RunAutomaton.java automaton field, read by RunAutomaton.ramBytesUsed and CompiledAutomaton's ctor on its own run automaton |
| `util::automaton::character_run_automaton::CharacterRunAutomaton::get_char_intervals` | P? | **P** | none | RunAutomaton.java getCharIntervals (inherited) |
| `util::automaton::character_run_automaton::CharacterRunAutomaton::is_accept` | W? | **W=** | none (inherited member; CompiledAutomaton's callers act on its ByteRunAutomaton) | RunAutomaton.java isAccept; callers CompiledAutomaton.java addTail/floor |
| `util::automaton::character_run_automaton::CharacterRunAutomaton::new` | P? | **P** | none | CharacterRunAutomaton.java ctor |
| `util::automaton::character_run_automaton::CharacterRunAutomaton::run` | P? | **P** | none | CharacterRunAutomaton.java run(String) |
| `util::automaton::character_run_automaton::CharacterRunAutomaton::run_automaton` | D? | **P** | none (declared composition, as ByteRunAutomaton's) | CharacterRunAutomaton.java extends RunAutomaton |
| `util::automaton::character_run_automaton::CharacterRunAutomaton::run_chars` | D? | **P** | none (renamed overload) | CharacterRunAutomaton.java run(char[], int, int) |
| `util::automaton::character_run_automaton::CharacterRunAutomaton::size` | W? | **W=** | none (accessor over the run table's `size`) | RunAutomaton.java size field / getSize, read by its own methods |
| `util::automaton::character_run_automaton::CharacterRunAutomaton::step` | W? | **W=** | none (inherited member; `run` steps the composed table) | RunAutomaton.java step; callers CharacterRunAutomaton.run, CompiledAutomaton.floor |
| `util::automaton::compiled_automaton::CompiledAutomaton::from_automaton` | D? | **P** | none (renamed ctor) | CompiledAutomaton.java CompiledAutomaton(Automaton) |
| `util::automaton::compiled_automaton::CompiledAutomaton::get_byte_runnable` | W? | **W-blocked** | blocked: `FieldReader::intersect` (field_reader.rs:209-234) and `AutomatonTermsEnum` (automaton_terms_enum.rs:80-105) read the DFA fields `run_automaton`/`automaton` and take concrete types, so an NFA-compiled automaton is refused ("no runnable form") where Java runs it through `getByteRunnable()`/`getTransitionAccessor()`; wiring needs both enums to take `dyn ByteRunnable`/`dyn TransitionAccessor` | CompiledAutomaton.java getByteRunnable; callers FieldReader.java:176-177, AutomatonTermsEnum ctor |
| `util::automaton::compiled_automaton::CompiledAutomaton::get_transition_accessor` | W? | **W-blocked** | blocked: as `get_byte_runnable` | CompiledAutomaton.java getTransitionAccessor; same callers |
| `util::automaton::int_set::FrozenIntSet::set_state` | D? | **D** | removed | no Lucene member: FrozenIntSet.state is `final` (FrozenIntSet.java:23-28) |
| `util::automaton::int_set::FrozenIntSet::state` | W? | **W=** | none (field accessor; `determinize` reads the field, operations.rs:699) | FrozenIntSet.java:23 state; caller Operations.determinize |
| `util::automaton::levenshtein_automata::LevenshteinAutomata::alpha_max` | D? | **P** | none (stands for the package-visible field) | LevenshteinAutomata.java:44 `final int alphaMax` |
| `util::automaton::levenshtein_automata::LevenshteinAutomata::from_str` | D? | **P** | none (renamed ctor) | LevenshteinAutomata.java LevenshteinAutomata(String, boolean) |
| `util::automaton::levenshtein_automata::LevenshteinAutomata::to_automaton` | W? | **P** | none | LevenshteinAutomata.java toAutomaton(int), which only delegates to toAutomaton(int, String); FuzzyAutomatonBuilder.java:63,73 calls the prefix overload, as fuzzy_automaton_builder.rs:111 does |
| `util::automaton::limited_finite_strings_iterator::LimitedFiniteStringsIterator` | P? | **P** | none | LimitedFiniteStringsIterator.java |
| `util::automaton::limited_finite_strings_iterator::LimitedFiniteStringsIterator::new` | P? | **P** | none | LimitedFiniteStringsIterator.java ctor |
| `util::automaton::limited_finite_strings_iterator::LimitedFiniteStringsIterator::next` | W? | **P** | none | LimitedFiniteStringsIterator.java next() override; GraphTokenStreamFiniteStrings.java:133 builds a plain FiniteStringsIterator, as graph.rs:238 does |
| `util::automaton::limited_finite_strings_iterator::LimitedFiniteStringsIterator::size` | P? | **P** | none | LimitedFiniteStringsIterator.java size() |
| `util::automaton::nfa_run_automaton::NFARunAutomaton::automaton` | W? | **W=** | none (field accessor; its own methods read the field) | NFARunAutomaton.java automaton field |
| `util::automaton::nfa_run_automaton::NFARunAutomaton::from_automaton` | D? | **P** | none (renamed ctor) | NFARunAutomaton.java NFARunAutomaton(Automaton) |
| `util::automaton::nfa_run_automaton::NFARunAutomaton::run` | D? | **P** | none (faithful member uncalled in Lucene) | NFARunAutomaton.java run(int[]) |
| `util::automaton::operations::Operations::get_common_prefix` | W? | **W=** | none (both prefix forms share `get_common_prefix_code_points`, the body Java's `getCommonPrefixBytesRef` reaches through `getCommonPrefix`) | Operations.java:1149 getCommonPrefix; caller Operations.java:1212 |
| `util::automaton::operations::Operations::has_dead_states_to_accept` | P? | **P** | none | Operations.java hasDeadStatesToAccept |
| `util::automaton::operations::Operations::minus` | P? | **P** | none | Operations.java minus |
| `util::automaton::operations::Operations::run` | P? | **P** | none | Operations.java run(Automaton, String) |
| `util::automaton::operations::Operations::run_ints` | D? | **P** | none (renamed overload) | Operations.java run(Automaton, IntsRef) |
| `util::automaton::operations::Operations::topo_sort_states` | W? | **P** | none | Operations.java topoSortStates; only caller is its own recursive helper |
| `util::automaton::reg_exp::RegExp::get_identifiers` | W? | **P** | none | RegExp.java getIdentifiers; only caller its own overload |
| `util::automaton::reg_exp::RegExp::new` | W? | **P** | none | RegExp.java RegExp(String); RegexpQuery uses the 3-argument constructor, as regexp_query.rs:179 does (`with_match_flags`) |
| `util::automaton::reg_exp::RegExp::to_automaton` | W? | **P** | none | RegExp.java toAutomaton(); RegexpQuery.toAutomaton uses the provider overload, as regexp_query.rs:199 does |
| `util::automaton::reg_exp::RegExp::to_string_tree` | W? | **P** | none | RegExp.java toStringTree; only caller its own overload |
| `util::automaton::run_automaton::RunAutomaton::get_size` | P? | **P** | none | RunAutomaton.java getSize |
| `util::automaton::too_complex_to_determinize_exception::TooComplexToDeterminizeException::from_reg_exp` | D? | **P** | none (renamed ctor) | TooComplexToDeterminizeException.java (RegExp, TooComplexToDeterminizeException) ctor |
| `util::automaton::too_complex_to_determinize_exception::TooComplexToDeterminizeException::get_automaton` | P? | **P** | none | TooComplexToDeterminizeException.java getAutomaton |
| `util::automaton::too_complex_to_determinize_exception::TooComplexToDeterminizeException::get_determinize_work_limit` | P? | **P** | none | TooComplexToDeterminizeException.java getDeterminizeWorkLimit |
| `util::automaton::too_complex_to_determinize_exception::TooComplexToDeterminizeException::get_reg_exp` | P? | **P** | none | TooComplexToDeterminizeException.java getRegExp |
| `util::bit_set::BitSetIterator::get_bit_set` | W? | **W-blocked** | blocked: its Java callers narrow a `DocIdSetIterator` to `BitSetIterator` (AcceptDocs.java:97 reuse branch, ConjunctionDISI.BitSetConjunctionDISI), which a `dyn DocIdSetIterator` cannot do; both are declared (doc_id_set_iterator.rs `create_bit_set`, conjunction_disi.rs module doc) | BitSetIterator.java:69 getBitSet; callers AcceptDocs.java:97, ConjunctionDISI.java:244 |
| `util::bit_set::BitSetIterator::get_fixed_bit_set_or_null` | W? | **W-blocked** | blocked: its callers `FixedBitSet.xor(DocIdSetIterator)` and `andNot(DocIdSetIterator)` are not ported (and need the same narrowing) | BitSetIterator.java:43; callers FixedBitSet.java:618,665 |
| `util::bit_set::BitSetIterator::get_sparse_fixed_bit_set_or_null` | W? | **W-blocked** | blocked: its caller is the narrowing branch of `SparseFixedBitSet.or(DocIdSetIterator)`, unreachable for a `dyn DocIdSetIterator` (the rest of that method is wired, see `or_dense`) | BitSetIterator.java:50; caller SparseFixedBitSet.java:694 |
| `util::bit_set::BitSetIterator::set_doc_id` | W? | **W-blocked** | blocked: its caller `BitSetConjunctionDISI` is not ported (declared, conjunction_disi.rs module doc) | BitSetIterator.java:74; caller ConjunctionDISI.java:280 |
| `util::bit_set::DocBaseBitSetIterator::get_bit_set` | W? | **W-blocked** | blocked: its caller `FixedBitSet.andNot(DocIdSetIterator)` is not ported | DocBaseBitSetIterator.java:53; caller FixedBitSet.java:673 |
| `util::bit_set::DocBaseBitSetIterator::get_doc_base` | W? | **W-blocked** | blocked: as `get_bit_set` | DocBaseBitSetIterator.java:49; caller FixedBitSet.java:673 |
| `util::bit_set::FixedBits::apply_mask` | W? | **W** | wired: `impl Bits for FixedBits` overrides `Bits::apply_mask` with it, and it forwards to `FixedBitSet::apply_mask` (it read bit by bit and panicked); divergence note removed. Callers reach it through `Bits::apply_mask` (see `util::FixedBitSet::apply_mask`) | FixedBits.java:33-36 applyMask |
| `util::bit_set::FixedBits::bit_set` | W? | **W-blocked** | blocked: `FixedBitSet.copyOf(Bits)` narrows `bits instanceof FixedBits` to clone the backing set (FixedBitSet.java:866-873); Rust `Bits` has no such narrowing, so `FixedBitSet::copy_of` copies bit by bit | FixedBits.java:22 bitSet; caller FixedBitSet.java:867-869 |
| `util::bit_sets::DenseLiveDocs::deleted_count` | W? | **W=** | none (field accessor; the class's own methods read the field) | DenseLiveDocs.java deletedCount field / deletedCount(); readers deletedDocsIterator/liveDocsIterator/toString |
| `util::bit_sets::DenseLiveDocs::live_docs` | W? | **W=** | none (field accessor; the class's own methods read the field) | DenseLiveDocs.java liveDocs field; readers get/ramBytesUsed/iterators |
| `util::bit_sets::DenseLiveDocs::ram_bytes_used` | P? | **P** | none | DenseLiveDocs.java:152 ramBytesUsed |
| `util::bit_sets::RoaringDocIdSet::max_doc` | D? | **D** | removed (the `length` doc no longer refers to it) | no Lucene member: RoaringDocIdSet keeps `maxDoc` only on its Builder (RoaringDocIdSet.java:46-62) |
| `util::bit_sets::SparseFixedBitSet::clear_range` | D? | **W** | wired: `impl BitSet for SparseFixedBitSet` delegates `clear_range` to it (it cleared bit by bit). Fixed on the way: `mask` reduces its shift distances modulo 64 as Java's `long` shifts do; multi-word ranges overflowed (`1u64 << 4000`) and panicked in `clear_range`/`set_range`. Tests `the_bit_set_view_answers_with_the_sparse_overrides`, `ranges_across_words_and_blocks_set_and_clear_every_bit` | SparseFixedBitSet.java:330 clear(int, int); mask :353-355 |
| `util::bit_sets::SparseFixedBitSet::iter` | D? | **T** | none (first removed; the compiler showed bit_sets.rs tests use it; restored) | no Lucene member (Java iterates through BitSetIterator) |
| `util::bit_sets::SparseFixedBitSet::next_clear_bit` | W? | **W** | wired: the `BitSet` view overrides `next_clear_bit` with it, as Java's override does (the trait default ran the bit-by-bit range scan) | SparseFixedBitSet.java:418-421 nextClearBit(int); callers BitSet.nextClearBit dispatch, BitSetIterator.docIDRunEnd, MergingHnswGraphBuilder.build |
| `util::bit_sets::SparseFixedBitSet::next_clear_bit_up_to` | D? | **W** | wired: the `BitSet` view's `next_clear_bit_in_range` delegates to it (it scanned bit by bit) | SparseFixedBitSet.java:424-427 nextClearBit(int, int) |
| `util::bit_sets::SparseFixedBitSet::or_dense` | W? | **W** | wired: the `BitSet` view overrides `or` with Java's `SparseFixedBitSet.or(DocIdSetIterator)`: a dense iterator (cost at least the block count) goes through `or_dense`, a sparse one bit by bit; the `getSparseFixedBitSetOrNull` branch is declared unreachable (no iterator narrowing). Test `or_unions_dense_iterators_through_or_dense` | SparseFixedBitSet.java:691-713 or; orDense caller :711 |
| `util::bit_sets::SparseFixedBitSet::prev_set_bit` | P? | **W** | wired: the `BitSet` view delegates `prev_set_bit` to it (it walked back bit by bit) | SparseFixedBitSet.java:553 prevSetBit (BitSet override) |
| `util::bit_sets::SparseFixedBitSet::set_range` | D? | **P** | none (renamed overload; fixed by the `mask` change, see `clear_range`) | SparseFixedBitSet.java:251 set(int, int) |
| `util::bit_sets::SparseLiveDocs::deleted_count` | W? | **W=** | none (field accessor) | SparseLiveDocs.java deletedCount field / deletedCount() |
| `util::bit_sets::SparseLiveDocs::deleted_docs` | W? | **W=** | none (field accessor) | SparseLiveDocs.java deletedDocs field |
| `util::bit_sets::SparseLiveDocs::ram_bytes_used` | P? | **P** | none | SparseLiveDocs.java:152 ramBytesUsed |
| `util::bkd::BKDReader::config` | W? | **W=** | none (field accessor; the reader's methods read the field) | BKDReader.java config field, read by its own methods |
| `util::bkd::BKDUtil::get_equals_predicate` | W? | **W=** | none (BKDWriter compares through `BKDUtil::equals`/`common_prefix_length`, the bodies of the lambdas these factories return; same results) | BKDUtil.java:78 getEqualsPredicate; caller BKDWriter.java:182 (uses :739, 1394, 1727) |
| `util::bkd::BKDUtil::get_prefix_length_comparator` | W? | **W=** | none (as `get_equals_predicate`) | BKDUtil.java:32 getPrefixLengthComparator; caller BKDWriter.java:183 (uses :846, 1170, 1673) |
| `util::bkd::BKDWriter::new_default` | D? | **P** | none (renamed overload) | BKDWriter.java:140-157 BKDWriter(int, Directory, String, BKDConfig, double, long), which passes VERSION_CURRENT to the 7-argument constructor (:158, Rust `new`) |
| `util::bkd::BKDWriter::point_count` | W? | **W=** | none (field accessor) | BKDWriter.java pointCount field, read by add/finish/writeIndex |
| `util::bkd::HeapPointReader::point_value_owned` | D? | **D** | removed | no Lucene member: HeapPointReader.java has pointValue() (a view) only |
| `util::bkd::HeapPointWriter::compute_cardinality` | W? | **W** | wired: the heap path of `BKDWriter::build` counts the leaf cardinality with it, as Java does; BKDWriter's private copy now serves only the mutable-tree path, whose inline loop it ports (BKDWriter.java:1718-1740; doc updated) | HeapPointWriter.java computeCardinality; caller BKDWriter.java:1989 |
| `util::bkd::OfflinePointWriter::sorted_reader` | D? | **T** | none (used only by the `offline_point_writer_sorted_reader` test) | no Lucene member: OfflinePointWriter.java has getReader(long, long) only |
| `util::bkd::PointValue::packed_value` | W? | **W=** | none (callers read the owned `packed` field, the bytes the Java view exposes) | PointValue.java:30 packedValue; callers BKDRadixSelector, BKDWriter |
| `util::bkd::mutable_point_tree_reader_utils::MutablePointTreeReaderUtils` | W? | **W** | wired: `BKDWriter`'s `MutablePointTree` path (`write_field_1dim`, `build_mutable`) sorts and partitions through it; the private copies over `BuildPoints` in bkd.rs (`SortByDimOps`, `PartitionOps`, `sort_by_packed_value`, `apply_permutation`, `BKDWriter::sort_by_dim`/`partition_by_dim`) removed | MutablePointTreeReaderUtils.java:36; callers BKDWriter.java:613,1709,1802 |
| `util::bkd::mutable_point_tree_reader_utils::MutablePointTreeReaderUtils::partition` | W? | **W** | wired: `BKDWriter::build_mutable` partitions each inner node with it instead of the private `BKDWriter::partition_by_dim`. Control L32 | MutablePointTreeReaderUtils.java:146-241; caller BKDWriter.java:1802 |
| `util::bkd::mutable_point_tree_reader_utils::MutablePointTreeReaderUtils::sort` | W? | **W** | wired: `BKDWriter::write_field_1dim` calls it (Java's `StableMSBRadixSorter` over value then doc-ID bytes, with the sorted-by-doc shortcut) instead of a private std stable sort plus permutation. Controls L30 (and L33 for its stable fallback) | MutablePointTreeReaderUtils.java:41-83; caller BKDWriter.java:613 writeField1Dim |
| `util::bkd::mutable_point_tree_reader_utils::MutablePointTreeReaderUtils::sort_by_dim` | W? | **W** | wired: `BKDWriter::build_mutable` sorts each leaf with it instead of the private `BKDWriter::sort_by_dim`. Control L31 | MutablePointTreeReaderUtils.java:88-140; caller BKDWriter.java:1709 |
| `util::byte_block_pool::ByteBlockPool::append` | W? | **W** | wired: `NodeHash`'s `copy_node_bytes` appends with it (Java's eager `nextBuffer` when a block fills) instead of the private `pool_append` of byte_block_pool_reverse_bytes_reader.rs (lazy block allocation), removed. `BytesRefArray` still appends through its private `ByteArena` (see `set_bytes_ref`). Control L36 | ByteBlockPool.java:287-306 append(byte[], int, int); callers NodeHash.java:304, BytesRefArray.java:107 |
| `util::byte_block_pool::ByteBlockPool::append_pool` | D? | **W** | Java's `append(ByteBlockPool, long, int)` (renamed overload); wired: `NodeHash`'s `copy_fallback_node_bytes` calls it instead of the private `pool_append_from_pool` (removed). Test `a_tiny_suffix_ram_limit_promotes_fallback_nodes_as_lucene_does` (Java-pinned, oracle FstFallback.java), control L37 | ByteBlockPool.java:238-253; caller NodeHash.java:319 |
| `util::byte_block_pool::ByteBlockPool::position` | W? | **W** | wired: `NodeHash` (`add`, `copy_node_bytes`, `copy_fallback_node_bytes`) reads it instead of the private `pool_position` (removed). Control L34 | ByteBlockPool.java:352 getPosition; callers NodeHash.java:160,307,322,336 |
| `util::byte_block_pool::ByteBlockPool::read_byte` | W? | **W** | wired: `ByteBlockPoolReverseBytesReader` reads every byte with it instead of the private `pool_read_byte` (removed; it mapped Java's `ArrayIndexOutOfBoundsException` to `CorruptIndex`); the parameter is Java's `long` (was `i32`), with Java's unchecked `int` cast of the block index. Control L35 | ByteBlockPool.java:335-339 readByte(long); callers ByteBlockPoolReverseBytesReader.java:36,42 |
| `util::byte_block_pool::ByteBlockPool::set_bytes_ref` | W? | **W-blocked** | blocked: `BytesRefArray` stores into a private `ByteArena` copy (bytes_ref_array.rs `ByteArena`, declared); moving it onto `ByteBlockPool` needs `reset(false, true)` (first-buffer reuse) and a `Counter`-tracking allocator, neither ported (`ByteBlockPool` takes `Arc<AtomicI64>` and has only `reset(false, false)`) | ByteBlockPool.java:207-224 setBytesRef, :147-185 reset(boolean, boolean), :86-104 DirectTrackingAllocator; caller BytesRefArray.java:116,142,167 |
| `util::bytes_ref_array::BytesRefBlockPool` | W? | **W=** | none: Rust `BytesRefHash` and the `FreqProxFields`/`TermVectorsConsumerPerField` ports reach the term bytes through `ByteBlockPool`, which carries `BytesRefBlockPool`'s methods (declared in the `BytesRefBlockPool` docs and in bytes_ref_hash.rs module docs) | BytesRefBlockPool.java:37; callers BytesRefHash.java:47,134, FreqProxFields.java:123, TermVectorsConsumerPerField.java:43 |
| `util::bytes_ref_array::BytesRefBlockPool::add_bytes_ref` | W? | **W=** | none: `BytesRefHash::add` calls `ByteBlockPool::add_bytes_ref` (bytes_ref_hash.rs `add`) | BytesRefBlockPool.java:83; caller BytesRefHash.java:343 |
| `util::bytes_ref_array::BytesRefBlockPool::byte_block_pool` | W? | **W=** | none: Java's field `byteBlockPool`, read only by `BytesRefBlockPool`'s own methods; the Rust methods read the field | BytesRefBlockPool.java:42 |
| `util::bytes_ref_array::BytesRefBlockPool::equals` | W? | **W=** | none: `BytesRefHash::find_hash` compares `pool.term_bytes(..)` with the probe (bytes_ref_hash.rs `find_hash`) | BytesRefBlockPool.java:150-170; caller BytesRefHash.java:382 |
| `util::bytes_ref_array::BytesRefBlockPool::fill_bytes_ref` | W? | **W=** | none: `BytesRefHash::get` and the freq-prox terms enum read `pool.term_bytes(..)` (bytes_ref_hash.rs `get`, freq_prox_terms_writer.rs `term_bytes`) | BytesRefBlockPool.java:60-81; callers BytesRefHash.java:166, FreqProxFields.java:154,174, TermVectorsConsumerPerField.java:109 |
| `util::bytes_ref_array::BytesRefBlockPool::hash` | W? | **W=** | none: `BytesRefHash::rehash` reads `pool.hash_at(..)` (bytes_ref_hash.rs `rehash`) | BytesRefBlockPool.java:117-127; caller BytesRefHash.java:303 |
| `util::bytes_ref_array::BytesRefBlockPool::reset` | W? | **W=** | none: `BytesRefHash::close` resets the pool with `pool.reset()` (bytes_ref_hash.rs `close`) | BytesRefBlockPool.java:53; caller BytesRefHash.java:299 |
| `util::bytes_ref_array::EmptyBytesRefIterator` | D? | **P** | none (constant counterpart) | BytesRefIterator.java:37 EMPTY; no core caller |
| `util::bytes_ref_array::FixedLengthBytesRefArray::get` | W? | **W-blocked** | blocked: `OneDimensionBKDWriter` keeps the split values in a flat `Vec<u8>` (bkd.rs `leaf_block_start_values`) because the port's `BKDTreeLeafNodes` is a struct of flat arrays, where Java's is an interface whose anonymous implementation reads `leafBlockStartValues.get` | FixedLengthBytesRefArray.java:137; caller BKDWriter.java:698-699,801 (BKDTreeLeafNodes BKDWriter.java:425) |
| `util::bytes_ref_array::FixedLengthBytesRefArray::iterator` | W? | **W=** | none: `OfflineSorter`'s `SortPartitionTask` calls the trait form `SortableBytesRefArray::sorted_iterator` (offline_sorter.rs `SortPartitionTask`), whose `FixedLengthBytesRefArray` impl builds the same sorted iterator; both are Java's one `iterator(Comparator)` | FixedLengthBytesRefArray.java:154; caller OfflineSorter.java:637 |
| `util::bytes_ref_array::SortState::indices` | W? | **W=** | none: `iterator_with_sort_state` reads the `indices` field, as Java reads the field | BytesRefArray.java:211,240 |
| `util::bytes_ref_hash::BytesRefHash::close` | P? | **P** | none | BytesRefHash.java:314; no core caller |
| `util::bytes_ref_hash::BytesRefHash::compact` | W? | **W-blocked** | blocked: `BytesRefHash::sort` is ported non-destructively (declared in its doc) and builds its own `0..count` id array; running Java's destructive `compact()` first needs `sort(&mut self)` with the bucket cache in the second half of `ids` | BytesRefHash.java:179-191 compact; caller BytesRefHash.java:198-199 sort |
| `util::bytes_ref_hash::BytesRefHash::do_hash` | W? | **W=** | none: the callers hash with `byte_block_pool::hash_bytes` (bytes_ref_hash.rs `add`/`find`, byte_block_pool.rs `hash_at`), which `do_hash` is defined as | BytesRefHash.java:472 doHash; callers BytesRefHash.java:332,364,371, BytesRefBlockPool.java:122 |
| `util::bytes_ref_hash::BytesRefHash::is_empty` | D? | **T** | none (used by bytes_ref_hash.rs tests only) | no Lucene member: BytesRefHash.java declares size() only |
| `util::bytes_ref_hash::BytesRefHash::is_keyed_by_pool_offset` | D? | **T** | none (used by bytes_ref_hash.rs tests only) | no Lucene member: Java's secondary table is a plain BytesRefHash fed by addByPoolOffset (BytesRefHash.java:397) |
| `util::bytes_ref_hash::BytesRefHash::new` | W? | **W=** | none: the Java constructor's callers are ported through the sibling constructors (`with_direct_bytes_start` for SortedDocValuesWriter/SortedSetDocValuesWriter/BufferedUpdates.BytesRefIntMap, `for_postings` for TermsHashPerField); ScoringRewrite deduplicates with a `HashMap` (declared, scoring_rewrite.rs `ParallelArraysTermCollector`) | BytesRefHash.java:88-109; callers SortedDocValuesWriter.java:59, SortedSetDocValuesWriter.java:71, TermsHashPerField.java:82, BufferedUpdates.java:307, ScoringRewrite.java:128 |
| `util::bytes_ref_hash::BytesRefHash::new_by_pool_offset` | D? | **T** | none (used by bytes_ref_hash.rs tests only; production uses `for_postings_by_pool_offset`) | no Lucene member (as `is_keyed_by_pool_offset`) |
| `util::bytes_ref_hash::BytesRefHash::ram_bytes_used` | W? | **P** | none | BytesRefHash.java:505; no core caller (the listed callers are other classes' ramBytesUsed) |
| `util::chars_ref::CharsRef::char_at` | P? | **P** | none | CharsRef.java:140 |
| `util::chars_ref::CharsRef::deep_copy_of` | P? | **P** | none | CharsRef.java:216 |
| `util::chars_ref::CharsRef::get_utf16_sorted_as_utf8_comparator` | P? | **P** | none | CharsRef.java:164 |
| `util::chars_ref::CharsRef::is_valid` | W? | **W** | wired: `from_chars_offset` (Java's `CharsRef(char[], int, int)`) asserts it as Java's `assert isValid()`, replacing an always-on custom bounds `assert!`; messages are Java's. Tests `from_chars_offset_asserts_is_valid`, `is_valid_reports_lucenes_messages` | CharsRef.java:224-256 isValid; caller CharsRef.java:56 |
| `util::chars_ref::CharsRef::length` | W? | **W=** | none: Java's public field `length`; the Rust callers read the field | CharsRef.java:40 |
| `util::chars_ref::CharsRef::offset` | W? | **W=** | none: Java's public field `offset`; the Rust callers read the field | CharsRef.java:37 |
| `util::chars_ref::CharsRef::sub_sequence` | P? | **P** | doc declares the copy (Java shares the array) | CharsRef.java:146-150 |
| `util::chars_ref::CharsRef::with_capacity` | D? | **P** | fixed: the buffer holds `capacity` NULs (was an empty `Vec` with that capacity), as Java's `new char[capacity]`. Test `empty_and_capacity` | CharsRef.java:47-49 CharsRef(int) |
| `util::collection_util::intro_sort` | D? | **P** | none (renamed overload) | CollectionUtil.java:171 introSort(List) |
| `util::collection_util::new_hash_map` | D? | **P** | none | CollectionUtil.java:44 newHashMap(int) |
| `util::collection_util::new_hash_set` | D? | **P** | none | CollectionUtil.java:55 newHashSet(int) |
| `util::collection_util::tim_sort` | D? | **P** | none (renamed overload) | CollectionUtil.java:201 timSort(List) |
| `util::command_line_util::CommandLineUtil::load_directory_class` | P? | **P** | fixed (finding 4): it accepts every `org.apache.lucene.store` `Directory` (12 classes, from `javap`), not only the `FSDirectory` kinds, and returns the class's binary name (declared: no `Class` objects); failures are Java's `ClassNotFoundException`/`ClassCastException` texts. Test `the_load_methods_throw_what_class_for_name_throws` (oracle `Cli.java`) | CommandLineUtil.java:76-79 loadDirectoryClass (no core caller) |
| `util::command_line_util::FSDirectoryKind::class_name` | D? | **D** | removed | no Lucene member: the kind stands in for a `Class<? extends FSDirectory>`, whose name Lucene never asks for in `CommandLineUtil` |
| `util::concurrent::Counter::is_thread_safe` | D? | **D** | removed | no Lucene member: Counter.java:27-90 declares addAndGet, get, newCounter() and newCounter(boolean) only |
| `util::concurrent::NamedThreadFactory` | W? | **W-blocked** | blocked: `CheckIndex` spawns one scoped thread per segment gated by permits (declared at check_index.rs `run_async_segment_check`) instead of Java's fixed thread pool, so there is no `ThreadFactory` to hand over | NamedThreadFactory.java:29; caller CheckIndex.java:552 |
| `util::concurrent::NamedThreadFactory::new` | W? | **W-blocked** | blocked: as `NamedThreadFactory` | NamedThreadFactory.java:41; caller CheckIndex.java:552 |
| `util::concurrent::NamedThreadFactory::new_thread` | P? | **P** | none | NamedThreadFactory.java:72 |
| `util::concurrent::NamedThreadFactory::thread_name_prefix` | W? | **W=** | none: Java's field, read by `newThread`; the Rust `next_thread_name`/`new_thread` read the field | NamedThreadFactory.java:34,78 |
| `util::concurrent::SameThreadExecutorService::await_termination` | P? | **P** | none | SameThreadExecutorService.java:63 |
| `util::concurrent::SameThreadExecutorService::execute` | P? | **P** | none | SameThreadExecutorService.java:34 |
| `util::concurrent::SameThreadExecutorService::is_terminated` | P? | **P** | none | SameThreadExecutorService.java:51 |
| `util::concurrent::SameThreadExecutorService::shutdown_now` | P? | **P** | none | SameThreadExecutorService.java:40 |
| `util::concurrent::SetOnce` | W? | **W=** | none: the two Java owners hold their once-set fields as std primitives with the same contract: `IndexWriterConfig.writer` is a `OnceLock` (index_writer_config.rs `index_writer`, declared), and `DocumentsWriterPerThread.flushPending`/`hasFlushed` are atomics (documents_writer.rs module docs) | SetOnce.java:28; callers DocumentsWriterPerThread.java:126-128, IndexWriterConfig.java:112 |
| `util::concurrent::SetOnce::get` | W? | **W=** | none: as `SetOnce` (`OnceLock::get`, atomic loads) | SetOnce.java:84; callers DocumentsWriterPerThread.java:443,540, IndexWriterConfig.java:120,502 |
| `util::concurrent::SetOnce::set` | W? | **W=** | none: as `SetOnce` (`OnceLock::set` after Java's `get() != null` check, atomic stores) | SetOnce.java:52; callers DocumentsWriterPerThread.java:462,472, IndexWriterConfig.java:124 |
| `util::concurrent::WeakIdentityMap` | P? | **P** | none | WeakIdentityMap.java:60 |
| `util::concurrent::WeakIdentityMap::clear` | P? | **P** | none | WeakIdentityMap.java:108 |
| `util::concurrent::WeakIdentityMap::contains_key` | P? | **P** | none | WeakIdentityMap.java:114 |
| `util::concurrent::WeakIdentityMap::get` | P? | **P** | none | WeakIdentityMap.java:120 |
| `util::concurrent::WeakIdentityMap::is_empty` | P? | **P** | none | WeakIdentityMap.java:135 |
| `util::concurrent::WeakIdentityMap::key_iterator` | P? | **P** | none | WeakIdentityMap.java:165 |
| `util::concurrent::WeakIdentityMap::new_concurrent_hash_map` | W? | **W=** | none: both concurrent factories delegate to `new_hash_map_with`, the body of Java's `newConcurrentHashMap(boolean)`, with Java's `true` | WeakIdentityMap.java:87-89 newConcurrentHashMap() -> newConcurrentHashMap(true) |
| `util::concurrent::WeakIdentityMap::new_concurrent_hash_map_with` | D? | **P** | none (renamed overload) | WeakIdentityMap.java:97 newConcurrentHashMap(boolean) |
| `util::concurrent::WeakIdentityMap::put` | P? | **P** | none | WeakIdentityMap.java:129 |
| `util::concurrent::WeakIdentityMap::remove` | P? | **P** | none | WeakIdentityMap.java:145 |
| `util::concurrent::WeakIdentityMap::value_iterator` | P? | **P** | none | WeakIdentityMap.java:227 |
| `util::doc_id_set::BitDocIdSet::from_set` | D? | **P** | none (renamed constructor) | BitDocIdSet.java:51-53 BitDocIdSet(BitSet); no core caller |
| `util::doc_id_set::BitDocIdSet::set` | W? | **W=** | none: Java's field `set`, read by `bits()`/`iterator()`/`ramBytesUsed()`/`toString()`; the Rust impls read the field | BitDocIdSet.java:32 |
| `util::doc_id_set::DocIdSetBuilder::multivalued` | W? | **W=** | none: Java's field, read by `build()`; the Rust `build` reads the field | DocIdSetBuilder.java:137,310 |
| `util::doc_id_set::DocIdSetBuilder::num_values_per_doc` | W? | **W=** | none: Java's field, read by `build()`; the Rust `build` reads the field | DocIdSetBuilder.java:138,303 |
| `util::doc_id_set::IntArrayDocIdSet::is_empty` | D? | **D** | removed | no Lucene member: IntArrayDocIdSet.java:25-61 declares ramBytesUsed and iterator only |
| `util::doc_id_set::IntArrayDocIdSet::len` | D? | **D** | removed | no Lucene member (as `is_empty`) |
| `util::doc_id_set::NotDocIdSet` | W? | **W-blocked** | blocked: the Rust `RoaringDocIdSet::Builder::flush` stores a dense block as a `RoaringBlock` variant over the excluded docs instead of `new NotDocIdSet(BLOCK_SIZE, new IntArrayDocIdSet(..))`, a structural change (see `FixedBitSet::flip`) | NotDocIdSet.java:30; caller RoaringDocIdSet.java:101 |
| `util::doc_id_set::NotDocIdSet::new` | W? | **W-blocked** | blocked: as `NotDocIdSet` | NotDocIdSet.java:39; caller RoaringDocIdSet.java:101 |
| `util::extra::LongBitSet::and` | P? | **P** | none | LongBitSet.java:266 and |
| `util::extra::LongBitSet::and_not` | P? | **P** | none | LongBitSet.java:277 andNot |
| `util::extra::LongBitSet::clear` | P? | **P** | none | LongBitSet.java:171 clear(long) |
| `util::extra::LongBitSet::clear_range` | D? | **P** | none (renamed overload) | LongBitSet.java:392 clear(long, long) |
| `util::extra::LongBitSet::ensure_capacity` | P? | **P** | none | LongBitSet.java:44 ensureCapacity |
| `util::extra::LongBitSet::flip` | P? | **P** | none | LongBitSet.java:347 flip(long) |
| `util::extra::LongBitSet::flip_range` | D? | **P** | none (renamed overload) | LongBitSet.java:310 flip(long, long) |
| `util::extra::LongBitSet::get_and_clear` | P? | **P** | none | LongBitSet.java:178 getAndClear |
| `util::extra::LongBitSet::get_bits` | W? | **W=** | none (`ensure_capacity` takes the backing array by value, the array `getBits()` hands Java's `ensureCapacity`) | LongBitSet.java:129 getBits; caller LongBitSet.java:51 |
| `util::extra::LongBitSet::intersects` | P? | **P** | none | LongBitSet.java:256 intersects |
| `util::extra::LongBitSet::length` | P? | **P** | none | LongBitSet.java:124 length |
| `util::extra::LongBitSet::next_set_bit` | P? | **P** | none | LongBitSet.java:191 nextSetBit |
| `util::extra::LongBitSet::or` | P? | **P** | none | LongBitSet.java:236 or |
| `util::extra::LongBitSet::prev_set_bit` | P? | **P** | none | LongBitSet.java:215 prevSetBit |
| `util::extra::LongBitSet::scan_is_empty` | P? | **P** | none | LongBitSet.java:290 scanIsEmpty |
| `util::extra::LongBitSet::set_range` | D? | **P** | none (renamed overload) | LongBitSet.java:360 set(long, long) |
| `util::extra::LongBitSet::xor` | P? | **P** | none | LongBitSet.java:246 xor |
| `util::extra::MergedIterator::new` | W? | **W-blocked** | blocked: `MultiFields::iterator` unions its subs through a `BTreeSet` (multi_fields.rs:276-289) because its sub `Fields` iterate in `FieldInfos` order (multi_fields.rs:97-109), while `MergedIterator` needs sorted subs, as Java's `FieldsProducer` iterators are. `PerFieldPostingsFormat` merge already uses the two-argument constructor (per_field.rs:300, W=) | MergedIterator.java:53 MergedIterator(Iterator...); callers MultiFields.java:57, PerFieldPostingsFormat.java:177 (two-argument form) |
| `util::extra::PriorityQueue::remove` | P? | **P** | none | PriorityQueue.java:287 remove |
| `util::extra::Version::get_package_implementation_version` | P? | **P** | none | Version.java getPackageImplementationVersion |
| `util::file_deleter::FileDeleter::delete_file_if_no_ref` | P? | **P** | none | FileDeleter.java:198 |
| `util::file_deleter::FileDeleter::force_delete` | P? | **P** | none | FileDeleter.java:193 |
| `util::file_deleter::FileDeleter::get_ref_count` | P? | **P** | none | FileDeleter.java:144 |
| `util::fst::byte_sequence_outputs::ByteSequenceOutputs` | P? | **P** | none | ByteSequenceOutputs.java:32 |
| `util::fst::byte_sequence_outputs::ByteSequenceOutputs::get_singleton` | P? | **P** | none | ByteSequenceOutputs.java:39 |
| `util::fst::bytes_ref_fst_enum::BytesRefFSTEnum` | P? | **P** | none | BytesRefFSTEnum.java:28 |
| `util::fst::bytes_ref_fst_enum::BytesRefFSTEnum::current` | W? | **W=** | none: Java's callers read the field `current`, as the Rust methods do; the public `current()` has no core caller | BytesRefFSTEnum.java:29,102 |
| `util::fst::bytes_ref_fst_enum::BytesRefFSTEnum::new` | P? | **P** | none | BytesRefFSTEnum.java:43 |
| `util::fst::bytes_ref_fst_enum::BytesRefFSTEnum::next` | P? | **P** | none | BytesRefFSTEnum.java:53 |
| `util::fst::bytes_ref_fst_enum::BytesRefFSTEnum::seek_ceil` | P? | **P** | none | BytesRefFSTEnum.java:60 |
| `util::fst::bytes_ref_fst_enum::BytesRefFSTEnum::seek_exact` | P? | **P** | none | BytesRefFSTEnum.java:80 |
| `util::fst::bytes_ref_fst_enum::BytesRefFSTEnum::seek_floor` | P? | **P** | none | BytesRefFSTEnum.java:68 |
| `util::fst::char_sequence_outputs::CharSequenceOutputs` | P? | **P** | none | CharSequenceOutputs.java:31 |
| `util::fst::char_sequence_outputs::CharSequenceOutputs::get_singleton` | P? | **P** | none | CharSequenceOutputs.java:38 |
| `util::fst::forward_bytes_reader::ForwardBytesReader` | D? | **P** | none (faithful member uncalled in Lucene) | ForwardBytesReader.java:23; no caller anywhere in lucene/ |
| `util::fst::forward_bytes_reader::ForwardBytesReader::new` | D? | **P** | none (as the struct) | ForwardBytesReader.java:27 |
| `util::fst::fst::Arc::bit_table_start` | W? | **W=** | none: Java's field, read by `copyFrom` and `BitTable`; the Rust code reads the field | FST.java:183 |
| `util::fst::fst::Arc::flags` | W? | **W=** | none: Java's field, read by `FST`; the Rust code reads the field | FST.java:157 |
| `util::fst::fst::Arc::next_arc` | W? | **W=** | none: Java's field, read by `FST`; the Rust code reads the field | FST.java:161 |
| `util::fst::fst::Arc::presence_index` | W? | **W=** | none: Java's field, read by `readNextRealArc`/`copyFrom`; the Rust code reads the field | FST.java:190 |
| `util::fst::fst::FST::empty_output` | P? | **P** | none | FST.java:510 getEmptyOutput |
| `util::fst::fst::FST::from_fst_reader` | P? | **P** | none | FST.java:438 |
| `util::fst::fst::FST::num_bytes` | P? | **P** | none | FST.java:506 |
| `util::fst::fst::FST::read_from_path` | D? | **P** | none (renamed static) | FST.java:538 read(Path, Outputs) |
| `util::fst::fst::FST::save` | W? | **W=** | none: Java's only caller is `save(Path)`, whose `save(out, out)` the Rust `save_to_path` runs as its two statements on the one output (Rust cannot pass `&mut out` twice) | FST.java:524; caller FST.java:530-534 |
| `util::fst::fst::FST::save_to_path` | D? | **P** | none (renamed overload) | FST.java:530 save(Path) |
| `util::fst::fst::FSTMetadata::empty_output` | W? | **W=** | none: Java's field `emptyOutput`; the Rust callers read the field | FST.java:1200, field FST.java:1172 |
| `util::fst::fst::FSTMetadata::outputs` | W? | **W=** | none: Java's field, read by the FST constructor and `save`; the Rust code reads the field | FST.java:1159 |
| `util::fst::fst::FSTMetadata::start_node` | W? | **W=** | none: Java's field, read by `getFirstArc`/`save` and set by `FSTCompiler.finish`; the Rust code uses the field | FST.java:1161 |
| `util::fst::fst::FSTMetadata::version` | W? | **W=** | none: Java's field, read by `readLabel`; the Rust `read_label` reads the field | FST.java:1162,1196 |
| `util::fst::fst_compiler::Builder` | P? | **P** | none | FSTCompiler.java:241 Builder |
| `util::fst::fst_compiler::Builder::allow_fixed_length_arcs` | W? | **P** | none: the public setter `allowFixedLengthArcs(boolean)`; the field it sets is read by the Rust `build`, as Java's `build()` reads it | FSTCompiler.java:295; no core caller builds an FSTCompiler |
| `util::fst::fst_compiler::Builder::build` | P? | **P** | none | FSTCompiler.java:350 |
| `util::fst::fst_compiler::Builder::data_output` | W? | **P** | none: the public setter `dataOutput(DataOutput)`; the field it sets is read by the Rust `build`, as Java's `build()` reads it | FSTCompiler.java:311; no core caller builds an FSTCompiler |
| `util::fst::fst_compiler::Builder::direct_addressing_max_oversizing_factor` | W? | **P** | none: the public setter `directAddressingMaxOversizingFactor(float)`; the field it sets is read by the Rust `build`, as Java's `build()` reads it | FSTCompiler.java:329; no core caller builds an FSTCompiler |
| `util::fst::fst_compiler::Builder::new` | P? | **P** | none | FSTCompiler.java:259 |
| `util::fst::fst_compiler::Builder::set_version` | P? | **P** | none | FSTCompiler.java:335 |
| `util::fst::fst_compiler::Builder::suffix_ram_limit_mb` | W? | **P** | none: the public setter `suffixRAMLimitMB(double)`; the field it sets is read by the Rust `build`, as Java's `build()` reads it | FSTCompiler.java:280; no core caller builds an FSTCompiler |
| `util::fst::fst_compiler::FSTCompiler::add` | P? | **P** | none | FSTCompiler.java:841 |
| `util::fst::fst_compiler::FSTCompiler::arc_count` | W? | **W=** | none: Java's field `arcCount`, incremented by `addNode`; the Rust `add_node` increments the field | FSTCompiler.java:130,375,432 |
| `util::fst::fst_compiler::FSTCompiler::binary_search_node_count` | W? | **W=** | none: Java's field, incremented by `addNode`; the Rust code increments the field | FSTCompiler.java:132,553 |
| `util::fst::fst_compiler::FSTCompiler::compile` | P? | **P** | none | FSTCompiler.java:981 |
| `util::fst::fst_compiler::FSTCompiler::continuous_node_count` | W? | **W=** | none: Java's field, incremented by `addNode`; the Rust code increments the field | FSTCompiler.java:134,545 |
| `util::fst::fst_compiler::FSTCompiler::direct_addressing_max_oversizing_factor` | W? | **W=** | none: Java's field, read by the arc-layout code; the Rust code reads the field | FSTCompiler.java:137,366,614,630 |
| `util::fst::fst_compiler::FSTCompiler::direct_addressing_node_count` | W? | **W=** | none: Java's field, incremented by `addNode`; the Rust code increments the field | FSTCompiler.java:133,550 |
| `util::fst::fst_compiler::FSTCompiler::fst_ram_bytes_used` | P? | **P** | none | FSTCompiler.java:1035 |
| `util::fst::fst_compiler::FSTCompiler::fst_reader` | P? | **P** | none | FSTCompiler.java:227 getFSTReader |
| `util::fst::fst_compiler::FSTCompiler::fst_size_in_bytes` | P? | **P** | none | FSTCompiler.java:1043 |
| `util::fst::fst_compiler::FSTCompiler::into_fst_reader` | D? | **P** | none: the owning form of `getFSTReader()`, for `FST.fromFSTReader(metadata, compiler.getFSTReader())`, whose Rust `from_fst_reader` takes the reader by value (used by the module docs and tests) | FSTCompiler.java:227 getFSTReader; FST.java:438 |
| `util::fst::fst_compiler::FSTCompiler::node_count` | W? | **W=** | none: Java's field `nodeCount`, incremented by `addNode`; the Rust code increments the field | FSTCompiler.java:131,370,565 |
| `util::fst::int_sequence_outputs::IntSequenceOutputs` | P? | **P** | none | IntSequenceOutputs.java:31 |
| `util::fst::int_sequence_outputs::IntSequenceOutputs::get_singleton` | P? | **P** | none | IntSequenceOutputs.java:38 |
| `util::fst::ints_ref_fst_enum::IntsRefFSTEnum` | P? | **P** | none | IntsRefFSTEnum.java:28 |
| `util::fst::ints_ref_fst_enum::IntsRefFSTEnum::current` | W? | **W=** | none: Java's callers read the field `current`, as the Rust methods do; the public `current()` has no core caller | IntsRefFSTEnum.java:29,102 |
| `util::fst::ints_ref_fst_enum::IntsRefFSTEnum::new` | P? | **P** | none | IntsRefFSTEnum.java:43 |
| `util::fst::ints_ref_fst_enum::IntsRefFSTEnum::next` | P? | **P** | none | IntsRefFSTEnum.java:53 |
| `util::fst::ints_ref_fst_enum::IntsRefFSTEnum::seek_ceil` | P? | **P** | none | IntsRefFSTEnum.java:60 |
| `util::fst::ints_ref_fst_enum::IntsRefFSTEnum::seek_exact` | P? | **P** | none | IntsRefFSTEnum.java:80 |
| `util::fst::ints_ref_fst_enum::IntsRefFSTEnum::seek_floor` | P? | **P** | none | IntsRefFSTEnum.java:68 |
| `util::fst::no_outputs::NoOutputs` | P? | **P** | none | NoOutputs.java:27 |
| `util::fst::no_outputs::NoOutputs::get_singleton` | P? | **P** | none | NoOutputs.java:48 |
| `util::fst::no_outputs::NoOutputs::no_output_value` | D? | **D** | removed (a static duplicate of `impl Outputs for NoOutputs::no_output`, which is Java's `getNoOutput`) | no further Lucene member: NoOutputs.java:93 getNoOutput is the trait method; NO_OUTPUT (:29) is package-private and is the `()` value itself |
| `util::fst::off_heap_fst_store::OffHeapFSTStore` | P? | **P** | none | OffHeapFSTStore.java:30 |
| `util::fst::off_heap_fst_store::OffHeapFSTStore::new` | P? | **P** | none | OffHeapFSTStore.java:39 |
| `util::fst::off_heap_fst_store::OffHeapFSTStore::size` | P? | **P** | none | OffHeapFSTStore.java:50 |
| `util::fst::pair_outputs::PairOutputs` | P? | **P** | none | PairOutputs.java:29 |
| `util::fst::pair_outputs::PairOutputs::new` | P? | **P** | none | PairOutputs.java:69 |
| `util::fst::pair_outputs::PairOutputs::outputs1` | W? | **W=** | none: Java's field, read by `PairOutputs`' own methods; the Rust methods read the field | PairOutputs.java:32 |
| `util::fst::pair_outputs::PairOutputs::outputs2` | W? | **W=** | none: Java's field, read by `PairOutputs`' own methods; the Rust methods read the field | PairOutputs.java:33 |
| `util::fst::positive_int_outputs::PositiveIntOutputs` | P? | **P** | none | PositiveIntOutputs.java:29 |
| `util::fst::positive_int_outputs::PositiveIntOutputs::get_singleton` | P? | **P** | none | PositiveIntOutputs.java:37 |
| `util::fst::read_write_data_output::ReadWriteDataOutput::is_frozen` | D? | **D** | removed | no Lucene member: `frozen` is a private field read only by assertions (ReadWriteDataOutput.java:40,52,58), which the Rust `debug_assert!`s read directly |
| `util::fst::util::TopResults::iter` | D? | **P** | none (renamed: `Iterable.iterator`) | Util.java:432-453 TopResults.iterator |
| `util::fst::util::Util` | W? | **W=** | none: `FSTEnum` calls Java's static `Util.binarySearch`, ported as the module function `fst::util::binary_search`, which the Rust `FSTEnum` calls | Util.java:43; callers FSTEnum.java:228,547,560 |
| `util::fst::util::Util::get` | P? | **P** | none | Util.java:47 get(FST, IntsRef) |
| `util::fst::util::Util::get_bytes` | D? | **P** | none (renamed overload) | Util.java:73 get(FST, BytesRef) |
| `util::fst::util::Util::read_ceil_arc` | P? | **P** | none | Util.java:819 |
| `util::fst::util::Util::shortest_paths` | P? | **P** | none | Util.java:456 |
| `util::fst::util::Util::to_bytes_ref` | P? | **P** | none | Util.java:787 |
| `util::fst::util::Util::to_dot` | P? | **P** | none | Util.java:500 |
| `util::fst::util::Util::to_ints_ref` | P? | **P** | none | Util.java:777 |
| `util::fst::util::Util::to_utf16` | P? | **P** | none | Util.java:728 |
| `util::fst::util::Util::to_utf32` | P? | **P** | none | Util.java:742 toUTF32(CharSequence, IntsRefBuilder) |
| `util::fst::util::Util::to_utf32_chars` | D? | **P** | none (renamed overload) | Util.java:761 toUTF32(char[], int, int, IntsRefBuilder) |
| `util::graph::GraphTokenStreamFiniteStrings::automaton` | D? | **D** | removed | no Lucene member: GraphTokenStreamFiniteStrings.java:47-200 keeps `det` private and exposes no accessor |
| `util::graph::GraphTokenStreamFiniteStrings::get_terms_for_field` | D? | **P** | none (renamed overload) | GraphTokenStreamFiniteStrings.java:120 getTerms(String, int); QueryBuilder.java:541 calls getTerms(int), as query_builder.rs does |
| `util::hnsw::blocking_float_heap::BlockingFloatHeap::new` | P? | **P** | none | BlockingFloatHeap.java:37 |
| `util::hnsw::blocking_float_heap::BlockingFloatHeap::offer` | W? | **P** | none: Java's `offer(float)`; `MultiLeafKnnCollector.collect` calls `offer(float[], int)`, ported as `offer_all` and called by multi_leaf_knn_collector.rs `collect` | BlockingFloatHeap.java:52; MultiLeafKnnCollector.java:117 -> :78 |
| `util::hnsw::blocking_float_heap::BlockingFloatHeap::peek` | P? | **P** | none | BlockingFloatHeap.java:128 |
| `util::hnsw::blocking_float_heap::BlockingFloatHeap::poll` | P? | **P** | none | BlockingFloatHeap.java:104 |
| `util::hnsw::blocking_float_heap::BlockingFloatHeap::size` | W? | **W=** | none: Java's field `size`, read by the heap's own methods; the Rust methods read the field | BlockingFloatHeap.java:35,55,82,105 |
| `util::hnsw::builder::HnswGraphBuilder::add_vectors` | W? | **W=** | none: `build` and the concurrent worker call `add_vectors_skipping`, which is Java's `addVectors(int, int)` with the worker's `addGraphNode` override as a parameter (declared at builder.rs `add_vectors_skipping`) | HnswGraphBuilder.java:247; callers HnswGraphBuilder.java:224,278, HnswConcurrentMergeBuilder.java:210 |
| `util::hnsw::builder::HnswGraphBuilder::connect_components` | W? | **P** | none (faithful member uncalled in Lucene: the call in `finish()` is commented out) | HnswGraphBuilder.java:555 `// connectComponents();`, :560 |
| `util::hnsw::concurrent_merge_builder::HnswConcurrentMergeBuilder::num_worker` | D? | **D** | removed | no Lucene member: HnswConcurrentMergeBuilder.java keeps `workers` private with no accessor |
| `util::hnsw::concurrent_merge_builder::HnswConcurrentMergeBuilder::set_batch_size` | D? | **P** | none (faithful member; Java and the port use it from tests) | HnswConcurrentMergeBuilder.java:159 |
| `util::hnsw::float_heap::FloatHeap::clear` | P? | **P** | none | FloatHeap.java:104 |
| `util::hnsw::float_heap::FloatHeap::get_heap` | P? | **P** | none | FloatHeap.java:63 |
| `util::hnsw::hnsw_lock::HnswLock` | W? | **W-blocked** | blocked: declared divergence, `OnHeapHnswGraph` locks every neighbour array itself because Java's workers also write without a lock, a data race in Rust (on_heap.rs `OnHeapHnswGraph` docs, hnsw_lock.rs docs), so the concurrent merge takes no stripe | HnswLock.java:27; callers HnswConcurrentMergeBuilder.java:63,182, HnswGraphBuilder.java:83 |
| `util::hnsw::hnsw_lock::HnswLock::read` | W? | **W-blocked** | blocked: as `HnswLock` | HnswLock.java:38; caller HnswConcurrentMergeBuilder.java:251 graphSeek |
| `util::hnsw::hnsw_lock::HnswLock::write` | W? | **W-blocked** | blocked: as `HnswLock` | HnswLock.java:45; caller HnswGraphBuilder.java:433 addDiverseNeighbors |
| `util::hnsw::incremental_merger::IncrementalHnswGraphMerger::num_readers` | W? | **W=** | none: Java's field, read by `addReader`/`createBuilder`; the Rust methods use the field | IncrementalHnswGraphMerger.java:54,88,147 |
| `util::hnsw::initialized_builder::InitializedHnswGraphBuilder::into_graph` | D? | **D** | removed (a duplicate of the builder's `HnswBuilder::into_completed_graph`) | no Lucene member beyond the inherited getCompletedGraph (HnswGraphBuilder.java:234), which `into_completed_graph` ports |
| `util::hnsw::neighbor::NeighborArray::insert_sorted` | D? | **P** | none (port of the package-private test helper, compiled for tests as Java uses it from tests) | NeighborArray.java:197 |
| `util::hnsw::neighbor::NeighborQueue::incomplete` | W? | **W=** | none: Java's field, read by `incomplete()`; the Rust code reads the field | NeighborQueue.java:57,170 |
| `util::hnsw::neighbor::NeighborQueue::mark_incomplete` | P? | **P** | none | NeighborQueue.java:174 |
| `util::hnsw::neighbor::NeighborQueue::set_visited_count` | P? | **P** | none | NeighborQueue.java:166 |
| `util::hnsw::neighbor::NeighborQueue::visited_count` | W? | **W=** | none: Java's field, read by `visitedCount()`; the Rust code reads the field | NeighborQueue.java:55,162 |
| `util::hnsw::on_heap::OnHeapHnswGraph::set_neighbors` | D? | **D** | removed | no Lucene member: OnHeapHnswGraph.java has no neighbour-array setter |
| `util::hnsw::ordinal_translated_knn_collector::OrdinalTranslatedKnnCollector` | W? | **W-blocked** | blocked: declared divergence, the Lucene99 reader collects ordinals into its own `TopKnnCollector` and translates them after the search, because this type owns the collector it wraps and the caller's arrives as a borrow (hnsw_vectors_format.rs `search_with_scorer` docs) | OrdinalTranslatedKnnCollector.java:27; caller Lucene99HnswVectorsReader.java:342 |
| `util::hnsw::ordinal_translated_knn_collector::OrdinalTranslatedKnnCollector::inner` | D? | **D** | removed | no Lucene member: KnnCollector.Decorator's `collector` is a protected field with no accessor (KnnCollector.java:103) |
| `util::hnsw::ordinal_translated_knn_collector::OrdinalTranslatedKnnCollector::new` | W? | **W-blocked** | blocked: as `OrdinalTranslatedKnnCollector` | OrdinalTranslatedKnnCollector.java:31; caller Lucene99HnswVectorsReader.java:342 |
| `util::hnsw::searcher::HnswGraphSearcher::search_graph` | D? | **D** | removed (a duplicate of the `AbstractHnswGraphSearcher::search` default the searcher already implements) | no further Lucene member: AbstractHnswGraphSearcher.search(KnnCollector, RandomVectorScorer, HnswGraph, Bits) is the trait method (abstract_searcher.rs `search`) |
| `util::hnsw::searcher::HnswGraphSearcher::search_on_heap` | D? | **P** | fixed: visits through a `SparseFixedBitSet` of `getGraphSize(graph)` bits, as Java (was `create_bit_set`, which may pick a `FixedBitSet` and pads an empty graph to one bit), so a graph without nodes is refused with Java's `length needs to be >= 1`. Test `search_on_heap_over_a_graph_without_nodes_throws_what_the_bit_set_throws` | HnswGraphSearcher.java:177-186 search(RandomVectorScorer, int, OnHeapHnswGraph, Bits, int); SparseFixedBitSet.java:63-66 |
| `util::hnsw::util::HnswUtil::component_sizes` | W? | **P** | none: Java's `componentSizes(HnswGraph, int)`; its one caller, the `componentSizes(HnswGraph)` overload, is not ported and has no core caller | HnswUtil.java:75; caller HnswUtil.java:64-66 |
| `util::hnsw::util::HnswUtil::graph_is_rooted` | P? | **P** | none | HnswUtil.java:208 |
| `util::info_stream::JavaLoggingInfoStream` | P? | **P** | none | JavaLoggingInfoStream.java:35 |
| `util::info_stream::JavaLoggingInfoStream::new` | W? | **P** | none: Java's `JavaLoggingInfoStream(Level)`, which chains to the prefix constructor as the Rust `new` does | JavaLoggingInfoStream.java:47-49 |
| `util::info_stream::PrintStreamInfoStream::is_system_stream` | W? | **W=** | none: `close` reads the field `is_system_stream` returns (Java's `close()` calls `isSystemStream()`; there is no subclass to override it) | PrintStreamInfoStream.java:72; caller :66 |
| `util::info_stream::PrintStreamInfoStream::message_id` | W? | **W=** | none: Java's field `messageID`, read by `message`; the Rust `message` reads the field | PrintStreamInfoStream.java:32,50 |
| `util::info_stream::PrintStreamInfoStream::new` | W? | **W-blocked** | blocked: both Java callers are unported: `IndexUpgrader.parseArgs` (no IndexUpgrader port) and the `IndexWriterConfig.setInfoStream(PrintStream)` overload (the port has only `set_info_stream(Arc<dyn InfoStream>)`) | PrintStreamInfoStream.java:36; callers IndexUpgrader.java:100, IndexWriterConfig.java:411-415 |
| `util::int_block_pool::IntBlockPool::num_buffers` | D? | **D** | removed | no Lucene member: Java exposes the public field `buffers` (IntBlockPool.java:62) and no count accessor |
| `util::int_block_pool::RecyclingByteBlockAllocator` | P? | **P** | none | RecyclingByteBlockAllocator.java:29 |
| `util::int_block_pool::RecyclingByteBlockAllocator::bytes_used` | W? | **W=** | none: Java's field `bytesUsed`, used by the allocator's own methods; the Rust methods use the field | RecyclingByteBlockAllocator.java:33,70,96 |
| `util::int_block_pool::RecyclingByteBlockAllocator::free_blocks` | W? | **W=** | none: Java's field `freeBlocks` and `freeBlocks(int)`, used by the allocator's own methods; the Rust methods use the field | RecyclingByteBlockAllocator.java:32,127 |
| `util::int_block_pool::RecyclingByteBlockAllocator::max_buffered_blocks` | W? | **W=** | none: Java's field, read by `recycleByteBlocks`; the Rust method reads the field | RecyclingByteBlockAllocator.java:31,79 |
| `util::int_block_pool::RecyclingByteBlockAllocator::num_buffered_blocks` | P? | **P** | none | RecyclingByteBlockAllocator.java:103 |
| `util::int_block_pool::RecyclingByteBlockAllocator::with_max_buffered_blocks` | D? | **P** | none (renamed constructor) | RecyclingByteBlockAllocator.java:54 RecyclingByteBlockAllocator(int) |
| `util::int_block_pool::RecyclingIntBlockAllocator` | P? | **P** | none | RecyclingIntBlockAllocator.java:29 |
| `util::int_block_pool::RecyclingIntBlockAllocator::bytes_used` | W? | **W=** | none: Java's field `bytesUsed`, used by the allocator's own methods; the Rust methods use the field | RecyclingIntBlockAllocator.java:33,72,98 |
| `util::int_block_pool::RecyclingIntBlockAllocator::free_blocks` | W? | **W=** | none: Java's field `freeBlocks` and `freeBlocks(int)`, used by the allocator's own methods; the Rust methods use the field | RecyclingIntBlockAllocator.java:32,129 |
| `util::int_block_pool::RecyclingIntBlockAllocator::max_buffered_blocks` | W? | **W=** | none: Java's field, read by `recycleIntBlocks`; the Rust method reads the field | RecyclingIntBlockAllocator.java:31,81 |
| `util::int_block_pool::RecyclingIntBlockAllocator::num_buffered_blocks` | P? | **P** | none | RecyclingIntBlockAllocator.java:105 |
| `util::int_block_pool::RecyclingIntBlockAllocator::with_block_size` | D? | **P** | none (renamed constructor) | RecyclingIntBlockAllocator.java:56 RecyclingIntBlockAllocator(int, int) |
| `util::io_function::FilterIterator` | P? | **P** | none | FilterIterator.java:32 |
| `util::io_function::FilterIterator::new` | P? | **P** | none | FilterIterator.java:41 |
| `util::io_function::FilterIterator::remove` | P? | **P** | none | FilterIterator.java:65 |
| `util::java_collections::JavaHashMap::copy_of` | D? | **T** | none (used by java_collections.rs tests, tests/portability/java_collections.rs and tests/portability/seal_bytes.rs only) | no Lucene member: emulates the JDK's `new HashMap<>(Map)` for the order tests |
| `util::java_collections::JavaHashMap::put_all` | D? | **T** | none (used by tests/portability/java_collections.rs only) | no Lucene member: emulates `HashMap.putAll` for the order tests |
| `util::java_collections::JavaHashMap::values_mut` | D? | **D** | removed | no Lucene member: an unused method of the port's java.util emulation (`HashMap.values()`), no caller anywhere |
| `util::java_collections::JavaHashSet::is_empty` | D? | **P** | none: the JDK `isEmpty()` of the emulated class, kept beside `len` (clippy `len_without_is_empty`) | no Lucene member; java.util.HashSet.isEmpty |
| `util::java_collections::JavaHashSet::is_treeified` | D? | **T** | none (used by tests/portability/java_collections.rs only) | no Lucene member: flags the declared tree-bin divergence of the emulation |
| `util::java_collections::JavaHashSet::table_length` | D? | **T** | none (used by java_collections.rs tests only) | no Lucene member: the emulated table's length |
| `util::java_collections::JavaIntHashMap::is_empty` | D? | **P** | none: as `JavaHashSet::is_empty` | no Lucene member; java.util.HashMap.isEmpty |
| `util::java_collections::JavaIntHashMap::is_treeified` | D? | **P** | none: the flag of the declared tree-bin divergence named in the `JavaIntHashMap` docs (its `treeified` field has no other reader) | no Lucene member; see JavaHashMap::is_treeified |
| `util::java_collections::JavaIntHashMap::table_length` | D? | **D** | removed | no Lucene member: an unused method of the port's java.util emulation (table length), no caller anywhere |
| `util::java_collections::JavaLongHashMap::is_empty` | D? | **P** | none: as `JavaHashSet::is_empty` | no Lucene member; java.util.HashMap.isEmpty |
| `util::java_collections::JavaLongHashMap::is_treeified_at` | D? | **T** | none (used by tests/java_long_hash_map_order.rs only) | no Lucene member: flags the declared tree-bin divergence |
| `util::java_collections::JavaLongHashMap::keys` | D? | **D** | removed | no Lucene member: an unused method of the port's java.util emulation (`HashMap.keySet()`), no caller anywhere |
| `util::java_collections::JavaLongHashMap::table_length` | D? | **T** | none (used by tests/java_long_hash_map_order.rs only) | no Lucene member: the emulated table's length |
| `util::java_collections::JavaStringSet::is_empty` | D? | **P** | none: as `JavaHashSet::is_empty` (used by store.rs tests) | no Lucene member; java.util.HashSet.isEmpty |
| `util::java_collections::JavaStringSet::to_std` | D? | **T** | none (used by store.rs tests only) | no Lucene member: converts to a std set for assertions |
| `util::jvm::MethodClass::extending` | D? | **P** | none: the port's model of `java.lang.Class` that `VirtualMethod`'s API takes (declared at `MethodClass`); a subclass constructor | VirtualMethod.java:98 takes Class<C>; no Lucene member of its own |
| `util::jvm::MethodClass::get_name` | D? | **P** | none: models `Class.getName()` (as `extending`) | java.lang.Class.getName |
| `util::jvm::MethodClass::get_superclass` | D? | **P** | none: models `Class.getSuperclass()` (as `extending`) | java.lang.Class.getSuperclass |
| `util::jvm::MethodClass::new` | D? | **P** | none: a root-class constructor of the model (as `extending`) | VirtualMethod.java:98 |
| `util::jvm::VirtualMethod::compare_implementation_distance` | P? | **P** | none | VirtualMethod.java:174 |
| `util::jvm::VirtualMethod::is_overridden_as_of` | P? | **P** | none | VirtualMethod.java:132 |
| `util::jvm::VirtualMethod::new` | P? | **P** | none | VirtualMethod.java:98 |
| `util::long_heap::LongHeap` | P? | **P** | none | LongHeap.java:31 |
| `util::long_heap::LongHeap::clear` | P? | **P** | none | LongHeap.java:163 |
| `util::long_heap::LongHeap::filled` | D? | **P** | none (renamed constructor) | LongHeap.java:44 LongHeap(int, long) |
| `util::long_heap::LongHeap::get` | P? | **P** | none | LongHeap.java:208 |
| `util::long_heap::LongHeap::get_heap_array` | D? | **P** | none (faithful member uncalled in Lucene) | LongHeap.java:218 getHeapArray |
| `util::long_heap::LongHeap::insert_with_overflow` | P? | **P** | none | LongHeap.java:93 |
| `util::long_heap::LongHeap::is_empty` | D? | **D** | removed | no Lucene member: LongHeap.java:31-225 declares size() only |
| `util::long_heap::LongHeap::pop` | P? | **P** | none | LongHeap.java:119 |
| `util::long_heap::LongHeap::push_all` | P? | **P** | none | LongHeap.java:198 |
| `util::long_heap::LongHeap::size` | W? | **W=** | none: Java's field `size`, used by the heap's own methods; the Rust methods use the field | LongHeap.java:36,158 |
| `util::long_heap::LongHeap::top` | P? | **P** | none | LongHeap.java:110 |
| `util::long_heap::TernaryLongHeap::get_heap_array` | D? | **P** | none (faithful member uncalled in Lucene) | TernaryLongHeap.java:187 |
| `util::long_heap::TernaryLongHeap::is_empty` | D? | **D** | removed | no Lucene member: TernaryLongHeap.java declares size() only |
| `util::long_heap::TernaryLongHeap::push_all` | P? | **P** | none | TernaryLongHeap.java:167 |
| `util::math_util::MathUtil::acosh` | P? | **P** | none | MathUtil.java:127 |
| `util::math_util::MathUtil::asinh` | P? | **P** | none | MathUtil.java:102 |
| `util::math_util::MathUtil::atanh` | P? | **P** | none | MathUtil.java:144 |
| `util::math_util::MathUtil::gcd` | W? | **W** | wired: `Lucene90DocValuesConsumer`'s value statistics take `MathUtil::gcd` (Java's binary GCD) instead of a private Euclid `gcd_i64` (removed) in lucene90/doc_values.rs. Control LA (4 portability_doc_values tests) | MathUtil.java:63-90; caller Lucene90DocValuesConsumer.java:392 |
| `util::math_util::MathUtil::log_base` | D? | **P** | none (renamed overload) | MathUtil.java:48 log(double, double) |
| `util::misc::MapOfSets` | P? | **P** | none | MapOfSets.java:30 |
| `util::misc::MapOfSets::get_map` | P? | **P** | none | MapOfSets.java:44 |
| `util::misc::MapOfSets::get_map_mut` | D? | **P** | none: the mutable half of Java's `getMap()`, which hands out the live map | MapOfSets.java:44 |
| `util::misc::MapOfSets::new` | P? | **P** | none | MapOfSets.java:37 |
| `util::misc::MapOfSets::put` | P? | **P** | none | MapOfSets.java:54 |
| `util::misc::MapOfSets::put_all` | P? | **P** | none | MapOfSets.java:66 |
| `util::misc::SentinelIntSet` | P? | **P** | none | SentinelIntSet.java:32 |
| `util::misc::SentinelIntSet::clear` | W? | **W** | wired: `SentinelIntSet::new` allocates zeroed keys and calls `clear()` when `emptyVal != 0`, as Java's constructor (it filled the array inline). Test `a_non_zero_empty_value_is_cleared_into_every_slot` | SentinelIntSet.java:68; caller SentinelIntSet.java:64-65 |
| `util::misc::SentinelIntSet::exists` | P? | **P** | none | SentinelIntSet.java:117 |
| `util::misc::SentinelIntSet::is_empty` | D? | **D** | removed | no Lucene member: SentinelIntSet.java declares size() only |
| `util::misc::SentinelIntSet::new` | P? | **P** | none | SentinelIntSet.java:56 |
| `util::misc::SentinelIntSet::put` | P? | **P** | none | SentinelIntSet.java:125 |
| `util::misc::SentinelIntSet::ram_bytes_used` | P? | **P** | none | SentinelIntSet.java:156 |
| `util::misc::SentinelIntSet::size` | P? | **P** | none | SentinelIntSet.java:82 |
| `util::misc::StrictStringTokenizer` | W? | **W** | wired: `Version::parse` splits with it, as `Version.parse` does (it used `str::split`); see finding 8 | StrictStringTokenizer.java; caller Version.java:290 |
| `util::misc::StrictStringTokenizer::has_more_tokens` | W? | **W** | wired: `Version::parse` asks it before each component. Control K21 (a mutated tokenizer fails the four version tests) | StrictStringTokenizer.java hasMoreTokens; callers Version.java:291,308,327,340,366 |
| `util::misc::StrictStringTokenizer::new` | W? | **W** | wired: `Version::parse` builds it over the version on `'.'` | StrictStringTokenizer.java ctor; caller Version.java:290 |
| `util::misc::StrictStringTokenizer::next_token` | W? | **W** | wired: `Version::parse` reads each component with it | StrictStringTokenizer.java nextToken; callers Version.java:297,314,329,341 |
| `util::misc::TermAndVector` | P? | **P** | none | TermAndVector.java:27 |
| `util::misc::TermAndVector::new` | D? | **P** | none (the record's canonical constructor) | TermAndVector.java:27 record TermAndVector(BytesRef term, float[] vector) |
| `util::misc::TermAndVector::normalize_vector` | P? | **P** | none | TermAndVector.java:34 |
| `util::misc::TermAndVector::size` | P? | **P** | none | TermAndVector.java:29 |
| `util::misc::TermAndVector::term` | D? | **P** | none (record component accessor) | TermAndVector.java:27 term() |
| `util::misc::TermAndVector::vector` | D? | **P** | none (record component accessor) | TermAndVector.java:27 vector() |
| `util::misc::ToStringUtils` | W? | **W-blocked** | blocked: its Java callers are `PendingTerm.toString` and `PendingBlock.toString` of the block-tree writer, not ported (blocktree.rs has no Display/toString for them) | ToStringUtils.java:20; callers Lucene103BlockTreeTermsWriter.java:423,454,750 |
| `util::misc::ToStringUtils::byte_array` | P? | **P** | none | ToStringUtils.java:24 |
| `util::misc::ToStringUtils::bytes_ref_builder_to_string` | D? | **P** | none (renamed overload) | ToStringUtils.java:66 bytesRefToString(BytesRefBuilder) |
| `util::misc::ToStringUtils::bytes_to_string` | D? | **P** | none (renamed overload) | ToStringUtils.java:70 bytesRefToString(byte[]) |
| `util::misc::ToStringUtils::long_hex` | P? | **P** | none | ToStringUtils.java:39 |
| `util::mutable::MutableValueBool` | P? | **P** | none | MutableValueBool.java:25 |
| `util::mutable::MutableValueBool::new` | D? | **P** | none (Java's implicit no-argument constructor) | MutableValueBool.java:25 (default constructor) |
| `util::mutable::MutableValueDate::new` | D? | **P** | none (Java's implicit no-argument constructor) | MutableValueDate.java:26 (default constructor) |
| `util::mutable::MutableValueDouble` | P? | **P** | none | MutableValueDouble.java:24 |
| `util::mutable::MutableValueDouble::new` | D? | **P** | none (Java's implicit no-argument constructor) | MutableValueDouble.java:24 (default constructor) |
| `util::mutable::MutableValueFloat` | P? | **P** | none | MutableValueFloat.java:24 |
| `util::mutable::MutableValueFloat::new` | D? | **P** | none (Java's implicit no-argument constructor) | MutableValueFloat.java:24 (default constructor) |
| `util::mutable::MutableValueInt` | P? | **P** | none | MutableValueInt.java:24 |
| `util::mutable::MutableValueInt::new` | D? | **P** | none (Java's implicit no-argument constructor) | MutableValueInt.java:24 (default constructor) |
| `util::mutable::MutableValueLong::new` | D? | **P** | none (Java's implicit no-argument constructor) | MutableValueLong.java:24 (default constructor) |
| `util::mutable::MutableValueStr` | P? | **P** | none | MutableValueStr.java:26 |
| `util::mutable::MutableValueStr::new` | D? | **P** | none (Java's implicit no-argument constructor) | MutableValueStr.java:26 (default constructor) |
| `util::offline_sorter::BufferSize::megabytes` | P? | **P** | none | OfflineSorter.java:101 |
| `util::offline_sorter::OfflineSorter` | P? | **P** | none | OfflineSorter.java:52 |
| `util::offline_sorter::OfflineSorter::get_comparator` | P? | **P** | none | OfflineSorter.java:610 |
| `util::offline_sorter::OfflineSorter::get_directory` | P? | **P** | none | OfflineSorter.java:269 |
| `util::offline_sorter::OfflineSorter::get_temp_file_name_prefix` | P? | **P** | none | OfflineSorter.java:277 |
| `util::offline_sorter::OfflineSorter::new` | W? | **P** | none: Java's `OfflineSorter(Directory, String)`, which chains to the full constructor as the Rust `new` chains to `with_options` | OfflineSorter.java:197 |
| `util::offline_sorter::OfflineSorter::sort` | P? | **P** | none | OfflineSorter.java:282 |
| `util::offline_sorter::OfflineSorter::sort_info` | W? | **W=** | none: Java's field `sortInfo`, used by `sort`/`mergePartitions`/`readPartition` and the tasks; the Rust code uses the field | OfflineSorter.java:185,284,308,354,391 |
| `util::offline_sorter::OfflineSorter::with_comparator` | D? | **P** | none (renamed constructor) | OfflineSorter.java:214 OfflineSorter(Directory, String, Comparator) |
| `util::packed::BlockPackedReaderIterator::next_batch` | D? | **W-blocked** | Java's `next(int count)` (renamed overload); blocked: the Rust term-vectors reader decodes each per-term array whole with a fresh iterator and `next()` (lucene90/term_vectors.rs), where Java resets one iterator and reads only the document's range with `skip`/`next(int)`/`ord`; reproducing that is a rewrite of the reader's `get`/`readPositions` | BlockPackedReaderIterator.java:166; callers Lucene90CompressingTermVectorsReader.java:500,521,542 |
| `util::packed::BlockPackedReaderIterator::ord` | W? | **W-blocked** | blocked: the Rust term-vectors reader decodes each per-term array whole with a fresh iterator and `next()` (lucene90/term_vectors.rs), where Java resets one iterator and reads only the document's range with `skip`/`next(int)`/`ord`; reproducing that is a rewrite of the reader's `get`/`readPositions` | BlockPackedReaderIterator.java:223 ord(); caller Lucene90CompressingTermVectorsReader.java:506 |
| `util::packed::BlockPackedReaderIterator::reset` | W? | **W-blocked** | blocked: the Rust term-vectors reader decodes each per-term array whole with a fresh iterator and `next()` (lucene90/term_vectors.rs), where Java resets one iterator and reads only the document's range with `skip`/`next(int)`/`ord`; reproducing that is a rewrite of the reader's `get`/`readPositions` | BlockPackedReaderIterator.java:84; callers Lucene90CompressingTermVectorsReader.java:394,487,508,540,649,794 |
| `util::packed::BlockPackedReaderIterator::skip` | W? | **W-blocked** | blocked: the Rust term-vectors reader decodes each per-term array whole with a fresh iterator and `next()` (lucene90/term_vectors.rs), where Java resets one iterator and reads only the document's range with `skip`/`next(int)`/`ord`; reproducing that is a rewrite of the reader's `get`/`readPositions` | BlockPackedReaderIterator.java:93; callers Lucene90CompressingTermVectorsReader.java:493,506,809 |
| `util::packed::BlockPackedWriter::ord` | W? | **W=** | none: Java's field `ord` of AbstractBlockPackedWriter, used by `add`/`addBlockOfZeros`; the Rust methods use the field | AbstractBlockPackedWriter.java:46,82,96 |
| `util::packed::DirectMonotonicReader::binary_search` | W? | **W-blocked** | blocked: the Rust `FieldsIndexReader` decodes `docs`/`startPointers` onto the heap when it opens (lucene90/stored_fields.rs `FieldsIndexReader`) and binary-searches the vector inline, where Java keeps `DirectMonotonicReader` views over the .fdx slices; searching through this needs those readers | DirectMonotonicReader.java:157; caller FieldsIndexReader.java:122 |
| `util::packed::DirectWriter::bytes_required` | P? | **P** | none | DirectWriter.java:244 |
| `util::packed::Format::by_id` | P? | **P** | none | PackedInts.java:114 |
| `util::packed::Format::id` | W? | **W=** | none: Java's field `id`, read by `byId`/`getId`; the Rust code reads the field | PackedInts.java:116,130 |
| `util::packed::Format::overhead_ratio` | P? | **P** | none | PackedInts.java:167 |
| `util::packed::PackedInts::byte_count_packed` | D? | **D** | removed (undocumented, no caller) | no Lucene member: PackedInts.java has `Format.byteCount(int, int, int)` only, ported as `Format::byte_count` |
| `util::packed::monotonic_reader_constructions` | D? | **T** | none (instrumentation read by tests/knn_scorers.rs only; declared there as having no Lucene counterpart) | no Lucene member |
| `util::packed::block_packed::AbstractBlockPackedWriter::block_size` | D? | **D** | removed | no Lucene member: AbstractBlockPackedWriter.java keeps `values` protected and reads `values.length` itself |
| `util::packed::block_packed::AbstractBlockPackedWriter::finished` | W? | **W=** | none: Java's field, read by `checkNotFinished()`; the Rust `check_not_finished` reads the field | AbstractBlockPackedWriter.java:47,66 |
| `util::packed::block_packed::AbstractBlockPackedWriter::reset` | W? | **W=** | none: Java's constructor calls `reset(out)`; the Rust `new` initialises the four fields `reset` sets (it cannot call a method on a value it is still building, and `out` cannot be handed over twice) | AbstractBlockPackedWriter.java:56-68 |
| `util::packed::block_packed::MonotonicBlockPackedReader` | W? | **W=** | none: the Java users import its static `expected(long, float, int)`, ported as the module function `block_packed::expected`, which the Rust `MonotonicBlockPackedWriter` and `MonotonicLongValues` call | MonotonicBlockPackedReader.java:35; MonotonicBlockPackedWriter.java:19, MonotonicLongValues.java:19 |
| `util::packed::block_packed::MonotonicBlockPackedReader::of` | P? | **P** | none | MonotonicBlockPackedReader.java:54 |
| `util::packed::block_packed::MonotonicBlockPackedReader::size` | P? | **P** | none | MonotonicBlockPackedReader.java:127 |
| `util::packed::block_packed::MonotonicBlockPackedWriter` | P? | **P** | none | MonotonicBlockPackedWriter.java:53 |
| `util::packed::block_packed::MonotonicBlockPackedWriter::add` | W? | **P** | none: Java's override, which delegates to the base `add` as the Rust one does; no core caller | MonotonicBlockPackedWriter.java:65 |
| `util::packed::block_packed::MonotonicBlockPackedWriter::finish` | D? | **P** | none (inherited public `finish()`, no core caller) | AbstractBlockPackedWriter.java:103 |
| `util::packed::block_packed::MonotonicBlockPackedWriter::new` | P? | **P** | none | MonotonicBlockPackedWriter.java:60 |
| `util::packed::block_packed::MonotonicBlockPackedWriter::ord` | W? | **W=** | none: Java's field `ord`, used by `add`/`addBlockOfZeros`; the Rust code uses the base field | AbstractBlockPackedWriter.java:46,82,96 |
| `util::packed::bulk_operation::BulkOperationPacked1` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked1()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked1)` | BulkOperationPacked1.java:22; caller BulkOperation.java:25 |
| `util::packed::bulk_operation::BulkOperationPacked10` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked10()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked10)` | BulkOperationPacked10.java:22; caller BulkOperation.java:34 |
| `util::packed::bulk_operation::BulkOperationPacked11` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked11()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked11)` | BulkOperationPacked11.java:22; caller BulkOperation.java:35 |
| `util::packed::bulk_operation::BulkOperationPacked12` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked12()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked12)` | BulkOperationPacked12.java:22; caller BulkOperation.java:36 |
| `util::packed::bulk_operation::BulkOperationPacked13` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked13()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked13)` | BulkOperationPacked13.java:22; caller BulkOperation.java:37 |
| `util::packed::bulk_operation::BulkOperationPacked14` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked14()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked14)` | BulkOperationPacked14.java:22; caller BulkOperation.java:38 |
| `util::packed::bulk_operation::BulkOperationPacked15` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked15()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked15)` | BulkOperationPacked15.java:22; caller BulkOperation.java:39 |
| `util::packed::bulk_operation::BulkOperationPacked16` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked16()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked16)` | BulkOperationPacked16.java:22; caller BulkOperation.java:40 |
| `util::packed::bulk_operation::BulkOperationPacked17` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked17()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked17)` | BulkOperationPacked17.java:22; caller BulkOperation.java:41 |
| `util::packed::bulk_operation::BulkOperationPacked18` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked18()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked18)` | BulkOperationPacked18.java:22; caller BulkOperation.java:42 |
| `util::packed::bulk_operation::BulkOperationPacked19` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked19()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked19)` | BulkOperationPacked19.java:22; caller BulkOperation.java:43 |
| `util::packed::bulk_operation::BulkOperationPacked2` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked2()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked2)` | BulkOperationPacked2.java:22; caller BulkOperation.java:26 |
| `util::packed::bulk_operation::BulkOperationPacked20` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked20()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked20)` | BulkOperationPacked20.java:22; caller BulkOperation.java:44 |
| `util::packed::bulk_operation::BulkOperationPacked21` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked21()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked21)` | BulkOperationPacked21.java:22; caller BulkOperation.java:45 |
| `util::packed::bulk_operation::BulkOperationPacked22` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked22()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked22)` | BulkOperationPacked22.java:22; caller BulkOperation.java:46 |
| `util::packed::bulk_operation::BulkOperationPacked23` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked23()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked23)` | BulkOperationPacked23.java:22; caller BulkOperation.java:47 |
| `util::packed::bulk_operation::BulkOperationPacked24` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked24()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked24)` | BulkOperationPacked24.java:22; caller BulkOperation.java:48 |
| `util::packed::bulk_operation::BulkOperationPacked3` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked3()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked3)` | BulkOperationPacked3.java:22; caller BulkOperation.java:27 |
| `util::packed::bulk_operation::BulkOperationPacked4` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked4()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked4)` | BulkOperationPacked4.java:22; caller BulkOperation.java:28 |
| `util::packed::bulk_operation::BulkOperationPacked5` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked5()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked5)` | BulkOperationPacked5.java:22; caller BulkOperation.java:29 |
| `util::packed::bulk_operation::BulkOperationPacked6` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked6()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked6)` | BulkOperationPacked6.java:22; caller BulkOperation.java:30 |
| `util::packed::bulk_operation::BulkOperationPacked7` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked7()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked7)` | BulkOperationPacked7.java:22; caller BulkOperation.java:31 |
| `util::packed::bulk_operation::BulkOperationPacked8` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked8()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked8)` | BulkOperationPacked8.java:22; caller BulkOperation.java:32 |
| `util::packed::bulk_operation::BulkOperationPacked9` | W? | **W=** | none: `BulkOperation.packedBulkOps` holds `new BulkOperationPacked9()`; the Rust `PACKED_BULK_OPS` is filled by the macro-generated `push_specialised_packed_operations`, which pushes `Box::new(BulkOperationPacked9)` | BulkOperationPacked9.java:22; caller BulkOperation.java:33 |
| `util::packed::bulk_operation::BulkOperationPacked::bits_per_value` | W? | **W=** | none: Java's field, read by `decode`/`encode`; the Rust code reads the field | BulkOperationPacked.java:22 |
| `util::packed::bulk_operation::BulkOperationPackedSingleBlock::bits_per_value` | W? | **W=** | none: Java's field, read by `decode`/`encode`; the Rust code reads the field | BulkOperationPackedSingleBlock.java:24 |
| `util::packed::bulk_operation::get_decoder` | D? | **P** | none (Java's static, ported as a module function) | PackedInts.java:644 getDecoder(Format, int, int) |
| `util::packed::direct_packed_reader::DirectPacked64SingleBlockReader` | D? | **P** | none (faithful member uncalled in Lucene) | DirectPacked64SingleBlockReader.java:22; no caller in lucene/ |
| `util::packed::direct_packed_reader::DirectPacked64SingleBlockReader::new` | D? | **P** | none (as the struct) | DirectPacked64SingleBlockReader.java:30 |
| `util::packed::direct_packed_reader::DirectPackedReader` | D? | **P** | none (faithful member uncalled in Lucene; `DirectReader` uses its own `DirectPackedReaderN` classes) | DirectPackedReader.java:24; no caller in lucene/ |
| `util::packed::direct_packed_reader::DirectPackedReader::new` | D? | **P** | none (as the struct) | DirectPackedReader.java:30 |
| `util::packed::growable_writer::GrowableWriter::mutable` | P? | **P** | none | GrowableWriter.java:66 getMutable |
| `util::packed::growable_writer::GrowableWriter::resize` | P? | **P** | none | GrowableWriter.java:95 |
| `util::packed::packed64::Packed64::blocks` | W? | **W=** | none: Java's private field, used by Packed64's own methods; the Rust methods use the field | Packed64.java:42 |
| `util::packed::packed_data_io::PackedDataInput` | P? | **P** | none | PackedDataInput.java:29 |
| `util::packed::packed_data_io::PackedDataInput::new` | P? | **P** | none | PackedDataInput.java:36 |
| `util::packed::packed_data_io::PackedDataInput::read_long` | P? | **P** | none | PackedDataInput.java:42 |
| `util::packed::packed_data_io::PackedDataOutput` | P? | **P** | none | PackedDataOutput.java:28 |
| `util::packed::packed_data_io::PackedDataOutput::flush` | P? | **P** | none | PackedDataOutput.java:61 |
| `util::packed::packed_data_io::PackedDataOutput::new` | P? | **P** | none | PackedDataOutput.java:35 |
| `util::packed::packed_data_io::PackedDataOutput::write_long` | P? | **P** | none | PackedDataOutput.java:42 |
| `util::packed::packed_long_values::DeltaPackedLongValues::get` | W? | **W=** | none: Java's `PackedLongValues.get` dispatches to the subclass; the Rust `LongValues`/`PackedLongValuesAccess` impls reach this | PackedLongValues.java:108-118; DeltaPackedLongValues.java:38 |
| `util::packed::packed_long_values::DeltaPackedLongValues::iterator` | W? | **W=** | none: the Java callers' `iterator()` is ported as `OwnedPackedLongValuesIterator::new(Arc<..>)`, the owning form the buffered doc-values and norms views hold (doc_values_writer.rs, norms_writer.rs); `OrdinalMap` keeps its deltas in `Vec<i64>` (doc_values.rs `OrdinalMap::new`), not in `PackedLongValues` | PackedLongValues.java:131 iterator(); callers NumericDocValuesWriter.java:107, BinaryDocValuesWriter.java:124, NormValuesWriter.java:79, SortedNumericDocValuesWriter.java:195, SortedSetDocValuesWriter.java:258 |
| `util::packed::packed_long_values::DeltaPackedLongValues::size` | W? | **W=** | none: the iterators read the size through `PackedLongValuesAccess::size` (as Java's iterator reads the field); see `iterator` for `OrdinalMap` | PackedLongValues.java:87; callers OrdinalMap.java:363-365 |
| `util::packed::packed_long_values::MonotonicLongValues::iterator` | W? | **P** | none: no Rust or Java core caller iterates a monotonic instance (the listed callers iterate delta and plain ones) | PackedLongValues.java:131 |
| `util::packed::packed_long_values::MonotonicLongValues::size` | W? | **P** | none (as `iterator`) | PackedLongValues.java:87 |
| `util::packed::packed_long_values::OwnedPackedLongValuesIterator::has_next` | W? | **W=** | none: the cursor's `has_next`, which this returns, guards the Rust `next_value` as Java's `next()` asserts `hasNext()`; the `OrdinalMap` caller iterates a `Vec<i64>` (see `DeltaPackedLongValues::iterator`) | PackedLongValues.java:158,164; caller OrdinalMap.java:374 |
| `util::packed::packed_long_values::PackedLongValues::iterator` | W? | **W=** | none: the Java callers' `iterator()` is ported as `OwnedPackedLongValuesIterator::new(Arc<..>)`, the owning form the buffered doc-values and norms views hold (doc_values_writer.rs, norms_writer.rs); `OrdinalMap` keeps its deltas in `Vec<i64>` (doc_values.rs `OrdinalMap::new`), not in `PackedLongValues` | PackedLongValues.java:131; callers SortedDocValuesWriter.java:233,275 |
| `util::packed::packed_long_values::PackedLongValues::size` | W? | **W=** | none (as `DeltaPackedLongValues::size`) | PackedLongValues.java:87 |
| `util::packed::packed_writer::PackedInts::get_reader_iterator_no_header` | D? | **W** | wired: the Lucene90 term-vectors reader reads the chunk's distinct field numbers through it (`Format::Packed`, `packedIntsVersion`, mem 1) instead of the crate helper `read_packed_ints_no_header` (removed). Control L42b | PackedInts.java:678; caller Lucene90CompressingTermVectorsReader.java:425-435 |
| `util::packed::packed_writer::PackedInts::get_writer_no_header` | D? | **W** | wired: the Lucene90 term-vectors writer writes the distinct field numbers through it (`add` per field, `finish`) instead of the crate helper `write_packed_ints_no_header` (removed). Control L42a | PackedInts.java:765; caller Lucene90CompressingTermVectorsWriter.java:473-479 |
| `util::packed::paged_mutable::PagedGrowableWriter::acceptable_overhead_ratio` | W? | **W=** | none: Java's field, read by `newMutable`/`newUnfilledCopy`; the Rust methods read the field | PagedGrowableWriter.java:32,54,68 |
| `util::packed::paged_mutable::PagedGrowableWriter::page_size` | W? | **W=** | none: the inherited `AbstractPagedMutable.pageSize()`, which the shared base calls; this exposes it on the subclass | AbstractPagedMutable.java:54,57,66 |
| `util::packed::paged_mutable::PagedMutable::format` | W? | **W=** | none: Java's field, read by `newMutable`/`newUnfilledCopy`; the Rust methods read the field | PagedMutable.java:31,66 |
| `util::packed::paged_mutable::PagedMutable::page_size` | W? | **W=** | none (as `PagedGrowableWriter::page_size`) | AbstractPagedMutable.java:54,57,66 |
| `util::paged_bytes::PagedBytes::block_size` | W? | **W=** | none: Java's field `blockSize`, used by PagedBytes' own methods; the Rust methods use the field | PagedBytes.java:38 |
| `util::paged_bytes::PagedBytes::copy` | P? | **P** | none | PagedBytes.java:198 copy(BytesRef, BytesRef) |
| `util::paged_bytes::PagedBytes::copy_from_input` | D? | **P** | none (renamed overload) | PagedBytes.java:171 copy(IndexInput, long) |
| `util::paged_bytes::PagedBytes::copy_using_length_prefix` | P? | **P** | none | PagedBytes.java:263 |
| `util::paged_bytes::PagedBytes::get_data_input` | W? | **W=** | none: `BinaryDocValuesWriter` reads through `get_shared_data_input`, the owning form of Java's `getDataInput()` its buffered values need (declared at paged_bytes.rs `get_shared_data_input`; doc_values_writer.rs `freeze_bytes`) | PagedBytes.java:431; callers BinaryDocValuesWriter.java:124,141,155 |
| `util::paged_bytes::PagedBytesDataOutput::get_position` | P? | **P** | none | PagedBytes.java:425 |
| `util::paged_bytes::PagedBytesReader::block_size` | W? | **W=** | none: Java's field, read by `fillSlice`/`toString`; the Rust methods read the field | PagedBytes.java:60 |
| `util::paged_bytes::PagedBytesReader::fill` | P? | **P** | none | PagedBytes.java:122 |
| `util::paged_bytes::PagedBytesReader::fill_slice` | W? | **W-blocked** | blocked: declared divergence, `PointValuesWriter` buffers the packed values in a flat `Vec<u8>` (point_values_writer.rs module docs), not a `PagedBytes` | PagedBytes.java:80 fillSlice; caller PointValuesWriter.java:249 |
| `util::paged_bytes::PagedBytesReader::get_byte` | W? | **W-blocked** | blocked: as `fill_slice` | PagedBytes.java:108; caller PointValuesWriter.java:255 |
| `util::quantization::optimized_scalar_quantizer::OptimizedScalarQuantizer::multi_scalar_quantize` | P? | **P** | none | OptimizedScalarQuantizer.java:119 |
| `util::quantization::optimized_scalar_quantizer::OptimizedScalarQuantizer::similarity_function` | W? | **W=** | none: Java's field, read by `scalarQuantize`/`multiScalarQuantize`; the Rust methods read the field | OptimizedScalarQuantizer.java:60 |
| `util::quantization::optimized_scalar_quantizer::OptimizedScalarQuantizer::with_params` | D? | **P** | none (renamed constructor) | OptimizedScalarQuantizer.java:80 OptimizedScalarQuantizer(VectorSimilarityFunction, float, int) |
| `util::quantization::quantized_byte_vector_values::BaseQuantizedByteVectorValues::get_slice` | W? | **W=** | none: the Lucene99 scorer reads `getSlice()` through `LegacyQuantizedByteVectorValues::get_slice`/`get_slice_mut` (lucene99/scalar_quantized_scorer.rs), whose default answers `None` as this base override does | BaseQuantizedByteVectorValues.java:44; callers Lucene99ScalarQuantizedVectorScorer.java:127,241-242 |
| `util::quantization::quantized_byte_vector_values::ScalarEncoding::from_num_bits` | P? | **P** | none | QuantizedByteVectorValues.java:87 |
| `util::quantization::quantized_byte_vector_values::ScalarEncoding::get_doc_length` | D? | **D** | removed (a usize copy of `get_doc_packed_length`, no caller) | no further Lucene member: QuantizedByteVectorValues.java:161 getDocPackedLength, ported as `get_doc_packed_length` |
| `util::quantization::scalar_quantized_vector_similarity::dyn::from_vector_similarity` | W? | **W=** | none: called as `<dyn ScalarQuantizedVectorSimilarity>::from_vector_similarity` at all three Java sites (hnsw/scalar_quantized_scorer.rs `get_random_vector_scorer` and the supplier, scalar_quantizer.rs `score_error_correlation`) | ScalarQuantizedVectorSimilarity.java:39; callers ScalarQuantizedVectorScorer.java:86,133, ScalarQuantizer.java:654 |
| `util::quantization::scalar_quantizer::ScalarQuantizer::de_quantize` | P? | **P** | none | ScalarQuantizer.java:159 |
| `util::quantization::scalar_quantizer::ScalarQuantizer::from_sample` | D? | **D** | removed (no caller) | no Lucene member: ScalarQuantizer.java computes quantiles only inside fromVectors/fromVectorsAutoInterval |
| `util::quantization::scalar_quantizer::ScalarQuantizer::from_vectors` | W? | **P** | none: Java's public 4-argument overload, which chains to the 5-argument one as the Rust one chains to `from_vectors_with_sample_size` | ScalarQuantizer.java:223-236 |
| `util::quantization::scalar_quantizer::ScalarQuantizer::from_vectors_auto_interval` | P? | **P** | none | ScalarQuantizer.java:309 |
| `util::quantization::scalar_quantizer::ScalarQuantizer::get_lower_quantile` | P? | **P** | none | ScalarQuantizer.java:166 |
| `util::quantization::scalar_quantizer::ScalarQuantizer::get_upper_quantile` | P? | **P** | none | ScalarQuantizer.java:170 |
| `util::quantization::scalar_quantizer::ScalarQuantizer::recalculate_corrective_offset` | P? | **P** | none | ScalarQuantizer.java:136 |
| `util::quantization::scalar_quantizer::is_unit_vector` | D? | **D** | removed (a wrapper of `vector_util::is_unit_vector`, no caller) | no Lucene member: ScalarQuantizer.java has no isUnitVector; VectorUtil.isUnitVector is `vector_util::is_unit_vector` |
| `util::query_builder::QueryBuilder::new` | P? | **P** | none | QueryBuilder.java:79 |
| `util::query_builder::QueryBuilder::set_analyzer` | P? | **P** | none | QueryBuilder.java:185 |
| `util::query_builder::QueryBuilder::set_auto_generate_multi_term_synonyms_phrase_query` | P? | **P** | none | QueryBuilder.java:224 |
| `util::query_builder::QueryBuilder::set_enable_graph_queries` | P? | **P** | none | QueryBuilder.java:267 |
| `util::query_builder::QueryBuilder::set_enable_position_increments` | P? | **P** | none | QueryBuilder.java:207 |
| `util::refs::CharsRefBuilder` | P? | **P** | none | CharsRefBuilder.java:24 |
| `util::refs::CharsRefBuilder::append` | W? | **P** | none: Java's `append(char)`; its listed caller is the overload chain inside CharsRefBuilder | CharsRefBuilder.java:86 |
| `util::refs::CharsRefBuilder::append_str` | D? | **P** | none (renamed overload) | CharsRefBuilder.java:66 append(CharSequence) |
| `util::refs::CharsRefBuilder::char_at` | W? | **P** | none | CharsRefBuilder.java:51 |
| `util::refs::CharsRefBuilder::chars` | P? | **P** | none | CharsRefBuilder.java:36 |
| `util::refs::CharsRefBuilder::clear` | P? | **P** | none | CharsRefBuilder.java:61 |
| `util::refs::CharsRefBuilder::copy_chars_ref` | D? | **P** | none (renamed overload) | CharsRefBuilder.java:93 copyChars(CharsRef) |
| `util::refs::CharsRefBuilder::copy_utf8_bytes_ref` | D? | **P** | none (renamed overload) | CharsRefBuilder.java:109 copyUTF8Bytes(BytesRef) |
| `util::refs::CharsRefBuilder::length` | P? | **P** | none | CharsRefBuilder.java:41 |
| `util::refs::CharsRefBuilder::new` | P? | **P** | none | CharsRefBuilder.java:31 |
| `util::refs::CharsRefBuilder::set_length` | P? | **P** | none | CharsRefBuilder.java:46 |
| `util::refs::CharsRefBuilder::to_chars_ref` | P? | **P** | none | CharsRefBuilder.java:138 |
| `util::refs::IntsRefBuilder::clear` | P? | **P** | none | IntsRefBuilder.java:49 |
| `util::refs::IntsRefBuilder::copy_utf8_bytes` | P? | **P** | none | IntsRefBuilder.java:101 |
| `util::refs::IntsRefBuilder::ints` | P? | **P** | none | IntsRefBuilder.java:34 |
| `util::refs::IntsRefBuilder::ints_mut` | D? | **P** | none: the mutable half of Java's `ints()`, which hands out the live array | IntsRefBuilder.java:34 |
| `util::refs::IntsRefBuilder::to_ints_ref` | P? | **P** | none | IntsRefBuilder.java:116 |
| `util::refs::LongsRef` | W? | **W=** | none: the Java callers use `LongsRef` as an (array, offset, length) view of decoded values; the Rust ports hand out `&[i64]` slices, the same view (e.g. `BlockPackedReaderIterator::next_batch`, `PackedReaderIterator::next_batch`) | LongsRef.java:27; callers BlockPackedReaderIterator.java:59,166, PackedReaderIterator.java:31, Lucene90CompressingStoredFieldsReader.java:422, FreqProxTermsWriter.java:326 |
| `util::refs::LongsRef::deep_copy_of` | P? | **P** | none | LongsRef.java:136 |
| `util::refs::LongsRef::from_longs` | D? | **P** | none (renamed constructor) | LongsRef.java:54 LongsRef(long[], int, int) |
| `util::refs::LongsRef::hash_code` | P? | **P** | none | LongsRef.java:73 |
| `util::refs::LongsRef::new` | W? | **W=** | none: as `LongsRef` (the readers build their views as slices) | LongsRef.java:41; callers BlockPackedReaderIterator.java:59, PackedReaderIterator.java:31, DocIdsWriter.java:43 |
| `util::refs::LongsRef::with_capacity` | D? | **P** | none (renamed constructor) | LongsRef.java:49 LongsRef(int) |
| `util::resource_loader::ClassLoader::get_parent` | D? | **P** | none: the port's model of `java.lang.ClassLoader` that `ClassLoaderUtils.isParentClassLoader` walks (declared in the module docs); models `getParent()` | ClassLoaderUtils.java:35-45; java.lang.ClassLoader.getParent |
| `util::resource_loader::ClassLoader::name` | D? | **P** | none: as `get_parent` (models `getName()`) | java.lang.ClassLoader.getName |
| `util::resource_loader::ClassLoader::new` | D? | **P** | none: as `get_parent` (a root loader of the model) | ClassLoaderUtils.java:35 |
| `util::resource_loader::ClassLoader::with_parent` | D? | **P** | none: as `get_parent` (a child loader of the model) | ClassLoaderUtils.java:35 |
| `util::resource_loader::ClassLoaderUtils` | W? | **W-blocked** | blocked: declared divergence, the Rust SPI loaders take their providers explicitly instead of a `ClassLoader` (spi.rs module docs), so there is no loader for the constructors' parent check | ClassLoaderUtils.java:35; callers NamedSPILoader.java:49, AnalysisSPILoader.java:59 |
| `util::resource_loader::ClassLoaderUtils::is_parent_class_loader` | W? | **W-blocked** | blocked: as `ClassLoaderUtils` | ClassLoaderUtils.java:35; callers NamedSPILoader.java:49, AnalysisSPILoader.java:59 |
| `util::resource_loader::ClassRegistry` | D? | **P** | none: the declared stand-in for the `findClass`/`newInstance` half of `ResourceLoader` (module docs: no runtime class loading, and a generic trait method would forbid `dyn ResourceLoader`) | ResourceLoader.java:29-40 |
| `util::resource_loader::ClassRegistry::find_class` | D? | **P** | none: as `ClassRegistry` (Java's `findClass`) | ResourceLoader.java:29 |
| `util::resource_loader::ClassRegistry::new` | D? | **P** | none: as `ClassRegistry` | ResourceLoader.java:29-40 |
| `util::resource_loader::ClassRegistry::new_instance` | D? | **P** | none: as `ClassRegistry` (Java's `newInstance`) | ResourceLoader.java:33 |
| `util::resource_loader::ClassRegistry::register` | D? | **P** | none: as `ClassRegistry` (fills the table that stands in for the class path) | ResourceLoader.java:29-40 |
| `util::resource_loader::ClasspathResourceLoader` | P? | **P** | none | ClasspathResourceLoader.java:33 |
| `util::resource_loader::ClasspathResourceLoader::add_resource` | D? | **D** | removed (unused mutator of the embedded table) | no Lucene member: ClasspathResourceLoader.java:33-90 has constructors, openResource and findClass only |
| `util::resource_loader::ClasspathResourceLoader::new` | W? | **P** | none: Java's `ClasspathResourceLoader(ClassLoader)`, whose listed caller is the constructor chain | ClasspathResourceLoader.java:41 |
| `util::resource_loader::ClasspathResourceLoader::with_prefix` | D? | **P** | none (renamed constructor) | ClasspathResourceLoader.java:49 ClasspathResourceLoader(Class) |
| `util::resource_loader::ModuleResourceLoader` | P? | **P** | none | ModuleResourceLoader.java:29 |
| `util::resource_loader::ModuleResourceLoader::new` | P? | **P** | none | ModuleResourceLoader.java:34 |
| `util::resource_loader::ModuleResourceLoader::root` | D? | **D** | removed (unused accessor) | no Lucene member: ModuleResourceLoader.java:31 keeps `module` private |
| `util::resource_loader::ResourceLoaderAware` | W? | **P** | none: no core class implements it; `AbstractAnalysisFactory` names it in its documentation only (the implementing factories live in analysis-common) | ResourceLoaderAware.java:26; AbstractAnalysisFactory.java:38,50 |
| `util::ring_buffer::FrequencyTrackingRingBuffer::as_frequency_map` | D? | **P** | none (port of the package-private test accessor) | FrequencyTrackingRingBuffer.java:91 asFrequencyMap |
| `util::ring_buffer::RollingBuffer::get_buffer_size` | P? | **P** | none | RollingBuffer.java:126 |
| `util::ring_buffer::RollingBuffer::reset` | P? | **P** | none | RollingBuffer.java:52 |
| `util::selector::BytesRefSelector` | D? | **T** | none (used by selector.rs tests only; its doc says it exists for them) | no Lucene member: Selector.java, RadixSelector.java and IntroSelector.java are abstract, with no byte-string selector |
| `util::selector::BytesRefSelector::max_length` | D? | **T** | none (as `BytesRefSelector`) | no Lucene member |
| `util::selector::BytesRefSelector::new` | D? | **T** | none (as `BytesRefSelector`) | no Lucene member |
| `util::small_float::SmallFloat::byte315_to_float` | P? | **P** | none | SmallFloat.java:93 |
| `util::small_float::SmallFloat::byte_to_float` | P? | **P** | none | SmallFloat.java:59 |
| `util::small_float::SmallFloat::float_to_byte` | P? | **P** | none | SmallFloat.java:41 |
| `util::small_float::SmallFloat::float_to_byte315` | P? | **P** | none | SmallFloat.java:80 |
| `util::sorter::ArrayInPlaceMergeSorter` | D? | **P** | none (faithful member uncalled in Lucene) | ArrayInPlaceMergeSorter.java:26; no caller in lucene/core |
| `util::sorter::ArrayInPlaceMergeSorter::new` | D? | **P** | none (as the struct) | ArrayInPlaceMergeSorter.java:32 |
| `util::sorter::StableMSBRadixSorter::max_length` | W? | **W=** | none: Java's field `maxLength`, read by MSBRadixSorter's own methods and the stable fallback; the Rust sorter reads the inner field | MSBRadixSorter.java:46; StableMSBRadixSorter.java:59 |
| `util::sorter::StringSorter::compare` | W? | **P** | none: Java's `compare(int, int)` override, reached only through the inherited `Sorter` algorithms, which StringSorter never runs on itself (its radix and fallback sorters carry their own `compare`) | StringSorter.java:45; Sorter.java:52 |
| `util::sorter::stable_fallback_sort` | D? | **W** | wired: `MutablePointTreeReaderUtils`'s `StableMSBRadixSorter` (`SortOps`) falls back to it, Java's stable `MergeSorter`, instead of the default unstable `IntroSorter`; without it equal values in a short range lost their doc order. Test `mutable_point_tree_sort_keeps_equal_values_in_doc_order` | StableMSBRadixSorter.java:40-72 getFallbackSorter; caller MSBRadixSorter.java (sort -> getFallbackSorter) through MutablePointTreeReaderUtils.java:51-83 |
| `util::spi::NamedSPILoader::iter` | D? | **P** | none (renamed: `Iterable.iterator`) | NamedSPILoader.java:121 |
| `util::string_helper::IntsRef::deep_copy_of` | W? | **W=** | none (`IntsRefBuilder::to_ints_ref` returns `get()`, which already copies the ints, refs.rs:498-515) | IntsRef.java deepCopyOf; caller IntsRefBuilder.java:116-118 |
| `util::string_helper::IntsRef::hash_code` | P? | **P** | none | IntsRef.java hashCode |
| `util::string_helper::IntsRef::is_valid` | W? | **W-blocked** | blocked: its caller is the asserting constructor `IntsRef(int[], int, int)` (IntsRef.java:54-59), which has no Rust counterpart (Rust builds `IntsRef` with `new` or a struct literal) | IntsRef.java isValid; caller IntsRef.java:58 |
| `util::string_helper::StringHelper::murmurhash3_x64_128_bytes_ref` | D? | **P** | none (renamed overload) | StringHelper.java:359 murmurhash3_x64_128(BytesRef) |
| `util::string_helper::read_string` | D? | **D** | removed (finding 1), with its private `read_v_int`/`read_byte` and its two self-tests | no Lucene member: StringHelper.java has no string codec; Java's string I/O is `DataInput.readString`/`DataOutput.writeString`, ported on the store traits (store.rs:268, 477) |
| `util::string_helper::write_string` | D? | **D** | removed, with its private `write_v_int` | no Lucene member (as `read_string`) |
| `util::unicode_util::UnicodeUtil::append_code_point_utf16` | D? | **D** | removed (its doc declared it not a Lucene method; no caller) | no Lucene member: UnicodeUtil.newString encodes surrogates inline |
| `util::unicode_util::UnicodeUtil::calc_utf16_to_utf8_length` | D? | **P** | none | UnicodeUtil.java:243 |
| `util::unicode_util::UnicodeUtil::max_utf8_length` | W? | **W** | wired: `BytesRefBuilder::copy_chars` grows to `maxUTF8Length` of the text's UTF-16 length first, as Java's `copyChars` (it grew to the UTF-8 length). Test `bytes_ref_builder_copy_chars_grows_to_the_max_utf8_length`. Java's other caller, `BytesRef(CharSequence)`, has no port | UnicodeUtil.java:675; callers BytesRefBuilder.java:138,147, BytesRef.java:80 |
| `util::unicode_util::UnicodeUtil::to_hex_string` | P? | **P** | none | UnicodeUtil.java:601 |
| `util::unicode_util::UnicodeUtil::utf16_to_utf8_str` | D? | **P** | none (renamed overload) | UnicodeUtil.java:195 UTF16toUTF8(CharSequence, int, int, byte[], int) |
| `util::unicode_util::UnicodeUtil::utf8_to_utf16_ref` | D? | **P** | none (renamed overload) | UnicodeUtil.java:684 UTF8toUTF16(BytesRef, char[]) |
| `util::unicode_util::UnicodeUtil::valid_utf16_string` | P? | **P** | none | UnicodeUtil.java:353 validUTF16String(char[], int) |
| `util::unicode_util::big_term` | P? | **P** | none (the constant as a function) | UnicodeUtil.java:102 BIG_TERM |
| `util::vector_util::add` | D? | **P** | none | VectorUtil.java:212 |
| `util::vector_util::filter_by_score` | D? | **P** | none | VectorUtil.java:541 |
| `util::vector_util::find_next_geq` | D? | **P** | none | VectorUtil.java:482 |
| `util::vector_util::int4_dot_product_both_packed` | D? | **P** | none | VectorUtil.java:291 |
| `util::vector_util::int4_square_distance` | D? | **P** | none | VectorUtil.java:124 |
| `util::vector_util::int4_square_distance_both_packed` | D? | **P** | none | VectorUtil.java:147 |
| `util::vector_util::int4_square_distance_single_packed` | D? | **P** | none | VectorUtil.java:135 |
| `util::vector_util::xor_bit_count` | D? | **P** | none | VectorUtil.java:348 |

## Removed as a consequence (not in the input list)

- `src/search/bit_set_util.rs` (whole module, `pub mod bit_set_util` dropped from `search.rs`): private copies of `FixedBitSet.nextSetBit/forEach/cardinality(int,int)/intoArray/and/clear(int,int)/scanIsEmpty` and of the default `Bits.applyMask`, documented as waiting for `util` to expose them. Its two listed `search` items (`clear_range`, `collect_set_bits`, block of `search`) go with it. Every caller now calls the `FixedBitSet` method or `Bits::apply_mask`.
- `ArrayUtil::compare_unsigned_generic` (compared to the slice ends; only `get_unsigned_comparator` used it).
- `BKDUtil::subtract_unsigned_bytes` (private copy of `NumericUtils.subtract`, message `subtract_unsigned_bytes: a < b`).
- `point_range_query.rs` `flip_all`, `spatial_query.rs` `and_not` (private copies of `FixedBitSet.flip(int,int)` / `andNot(FixedBitSet)`).
- `tessellator.rs` `interleave` (private copy of `BitUtil.interleave`).
- `postings_util.rs` `read_group_vints`/`read_int_in_group`/`write_group_vints`/`write_int_in_group`/`num_bytes`, `hnsw_vectors_format.rs` `read_group_v_ints`/`read_int_in_group` (private copies of `GroupVIntUtil`).
- `Accountables::human_readable_units` + `format_one_decimal` (private copy of `RamUsageEstimator.humanReadableUnits`, rounding half away from zero).
- `doc_id_set_iterator.rs` `BitSetDocIdSetIterator` (dead private struct under `#[allow(dead_code)]`) and `next_set_bit` (its helper, last used by the inlined `applyMask` replaced by `Bits::apply_mask`).
- `GroupVIntUtil`'s private slice helpers (`read_group_vint`, `read_int_in_group`, `read_byte`, `read_le_short`, `read_le_int`, `read_vint`, `write_vint`) with the two slice-based D items.
- The bit-by-bit `clear_range`/`prev_set_bit`/`next_clear_bit_in_range` bodies of `impl BitSet for SparseFixedBitSet`, and the bit-by-bit `clear_range` and second `approximate_cardinality` body of `impl BitSet for FixedBitSet` (all now delegate to the inherent ports).

## Signature changes on the way

- `ByteArrayComparator` is `Box<dyn Fn(&[u8], usize, &[u8], usize) -> i32 + Send + Sync>` (was a `fn` pointer, which could not capture Java's `numBytes`).
- `ArrayUtil::parse_int(chars, offset, len)` (Java's three-argument form).
- `NumericUtils::subtract`/`add` return `Result` (Java's `IllegalArgumentException` instead of a panic).
- `FixedBitSet::for_each` is generic over the consumer's error type (Java's `CheckedIntConsumer<E>`).
- `Bits` gained `apply_mask` (Java's default method); `FixedBits::apply_mask` returns `Result`.
- `default_info_stream()` returns `Arc<dyn InfoStream>`, `set_default_info_stream(Arc<dyn InfoStream>)`; the codec test helpers that used it as "a no-output stream" pass `&NoOutputInfoStream` (src/codecs/*, src/index/segment_info.rs, tests/fuzz/{points,doc_values,norms}.rs, tests/portability/scalar_quantized_vectors.rs).
- `GroupVIntUtil::read_group_vints_baseline` reads a `DataInput`.

## Positive controls (pass B)

Driver `ctl.sh`/`ctl2.sh` (mutations in `mut.py`/`mut2.py`, restore by `trap ... EXIT`, `md5sum -c` after each batch and once after each run: all `OK`, `ALL_RESTORED`).

| batch | mutation | failing tests |
|---|---|---|
| K1 | comparator generic width to slice ends; `parse_int` whole slice; `subtract` panics; half-up rounding; `use_or_suppress` keeps second; fsync opens read-only; ghost-bit assert removed; writer config takes `NoOutputInfoStream` | the 10 matching `util::tests` + `util::accountables::tests::to_string_renders_sizes_with_human_readable_units` + `util::bkd::tests::bkd_util_helpers` (11 failed) |
| K2 | `set_default_info_stream` no-op; `FSDirectory` fsync back to read-only `sync_data`; `FixedBitSet::approximate_cardinality` returns the exact count; `for_each` no-op | `a_new_writer_config_reads_the_default_info_stream`, `fs_directory_sync_opens_each_file_for_writing`, `approximate_cardinality_is_the_fixed_bit_set_sample`, `for_each_up_to_visits_the_set_bits_rebased` |
| K3 | `cardinality_range`/`into_array` return 0; `clear_range` no-op; `ArrayUtil::select` no-op; CheckIndex/live-docs batches back to bit-by-bit; group-varint decode shifted | `count_up_to...`, `into_array_up_to...`, `clear_range_is_the_fixed_bit_set_clear`, `contains_agrees_with_the_union_of_the_components`, `bits_cardinality_counts_through_apply_mask`, `write_bits_copies_through_apply_mask` (+ postings round trips) |
| K4 | `FixedBitSet::apply_mask` override no-op | 11 lib tests (live-docs round trips, writer delete tests) |
| K6b | `NumericUtils::subtract` writes zeros | `a_deep_multi_dimension_tree_matches_lucene`, `a_partitioned_multi_dimension_tree_matches_lucene` (`_0.kdd differs at byte 750`), `multi_leaf_two_dimensional_bkd_cursor_trace_matches_java`, `bkd_writer_spills_and_writes_what_lucene_writes` |
| K7 | `FixedBitSet::and_not` (and `and`) no-op | `the_dense_relations_drop_the_excluded_documents_as_lucene_does` |
| K9 | `BitUtil::interleave` bit-reversed; `flip_range` no-op | `a_node_carries_lucenes_morton_code`; `bulk_scorers_over_deleted_documents_count_as_lucene_does` (range count 80, not 3196) |
| K10 | `FixedBitSet::and` no-op | `dense_windows_mask_live_docs_and_intersect_clauses` |
| K11 | `scan_is_empty` returns false | `windows_are_masked_with_the_live_docs_apply_mask`, `dense_windows_mask_live_docs_and_intersect_clauses` (the assertions fire) |
| K12/K15 | `Bits::apply_mask` default (and override) no-op | `bulk_scorers_over_deleted_documents...` (`both` = 683, not 546), `windows_are_masked_with_the_live_docs_apply_mask` |
| K13 | `for_each` no-op | `bulk_scorers_over_deleted_documents...` (top hits empty) |
| K20 | `mask` without modulo; `grow` asserts; `size_from_balanced_tree` casts; `LUCENE_XY` rewrite dropped; static-implementation factory always delegates | sparse range tests (overflow panic), `merge_visitor_grow...`, `size_from_balanced_tree...`, `parse_leniently...`, 26+ `analysis::` tests |
| K21 | `StrictStringTokenizer::next_token` appends a space; `SparseFixedBitSet::prev_set_bit` returns -1; default `apply_mask` no-op | the 4 `util::extra` version tests, the sparse view test (prev), `create_bit_set_applies_the_live_docs_mask` |
| K22 | `SparseFixedBitSet::next_clear_bit` None; `or_dense` no-op | sparse view test (nextClear), `or_unions_dense_iterators_through_or_dense` |
| K23 | `next_clear_bit_up_to` None; `HeapPointWriter::compute_cardinality` returns 1 | sparse view test (nextClear range), `bkd_writer_spills_and_writes_what_lucene_writes` |
| K24a-e | `String.trim` back to Unicode `trim`; `load_fs_directory_class` back to "not found"; `load_directory_class` FS-only; interface spelling dropped; lock errno keeps ` (os error N)` | `the_load_methods_throw_what_class_for_name_throws` (each at its own assertion), `try_lock_failure_answers_as_the_jdk_try_lock` |

Controls that did not fail (reported, not hidden): K5 (`flip_range` against `delete_by_query` targets: those queries never take the inverse branch; K9 proves it with a test that does), K8/K14 (`scan_is_empty`, `clear_range` against the end-to-end search test: that test does not reach those sites; K11 proves `scan_is_empty` with direct scorer tests). **No failing control for the `MaxScoreBulkScorer` `clear(0, innerWindowSize)` site**: no test observes a stale window bit there. **No control for the `Constants::linux`/`windows` wiring in `IOUtils::fsync`**: the Windows branch and the Linux assertion (directory fsync failure) cannot be reached on the Linux test host.

## Java oracles (pass B)

`oracle/b3/`: `Cli.java` (CommandLineUtil loads), `HumanUnits.java` (humanReadableUnits, getUnsignedComparator(3), NumericUtils errors, parseInt), `FsyncRO.java` (read-only fsync → `AccessDeniedException`), `TessMorton.java` + `Morton.java` (Morton path triangles and codes), `ShapeContains.java` (LatLonShape CONTAINS/WITHIN/DISJOINT dense paths), `Scorers.java` (bulk scorers over deleted docs), `Lenient.java`, `ParseV.java` (Version parsing). Every expectation in the new tests is copied from their output.


## The ten logged findings

| # | finding | fix | test | control | evidence |
|---|---|---|---|---|---|
| 1 | `string_helper::read_string` strict UTF-8, no counterpart | removed (D) with `write_string`, their private VInt helpers and their two self-tests; re-export dropped from `util.rs` | — (compile) | — | observed: no caller outside its own tests; Java string I/O is `DataInput.readString`/`DataOutput.writeString` (store.rs:268, 477) |
| 2 | `IOUtils::use_or_suppress` keeps the second error | keeps the first (`IOUtils.java:490-497`); the second is dropped (declared: `io::Error` has no suppressed list) | `ioutils_close_reports_the_first_failure` | K1 | observed (source + test) |
| 3 | `load_fs_directory_class` returns `IllegalArgument("…not found")` | returns Java's `ClassNotFoundException`/`ClassCastException` texts (`LuceneError::Other`, as pass A's causes); the "not found" text stays `newFSDirectory`'s wrapping | `the_load_methods_throw_what_class_for_name_throws` | K24b | observed (oracle `Cli.java`) |
| 4 | `load_directory_class` refuses non-FS `Directory` classes | accepts the 12 store `Directory` classes, returns the binary name (declared: no `Class` objects) | same | K24c | observed (oracle) |
| 5 | `adjust_directory_class_name` trims Unicode whitespace | trims only chars up to U+0020, as `String.trim` | same | K24a | observed (oracle: `" "` → `ClassNotFoundException`) |
| 6 | `NativeFSLock` other errnos print `… (os error N)` | `try_lock_failure` keeps the errno's kind and the bare system text, `JNU_ThrowIOExceptionWithLastError` | `try_lock_failure_answers_as_the_jdk_try_lock` | K24e | inferred from the JDK source (`UnixFileDispatcherImpl.c` lock0); an EBADF from `tryLock` cannot be provoked from Java |
| 7 | `CommandLineUtil` builds a concrete `FSDirectory` | already faithful in the tree: `new_fs_directory_of_kind` refuses `FSDirectory` with the `NoSuchMethodException` text (no change in pass B) | existing `the_abstract_fs_directory_loads_but_cannot_be_instantiated` | — | observed (that test's oracle output, `Probe23.java cli`) |
| 8 | `Version.parseLeniently` message and cause | rewritten: upper-case, the three `replaceFirst` rewrites in order, then `parse` (a plain `8.11.0` is accepted, it was refused), failure wrapped as `failed to parse lenient version string "<in>": <reason>` with the first error as cause. `Version.parse` now splits with `StrictStringTokenizer`, reads `int`s and lets the constructor's range checks speak (`failed to parse version string "256.0.0": Illegal major version: 256`) | `parse_leniently_rewrites_and_wraps_as_lucene_does`, `parse_reads_ints_and_checks_ranges_as_lucene_does` | K20, K21 | observed (oracles `Lenient.java`, `ParseV.java`) |
| 9 | `bkd.rs` `grow` panics, doc says no error channel | returns `IllegalStateException("array length must be <= to … but was: …")` | `merge_visitor_grow_refuses_packed_values_past_the_max_array_length` | K20 | observed in the Java source (BKDWriter.java:370-376); not oracle-run |
| 10 | `size_from_balanced_tree` truncates with `as i32` | `error::to_int_exact` (`ArithmeticException("integer overflow")`), Java's assertion as `debug_assert!`, returns `Result` | `size_from_balanced_tree_refuses_an_extra_point_count_past_an_int` | K20 | observed in the Java source (BKDReader.java:556-558); not oracle-run |

## Declared divergences (new or re-worded in pass B)

- `IOUtils::use_or_suppress` drops the second error (`IOUtils.java:490-497`; `io::Error` has no suppressed list).
- `Version::parse`/`parse_leniently` report Java's `ParseException` as `IllegalArgument` with Java's messages and causes (`Version.java:288-420`); the pre-existing mapping is kept and now declared on the items.
- `CommandLineUtil::load_directory_class` returns the class's binary name for Java's `Class` (`CommandLineUtil.java:76-79`).
- `impl BitSet for SparseFixedBitSet::or` skips the `getSparseFixedBitSetOrNull` branch (`SparseFixedBitSet.java:692-700`): a `dyn DocIdSetIterator` cannot be narrowed.
- `spatial_query.rs` `and_not_iterator` is documented as the stand-in for the unported `FixedBitSet.andNot(DocIdSetIterator)` (`FixedBitSet.java:664-679`).
- `GroupVIntUtil::read_group_vints_baseline` now matches Java's `DataInput` signature (no divergence left).

## Noticed, not acted on (outside scope)

- Behaviour-identical private copies of `ArrayUtil.getUnsignedComparator`'s comparisons: `point_range_query.rs:40` `compare_unsigned`, `bkd.rs:296` `BKDUtil::unsigned_compare`, `spatial_query.rs:2162` `compare_unsigned4`.
- `FixedBitSet::from_bits` panics `given buffer is too small to hold N bits` where Java throws `IllegalArgumentException("The given long array is too small  to hold N bits")` (FixedBitSet.java:163-167; util.rs `from_bits`, not a listed item).
- `FSDirectory::rename` refuses a pending-delete source with `ensure_can_read`'s "cannot be opened for read" text; Java's is "file \"…\" is pending delete and cannot be moved" (FSDirectory.java:256-259; store.rs `rename`).
- `FieldReader::intersect` (field_reader.rs:209-234) and `AutomatonTermsEnum` (automaton_terms_enum.rs:80-105) refuse an NFA-compiled automaton that Java runs (see `get_byte_runnable`).
- `ComponentTree`'s comparators use `f64::total_cmp`; Java's `Comparator.comparingDouble` uses `Double.compare`, which orders NaNs differently (component_tree.rs:44-59).
- `Version::parse` reads components with Rust's `i32` parser (ASCII digits); Java's `Integer.parseInt` also accepts other Unicode decimal digits (`Character.digit`).
- `ArrayUtil::parse_int_radix` messages differ from Java's `NumberFormatException` (Java's out-of-range radix has a null message).
- `MaxScoreBulkScorer`'s `clear(0, innerWindowSize)` site has no failing control (see above).

## Gate (pass B, at this stop)

Target dir `.rucene-dev-target`, memory-capped runner set.
- `cargo build --all-targets`: 0 warnings.
- `cargo fmt --all -- --check`: clean.
- `cargo clippy --all-targets --message-format=json`: 151 unique (code, file, line), none on a changed line of the working tree (four `manual_is_multiple_of` in the new tests were fixed).
- `cargo test --no-fail-fast --lib` plus 31 integration targets (`search_on_disk`, `portability_search_domain`, `portability_object_contract_search`, `portability_points`, `portability_vector_point`, `portability_hnsw_graph`, `portability_codecs`, `postings_portability`, `check_index_smoke`, `delete_by_query`, `portability_delete_by_query`, `knn_accept_docs`, `knn_scorers`, `document_portability`, `points_spill`, `store_portability`, `native_fs_lock_interop`, `portability_flush_files`, `index_writer_deletes_and_updates`, `portability_io_context_contracts`, `portability_bkd_doc_ids`, `portability_vectors`, `portability_scalar_quantized_vectors`, `hnsw_vector_scorers`, `index_searcher_defaults`, `exitable_directory_reader_iteration`, `portability_index_interop`, `portability_token_stream_to_automaton`, `portability_store_analysis_contracts`, `portability_object_contract_core`, `stored_field_and_doc_values`): 2286 passed, 0 failed. The `fuzz_*` targets (whose imports changed) were not run, as instructed.

## Pass B, second half (the 71 files left by the first agent)

### Removed as a consequence (not in the input list)

- `bkd.rs`: the private copies of `MutablePointTreeReaderUtils` over `BuildPoints` — `SortByDimOps`, `PartitionOps`, `BKDWriter::sort_by_packed_value` (a std stable sort plus `apply_permutation`), `BKDWriter::sort_by_dim`, `BKDWriter::partition_by_dim`, `apply_permutation` — and `BuildPoints::swap`, which only they used. `build_mutable` now takes `&mut MutableTreePoints` and hands `points.tree` to the util.
- `fst/byte_block_pool_reverse_bytes_reader.rs`: `pool_position`, `pool_read_byte`, `pool_read_bytes`, `pool_append` (lazy block allocation, unlike Java's eager `nextBuffer`), `pool_append_from_pool` — documented as standing in until `ByteBlockPool` exposed Java's methods, which it now does.
- `codecs/lucene90/doc_values.rs`: the private Euclid `gcd_i64` (Java calls `MathUtil.gcd`, a binary GCD).
- `util/packed.rs`: `write_packed_ints_no_header` / `read_packed_ints_no_header`, bulk copies of the `PackedInts` no-header writer and reader iterator used only by the term-vectors codec.
- `util/unicode_util.rs`: the six private surrogate constants, used only by the removed `append_code_point_utf16`.

### Signature changes on the way

- `ByteBlockPool::read_byte(offset: i64)` (Java's `long`; was `i32`).
- `NodeHash` `PagedGrowableHash::bytes` returns `Vec<u8>` (the pool read cannot fail).
- `CharsRef::with_capacity` holds `capacity` NULs; `CharsRef::is_valid` reports Java's three messages; `CharsRef::from_chars_offset` asserts `is_valid` in debug builds only (Java's `assert isValid()`), replacing an always-on `assert!` with a message of its own.
- `HnswGraphSearcher::search_on_heap` returns Java's `IllegalArgumentException("length needs to be >= 1")` for a graph without nodes.
- `BytesRefBuilder::copy_chars` grows the buffer to `maxUTF8Length(utf16 length)` before copying.

### Declared divergences (new or re-worded)

- `ByteBlockPool::read_byte` and `ByteBlockPoolReverseBytesReader::next` panic where Java throws `ArrayIndexOutOfBoundsException` (ByteBlockPool.java:335-339); the removed private copy had mapped it to `CorruptIndex`.
- `CharsRef::sub_sequence` copies the characters (offset zero) where Java shares the array at `offset + start` (CharsRef.java:146-150): a `Vec<char>` has one owner.

### Noticed, not acted on (outside scope)

- `src/index/point_values_writer.rs:60-66` (module docs) still says "`PagedBytes` is not ported"; `util::paged_bytes` is ported, so the stated reason for the flat `Vec<u8>` is stale.
- `src/index/doc_values.rs:1802-1903` `OrdinalMap::new` keeps the global and per-segment deltas in `Vec<i64>` where Java builds monotonic `PackedLongValues` and repacks them (OrdinalMap.java:325-380), so `ramBytesUsed` differs from Java's.
- `src/codecs/lucene90/stored_fields.rs:448-470` `FieldsIndexReader` decodes `docs`/`startPointers` onto the heap where Java keeps `DirectMonotonicReader` views over the `.fdx` slices; its `get_block_id` (stored_fields.rs:625-631) refuses an out-of-range doc with `doc_id {d} out of bounds [0, {max})` where Java's `Objects.checkIndex` says `Index {d} out of bounds for length {max}` (FieldsIndexReader.java:121).
- `BytesRef(CharSequence)` (BytesRef.java:79-82) has no port; callers build from `str::as_bytes`, so `bytes.len()` is the UTF-8 length, not Java's `maxUTF8Length` array — visible to RAM estimates that size `bytes.length`.
- `src/util/unicode_util.rs` `UnicodeUtil::new_string` refuses lone surrogate code points and names its errors, where Java's `newString` stores them as `char`s and throws `IllegalArgumentException` with no message (UnicodeUtil.java:565-597).
- `src/util/refs.rs:74-80` `LongsRef::from_longs` panics through `expect` on `is_valid` where Java only asserts (LongsRef.java:54-60).
- `CharsRef::char_at` / `sub_sequence` panic with messages of their own where Java's `Objects.checkIndex` / `checkFromToIndex` throw `IndexOutOfBoundsException` with the JDK's.
- `BytesRefHash::sort` is non-destructive (declared at its doc); Java's `sort()` consumes the table through `compact()` (BytesRefHash.java:198-262) — the reason `compact` is W-blocked.

### Positive controls (second half)

Driver `b3/ctl3.sh` with `b3/mut3.py` (restore by `trap ... EXIT`, `md5sum -c` after each batch: all `OK`; whole tree checked against `md5_pre_b4*.txt` afterwards: `ALL_RESTORED`).

| batch | mutation | failing tests |
|---|---|---|
| LA | `MathUtil::gcd` returns 1; `SentinelIntSet::clear` no-op; `CharsRef` debug assert dropped and `with_capacity` back to `Vec::with_capacity`; `search_on_heap` back to `create_bit_set` without the size check; `copy_chars` without `maxUTF8Length` | `numeric_doc_values_match_lucene`, `sparse_numeric_doc_values_match_lucene`, `mixed_doc_values_match_lucene`, `single_term_sorted_doc_values_match_lucene` (portability_doc_values); `a_non_zero_empty_value_is_cleared_into_every_slot`; `empty_and_capacity`, `from_chars_offset_asserts_is_valid`; `search_on_heap_over_a_graph_without_nodes_throws_what_the_bit_set_throws`; `bytes_ref_builder_copy_chars_grows_to_the_max_utf8_length` (5 lib + 4 integration) |
| L30 | `MutablePointTreeReaderUtils::sort` returns at once | `one_dimensional_int_points_match_lucene`, `eight_byte_points_match_lucene`, `sixteen_byte_points_match_lucene`, `a_single_leaf_tree_matches_lucene`, `the_kdm_field_order_follows_the_field_hash`, `mutable_point_tree_sort_keeps_equal_values_in_doc_order`, `a_merge_takes_the_bkd_merge_path_only_when_every_reader_is_lucene90` |
| L31 | `sort_by_dim` returns at once | 7 portability_points tests (`two_dimensional_points_match_lucene`, `a_deep_multi_dimension_tree_matches_lucene`, `the_leaf_sorted_dimension_is_chosen_as_lucene_chooses_it`, …) |
| L32 | `partition` returns at once | 4 portability_points multi-dimension tests and 2 points_spill tests |
| L33 | `stable_fallback_sort` runs an intro sort | `mutable_point_tree_sort_keeps_equal_values_in_doc_order`, `multi_valued_and_sparse_points_match_lucene`, `the_kdm_field_order_follows_the_field_hash` — i.e. without the stable fallback the newly wired `MutablePointTreeReaderUtils::sort` writes leaves Lucene does not |
| L34 | `ByteBlockPool::position` + 1 | `fixed_length_arc_nodes_match_lucene_1050` |
| L35 | `ByteBlockPool::read_byte` returns 0 | `fixed_length_arc_nodes_match_lucene_1050` (+ two pool unit tests) |
| L36 | `ByteBlockPool::append` drops the last byte | `fixed_length_arcs_buffer_matches_lucene_1050`, `fixed_length_arc_nodes_match_lucene_1050` (+ one pool unit test) |
| L37 | `ByteBlockPool::append_pool` copies nothing | first run: **no failing test** (no test reached `copyFallbackNodeBytes`); after adding `a_tiny_suffix_ram_limit_promotes_fallback_nodes_as_lucene_does` (pinned to the oracle `oracle/b4/FstFallback.java 0.001 3000`): that test fails |
| L42a | `PackedInts::get_writer_no_header` writes one bit wider | 5 portability_term_vectors tests + 8 lib term-vectors tests |
| L42b | `PackedInts::get_reader_iterator_no_header` reads one bit wider | 6 portability_term_vectors tests (including `rucene_reads_the_term_vectors_of_a_compound_file_segment_written_by_lucene`) + 7 lib term-vectors tests |

### Java oracle (second half)

`oracle/b4/FstFallback.java <mb> <n>`: builds an FST of `n` generated terms under `suffixRAMLimitMB(mb)` and prints node/arc counts, the saved byte length and its CRC32. `1000 3000` → `nodes=450 arcs=2590 bytes=10777`; `0.001 3000` → `nodes=13316 arcs=16314 bytes=45999 crc=1824107834` (pinned in the new FST test).

### Gate (second half)

Target dir `.rucene-dev-target`, memory-capped runner set.
- `cargo build --all-targets`: 0 warnings.
- `cargo fmt --all -- --check`: clean.
- `cargo clippy --all-targets --message-format=json`: 151 unique (code, file, line), none on a changed line of the working tree (one `err_expect` in a new test was rewritten as `let … else`).
- `cargo test --no-fail-fast --lib` plus 28 integration targets (`portability_points`, `points_spill`, `portability_bkd_doc_ids`, `portability_vector_point`, `document_portability`, `portability_flush_files`, `portability_index_interop`, `portability_fst_arcs_buffer`, `portability_byte_block_pool_accounting`, `portability_prefix_coded_terms`, `portability_codecs`, `portability_doc_values`, `stored_field_and_doc_values`, `doc_values_writer_ram`, `portability_doc_values_skipper`, `portability_term_vectors`, `portability_compressing_accounting`, `portability_hnsw_graph`, `knn_scorers`, `hnsw_vector_scorers`, `portability_vectors`, `portability_scalar_quantized_vectors`, `portability_java_collections`, `java_long_hash_map_order`, `portability_seal_bytes`, `check_index_smoke`, `portability_stored_fields`, `portability_norms`, `portability_indexing_chain`): 2308 passed, 0 failed (lib 2085). Added afterwards and run on their own: `a_tiny_suffix_ram_limit_promotes_fallback_nodes_as_lucene_does` (pass) and the `let … else` form of the empty-graph test (pass). The `fuzz_*` targets were not run.

