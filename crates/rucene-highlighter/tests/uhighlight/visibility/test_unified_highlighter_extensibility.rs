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

//! Port of `visibility/TestUnifiedHighlighterExtensibility.java`: it keeps
//! the extension points of the unified highlighter reachable from outside
//! its package (here: from outside the crate).
//!
//! Java's `new FieldHighlighter(fieldName, null, null, null, 1, 1, null,
//! null)` passes `null` for parts the test never reads; the port's
//! constructor takes non-null values, so the test passes unused instances.
//! `highlightFieldsAsObjects` and `createOffsetsEnumFromReader` overrides
//! that only call `super` are omitted: the port's dispatch reaches the base
//! bodies without them.

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::error::{LuceneError, Result};
use rucene::index::{LeafReader, Term};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::query::Query;
use rucene_analysis_common::jdk::break_iterator::BreakIterator;
use rucene_highlighter::uhighlight::default_passage_formatter::DefaultPassageFormatter;
use rucene_highlighter::uhighlight::field_highlighter::{
    FieldHighlighter, FieldHighlighterOverrides,
};
use rucene_highlighter::uhighlight::field_offset_strategy::FieldOffsetStrategy;
use rucene_highlighter::uhighlight::offsets_enum::{empty, OffsetsEnum};
use rucene_highlighter::uhighlight::passage::Passage;
use rucene_highlighter::uhighlight::passage_formatter::{FormattedPassages, PassageFormatter};
use rucene_highlighter::uhighlight::passage_scorer::PassageScorer;
use rucene_highlighter::uhighlight::phrase_helper::PhraseHelper;
use rucene_highlighter::uhighlight::splitting_break_iterator::SplittingBreakIterator;
use rucene_highlighter::uhighlight::uh_components::{FieldMatcher, UHComponents};
use rucene_highlighter::uhighlight::unified_highlighter::{
    Builder, LimitedStoredFieldVisitorExt, OffsetSource, PassageSortComparator,
    SortedDocIdIterator, UnifiedHighlighter, UnifiedHighlighterOverrides, MULTIVAL_SEP_CHAR,
};
use rucene_highlighter::uhighlight::whole_break_iterator::WholeBreakIterator;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_test_framework::analysis::MockAnalyzer;

use crate::highlighter_support::core_support::{class, random, test};

/// `TestUnifiedHighlighterExtensibility`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.uhighlight.visibility.TestUnifiedHighlighterExtensibility",
        vec![
            test(
                "testFieldOffsetStrategyExtensibility",
                test_field_offset_strategy_extensibility,
            ),
            test(
                "testUnifiedHighlighterExtensibility",
                test_unified_highlighter_extensibility,
            ),
            test(
                "testPassageFormatterExtensibility",
                test_passage_formatter_extensibility,
            ),
            test(
                "testFieldHiglighterExtensibility",
                test_field_higlighter_extensibility,
            ),
        ],
    )
}

/// The anonymous `FieldOffsetStrategy` of
/// `testFieldOffsetStrategyExtensibility`.
#[derive(Debug)]
struct CustomOffsetStrategy {
    components: UHComponents,
    offset_source: OffsetSource,
}

impl FieldOffsetStrategy for CustomOffsetStrategy {
    fn components(&self) -> Option<&UHComponents> {
        Some(&self.components)
    }

    fn get_offset_source(&self) -> OffsetSource {
        self.offset_source
    }

    fn get_offsets_enum(
        &self,
        _reader: Option<&Arc<dyn LeafReader>>,
        _doc_id: i32,
        _content: &str,
    ) -> Result<Box<dyn OffsetsEnum>> {
        Ok(empty())
    }
}

/// `new UHComponents("field", (s) -> false, MatchAllDocsQuery.INSTANCE, new
/// BytesRef[0], PhraseHelper.NONE, new LabelledCharArrayMatcher[0], false,
/// Collections.emptySet())`.
fn empty_components() -> UHComponents {
    let matcher: FieldMatcher = Arc::new(|_s: &str| false);
    UHComponents::new(
        "field",
        matcher,
        Arc::new(MatchAllDocsQuery::instance()),
        Some(Vec::new()),
        PhraseHelper::none(),
        Some(Vec::new()),
        false,
        HashSet::new(),
    )
}

/// This test is for maintaining the extensibility of the
/// FieldOffsetStrategy for customizations out of package.
fn test_field_offset_strategy_extensibility() -> Result<()> {
    let offset_source = OffsetSource::NONE_NEEDED;
    let strategy = CustomOffsetStrategy {
        components: empty_components(),
        offset_source,
    };
    Assert::assert_true(offset_source == strategy.get_offset_source());
    Ok(())
}

/// The overrides of the anonymous highlighter of
/// `testUnifiedHighlighterExtensibility`.
#[derive(Debug)]
struct ExtensibleOverrides {
    max_length: i32,
}

impl UnifiedHighlighterOverrides for ExtensibleOverrides {
    fn get_offset_source(&self, this: &UnifiedHighlighter, field: &str) -> Result<OffsetSource> {
        this.super_get_offset_source(field)
    }

    fn get_break_iterator(&self, this: &UnifiedHighlighter, field: &str) -> Box<dyn BreakIterator> {
        this.super_get_break_iterator(field)
    }

    fn get_scorer(&self, this: &UnifiedHighlighter, field: &str) -> PassageScorer {
        this.super_get_scorer(field)
    }

    fn get_formatter(&self, this: &UnifiedHighlighter, field: &str) -> Arc<dyn PassageFormatter> {
        this.super_get_formatter(field)
    }

    fn get_passage_sort_comparator(
        &self,
        this: &UnifiedHighlighter,
        field: &str,
    ) -> PassageSortComparator {
        this.super_get_passage_sort_comparator(field)
    }

    fn get_index_analyzer<'a>(&'a self, this: &'a UnifiedHighlighter) -> &'a Arc<dyn Analyzer> {
        this.super_get_index_analyzer()
    }

    fn get_index_searcher<'a>(
        &'a self,
        this: &'a UnifiedHighlighter,
    ) -> Option<&'a Arc<IndexSearcher>> {
        this.super_get_index_searcher()
    }

    fn get_max_no_highlight_passages(&self, this: &UnifiedHighlighter, field: &str) -> i32 {
        this.super_get_max_no_highlight_passages(field)
    }

    fn requires_rewrite(&self, _span_query: &dyn SpanQuery) -> Option<bool> {
        // super.requiresRewrite(spanQuery): the base returns null
        None
    }

    fn new_limited_stored_fields_visitor(
        &self,
        this: &UnifiedHighlighter,
        fields: &[String],
    ) -> Box<dyn LimitedStoredFieldVisitorExt> {
        Box::new(this.super_new_limited_stored_fields_visitor(fields))
    }

    fn load_field_values(
        &self,
        this: &UnifiedHighlighter,
        fields: &[String],
        doc_iter: &mut SortedDocIdIterator<'_>,
        cache_chars_threshold: i32,
    ) -> Result<Vec<Vec<Option<Vec<u16>>>>> {
        this.super_load_field_values(fields, doc_iter, cache_chars_threshold)
    }

    fn get_field_highlighter(
        &self,
        this: &UnifiedHighlighter,
        field: &str,
        query: &Arc<dyn Query>,
        all_terms: &HashSet<Term>,
        max_passages: i32,
    ) -> Result<FieldHighlighter> {
        // THIS IS A COPY of the superclass impl; but use CustomFieldHighlighter
        let components = this.get_highlight_components(field, query, all_terms)?;
        let offset_source = this.get_optimized_offset_source(&components)?;

        // test all is accessible
        components.field();
        components.field_matcher();
        components.query();
        components.terms();
        components.phrase_helper();
        components.automata();
        components.has_unrecognized_query_part();
        components.highlight_flags();

        Ok(custom_field_highlighter(
            field,
            this.get_offset_strategy(offset_source, components)?,
            Box::new(SplittingBreakIterator::new(
                this.get_break_iterator(field),
                MULTIVAL_SEP_CHAR,
            )),
            this.get_scorer(field),
            max_passages,
            this.get_max_no_highlight_passages(field),
            this.get_formatter(field),
            this.get_passage_sort_comparator(field),
        ))
    }

    fn get_highlight_components(
        &self,
        this: &UnifiedHighlighter,
        field: &str,
        query: &Arc<dyn Query>,
        all_terms: &HashSet<Term>,
    ) -> Result<UHComponents> {
        let field_matcher = this.get_field_matcher(field);
        let terms = UnifiedHighlighter::filter_extracted_terms(&field_matcher, all_terms);
        let highlight_flags = this.get_flags(field);
        let phrase_helper = this.get_phrase_helper(field, query, &highlight_flags)?;
        let automata = this.get_automata(field, &**query, &highlight_flags);
        let query_has_unrecognized_part = false;
        Ok(UHComponents::new(
            field,
            field_matcher,
            Arc::clone(query),
            Some(terms),
            phrase_helper,
            Some(automata),
            query_has_unrecognized_part,
            highlight_flags,
        ))
    }

    fn get_offset_strategy(
        &self,
        this: &UnifiedHighlighter,
        offset_source: OffsetSource,
        components: UHComponents,
    ) -> Result<Box<dyn FieldOffsetStrategy>> {
        this.super_get_offset_strategy(offset_source, components)
    }

    fn get_max_length(&self, _this: &UnifiedHighlighter) -> i32 {
        self.max_length
    }
}

/// This test is for maintaining the extensibility of the UnifiedHighlighter
/// for customizations out of package.
fn test_unified_highlighter_extensibility() -> Result<()> {
    let max_length = 1000;
    let uh_builder = Builder::new(None, Arc::new(MockAnalyzer::new(&*random())));
    let mut uh = UnifiedHighlighter::from_builder(uh_builder);
    uh.set_overrides(Arc::new(ExtensibleOverrides { max_length }));
    Assert::assert_equals(uh.get_max_length(), max_length);
    Ok(())
}

/// The response object of `testPassageFormatterExtensibility`: a plain
/// `new Object()`, equal only to itself.
#[derive(Debug)]
struct PlainObject;

impl fmt::Display for PlainObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "java.lang.Object@{:x}",
            self as *const Self as usize as u32
        )
    }
}

/// The anonymous `PassageFormatter` of `testPassageFormatterExtensibility`.
#[derive(Debug)]
struct ConstantFormatter {
    formatted_response: Arc<PlainObject>,
}

/// The same `formattedResponse` object, handed out by the formatter.
#[derive(Debug)]
struct SharedObject(Arc<PlainObject>);

impl fmt::Display for SharedObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&*self.0, f)
    }
}

impl PassageFormatter for ConstantFormatter {
    fn format(&self, _passages: &[Passage], _content: &str) -> Result<Box<dyn FormattedPassages>> {
        Ok(Box::new(SharedObject(Arc::clone(&self.formatted_response))))
    }
}

fn test_passage_formatter_extensibility() -> Result<()> {
    let formatted_response = Arc::new(PlainObject);
    let formatter = ConstantFormatter {
        formatted_response: Arc::clone(&formatted_response),
    };
    let formatted = formatter.format(&[], "")?;
    // assertEquals(Object, Object): Object.equals is identity
    let same = (*formatted)
        .as_any()
        .downcast_ref::<SharedObject>()
        .is_some_and(|o| Arc::ptr_eq(&o.0, &formatted_response));
    Assert::assert_true(same);
    Ok(())
}

/// The anonymous `FieldHighlighter` of `testFieldHiglighterExtensibility`,
/// whose `highlightOffsetsEnums` only calls `super`.
struct PassThroughHighlighter;

impl FieldHighlighterOverrides for PassThroughHighlighter {
    fn highlight_offsets_enums(
        &self,
        this: &mut FieldHighlighter,
        off: &mut dyn OffsetsEnum,
    ) -> Result<Vec<Passage>> {
        this.super_highlight_offsets_enums(off)
    }
}

fn test_field_higlighter_extensibility() -> Result<()> {
    let field_name = "fieldName";
    // the unused parts Java passes as null
    let mut field_highlighter = FieldHighlighter::new(
        field_name,
        Box::new(CustomOffsetStrategy {
            components: empty_components(),
            offset_source: OffsetSource::NONE_NEEDED,
        }),
        Box::new(WholeBreakIterator::new()),
        PassageScorer::new(),
        1,
        1,
        Arc::new(DefaultPassageFormatter::new()),
        Arc::new(|a: &Passage, b: &Passage| a.get_start_offset().cmp(&b.get_start_offset())),
    );
    field_highlighter.set_overrides(Arc::new(PassThroughHighlighter));
    Assert::assert_equals(field_highlighter.get_field(), field_name);
    Ok(())
}

/// `new CustomFieldHighlighter(...)`: tests maintaining
/// extensibility/visibility of `FieldHighlighter` out of package.
#[allow(clippy::too_many_arguments)]
fn custom_field_highlighter(
    field: &str,
    field_offset_strategy: Box<dyn FieldOffsetStrategy>,
    break_iterator: Box<dyn BreakIterator>,
    passage_scorer: PassageScorer,
    max_passages: i32,
    max_no_highlight_passages: i32,
    passage_formatter: Arc<dyn PassageFormatter>,
    passage_sort_comparator: PassageSortComparator,
) -> FieldHighlighter {
    let mut highlighter = FieldHighlighter::new(
        field,
        field_offset_strategy,
        break_iterator,
        passage_scorer,
        max_passages,
        max_no_highlight_passages,
        passage_formatter,
        passage_sort_comparator,
    );
    highlighter.set_overrides(Arc::new(CustomFieldHighlighter));
    highlighter
}

/// `protected static class CustomFieldHighlighter extends FieldHighlighter`.
struct CustomFieldHighlighter;

impl FieldHighlighterOverrides for CustomFieldHighlighter {
    fn highlight_field_for_doc(
        &self,
        this: &mut FieldHighlighter,
        reader: Option<&Arc<dyn LeafReader>>,
        doc_id: i32,
        content: &str,
    ) -> Result<Option<Box<dyn FormattedPassages>>> {
        this.super_highlight_field_for_doc(reader, doc_id, content)
    }

    fn highlight_offsets_enums(
        &self,
        _this: &mut FieldHighlighter,
        _off: &mut dyn OffsetsEnum,
    ) -> Result<Vec<Passage>> {
        // TEST OffsetsEnums & Passage visibility
        // this code never runs; just for compilation
        // Java's first statement, `new OffsetsEnum.OfPostings(null, null)`, dereferences the null
        // PostingsEnum (`postingsEnum.freq()`), so the Passage calls and the
        // `super.highlightOffsetsEnums` call after it are never reached.
        Err(LuceneError::null_pointer())
    }
}
