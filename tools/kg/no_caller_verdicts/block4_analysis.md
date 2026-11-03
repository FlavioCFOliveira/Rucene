# Task #269 block 4 — `analysis` no-production-caller verdicts

Input: `list_0921226.jsonl`, rows with `"subsystem": "analysis"` (123 items in 13 files), checked against the working tree (HEAD `0921226` plus pass H1, plus this pass H2, which also carries task #320). Java citations are relative to `/tmp/lucene1050/lucene/core/src/java/org/apache/lucene/analysis/` (10.5.0 tag) unless a path says otherwise. Oracles: `scratchpad/oracle/h2/SpiOracle.java` and `MiscOracle.java`, run with `java -cp lucene-core-10.5.0.jar <Oracle>.java` (outputs `spi_out.txt`, `misc_out.txt`).

Final classes as in block 3: **W** link ported now; **W=** the Java link already exists in Rust; **W-blocked** missing link whose Java caller is not ported or needs a larger change (the Java caller is named); **P** legitimate API, or a faithful member uncalled in Lucene, kept; **D** no Lucene counterpart, removed; **T** test-only, no Lucene counterpart, left.

## Counts

Auto (all 123): D? 41, P? 56, W? 26  
Final: D 9, P 84, T 4, W 16, W-blocked 4, W= 6

| auto \ final | D | P | T | W | W-blocked | W= |
|---|---|---|---|---|---|---|
| D? | 8 | 27 | 4 | 2 | 0 | 0 |
| P? | 0 | 56 | 0 | 0 | 0 | 0 |
| W? | 1 | 1 | 0 | 14 | 4 | 6 |

## Items

| item | auto | final | action | Java evidence |
|---|---|---|---|---|
| `analysis::AnalyzerState::id` | D? | **D** | removed (with the `id` field and its counter) | no Lucene member: `Analyzer` holds `reuseStrategy`, `version` and `storedValue` (Analyzer.java:86-100), no identifier; no Rust caller |
| `analysis::AnalyzerWrapper` | W? | **W-blocked** | blocked: Java's `DelegatingAnalyzerWrapper` extends it and installs a `DelegatingReuseStrategy` that reaches the wrapper's `getWrappedAnalyzer` (DelegatingAnalyzerWrapper.java:45-49, 78-103); the Rust `ReuseStrategy` receives the `AnalyzerState`, not the `Analyzer`, and a default trait method cannot pass `&Self` as `&dyn Analyzer`, so `DelegatingAnalyzerWrapper` delegates `token_stream`/`normalize` instead (same observable result) | AnalyzerWrapper.java:31; Java caller DelegatingAnalyzerWrapper (class extends AnalyzerWrapper) |
| `analysis::AnalyzerWrapper::new` | W? | **W-blocked** | blocked: as `AnalyzerWrapper` | AnalyzerWrapper.java:55; Java caller DelegatingAnalyzerWrapper(ReuseStrategy) `super(new DelegatingReuseStrategy(...))` (DelegatingAnalyzerWrapper.java:45-49) |
| `analysis::CharFilterFn` | D? | **T** | none: closure-built `CharFilter` used by lib and portability tests (`doubling_filter`, new offset test) | no Lucene member: `CharFilter` is abstract with no closure subclass |
| `analysis::CharFilterFn::new` | D? | **T** | none: as `CharFilterFn` | as `CharFilterFn` |
| `analysis::DelegatingAnalyzerWrapper` | P? | **P** | none | DelegatingAnalyzerWrapper.java:37 |
| `analysis::DelegatingAnalyzerWrapper::new` | P? | **P** | none | DelegatingAnalyzerWrapper.java:45 |
| `analysis::PerFieldReuseStrategy` | D? | **P** | none (Lucene's anonymous `PER_FIELD_REUSE_STRATEGY` constant) | Analyzer.java:488 PER_FIELD_REUSE_STRATEGY |
| `analysis::TokenStreamComponents::from_tokenizer` | D? | **P** | doc names the Java constructor (renamed overload) | Analyzer.java:380 TokenStreamComponents(Tokenizer) |
| `analysis::TokenStreamComponents::reset` | D? | **D** | removed | no Lucene member: TokenStreamComponents has setReader/getTokenStream/getSource only (Analyzer.java:345-410); no Rust caller |
| `analysis::TokenStreamComponents::with_sink` | D? | **T** | none: test-only (`analyzer_wrapper_delegates_and_can_wrap_components`) | no Lucene member (Analyzer.java:345-410) |
| `analysis::Tokenizer::correct_offset` | W? | **W** | wired: `StandardTokenizerLogic` corrects every token offset and the final offset through `Tokenizer::correct_offset_of` (the body of `correctOffset` over the input the tokenizer hands the logic); `TokenizerLogic::end` now receives that input. Test `offsets_are_corrected_through_the_char_filters` (oracle O5) | Tokenizer.java:79; callers StandardTokenizer.java:167 (incrementToken), :181 (end) |
| `analysis::automaton_conversion::AutomatonToTokenStream` | P? | **P** | none | AutomatonToTokenStream.java:35 |
| `analysis::automaton_conversion::AutomatonToTokenStream::to_token_stream` | P? | **P** | none | AutomatonToTokenStream.java:48 |
| `analysis::automaton_conversion::TokenStreamToAutomaton` | P? | **P** | none | TokenStreamToAutomaton.java:37 |
| `analysis::automaton_conversion::TokenStreamToAutomaton::set_final_offset_gap_as_hole` | P? | **P** | none | TokenStreamToAutomaton.java:56 |
| `analysis::automaton_conversion::TokenStreamToAutomaton::set_preserve_position_increments` | P? | **P** | none | TokenStreamToAutomaton.java:51 |
| `analysis::automaton_conversion::TokenStreamToAutomaton::set_unicode_arcs` | P? | **P** | none | TokenStreamToAutomaton.java:64 |
| `analysis::automaton_conversion::TokenStreamToAutomaton::to_automaton` | P? | **P** | none | TokenStreamToAutomaton.java:108 |
| `analysis::caching_token_filter::CachingTokenFilter::is_cached` | P? | **P** | doc claim added | CachingTokenFilter.java:100 isCached |
| `analysis::caching_token_filter::new_caching_token_filter` | D? | **D** | removed (and its re-export) | no Lucene member: duplicate of `CachingTokenFilter::new` (CachingTokenFilter.java constructor); no caller |
| `analysis::char_array_map::CharArrayMap::contains_key` | W? | **W** | wired: `CharArraySet::contains` is `map.contains_key(cs)` (was `contains_chars`); control CN (oracle O3 contains rows) | CharArrayMap.java:103; caller CharArraySet.java:93 contains(CharSequence) |
| `analysis::char_array_map::CharArrayMap::create_entry_set` | W? | **W-blocked** | blocked: the Rust `EntrySet` needs `&mut CharArrayMap` (for `setValue`) while `entry_set`/`Display` take `&self`, so `entry_set` stays a read-only snapshot | CharArrayMap.java:322; Java caller CharArrayMap.entrySet() (CharArrayMap.java:327-332) |
| `analysis::char_array_map::CharArrayMap::get` | W? | **W=** | none: the Java `get(Object)` overload dispatches to `get(char[],int,int)`, which `EntrySet::contains` calls directly as `get_chars` | CharArrayMap.java:124,129; callers get(Object), EntrySet.contains (CharArrayMap.java:485-493) |
| `analysis::char_array_map::CharArrayMap::get_hash_code` | W? | **W** | wired: `get_slot` probes from `get_hash_code` (was `CharsRef::string_hash_code` of a fully lower-cased copy), which is now Java's: UTF-16 units when case matters, `Character.toLowerCase` per code point when not; `equals` compares UTF-16 lengths and simple-lower-cased code points; `put` lower-cases with `CharacterUtils.toLowerCase`. Test `iteration_and_lookup_follow_javas_hash_and_case_rules` (oracle O3) | CharArrayMap.java:266-279; callers getSlot (CharArrayMap.java:136-165) |
| `analysis::char_array_map::CharArrayMap::key_set` | W? | **W=** | none: the listed caller is `keySet()` reading its own cache field; the Rust returns owned keys (declared in its doc) | CharArrayMap.java:320,345 |
| `analysis::char_array_map::CharArrayMap::remove` | P? | **P** | none | CharArrayMap.java:300 |
| `analysis::char_array_map::EntrySet::clear` | P? | **P** | none | CharArrayMap.java:505 |
| `analysis::char_array_map::EntrySet::contains` | P? | **P** | none | CharArrayMap.java:485 |
| `analysis::char_array_map::EntrySet::iter_mut` | D? | **P** | none (renamed: `EntrySet.iterator()`) | CharArrayMap.java:479-481 |
| `analysis::char_array_map::EntrySet::size` | P? | **P** | none | CharArrayMap.java:500 |
| `analysis::char_array_map::MapEntry::get_key` | D? | **P** | none (`Map.Entry` API) | CharArrayMap.java:443 |
| `analysis::char_array_map::MapEntry::get_value` | D? | **P** | none (`Map.Entry` API) | CharArrayMap.java:450 |
| `analysis::char_array_map::MapEntry::set_value` | D? | **P** | none (`Map.Entry` API) | CharArrayMap.java:455 |
| `analysis::char_array_set::CharArraySet::clear` | P? | **P** | none | CharArraySet.java:80 |
| `analysis::char_array_set::CharArraySet::contains` | W? | **W** | wired: calls `map.contains_key` as Java's `contains(CharSequence)`; Java's caller StopFilter.accept uses the `char[]` overload, ported as `contains_chars` (W=) | CharArraySet.java:88-100; caller StopFilter.java:96-98 |
| `analysis::char_array_set::CharArraySet::ignore_case` | D? | **D** | removed, with `CharArrayMap::ignore_case` (its only caller) | no Lucene member: `ignoreCase` is a package-private field of CharArrayMap with no accessor |
| `analysis::char_array_set::CharArraySet::is_empty` | D? | **P** | doc claim added | AbstractCollection.isEmpty (inherited) |
| `analysis::char_array_set::CharArraySet::size` | P? | **P** | doc claim added | CharArraySet.java:126 |
| `analysis::character_utils::CharacterBuffer::buffer` | W? | **W=** | none: `fill` reads the field, as Java's `fill` reads `buffer.buffer` | CharacterUtils.java:213 getBuffer; callers CharacterUtils.java:146 fill |
| `analysis::character_utils::CharacterBuffer::length` | W? | **W=** | none: as `buffer` | CharacterUtils.java:231 getLength; caller CharacterUtils.java:161-170 fill |
| `analysis::character_utils::CharacterBuffer::offset` | W? | **W=** | none: the field read by its own getter | CharacterUtils.java:222 getOffset |
| `analysis::character_utils::CharacterBuffer::reset` | P? | **P** | none | CharacterUtils.java:236 |
| `analysis::character_utils::fill_buffer` | D? | **P** | doc claim added (renamed overload) | CharacterUtils.java:176 fill(CharacterBuffer, Reader) |
| `analysis::character_utils::new_character_buffer` | D? | **P** | fixed: message `buffersize must be >= 2` (was `buffer_size …`); test `case_mappings_and_messages_are_javas` | CharacterUtils.java:38-43 |
| `analysis::character_utils::to_chars` | D? | **P** | doc claim added | CharacterUtils.java:105 |
| `analysis::character_utils::to_code_points` | D? | **P** | doc claim added | CharacterUtils.java:86 |
| `analysis::character_utils::to_lower_case` | D? | **W** | wired and fixed: Java's simple one-for-one `Character.toLowerCase` per code point, in place (`&mut [char]`, was a full-mapping splice that could grow the buffer); `LowerCaseFilter` lower-cases the term buffer with it (was `String::to_lowercase`), `CharArrayMap::put` lower-cases keys with it; module divergence notes removed. Tests `case_mappings_and_messages_are_javas`, `the_filter_lower_cases_with_the_simple_mapping` (oracle O3) | CharacterUtils.java:53-61; callers LowerCaseFilter.java:38, CharArrayMap.java:191 |
| `analysis::character_utils::to_upper_case` | D? | **P** | fixed: Java's simple `Character.toUpperCase` per code point (ß stays ß); test `case_mappings_and_messages_are_javas` | CharacterUtils.java:71-79; no caller in core |
| `analysis::factories::AbstractAnalysisFactory::default_ctor_exception` | W? | **W** | wired: `StandardTokenizerFactory::new_without_args`, the port of the public no-argument constructor, returns it. Test `the_no_argument_constructor_throws_the_default_ctor_exception` (oracle A9) | AbstractAnalysisFactory.java:88; callers AbstractAnalysisFactory() (:76-78, no Rust counterpart), StandardTokenizerFactory.java:53-55 |
| `analysis::factories::AbstractAnalysisFactory::get` | W? | **W** | wired: the constructor reads `luceneMatchVersion` with `get` (was `args.remove`). Test `the_constructor_reads_the_version_and_leaves_it_implicit` | AbstractAnalysisFactory.java:156; caller AbstractAnalysisFactory.java:97 |
| `analysis::factories::AbstractAnalysisFactory::get_boolean` | P? | **P** | fixed: `Boolean.parseBoolean` (any-case `true`, else `false`, never fails; returns `bool`), was Rust's strict `bool` parser | AbstractAnalysisFactory.java:213-216 |
| `analysis::factories::AbstractAnalysisFactory::get_char` | P? | **P** | fixed: refuses a value whose UTF-16 length is not 1 (a supplementary character) | AbstractAnalysisFactory.java:231-242 |
| `analysis::factories::AbstractAnalysisFactory::get_class_arg` | P? | **P** | fixed: Java's `getClassArg()` — the `class` argument or the factory's class name (passed in, as `get_pattern`'s simple name); was a comma-splitting reader of any argument | AbstractAnalysisFactory.java:369-377 |
| `analysis::factories::AbstractAnalysisFactory::get_float` | P? | **P** | fixed: `Float.parseFloat` grammar (trim ≤U+0020, sign, NaN/Infinity, hex floats, f/F/d/D suffix) and its messages (`empty String`, `For input string: "…"`) | AbstractAnalysisFactory.java:222-225 |
| `analysis::factories::AbstractAnalysisFactory::get_lucene_match_version` | P? | **P** | none | AbstractAnalysisFactory.java:115 |
| `analysis::factories::AbstractAnalysisFactory::get_or` | D? | **P** | doc names the overload (renamed) | AbstractAnalysisFactory.java:160 get(Map, String, String) |
| `analysis::factories::AbstractAnalysisFactory::get_original_args` | P? | **P** | none | AbstractAnalysisFactory.java:111 |
| `analysis::factories::AbstractAnalysisFactory::get_pattern` | P? | **P** | none (pass H1) | AbstractAnalysisFactory.java:266-277 |
| `analysis::factories::AbstractAnalysisFactory::get_set` | P? | **P** | fixed: matches of `[^,\s]+` into a `JavaHashSet` (Java's `HashSet` order), `None` when no item; was a comma split with trimming into a `Vec` | AbstractAnalysisFactory.java:244-263 |
| `analysis::factories::AbstractAnalysisFactory::get_snowball_word_set` | P? | **P** | none | AbstractAnalysisFactory.java:308-328 |
| `analysis::factories::AbstractAnalysisFactory::is_explicit_lucene_match_version` | W? | **W=** | fixed: the flag starts `false` and the constructor leaves it (Rust set it `true` when the version argument was present); the getter reads the field as Java's does. Test `the_constructor_reads_the_version_and_leaves_it_implicit` (oracles O2, A6 `explicit=false`) | AbstractAnalysisFactory.java:64,379-381 |
| `analysis::factories::AbstractAnalysisFactory::require_boolean` | P? | **P** | fixed: as `get_boolean` | AbstractAnalysisFactory.java:209-211 |
| `analysis::factories::AbstractAnalysisFactory::require_char` | P? | **P** | fixed: an empty value fails with `charAt(0)`'s `Index 0 out of bounds for length 0` (as `LuceneError::Other`, declared) | AbstractAnalysisFactory.java:227-229 |
| `analysis::factories::AbstractAnalysisFactory::require_float` | P? | **P** | fixed: as `get_float` | AbstractAnalysisFactory.java:218-220 |
| `analysis::factories::AbstractAnalysisFactory::require_int` | P? | **P** | fixed: `Integer.parseInt` (`util::extra::java_parse_int`, now `pub(crate)`) with its message `For input string: "…"` | AbstractAnalysisFactory.java:200-202 |
| `analysis::factories::AbstractAnalysisFactory::require_one_of` | D? | **P** | doc names the overload (renamed); fixed: `equalsIgnoreCase` (simple case maps per code point, was ASCII) and the list rendered `[a, b]` (was `{:?}`) | AbstractAnalysisFactory.java:131-154 require(Map, String, Collection, boolean) |
| `analysis::factories::AbstractAnalysisFactory::set_explicit_lucene_match_version` | P? | **P** | none | AbstractAnalysisFactory.java:383 |
| `analysis::factories::AnalysisSPILoader::lookup` | D? | **D** | removed: replaced by Java's `new_instance(name, args)` | no Lucene member: AnalysisSPILoader has newInstance/lookupClass/availableServices/lookupSPIName/newFactoryClassInstance (AnalysisSPILoader.java:139-213) |
| `analysis::factories::AnalysisSPILoader::lookup_spi_name` | W? | **W** | wired (#320): `reload` reads each scanned class's name with it, and the three `find_*_spi_name` wrap its failure as `IllegalStateException(e)`; it reads `AnalysisFactoryClass`'s `NAME` field model (`SPINameField`). Tests `refused_factory_classes_leave_the_loader_unchanged`, `new_instance_and_find_spi_name_report_as_java_does` (oracles A3, A8) | AnalysisSPILoader.java:175-186; callers :88 reload, TokenizerFactory/TokenFilterFactory/CharFilterFactory.findSPIName |
| `analysis::factories::AnalysisSPILoader::new_factory_class_instance` | W? | **W** | wired (#320): `new_instance` and the three `*_for_name` call it on the looked-up class (the loader now holds classes, not instances). Test `new_instance_and_find_spi_name_report_as_java_does` (oracle A6) | AnalysisSPILoader.java:192-213; caller :139-142 newInstance |
| `analysis::factories::available_char_filters` | P? | **P** | none | CharFilterFactory.availableCharFilters |
| `analysis::factories::available_token_filters` | P? | **P** | none | TokenFilterFactory.availableTokenFilters |
| `analysis::factories::available_tokenizers` | P? | **P** | none (now `[standard]` from lucene-core's service file, oracle A1) | TokenizerFactory.availableTokenizers |
| `analysis::factories::char_filter_for_name` | D? | **P** | fixed (#320): Java's `forName(String, Map)` — takes the argument map and instantiates the class (was a lookup of a registered instance) | CharFilterFactory.forName |
| `analysis::factories::find_char_filter_spi_name` | D? | **P** | fixed (#320): takes the factory class, calls `lookup_spi_name`, wraps its failure | CharFilterFactory.findSPIName |
| `analysis::factories::find_token_filter_spi_name` | D? | **P** | fixed (#320): as `find_char_filter_spi_name` | TokenFilterFactory.findSPIName |
| `analysis::factories::find_tokenizer_spi_name` | D? | **P** | fixed (#320): as `find_char_filter_spi_name` | TokenizerFactory.findSPIName |
| `analysis::factories::reload_char_filters` | P? | **P** | fixed (#320): `reloadCharFilters(ClassLoader)` — reloads from a class loader (was: registers one factory) | CharFilterFactory.reloadCharFilters |
| `analysis::factories::reload_token_filters` | P? | **P** | fixed (#320): as `reload_char_filters` | TokenFilterFactory.reloadTokenFilters |
| `analysis::factories::reload_tokenizers` | P? | **P** | fixed (#320): as `reload_char_filters` | TokenizerFactory.reloadTokenizers |
| `analysis::factories::token_filter_for_name` | D? | **P** | fixed (#320): as `char_filter_for_name` | TokenFilterFactory.forName |
| `analysis::factories::tokenizer_for_name` | D? | **P** | fixed (#320): as `char_filter_for_name` | TokenizerFactory.forName |
| `analysis::filtering_token_filter::new_filtering_token_filter` | D? | **W** | wired: `new_stop_filter` builds through it (Java's `super(in)`), replacing an inline copy of its body; doc names the constructor | FilteringTokenFilter.java:38; caller StopFilter.java:37-40 |
| `analysis::graph_token_filter::GraphTokenFilter` | P? | **P** | none | GraphTokenFilter.java:41 |
| `analysis::graph_token_filter::GraphTokenFilter::cached_token_count` | D? | **P** | none (package-private, read by Lucene's tests) | GraphTokenFilter.java:177 |
| `analysis::graph_token_filter::GraphTokenFilter::get_final_offsets` | D? | **D** | removed; the field's Java reader, `end()`, is ported as `GraphTokenFilter::end` (test `end_sets_the_recorded_trailing_state_again`, oracle O6b) | no Lucene member: `finalOffsets` is private, read only by end() (GraphTokenFilter.java:56,151-162) |
| `analysis::graph_token_filter::GraphTokenFilter::get_trailing_positions` | P? | **P** | none | GraphTokenFilter.java:147 |
| `analysis::graph_token_filter::GraphTokenFilter::increment_base_token` | P? | **P** | none | GraphTokenFilter.java:76 |
| `analysis::graph_token_filter::GraphTokenFilter::increment_graph` | P? | **P** | none | GraphTokenFilter.java:119 |
| `analysis::graph_token_filter::GraphTokenFilter::increment_graph_token` | P? | **P** | none | GraphTokenFilter.java:97 |
| `analysis::graph_token_filter::GraphTokenFilter::new` | P? | **P** | none | GraphTokenFilter.java:65 |
| `analysis::graph_token_filter::GraphTokenFilter::reset` | W? | **W-blocked** | blocked: Java's callers are the `TokenStream.reset()` dispatch sites; the Rust `GraphTokenFilter` is not a `TokenStream` (Java's class is abstract and has no subclass in core), so nothing dispatches to it | GraphTokenFilter.java:165; Java callers Analyzer.normalize, CachingTokenFilter.reset, FilteringTokenFilter.reset, AutomatonToTokenStream.TopoTokenStream.reset (through TokenStream.reset) |
| `analysis::standard::StandardAnalyzer::max_token_length` | W? | **W** | wired: the field is an `Arc<AtomicI32>` the components' source reads on every reader, as Java's lambda reads `StandardAnalyzer.this.maxTokenLength`; getter returns Java's `int`. Test `the_analyzer_applies_its_max_token_length_on_every_reader` (oracle O4) | StandardAnalyzer.java:39,80; callers :85-93 createComponents and its lambda |
| `analysis::standard::StandardAnalyzer::set_max_token_length` | P? | **P** | fixed: stores any `int` without checking (was an early range check); the tokenizer refuses it at the next `token_stream` (declared: create_components cannot fail, the source reports it) | StandardAnalyzer.java:71-73 |
| `analysis::standard::StandardAnalyzer::stopwords` | W? | **W** | wired: returns the held `StopwordAnalyzerBase`'s set (Java's inherited field / getStopwordSet) | StopwordAnalyzerBase.java:34,41; callers StandardAnalyzer.java:89 createComponents |
| `analysis::standard::StandardTokenizerFactory` | P? | **P** | none | StandardTokenizerFactory.java:36 |
| `analysis::standard::StandardTokenizerFactory::max_token_length` | W? | **D** | removed: Java's `maxTokenLength` is a private field with no accessor; its reader `create()` reads the field (W=) | StandardTokenizerFactory.java:41,60 |
| `analysis::standard::StandardTokenizerFactory::new` | P? | **P** | fixed: `maxTokenLength` kept as Java's `int` (was clamped to `usize`); leftovers reported as `Unknown parameters: {k=v, …}` (was `{:?}` of keys; order declared) | StandardTokenizerFactory.java:44-51 |
| `analysis::standard::StandardTokenizerImpl::clear_skipped_positions` | D? | **D** | removed (with the `skipped_positions` field) | no Lucene member: the JFlex StandardTokenizerImpl has no skipped-position state (StandardTokenizer keeps `skippedPositions`) |
| `analysis::standard::StandardTokenizerImpl::skipped_positions` | D? | **D** | removed | as `clear_skipped_positions` |
| `analysis::standard::StandardTokenizerLogic::max_token_length` | D? | **P** | doc claim added: `StandardTokenizerLogic` is the body of `StandardTokenizer` | StandardTokenizer.java:112 getMaxTokenLength |
| `analysis::standard::StopwordAnalyzerBase` | W? | **W** | wired: `StandardAnalyzer` holds one (Java: extends it) and keeps its stop words there | StopwordAnalyzerBase.java:30; dependent StandardAnalyzer |
| `analysis::standard::StopwordAnalyzerBase::load_stopword_set_from_path` | D? | **P** | doc names the overload (renamed) | StopwordAnalyzerBase.java:70 loadStopwordSet(Path) |
| `analysis::standard::StopwordAnalyzerBase::new` | W? | **W** | wired: `StandardAnalyzer::with_stopwords` is `super(stopWords)` (unmodifiable copy). Test `the_analyzer_keeps_an_unmodifiable_copy_of_its_stop_words` (oracle O4) | StopwordAnalyzerBase.java:50-56; caller StandardAnalyzer.java:46-48 |
| `analysis::standard::StopwordAnalyzerBase::stopwords` | W? | **W** | wired: read by `StandardAnalyzer::create_components` and `stopwords()` | StopwordAnalyzerBase.java:34,41; callers StandardAnalyzer.java:89 |
| `analysis::stop_filter::make_stop_set` | W? | **W** | wired: `AbstractAnalysisFactory::get_word_set` adds `make_stop_set(lines, ignore_case)` per file (Java's `words.addAll(StopFilter.makeStopSet(wlist, ignoreCase))`). Test `get_word_set_adds_each_files_stop_set` (oracle O2 `[bar, foo]`) | StopFilter.java:87; caller AbstractAnalysisFactory.java:292-293 |
| `analysis::tokenattributes::BytesTermAttributeImpl::new` | P? | **P** | none | BytesTermAttributeImpl.java:35 |
| `analysis::tokenattributes::BytesTermAttributeImpl::with_bytes` | D? | **T** | none: test-only | no Lucene member: BytesTermAttributeImpl has only the no-argument constructor (BytesTermAttributeImpl.java:35) |
| `analysis::tokenattributes::CharTermAttributeImpl::append_null` | W? | **P** | none: Java's callers are the `append(...)` null branches (`csq == null`), which a Rust `&str`/`&dyn CharTermAttribute` cannot reach (declared in its doc) | CharTermAttributeImpl.java:196; null branches at :120,169,179,189 |
| `analysis::tokenattributes::FlagsAttributeImpl` | P? | **P** | none | FlagsAttributeImpl.java:23 |
| `analysis::tokenattributes::FlagsAttributeImpl::new` | P? | **P** | none | FlagsAttributeImpl.java:27 |
| `analysis::tokenattributes::KeywordAttributeImpl` | P? | **P** | none | KeywordAttributeImpl.java:23 |
| `analysis::tokenattributes::KeywordAttributeImpl::new` | P? | **P** | none | KeywordAttributeImpl.java:27 |
| `analysis::tokenattributes::OffsetAttributeImpl::new` | P? | **P** | none | OffsetAttributeImpl.java:28 |
| `analysis::tokenattributes::PayloadAttributeImpl::new` | P? | **P** | none | PayloadAttributeImpl.java:28 |
| `analysis::tokenattributes::PayloadAttributeImpl::with_payload` | D? | **P** | doc names the constructor (renamed) | PayloadAttributeImpl.java:31 PayloadAttributeImpl(BytesRef) |
| `analysis::tokenattributes::SentenceAttributeImpl` | P? | **P** | none | SentenceAttributeImpl.java:30 |
| `analysis::tokenattributes::SentenceAttributeImpl::new` | P? | **P** | none | SentenceAttributeImpl.java:35 |
| `analysis::tokenattributes::TypeAttributeImpl` | P? | **P** | none | TypeAttributeImpl.java:24 |
| `analysis::tokenattributes::TypeAttributeImpl::with_type` | D? | **P** | doc names the constructor (renamed) | TypeAttributeImpl.java:33 TypeAttributeImpl(String) |
| `analysis::wordlist_loader::get_stem_dict` | D? | **P** | none (its missing-tab refusal is declared at the item) | WordlistLoader.java:276 |
| `analysis::wordlist_loader::get_word_set_ignoring_comments` | D? | **P** | doc names the overload (renamed) | WordlistLoader.java:143 getWordSet(Reader, String) |

## Task #320 (same pass): class loaders for the SPI loaders

- `util::resource_loader::ClassLoader` now also carries the lines of its `META-INF/services` files (`ServiceProvider`: service name, provider class name, type-erased class object); `ClassLoader::with_services` builds one, `get_resources(service)` lists the lines parent-first (`ClassLoader.getResources` order).
- `util::spi::load_service_classes` is the class-path half of `ServiceLoader.load`: lines in that order, a class name already seen in the scan skipped (observed, SpiOracle C1: `CountA` instantiated once although two files list it), a class object of the wrong type refused as `<service>: <class> not a subtype` (observed, C3b).
- `NamedSPILoader::new(&ServiceClass, Option<&Arc<ClassLoader>>)` and `reload(&Arc<ClassLoader>)` are Java's (`NamedSPILoader.java:41-79`): `ClassLoaderUtils::is_parent_class_loader` decides whether the SPI's own loader is scanned first (observed, C1/C2); `reload` instantiates every provider reached (C3, C6), checks only names it adds, and installs all or nothing (C3). The empty constructor of the crate's registries is renamed `NamedSPILoader::empty` (port-only, documented); `with_providers` and the provider-list `reload` are gone.
- `AnalysisSPILoader::new`/`reload` likewise (`AnalysisSPILoader.java:51-133`), over `AnalysisFactoryClass<T>` (binary name, `SPINameField` model of the `NAME` field, `Map` constructor): it holds classes and instantiates on `new_instance`, as Java.
- `lucene_core_class_loader()` holds lucene-core 10.5.0's eight service files. The sort-field loader is now `NamedSPILoader::new(SortFieldProvider.class)` over it; the analysis global loaders likewise (`available_tokenizers() == [standard]`, A1).
- Entry points take a class loader, as Java: `reload_codecs`, `reload_postings_formats`, `reload_doc_values_formats`, `reload_knn_vectors_format`, `reload_sort_field_providers`, `reload_tokenizers`, `reload_token_filters`, `reload_char_filters`.
- `ClassLoaderUtils` / `ClassLoaderUtils::is_parent_class_loader` (block 3 W-blocked rows 682-683): now **W**, called by both constructors.

## Removed as a consequence (not in the input list)

- `AnalysisFactorySPIName` (trait and its three impls) and the `name()` method of `TokenizerFactory`/`TokenFilterFactory`/`CharFilterFactory`: stand-ins for the static `NAME` field, which now lives in `AnalysisFactoryClass`.
- `AnalysisSPILoader::register` (port-only one-service `reload`), `NamedSPILoader::with_providers`.
- `CharArrayMap::ignore_case` (only caller was the removed `CharArraySet::ignore_case`), `CharArrayMap::normalize` (private full-lowercasing copy replaced by Java's hash/equals rules).
- The old `factories.rs` `reload_tests` module (tested the explicit-provider loader), replaced by `spi_tests`.

## Signature changes on the way

- `TokenizerLogic::end(&mut self, source, input: Option<&dyn CharReader>)` (the logic needs the input for `correctOffset`); new `Tokenizer::correct_offset_of(input, off)`.
- `StandardTokenizerLogic::set_max_token_length(i32)`, `StandardAnalyzer::set_max_token_length(&mut self, i32)` (no `Result`), `StandardAnalyzer::max_token_length() -> i32`; `StandardTokenizerFactory` keeps Java's `int`; new `StandardTokenizerFactory::new_without_args`, `StandardTokenizerFactory::class()`.
- `character_utils::to_lower_case`/`to_upper_case(&mut [char], usize, usize)` (in place, no return).
- `AbstractAnalysisFactory::get_boolean -> bool`, `get_set -> Option<JavaHashSet>`, `get_class_arg(&self, java_class_name) -> String`.
- `*_for_name(name, &mut args)`, `*_lookup_class -> Arc<AnalysisFactoryClass<_>>`, `find_*_spi_name(&AnalysisFactoryClass<_>) -> Result<&'static str>`.
- `util::extra::java_parse_int` is `pub(crate)`.

## Java oracles (pass H2)

`scratchpad/oracle/h2/SpiOracle.java` (C1-C7 NamedSPILoader over custom loaders; A1-A9 AnalysisSPILoader, findSPIName, forName, no-arg ctor) → `spi_out.txt`; `MiscOracle.java` (O2 AbstractAnalysisFactory accessors incl. 47 `Float.parseFloat` inputs; O3 CharArraySet slot order and CharacterUtils case maps; O4 StandardAnalyzer max length and stop words; O5 StandardTokenizer offsets through two char filters; O6/O6b GraphTokenFilter.end) → `misc_out.txt`; `FloatExtra.java` (two extra float rows). Every expectation in the new tests is copied from these outputs.

## Positive controls (pass H2)

Driver `scratchpad/oracle/h2/scripts/ctl.sh` with `mut.py` (restore by `trap … EXIT`, `md5sum -c` after each batch: all OK). Each batch ran exactly its targeted lib tests; every targeted test failed, and passed after restore (final lib run 246 passed).

| batch | mutations | failing tests |
|---|---|---|
| N1 | NamedSPILoader/AnalysisSPILoader parent check `if false`; sort-field loader built empty; each of the 8 `reload_*` entry points scans an empty loader; `lookup_spi_name` accepts a missing NAME | `a_non_descendant_loader_is_scanned_after_the_spi_loader`, `a_non_descendant_loader_keeps_the_factory_loaders_classes`, `the_sort_field_loader_scans_lucene_cores_service_file`, the 8 `reload_*_scans_the_given_loader`, `refused_factory_classes_leave_the_loader_unchanged`, `new_instance_and_find_spi_name_report_as_java_does` (13/13) |
| N2 | NamedSPILoader parent check `if true`; `new_factory_class_instance` never calls the constructor; explicit-version flag starts `true` | `a_descendant_loader_is_scanned_once_with_duplicates_skipped`, `the_default_loaders_hold_what_lucene_core_lists`, `a_leftover_argument_is_reported_as_the_map`, `new_instance_and_find_spi_name_report_as_java_does`, `the_constructor_reads_the_version_and_leaves_it_implicit` (5/5) |
| N3 | lucene-core TokenizerFactory line under another service; `get` returns `None`; scan dedup off; name check skipped; not-a-subtype message | `the_default_loaders_hold_what_lucene_core_lists`, `the_constructor_reads_the_version_and_leaves_it_implicit`, `a_descendant_loader_is_scanned_once_with_duplicates_skipped`, `reload_is_all_or_nothing_and_instantiates_what_it_reaches`, `a_provider_of_another_type_is_not_a_subtype` (5/5) |
| N4 | both loaders build in place (not on copies); `parseBoolean`, getChar length, requireChar message, parseInt message, float suffix, getSet separators, equalsIgnoreCase, getClassArg, `make_stop_set` empty, `correct_offset_of` identity, analyzer source does not re-apply the length, base set modifiable, full-mapping uppercase, buffersize message, LowerCaseFilter via `to_lowercase`, hash over code points, `contains_key` false, GraphTokenFilter.end skips the position increment, no-arg ctor succeeds, leftovers `k:v`, findSPIName message | 23/23: `reload_is_all_or_nothing…`, `refused_factory_classes…`, `boolean_arguments…`, `get_char_counts_utf16_units`, `require_char_of_an_empty_value…`, `int_arguments…`, `float_arguments…`, `set_arguments…`, `allowed_values_compare_with_equals_ignore_case`, `get_class_arg…`, `get_word_set_adds_each_files_stop_set`, `offsets_are_corrected_through_the_char_filters`, `the_analyzer_applies_its_max_token_length_on_every_reader`, `the_analyzer_keeps_an_unmodifiable_copy_of_its_stop_words`, `case_mappings_are_javas_simple_mappings`, `buffer_errors_carry_javas_messages`, `the_filter_lower_cases_with_the_simple_mapping`, `iteration_follows_javas_slot_order`, `lookups_lower_case_each_code_point`, `end_sets_the_recorded_trailing_state_again`, `the_no_argument_constructor_throws_the_default_ctor_exception`, `a_leftover_argument_is_reported_as_the_map`, `new_instance_and_find_spi_name…` |
| N5 | `CharArrayMap::put` does not lower-case; `new_filtering_token_filter` panics (callee mutation for the `new_stop_filter` wiring) | `iteration_follows_javas_slot_order`, `lookups_lower_case_each_code_point`, `analysis::tests::stop_filter_removes_configured_words` (3/3) |

No control for: the W= rows (no change), the doc-claim-only P rows, and `CharArrayMap::get_hash_code`'s ignore-case branch separately from `put` (N4/N5 cover the hash and the stored lower-casing; the ignore-case equality is exercised by `lookups_lower_case_each_code_point`).

## Declared divergences

New (each at its item): `ServiceClass` has no bootstrap (`null`) loader; `ServiceLoader`'s "Provider … not found", public-no-arg-constructor check and named-module providers have no counterpart (a provider line carries its class object); `AnalysisSPILoader` keeps names in registration order where Java's `Set.copyOf` order is unspecified, and reports `ServiceConfigurationError` as `IllegalArgument` (pre-existing stand-in, now declared on the type); `new_factory_class_instance` drops the `NoSuchMethodException` cause and does not wrap checked exceptions; `require_char` reports `StringIndexOutOfBoundsException` as `LuceneError::Other` and returns the whole first character where Java returns a UTF-16 unit; `StandardTokenizerFactory` renders leftovers in the order of a `HashMap` holding them alone (filled in key order); `StandardAnalyzer::create_components` leaves an out-of-range length's error to the source, raised within the same `token_stream` call; `CharArrayMap::put_chars` lower-cases its own copy where Java lower-cases the caller's array.

Removed: spi.rs module divergence (providers taken explicitly instead of a `ClassLoader`); `AnalysisSPILoader` "registered explicitly" divergence, and those of `new_factory_class_instance` (caller names the constructor) and `lookup_spi_name` (no reflection); the "Rust has no run-time service discovery, so the caller supplies the …" divergences of the 8 `reload_*` entry points; `LowerCaseFilter`'s and `character_utils::to_lower_case`/`to_upper_case`'s full-mapping divergences.

## Second list (7984811)

Input: the 14 rows of `list_7984811.jsonl` absent from `list_0921226.jsonl`, all created by passes H1-H2; checked against HEAD `7984811` plus pass I. Java citations as above; `codecs/` and `util/` paths are relative to `/tmp/lucene1050/lucene/core/src/java/org/apache/lucene/`. Oracle: `scratchpad/oracle/passI/analysis/AnalysisOracle.java` (`out.txt`).

Counts: auto D? 13, W? 1. Final: D 7, P 2, T 3, W 1, W-blocked 1, W= 0 (pass S reclassified the two `ServiceProvider` constructors from W= to T).

| item | auto | final | action | Java evidence |
|---|---|---|---|---|
| `analysis::factories::AnalysisFactoryClass::spi_name_field` | D? | **W** | wired: `AnalysisSPILoader::lookup_spi_name` reads the class's `NAME` field model through it (it matched the private field); doc names `Class.getField("NAME")`. Control I-C7: the accessor answering `Missing` fails 6 `spi_tests` | AnalysisSPILoader.java:172-185 `lookupSPIName` (`service.getField("NAME")`); callers :88 `reload`, `TokenizerFactory/TokenFilterFactory/CharFilterFactory.findSPIName` |
| `analysis::graph_token_filter::GraphTokenFilter::end` | W? | **W-blocked** | blocked, as `GraphTokenFilter::reset` (row above): Java's callers reach it through `TokenStream.end()` dispatch on a subclass; the Rust `GraphTokenFilter` is not a `TokenStream` and lucene-core has no subclass (only analysis-common's `FixedShingleFilter` and the test `TestGraphTokenFilter` extend it). Its test `end_sets_the_recorded_trailing_state_again` calls it. No open task ports a subclass | GraphTokenFilter.java:151-162; Java callers TokenFilter.end, FilteringTokenFilter.end, CachingTokenFilter.fillCache, IndexingChain$PerField.invertTokenStream (through TokenStream.end) |
| `analysis::tokenattributes::PackedTokenAttributeImpl::term` | D? | **P** | doc names the Java member: `toString()`, inherited from `CharTermAttributeImpl`; used by the analysis tests | tokenattributes/CharTermAttributeImpl.java:252-254 `toString()` |
| `codecs::CodecRegistry::new` | D? | **T** | none: builds the empty registry four `codecs.rs` unit tests register into | no Lucene member: `Codec.Holder.LOADER` is `new NamedSPILoader<>(Codec.class)`, which scans (Codec.java:44-60; util/NamedSPILoader.java:37-63); no empty loader |
| `codecs::CodecRegistry::register_arc` | D? | **D** | removed | no Lucene member: `NamedSPILoader` has `reload(ClassLoader)`, `checkServiceName`, `lookup`, `availableServices`, `iterator` (util/NamedSPILoader.java:65-127), no explicit registration; reload is ported through the class-loader scan |
| `codecs::doc_values::DocValuesFormatRegistry::new` | D? | **D** | removed (no caller, tests included) | as `CodecRegistry::new` (DocValuesFormat.java Holder) |
| `codecs::doc_values::DocValuesFormatRegistry::register_arc` | D? | **D** | removed | as `CodecRegistry::register_arc` |
| `codecs::knn_vectors::KnnVectorsFormatRegistry::new` | D? | **D** | removed (no caller, tests included) | as `CodecRegistry::new` (KnnVectorsFormat.java:45-58 Holder) |
| `codecs::knn_vectors::KnnVectorsFormatRegistry::register_arc` | D? | **D** | removed | as `CodecRegistry::register_arc` |
| `codecs::postings::PostingsFormatRegistry::new` | D? | **D** | removed (no caller, tests included) | as `CodecRegistry::new` (PostingsFormat.java Holder) |
| `codecs::postings::PostingsFormatRegistry::register_arc` | D? | **D** | removed | as `CodecRegistry::register_arc` |
| `util::resource_loader::ServiceProvider::not_found` | D? | **T** | none (reclassified in pass S from W=): a test-only constructor of the declared stand-in `ServiceProvider` (a `META-INF/services` line with its class object, a divergence declared at `ClassLoader`); it builds a line whose class the loader cannot find, which `load_service_classes` refuses as Java's `ServiceLoader` (spi.rs:156-160, `<service>: Provider <class> not found`, oracle AuditHOracle); only the SPI tests call it, and lucene-core's service files have no such line | no Lucene member: `ServiceLoader.LazyClassPathLookupIterator` fails on the class name it reads (`fail(service, "Provider " + cn + " not found")`); there is no line object to construct |
| `util::resource_loader::ServiceProvider::service` | D? | **P** | none: the declared stand-in's accessor of the service file's name (`ClassLoader::get_resources` matches the field), as block 3 kept `ClassRegistry`'s | the `META-INF/services/<service>` file name `ServiceLoader` opens through `ClassLoader.getResources` |
| `util::resource_loader::ServiceProvider::without_public_no_arg_constructor` | D? | **T** | none (reclassified in pass S from W=): as `not_found`, a test-only constructor of the stand-in; `load_service_classes` refuses its line (spi.rs:166-171, `<service>: <class> Unable to get public no-arg constructor`, cause `NoSuchMethodException` `<class>.<init>()`, now carried as `LuceneError::NoSuchMethod`) | no Lucene member: `ServiceLoader.getConstructor` fails on the class (`fail(service, clazz + " Unable to get public no-arg constructor", e)`); there is no line object to construct |

Pass I also closed two declared divergences of block 4: `require_char` now returns `LuceneError::StringIndexOutOfBounds` (new variant, `java_simple_name` `StringIndexOutOfBoundsException`, message `Index 0 out of bounds for length 0` — AnalysisOracle); `new_factory_class_instance` returns `LuceneError::UnsupportedOperationWithCause` (new variant, modelled on `IllegalArgumentWithCause`) whose cause is the `NoSuchMethodException` `<class>.<init>(java.util.Map)` as `LuceneError::NoSuchMethod` (new variant in audit I, `java_simple_name` `NoSuchMethodException`; AnalysisOracle: `java.lang.NoSuchMethodException: AnalysisOracle$NoMapCtor.<init>(java.util.Map)`). Controls I-C5 and I-C6.

## Noticed, not acted on (outside scope)

- Rust `PackedTokenAttributeImpl::end` only zeroes the position increment; Java's `AttributeImpl.end()` is `clear()` first (MiscOracle O6: term `''` after `end()`), and Rust `Tokenizer::end` runs the logic before `end_attributes` where Java's subclasses call `super.end()` first.
- `Analyzer::token_stream_from_str` builds a new `ReusableStringReader` per call; Java reuses `components.reusableStringReader` (Analyzer.java:183-199).
- The global codec registry is not seeded with `Lucene104Codec` (Java's `Codec.forName("Lucene104")` resolves through lucene-core's service file); the codec/format registries are not built from `lucene_core_class_loader`. `CodecRegistry::register_arc` now has no caller.
- `AbstractAnalysisFactory`'s `get(Map, String, Collection[, String[, boolean]])` and `require(Map, String, Collection)` overloads have no port.

## Gate (pass H2)

Target dir `.rucene-dev-target`, memory-capped runner set.
- `cargo build --all-targets`: 0 warnings. `cargo fmt --all -- --check`: clean.
- `cargo clippy --all-targets --message-format=json`: 134 unique (code, file, line), baseline 187; none on a changed line of `git diff -U0 HEAD` or the untracked files (four on new lines were fixed: one `map_entry`, three `type_complexity` via `AnalysisFactoryConstructor<T>`).
- Lib tests of the changed modules (`analysis::`, `util::spi`, `util::resource_loader`, `util::extra`, codec registries, `index::index_sorter`, `segment_info`): 246 passed, 0 failed.
- 27 integration/portability targets covering the change (analysis users, index-sort users of the sort-field loader, codec-registry users): 255 passed, 0 failed.
- `fuzz_*` targets using the codec/format registries: `fuzz_doc_values` 12, `fuzz_norms` 8, `fuzz_points` 9, `fuzz_stored_fields` 4, `fuzz_term_vectors` 5 — 38 passed, 0 failed.
