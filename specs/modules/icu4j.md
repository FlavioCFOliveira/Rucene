# Module Specification: `icu4j`

**Spec ID:** ICU4J  
**Upstream:** ICU4J 77.1 (`com.ibm.icu:icu4j:77.1`), the transitive closure `lucene/analysis/icu` and `lucene/benchmark` use  
**Crate:** `crates/icu4j`

## 1. Purpose

Provide the ICU4J 77.1 classes Lucene 10.5.0 `lucene/analysis/icu` and `lucene/benchmark` call, with ICU4J's results for the same inputs and data.

## 2. Key classes / concepts ported

- Foundations: `ICUBinary`/`ICUData` data loading, `Trie2`, `CodePointTrie`, `BytesTrie`/`CharsTrie` and builders, `UTF16`, `Utility`, caches.
- Resource bundles, `ULocale`, `ICUService`, locale display names and language matching.
- Character properties (`UCharacter`, `UScript`, `UProperty`, case mapping) and `UnicodeSet`.
- Normalization (`Normalizer2`, `FilteredNormalizer2`, `Normalizer`).
- Break iteration: `RuleBasedBreakIterator`, the dictionary engines, the RBBI rule compiler.
- Collation: `Collator`, `RuleBasedCollator`, the collation builder, `CollationKey`, `RawCollationKey`.
- Transliteration: `Transliterator`, the registry, rule-based and algorithmic transliterators.
- Number formatting: the number pipeline, `DecimalFormat`, `RuleBasedNumberFormat`, `PluralFormat`, `Currency`, `CurrencyMetaInfo`, with the `java.math` and `java.text` emulations they need.
- The ICU data files the closure reads (233 files from `icu4j-77.1.jar`, embedded unmodified under `data/`).

## 3. Design notes

- Java text is UTF-16 code units: `&[u16]` parameters, `Vec<u16>` results, `&mut Vec<u16>` for `StringBuilder`/`Appendable`.
- Explicit ICU `throw`s, documented JDK throws and the unchecked exceptions of the resource data are `Result<_, LuceneError>` errors (rmp #643; `CurrencyMetaInfo` and `NumberFormat::set_context` follow it since rmp #642); the `IndexOutOfBoundsException` family and implicit `NullPointerException`s are panics with the Java text.
- Java `int` arithmetic that wraps is ported with wrapping operations (such as `BMPSet.set32x64Bits`).
- Class-path lookups resolve over a sorted embedded table, as `Class.getResourceAsStream` does.
- Divergences are declared at the point of divergence in the crate docs.

## 4. Acceptance criteria

- The `rucene-analysis-icu` suites and oracles (see `specs/modules/analysis-icu.md`) pass over this crate.
- The benchmark oracle of `NewCollationAnalyzer(impl:icu)` (`benchmark_collation_icu_oracle`) equals Lucene 10.5.0 over ICU4J 77.1.
