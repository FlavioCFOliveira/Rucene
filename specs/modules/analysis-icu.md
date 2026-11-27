# Module Specification: `analysis-icu`

**Spec ID:** ANALYSIS-ICU  
**Java module:** `lucene/analysis/icu` (packages `org.apache.lucene.analysis.icu`, `.segmentation`, `.tokenattributes`)  
**Crate:** `crates/rucene-analysis-icu` (over `crates/icu4j`, see `specs/modules/icu4j.md`)

## 1. Purpose

Provide Lucene 10.5.0's analysis integration with ICU: Unicode normalization and folding, script-aware word segmentation, transliteration and collation keys, with the same tokens, offsets, types and key bytes as Lucene over ICU4J 77.1.

## 2. Key classes / concepts ported

- Normalization: `ICUNormalizer2Filter`, `ICUNormalizer2CharFilter`, `ICUFoldingFilter` (`utr30.nrm`) and their factories.
- Transforms: `ICUTransformFilter` (with the rule-based source-set filter optimization) and its factory.
- Segmentation: `ICUTokenizer`, `ICUTokenizerConfig`, `DefaultICUTokenizerConfig` (`Default.brk`, `MyanmarSyllable.brk`), `CompositeBreakIterator`, `BreakIteratorWrapper`, `ScriptIterator`, `CharArrayIterator`, `ICUTokenizerFactory` (per-script `rulefiles`).
- Attributes: `ScriptAttribute`, `ScriptAttributeImpl`.
- Collation: `ICUCollationKeyAnalyzer`, `ICUCollationAttributeFactory`, `ICUCollatedTermAttributeImpl`, `ICUCollationDocValuesField`.
- The factory registry of `META-INF/services` (`spi`), the three data files byte for byte (`resources`), and the two tools of `src/tools` as binaries (`rbbi_rule_compiler`, `generate_utr30_data_files`).

## 3. Design notes

- One crate module per Java source file; the module tree mirrors the Java packages.
- `ICUTokenizerConfig` is a trait shared as `Arc<dyn ICUTokenizerConfig>`; `getType` returns `Cow<'static, str>` so the `StandardTokenizer.TOKEN_TYPES` constants keep their identity: `DefaultICUTokenizerConfig`'s `WORD_*` statics are references to the elements of core's `TOKEN_TYPES` (read at run time), which `CJKBigramFilter`'s identity test requires.
- Break iterators share the tokenizer buffer as `Arc<[u16]>` snapshots taken at every `setText`; `ReplaceableTermAttribute` edits a UTF-16 copy of the term (declared in the crate docs).
- `generate_utr30_data_files` downloads over `ureq` with the request `HttpURLConnection` sends (headers, redirects, JDK exception texts); declared divergences are in its module docs.
- `NewCollationAnalyzerTask` of `rucene-benchmark` builds `ICUCollationKeyAnalyzer` for `impl:icu`.

## 4. Acceptance criteria

- The 16 test classes of `lucene/analysis/icu/src/test` (with the three `.rbbi` resources) are ported in `crates/rucene-analysis-icu/tests/` (`icu_normalizer_transform_tests.rs`, `icu_collation_tests.rs`, `icu_segmentation_tests.rs`) and pass; `TestICUTokenizerCJK` keeps Lucene's class-level `@AwaitsFix`.
- The oracle `icu_oracle.rs` (`AnalysisIcuOracle.java`, `Module::AnalysisIcu`) gives token streams equal to Lucene 10.5.0 for every tokenizer configuration, filter, normalizer, char filter and 14 transliterator IDs over a seeded multilingual corpus, and equal `ICUCollationKeyAnalyzer` keys for 17 locales and 3 strengths.
- The benchmark oracle `benchmark_collation_icu_oracle` gives the same report and keys as Lucene for `NewCollationAnalyzer(impl:icu)`.
