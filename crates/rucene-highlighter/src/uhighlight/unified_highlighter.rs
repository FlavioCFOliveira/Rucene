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

//! Port of `UnifiedHighlighter.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};

use rucene::analysis::Analyzer;
use rucene::error::{LuceneError, Result};
use rucene::index::base_composite_reader::BaseCompositeReader;
use rucene::index::filter_reader::FilterLeafReader;
use rucene::index::index_reader::{
    build_composite_context, CacheHelper, CompositeReader, IndexReaderCore, StoredFields,
};
use rucene::index::multi_reader::reader_util::sub_index_from_leaves;
use rucene::index::{
    FieldInfo, FieldInfos, Fields, IndexOptions, IndexReader, IndexReaderContext, LeafReader,
    StoredFieldVisitor, StoredFieldVisitorStatus, Term, TermVectors, Terms,
};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::match_no_docs_query::MatchNoDocsQuery;
use rucene::search::query::Query;
use rucene::search::query_visitor::{term_collector, QueryVisitor};
use rucene::search::TopDocs;
use rucene::util::BytesRef;
use rucene_analysis_common::jdk::break_iterator::{get_sentence_instance, BreakIterator};
use rucene_analysis_common::jdk::locale::Locale;
use rucene_queries::spans::span_query::SpanQuery;

use crate::uhighlight::default_passage_formatter::DefaultPassageFormatter;
use crate::uhighlight::field_highlighter::FieldHighlighter;
use crate::uhighlight::field_offset_strategy::FieldOffsetStrategy;
use crate::uhighlight::labelled_char_array_matcher::LabelledCharArrayMatcher;
use crate::uhighlight::memory_index_offset_strategy::MemoryIndexOffsetStrategy;
use crate::uhighlight::multi_fields_offset_strategy::MultiFieldsOffsetStrategy;
use crate::uhighlight::multi_term_highlighting::MultiTermHighlighting;
use crate::uhighlight::no_op_offset_strategy::NoOpOffsetStrategy;
use crate::uhighlight::passage::Passage;
use crate::uhighlight::passage_formatter::{FormattedPassages, PassageFormatter};
use crate::uhighlight::passage_scorer::PassageScorer;
use crate::uhighlight::phrase_helper::PhraseHelper;
use crate::uhighlight::postings_offset_strategy::PostingsOffsetStrategy;
use crate::uhighlight::postings_with_term_vectors_offset_strategy::PostingsWithTermVectorsOffsetStrategy;
use crate::uhighlight::splitting_break_iterator::SplittingBreakIterator;
use crate::uhighlight::term_vector_offset_strategy::TermVectorOffsetStrategy;
use crate::uhighlight::token_stream_offset_strategy::TokenStreamOffsetStrategy;
use crate::uhighlight::uh_components::{FieldMatcher, UHComponents};

/// Equivalent to the protected static `MULTIVAL_SEP_CHAR`: the separator
/// of the values of a multi-valued field, `(char) 0`.
pub const MULTIVAL_SEP_CHAR: u16 = 0;

/// `Supplier<BreakIterator>`.
pub type BreakIteratorSupplier = Arc<dyn Fn() -> Box<dyn BreakIterator> + Send + Sync>;

/// `Comparator<Passage>`.
pub type PassageSortComparator = Arc<dyn Fn(&Passage, &Passage) -> Ordering + Send + Sync>;

/// `Function<String, Set<String>>`: the masked fields of a field, in the
/// set's iteration order; `None` for Java's `null`.
pub type MaskedFieldsFunc = Arc<dyn Fn(&str) -> Option<Vec<String>> + Send + Sync>;

const DEFAULT_ENABLE_MULTI_TERM_QUERY: bool = true;
const DEFAULT_ENABLE_HIGHLIGHT_PHRASES_STRICTLY: bool = true;
const DEFAULT_ENABLE_WEIGHT_MATCHES: bool = true;
const DEFAULT_ENABLE_RELEVANCY_OVER_SPEED: bool = true;
const DEFAULT_MAX_HIGHLIGHT_PASSAGES: i32 = -1;

fn default_break_iterator() -> BreakIteratorSupplier {
    Arc::new(|| get_sentence_instance(&Locale::of_parts("", "", "", "")))
}

fn default_passage_formatter() -> Arc<dyn PassageFormatter> {
    static FORMATTER: OnceLock<Arc<dyn PassageFormatter>> = OnceLock::new();
    Arc::clone(FORMATTER.get_or_init(|| Arc::new(DefaultPassageFormatter::new())))
}

fn default_passage_sort_comparator() -> PassageSortComparator {
    Arc::new(|a: &Passage, b: &Passage| a.get_start_offset().cmp(&b.get_start_offset()))
}

/// `String.compareTo`: the UTF-16 code unit order.
fn java_compare(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Source of term offsets; essential for highlighting.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.UnifiedHighlighter.OffsetSource`.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OffsetSource {
    /// Equivalent to `POSTINGS`.
    POSTINGS,
    /// Equivalent to `TERM_VECTORS`.
    TERM_VECTORS,
    /// Equivalent to `ANALYSIS`.
    ANALYSIS,
    /// Equivalent to `POSTINGS_WITH_TERM_VECTORS`.
    POSTINGS_WITH_TERM_VECTORS,
    /// Equivalent to `NONE_NEEDED`.
    NONE_NEEDED,
}

/// Flags for controlling highlighting behavior.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.UnifiedHighlighter.HighlightFlag`.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HighlightFlag {
    /// Equivalent to `PHRASES`.
    PHRASES,
    /// Equivalent to `MULTI_TERM_QUERY`.
    MULTI_TERM_QUERY,
    /// Equivalent to `PASSAGE_RELEVANCY_OVER_SPEED`.
    PASSAGE_RELEVANCY_OVER_SPEED,
    /// Equivalent to `WEIGHT_MATCHES`.
    WEIGHT_MATCHES,
}

/// Builder for UnifiedHighlighter. Use `UnifiedHighlighter::builder` or
/// `builder_without_searcher` to obtain one.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.UnifiedHighlighter.Builder`.
#[derive(Clone)]
pub struct Builder {
    /// If null, can only use highlightWithoutSearcher.
    searcher: Option<Arc<IndexSearcher>>,
    index_analyzer: Arc<dyn Analyzer>,
    field_matcher: Option<FieldMatcher>,
    masked_fields_func: Option<MaskedFieldsFunc>,
    flags: Option<HashSet<HighlightFlag>>,
    handle_multi_term_query: bool,
    highlight_phrases_strictly: bool,
    passage_relevancy_over_speed: bool,
    weight_matches: bool,
    max_length: i32,
    break_iterator: BreakIteratorSupplier,
    scorer: PassageScorer,
    formatter: Arc<dyn PassageFormatter>,
    max_no_highlight_passages: i32,
    cache_field_val_chars_threshold: i32,
    passage_sort_comparator: PassageSortComparator,
}

impl fmt::Debug for Builder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Builder")
            .field("flags", &self.flags)
            .field("max_length", &self.max_length)
            .finish_non_exhaustive()
    }
}

impl Builder {
    /// Constructor for UH builder which accepts `IndexSearcher` and
    /// `Analyzer` objects. `IndexSearcher` object can only be `None` when
    /// `UnifiedHighlighter::highlight_without_searcher` is used.
    ///
    /// Equivalent to `new Builder(IndexSearcher, Analyzer)`.
    pub fn new(searcher: Option<Arc<IndexSearcher>>, index_analyzer: Arc<dyn Analyzer>) -> Self {
        Self {
            searcher,
            index_analyzer,
            field_matcher: None,
            masked_fields_func: None,
            flags: None,
            handle_multi_term_query: DEFAULT_ENABLE_MULTI_TERM_QUERY,
            highlight_phrases_strictly: DEFAULT_ENABLE_HIGHLIGHT_PHRASES_STRICTLY,
            passage_relevancy_over_speed: DEFAULT_ENABLE_RELEVANCY_OVER_SPEED,
            weight_matches: DEFAULT_ENABLE_WEIGHT_MATCHES,
            max_length: UnifiedHighlighter::DEFAULT_MAX_LENGTH,
            break_iterator: default_break_iterator(),
            scorer: PassageScorer::new(),
            formatter: default_passage_formatter(),
            max_no_highlight_passages: DEFAULT_MAX_HIGHLIGHT_PASSAGES,
            cache_field_val_chars_threshold: UnifiedHighlighter::DEFAULT_CACHE_CHARS_THRESHOLD,
            passage_sort_comparator: default_passage_sort_comparator(),
        }
    }

    /// User-defined set of `HighlightFlag` values which will override the
    /// flags set by `with_highlight_phrases_strictly`,
    /// `with_handle_multi_term_query`, `with_passage_relevancy_over_speed`
    /// and `with_weight_matches`.
    ///
    /// Equivalent to `withFlags(Set)`.
    pub fn with_flags(mut self, values: HashSet<HighlightFlag>) -> Self {
        self.flags = Some(values);
        self
    }

    /// Here position sensitive queries (e.g. phrases and `SpanQuery`s) are
    /// highlighted strictly based on query matches (slower) versus any/all
    /// occurrences of the underlying terms. By default it's enabled, but
    /// there's no overhead if such queries aren't used.
    ///
    /// Equivalent to `withHighlightPhrasesStrictly(boolean)`.
    pub fn with_highlight_phrases_strictly(mut self, value: bool) -> Self {
        self.highlight_phrases_strictly = value;
        self
    }

    /// Here `MultiTermQuery` derivatives will be highlighted. By default
    /// it's enabled. MTQ highlighting can be expensive, particularly when
    /// using offsets in postings.
    ///
    /// Equivalent to `withHandleMultiTermQuery(boolean)`.
    pub fn with_handle_multi_term_query(mut self, value: bool) -> Self {
        self.handle_multi_term_query = value;
        self
    }

    /// Passage relevancy is more important than speed. True by default.
    ///
    /// Equivalent to `withPassageRelevancyOverSpeed(boolean)`.
    pub fn with_passage_relevancy_over_speed(mut self, value: bool) -> Self {
        self.passage_relevancy_over_speed = value;
        self
    }

    /// Internally use the `Weight.matches(LeafReaderContext, int)` API for
    /// highlighting. It's more accurate to the query, and the
    /// `HighlightFlag.PHRASES` and `HighlightFlag.MULTI_TERM_QUERY` options
    /// are effectively always enabled.
    ///
    /// Equivalent to `withWeightMatches(boolean)`.
    pub fn with_weight_matches(mut self, value: bool) -> Self {
        self.weight_matches = value;
        self
    }

    /// The text to be highlight is effectively truncated by this length.
    ///
    /// Equivalent to `withMaxLength(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when `value` is negative or
    /// `i32::MAX`.
    pub fn with_max_length(mut self, value: i32) -> Result<Self> {
        if value < 0 || value == i32::MAX {
            // two reasons: no overflow problems in BreakIterator.preceding(offset+1),
            // our sentinel in the offsets queue uses this value to terminate.
            return Err(LuceneError::illegal_argument(
                "maxLength must be < Integer.MAX_VALUE",
            ));
        }
        self.max_length = value;
        Ok(self)
    }

    /// Equivalent to `withBreakIterator(Supplier)`.
    pub fn with_break_iterator(mut self, value: BreakIteratorSupplier) -> Self {
        self.break_iterator = value;
        self
    }

    /// By default, `None`, meaning that only the field itself is accepted.
    ///
    /// Equivalent to `withFieldMatcher(Predicate)`.
    pub fn with_field_matcher(mut self, value: Option<FieldMatcher>) -> Self {
        self.field_matcher = value;
        self
    }

    /// Set up a function that given a field retuns a set of masked fields
    /// whose matches are combined to highlight the given field. Masked
    /// fields should not include the original field. This is useful when
    /// you want to highlight a field based on matches from several fields.
    ///
    /// Equivalent to `withMaskedFieldsFunc(Function)`.
    pub fn with_masked_fields_func(mut self, masked_fields_func: Option<MaskedFieldsFunc>) -> Self {
        self.masked_fields_func = masked_fields_func;
        self
    }

    /// Equivalent to `withScorer(PassageScorer)`.
    pub fn with_scorer(mut self, value: PassageScorer) -> Self {
        self.scorer = value;
        self
    }

    /// Equivalent to `withFormatter(PassageFormatter)`.
    pub fn with_formatter(mut self, value: Arc<dyn PassageFormatter>) -> Self {
        self.formatter = value;
        self
    }

    /// Equivalent to `withMaxNoHighlightPassages(int)`.
    pub fn with_max_no_highlight_passages(mut self, value: i32) -> Self {
        self.max_no_highlight_passages = value;
        self
    }

    /// Equivalent to `withCacheFieldValCharsThreshold(int)`.
    pub fn with_cache_field_val_chars_threshold(mut self, value: i32) -> Self {
        self.cache_field_val_chars_threshold = value;
        self
    }

    /// Set up a function that given a field returns the comparator that
    /// sorts the passages.
    ///
    /// Equivalent to `withPassageSortComparator(Comparator)`.
    pub fn with_passage_sort_comparator(mut self, value: PassageSortComparator) -> Self {
        self.passage_sort_comparator = value;
        self
    }

    /// Equivalent to `build()`.
    pub fn build(self) -> UnifiedHighlighter {
        UnifiedHighlighter::from_builder(self)
    }

    /// `None` if no searcher was provided.
    ///
    /// Equivalent to `getIndexSearcher()`.
    pub fn get_index_searcher(&self) -> Option<&Arc<IndexSearcher>> {
        self.searcher.as_ref()
    }

    /// Equivalent to `getIndexAnalyzer()`.
    pub fn get_index_analyzer(&self) -> &Arc<dyn Analyzer> {
        &self.index_analyzer
    }

    /// Equivalent to `getFlags()`.
    pub fn get_flags(&self) -> Option<&HashSet<HighlightFlag>> {
        self.flags.as_ref()
    }
}

/// The protected methods of `UnifiedHighlighter` a Java subclass overrides.
/// Each default is the base class's body, reachable as the highlighter's
/// `super_*` method.
#[allow(unused_variables)]
pub trait UnifiedHighlighterOverrides: Send + Sync {
    /// Equivalent to the deprecated protected `shouldHandleMultiTermQuery(String)`.
    fn should_handle_multi_term_query(&self, this: &UnifiedHighlighter, field: &str) -> bool {
        this.handle_multi_term_query
    }

    /// Equivalent to the deprecated protected `shouldHighlightPhrasesStrictly(String)`.
    fn should_highlight_phrases_strictly(&self, this: &UnifiedHighlighter, field: &str) -> bool {
        this.highlight_phrases_strictly
    }

    /// Equivalent to the deprecated protected `shouldPreferPassageRelevancyOverSpeed(String)`.
    fn should_prefer_passage_relevancy_over_speed(
        &self,
        this: &UnifiedHighlighter,
        field: &str,
    ) -> bool {
        this.passage_relevancy_over_speed
    }

    /// Equivalent to the protected `getFieldMatcher(String)`.
    fn get_field_matcher(&self, this: &UnifiedHighlighter, field: &str) -> FieldMatcher {
        this.super_get_field_matcher(field)
    }

    /// Equivalent to the protected `getMaskedFields(String)`.
    fn get_masked_fields(&self, this: &UnifiedHighlighter, field: &str) -> Option<Vec<String>> {
        this.super_get_masked_fields(field)
    }

    /// Equivalent to the protected `getFlags(String)`.
    fn get_flags(&self, this: &UnifiedHighlighter, field: &str) -> HashSet<HighlightFlag> {
        this.super_get_flags(field)
    }

    /// Equivalent to the protected `getBreakIterator(String)`.
    fn get_break_iterator(&self, this: &UnifiedHighlighter, field: &str) -> Box<dyn BreakIterator> {
        (this.break_iterator)()
    }

    /// Equivalent to the protected `getScorer(String)`.
    fn get_scorer(&self, this: &UnifiedHighlighter, field: &str) -> PassageScorer {
        this.scorer.clone()
    }

    /// Equivalent to the protected `getFormatter(String)`.
    fn get_formatter(&self, this: &UnifiedHighlighter, field: &str) -> Arc<dyn PassageFormatter> {
        Arc::clone(&this.formatter)
    }

    /// Equivalent to the protected `getPassageSortComparator(String)`.
    fn get_passage_sort_comparator(
        &self,
        this: &UnifiedHighlighter,
        field: &str,
    ) -> PassageSortComparator {
        Arc::clone(&this.passage_sort_comparator)
    }

    /// Equivalent to the protected `getMaxNoHighlightPassages(String)`.
    fn get_max_no_highlight_passages(&self, this: &UnifiedHighlighter, field: &str) -> i32 {
        this.max_no_highlight_passages
    }

    /// Equivalent to the protected `getOffsetSource(String)`.
    ///
    /// # Errors
    ///
    /// What reading the field infos raises.
    fn get_offset_source(&self, this: &UnifiedHighlighter, field: &str) -> Result<OffsetSource> {
        this.super_get_offset_source(field)
    }

    /// Equivalent to the protected `getFieldInfo(String)`.
    ///
    /// # Errors
    ///
    /// What reading the field infos raises.
    fn get_field_info(&self, this: &UnifiedHighlighter, field: &str) -> Result<Option<FieldInfo>> {
        this.super_get_field_info(field)
    }

    /// Equivalent to the protected `getFieldHighlighter(String, Query, Set,
    /// int)`.
    ///
    /// # Errors
    ///
    /// What building the strategy raises.
    fn get_field_highlighter(
        &self,
        this: &UnifiedHighlighter,
        field: &str,
        query: &Arc<dyn Query>,
        all_terms: &HashSet<Term>,
        max_passages: i32,
    ) -> Result<FieldHighlighter> {
        this.super_get_field_highlighter(field, query, all_terms, max_passages)
    }

    /// Equivalent to the protected `newFieldHighlighter(String,
    /// FieldOffsetStrategy, BreakIterator, PassageScorer, int, int,
    /// PassageFormatter, Comparator)`.
    #[allow(clippy::too_many_arguments)]
    fn new_field_highlighter(
        &self,
        this: &UnifiedHighlighter,
        field: &str,
        field_offset_strategy: Box<dyn FieldOffsetStrategy>,
        break_iterator: Box<dyn BreakIterator>,
        passage_scorer: PassageScorer,
        max_passages: i32,
        max_no_highlight_passages: i32,
        passage_formatter: Arc<dyn PassageFormatter>,
        passage_sort_comparator: PassageSortComparator,
    ) -> FieldHighlighter {
        FieldHighlighter::new(
            field,
            field_offset_strategy,
            break_iterator,
            passage_scorer,
            max_passages,
            max_no_highlight_passages,
            passage_formatter,
            passage_sort_comparator,
        )
    }

    /// Equivalent to the protected `getHighlightComponents(String, Query,
    /// Set)`.
    ///
    /// # Errors
    ///
    /// What building the phrase helper raises.
    fn get_highlight_components(
        &self,
        this: &UnifiedHighlighter,
        field: &str,
        query: &Arc<dyn Query>,
        all_terms: &HashSet<Term>,
    ) -> Result<UHComponents> {
        this.super_get_highlight_components(field, query, all_terms)
    }

    /// Equivalent to the protected `hasUnrecognizedQuery(Predicate, Query)`.
    fn has_unrecognized_query(
        &self,
        this: &UnifiedHighlighter,
        field_matcher: &FieldMatcher,
        query: &dyn Query,
    ) -> bool {
        this.super_has_unrecognized_query(field_matcher, query)
    }

    /// Equivalent to the protected `getPhraseHelper(String, Query, Set)`.
    ///
    /// # Errors
    ///
    /// What building the phrase helper raises.
    fn get_phrase_helper(
        &self,
        this: &UnifiedHighlighter,
        field: &str,
        query: &Arc<dyn Query>,
        highlight_flags: &HashSet<HighlightFlag>,
    ) -> Result<Arc<PhraseHelper>> {
        this.super_get_phrase_helper(field, query, highlight_flags)
    }

    /// Equivalent to the protected `getAutomata(String, Query, Set)`.
    fn get_automata(
        &self,
        this: &UnifiedHighlighter,
        field: &str,
        query: &dyn Query,
        highlight_flags: &HashSet<HighlightFlag>,
    ) -> Vec<Arc<dyn LabelledCharArrayMatcher>> {
        this.super_get_automata(field, query, highlight_flags)
    }

    /// Equivalent to the protected `getOptimizedOffsetSource(UHComponents)`.
    ///
    /// # Errors
    ///
    /// What reading the field infos raises.
    fn get_optimized_offset_source(
        &self,
        this: &UnifiedHighlighter,
        components: &UHComponents,
    ) -> Result<OffsetSource> {
        this.super_get_optimized_offset_source(components)
    }

    /// Equivalent to the protected `getOffsetStrategy(OffsetSource,
    /// UHComponents)`.
    ///
    /// # Errors
    ///
    /// What building the strategy raises.
    fn get_offset_strategy(
        &self,
        this: &UnifiedHighlighter,
        offset_source: OffsetSource,
        components: UHComponents,
    ) -> Result<Box<dyn FieldOffsetStrategy>> {
        this.super_get_offset_strategy(offset_source, components)
    }

    /// When highlighting phrases accurately, we need to know which
    /// `SpanQuery`'s need to have `Query.rewrite` called on them. It helps
    /// performance to avoid it if it's not needed. This method will be
    /// invoked on all SpanQuery instances recursively. If you have custom
    /// SpanQuery queries then override this to check instanceof and provide
    /// a definitive answer. If the query isn't your custom one, simply
    /// return `None` to have the default rules apply, which govern the ones
    /// included in Lucene.
    ///
    /// Equivalent to the protected `requiresRewrite(SpanQuery)`.
    fn requires_rewrite(&self, span_query: &dyn SpanQuery) -> Option<bool> {
        None
    }

    /// When highlighting phrases accurately, we may need to handle custom
    /// queries that aren't supported in the `WeightedSpanTermExtractor` as
    /// called by the `PhraseHelper`. Should custom query types be needed,
    /// this method should be overriden to return a collection of queries if
    /// appropriate, or `None` if nothing to do. If the query is not custom,
    /// simply returning `None` will allow the default rules to apply.
    ///
    /// Equivalent to the protected `preSpanQueryRewrite(Query)`.
    fn pre_span_query_rewrite(&self, query: &Arc<dyn Query>) -> Option<Vec<Arc<dyn Query>>> {
        None
    }

    /// Loads the String values for each docId by field to be highlighted.
    /// By default this loads from stored fields by the same name as given,
    /// but a subclass can change the source. The returned strings must be
    /// identical to what was indexed (at least for postings or term-vectors
    /// offset sources). This method must load fields for at least one
    /// document from the given `DocIdSetIterator` but need not return all
    /// of them; by default the character lengths are summed and this method
    /// will return early when `cache_chars_threshold` is exceeded.
    ///
    /// Equivalent to the protected `loadFieldValues(String[],
    /// DocIdSetIterator, int)`: `doc_iter` yields the sorted doc ids.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    fn load_field_values(
        &self,
        this: &UnifiedHighlighter,
        fields: &[String],
        doc_iter: &mut SortedDocIdIterator<'_>,
        cache_chars_threshold: i32,
    ) -> Result<Vec<Vec<Option<Vec<u16>>>>> {
        this.super_load_field_values(fields, doc_iter, cache_chars_threshold)
    }

    /// Equivalent to the protected `newLimitedStoredFieldsVisitor(String[])`;
    /// a subclass may return a subclass of `LimitedStoredFieldVisitor`.
    fn new_limited_stored_fields_visitor(
        &self,
        this: &UnifiedHighlighter,
        fields: &[String],
    ) -> Box<dyn LimitedStoredFieldVisitorExt> {
        Box::new(this.super_new_limited_stored_fields_visitor(fields))
    }
}

/// A Highlighter that can get offsets from either postings
/// (`IndexOptions.DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS`), term vectors
/// (`FieldType.setStoreTermVectorOffsets(boolean)`), or via re-analyzing
/// text.
///
/// This highlighter treats the single original document as the whole
/// corpus, and then scores individual passages as if they were documents
/// in this corpus. It uses a `BreakIterator` to find passages in the text;
/// by default it breaks using a locale-independent sentence iterator.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.UnifiedHighlighter`;
/// a subclass's overrides are a [`UnifiedHighlighterOverrides`].
pub struct UnifiedHighlighter {
    /// Equivalent to the protected final field `searcher` (if `None`, can
    /// only use highlightWithoutSearcher).
    pub searcher: Option<Arc<IndexSearcher>>,
    /// Equivalent to the protected final field `indexAnalyzer`.
    pub index_analyzer: Arc<dyn Analyzer>,
    /// Equivalent to the protected volatile field `fieldInfos`, lazy
    /// initialized with double-check locking.
    field_infos: OnceLock<FieldInfos>,
    field_matcher: Option<FieldMatcher>,
    masked_fields_func: Option<MaskedFieldsFunc>,
    flags: Option<HashSet<HighlightFlag>>,

    // e.g. wildcards
    handle_multi_term_query: bool,
    // AKA "accuracy" or "query debugging"
    highlight_phrases_strictly: bool,
    weight_matches: bool,
    // For analysis, prefer MemoryIndexOffsetStrategy
    passage_relevancy_over_speed: bool,

    max_length: i32,

    break_iterator: BreakIteratorSupplier,

    scorer: PassageScorer,

    formatter: Arc<dyn PassageFormatter>,

    max_no_highlight_passages: i32,

    cache_field_val_chars_threshold: i32,

    passage_sort_comparator: PassageSortComparator,

    overrides: Option<Arc<dyn UnifiedHighlighterOverrides>>,
}

impl fmt::Debug for UnifiedHighlighter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnifiedHighlighter")
            .field("flags", &self.flags)
            .field("max_length", &self.max_length)
            .finish_non_exhaustive()
    }
}

/// Java's anonymous `DocIdSetIterator` over the sorted doc ids
/// (`asDocIdSetIterator(int[])`).
pub struct SortedDocIdIterator<'a> {
    sorted_doc_ids: &'a [i32],
    idx: i64,
}

impl SortedDocIdIterator<'_> {
    /// `docID()`.
    pub fn doc_id(&self) -> i32 {
        if self.idx < 0 || self.idx >= self.sorted_doc_ids.len() as i64 {
            return rucene::search::NO_MORE_DOCS;
        }
        self.sorted_doc_ids[self.idx as usize]
    }

    /// `nextDoc()`.
    pub fn next_doc(&mut self) -> i32 {
        self.idx += 1;
        self.doc_id()
    }

    /// `cost()`: the remaining docs.
    pub fn cost(&self) -> i64 {
        0.max(self.sorted_doc_ids.len() as i64 - (self.idx + 1))
    }
}

/// The stored-field visitor `loadFieldValues` uses: loads the values of the
/// requested fields up to a length limit, joining multiple values with the
/// separator.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.UnifiedHighlighter.LimitedStoredFieldVisitor`;
/// values are UTF-16 (Java's `CharSequence`).
#[derive(Debug)]
pub struct LimitedStoredFieldVisitor {
    /// Equivalent to the protected final field `fields` (sorted).
    pub fields: Vec<String>,
    /// Equivalent to the protected final field `valueSeparator`.
    pub value_separator: u16,
    /// Equivalent to the protected final field `maxLength`.
    pub max_length: i32,
    /// Equivalent to the protected field `values`.
    pub values: Vec<Option<Vec<u16>>>,
    /// Equivalent to the protected field `currentField`.
    pub current_field: i32,
}

impl LimitedStoredFieldVisitor {
    /// Equivalent to `new LimitedStoredFieldVisitor(String[], char, int)`.
    pub fn new(fields: Vec<String>, value_separator: u16, max_length: i32) -> Self {
        Self {
            fields,
            value_separator,
            max_length,
            values: Vec::new(),
            current_field: -1,
        }
    }

    /// Equivalent to the package-private `init()`.
    pub(crate) fn init(&mut self) {
        self.values = vec![None; self.fields.len()];
        self.current_field = -1;
    }

    /// Equivalent to the package-private `getValuesByField()`.
    pub(crate) fn get_values_by_field(&mut self) -> Vec<Option<Vec<u16>>> {
        std::mem::take(&mut self.values)
    }
}

impl StoredFieldVisitor for LimitedStoredFieldVisitor {
    fn string_field(&mut self, _field_info: &Arc<FieldInfo>, value: String) -> Result<()> {
        debug_assert!(self.current_field >= 0);
        let value: Vec<u16> = value.encode_utf16().collect();
        let max_length = self.max_length.max(0) as usize;
        let cur = &mut self.values[self.current_field as usize];
        let Some(cur_value) = cur else {
            // TODO add caching heuristic for multi-valued fields
            *cur = Some(value[..max_length.min(value.len())].to_vec()); // note: may return 'this'
            return Ok(());
        };
        let length_budget = self.max_length - cur_value.len() as i32;
        if length_budget <= 0 {
            return Ok(());
        }
        cur_value.push(self.value_separator);
        let take = ((length_budget - 1).max(0) as usize).min(value.len());
        cur_value.extend_from_slice(&value[..take]);
        Ok(())
    }

    fn needs_field(&mut self, field_info: &FieldInfo) -> Result<StoredFieldVisitorStatus> {
        self.current_field = match self
            .fields
            .binary_search_by(|f| java_compare(f, &field_info.name))
        {
            Ok(i) => i as i32,
            Err(i) => -(i as i32) - 1,
        };
        if self.current_field < 0 {
            return Ok(StoredFieldVisitorStatus::No);
        }
        if let Some(cur_val) = &self.values[self.current_field as usize] {
            if cur_val.len() as i64 >= self.max_length as i64 {
                return Ok(if self.fields.len() == 1 {
                    StoredFieldVisitorStatus::Stop
                } else {
                    StoredFieldVisitorStatus::No
                });
            }
        }
        Ok(StoredFieldVisitorStatus::Yes)
    }
}

/// A `LimitedStoredFieldVisitor` or a subclass of it: the visitor the
/// `newLimitedStoredFieldsVisitor` hook returns. A subclass overrides the
/// `StoredFieldVisitor` methods and hands out its base state, which
/// `loadFieldValues` drives through the package-private `init()` and
/// `getValuesByField()` (`UnifiedHighlighter.java:1419-1484`).
pub trait LimitedStoredFieldVisitorExt: StoredFieldVisitor + Send {
    /// The `LimitedStoredFieldVisitor` part of the visitor.
    fn limited(&mut self) -> &mut LimitedStoredFieldVisitor;
}

impl LimitedStoredFieldVisitorExt for LimitedStoredFieldVisitor {
    fn limited(&mut self) -> &mut LimitedStoredFieldVisitor {
        self
    }
}

/// The visitor of `hasUnrecognizedQuery`.
struct UnknownLeafVisitor<'a> {
    field_matcher: &'a FieldMatcher,
    has_unknown_leaf: bool,
}

impl QueryVisitor for UnknownLeafVisitor<'_> {
    fn accept_field(&self, field: &str) -> bool {
        !self.has_unknown_leaf && (self.field_matcher)(field)
    }

    fn visit_leaf(&mut self, query: &dyn Query) {
        // Recognize leaves that we can extract automata from, and
        // MatchAll/MatchNone, which have no terms
        if !MultiTermHighlighting::can_extract_automata_from_leaf_query(query) {
            let any = query.as_any();
            if !(any.is::<MatchAllDocsQuery>() || any.is::<MatchNoDocsQuery>()) {
                self.has_unknown_leaf = true;
            }
        }
    }
}

impl UnifiedHighlighter {
    /// Equivalent to `UnifiedHighlighter.DEFAULT_MAX_LENGTH`.
    pub const DEFAULT_MAX_LENGTH: i32 = 10000;
    /// Equivalent to `UnifiedHighlighter.DEFAULT_CACHE_CHARS_THRESHOLD`
    /// (~ 1 MB (2 byte chars)).
    pub const DEFAULT_CACHE_CHARS_THRESHOLD: i32 = 524288;

    /// Constructs the highlighter with the given index searcher and
    /// analyzer.
    ///
    /// Equivalent to the deprecated `new UnifiedHighlighter(IndexSearcher,
    /// Analyzer)`; `index_searcher` is `None` when only
    /// `highlight_without_searcher` is used.
    #[deprecated]
    pub fn new(
        index_searcher: Option<Arc<IndexSearcher>>,
        index_analyzer: Arc<dyn Analyzer>,
    ) -> Self {
        Self {
            searcher: index_searcher, // TODO: make non nullable
            index_analyzer,
            field_infos: OnceLock::new(),
            field_matcher: None,
            masked_fields_func: None,
            flags: None,
            handle_multi_term_query: DEFAULT_ENABLE_MULTI_TERM_QUERY,
            highlight_phrases_strictly: DEFAULT_ENABLE_HIGHLIGHT_PHRASES_STRICTLY,
            weight_matches: DEFAULT_ENABLE_WEIGHT_MATCHES,
            passage_relevancy_over_speed: DEFAULT_ENABLE_RELEVANCY_OVER_SPEED,
            max_length: Self::DEFAULT_MAX_LENGTH,
            break_iterator: default_break_iterator(),
            scorer: PassageScorer::new(),
            formatter: default_passage_formatter(),
            max_no_highlight_passages: DEFAULT_MAX_HIGHLIGHT_PASSAGES,
            cache_field_val_chars_threshold: Self::DEFAULT_CACHE_CHARS_THRESHOLD,
            passage_sort_comparator: default_passage_sort_comparator(),
            overrides: None,
        }
    }

    /// Constructs the highlighter with the given `Builder`.
    ///
    /// Equivalent to `new UnifiedHighlighter(Builder)`.
    pub fn from_builder(builder: Builder) -> Self {
        let mut this = Self {
            searcher: builder.searcher.clone(),
            index_analyzer: Arc::clone(&builder.index_analyzer),
            field_infos: OnceLock::new(),
            field_matcher: builder.field_matcher.clone(),
            masked_fields_func: builder.masked_fields_func.clone(),
            flags: None,
            handle_multi_term_query: DEFAULT_ENABLE_MULTI_TERM_QUERY,
            highlight_phrases_strictly: DEFAULT_ENABLE_HIGHLIGHT_PHRASES_STRICTLY,
            weight_matches: DEFAULT_ENABLE_WEIGHT_MATCHES,
            passage_relevancy_over_speed: DEFAULT_ENABLE_RELEVANCY_OVER_SPEED,
            max_length: builder.max_length,
            break_iterator: Arc::clone(&builder.break_iterator),
            scorer: builder.scorer.clone(),
            formatter: Arc::clone(&builder.formatter),
            max_no_highlight_passages: builder.max_no_highlight_passages,
            cache_field_val_chars_threshold: builder.cache_field_val_chars_threshold,
            passage_sort_comparator: Arc::clone(&builder.passage_sort_comparator),
            overrides: None,
        };
        this.flags = Some(this.evaluate_flags_from_builder(&builder));
        this
    }

    /// Creates a `Builder` object where `IndexSearcher` and `Analyzer` are
    /// not null.
    ///
    /// Equivalent to the static `builder(IndexSearcher, Analyzer)`.
    pub fn builder(searcher: Arc<IndexSearcher>, index_analyzer: Arc<dyn Analyzer>) -> Builder {
        Builder::new(Some(searcher), index_analyzer)
    }

    /// Creates a `Builder` object in which you can only use
    /// `highlight_without_searcher` for highlighting.
    ///
    /// Equivalent to the static `builderWithoutSearcher(Analyzer)`.
    pub fn builder_without_searcher(index_analyzer: Arc<dyn Analyzer>) -> Builder {
        Builder::new(None, index_analyzer)
    }

    /// Sets the overridden protected methods, as a Java subclass does.
    pub fn set_overrides(&mut self, overrides: Arc<dyn UnifiedHighlighterOverrides>) {
        self.overrides = Some(overrides);
    }

    /// Equivalent to the deprecated `setHandleMultiTermQuery(boolean)`.
    #[deprecated]
    pub fn set_handle_multi_term_query(&mut self, handle_mtq: bool) {
        self.handle_multi_term_query = handle_mtq;
    }

    /// Equivalent to the deprecated `setHighlightPhrasesStrictly(boolean)`.
    #[deprecated]
    pub fn set_highlight_phrases_strictly(&mut self, highlight_phrases_strictly: bool) {
        self.highlight_phrases_strictly = highlight_phrases_strictly;
    }

    /// Equivalent to the deprecated `setPassageRelevancyOverSpeed(boolean)`.
    #[deprecated]
    pub fn set_passage_relevancy_over_speed(&mut self, passage_relevancy_over_speed: bool) {
        self.passage_relevancy_over_speed = passage_relevancy_over_speed;
    }

    /// Equivalent to the deprecated `setMaxLength(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when `max_length` is negative or
    /// `i32::MAX`.
    #[deprecated]
    pub fn set_max_length(&mut self, max_length: i32) -> Result<()> {
        if max_length < 0 || max_length == i32::MAX {
            // two reasons: no overflow problems in BreakIterator.preceding(offset+1),
            // our sentinel in the offsets queue uses this value to terminate.
            return Err(LuceneError::illegal_argument(
                "maxLength must be < Integer.MAX_VALUE",
            ));
        }
        self.max_length = max_length;
        Ok(())
    }

    /// Equivalent to the deprecated `setBreakIterator(Supplier)`.
    #[deprecated]
    pub fn set_break_iterator(&mut self, break_iterator: BreakIteratorSupplier) {
        self.break_iterator = break_iterator;
    }

    /// Equivalent to the deprecated `setScorer(PassageScorer)`.
    #[deprecated]
    pub fn set_scorer(&mut self, scorer: PassageScorer) {
        self.scorer = scorer;
    }

    /// Equivalent to the deprecated `setFormatter(PassageFormatter)`.
    #[deprecated]
    pub fn set_formatter(&mut self, formatter: Arc<dyn PassageFormatter>) {
        self.formatter = formatter;
    }

    /// Equivalent to the deprecated `setMaxNoHighlightPassages(int)`.
    #[deprecated]
    pub fn set_max_no_highlight_passages(&mut self, default_max_no_highlight_passages: i32) {
        self.max_no_highlight_passages = default_max_no_highlight_passages;
    }

    /// Equivalent to the deprecated `setCacheFieldValCharsThreshold(int)`.
    #[deprecated]
    pub fn set_cache_field_val_chars_threshold(&mut self, cache_field_val_chars_threshold: i32) {
        self.cache_field_val_chars_threshold = cache_field_val_chars_threshold;
    }

    /// Equivalent to the deprecated `setFieldMatcher(Predicate)`.
    #[deprecated]
    pub fn set_field_matcher(&mut self, predicate: Option<FieldMatcher>) {
        self.field_matcher = predicate;
    }

    /// Equivalent to the deprecated `setWeightMatches(boolean)`.
    #[deprecated]
    pub fn set_weight_matches(&mut self, weight_matches: bool) {
        self.weight_matches = weight_matches;
    }

    /// Returns whether `MultiTermQuery` derivatives will be highlighted.
    ///
    /// Equivalent to the deprecated protected `shouldHandleMultiTermQuery(String)`.
    pub fn should_handle_multi_term_query(&self, field: &str) -> bool {
        match &self.overrides {
            Some(o) => o.should_handle_multi_term_query(self, field),
            None => self.handle_multi_term_query,
        }
    }

    /// Returns whether position sensitive queries (e.g. phrases and
    /// `SpanQuery`s) should be highlighted strictly.
    ///
    /// Equivalent to the deprecated protected `shouldHighlightPhrasesStrictly(String)`.
    pub fn should_highlight_phrases_strictly(&self, field: &str) -> bool {
        match &self.overrides {
            Some(o) => o.should_highlight_phrases_strictly(self, field),
            None => self.highlight_phrases_strictly,
        }
    }

    /// Equivalent to the deprecated protected
    /// `shouldPreferPassageRelevancyOverSpeed(String)`.
    pub fn should_prefer_passage_relevancy_over_speed(&self, field: &str) -> bool {
        match &self.overrides {
            Some(o) => o.should_prefer_passage_relevancy_over_speed(self, field),
            None => self.passage_relevancy_over_speed,
        }
    }

    /// Extracts matching terms.
    ///
    /// Equivalent to the protected static `extractTerms(Query)`.
    pub fn extract_terms(query: &dyn Query) -> HashSet<Term> {
        let mut query_terms: HashSet<Term> = HashSet::new();
        query.visit(&mut term_collector(&mut query_terms));
        query_terms
    }

    /// Returns the `HighlightFlag`s applicable for the current UH instance.
    ///
    /// Equivalent to the protected `evaluateFlags(boolean, boolean, boolean,
    /// boolean)`.
    pub fn evaluate_flags(
        &self,
        should_handle_multi_term_query: bool,
        should_highlight_phrases_strictly: bool,
        should_passage_relevancy_over_speed: bool,
        should_enable_weight_matches: bool,
    ) -> HashSet<HighlightFlag> {
        let mut highlight_flags: HashSet<HighlightFlag> = HashSet::new();
        if should_handle_multi_term_query {
            highlight_flags.insert(HighlightFlag::MULTI_TERM_QUERY);
        }
        if should_highlight_phrases_strictly {
            highlight_flags.insert(HighlightFlag::PHRASES);
        }
        if should_passage_relevancy_over_speed {
            highlight_flags.insert(HighlightFlag::PASSAGE_RELEVANCY_OVER_SPEED);
        }

        // Evaluate if WEIGHT_MATCHES can be added as a flag.
        let apply_weight_matches = highlight_flags.contains(&HighlightFlag::MULTI_TERM_QUERY)
            && highlight_flags.contains(&HighlightFlag::PHRASES)
            && highlight_flags.contains(&HighlightFlag::PASSAGE_RELEVANCY_OVER_SPEED)
            // User can also opt-out of WEIGHT_MATCHES.
            && should_enable_weight_matches;

        if apply_weight_matches {
            highlight_flags.insert(HighlightFlag::WEIGHT_MATCHES);
        }
        highlight_flags
    }

    /// Evaluate the highlight flags and set the `flags` variable. This is
    /// called only once when the Builder object is used to create a UH
    /// object.
    ///
    /// Equivalent to the protected `evaluateFlags(Builder)`.
    pub fn evaluate_flags_from_builder(&self, uh_builder: &Builder) -> HashSet<HighlightFlag> {
        if let Some(flags) = &self.flags {
            return flags.clone();
        }
        if let Some(flags) = &uh_builder.flags {
            // `this.flags = evaluateFlags(uhBuilder)` reads the builder's
            // flags through the constructor's assignment
            return flags.clone();
        }
        self.evaluate_flags(
            uh_builder.handle_multi_term_query,
            uh_builder.highlight_phrases_strictly,
            uh_builder.passage_relevancy_over_speed,
            uh_builder.weight_matches,
        )
    }

    /// Evaluate the highlight flags from the deprecated setters.
    ///
    /// Equivalent to the deprecated protected `evaluateFlags(UnifiedHighlighter)`.
    pub fn evaluate_flags_from_highlighter(
        &self,
        uh: &UnifiedHighlighter,
    ) -> HashSet<HighlightFlag> {
        self.evaluate_flags(
            uh.handle_multi_term_query,
            uh.highlight_phrases_strictly,
            uh.passage_relevancy_over_speed,
            uh.weight_matches,
        )
    }

    /// Returns the predicate to use for extracting the query part that
    /// must be highlighted. By default only queries that target the current
    /// field are kept. (AKA requireFieldMatch)
    ///
    /// Equivalent to the protected `getFieldMatcher(String)`.
    pub fn get_field_matcher(&self, field: &str) -> FieldMatcher {
        match &self.overrides {
            Some(o) => o.get_field_matcher(self, field),
            None => self.super_get_field_matcher(field),
        }
    }

    /// The base class's `getFieldMatcher(String)`.
    pub fn super_get_field_matcher(&self, field: &str) -> FieldMatcher {
        match &self.field_matcher {
            Some(field_matcher) => Arc::clone(field_matcher),
            None => {
                // requireFieldMatch = true
                let field = field.to_string();
                Arc::new(move |qf: &str| field == qf)
            }
        }
    }

    /// Returns the set of masked fields of `field`.
    ///
    /// Equivalent to the protected `getMaskedFields(String)`.
    pub fn get_masked_fields(&self, field: &str) -> Option<Vec<String>> {
        match &self.overrides {
            Some(o) => o.get_masked_fields(self, field),
            None => self.super_get_masked_fields(field),
        }
    }

    /// The base class's `getMaskedFields(String)`.
    pub fn super_get_masked_fields(&self, field: &str) -> Option<Vec<String>> {
        self.masked_fields_func
            .as_ref()
            .and_then(|func| func(field))
    }

    /// Returns the `HighlightFlag`s applicable for the current UH instance.
    ///
    /// Equivalent to the protected `getFlags(String)`.
    pub fn get_flags(&self, field: &str) -> HashSet<HighlightFlag> {
        match &self.overrides {
            Some(o) => o.get_flags(self, field),
            None => self.super_get_flags(field),
        }
    }

    /// The base class's `getFlags(String)`.
    pub fn super_get_flags(&self, _field: &str) -> HashSet<HighlightFlag> {
        if let Some(flags) = &self.flags {
            return flags.clone();
        }
        self.evaluate_flags_from_highlighter(self)
    }

    /// The maximum content size to process. Content will be truncated to
    /// this size before highlighting. Typically snippets closer to the
    /// beginning of the document better summarize its content.
    ///
    /// Equivalent to `getMaxLength()`.
    pub fn get_max_length(&self) -> i32 {
        self.max_length
    }

    /// Returns the `BreakIterator` to use for dividing text into passages.
    /// This returns `BreakIterator.getSentenceInstance(Locale.ROOT)` by
    /// default; subclasses can override to customize.
    ///
    /// Note: this highlighter will call `preceding(int)` and `next()` many
    /// times on it. The default generic JDK implementation of `preceding`
    /// performs poorly.
    ///
    /// Equivalent to the protected `getBreakIterator(String)`.
    pub fn get_break_iterator(&self, field: &str) -> Box<dyn BreakIterator> {
        match &self.overrides {
            Some(o) => o.get_break_iterator(self, field),
            None => (self.break_iterator)(),
        }
    }

    /// Returns the `PassageScorer` to use for ranking passages.
    ///
    /// Equivalent to the protected `getScorer(String)`.
    pub fn get_scorer(&self, field: &str) -> PassageScorer {
        match &self.overrides {
            Some(o) => o.get_scorer(self, field),
            None => self.scorer.clone(),
        }
    }

    /// Returns the `PassageFormatter` to use for formatting passages into
    /// highlighted snippets.
    ///
    /// Equivalent to the protected `getFormatter(String)`.
    pub fn get_formatter(&self, field: &str) -> Arc<dyn PassageFormatter> {
        match &self.overrides {
            Some(o) => o.get_formatter(self, field),
            None => Arc::clone(&self.formatter),
        }
    }

    /// Returns the `Comparator` to use for finally sorting passages.
    ///
    /// Equivalent to the protected `getPassageSortComparator(String)`.
    pub fn get_passage_sort_comparator(&self, field: &str) -> PassageSortComparator {
        match &self.overrides {
            Some(o) => o.get_passage_sort_comparator(self, field),
            None => Arc::clone(&self.passage_sort_comparator),
        }
    }

    /// Returns the number of leading passages (as delineated by the
    /// `BreakIterator`) when no highlights could be found. If it's less
    /// than 0 (the default) then this defaults to the `maxPassages`
    /// parameter given for each request. If this is 0 then the resulting
    /// highlight is null (not formatted).
    ///
    /// Equivalent to the protected `getMaxNoHighlightPassages(String)`.
    pub fn get_max_no_highlight_passages(&self, field: &str) -> i32 {
        match &self.overrides {
            Some(o) => o.get_max_no_highlight_passages(self, field),
            None => self.max_no_highlight_passages,
        }
    }

    /// Limits the amount of field value pre-fetching until this threshold
    /// is passed. The highlighter internally highlights in batches of
    /// documents sized on the sum field value length (in chars) of the
    /// fields to be highlighted (bounded by `get_max_length()` for each
    /// field). By setting this to 0, you can force documents to be fetched
    /// and highlighted one at a time, which you usually shouldn't do. The
    /// default is 524288 chars which translates to about a megabyte.
    /// However, note that the highlighter sometimes ignores this and
    /// highlights one document at a time (without caching a bunch of
    /// documents in advance) when it can detect there's no point in it --
    /// such as when all fields will be highlighted via re-analysis as one
    /// example.
    ///
    /// Equivalent to `getCacheFieldValCharsThreshold()`.
    pub fn get_cache_field_val_chars_threshold(&self) -> i32 {
        // question: should we size by bytes instead?
        self.cache_field_val_chars_threshold
    }

    /// ... as passed in from constructor.
    ///
    /// Equivalent to `getIndexSearcher()`.
    pub fn get_index_searcher(&self) -> Option<&Arc<IndexSearcher>> {
        self.searcher.as_ref()
    }

    /// ... as passed in from constructor.
    ///
    /// Equivalent to `getIndexAnalyzer()`.
    pub fn get_index_analyzer(&self) -> &Arc<dyn Analyzer> {
        &self.index_analyzer
    }

    /// Determine the offset source for the specified field. The default
    /// algorithm is as follows:
    ///
    /// 1. This calls `get_field_info`, which may consult the searcher's
    ///    field infos (fetched once and cached).
    /// 2. If there's a field info it first checks for offsets in the
    ///    postings, then term vectors with offsets.
    /// 3. Otherwise, use the analysis approach.
    ///
    /// Equivalent to the protected `getOffsetSource(String)`.
    ///
    /// # Errors
    ///
    /// What reading the field infos raises.
    pub fn get_offset_source(&self, field: &str) -> Result<OffsetSource> {
        match &self.overrides {
            Some(o) => o.get_offset_source(self, field),
            None => self.super_get_offset_source(field),
        }
    }

    /// The base class's `getOffsetSource(String)`.
    ///
    /// # Errors
    ///
    /// What reading the field infos raises.
    pub fn super_get_offset_source(&self, field: &str) -> Result<OffsetSource> {
        if let Some(field_info) = self.get_field_info(field)? {
            if field_info.get_index_options()
                == IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS
            {
                return Ok(if field_info.has_term_vectors() {
                    OffsetSource::POSTINGS_WITH_TERM_VECTORS
                } else {
                    OffsetSource::POSTINGS
                });
            }
            if field_info.has_term_vectors() {
                // unfortunately we can't also check if the TV has offsets
                return Ok(OffsetSource::TERM_VECTORS);
            }
        }
        Ok(OffsetSource::ANALYSIS)
    }

    /// Called by the default implementation of `get_offset_source` to get
    /// field information about the field to be highlighted. Can be
    /// overridden by subclasses.
    ///
    /// Equivalent to the protected `getFieldInfo(String)`.
    ///
    /// # Errors
    ///
    /// What reading the field infos raises.
    pub fn get_field_info(&self, field: &str) -> Result<Option<FieldInfo>> {
        match &self.overrides {
            Some(o) => o.get_field_info(self, field),
            None => self.super_get_field_info(field),
        }
    }

    /// The base class's `getFieldInfo(String)`.
    ///
    /// # Errors
    ///
    /// What reading the field infos raises.
    pub fn super_get_field_info(&self, field: &str) -> Result<Option<FieldInfo>> {
        let Some(searcher) = &self.searcher else {
            return Ok(None);
        };
        // Need thread-safety for lazy-init but lets avoid 'synchronized' by
        // using double-check locking idiom
        let field_infos = match self.field_infos.get() {
            Some(field_infos) => field_infos,
            None => {
                let merged =
                    FieldInfos::get_merged_field_infos(Arc::clone(searcher.get_index_reader()))?;
                self.field_infos.get_or_init(|| merged)
            }
        };
        Ok(field_infos.field_info(field).cloned())
    }

    /// Highlights the top passages from a single field.
    ///
    /// Equivalent to `highlight(String, Query, TopDocs)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors; `IllegalArgumentException` if `field` was
    /// indexed without positions or offsets.
    pub fn highlight(
        &self,
        field: &str,
        query: &Arc<dyn Query>,
        top_docs: &TopDocs,
    ) -> Result<Vec<Option<String>>> {
        self.highlight_max_passages(field, query, top_docs, 1)
    }

    /// Highlights the top-N passages from a single field.
    ///
    /// Equivalent to `highlight(String, Query, TopDocs, int)`.
    ///
    /// # Errors
    ///
    /// As [`highlight`](Self::highlight).
    pub fn highlight_max_passages(
        &self,
        field: &str,
        query: &Arc<dyn Query>,
        top_docs: &TopDocs,
        max_passages: i32,
    ) -> Result<Vec<Option<String>>> {
        let mut res = self.highlight_fields_top_docs_max_passages(
            &[field.to_string()],
            query,
            top_docs,
            &[max_passages],
        )?;
        Ok(res.remove(field).unwrap_or_default())
    }

    /// Highlights the top passages from multiple fields.
    ///
    /// Equivalent to `highlightFields(String[], Query, TopDocs)`.
    ///
    /// # Errors
    ///
    /// As [`highlight`](Self::highlight).
    pub fn highlight_fields(
        &self,
        fields: &[String],
        query: &Arc<dyn Query>,
        top_docs: &TopDocs,
    ) -> Result<HashMap<String, Vec<Option<String>>>> {
        let max_passages = vec![1; fields.len()];
        self.highlight_fields_top_docs_max_passages(fields, query, top_docs, &max_passages)
    }

    /// Highlights the top-N passages from multiple fields.
    ///
    /// Equivalent to `highlightFields(String[], Query, TopDocs, int[])`.
    ///
    /// # Errors
    ///
    /// As [`highlight`](Self::highlight).
    pub fn highlight_fields_top_docs_max_passages(
        &self,
        fields: &[String],
        query: &Arc<dyn Query>,
        top_docs: &TopDocs,
        max_passages: &[i32],
    ) -> Result<HashMap<String, Vec<Option<String>>>> {
        let docids: Vec<i32> = top_docs.score_docs.iter().map(|sd| sd.doc).collect();
        self.highlight_fields_doc_ids(fields, query, &docids, max_passages)
    }

    /// Highlights the top-N passages from multiple fields, for the provided
    /// int[] docids.
    ///
    /// Equivalent to `highlightFields(String[], Query, int[], int[])`.
    ///
    /// # Errors
    ///
    /// As [`highlight`](Self::highlight).
    pub fn highlight_fields_doc_ids(
        &self,
        fields_in: &[String],
        query: &Arc<dyn Query>,
        docids_in: &[i32],
        max_passages_in: &[i32],
    ) -> Result<HashMap<String, Vec<Option<String>>>> {
        let mut snippets: HashMap<String, Vec<Option<String>>> = HashMap::new();
        for (key, snippet_objects) in
            self.highlight_fields_as_objects(fields_in, query, docids_in, max_passages_in)?
        {
            let snippet_strings: Vec<Option<String>> = snippet_objects
                .iter()
                .map(|snippet| snippet.as_ref().map(|s| s.to_string()))
                .collect();
            snippets.insert(key, snippet_strings);
        }
        Ok(snippets)
    }

    /// Expert: highlights the top-N passages from multiple fields, for the
    /// provided int[] docids, to custom Object as returned by the
    /// `PassageFormatter`. Use this API to render to something other than
    /// String.
    ///
    /// Equivalent to the protected `highlightFieldsAsObjects(String[], Query,
    /// int[], int[])`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an empty or mismatched field list;
    /// `IllegalStateException` without a searcher; propagates I/O errors.
    pub fn highlight_fields_as_objects(
        &self,
        fields_in: &[String],
        query: &Arc<dyn Query>,
        doc_ids_in: &[i32],
        max_passages_in: &[i32],
    ) -> Result<HashMap<String, Vec<Option<Box<dyn FormattedPassages>>>>> {
        if fields_in.is_empty() {
            return Err(LuceneError::illegal_argument("fieldsIn must not be empty"));
        }
        if fields_in.len() != max_passages_in.len() {
            return Err(LuceneError::illegal_argument(
                "invalid number of maxPassagesIn",
            ));
        }
        let Some(searcher) = &self.searcher else {
            return Err(LuceneError::illegal_state(
                "This method requires that an indexSearcher was passed in the \
                 constructor.  Perhaps you mean to call highlightWithoutSearcher?",
            ));
        };

        // Sort docs & fields for sequential i/o

        // Sort doc IDs w/ index to original order: (copy input arrays since
        // we sort in-place)
        let mut doc_order: Vec<usize> = (0..doc_ids_in.len()).collect();
        doc_order.sort_by_key(|&i| doc_ids_in[i]); // stable, as InPlaceMergeSorter
        let doc_ids: Vec<i32> = doc_order.iter().map(|&i| doc_ids_in[i]).collect();
        let doc_in_indexes: Vec<usize> = doc_order; // fill in ascending order; points into docIdsIn[]

        // Sort fields w/ maxPassages pair: (copy input arrays since we sort
        // in-place)
        let mut field_order: Vec<usize> = (0..fields_in.len()).collect();
        field_order.sort_by(|&i, &j| java_compare(&fields_in[i], &fields_in[j]));
        let fields: Vec<String> = field_order.iter().map(|&i| fields_in[i].clone()).collect();
        let max_passages: Vec<i32> = field_order.iter().map(|&i| max_passages_in[i]).collect();

        // Init field highlighters (where most of the highlight logic lives,
        // and on a per field basis)
        let query_terms = Self::extract_terms(&**query);
        let mut field_highlighters: Vec<FieldHighlighter> = Vec::with_capacity(fields.len());
        let mut num_term_vectors = 0;
        let mut num_postings = 0;
        for (f, field) in fields.iter().enumerate() {
            let field_highlighter =
                self.get_field_highlighter(field, query, &query_terms, max_passages[f])?;
            match field_highlighter.get_offset_source() {
                OffsetSource::TERM_VECTORS => num_term_vectors += 1,
                OffsetSource::POSTINGS => num_postings += 1,
                OffsetSource::POSTINGS_WITH_TERM_VECTORS => {
                    num_term_vectors += 1;
                    num_postings += 1;
                }
                OffsetSource::ANALYSIS | OffsetSource::NONE_NEEDED => {}
            }
            field_highlighters.push(field_highlighter);
        }

        let cache_chars_threshold =
            self.calculate_optimal_cache_chars_threshold(num_term_vectors, num_postings);

        let index_reader_with_term_vec_cache: Option<Arc<dyn IndexReader>> =
            if num_term_vectors >= 2 {
                Some(TermVectorReusingLeafReader::wrap(Arc::clone(
                    searcher.get_index_reader(),
                ))?)
            } else {
                None
            };

        // [fieldIdx][docIdInIndex] of highlightDoc result
        let mut highlight_docs_in_by_field: Vec<Vec<Option<Box<dyn FormattedPassages>>>> = (0
            ..fields.len())
            .map(|_| (0..doc_ids.len()).map(|_| None).collect())
            .collect();
        // Highlight in doc batches determined by loadFieldValues (consumes
        // from docIdIter)
        let mut doc_id_iter = SortedDocIdIterator {
            sorted_doc_ids: &doc_ids,
            idx: -1,
        };
        let mut batch_doc_idx = 0;
        while batch_doc_idx < doc_ids.len() {
            // Load the field values of the first batch of document(s) (note:
            // commonly all docs are in this batch)
            let field_vals_by_doc =
                self.load_field_values(&fields, &mut doc_id_iter, cache_chars_threshold)?;
            //    the size of the above list is the size of the batch (num
            //    of docs in the batch)

            // Highlight in per-field order first, then by doc (better I/O
            // pattern)
            for (field_idx, field_highlighter) in field_highlighters.iter_mut().enumerate() {
                let result_by_doc_in = &mut highlight_docs_in_by_field[field_idx]; // parallel to docIdsIn
                let mut doc_idx = batch_doc_idx;
                while doc_idx - batch_doc_idx < field_vals_by_doc.len() {
                    let mut doc_id = doc_ids[doc_idx]; // sorted order
                    let content = &field_vals_by_doc[doc_idx - batch_doc_idx][field_idx];
                    let Some(content) = content else {
                        doc_idx += 1;
                        continue;
                    };
                    let index_reader: Arc<dyn IndexReader> = match (
                        field_highlighter.get_offset_source() == OffsetSource::TERM_VECTORS,
                        &index_reader_with_term_vec_cache,
                    ) {
                        (true, Some(cache)) => Arc::clone(cache),
                        _ => Arc::clone(searcher.get_index_reader()),
                    };
                    let leaves = index_reader.leaves()?;
                    let leaf_reader_context =
                        &leaves[sub_index_from_leaves(doc_id, &leaves) as usize];
                    let leaf_reader = leaf_reader_context.leaf_reader();
                    doc_id -= leaf_reader_context.doc_base(); // adjust 'doc' to be within this leaf reader
                    let doc_in_index = doc_in_indexes[doc_idx]; // original input order
                    debug_assert!(result_by_doc_in[doc_in_index].is_none());
                    result_by_doc_in[doc_in_index] = field_highlighter.highlight_field_for_doc(
                        Some(&leaf_reader),
                        doc_id,
                        &String::from_utf16_lossy(content),
                    )?;
                    doc_idx += 1;
                }
            }

            batch_doc_idx += field_vals_by_doc.len();
        }
        if let Some(cache) = &index_reader_with_term_vec_cache {
            cache.close()?; // FYI won't close underlying reader
        }
        debug_assert!(
            doc_id_iter.doc_id() == rucene::search::NO_MORE_DOCS
                || doc_id_iter.next_doc() == rucene::search::NO_MORE_DOCS
        );

        // TODO reconsider the return type; since this is an "advanced"
        // method, this API can be a bit more raw
        let mut result_map: HashMap<String, Vec<Option<Box<dyn FormattedPassages>>>> =
            HashMap::with_capacity(fields.len());
        for (f, results) in highlight_docs_in_by_field.into_iter().enumerate() {
            result_map.insert(fields[f].clone(), results);
        }
        Ok(result_map)
    }

    /// When cacheCharsThreshold is 0, loadFieldValues() only fetches one
    /// document at a time. We override it to be 0 in two circumstances:
    fn calculate_optimal_cache_chars_threshold(
        &self,
        num_term_vectors: i32,
        num_postings: i32,
    ) -> i32 {
        if num_postings == 0 && num_term_vectors == 0 {
            // (1) When all fields are highlighted via re-analysis
            0
        } else if num_term_vectors >= 2 {
            // (2) When 2+ fields are highlighted via term vectors
            0
        } else {
            self.get_cache_field_val_chars_threshold()
        }
    }

    /// Highlights text passed as a parameter. This requires the
    /// `IndexSearcher` provided to this highlighter is `None`. This use-case
    /// is more rare. Naturally, the mode of highlighting will be
    /// `OffsetSource.ANALYSIS`. The supplied `field` will be put into
    /// field infos, so its name will be used as such.
    ///
    /// Equivalent to `highlightWithoutSearcher(String, Query, String, int)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` when a searcher was given; propagates I/O
    /// errors.
    pub fn highlight_without_searcher(
        &self,
        field: &str,
        query: &Arc<dyn Query>,
        content: &str,
        max_passages: i32,
    ) -> Result<Option<Box<dyn FormattedPassages>>> {
        if self.searcher.is_some() {
            return Err(LuceneError::illegal_state(
                "highlightWithoutSearcher should only be called on a UnifiedHighlighter without an IndexSearcher.",
            ));
        }
        let query_terms = Self::extract_terms(&**query);
        self.get_field_highlighter(field, query, &query_terms, max_passages)?
            .highlight_field_for_doc(None, -1, content)
    }

    /// Equivalent to the protected `getFieldHighlighter(String, Query, Set,
    /// int)`.
    ///
    /// # Errors
    ///
    /// What building the strategy raises.
    pub fn get_field_highlighter(
        &self,
        field: &str,
        query: &Arc<dyn Query>,
        all_terms: &HashSet<Term>,
        max_passages: i32,
    ) -> Result<FieldHighlighter> {
        match &self.overrides {
            Some(o) => o.get_field_highlighter(self, field, query, all_terms, max_passages),
            None => self.super_get_field_highlighter(field, query, all_terms, max_passages),
        }
    }

    /// The base class's `getFieldHighlighter(String, Query, Set, int)`.
    ///
    /// # Errors
    ///
    /// What building the strategy raises.
    pub fn super_get_field_highlighter(
        &self,
        field: &str,
        query: &Arc<dyn Query>,
        all_terms: &HashSet<Term>,
        max_passages: i32,
    ) -> Result<FieldHighlighter> {
        let masked_fields = self.get_masked_fields(field);
        let field_offset_strategy: Box<dyn FieldOffsetStrategy> = match masked_fields {
            Some(masked_fields) if !masked_fields.is_empty() => {
                let mut fields_offset_strategies: Vec<Box<dyn FieldOffsetStrategy>> =
                    Vec::with_capacity(masked_fields.len() + 1);
                for masked_field in &masked_fields {
                    let components =
                        self.get_highlight_components(masked_field, query, all_terms)?;
                    let offset_source = self.get_optimized_offset_source(&components)?;
                    fields_offset_strategies
                        .push(self.get_offset_strategy(offset_source, components)?);
                }
                // adding original field as well
                let components = self.get_highlight_components(field, query, all_terms)?;
                let offset_source = self.get_optimized_offset_source(&components)?;
                fields_offset_strategies.push(self.get_offset_strategy(offset_source, components)?);

                Box::new(MultiFieldsOffsetStrategy::new(fields_offset_strategies))
            }
            _ => {
                let components = self.get_highlight_components(field, query, all_terms)?;
                let offset_source = self.get_optimized_offset_source(&components)?;
                self.get_offset_strategy(offset_source, components)?
            }
        };

        Ok(self.new_field_highlighter(
            field,
            field_offset_strategy,
            Box::new(SplittingBreakIterator::new(
                self.get_break_iterator(field),
                MULTIVAL_SEP_CHAR,
            )),
            self.get_scorer(field),
            max_passages,
            self.get_max_no_highlight_passages(field),
            self.get_formatter(field),
            self.get_passage_sort_comparator(field),
        ))
    }

    /// Equivalent to the protected `newFieldHighlighter(String,
    /// FieldOffsetStrategy, BreakIterator, PassageScorer, int, int,
    /// PassageFormatter, Comparator)`.
    #[allow(clippy::too_many_arguments)]
    pub fn new_field_highlighter(
        &self,
        field: &str,
        field_offset_strategy: Box<dyn FieldOffsetStrategy>,
        break_iterator: Box<dyn BreakIterator>,
        passage_scorer: PassageScorer,
        max_passages: i32,
        max_no_highlight_passages: i32,
        passage_formatter: Arc<dyn PassageFormatter>,
        passage_sort_comparator: PassageSortComparator,
    ) -> FieldHighlighter {
        match &self.overrides {
            Some(o) => o.new_field_highlighter(
                self,
                field,
                field_offset_strategy,
                break_iterator,
                passage_scorer,
                max_passages,
                max_no_highlight_passages,
                passage_formatter,
                passage_sort_comparator,
            ),
            None => FieldHighlighter::new(
                field,
                field_offset_strategy,
                break_iterator,
                passage_scorer,
                max_passages,
                max_no_highlight_passages,
                passage_formatter,
                passage_sort_comparator,
            ),
        }
    }

    /// Equivalent to the protected `getHighlightComponents(String, Query,
    /// Set)`.
    ///
    /// # Errors
    ///
    /// What building the phrase helper raises.
    pub fn get_highlight_components(
        &self,
        field: &str,
        query: &Arc<dyn Query>,
        all_terms: &HashSet<Term>,
    ) -> Result<UHComponents> {
        match &self.overrides {
            Some(o) => o.get_highlight_components(self, field, query, all_terms),
            None => self.super_get_highlight_components(field, query, all_terms),
        }
    }

    /// The base class's `getHighlightComponents(String, Query, Set)`.
    ///
    /// # Errors
    ///
    /// What building the phrase helper raises.
    pub fn super_get_highlight_components(
        &self,
        field: &str,
        query: &Arc<dyn Query>,
        all_terms: &HashSet<Term>,
    ) -> Result<UHComponents> {
        let field_matcher = self.get_field_matcher(field);
        let highlight_flags = self.get_flags(field);
        let phrase_helper = self.get_phrase_helper(field, query, &highlight_flags)?;
        let query_has_unrecognized_part = self.has_unrecognized_query(&field_matcher, &**query);
        let mut terms = None;
        let mut automata = None;
        if !highlight_flags.contains(&HighlightFlag::WEIGHT_MATCHES) || !query_has_unrecognized_part
        {
            terms = Some(Self::filter_extracted_terms(&field_matcher, all_terms));
            automata = Some(self.get_automata(field, &**query, &highlight_flags));
        } // otherwise don't need to extract
        Ok(UHComponents::new(
            field,
            field_matcher,
            Arc::clone(query),
            terms,
            phrase_helper,
            automata,
            query_has_unrecognized_part,
            highlight_flags,
        ))
    }

    /// Equivalent to the protected `hasUnrecognizedQuery(Predicate, Query)`.
    pub fn has_unrecognized_query(&self, field_matcher: &FieldMatcher, query: &dyn Query) -> bool {
        match &self.overrides {
            Some(o) => o.has_unrecognized_query(self, field_matcher, query),
            None => self.super_has_unrecognized_query(field_matcher, query),
        }
    }

    /// The base class's `hasUnrecognizedQuery(Predicate, Query)`.
    pub fn super_has_unrecognized_query(
        &self,
        field_matcher: &FieldMatcher,
        query: &dyn Query,
    ) -> bool {
        let mut visitor = UnknownLeafVisitor {
            field_matcher,
            has_unknown_leaf: false,
        };
        query.visit(&mut visitor);
        visitor.has_unknown_leaf
    }

    /// Equivalent to the protected static `filterExtractedTerms(Predicate,
    /// Set)`: the terms of the accepted fields, sorted.
    pub fn filter_extracted_terms(
        field_matcher: &FieldMatcher,
        query_terms: &HashSet<Term>,
    ) -> Vec<BytesRef> {
        // Strip off the redundant field and sort the remaining terms
        let mut filtered_terms: BTreeSet<BytesRef> = BTreeSet::new();
        for term in query_terms {
            if term.field_opt().is_some_and(|f| field_matcher(f)) {
                filtered_terms.insert(term.bytes().clone());
            }
        }
        filtered_terms.into_iter().collect()
    }

    /// Equivalent to the protected `getPhraseHelper(String, Query, Set)`.
    ///
    /// # Errors
    ///
    /// What building the phrase helper raises.
    pub fn get_phrase_helper(
        &self,
        field: &str,
        query: &Arc<dyn Query>,
        highlight_flags: &HashSet<HighlightFlag>,
    ) -> Result<Arc<PhraseHelper>> {
        match &self.overrides {
            Some(o) => o.get_phrase_helper(self, field, query, highlight_flags),
            None => self.super_get_phrase_helper(field, query, highlight_flags),
        }
    }

    /// The base class's `getPhraseHelper(String, Query, Set)`.
    ///
    /// # Errors
    ///
    /// What building the phrase helper raises.
    pub fn super_get_phrase_helper(
        &self,
        field: &str,
        query: &Arc<dyn Query>,
        highlight_flags: &HashSet<HighlightFlag>,
    ) -> Result<Arc<PhraseHelper>> {
        let use_weight_matches_iter = highlight_flags.contains(&HighlightFlag::WEIGHT_MATCHES);
        if use_weight_matches_iter {
            return Ok(PhraseHelper::none()); // will be handled by Weight.matches which always considers phrases
        }
        let highlight_phrases_strictly = highlight_flags.contains(&HighlightFlag::PHRASES);
        let handle_multi_term_query = highlight_flags.contains(&HighlightFlag::MULTI_TERM_QUERY);
        if !highlight_phrases_strictly {
            return Ok(PhraseHelper::none());
        }
        let overrides_for_rewrite = self.overrides.clone();
        let overrides_for_pre = self.overrides.clone();
        Ok(Arc::new(PhraseHelper::new(
            Arc::clone(query),
            field,
            self.get_field_matcher(field),
            Arc::new(
                move |span_query: &dyn SpanQuery| match &overrides_for_rewrite {
                    Some(o) => o.requires_rewrite(span_query),
                    None => None,
                },
            ),
            Arc::new(move |query: &Arc<dyn Query>| match &overrides_for_pre {
                Some(o) => o.pre_span_query_rewrite(query),
                None => None,
            }),
            !handle_multi_term_query,
        )?))
    }

    /// Equivalent to the protected `getAutomata(String, Query, Set)`.
    pub fn get_automata(
        &self,
        field: &str,
        query: &dyn Query,
        highlight_flags: &HashSet<HighlightFlag>,
    ) -> Vec<Arc<dyn LabelledCharArrayMatcher>> {
        match &self.overrides {
            Some(o) => o.get_automata(self, field, query, highlight_flags),
            None => self.super_get_automata(field, query, highlight_flags),
        }
    }

    /// The base class's `getAutomata(String, Query, Set)`.
    pub fn super_get_automata(
        &self,
        field: &str,
        query: &dyn Query,
        highlight_flags: &HashSet<HighlightFlag>,
    ) -> Vec<Arc<dyn LabelledCharArrayMatcher>> {
        // do we "eagerly" look in span queries for automata here, or do we
        // not and let PhraseHelper handle those (and global automata
        // extraction)?
        let look_in_span = !highlight_flags.contains(&HighlightFlag::PHRASES) // no PhraseHelper
            || highlight_flags.contains(&HighlightFlag::WEIGHT_MATCHES); // Weight.Matches will find all

        if highlight_flags.contains(&HighlightFlag::MULTI_TERM_QUERY) {
            MultiTermHighlighting::extract_automata(
                query,
                &self.get_field_matcher(field),
                look_in_span,
            )
        } else {
            Vec::new()
        }
    }

    /// Equivalent to the protected `getOptimizedOffsetSource(UHComponents)`.
    ///
    /// # Errors
    ///
    /// What reading the field infos raises.
    pub fn get_optimized_offset_source(&self, components: &UHComponents) -> Result<OffsetSource> {
        match &self.overrides {
            Some(o) => o.get_optimized_offset_source(self, components),
            None => self.super_get_optimized_offset_source(components),
        }
    }

    /// The base class's `getOptimizedOffsetSource(UHComponents)`.
    ///
    /// # Errors
    ///
    /// What reading the field infos raises.
    pub fn super_get_optimized_offset_source(
        &self,
        components: &UHComponents,
    ) -> Result<OffsetSource> {
        let offset_source = self.get_offset_source(components.field())?;

        // null automata means unknown, so assume a possibility
        let mtq_or_rewrite = components.automata().is_none_or(|a| !a.is_empty())
            || components.phrase_helper().will_rewrite()
            || components.has_unrecognized_query_part();

        // null terms means unknown, so assume something to highlight
        if !mtq_or_rewrite && components.terms().is_some_and(|t| t.is_empty()) {
            return Ok(OffsetSource::NONE_NEEDED); // nothing to highlight
        }

        match offset_source {
            OffsetSource::POSTINGS => {
                if mtq_or_rewrite {
                    // may need to see scan through all terms for the
                    // highlighted document efficiently
                    return Ok(OffsetSource::ANALYSIS);
                }
            }
            OffsetSource::POSTINGS_WITH_TERM_VECTORS => {
                if !mtq_or_rewrite {
                    return Ok(OffsetSource::POSTINGS); // We don't need term vectors
                }
            }
            OffsetSource::ANALYSIS | OffsetSource::TERM_VECTORS | OffsetSource::NONE_NEEDED => {
                // stick with the original offset source
            }
        }

        Ok(offset_source)
    }

    /// Equivalent to the protected `getOffsetStrategy(OffsetSource,
    /// UHComponents)`.
    ///
    /// # Errors
    ///
    /// What building the strategy raises.
    pub fn get_offset_strategy(
        &self,
        offset_source: OffsetSource,
        components: UHComponents,
    ) -> Result<Box<dyn FieldOffsetStrategy>> {
        match &self.overrides {
            Some(o) => o.get_offset_strategy(self, offset_source, components),
            None => self.super_get_offset_strategy(offset_source, components),
        }
    }

    /// The base class's `getOffsetStrategy(OffsetSource, UHComponents)`.
    ///
    /// # Errors
    ///
    /// What building the strategy raises.
    pub fn super_get_offset_strategy(
        &self,
        offset_source: OffsetSource,
        components: UHComponents,
    ) -> Result<Box<dyn FieldOffsetStrategy>> {
        Ok(match offset_source {
            OffsetSource::ANALYSIS => {
                if !components.phrase_helper().has_position_sensitivity()
                    && !components
                        .highlight_flags()
                        .contains(&HighlightFlag::PASSAGE_RELEVANCY_OVER_SPEED)
                    && !components
                        .highlight_flags()
                        .contains(&HighlightFlag::WEIGHT_MATCHES)
                {
                    // skip using a memory index since it's pure term filtering
                    Box::new(TokenStreamOffsetStrategy::new(
                        components,
                        Arc::clone(self.get_index_analyzer()),
                    )?)
                } else {
                    Box::new(MemoryIndexOffsetStrategy::new(
                        components,
                        Arc::clone(self.get_index_analyzer()),
                    )?)
                }
            }
            OffsetSource::NONE_NEEDED => Box::new(NoOpOffsetStrategy::instance()),
            OffsetSource::TERM_VECTORS => Box::new(TermVectorOffsetStrategy::new(components)),
            OffsetSource::POSTINGS => Box::new(PostingsOffsetStrategy::new(components)),
            OffsetSource::POSTINGS_WITH_TERM_VECTORS => {
                Box::new(PostingsWithTermVectorsOffsetStrategy::new(components))
            }
        })
    }

    /// Equivalent to the protected `loadFieldValues(String[],
    /// DocIdSetIterator, int)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn load_field_values(
        &self,
        fields: &[String],
        doc_iter: &mut SortedDocIdIterator<'_>,
        cache_chars_threshold: i32,
    ) -> Result<Vec<Vec<Option<Vec<u16>>>>> {
        match &self.overrides {
            Some(o) => o.load_field_values(self, fields, doc_iter, cache_chars_threshold),
            None => self.super_load_field_values(fields, doc_iter, cache_chars_threshold),
        }
    }

    /// The base class's `loadFieldValues(String[], DocIdSetIterator, int)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors; `IllegalStateException` without a searcher
    /// (Java's `NullPointerException`).
    pub fn super_load_field_values(
        &self,
        fields: &[String],
        doc_iter: &mut SortedDocIdIterator<'_>,
        cache_chars_threshold: i32,
    ) -> Result<Vec<Vec<Option<Vec<u16>>>>> {
        let capacity = if cache_chars_threshold == 0 {
            1
        } else {
            64.min(doc_iter.cost()) as usize
        };
        let mut doc_list_of_fields: Vec<Vec<Option<Vec<u16>>>> = Vec::with_capacity(capacity);

        let mut visitor = self.new_limited_stored_fields_visitor(fields);
        let searcher = self
            .searcher
            .as_ref()
            .ok_or_else(LuceneError::null_pointer)?;
        let stored_fields: Box<dyn StoredFields> = searcher.stored_fields()?;
        let mut sum_chars: i64 = 0;
        loop {
            let doc_id = doc_iter.next_doc();
            if doc_id == rucene::search::NO_MORE_DOCS {
                break;
            }
            visitor.limited().init();
            stored_fields.document_with_visitor(doc_id, &mut *visitor)?;
            let values_by_field = visitor.limited().get_values_by_field();
            for val in &values_by_field {
                sum_chars += val.as_ref().map_or(0, |v| v.len() as i64);
            }
            doc_list_of_fields.push(values_by_field);
            if !(sum_chars <= cache_chars_threshold as i64 && cache_chars_threshold != 0) {
                break;
            }
        }
        Ok(doc_list_of_fields)
    }

    /// Equivalent to the protected `newLimitedStoredFieldsVisitor(String[])`.
    pub fn new_limited_stored_fields_visitor(
        &self,
        fields: &[String],
    ) -> Box<dyn LimitedStoredFieldVisitorExt> {
        match &self.overrides {
            Some(o) => o.new_limited_stored_fields_visitor(self, fields),
            None => Box::new(self.super_new_limited_stored_fields_visitor(fields)),
        }
    }

    /// The base class's `newLimitedStoredFieldsVisitor(String[])`.
    pub fn super_new_limited_stored_fields_visitor(
        &self,
        fields: &[String],
    ) -> LimitedStoredFieldVisitor {
        LimitedStoredFieldVisitor::new(fields.to_vec(), MULTIVAL_SEP_CHAR, self.get_max_length())
    }
}

/// Wraps an IndexReader that remembers/caches the last call to
/// `TermVectors.get(int)` so that if the next call has the same ID, then
/// it is reused. If TV's were column-stride (like doc-values), there would
/// be no need for this.
///
/// Equivalent to the private nested class
/// `UnifiedHighlighter.TermVectorReusingLeafReader`.
pub(crate) struct TermVectorReusingLeafReader {
    base: FilterLeafReader,
    cache: Arc<Mutex<(i32, Option<Arc<dyn Fields>>)>>,
}

impl fmt::Debug for TermVectorReusingLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TermVectorReusingLeafReader")
            .finish_non_exhaustive()
    }
}

impl fmt::Display for TermVectorReusingLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.base, f)
    }
}

impl TermVectorReusingLeafReader {
    fn wrap(reader: Arc<dyn IndexReader>) -> Result<Arc<dyn IndexReader>> {
        let mut leaf_readers: Vec<Arc<dyn IndexReader>> = Vec::new();
        for context in reader.leaves()? {
            let wrapped: Arc<dyn LeafReader> = Arc::new(Self::new(context.leaf_reader())?);
            leaf_readers.push(wrapped.as_index_reader());
        }
        Ok(Arc::new(TermVectorReusingCompositeReader::new(
            leaf_readers,
        )?))
    }

    fn new(input: Arc<dyn LeafReader>) -> Result<Self> {
        Ok(Self {
            base: FilterLeafReader::new(input)?,
            cache: Arc::new(Mutex::new((-1, None))),
        })
    }
}

/// The anonymous `TermVectors` of `termVectors()`.
struct ReusingTermVectors {
    orig: Box<dyn TermVectors>,
    cache: Arc<Mutex<(i32, Option<Arc<dyn Fields>>)>>,
}

impl fmt::Debug for ReusingTermVectors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TermVectorReusingLeafReader$1")
            .finish_non_exhaustive()
    }
}

/// A shared `Fields`: the object the cache hands out again.
struct SharedFields(Arc<dyn Fields>);

impl Fields for SharedFields {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        self.0.iterator()
    }

    fn terms(&self, field: &str) -> Result<Option<Arc<dyn Terms>>> {
        self.0.terms(field)
    }

    fn size(&self) -> i32 {
        self.0.size()
    }
}

impl TermVectors for ReusingTermVectors {
    fn get(&self, doc_id: i32) -> Result<Option<Box<dyn Fields>>> {
        let mut cache = lock(&self.cache);
        if doc_id != cache.0 {
            cache.0 = doc_id;
            cache.1 = self.orig.get(doc_id)?.map(Arc::from);
        }
        Ok(cache
            .1
            .as_ref()
            .map(|fields| Box::new(SharedFields(Arc::clone(fields))) as Box<dyn Fields>))
    }
}

impl LeafReader for TermVectorReusingLeafReader {
    crate::forward_leaf_reader!(filter base;
        core,
        filter_leaf_reader_delegate,
        do_close,
        num_docs,
        max_doc,
        stored_fields,
        terms,
        get_numeric_doc_values,
        get_binary_doc_values,
        get_sorted_doc_values,
        get_sorted_numeric_doc_values,
        get_sorted_set_doc_values,
        get_norm_values,
        get_doc_values_skipper,
        get_float_vector_values,
        get_byte_vector_values,
        search_nearest_vectors,
        search_nearest_vectors_byte,
        get_field_infos,
        get_live_docs,
        get_point_values,
        check_integrity,
        get_meta_data,
    );

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        let orig = self.base.get_delegate().term_vectors()?;
        Ok(Box::new(ReusingTermVectors {
            orig,
            cache: Arc::clone(&self.cache),
        }))
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
}

/// The anonymous `BaseCompositeReader` of `TermVectorReusingLeafReader.wrap`:
/// its `doClose()` does not close the underlying reader.
struct TermVectorReusingCompositeReader {
    core: IndexReaderCore,
    base: BaseCompositeReader,
}

impl TermVectorReusingCompositeReader {
    fn new(sub_readers: Vec<Arc<dyn IndexReader>>) -> Result<Self> {
        let core = IndexReaderCore::new();
        let base = BaseCompositeReader::new(sub_readers, false, &core)?;
        Ok(Self { core, base })
    }
}

impl fmt::Debug for TermVectorReusingCompositeReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnifiedHighlighter$TermVectorReusingLeafReader$1")
            .finish_non_exhaustive()
    }
}

impl fmt::Display for TermVectorReusingCompositeReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UnifiedHighlighter$TermVectorReusingLeafReader$1")
    }
}

impl IndexReader for TermVectorReusingCompositeReader {
    fn core(&self) -> &IndexReaderCore {
        &self.core
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        self.ensure_open()?;
        self.base.term_vectors()
    }

    fn num_docs(&self) -> i32 {
        self.base.num_docs()
    }

    fn max_doc(&self) -> i32 {
        self.base.max_doc()
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        self.ensure_open()?;
        self.base.stored_fields()
    }

    fn do_close(&self) -> Result<()> {
        // don't close the underlying reader
        Ok(())
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn doc_freq(&self, term: &Term) -> Result<i32> {
        self.ensure_open()?;
        self.base.doc_freq(term)
    }

    fn total_term_freq(&self, term: &Term) -> Result<i64> {
        self.ensure_open()?;
        self.base.total_term_freq(term)
    }

    fn get_sum_doc_freq(&self, field: &str) -> Result<i64> {
        self.ensure_open()?;
        self.base.get_sum_doc_freq(field)
    }

    fn get_doc_count(&self, field: &str) -> Result<i32> {
        self.ensure_open()?;
        self.base.get_doc_count(field)
    }

    fn get_sum_total_term_freq(&self, field: &str) -> Result<i64> {
        self.ensure_open()?;
        self.base.get_sum_total_term_freq(field)
    }

    fn build_context(
        self: Arc<Self>,
        parent: Option<Weak<dyn IndexReaderContext>>,
        ord_in_parent: i32,
        doc_base_in_parent: i32,
        leaf_ord: i32,
        leaf_doc_base: i32,
    ) -> Arc<dyn IndexReaderContext> {
        let composite: Arc<dyn CompositeReader> = self;
        build_composite_context(
            composite,
            parent,
            ord_in_parent,
            doc_base_in_parent,
            leaf_ord,
            leaf_doc_base,
        )
    }
}

impl CompositeReader for TermVectorReusingCompositeReader {
    fn get_sequential_sub_readers(&self) -> Vec<Arc<dyn IndexReader>> {
        self.base.get_sequential_sub_readers()
    }
}
