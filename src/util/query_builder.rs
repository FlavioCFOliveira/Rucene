//! Query construction from the analysis chain, ported from
//! `org.apache.lucene.util.QueryBuilder`.
//!
//! Example usage:
//!
//! ```ignore
//! let builder = QueryBuilder::new(analyzer);
//! let a = builder.create_boolean_query("body", "just a test")?;
//! let b = builder.create_phrase_query("body", "another test")?;
//! let c = builder.create_min_should_match_query("body", "another test", 0.5)?;
//! ```
//!
//! # Divergences from Lucene 10.5.0
//!
//! * **The subclassing surface is a trait.** Java documents this class as "also
//!   a subclass for query parsers", with `protected` factory methods
//!   (`newTermQuery`, `newSynonymQuery`, `newGraphSynonymQuery`,
//!   `newBooleanQuery`, `newMultiPhraseQueryBuilder`) a parser overrides to
//!   customise what it produces. Rust has no inheritance, so those methods and
//!   the algorithm that calls them live on [`QueryBuilderOps`], which
//!   [`QueryBuilder`] implements with Lucene's bodies. A customising parser
//!   implements the trait over its own type and overrides only the factories it
//!   cares about, which is the extension point Java's `protected` gives. This is
//!   the state-struct-plus-ops-trait shape the crate already uses for Lucene's
//!   abstract classes.
//! * **A missing `TermToBytesRefAttribute` and an empty stream both return
//!   `None`.** Java returns a `null` `Query` in both cases; `Option` says the
//!   same thing.
//! * **Java's unchecked exceptions become `Result`.** `createFieldQuery` wraps
//!   an `IOException` from the analysis chain in a `RuntimeException`; here it
//!   propagates as [`LuceneError`]. The argument checks that Java raises as
//!   `IllegalArgumentException` return [`LuceneError::IllegalArgument`].
//! * **Attributes are read through the concrete implementations.** Java asks the
//!   stream for an interface (`getAttribute(TermToBytesRefAttribute.class)`);
//!   this crate's `AttributeSource` is keyed by implementation type, so the
//!   helpers below try [`PackedTokenAttributeImpl`] — the implementation the
//!   default factory installs for all four attributes — and fall back to the
//!   stand-alone implementations. The answer is the same one Java's lookup
//!   gives for any stream built by this crate's analysis chain.

use std::sync::Arc;

use crate::analysis::tokenattributes::{
    BytesTermAttributeImpl, PackedTokenAttributeImpl, PositionIncrementAttribute,
    PositionIncrementAttributeImpl, PositionLengthAttribute, PositionLengthAttributeImpl,
    TermToBytesRefAttribute,
};
use crate::analysis::{Analyzer, CachingTokenFilter, SharedTokenStream, TokenStream};
use crate::error::{LuceneError, Result};
use crate::index::Term;
use crate::search::boost_attribute::{boost_of, DEFAULT_BOOST};
use crate::search::{
    boolean_query, multi_phrase_query, phrase_query, synonym_query, BooleanClause, BooleanQuery,
    BoostQuery, Occur, Query, TermQuery,
};
use crate::util::graph::GraphTokenStreamFiniteStrings;
use crate::util::AttributeSource;
use crate::util::BytesRef;

/// Wraps a term and a boost.
///
/// Equivalent to the `org.apache.lucene.util.QueryBuilder.TermAndBoost` record.
/// Note this is a different type from `SynonymQuery`'s record of the same name,
/// exactly as in Java: this one carries the raw term bytes, that one a `Term`.
#[derive(Clone, Debug, PartialEq)]
pub struct TermAndBoost {
    term: BytesRef,
    boost: f32,
}

impl TermAndBoost {
    /// Creates a new `TermAndBoost`, deep-copying `term`.
    ///
    /// The copy is Lucene's: the record's compact constructor calls
    /// `BytesRef.deepCopyOf`, because the bytes come from a token stream
    /// attribute that the next token overwrites.
    pub fn new(term: &BytesRef, boost: f32) -> Self {
        Self {
            term: BytesRef::deep_copy_of(term),
            boost,
        }
    }

    /// Returns the term bytes.
    pub fn term(&self) -> &BytesRef {
        &self.term
    }

    /// Returns the boost.
    pub fn boost(&self) -> f32 {
        self.boost
    }
}

/// Creates queries from the [`Analyzer`] chain.
///
/// Port of `org.apache.lucene.util.QueryBuilder`. The algorithm and the
/// overridable factories live on [`QueryBuilderOps`], which this type
/// implements; see the module documentation for why.
#[derive(Debug)]
pub struct QueryBuilder {
    analyzer: Arc<dyn Analyzer>,
    enable_position_increments: bool,
    enable_graph_queries: bool,
    auto_generate_multi_term_synonyms_phrase_query: bool,
}

impl QueryBuilder {
    /// Creates a new query builder using the given analyzer.
    ///
    /// Equivalent to `new QueryBuilder(Analyzer)`.
    pub fn new(analyzer: Arc<dyn Analyzer>) -> Self {
        Self {
            analyzer,
            enable_position_increments: true,
            enable_graph_queries: true,
            auto_generate_multi_term_synonyms_phrase_query: false,
        }
    }

    /// Returns the analyzer.
    ///
    /// Equivalent to `getAnalyzer()`.
    pub fn get_analyzer(&self) -> &Arc<dyn Analyzer> {
        &self.analyzer
    }

    /// Sets the analyzer used to tokenize text.
    ///
    /// Equivalent to `setAnalyzer(Analyzer)`.
    pub fn set_analyzer(&mut self, analyzer: Arc<dyn Analyzer>) {
        self.analyzer = analyzer;
    }

    /// Returns true if position increments are enabled.
    ///
    /// Equivalent to `getEnablePositionIncrements()`.
    pub fn get_enable_position_increments(&self) -> bool {
        self.enable_position_increments
    }

    /// Set to `true` to enable position increments in the result query.
    ///
    /// When set, the resulting phrase and multi-phrase queries are aware of
    /// position increments — useful when, for example, a stop filter raises the
    /// position increment of the token that follows an omitted one.
    ///
    /// Default: `true`. Equivalent to `setEnablePositionIncrements(boolean)`.
    pub fn set_enable_position_increments(&mut self, enable: bool) {
        self.enable_position_increments = enable;
    }

    /// Returns true if a phrase query is generated automatically for
    /// multi-term synonyms.
    ///
    /// Equivalent to `getAutoGenerateMultiTermSynonymsPhraseQuery()`.
    pub fn get_auto_generate_multi_term_synonyms_phrase_query(&self) -> bool {
        self.auto_generate_multi_term_synonyms_phrase_query
    }

    /// Set to `true` if phrase queries should be generated automatically for
    /// multi-term synonyms. Default: `false`.
    ///
    /// Equivalent to `setAutoGenerateMultiTermSynonymsPhraseQuery(boolean)`.
    pub fn set_auto_generate_multi_term_synonyms_phrase_query(&mut self, enable: bool) {
        self.auto_generate_multi_term_synonyms_phrase_query = enable;
    }

    /// Enables or disables graph token-stream processing. Enabled by default.
    ///
    /// Equivalent to `setEnableGraphQueries(boolean)`.
    pub fn set_enable_graph_queries(&mut self, v: bool) {
        self.enable_graph_queries = v;
    }

    /// Returns true if graph token-stream processing is enabled.
    ///
    /// Equivalent to `getEnableGraphQueries()`.
    pub fn get_enable_graph_queries(&self) -> bool {
        self.enable_graph_queries
    }
}

/// The query-construction algorithm of [`QueryBuilder`], with the factory
/// methods a query parser customises.
///
/// This is the Rust rendering of Java's `protected` surface; see the module
/// documentation. Every method carries Lucene's own body as its default, so an
/// implementation that overrides nothing behaves exactly as `QueryBuilder`
/// does.
pub trait QueryBuilderOps {
    /// Returns the builder state this implementation works over.
    fn query_builder(&self) -> &QueryBuilder;

    // -----------------------------------------------------------------------
    // Public entry points
    // -----------------------------------------------------------------------

    /// Creates a boolean query from the query text.
    ///
    /// Equivalent to `createBooleanQuery(String, String)`, which is
    /// `createBooleanQuery(field, queryText, Occur.SHOULD)`.
    ///
    /// # Errors
    ///
    /// Propagates any error from the analysis chain.
    fn create_boolean_query(
        &self,
        field: &str,
        query_text: &str,
    ) -> Result<Option<Arc<dyn Query>>> {
        self.create_boolean_query_with_operator(field, query_text, Occur::SHOULD)
    }

    /// Creates a boolean query from the query text, with an explicit operator
    /// between the analyzer's tokens.
    ///
    /// Equivalent to `createBooleanQuery(String, String, BooleanClause.Occur)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] unless `operator` is
    /// [`Occur::SHOULD`] or [`Occur::MUST`], and propagates any error from the
    /// analysis chain.
    fn create_boolean_query_with_operator(
        &self,
        field: &str,
        query_text: &str,
        operator: Occur,
    ) -> Result<Option<Arc<dyn Query>>> {
        if operator != Occur::SHOULD && operator != Occur::MUST {
            return Err(LuceneError::IllegalArgument(
                "invalid operator: only SHOULD or MUST are allowed".to_string(),
            ));
        }
        self.create_field_query_from_text(operator, field, query_text, false, 0)
    }

    /// Creates a phrase query from the query text.
    ///
    /// Equivalent to `createPhraseQuery(String, String)`, which is
    /// `createPhraseQuery(field, queryText, 0)`.
    ///
    /// # Errors
    ///
    /// Propagates any error from the analysis chain.
    fn create_phrase_query(&self, field: &str, query_text: &str) -> Result<Option<Arc<dyn Query>>> {
        self.create_phrase_query_with_slop(field, query_text, 0)
    }

    /// Creates a phrase query from the query text, permitting `phrase_slop`
    /// other words between the words of the phrase.
    ///
    /// Equivalent to `createPhraseQuery(String, String, int)`.
    ///
    /// # Errors
    ///
    /// Propagates any error from the analysis chain.
    fn create_phrase_query_with_slop(
        &self,
        field: &str,
        query_text: &str,
        phrase_slop: i32,
    ) -> Result<Option<Arc<dyn Query>>> {
        self.create_field_query_from_text(Occur::MUST, field, query_text, true, phrase_slop)
    }

    /// Creates a minimum-should-match query from the query text, requiring
    /// `fraction` of the query's terms to match.
    ///
    /// Equivalent to `createMinShouldMatchQuery(String, String, float)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] unless `fraction` is in
    /// `[0, 1]`, and propagates any error from the analysis chain.
    fn create_min_should_match_query(
        &self,
        field: &str,
        query_text: &str,
        fraction: f32,
    ) -> Result<Option<Arc<dyn Query>>> {
        if fraction.is_nan() || fraction < 0.0 || fraction > 1.0 {
            return Err(LuceneError::IllegalArgument(
                "fraction should be >= 0 and <= 1".to_string(),
            ));
        }

        // TODO: weird that BQ equals/rewrite/scorer doesn't handle this?
        if fraction == 1.0 {
            return self.create_boolean_query_with_operator(field, query_text, Occur::MUST);
        }

        let query =
            self.create_field_query_from_text(Occur::SHOULD, field, query_text, false, 0)?;
        let Some(query) = query else {
            return Ok(None);
        };
        if let Some(boolean) = query.as_any().downcast_ref::<BooleanQuery>() {
            let rebuilt = add_min_should_match_to_boolean(boolean, fraction)?;
            return Ok(Some(Arc::new(rebuilt)));
        }
        Ok(Some(query))
    }

    // -----------------------------------------------------------------------
    // The algorithm
    // -----------------------------------------------------------------------

    /// Creates a query from the analysis chain.
    ///
    /// Expert: this is more useful for a query parser built on this trait. Using
    /// the builder directly, prefer [`create_boolean_query`] and
    /// [`create_phrase_query`]. It is a complex method and usually not the one
    /// to override; override [`new_boolean_query`] and friends instead.
    ///
    /// Equivalent to
    /// `createFieldQuery(Analyzer, Occur, String, String, boolean, int)`.
    ///
    /// [`create_boolean_query`]: Self::create_boolean_query
    /// [`create_phrase_query`]: Self::create_phrase_query
    /// [`new_boolean_query`]: Self::new_boolean_query
    ///
    /// # Errors
    ///
    /// Propagates any error from the analysis chain.
    fn create_field_query_from_text(
        &self,
        operator: Occur,
        field: &str,
        query_text: &str,
        quoted: bool,
        phrase_slop: i32,
    ) -> Result<Option<Arc<dyn Query>>> {
        debug_assert!(operator == Occur::SHOULD || operator == Occur::MUST);

        // Use the analyzer to get all the tokens, and then build an appropriate
        // query based on the analysis chain.
        let source = self
            .query_builder()
            .get_analyzer()
            .token_stream_from_str(field, query_text)?;
        self.create_field_query(
            Box::new(SharedTokenStream::new(source)),
            operator,
            field,
            quoted,
            phrase_slop,
        )
    }

    /// Creates a query from a token stream.
    ///
    /// Equivalent to `createFieldQuery(TokenStream, Occur, String, boolean, int)`.
    ///
    /// # Errors
    ///
    /// Propagates any error from the analysis chain.
    fn create_field_query(
        &self,
        source: Box<dyn TokenStream>,
        operator: Occur,
        field: &str,
        quoted: bool,
        phrase_slop: i32,
    ) -> Result<Option<Arc<dyn Query>>> {
        debug_assert!(operator == Occur::SHOULD || operator == Occur::MUST);

        // Build an appropriate query based on the analysis chain.
        let mut stream = CachingTokenFilter::new(source);

        if !has_term_attribute(stream.attribute_source()) {
            return Ok(None);
        }

        // phase 1: read through the stream and assess the situation: counting
        // the number of tokens/positions and marking if we have any synonyms.

        let mut num_tokens = 0usize;
        let mut position_count = 0i32;
        let mut has_synonyms = false;
        let mut is_graph = false;

        stream.reset()?;
        while stream.increment_token()? {
            num_tokens += 1;
            let position_increment = position_increment_of(stream.attribute_source());
            if position_increment != 0 {
                position_count += position_increment;
            } else {
                has_synonyms = true;
            }

            let position_length = position_length_of(stream.attribute_source());
            if self.query_builder().get_enable_graph_queries() && position_length > 1 {
                is_graph = true;
            }
        }

        // phase 2: based on token count, presence of synonyms, and options
        // formulate a single term, boolean, or phrase.

        // Java wraps the filter in a try-with-resources, so the stream is
        // closed on every path out of the method; the result is computed first
        // and returned after the close.
        let result = if num_tokens == 0 {
            Ok(None)
        } else if num_tokens == 1 {
            // single term
            self.analyze_term(field, &mut stream)
        } else if is_graph {
            // graph
            if quoted {
                self.analyze_graph_phrase(&mut stream, field, phrase_slop)
            } else {
                self.analyze_graph_boolean(field, &mut stream, operator)
            }
        } else if quoted && position_count > 1 {
            // phrase
            if has_synonyms {
                // complex phrase with synonyms
                self.analyze_multi_phrase(field, &mut stream, phrase_slop)
            } else {
                // simple phrase
                self.analyze_phrase(field, &mut stream, phrase_slop)
            }
        } else {
            // boolean
            if position_count == 1 {
                // only one position, with synonyms
                self.analyze_boolean(field, &mut stream)
            } else {
                // complex case: multiple positions
                self.analyze_multi_boolean(field, &mut stream, operator)
            }
        };
        stream.close()?;
        result
    }

    /// Creates a simple term query from the cached token-stream contents.
    ///
    /// Equivalent to `analyzeTerm(String, TokenStream)`.
    ///
    /// # Errors
    ///
    /// Propagates any error from the analysis chain.
    fn analyze_term(
        &self,
        field: &str,
        stream: &mut dyn TokenStream,
    ) -> Result<Option<Arc<dyn Query>>> {
        stream.reset()?;
        if !stream.increment_token()? {
            return Err(LuceneError::IllegalState(
                "the cached token stream produced no token where one was counted".to_string(),
            ));
        }

        let atts = stream.attribute_source();
        let term = Term::new(field, term_bytes_of(atts).unwrap_or_default());
        let boost = boost_of(atts);
        self.new_term_query(&term, boost).map(Some)
    }

    /// Creates a simple boolean query from the cached token-stream contents.
    ///
    /// Equivalent to `analyzeBoolean(String, TokenStream)`.
    ///
    /// # Errors
    ///
    /// Propagates any error from the analysis chain.
    fn analyze_boolean(
        &self,
        field: &str,
        stream: &mut dyn TokenStream,
    ) -> Result<Option<Arc<dyn Query>>> {
        stream.reset()?;
        let mut terms = Vec::new();
        while stream.increment_token()? {
            let atts = stream.attribute_source();
            terms.push(TermAndBoost::new(
                &term_bytes_of(atts).unwrap_or_default(),
                boost_of(atts),
            ));
        }

        self.new_synonym_query(field, &terms).map(Some)
    }

    /// Adds the terms collected at one position to the boolean query being
    /// built.
    ///
    /// Equivalent to `add(String, BooleanQuery.Builder, List, Occur)`.
    ///
    /// # Errors
    ///
    /// Propagates the errors of the query factories and of
    /// [`BooleanQuery`]'s clause limit.
    fn add_position(
        &self,
        field: &str,
        q: &mut boolean_query::Builder,
        current: &[TermAndBoost],
        operator: Occur,
    ) -> Result<()> {
        if current.is_empty() {
            return Ok(());
        }
        if current.len() == 1 {
            let term = Term::new(field, current[0].term().clone());
            let query = self.new_term_query(&term, current[0].boost())?;
            q.add(query, operator)?;
        } else {
            let query = self.new_synonym_query(field, current)?;
            q.add(query, operator)?;
        }
        Ok(())
    }

    /// Creates a complex boolean query from the cached token-stream contents.
    ///
    /// Equivalent to `analyzeMultiBoolean(String, TokenStream, Occur)`.
    ///
    /// # Errors
    ///
    /// Propagates any error from the analysis chain and the query factories.
    fn analyze_multi_boolean(
        &self,
        field: &str,
        stream: &mut dyn TokenStream,
        operator: Occur,
    ) -> Result<Option<Arc<dyn Query>>> {
        let mut q = self.new_boolean_query();
        let mut current_query: Vec<TermAndBoost> = Vec::new();

        stream.reset()?;
        while stream.increment_token()? {
            let atts = stream.attribute_source();
            if position_increment_of(atts) != 0 {
                self.add_position(field, &mut q, &current_query, operator)?;
                current_query.clear();
            }
            let atts = stream.attribute_source();
            current_query.push(TermAndBoost::new(
                &term_bytes_of(atts).unwrap_or_default(),
                boost_of(atts),
            ));
        }
        self.add_position(field, &mut q, &current_query, operator)?;

        Ok(Some(Arc::new(q.build())))
    }

    /// Creates a simple phrase query from the cached token-stream contents.
    ///
    /// Equivalent to `analyzePhrase(String, TokenStream, int)`.
    ///
    /// # Errors
    ///
    /// Propagates any error from the analysis chain and from
    /// [`phrase_query::Builder`].
    fn analyze_phrase(
        &self,
        field: &str,
        stream: &mut dyn TokenStream,
        slop: i32,
    ) -> Result<Option<Arc<dyn Query>>> {
        let mut builder = phrase_query::Builder::new();
        builder.set_slop(slop);

        let enable_position_increments = self.query_builder().get_enable_position_increments();
        let mut position: i32 = -1;
        let mut phrase_boost = DEFAULT_BOOST;
        stream.reset()?;
        while stream.increment_token()? {
            let atts = stream.attribute_source();
            if enable_position_increments {
                position += position_increment_of(atts);
            } else {
                position += 1;
            }
            let term = Term::new(field, term_bytes_of(atts).unwrap_or_default());
            builder.add_at(term, position)?;
            phrase_boost *= boost_of(atts);
        }
        let query = builder.build()?;
        if phrase_boost == DEFAULT_BOOST {
            return Ok(Some(Arc::new(query)));
        }
        Ok(Some(Arc::new(BoostQuery::new(
            Arc::new(query),
            phrase_boost,
        )?)))
    }

    /// Creates a complex phrase query from the cached token-stream contents.
    ///
    /// Equivalent to `analyzeMultiPhrase(String, TokenStream, int)`.
    ///
    /// # Errors
    ///
    /// Propagates any error from the analysis chain and from
    /// [`multi_phrase_query::Builder`].
    fn analyze_multi_phrase(
        &self,
        field: &str,
        stream: &mut dyn TokenStream,
        slop: i32,
    ) -> Result<Option<Arc<dyn Query>>> {
        let mut mpqb = self.new_multi_phrase_query_builder();
        mpqb.set_slop(slop)?;

        let enable_position_increments = self.query_builder().get_enable_position_increments();
        let mut position: i32 = -1;
        let mut multi_terms: Vec<Term> = Vec::new();
        stream.reset()?;
        while stream.increment_token()? {
            let atts = stream.attribute_source();
            let position_increment = position_increment_of(atts);

            if position_increment > 0 && !multi_terms.is_empty() {
                if enable_position_increments {
                    mpqb.add_at(std::mem::take(&mut multi_terms), position)?;
                } else {
                    mpqb.add_terms(std::mem::take(&mut multi_terms))?;
                }
                multi_terms.clear();
            }
            position += position_increment;
            let atts = stream.attribute_source();
            multi_terms.push(Term::new(field, term_bytes_of(atts).unwrap_or_default()));
        }

        if enable_position_increments {
            mpqb.add_at(multi_terms, position)?;
        } else {
            mpqb.add_terms(multi_terms)?;
        }
        Ok(Some(Arc::new(mpqb.build())))
    }

    /// Creates a boolean query from a graph token stream.
    ///
    /// The articulation points of the graph are visited in order and the queries
    /// created at each point are merged into the returned boolean query.
    ///
    /// Equivalent to `analyzeGraphBoolean(String, TokenStream, Occur)`.
    ///
    /// # Errors
    ///
    /// Propagates any error from the analysis chain and the query factories.
    fn analyze_graph_boolean(
        &self,
        field: &str,
        source: &mut dyn TokenStream,
        operator: Occur,
    ) -> Result<Option<Arc<dyn Query>>> {
        source.reset()?;
        let graph = GraphTokenStreamFiniteStrings::new(source)?;
        let mut builder = boolean_query::Builder::new();
        let articulation_points = graph.articulation_points()?;
        let mut last_state = 0i32;
        for i in 0..=articulation_points.len() {
            let start = last_state;
            let mut end = -1i32;
            if i < articulation_points.len() {
                end = articulation_points[i];
            }
            last_state = end;
            let positional_query: Option<Arc<dyn Query>>;
            if graph.has_side_path(start) {
                let mut queries: Vec<Arc<dyn Query>> = Vec::new();
                let mut side_paths = graph.get_finite_strings_between(start, end);
                while let Some(side_path) = side_paths.next()? {
                    let quoted = self
                        .query_builder()
                        .get_auto_generate_multi_term_synonyms_phrase_query();
                    if let Some(q) =
                        self.create_field_query(Box::new(side_path), Occur::MUST, field, quoted, 0)?
                    {
                        queries.push(q);
                    }
                }
                positional_query = self.new_graph_synonym_query(queries)?;
            } else {
                let terms: Vec<TermAndBoost> = graph
                    .get_terms(start)
                    .iter()
                    .map(|s| TermAndBoost::new(&term_bytes_of(s).unwrap_or_default(), boost_of(s)))
                    .collect();
                debug_assert!(!terms.is_empty());
                if terms.len() == 1 {
                    let term = Term::new(field, terms[0].term().clone());
                    positional_query = Some(self.new_term_query(&term, terms[0].boost())?);
                } else {
                    positional_query = Some(self.new_synonym_query(field, &terms)?);
                }
            }
            if let Some(positional_query) = positional_query {
                builder.add(positional_query, operator)?;
            }
        }
        Ok(Some(Arc::new(builder.build())))
    }

    /// Creates a graph phrase query from the token-stream contents.
    ///
    /// Equivalent to `analyzeGraphPhrase(TokenStream, String, int)`.
    ///
    /// # Errors
    ///
    /// Propagates any error from the analysis chain and the query factories.
    fn analyze_graph_phrase(
        &self,
        source: &mut dyn TokenStream,
        field: &str,
        phrase_slop: i32,
    ) -> Result<Option<Arc<dyn Query>>> {
        source.reset()?;
        let graph = GraphTokenStreamFiniteStrings::new(source)?;

        // Creates a boolean query from the graph token stream by extracting all
        // the finite strings from the graph and using them to create phrase
        // queries with the appropriate slop.
        let mut builder = boolean_query::Builder::new();
        let mut it = graph.get_finite_strings();
        while let Some(finite_string) = it.next()? {
            let query = self.create_field_query(
                Box::new(finite_string),
                Occur::MUST,
                field,
                true,
                phrase_slop,
            )?;
            if let Some(query) = query {
                builder.add(query, Occur::SHOULD)?;
            }
        }
        Ok(Some(Arc::new(builder.build())))
    }

    // -----------------------------------------------------------------------
    // The factories a query parser customises
    // -----------------------------------------------------------------------

    /// Builds a new boolean-query builder.
    ///
    /// Equivalent to the `protected newBooleanQuery()`, and intended for
    /// implementations that wish to customise the generated queries.
    fn new_boolean_query(&self) -> boolean_query::Builder {
        boolean_query::Builder::new()
    }

    /// Builds a new synonym query.
    ///
    /// Equivalent to the `protected newSynonymQuery(String, TermAndBoost[])`.
    ///
    /// # Errors
    ///
    /// Propagates the errors of `SynonymQuery`'s builder, which rejects a boost
    /// outside `(0, 1]`.
    fn new_synonym_query(&self, field: &str, terms: &[TermAndBoost]) -> Result<Arc<dyn Query>> {
        let mut builder = synonym_query::Builder::new(field);
        for t in terms {
            builder.add_bytes(t.term().clone(), t.boost())?;
        }
        Ok(Arc::new(builder.build()))
    }

    /// Builds a new graph query for multi-term synonyms.
    ///
    /// Equivalent to the `protected newGraphSynonymQuery(Iterator<Query>)`.
    ///
    /// # Errors
    ///
    /// Propagates the errors of [`BooleanQuery`]'s clause limit.
    fn new_graph_synonym_query(
        &self,
        queries: Vec<Arc<dyn Query>>,
    ) -> Result<Option<Arc<dyn Query>>> {
        let mut builder = boolean_query::Builder::new();
        for query in queries {
            builder.add(query, Occur::SHOULD)?;
        }
        let bq = builder.build();
        if bq.clauses().len() == 1 {
            return Ok(Some(Arc::clone(bq.clauses()[0].query())));
        }
        Ok(Some(Arc::new(bq)))
    }

    /// Builds a new term query.
    ///
    /// Equivalent to the `protected newTermQuery(Term, float)`.
    ///
    /// # Errors
    ///
    /// Propagates the errors of [`BoostQuery`], which rejects a boost that is
    /// negative or not finite.
    fn new_term_query(&self, term: &Term, boost: f32) -> Result<Arc<dyn Query>> {
        let q: Arc<dyn Query> = Arc::new(TermQuery::new(term.clone()));
        if boost == DEFAULT_BOOST {
            return Ok(q);
        }
        Ok(Arc::new(BoostQuery::new(q, boost)?))
    }

    /// Builds a new multi-phrase-query builder.
    ///
    /// Equivalent to the `protected newMultiPhraseQueryBuilder()`.
    fn new_multi_phrase_query_builder(&self) -> multi_phrase_query::Builder {
        multi_phrase_query::Builder::new()
    }
}

impl QueryBuilderOps for QueryBuilder {
    fn query_builder(&self) -> &QueryBuilder {
        self
    }
}

/// Rebuilds a boolean query with a new minimum-number-should-match value.
///
/// Equivalent to the private `addMinShouldMatchToBoolean(BooleanQuery, float)`.
fn add_min_should_match_to_boolean(query: &BooleanQuery, fraction: f32) -> Result<BooleanQuery> {
    let mut builder = boolean_query::Builder::new();
    builder.set_minimum_number_should_match((fraction * query.clauses().len() as f32) as i32);
    for clause in query.clauses() {
        builder.add_clause(BooleanClause::new(
            Arc::clone(clause.query()),
            clause.occur(),
        ))?;
    }
    Ok(builder.build())
}

// ---------------------------------------------------------------------------
// Attribute access
//
// Java asks the stream for an interface; this crate's AttributeSource is keyed
// by implementation type, so each helper tries the implementation the default
// factory installs for every attribute and then the stand-alone one.
// ---------------------------------------------------------------------------

/// Returns `true` when the stream carries a `TermToBytesRefAttribute`.
///
/// Equivalent to `stream.getAttribute(TermToBytesRefAttribute.class) != null`.
fn has_term_attribute(atts: &AttributeSource) -> bool {
    atts.has_attribute::<PackedTokenAttributeImpl>()
        || atts.has_attribute::<BytesTermAttributeImpl>()
}

/// Reads the current term bytes, or `None` when the stream carries no
/// `TermToBytesRefAttribute`.
fn term_bytes_of(atts: &AttributeSource) -> Option<BytesRef> {
    if let Some(att) = atts.get_attribute::<PackedTokenAttributeImpl>() {
        return Some(att.get_bytes_ref());
    }
    atts.get_attribute::<BytesTermAttributeImpl>()
        .map(|att| att.get_bytes_ref())
}

/// Reads the current position increment, defaulting to Lucene's `1` when the
/// attribute is absent — the value `addAttribute` would have created.
fn position_increment_of(atts: &AttributeSource) -> i32 {
    if let Some(att) = atts.get_attribute::<PackedTokenAttributeImpl>() {
        return att.get_position_increment();
    }
    atts.get_attribute::<PositionIncrementAttributeImpl>()
        .map_or(1, |att| att.get_position_increment())
}

/// Reads the current position length, defaulting to Lucene's `1` when the
/// attribute is absent — the value `addAttribute` would have created.
fn position_length_of(atts: &AttributeSource) -> i32 {
    if let Some(att) = atts.get_attribute::<PackedTokenAttributeImpl>() {
        return att.get_position_length();
    }
    atts.get_attribute::<PositionLengthAttributeImpl>()
        .map_or(1, |att| att.get_position_length())
}
