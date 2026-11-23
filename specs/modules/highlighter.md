# Module Specification: `highlighter`

**Spec ID:** HIGHLIGHTER  
**Java module:** `lucene/highlighter` (packages `org.apache.lucene.search.{highlight,vectorhighlight,uhighlight,matchhighlight}`)  
**Crate:** `crates/rucene-highlighter`

## 1. Purpose

Provide the "keyword in context" features of Lucene 10.5.0, used to highlight search terms in the text of results pages: four highlighters producing the fragments or passages Lucene produces for the same index, query and text.

## 2. Key classes / concepts ported

- `highlight` — `Highlighter` with `Fragmenter` (`SimpleFragmenter`, `SimpleSpanFragmenter`, `NullFragmenter`), fragment `Scorer` (`QueryScorer`, `QueryTermScorer`), `Formatter` (`SimpleHTMLFormatter`, `GradientFormatter`, `SpanGradientFormatter`) and `Encoder`; `WeightedSpanTermExtractor`; `TokenSources` and `TokenStreamFromTermVector`; `OffsetLimitTokenFilter`.
- `vectorhighlight` — `FastVectorHighlighter` over term vectors with positions and offsets: `FieldQuery`, `FieldTermStack`, `FieldPhraseList`, the `FragListBuilder`s, the `FragmentsBuilder`s and the `BoundaryScanner`s (`SimpleBoundaryScanner`, `BreakIteratorBoundaryScanner`).
- `uhighlight` — `UnifiedHighlighter`, with offsets from postings, term vectors or re-analysis (`FieldOffsetStrategy` and its implementations), `PhraseHelper`, `Passage`, `PassageScorer`, `DefaultPassageFormatter` and the break iterators (`CustomSeparatorBreakIterator`, `WholeBreakIterator`, `SplittingBreakIterator`, `LengthGoalBreakIterator`).
- `matchhighlight` — `MatchHighlighter` on the `Matches` API: `MatchRegionRetriever`, `OffsetsRetrievalStrategy` implementations, `PassageSelector`, `PassageFormatter`, `FieldValueHighlighters`.

## 3. Design notes

- One crate module per Java package and per Java source file (`UnifiedHighlighter.java` is `uhighlight::unified_highlighter`).
- Offsets of tokens, passages and fragments count UTF-16 units, and the text they slice is held as UTF-16, as in Java.
- `java.text.BreakIterator` (word, line and sentence instances, root and Thai) comes from `rucene-analysis-common`'s JDK 25 emulation (`jdk::break_iterator`); the line tables are built by an emulation of the JDK's `RuleBasedBreakIteratorBuilder`.
- Java subclass hooks (`newTermExtractor`, `extractUnknownQuery`, `flatten`, …) are override traits installed on the instance.

## 4. Acceptance criteria

- The Lucene 10.5.0 test classes under `lucene/highlighter/src/test` are ported in `crates/rucene-highlighter/tests/` and pass.
- The fragments/passages oracle (`highlighter_fragments_oracle.rs`) gives hits, score bits and the output of `Highlighter`, `FastVectorHighlighter`, `UnifiedHighlighter` and `MatchHighlighter` equal to Lucene 10.5.0 on a seeded corpus.
