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

//! The types declared in `SearchTravRetHighlightTask.java`, of the Java
//! package `org.apache.lucene.benchmark.byTask.tasks`.

use std::collections::HashSet;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use rucene::analysis::{Analyzer, SharedTokenStream, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::score_doc::ScoreDoc;
use rucene::search::top_docs::TopDocs;
use rucene_analysis_common::jdk::break_iterator::get_sentence_instance;
use rucene_analysis_common::jdk::locale::Locale;
use rucene_highlighter::highlight::default_encoder::DefaultEncoder;
use rucene_highlighter::highlight::encoder::Encoder;
use rucene_highlighter::highlight::highlighter::Highlighter;
use rucene_highlighter::highlight::query_scorer::QueryScorer;
use rucene_highlighter::highlight::simple_html_formatter::SimpleHTMLFormatter;
use rucene_highlighter::highlight::token_sources::TokenSources;
use rucene_highlighter::uhighlight::unified_highlighter::{
    Builder, OffsetSource, UnifiedHighlighter, UnifiedHighlighterOverrides,
};
use rucene_highlighter::vectorhighlight::break_iterator_boundary_scanner::BreakIteratorBoundaryScanner;
use rucene_highlighter::vectorhighlight::fast_vector_highlighter::FastVectorHighlighter;
use rucene_highlighter::vectorhighlight::score_order_fragments_builder::ScoreOrderFragmentsBuilder;
use rucene_highlighter::vectorhighlight::weighted_frag_list_builder::WeightedFragListBuilder;

use crate::by_task::feeds::query_maker::QueryMaker;
use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{PerfTask, PerfTaskBase};
use crate::by_task::tasks::read_task::{self, query_maker_of, ReadTask, ReadTaskBase};
use crate::by_task::tasks::search_trav_task::SearchTravState;
use crate::jdk;

/// The outer fields the highlighter implementations (Java inner classes)
/// read.
#[derive(Clone)]
pub(crate) struct Outer {
    /// `maxDocCharsToAnalyze`: max leading content chars to highlight.
    max_doc_chars_to_analyze: i32,
    /// `maxFrags`: aka passages.
    max_frags: i32,
    /// `hlFields`.
    hl_fields: HashSet<String>,
    /// `analyzer`.
    analyzer: Option<Arc<dyn Analyzer>>,
}

impl Outer {
    fn analyzer(&self) -> Result<&Arc<dyn Analyzer>> {
        self.analyzer.as_ref().ok_or_else(LuceneError::null_pointer)
    }
}

/// Equivalent to
/// `org.apache.lucene.benchmark.byTask.tasks.SearchTravRetHighlightTask.HLImpl`
/// (`private interface HLImpl`).
pub(crate) trait HLImpl: Send + Sync {
    /// `withTopDocs(IndexSearcher searcher, Query q, TopDocs hits)`.
    fn with_top_docs(
        &self,
        outer: &Outer,
        prevent_optimize_away: &AtomicI32,
        searcher: &Arc<IndexSearcher>,
        q: &Arc<dyn Query>,
        hits: &TopDocs,
    ) -> Result<()>;
}

/// `docIdOrder(ScoreDoc[] scoreDocs)`.
fn doc_id_order(score_docs: &[ScoreDoc]) -> Vec<ScoreDoc> {
    let mut clone: Vec<ScoreDoc> = score_docs.to_vec();
    clone.sort_unstable_by(|a, b| a.doc.cmp(&b.doc));
    clone
}

/// Equivalent to
/// `org.apache.lucene.benchmark.byTask.tasks.SearchTravRetHighlightTask.StandardHLImpl`
/// (`private class StandardHLImpl implements HLImpl`).
pub(crate) struct StandardHLImpl {
    highlighter: Mutex<Highlighter>,
    term_vecs: bool,
}

impl StandardHLImpl {
    /// `StandardHLImpl(boolean termVecs)`. Its field initialiser `new
    /// Highlighter(formatter, encoder, null)` throws (the constructor
    /// rejects a `null` scorer, `Highlighter.java:55`), so every
    /// construction fails as Java's does.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("'fragmentScorer' must not be null")`.
    pub(crate) fn new(term_vecs: bool, max_doc_chars_to_analyze: i32) -> Result<Self> {
        let _ = (term_vecs, max_doc_chars_to_analyze);
        Err(LuceneError::illegal_argument("'fragmentScorer' must not be null"))
    }

    /// The state the constructor would build past the field initialisers:
    /// `highlighter.setEncoder(new DefaultEncoder());
    /// highlighter.setMaxDocCharsToAnalyze(maxDocCharsToAnalyze)`.
    #[allow(dead_code)]
    fn configure(mut highlighter: Highlighter, term_vecs: bool, max_doc_chars_to_analyze: i32) -> Self {
        highlighter.set_encoder(Box::new(DefaultEncoder::new()));
        highlighter.set_max_doc_chars_to_analyze(max_doc_chars_to_analyze);
        let _ = SimpleHTMLFormatter::with_tags("<em>", "</em>");
        StandardHLImpl { highlighter: Mutex::new(highlighter), term_vecs }
    }
}

impl HLImpl for StandardHLImpl {
    fn with_top_docs(
        &self,
        outer: &Outer,
        prevent_optimize_away: &AtomicI32,
        searcher: &Arc<IndexSearcher>,
        q: &Arc<dyn Query>,
        hits: &TopDocs,
    ) -> Result<()> {
        let reader = searcher.get_index_reader();
        let stored_fields = reader.stored_fields()?;
        let term_vectors = reader.term_vectors()?;
        let mut highlighter = jdk::lock(&self.highlighter);
        highlighter.set_fragment_scorer(Box::new(QueryScorer::new(Arc::clone(q))));
        // highlighter.setTextFragmenter();  unfortunately no sentence mechanism, not even regex.
        // Default here is trivial
        for score_doc in doc_id_order(&hits.score_docs) {
            let document = stored_fields.document_fields(score_doc.doc, &outer.hl_fields)?;
            let tv_fields = if self.term_vecs { term_vectors.get(score_doc.doc)? } else { None };
            for indexable_field in document.get_fields() {
                let text = indexable_field.string_value().ok_or_else(LuceneError::null_pointer)?;
                let token_stream: Box<dyn TokenStream> = if self.term_vecs {
                    TokenSources::get_token_stream(
                        indexable_field.name(),
                        tv_fields.as_deref(),
                        &text,
                        &**outer.analyzer()?,
                        outer.max_doc_chars_to_analyze,
                    )?
                } else {
                    Box::new(SharedTokenStream::new(
                        outer.analyzer()?.token_stream_from_str(indexable_field.name(), &text)?,
                    ))
                };
                // will close TokenStream:
                let fragments = highlighter.get_best_fragments_from_stream(token_stream, &text, outer.max_frags)?;
                prevent_optimize_away.store(fragments.len() as i32, Ordering::SeqCst);
            }
        }
        Ok(())
    }
}

/// Equivalent to
/// `org.apache.lucene.benchmark.byTask.tasks.SearchTravRetHighlightTask.FastVectorHLImpl`
/// (`private class FastVectorHLImpl implements HLImpl`).
pub(crate) struct FastVectorHLImpl {
    frag_size: i32,
    frag_list_builder: WeightedFragListBuilder,
    fragments_builder: ScoreOrderFragmentsBuilder,
    pre_tags: Vec<String>,
    post_tags: Vec<String>,
    encoder: Box<dyn Encoder>,
    highlighter: FastVectorHighlighter,
}

impl FastVectorHLImpl {
    /// The field initialisers of `FastVectorHLImpl`.
    pub(crate) fn new() -> Self {
        let bs = BreakIteratorBoundaryScanner::new(get_sentence_instance(&Locale::english()));
        FastVectorHLImpl {
            frag_size: 100,
            frag_list_builder: WeightedFragListBuilder::new(),
            fragments_builder: ScoreOrderFragmentsBuilder::with_scanner(Box::new(bs)),
            pre_tags: vec!["<em>".to_string()],
            post_tags: vec!["</em>".to_string()],
            encoder: Box::new(DefaultEncoder::new()), // new SimpleHTMLEncoder();
            highlighter: FastVectorHighlighter::with_options(
                true,  // phraseHighlight
                false, // requireFieldMatch -- not pertinent to our benchmark
            ),
        }
    }
}

impl HLImpl for FastVectorHLImpl {
    fn with_top_docs(
        &self,
        outer: &Outer,
        prevent_optimize_away: &AtomicI32,
        searcher: &Arc<IndexSearcher>,
        q: &Arc<dyn Query>,
        hits: &TopDocs,
    ) -> Result<()> {
        let reader = searcher.get_index_reader();
        let fq = self.highlighter.get_field_query_with_reader(q, Some(reader))?;
        for score_doc in doc_id_order(&hits.score_docs) {
            for hl_field in &outer.hl_fields {
                let fragments = self
                    .highlighter
                    .get_best_fragments_with(
                        &fq,
                        &**reader,
                        score_doc.doc,
                        hl_field,
                        self.frag_size,
                        outer.max_frags,
                        &self.frag_list_builder,
                        &self.fragments_builder,
                        &self.pre_tags,
                        &self.post_tags,
                        &*self.encoder,
                    )?
                    .ok_or_else(LuceneError::null_pointer)?;
                prevent_optimize_away.store(fragments.len() as i32, Ordering::SeqCst);
            }
        }
        Ok(())
    }
}

/// The anonymous `UnifiedHighlighter` subclass of `UnifiedHLImpl.reset`,
/// whose `getOffsetSource` answers the configured source, or the default.
struct FixedOffsetSource(Option<OffsetSource>);

impl UnifiedHighlighterOverrides for FixedOffsetSource {
    fn get_offset_source(&self, this: &UnifiedHighlighter, field: &str) -> Result<OffsetSource> {
        match self.0 {
            Some(source) => Ok(source),
            None => this.super_get_offset_source(field),
        }
    }
}

/// Equivalent to
/// `org.apache.lucene.benchmark.byTask.tasks.SearchTravRetHighlightTask.UnifiedHLImpl`
/// (`private class UnifiedHLImpl implements HLImpl`).
pub(crate) struct UnifiedHLImpl {
    /// `(highlighter, lastSearcher)`.
    state: Mutex<(Option<UnifiedHighlighter>, Option<Arc<IndexSearcher>>)>,
    /// null means auto select
    offset_source: Option<OffsetSource>,
    fields: Vec<String>,
    max_passages: Vec<i32>,
}

impl UnifiedHLImpl {
    /// `UnifiedHLImpl(final UnifiedHighlighter.OffsetSource offsetSource)`.
    pub(crate) fn new(offset_source: Option<OffsetSource>, outer: &Outer) -> Self {
        UnifiedHLImpl {
            state: Mutex::new((None, None)),
            offset_source,
            fields: outer.hl_fields.iter().cloned().collect(),
            max_passages: vec![outer.max_frags; outer.hl_fields.len()],
        }
    }

    /// `reset(IndexSearcher searcher)`.
    fn reset(&self, outer: &Outer, searcher: &Arc<IndexSearcher>) -> Result<()> {
        let mut state = jdk::lock(&self.state);
        if state.1.as_ref().is_some_and(|last| Arc::ptr_eq(last, searcher)) {
            return Ok(());
        }
        state.1 = Some(Arc::clone(searcher));
        let uh_builder = Builder::new(Some(Arc::clone(searcher)), Arc::clone(outer.analyzer()?))
            .with_break_iterator(Arc::new(|| get_sentence_instance(&Locale::english())))
            .with_max_length(outer.max_doc_chars_to_analyze)?
            .with_highlight_phrases_strictly(true)
            .with_handle_multi_term_query(true);
        let mut highlighter = UnifiedHighlighter::from_builder(uh_builder);
        highlighter.set_overrides(Arc::new(FixedOffsetSource(self.offset_source)));
        state.0 = Some(highlighter);
        Ok(())
    }
}

impl HLImpl for UnifiedHLImpl {
    fn with_top_docs(
        &self,
        outer: &Outer,
        prevent_optimize_away: &AtomicI32,
        searcher: &Arc<IndexSearcher>,
        q: &Arc<dyn Query>,
        hits: &TopDocs,
    ) -> Result<()> {
        self.reset(outer, searcher)?;
        let state = jdk::lock(&self.state);
        let highlighter = state.0.as_ref().ok_or_else(LuceneError::null_pointer)?;
        let result = highlighter.highlight_fields_top_docs_max_passages(&self.fields, q, hits, &self.max_passages)?;
        prevent_optimize_away.store(result.len() as i32, Ordering::SeqCst);
        Ok(())
    }
}

/// Equivalent to
/// `org.apache.lucene.benchmark.byTask.tasks.SearchTravRetHighlightTask.NoHLImpl`
/// (`private class NoHLImpl implements HLImpl`).
pub(crate) struct NoHLImpl;

impl HLImpl for NoHLImpl {
    fn with_top_docs(
        &self,
        outer: &Outer,
        prevent_optimize_away: &AtomicI32,
        searcher: &Arc<IndexSearcher>,
        _q: &Arc<dyn Query>,
        hits: &TopDocs,
    ) -> Result<()> {
        // just retrieve the HL fields
        let stored_fields = searcher.stored_fields()?;
        for score_doc in doc_id_order(&hits.score_docs) {
            let has_next = !stored_fields.document_fields(score_doc.doc, &outer.hl_fields)?.get_fields().is_empty();
            prevent_optimize_away.fetch_add(if has_next { 2 } else { 1 }, Ordering::SeqCst);
        }
        Ok(())
    }
}

/// The fields `setParams` and `setup` set.
#[derive(Clone)]
struct State {
    outer: Outer,
    type_: Option<String>,
    hl_impl: Option<Arc<dyn HLImpl>>,
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.SearchTravRetHighlightTask`
/// (`public class SearchTravRetHighlightTask extends SearchTravTask`): search
/// and Traverse and Retrieve docs task. Highlight the fields in the retrieved
/// documents.
///
/// Note: This task reuses the reader if it is already open. Otherwise a
/// reader is opened at start and closed at the end.
///
/// Takes optional multivalued, comma separated param string as:
/// type[<enum>],maxFrags[<int>],fields[name1;name2;...]
///
/// * type - the highlighter implementation, e.g. "UH"
/// * maxFrags - The maximum number of fragments to score by the highlighter
/// * fields - The fields to highlight. If not specified all fields will be
///   highlighted (or at least attempted)
///
/// Example:
///
/// ```text
/// "SearchHlgtSameRdr" SearchTravRetHighlight(type[UH],maxFrags[3],fields[body]) > : 1000
/// ```
///
/// Documents must be stored in order for this task to work. Additionally,
/// term vector positions can be used as well, and offsets in postings is
/// another option.
///
/// Other side effects: counts additional 1 (record) for each traversed hit,
/// and 1 more for each retrieved (non null) document and 1 for each fragment
/// returned.
pub struct SearchTravRetHighlightTask {
    base: PerfTaskBase,
    read: ReadTaskBase,
    trav: SearchTravState,
    state: Mutex<State>,
    prevent_optimize_away: Arc<AtomicI32>,
}

impl SearchTravRetHighlightTask {
    /// `SearchTravRetHighlightTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor or of the query maker.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        let base = PerfTaskBase::new(Arc::clone(&run_data), "SearchTravRetHighlightTask")?;
        let read = ReadTaskBase::new(true, || query_maker_of::<SearchTravRetHighlightTask>(&run_data))?;
        Ok(SearchTravRetHighlightTask {
            base,
            read,
            trav: SearchTravState::new(),
            state: Mutex::new(State {
                outer: Outer {
                    max_doc_chars_to_analyze: 0,
                    max_frags: 1,
                    hl_fields: HashSet::from(["body".to_string()]),
                    analyzer: None,
                },
                type_: None,
                hl_impl: None,
            }),
            prevent_optimize_away: Arc::new(AtomicI32::new(0)),
        })
    }
}

/// `split.substring(prefix.length(), split.length() - 1)`.
fn bracket_value<'a>(split: &'a str, prefix: &str) -> Result<&'a str> {
    let end = split.len().checked_sub(1).filter(|&e| e >= prefix.len()).ok_or_else(|| {
        LuceneError::unchecked(
            "java.lang.StringIndexOutOfBoundsException",
            format!("begin {}, end {}, length {}", prefix.len(), split.len() as i64 - 1, split.len()),
        )
    })?;
    Ok(&split[prefix.len()..end])
}

impl PerfTask for SearchTravRetHighlightTask {
    fn base(&self) -> &PerfTaskBase {
        &self.base
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn do_logic(&self) -> Result<i32> {
        read_task::do_logic(self)
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        matches!(simple_class_name, "SearchTravRetHighlightTask" | "SearchTravTask" | "ReadTask")
    }

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(SearchTravRetHighlightTask {
            base: self.base.clone_base(),
            read: self.read.clone_base(),
            trav: self.trav.clone_state(),
            state: Mutex::new(jdk::lock(&self.state).clone()),
            prevent_optimize_away: Arc::clone(&self.prevent_optimize_away),
        }))
    }

    fn set_params(&self, params: &str) -> Result<()> {
        // can't call super because super doesn't understand our params syntax
        self.base.store_params(params);
        // TODO consider instead using data.getConfig().get("highlighter.*")?
        let mut state = jdk::lock(&self.state);
        for split in jdk::split(params, ",") {
            if split.starts_with("type[") {
                state.type_ = Some(bracket_value(&split, "type[")?.to_string());
            } else if split.starts_with("maxFrags[") {
                state.outer.max_frags = jdk::parse_float(bracket_value(&split, "maxFrags[")?)? as i32;
            } else if split.starts_with("fields[") {
                let field_names = bracket_value(&split, "fields[")?;
                state.outer.hl_fields = jdk::split(field_names, ";").into_iter().collect();
            }
        }
        Ok(())
    }

    fn supports_params(&self) -> bool {
        true
    }

    fn setup(&self) -> Result<()> {
        read_task::setup(self)?;
        // check to make sure either the doc is being stored
        let data = self.base.get_run_data();
        if !data.get_config().get_bool("doc.stored", false)? {
            return Err(jdk::exception("doc.stored must be set to true"));
        }
        let mut state = jdk::lock(&self.state);
        state.outer.max_doc_chars_to_analyze =
            data.get_config().get_int("highlighter.maxDocCharsToAnalyze", Highlighter::DEFAULT_MAX_CHARS_TO_ANALYZE)?;
        state.outer.analyzer = data.get_analyzer();
        let type_ = match state.type_.clone() {
            Some(t) => Some(t),
            None => data.get_config().get_str("highlighter", None)?,
        };
        // `switch (type)` on a null String throws NullPointerException.
        let type_ = type_.ok_or_else(LuceneError::null_pointer)?;
        let hl_impl: Arc<dyn HLImpl> = match type_.as_str() {
            "NONE" => Arc::new(NoHLImpl),
            "SH_A" => Arc::new(StandardHLImpl::new(false, state.outer.max_doc_chars_to_analyze)?),
            "SH_V" => Arc::new(StandardHLImpl::new(true, state.outer.max_doc_chars_to_analyze)?),
            "FVH_V" => Arc::new(FastVectorHLImpl::new()),
            "UH" => Arc::new(UnifiedHLImpl::new(None, &state.outer)),
            "UH_A" => Arc::new(UnifiedHLImpl::new(Some(OffsetSource::ANALYSIS), &state.outer)),
            "UH_V" => Arc::new(UnifiedHLImpl::new(Some(OffsetSource::TERM_VECTORS), &state.outer)),
            "UH_P" => Arc::new(UnifiedHLImpl::new(Some(OffsetSource::POSTINGS), &state.outer)),
            "UH_PV" => Arc::new(UnifiedHLImpl::new(Some(OffsetSource::POSTINGS_WITH_TERM_VECTORS), &state.outer)),
            _ => return Err(jdk::exception(format!("unrecognized highlighter type: {type_} (try 'UH')"))),
        };
        state.hl_impl = Some(hl_impl);
        Ok(())
    }
}

impl ReadTask for SearchTravRetHighlightTask {
    fn read_base(&self) -> &ReadTaskBase {
        &self.read
    }

    fn get_query_maker(&self) -> Result<Option<Arc<dyn QueryMaker>>> {
        self.base.get_run_data().get_query_maker(self).map(Some)
    }

    fn with_retrieve(&self) -> bool {
        false
    }

    fn with_search(&self) -> bool {
        true
    }

    fn with_traverse(&self) -> bool {
        true
    }

    fn with_warm(&self) -> bool {
        false
    }

    fn traversal_size(&self) -> i32 {
        self.trav.traversal_size.load(Ordering::SeqCst)
    }

    /// Here is where we intercept ReadTask's logic to do the highlighting,
    /// and nothing else (no retrieval of all field vals).
    fn with_top_docs(&self, searcher: &Arc<IndexSearcher>, q: &Arc<dyn Query>, hits: &TopDocs) -> Result<i32> {
        let (outer, hl_impl) = {
            let state = jdk::lock(&self.state);
            (state.outer.clone(), state.hl_impl.clone())
        };
        hl_impl.ok_or_else(LuceneError::null_pointer)?.with_top_docs(
            &outer,
            &self.prevent_optimize_away,
            searcher,
            q,
            hits,
        )?;
        // note: it'd be nice if we knew the sum kilobytes of text across these hits so we could return
        // that. It'd be a more
        //  useful number to gauge the amount of work. But given "average" document sizes and lots of
        // queries, returning the
        //  number of docs is reasonable.
        Ok(hits.score_docs.len() as i32) // always return # scored docs.
    }
}
