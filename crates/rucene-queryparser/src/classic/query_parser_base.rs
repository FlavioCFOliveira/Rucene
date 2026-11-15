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

//! The port of `QueryParserBase.java`, of the Java package
//! `org.apache.lucene.queryparser.classic`.
//!
//! Java's abstract class is rendered as the state [`QueryParserBase`] and the
//! trait [`QueryParserBaseOps`], whose provided methods carry Lucene's bodies
//! so that a parser overrides exactly the methods its Java class overrides.
//! Each overridable body is also a free function of this module, the target
//! of a subclass's `super.method(...)` call.
//!
//! A Java `String field` that may be `null` is an `Option<&str>`. A plain
//! `QueryParser` without a default field builds terms of a `null` field as
//! Lucene does ([`Term::with_nullable_field`]), which the multi-term query
//! constructors reject with `MultiTermQuery`'s
//! `NullPointerException("field must not be null")`.
//!
//! The analyzer receives that `null` field as Lucene's does
//! (`Analyzer.normalize`, and `Analyzer.tokenStream` through
//! `QueryBuilder.createFieldQuery`), so an analyzed term of a parser without
//! a default field becomes a term of a `null` field, as in Lucene.
//! `MultiFieldQueryParser`, which passes `null` on purpose, handles it before
//! the analyzer is reached, as in Lucene.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::analysis::StringCharReader;
use rucene::document::{DateTools, Resolution};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::Term;
use rucene::search::boolean_clause::BooleanClause;
use rucene::search::boolean_query::BooleanQuery;
use rucene::search::boost_query::BoostQuery;
use rucene::search::fuzzy_query::{self, FuzzyQuery};
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::multi_phrase_query::{self, MultiPhraseQuery};
use rucene::search::multi_term_query::{constant_score_blended_rewrite, RewriteMethod};
use rucene::search::phrase_query::{self, PhraseQuery};
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::regexp_query::{DefaultProvider, RegexpQuery};
use rucene::search::term_range_query::TermRangeQuery;
use rucene::search::wildcard_query::WildcardQuery;
use rucene::search::{Occur, Query};
use rucene::util::automaton::{Operations, RegExp, DEFAULT_DETERMINIZE_WORK_LIMIT};
use rucene::util::query_builder::{QueryBuilder, QueryBuilderOps};
use rucene::util::{BytesRef, BytesRefBuilder};

use super::parse_exception::ParseException;
use super::query_parser::Operator;
use super::token::TokenRef;
use super::token_mgr_error::TokenMgrError;
use crate::charstream::char_stream::CharStream;
use crate::charstream::fast_char_stream::FastCharStream;
use crate::flexible::standard::common_query_parser_configuration::CommonQueryParserConfiguration;
use crate::jdk::calendar::{self, Calendar};
use crate::jdk::date_format;
use crate::jdk::locale::Locale;
use crate::jdk::time_zone::TimeZone;

/// `CONJ_NONE`.
pub(crate) const CONJ_NONE: i32 = 0;
/// `CONJ_AND`.
pub(crate) const CONJ_AND: i32 = 1;
/// `CONJ_OR`.
pub(crate) const CONJ_OR: i32 = 2;

/// `MOD_NONE`.
pub(crate) const MOD_NONE: i32 = 0;
/// `MOD_NOT`.
pub(crate) const MOD_NOT: i32 = 10;
/// `MOD_REQ`.
pub(crate) const MOD_REQ: i32 = 11;

/// Alternative form of `QueryParser.Operator.AND`.
pub const AND_OPERATOR: Operator = Operator::And;
/// Alternative form of `QueryParser.Operator.OR`.
pub const OR_OPERATOR: Operator = Operator::Or;

/// The state of a classic query parser: the fields of Java's
/// `QueryParserBase`, and the `QueryBuilder` it extends.
///
/// Equivalent to `org.apache.lucene.queryparser.classic.QueryParserBase`,
/// whose behaviour is [`QueryParserBaseOps`].
#[derive(Debug)]
pub struct QueryParserBase {
    /// The `QueryBuilder` state Java's class extends.
    pub(crate) builder: QueryBuilder,
    /// The default operator for parsing queries.
    pub(crate) operator: Operator,
    /// The rewrite method of multi-term queries; `None` for Java's `null`,
    /// which the query constructors reject.
    pub(crate) multi_term_rewrite_method: Option<Arc<dyn RewriteMethod>>,
    pub(crate) allow_leading_wildcard: bool,
    /// `protected String field`: the default field, `None` for `null`.
    pub field: Option<String>,
    pub(crate) phrase_slop: i32,
    pub(crate) fuzzy_min_sim: f32,
    pub(crate) fuzzy_prefix_length: i32,
    /// The locale of date range parsing; `None` for Java's `null`.
    pub(crate) locale: Option<Locale>,
    /// The time zone of date range parsing; `None` for Java's `null`.
    pub(crate) time_zone: Option<TimeZone>,
    /// The default date resolution.
    pub(crate) date_resolution: Option<Resolution>,
    /// Maps field names to date resolutions; `None` until the first
    /// `setDateResolution(String, Resolution)`. A `null` value is kept as
    /// `None`, which reads back as the default resolution, as in Lucene.
    pub(crate) field_to_date_resolution: Option<HashMap<String, Option<Resolution>>>,
    pub(crate) auto_generate_phrase_queries: bool,
    pub(crate) determinize_work_limit: i32,
}

impl QueryParserBase {
    /// So the generated `QueryParser(CharStream)` won't error out.
    ///
    /// Equivalent to the `protected QueryParserBase()` constructor, which
    /// calls `super(null)`: the locale and time zone are the JVM defaults
    /// ([`Locale::get_default`], [`TimeZone::get_default`]).
    pub fn new() -> Self {
        Self {
            builder: QueryBuilder::new(None),
            operator: OR_OPERATOR,
            multi_term_rewrite_method: Some(constant_score_blended_rewrite()),
            allow_leading_wildcard: false,
            field: None,
            phrase_slop: 0,
            fuzzy_min_sim: fuzzy_query::DEFAULT_MAX_EDITS as f32,
            fuzzy_prefix_length: fuzzy_query::DEFAULT_PREFIX_LENGTH,
            locale: Some(Locale::get_default()),
            time_zone: Some(TimeZone::get_default()),
            date_resolution: None,
            field_to_date_resolution: None,
            auto_generate_phrase_queries: false,
            determinize_work_limit: DEFAULT_DETERMINIZE_WORK_LIMIT,
        }
    }
}

impl Default for QueryParserBase {
    fn default() -> Self {
        Self::new()
    }
}

/// The behaviour of Java's `QueryParserBase`: its public and protected
/// methods, each provided with Lucene's body.
///
/// Equivalent to the methods of
/// `org.apache.lucene.queryparser.classic.QueryParserBase`. The setters and
/// getters Java's class declares for `CommonQueryParserConfiguration` are
/// that trait's, implemented for every [`QueryParserBaseOps`].
pub trait QueryParserBaseOps: QueryBuilderOps {
    /// The state of the parser.
    fn query_parser_base(&self) -> &QueryParserBase;

    /// The state of the parser, for writing.
    fn query_parser_base_mut(&mut self) -> &mut QueryParserBase;

    /// Resets the input stream. Created by the generated `QueryParser`.
    ///
    /// Equivalent to the abstract `ReInit(CharStream)`.
    fn re_init(&mut self, stream: Box<dyn CharStream>);

    /// A query followed by the end of the input. Created by the generated
    /// `QueryParser`.
    ///
    /// Equivalent to the abstract `TopLevelQuery(String)`.
    ///
    /// # Errors
    ///
    /// The `ParseException` of the grammar, and the exceptions the query
    /// construction raises.
    fn top_level_query(&mut self, field: Option<&str>) -> Result<Option<Arc<dyn Query>>>;

    /// Initializes a query parser. Called by the `QueryParser` constructor.
    ///
    /// Equivalent to `init(String, Analyzer)`.
    ///
    /// # Errors
    ///
    /// As [`set_auto_generate_phrase_queries`](Self::set_auto_generate_phrase_queries).
    fn init(&mut self, f: Option<&str>, a: Option<Arc<dyn Analyzer>>) -> Result<()> {
        init(self, f, a)
    }

    /// Parses a query string, returning a query.
    ///
    /// Equivalent to `parse(String)`.
    ///
    /// # Errors
    ///
    /// The `ParseException` `Cannot parse '<query>': ...` of a syntax error,
    /// of a lexical error, or of too many boolean clauses.
    fn parse(&mut self, query: &str) -> Result<Arc<dyn Query>> {
        parse(self, query)
    }

    /// Returns the default field.
    ///
    /// Equivalent to `getField()`.
    fn get_field(&self) -> Option<&str> {
        self.query_parser_base().field.as_deref()
    }

    /// See [`set_auto_generate_phrase_queries`](Self::set_auto_generate_phrase_queries).
    ///
    /// Equivalent to the `final getAutoGeneratePhraseQueries()`.
    fn get_auto_generate_phrase_queries(&self) -> bool {
        self.query_parser_base().auto_generate_phrase_queries
    }

    /// Set to true if phrase queries will be automatically generated when
    /// the analyzer returns more than one term from whitespace delimited
    /// text. NOTE: this behavior may not be suitable for all languages.
    ///
    /// Equivalent to `setAutoGeneratePhraseQueries(boolean)`.
    ///
    /// # Errors
    ///
    /// None here; `QueryParser` overrides it to reject a combination.
    fn set_auto_generate_phrase_queries(&mut self, value: bool) -> Result<()> {
        self.query_parser_base_mut().auto_generate_phrase_queries = value;
        Ok(())
    }

    /// Sets the boolean operator of the query parser. In default mode
    /// (`OR_OPERATOR`) terms without any modifiers are considered optional.
    ///
    /// Equivalent to `setDefaultOperator(Operator)`.
    fn set_default_operator(&mut self, op: Operator) {
        self.query_parser_base_mut().operator = op;
    }

    /// Gets implicit operator setting, which will be either `AND_OPERATOR`
    /// or `OR_OPERATOR`.
    ///
    /// Equivalent to `getDefaultOperator()`.
    fn get_default_operator(&self) -> Operator {
        self.query_parser_base().operator
    }

    /// Sets the date resolution used by range queries for a specific field.
    ///
    /// Equivalent to `setDateResolution(String, DateTools.Resolution)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Field must not be null.")`.
    fn set_date_resolution_for_field(
        &mut self,
        field_name: Option<&str>,
        date_resolution: Option<Resolution>,
    ) -> Result<()> {
        let Some(field_name) = field_name else {
            return Err(LuceneError::illegal_argument("Field must not be null."));
        };
        self.query_parser_base_mut()
            .field_to_date_resolution
            .get_or_insert_with(HashMap::new)
            .insert(field_name.to_string(), date_resolution);
        Ok(())
    }

    /// Returns the date resolution that is used by RangeQueries for the
    /// given field. Returns `None`, if no default or field specific date
    /// resolution has been set for the given field.
    ///
    /// Equivalent to `getDateResolution(String)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Field must not be null.")`.
    fn get_date_resolution(&self, field_name: Option<&str>) -> Result<Option<Resolution>> {
        let Some(field_name) = field_name else {
            return Err(LuceneError::illegal_argument("Field must not be null."));
        };
        let base = self.query_parser_base();
        let Some(map) = &base.field_to_date_resolution else {
            // no field specific date resolutions set; return default date
            // resolution instead
            return Ok(base.date_resolution);
        };
        let resolution = map.get(field_name).copied().flatten();
        // no date resolutions set for the given field; return default date
        // resolution instead
        Ok(resolution.or(base.date_resolution))
    }

    /// Sets the maximum effort that determinizing a regexp or wildcard query
    /// may spend.
    ///
    /// Equivalent to `setDeterminizeWorkLimit(int)`.
    fn set_determinize_work_limit(&mut self, determinize_work_limit: i32) {
        self.query_parser_base_mut().determinize_work_limit = determinize_work_limit;
    }

    /// Equivalent to `getDeterminizeWorkLimit()`.
    fn get_determinize_work_limit(&self) -> i32 {
        self.query_parser_base().determinize_work_limit
    }

    /// Equivalent to `addClause(List<BooleanClause>, int, int, Query)`.
    ///
    /// # Errors
    ///
    /// `RuntimeException("Clause cannot be both required and prohibited")`.
    fn add_clause(
        &mut self,
        clauses: &mut Vec<BooleanClause>,
        conj: i32,
        mods: i32,
        q: Option<Arc<dyn Query>>,
    ) -> Result<()> {
        add_clause(self, clauses, conj, mods, q)
    }

    /// Adds clauses generated from analysis over text containing whitespace.
    /// There are no operators, so the query's clauses can either be MUST (if
    /// the default operator is AND) or SHOULD (default OR).
    ///
    /// If all of the clauses in the given Query are TermQuery-s, this method
    /// flattens the result by adding the TermQuery-s individually to the
    /// output clause list; otherwise, the given Query is added as a single
    /// clause including its nested clauses.
    ///
    /// Equivalent to `addMultiTermClauses(List<BooleanClause>, Query)`.
    fn add_multi_term_clauses(
        &mut self,
        clauses: &mut Vec<BooleanClause>,
        q: Option<Arc<dyn Query>>,
    ) {
        add_multi_term_clauses(self, clauses, q)
    }

    /// Equivalent to `getFieldQuery(String, String, boolean)`.
    ///
    /// # Errors
    ///
    /// The exceptions of the analysis and of the query construction.
    fn get_field_query(
        &mut self,
        field: Option<&str>,
        query_text: &str,
        quoted: bool,
    ) -> Result<Option<Arc<dyn Query>>> {
        get_field_query(self, field, query_text, quoted)
    }

    /// Equivalent to `newFieldQuery(Analyzer, String, String, boolean)`.
    ///
    /// # Errors
    ///
    /// The exceptions of the analysis and of the query construction.
    fn new_field_query(
        &mut self,
        analyzer: Option<Arc<dyn Analyzer>>,
        field: Option<&str>,
        query_text: &str,
        quoted: bool,
    ) -> Result<Option<Arc<dyn Query>>> {
        new_field_query(self, analyzer, field, query_text, quoted)
    }

    /// Base implementation delegates to
    /// [`get_field_query`](Self::get_field_query). This method may be
    /// overridden, for example, to return a SpanNearQuery instead of a
    /// PhraseQuery.
    ///
    /// Equivalent to `getFieldQuery(String, String, int)`.
    ///
    /// # Errors
    ///
    /// The exceptions of the analysis and of the query construction.
    fn get_field_query_with_slop(
        &mut self,
        field: Option<&str>,
        query_text: &str,
        slop: i32,
    ) -> Result<Option<Arc<dyn Query>>> {
        get_field_query_with_slop(self, field, query_text, slop)
    }

    /// Equivalent to `getRangeQuery(String, String, String, boolean,
    /// boolean)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of a `null` field, and the exceptions
    /// of the query construction.
    fn get_range_query(
        &mut self,
        field: Option<&str>,
        part1: Option<&str>,
        part2: Option<&str>,
        start_inclusive: bool,
        end_inclusive: bool,
    ) -> Result<Option<Arc<dyn Query>>> {
        get_range_query(self, field, part1, part2, start_inclusive, end_inclusive)
    }

    /// Builds a new BooleanClause instance.
    ///
    /// Equivalent to `newBooleanClause(Query, BooleanClause.Occur)`.
    fn new_boolean_clause(&mut self, q: Arc<dyn Query>, occur: Occur) -> BooleanClause {
        BooleanClause::new(q, occur)
    }

    /// Builds a new PrefixQuery instance.
    ///
    /// Equivalent to `newPrefixQuery(Term)`.
    ///
    /// # Errors
    ///
    /// The exceptions of the query construction.
    fn new_prefix_query(&mut self, prefix: Term) -> Result<Arc<dyn Query>> {
        new_prefix_query(self, prefix)
    }

    /// Builds a new RegexpQuery instance.
    ///
    /// Equivalent to `newRegexpQuery(Term)`.
    ///
    /// # Errors
    ///
    /// The exceptions of the regular expression and of the query
    /// construction.
    fn new_regexp_query(&mut self, regexp: Term) -> Result<Arc<dyn Query>> {
        new_regexp_query(self, regexp)
    }

    /// Builds a new FuzzyQuery instance.
    ///
    /// Equivalent to `newFuzzyQuery(Term, float, int)`.
    ///
    /// # Errors
    ///
    /// The exceptions of the query construction.
    fn new_fuzzy_query(
        &mut self,
        term: Term,
        minimum_similarity: f32,
        prefix_length: i32,
    ) -> Result<Arc<dyn Query>> {
        new_fuzzy_query(self, term, minimum_similarity, prefix_length)
    }

    /// Builds a new TermRangeQuery instance.
    ///
    /// Equivalent to `newRangeQuery(String, String, String, boolean,
    /// boolean)`.
    ///
    /// # Errors
    ///
    /// The exceptions of the normalization and of the query construction.
    fn new_range_query(
        &mut self,
        field: Option<&str>,
        part1: Option<&str>,
        part2: Option<&str>,
        start_inclusive: bool,
        end_inclusive: bool,
    ) -> Result<Arc<dyn Query>> {
        new_range_query(self, field, part1, part2, start_inclusive, end_inclusive)
    }

    /// Builds a new MatchAllDocsQuery instance.
    ///
    /// Equivalent to `newMatchAllDocsQuery()`.
    fn new_match_all_docs_query(&mut self) -> Arc<dyn Query> {
        Arc::new(MatchAllDocsQuery::instance())
    }

    /// Builds a new WildcardQuery instance.
    ///
    /// Equivalent to `newWildcardQuery(Term)`.
    ///
    /// # Errors
    ///
    /// The exceptions of the query construction.
    fn new_wildcard_query(&mut self, t: Term) -> Result<Arc<dyn Query>> {
        new_wildcard_query(self, t)
    }

    /// Factory method for generating query, given a set of clauses. By
    /// default creates a boolean query composed of clauses passed in.
    ///
    /// Equivalent to `getBooleanQuery(List<BooleanClause>)`.
    ///
    /// # Errors
    ///
    /// The `TooManyClauses` of the boolean query builder.
    fn get_boolean_query(&mut self, clauses: &[BooleanClause]) -> Result<Option<Arc<dyn Query>>> {
        get_boolean_query(self, clauses)
    }

    /// Factory method for generating a query. Called when parser parses an
    /// input term token that contains one or more wildcard characters (`?`
    /// and `*`), but is not a prefix term token (one that has just a single
    /// `*` character at the end).
    ///
    /// Equivalent to `getWildcardQuery(String, String)`.
    ///
    /// # Errors
    ///
    /// The `ParseException` of a leading wildcard where none is allowed.
    fn get_wildcard_query(
        &mut self,
        field: Option<&str>,
        term_str: &str,
    ) -> Result<Option<Arc<dyn Query>>> {
        get_wildcard_query(self, field, term_str)
    }

    /// Factory method for generating a query. Called when parser parses an
    /// input term token that contains a regular expression query.
    ///
    /// Equivalent to `getRegexpQuery(String, String)`.
    ///
    /// # Errors
    ///
    /// The exceptions of the normalization and of the query construction.
    fn get_regexp_query(
        &mut self,
        field: Option<&str>,
        term_str: &str,
    ) -> Result<Option<Arc<dyn Query>>> {
        get_regexp_query(self, field, term_str)
    }

    /// Factory method for generating a query (similar to
    /// [`get_wildcard_query`](Self::get_wildcard_query)). Called when parser
    /// parses an input term token that uses prefix notation; that is,
    /// contains a single `*` wildcard character as its last character.
    ///
    /// Equivalent to `getPrefixQuery(String, String)`.
    ///
    /// # Errors
    ///
    /// The `ParseException` of a leading wildcard where none is allowed.
    fn get_prefix_query(
        &mut self,
        field: Option<&str>,
        term_str: &str,
    ) -> Result<Option<Arc<dyn Query>>> {
        get_prefix_query(self, field, term_str)
    }

    /// Factory method for generating a query (similar to
    /// [`get_wildcard_query`](Self::get_wildcard_query)). Called when parser
    /// parses an input term token that has the fuzzy suffix (`~`) appended.
    ///
    /// Equivalent to `getFuzzyQuery(String, String, float)`.
    ///
    /// # Errors
    ///
    /// The exceptions of the normalization and of the query construction.
    fn get_fuzzy_query(
        &mut self,
        field: Option<&str>,
        term_str: &str,
        min_similarity: f32,
    ) -> Result<Option<Arc<dyn Query>>> {
        get_fuzzy_query(self, field, term_str, min_similarity)
    }

    /// Extracted from the `.jj` grammar.
    ///
    /// Equivalent to the package-private `handleBareTokenQuery(String,
    /// Token, Token, boolean, boolean, boolean, boolean)`.
    ///
    /// # Errors
    ///
    /// The exceptions of the factory method it calls.
    #[allow(clippy::too_many_arguments)]
    fn handle_bare_token_query(
        &mut self,
        qfield: Option<&str>,
        term: &TokenRef,
        fuzzy_slop: Option<&TokenRef>,
        prefix: bool,
        wildcard: bool,
        fuzzy: bool,
        regexp: bool,
    ) -> Result<Option<Arc<dyn Query>>> {
        handle_bare_token_query(
            self, qfield, term, fuzzy_slop, prefix, wildcard, fuzzy, regexp,
        )
    }

    /// Equivalent to `getFuzzyDistance(Token, String)`: the similarity the
    /// fuzzy token gives, or the default when it does not parse.
    fn get_fuzzy_distance(&mut self, fuzzy_token: Option<&TokenRef>, term_str: &str) -> f32 {
        let _ = term_str;
        // `try { return Float.parseFloat(fuzzyToken.image.substring(1)); }
        // catch (Exception ignored) {}`
        // Every error these calls raise is an `Exception`.
        let parsed = (|| -> Result<f32> {
            let token = crate::javacc::nn(fuzzy_token)?;
            let image = token.lock().image.clone();
            let image = crate::javacc::nn_str(image.as_deref())?;
            crate::javacc::parse_float(&crate::javacc::substring_from(image, 1)?)
        })();
        parsed.unwrap_or(self.query_parser_base().fuzzy_min_sim)
    }

    /// Equivalent to the package-private `handleBareFuzzy(String, Token,
    /// String)`.
    ///
    /// # Errors
    ///
    /// The `ParseException` of a similarity out of range or fractional above
    /// one.
    fn handle_bare_fuzzy(
        &mut self,
        qfield: Option<&str>,
        fuzzy_slop: Option<&TokenRef>,
        term_image: &str,
    ) -> Result<Option<Arc<dyn Query>>> {
        handle_bare_fuzzy(self, qfield, fuzzy_slop, term_image)
    }

    /// Extracted from the `.jj` grammar.
    ///
    /// Equivalent to the package-private `handleQuotedTerm(String, Token,
    /// Token)`.
    ///
    /// # Errors
    ///
    /// The exceptions of the escape handling and of the factory method.
    fn handle_quoted_term(
        &mut self,
        qfield: Option<&str>,
        term: &TokenRef,
        fuzzy_slop: Option<&TokenRef>,
    ) -> Result<Option<Arc<dyn Query>>> {
        handle_quoted_term(self, qfield, term, fuzzy_slop)
    }

    /// Extracted from the `.jj` grammar.
    ///
    /// Equivalent to the package-private `handleBoost(Query, Token)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `new BoostQuery` for a boost that
    /// parses to an infinite float.
    fn handle_boost(
        &mut self,
        q: Option<Arc<dyn Query>>,
        boost: Option<&TokenRef>,
    ) -> Result<Option<Arc<dyn Query>>> {
        handle_boost(self, q, boost)
    }

    /// Returns a String where the escape char has been removed, or kept
    /// only once if there was a double escape.
    ///
    /// Supports escaped Unicode characters, e.g. translates `\\u0041` to
    /// `A`.
    ///
    /// Equivalent to the package-private `discardEscapeChar(String)`.
    ///
    /// # Errors
    ///
    /// The `ParseException` of a truncated or non-hexadecimal Unicode escape
    /// and of a trailing escape character.
    fn discard_escape_char(&mut self, input: &str) -> Result<String> {
        discard_escape_char(input)
    }
}

/// `QueryParserBase.init(String, Analyzer)`: the body of
/// [`QueryParserBaseOps::init`].
///
/// # Errors
///
/// As [`QueryParserBaseOps::set_auto_generate_phrase_queries`].
pub fn init<P: QueryParserBaseOps + ?Sized>(
    this: &mut P,
    f: Option<&str>,
    a: Option<Arc<dyn Analyzer>>,
) -> Result<()> {
    this.query_parser_base_mut().builder.set_analyzer(a);
    this.query_parser_base_mut().field = f.map(str::to_string);
    this.set_auto_generate_phrase_queries(false)
}

/// `QueryParserBase.parse(String)`: the body of [`QueryParserBaseOps::parse`].
///
/// # Errors
///
/// As [`QueryParserBaseOps::parse`].
pub fn parse<P: QueryParserBaseOps + ?Sized>(this: &mut P, query: &str) -> Result<Arc<dyn Query>> {
    this.re_init(Box::new(FastCharStream::new(Box::new(
        StringCharReader::new(query),
    ))));
    // TopLevelQuery is a Query followed by the end-of-input (EOF)
    let field = this.query_parser_base().field.clone();
    match this.top_level_query(field.as_deref()) {
        Ok(Some(res)) => Ok(res),
        Ok(None) => Ok(Arc::new(this.new_boolean_query().build())),
        Err(tme) => {
            let message = if let Some(pe) = ParseException::of(&tme) {
                Some(crate::javacc::string_value_of(pe.get_message()))
            } else {
                TokenMgrError::of(&tme).map(|t| crate::javacc::string_value_of(t.get_message()))
            };
            if let Some(message) = message {
                // rethrow to include the original query:
                let mut e =
                    ParseException::with_message(format!("Cannot parse '{query}': {message}"));
                e.init_cause(tme);
                return Err(e.error());
            }
            if matches!(
                tme.kind(),
                LuceneErrorKind::TooManyClauses(_) | LuceneErrorKind::TooManyNestedClauses(_)
            ) {
                let mut e = ParseException::with_message(format!(
                    "Cannot parse '{query}': too many boolean clauses"
                ));
                e.init_cause(tme);
                return Err(e.error());
            }
            Err(tme)
        }
    }
}

/// `QueryParserBase.addClause`: the body of [`QueryParserBaseOps::add_clause`].
///
/// # Errors
///
/// As [`QueryParserBaseOps::add_clause`].
pub fn add_clause<P: QueryParserBaseOps + ?Sized>(
    this: &mut P,
    clauses: &mut Vec<BooleanClause>,
    conj: i32,
    mods: i32,
    q: Option<Arc<dyn Query>>,
) -> Result<()> {
    // If this term is introduced by AND, make the preceding term required,
    // unless it's already prohibited
    if !clauses.is_empty() && conj == CONJ_AND {
        let last = clauses.len() - 1;
        let c = &clauses[last];
        if !c.is_prohibited() {
            clauses[last] = BooleanClause::new(Arc::clone(c.query()), Occur::MUST);
        }
    }

    let operator = this.query_parser_base().operator;
    if !clauses.is_empty() && operator == AND_OPERATOR && conj == CONJ_OR {
        // If this term is introduced by OR, make the preceding term optional,
        // unless it's prohibited (that means we leave -a OR b but +a OR
        // b-->a OR b) notice if the input is a OR b, first term is parsed as
        // required; without this modification a OR b would be parsed as
        // +a OR b
        let last = clauses.len() - 1;
        let c = &clauses[last];
        if !c.is_prohibited() {
            clauses[last] = BooleanClause::new(Arc::clone(c.query()), Occur::SHOULD);
        }
    }

    // We might have been passed a null query; the term might have been
    // filtered away by the analyzer.
    let Some(q) = q else {
        return Ok(());
    };

    let (required, prohibited) = if operator == OR_OPERATOR {
        // We set REQUIRED if we're introduced by AND or +; PROHIBITED if
        // introduced by NOT or -; make sure not to set both.
        let prohibited = mods == MOD_NOT;
        (
            mods == MOD_REQ || (conj == CONJ_AND && !prohibited),
            prohibited,
        )
    } else {
        // We set PROHIBITED if we're introduced by NOT or -; We set REQUIRED
        // if not PROHIBITED and not introduced by OR
        let prohibited = mods == MOD_NOT;
        (!prohibited && conj != CONJ_OR, prohibited)
    };
    if required && !prohibited {
        let clause = this.new_boolean_clause(q, Occur::MUST);
        clauses.push(clause);
    } else if !required && !prohibited {
        let clause = this.new_boolean_clause(q, Occur::SHOULD);
        clauses.push(clause);
    } else if !required && prohibited {
        let clause = this.new_boolean_clause(q, Occur::MUST_NOT);
        clauses.push(clause);
    } else {
        return Err(LuceneError::unchecked(
            "java.lang.RuntimeException",
            "Clause cannot be both required and prohibited",
        ));
    }
    Ok(())
}

/// `QueryParserBase.addMultiTermClauses`: the body of
/// [`QueryParserBaseOps::add_multi_term_clauses`].
pub fn add_multi_term_clauses<P: QueryParserBaseOps + ?Sized>(
    this: &mut P,
    clauses: &mut Vec<BooleanClause>,
    q: Option<Arc<dyn Query>>,
) {
    // We might have been passed a null query; the term might have been
    // filtered away by the analyzer.
    let Some(q) = q else {
        return;
    };
    let boolean = q.as_any().downcast_ref::<BooleanQuery>();
    let mut all_nested_term_queries = false;
    if let Some(bq) = boolean {
        all_nested_term_queries = true;
        for clause in bq.clauses() {
            if clause
                .query()
                .as_any()
                .downcast_ref::<rucene::search::TermQuery>()
                .is_none()
            {
                all_nested_term_queries = false;
                break;
            }
        }
    }
    if all_nested_term_queries {
        if let Some(bq) = boolean {
            clauses.extend(bq.clauses().iter().cloned());
        }
    } else {
        let occur = if this.query_parser_base().operator == OR_OPERATOR {
            Occur::SHOULD
        } else {
            Occur::MUST
        };
        if let Some(bq) = boolean {
            for clause in bq.clauses() {
                let c = this.new_boolean_clause(Arc::clone(clause.query()), occur);
                clauses.push(c);
            }
        } else {
            let c = this.new_boolean_clause(Arc::clone(&q), occur);
            clauses.push(c);
        }
    }
}

/// `QueryParserBase.getFieldQuery(String, String, boolean)`: the body of
/// [`QueryParserBaseOps::get_field_query`].
///
/// # Errors
///
/// As [`QueryParserBaseOps::get_field_query`].
pub fn get_field_query<P: QueryParserBaseOps + ?Sized>(
    this: &mut P,
    field: Option<&str>,
    query_text: &str,
    quoted: bool,
) -> Result<Option<Arc<dyn Query>>> {
    let analyzer = this.query_builder().get_analyzer().cloned();
    this.new_field_query(analyzer, field, query_text, quoted)
}

/// `QueryParserBase.newFieldQuery`: the body of
/// [`QueryParserBaseOps::new_field_query`].
///
/// # Errors
///
/// As [`QueryParserBaseOps::new_field_query`].
pub fn new_field_query<P: QueryParserBaseOps + ?Sized>(
    this: &mut P,
    analyzer: Option<Arc<dyn Analyzer>>,
    field: Option<&str>,
    query_text: &str,
    quoted: bool,
) -> Result<Option<Arc<dyn Query>>> {
    let base = this.query_parser_base();
    let occur = if base.operator == Operator::And {
        Occur::MUST
    } else {
        Occur::SHOULD
    };
    let quoted = quoted || base.auto_generate_phrase_queries;
    let phrase_slop = base.phrase_slop;
    this.create_field_query_with_analyzer(
        analyzer.as_ref(),
        occur,
        field,
        query_text,
        quoted,
        phrase_slop,
    )
}

/// `QueryParserBase.getFieldQuery(String, String, int)`: the body of
/// [`QueryParserBaseOps::get_field_query_with_slop`].
///
/// # Errors
///
/// As [`QueryParserBaseOps::get_field_query_with_slop`].
pub fn get_field_query_with_slop<P: QueryParserBaseOps + ?Sized>(
    this: &mut P,
    field: Option<&str>,
    query_text: &str,
    slop: i32,
) -> Result<Option<Arc<dyn Query>>> {
    let mut query = this.get_field_query(field, query_text, true)?;

    if let Some(q) = &query {
        if let Some(pq) = q.as_any().downcast_ref::<PhraseQuery>() {
            query = Some(Arc::new(add_slop_to_phrase(pq, slop)?));
        } else if let Some(mpq) = q.as_any().downcast_ref::<MultiPhraseQuery>() {
            if slop != mpq.get_slop() {
                query = Some(Arc::new(
                    multi_phrase_query::Builder::from_query(mpq)
                        .set_slop(slop)?
                        .build(),
                ));
            }
        }
    }

    Ok(query)
}

/// Rebuild a phrase query with a slop value.
///
/// Equivalent to the private `addSlopToPhrase(PhraseQuery, int)`.
fn add_slop_to_phrase(query: &PhraseQuery, slop: i32) -> Result<PhraseQuery> {
    let mut builder = phrase_query::Builder::new();
    builder.set_slop(slop);
    let terms = query.get_terms();
    let positions = query.get_positions();
    for i in 0..terms.len() {
        builder.add_at(terms[i].clone(), positions[i])?;
    }
    builder.build()
}

/// `QueryParserBase.getRangeQuery`: the body of
/// [`QueryParserBaseOps::get_range_query`].
///
/// # Errors
///
/// As [`QueryParserBaseOps::get_range_query`].
pub fn get_range_query<P: QueryParserBaseOps + ?Sized>(
    this: &mut P,
    field: Option<&str>,
    part1: Option<&str>,
    part2: Option<&str>,
    start_inclusive: bool,
    end_inclusive: bool,
) -> Result<Option<Arc<dyn Query>>> {
    let locale = this.query_parser_base().locale.clone();
    let time_zone = this.query_parser_base().time_zone.clone();
    // `DateFormat.getDateInstance(DateFormat.SHORT, locale)` dereferences a
    // `null` locale (`QueryParserBase.java:510`), outside the `try` blocks.
    let locale = crate::javacc::nn_owned(locale)?;
    let df = date_format::get_date_instance(date_format::SHORT, &locale)?;
    df.set_lenient(true);
    let resolution = this.get_date_resolution(field)?;

    let mut part1 = part1.map(str::to_string);
    let mut part2 = part2.map(str::to_string);

    // `try { part1 = DateTools.dateToString(df.parse(part1), resolution); }
    // catch (Exception e) {}`: a `null` part, a date that does not parse and
    // a `null` resolution each throw, and leave the part as it is.
    let converted = (|| -> Result<Option<String>> {
        let source = crate::javacc::nn_str(part1.as_deref())?;
        let date = df.parse(source)?;
        let resolution = crate::javacc::nn_owned(resolution)?;
        Ok(Some(DateTools::time_to_string(date.get_time(), resolution)))
    })();
    match converted {
        Ok(value) => part1 = value,
        Err(e) if crate::javacc::is_exception(&e) => {}
        Err(e) => return Err(e),
    }

    let converted = (|| -> Result<Option<String>> {
        let source = crate::javacc::nn_str(part2.as_deref())?;
        let mut d2 = df.parse(source)?;
        if end_inclusive {
            // The user can only specify the date, not the time, so make sure
            // the time is set to the latest possible time of that date to
            // really include all documents:
            // `Calendar.getInstance(timeZone, locale)` dereferences a `null`
            // time zone (`QueryParserBase.java:527`), inside the `try`.
            let time_zone = crate::javacc::nn(time_zone.as_ref())?;
            let mut cal = Calendar::get_instance(time_zone, &locale)?;
            cal.set_time(d2);
            cal.set(calendar::HOUR_OF_DAY, 23);
            cal.set(calendar::MINUTE, 59);
            cal.set(calendar::SECOND, 59);
            cal.set(calendar::MILLISECOND, 999);
            d2 = cal.get_time()?;
        }
        let resolution = crate::javacc::nn_owned(resolution)?;
        Ok(Some(DateTools::time_to_string(d2.get_time(), resolution)))
    })();
    match converted {
        Ok(value) => part2 = value,
        Err(e) if crate::javacc::is_exception(&e) => {}
        Err(e) => return Err(e),
    }

    this.new_range_query(
        field,
        part1.as_deref(),
        part2.as_deref(),
        start_inclusive,
        end_inclusive,
    )
    .map(Some)
}

/// `QueryParserBase.newPrefixQuery`: the body of
/// [`QueryParserBaseOps::new_prefix_query`].
///
/// # Errors
///
/// As [`QueryParserBaseOps::new_prefix_query`].
pub fn new_prefix_query<P: QueryParserBaseOps + ?Sized>(
    this: &mut P,
    prefix: Term,
) -> Result<Arc<dyn Query>> {
    // `new PrefixQuery(prefix, multiTermRewriteMethod)`: the automaton of a
    // prefix cannot fail, then `MultiTermQuery` rejects a `null` field, then
    // a `null` method.
    if prefix.field_opt().is_none() {
        return Err(null_field());
    }
    let method = rewrite_method(this.query_parser_base())?;
    Ok(Arc::new(PrefixQuery::with_rewrite_method(prefix, method)?))
}

/// `QueryParserBase.newRegexpQuery`: the body of
/// [`QueryParserBaseOps::new_regexp_query`].
///
/// # Errors
///
/// As [`QueryParserBaseOps::new_regexp_query`].
pub fn new_regexp_query<P: QueryParserBaseOps + ?Sized>(
    this: &mut P,
    regexp: Term,
) -> Result<Arc<dyn Query>> {
    let base = this.query_parser_base();
    let Some(method) = base.multi_term_rewrite_method.clone() else {
        // Java builds the automaton (`new RegExp(...)` and the private
        // `RegexpQuery.toAutomaton`, which determinizes it) as the argument of
        // the `AutomatonQuery` constructor, before `MultiTermQuery` rejects
        // the `null` method: its errors come first.
        let automaton = RegExp::with_match_flags(&regexp.text(), RegExp::ALL, 0)?
            .to_automaton_with(None, Some(&DefaultProvider))?;
        Operations::determinize(&automaton, base.determinize_work_limit)?;
        if regexp.field_opt().is_none() {
            return Err(null_field());
        }
        return Err(null_rewrite_method());
    };
    Ok(Arc::new(RegexpQuery::with_everything(
        regexp,
        RegExp::ALL,
        0,
        &DefaultProvider,
        base.determinize_work_limit,
        method,
        true,
    )?))
}

/// `QueryParserBase.newFuzzyQuery`: the body of
/// [`QueryParserBaseOps::new_fuzzy_query`].
///
/// # Errors
///
/// As [`QueryParserBaseOps::new_fuzzy_query`].
pub fn new_fuzzy_query<P: QueryParserBaseOps + ?Sized>(
    this: &mut P,
    term: Term,
    minimum_similarity: f32,
    prefix_length: i32,
) -> Result<Arc<dyn Query>> {
    let _ = this;
    // FuzzyQuery doesn't yet allow constant score rewrite
    let text = term.text();
    let num_edits = FuzzyQuery::float_to_edits(minimum_similarity, text.chars().count() as i32);
    Ok(Arc::new(FuzzyQuery::with_prefix_length(
        term,
        num_edits,
        prefix_length,
    )?))
}

/// `QueryParserBase.newRangeQuery`: the body of
/// [`QueryParserBaseOps::new_range_query`].
///
/// # Errors
///
/// As [`QueryParserBaseOps::new_range_query`].
pub fn new_range_query<P: QueryParserBaseOps + ?Sized>(
    this: &mut P,
    field: Option<&str>,
    part1: Option<&str>,
    part2: Option<&str>,
    start_inclusive: bool,
    end_inclusive: bool,
) -> Result<Arc<dyn Query>> {
    let start = match part1 {
        None => None,
        Some(part1) => Some(normalize(this, field, part1)?),
    };
    let end = match part2 {
        None => None,
        Some(part2) => Some(normalize(this, field, part2)?),
    };
    // `new TermRangeQuery(field, start, end, ..., multiTermRewriteMethod)`:
    // `MultiTermQuery` checks the field, then the method.
    let field = field.ok_or_else(null_field)?;
    let method = rewrite_method(this.query_parser_base())?;
    Ok(Arc::new(TermRangeQuery::with_rewrite_method(
        field,
        start,
        end,
        start_inclusive,
        end_inclusive,
        method,
    )?))
}

/// The `NullPointerException` of `MultiTermQuery`'s
/// `Objects.requireNonNull(field, "field must not be null")`
/// (`MultiTermQuery.java:284`).
fn null_field() -> LuceneError {
    LuceneError::null_pointer_with_message("field must not be null")
}

/// The `NullPointerException` of `MultiTermQuery`'s
/// `Objects.requireNonNull(rewriteMethod, "rewriteMethod must not be null")`.
fn null_rewrite_method() -> LuceneError {
    LuceneError::null_pointer_with_message("rewriteMethod must not be null")
}

/// The parser's rewrite method, as a multi-term query constructor takes it.
///
/// # Errors
///
/// [`null_rewrite_method`] for `None` (Java's `null`).
fn rewrite_method(base: &QueryParserBase) -> Result<Arc<dyn RewriteMethod>> {
    base.multi_term_rewrite_method
        .clone()
        .ok_or_else(null_rewrite_method)
}

/// `getAnalyzer().normalize(field, text)`.
fn normalize<P: QueryParserBaseOps + ?Sized>(
    this: &P,
    field: Option<&str>,
    text: &str,
) -> Result<BytesRef> {
    let analyzer = crate::javacc::nn(this.query_builder().get_analyzer())?;
    analyzer.normalize_nullable_field(field, text)
}

/// `QueryParserBase.newWildcardQuery`: the body of
/// [`QueryParserBaseOps::new_wildcard_query`].
///
/// # Errors
///
/// As [`QueryParserBaseOps::new_wildcard_query`].
pub fn new_wildcard_query<P: QueryParserBaseOps + ?Sized>(
    this: &mut P,
    t: Term,
) -> Result<Arc<dyn Query>> {
    let base = this.query_parser_base();
    let Some(method) = base.multi_term_rewrite_method.clone() else {
        // Java builds the automaton (`WildcardQuery.toAutomaton`) as the
        // argument of the `AutomatonQuery` constructor, before
        // `MultiTermQuery` rejects the `null` method: its errors come first.
        WildcardQuery::to_automaton(&t, base.determinize_work_limit)?;
        if t.field_opt().is_none() {
            return Err(null_field());
        }
        return Err(null_rewrite_method());
    };
    Ok(Arc::new(WildcardQuery::with_rewrite_method(
        t,
        base.determinize_work_limit,
        method,
    )?))
}

/// `QueryParserBase.getBooleanQuery`: the body of
/// [`QueryParserBaseOps::get_boolean_query`].
///
/// # Errors
///
/// As [`QueryParserBaseOps::get_boolean_query`].
pub fn get_boolean_query<P: QueryParserBaseOps + ?Sized>(
    this: &mut P,
    clauses: &[BooleanClause],
) -> Result<Option<Arc<dyn Query>>> {
    if clauses.is_empty() {
        return Ok(None); // all clause words were filtered away by the analyzer.
    }
    let mut query = this.new_boolean_query();
    for clause in clauses {
        query.add_clause(clause.clone())?;
    }
    Ok(Some(Arc::new(query.build())))
}

/// `QueryParserBase.getWildcardQuery`: the body of
/// [`QueryParserBaseOps::get_wildcard_query`].
///
/// # Errors
///
/// As [`QueryParserBaseOps::get_wildcard_query`].
pub fn get_wildcard_query<P: QueryParserBaseOps + ?Sized>(
    this: &mut P,
    field: Option<&str>,
    term_str: &str,
) -> Result<Option<Arc<dyn Query>>> {
    if field == Some("*") && term_str == "*" {
        return Ok(Some(this.new_match_all_docs_query()));
    }
    if !this.query_parser_base().allow_leading_wildcard
        && (term_str.starts_with('*') || term_str.starts_with('?'))
    {
        return Err(ParseException::with_message(
            "'*' or '?' not allowed as first character in WildcardQuery",
        )
        .error());
    }

    // `new Term(field, analyzeWildcard(field, termStr))`: the field may be
    // `null` (a parser without a default field).
    let t = Term::with_nullable_field(
        field.map(str::to_string),
        analyze_wildcard(this, field, term_str)?,
    );
    this.new_wildcard_query(t).map(Some)
}

/// Best effort to not pass the wildcard characters and escaped characters
/// through `normalize`: the matches of `WILDCARD_PATTERN`,
/// `(\\.)|([?*]+)`, are kept as they are, the text between them normalized.
///
/// Equivalent to the private `analyzeWildcard(String, String)`. The pattern
/// is matched by hand: `.` matches any character but the line terminators
/// `\n`, `\r`, U+0085, U+2028 and U+2029, as `java.util.regex` without
/// `DOTALL`.
fn analyze_wildcard<P: QueryParserBaseOps + ?Sized>(
    this: &mut P,
    field: Option<&str>,
    term_str: &str,
) -> Result<BytesRef> {
    let chars: Vec<char> = term_str.chars().collect();
    let mut sb = BytesRefBuilder::new();
    let mut last = 0usize;
    let mut i = 0usize;
    let is_line_terminator =
        |c: char| matches!(c, '\n' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}');
    while i < chars.len() {
        let (start, end) =
            if chars[i] == '\\' && i + 1 < chars.len() && !is_line_terminator(chars[i + 1]) {
                (i, i + 2)
            } else if chars[i] == '?' || chars[i] == '*' {
                let mut j = i + 1;
                while j < chars.len() && (chars[j] == '?' || chars[j] == '*') {
                    j += 1;
                }
                (i, j)
            } else {
                i += 1;
                continue;
            };
        if start > 0 {
            let chunk: String = chars[last..start].iter().collect();
            let normalized = normalize(this, field, &chunk)?;
            sb.append_ref(&normalized);
        }
        // append the matched group - without normalizing
        let group: String = chars[start..end].iter().collect();
        sb.append_ref(&BytesRef::from_char_sequence(&group));
        last = end;
        i = end;
    }
    if last < chars.len() {
        let chunk: String = chars[last..].iter().collect();
        let normalized = normalize(this, field, &chunk)?;
        sb.append_ref(&normalized);
    }
    Ok(sb.to_bytes_ref())
}

/// `QueryParserBase.getRegexpQuery`: the body of
/// [`QueryParserBaseOps::get_regexp_query`].
///
/// # Errors
///
/// As [`QueryParserBaseOps::get_regexp_query`].
pub fn get_regexp_query<P: QueryParserBaseOps + ?Sized>(
    this: &mut P,
    field: Option<&str>,
    term_str: &str,
) -> Result<Option<Arc<dyn Query>>> {
    // We need to pass the whole string to #normalize, which will not work
    // with custom attribute factories for the binary term impl, and may not
    // work with some analyzers
    let term = normalize(this, field, term_str)?;
    let t = Term::with_nullable_field(field.map(str::to_string), term);
    this.new_regexp_query(t).map(Some)
}

/// `QueryParserBase.getPrefixQuery`: the body of
/// [`QueryParserBaseOps::get_prefix_query`].
///
/// # Errors
///
/// As [`QueryParserBaseOps::get_prefix_query`].
pub fn get_prefix_query<P: QueryParserBaseOps + ?Sized>(
    this: &mut P,
    field: Option<&str>,
    term_str: &str,
) -> Result<Option<Arc<dyn Query>>> {
    if !this.query_parser_base().allow_leading_wildcard && term_str.starts_with('*') {
        return Err(ParseException::with_message(
            "'*' not allowed as first character in PrefixQuery",
        )
        .error());
    }
    let term = normalize(this, field, term_str)?;
    let t = Term::with_nullable_field(field.map(str::to_string), term);
    this.new_prefix_query(t).map(Some)
}

/// `QueryParserBase.getFuzzyQuery`: the body of
/// [`QueryParserBaseOps::get_fuzzy_query`].
///
/// # Errors
///
/// As [`QueryParserBaseOps::get_fuzzy_query`].
pub fn get_fuzzy_query<P: QueryParserBaseOps + ?Sized>(
    this: &mut P,
    field: Option<&str>,
    term_str: &str,
    min_similarity: f32,
) -> Result<Option<Arc<dyn Query>>> {
    let term = normalize(this, field, term_str)?;
    let t = Term::with_nullable_field(field.map(str::to_string), term);
    let prefix_length = this.query_parser_base().fuzzy_prefix_length;
    this.new_fuzzy_query(t, min_similarity, prefix_length)
        .map(Some)
}

/// The image of a token: `token.image`, the `NullPointerException` of a
/// `null` image when it is dereferenced.
fn image_of(token: &TokenRef) -> Result<String> {
    let image = token.lock().image.clone();
    crate::javacc::nn_owned(image)
}

/// `QueryParserBase.handleBareTokenQuery`: the body of
/// [`QueryParserBaseOps::handle_bare_token_query`].
///
/// # Errors
///
/// As [`QueryParserBaseOps::handle_bare_token_query`].
#[allow(clippy::too_many_arguments)]
pub fn handle_bare_token_query<P: QueryParserBaseOps + ?Sized>(
    this: &mut P,
    qfield: Option<&str>,
    term: &TokenRef,
    fuzzy_slop: Option<&TokenRef>,
    prefix: bool,
    wildcard: bool,
    fuzzy: bool,
    regexp: bool,
) -> Result<Option<Arc<dyn Query>>> {
    let image = image_of(term)?;
    let term_image = this.discard_escape_char(&image)?;
    let length = crate::javacc::length(&image);
    if wildcard {
        this.get_wildcard_query(qfield, &image)
    } else if prefix {
        let text = this.discard_escape_char(&crate::javacc::substring(&image, 0, length - 1)?)?;
        this.get_prefix_query(qfield, &text)
    } else if regexp {
        this.get_regexp_query(qfield, &crate::javacc::substring(&image, 1, length - 1)?)
    } else if fuzzy {
        this.handle_bare_fuzzy(qfield, fuzzy_slop, &term_image)
    } else {
        this.get_field_query(qfield, &term_image, false)
    }
}

/// `QueryParserBase.handleBareFuzzy`: the body of
/// [`QueryParserBaseOps::handle_bare_fuzzy`].
///
/// # Errors
///
/// As [`QueryParserBaseOps::handle_bare_fuzzy`].
pub fn handle_bare_fuzzy<P: QueryParserBaseOps + ?Sized>(
    this: &mut P,
    qfield: Option<&str>,
    fuzzy_slop: Option<&TokenRef>,
    term_image: &str,
) -> Result<Option<Arc<dyn Query>>> {
    let fms = this.get_fuzzy_distance(fuzzy_slop, term_image);
    if fms < 0.0 {
        return Err(ParseException::with_message(
            "Minimum similarity for a FuzzyQuery has to be between 0.0f and 1.0f !",
        )
        .error());
    } else if fms >= 1.0 && fms != (fms as i32) as f32 {
        return Err(
            ParseException::with_message("Fractional edit distances are not allowed!").error(),
        );
    }
    this.get_fuzzy_query(qfield, term_image, fms)
}

/// `QueryParserBase.handleQuotedTerm`: the body of
/// [`QueryParserBaseOps::handle_quoted_term`].
///
/// # Errors
///
/// As [`QueryParserBaseOps::handle_quoted_term`].
pub fn handle_quoted_term<P: QueryParserBaseOps + ?Sized>(
    this: &mut P,
    qfield: Option<&str>,
    term: &TokenRef,
    fuzzy_slop: Option<&TokenRef>,
) -> Result<Option<Arc<dyn Query>>> {
    let mut s = this.query_parser_base().phrase_slop; // default
    if let Some(fuzzy_slop) = fuzzy_slop {
        // `try { s = (int) Float.parseFloat(fuzzySlop.image.substring(1)); }
        // catch (Exception ignored) {}`
        let parsed = (|| -> Result<f32> {
            let image = image_of(fuzzy_slop)?;
            crate::javacc::parse_float(&crate::javacc::substring_from(&image, 1)?)
        })();
        match parsed {
            Ok(value) => s = value as i32,
            Err(e) if crate::javacc::is_exception(&e) => {}
            Err(e) => return Err(e),
        }
    }
    let image = image_of(term)?;
    let length = crate::javacc::length(&image);
    let text = this.discard_escape_char(&crate::javacc::substring(&image, 1, length - 1)?)?;
    this.get_field_query_with_slop(qfield, &text, s)
}

/// `QueryParserBase.handleBoost`: the body of
/// [`QueryParserBaseOps::handle_boost`].
///
/// # Errors
///
/// As [`QueryParserBaseOps::handle_boost`].
pub fn handle_boost<P: QueryParserBaseOps + ?Sized>(
    this: &mut P,
    q: Option<Arc<dyn Query>>,
    boost: Option<&TokenRef>,
) -> Result<Option<Arc<dyn Query>>> {
    let _ = this;
    let mut q = q;
    if let Some(boost) = boost {
        let mut f = 1.0f32;
        // `try { f = Float.parseFloat(boost.image); } catch (Exception
        // ignored) {}`: should this be handled somehow? (defaults to "no
        // boost", if boost number is invalid)
        let parsed = (|| -> Result<f32> {
            let image = image_of(boost)?;
            crate::javacc::parse_float(&image)
        })();
        match parsed {
            Ok(value) => f = value,
            Err(e) if crate::javacc::is_exception(&e) => {}
            Err(e) => return Err(e),
        }

        // avoid boosting null queries, such as those caused by stop words
        if let Some(query) = q {
            q = Some(Arc::new(BoostQuery::new(query, f)?));
        }
    }
    Ok(q)
}

/// `QueryParserBase.discardEscapeChar(String)`: the body of
/// [`QueryParserBaseOps::discard_escape_char`].
///
/// **Divergence.** Java's result is built from UTF-16 units and may hold an
/// unpaired surrogate (`\\uD800`); a Rust string cannot, and such a unit
/// becomes U+FFFD.
///
/// # Errors
///
/// As [`QueryParserBaseOps::discard_escape_char`].
pub fn discard_escape_char(input: &str) -> Result<String> {
    let input: Vec<u16> = input.encode_utf16().collect();
    // Create char array to hold unescaped char sequence
    let mut output: Vec<u16> = Vec::with_capacity(input.len());

    // We remember whether the last processed character was an escape
    // character
    let mut last_char_was_escape_char = false;

    // The multiplier the current unicode digit must be multiplied with.
    // E.g. the first digit must be multiplied with 16^3, the second with
    // 16^2...
    let mut code_point_multiplier: i32 = 0;

    // Used to calculate the codepoint of the escaped unicode character
    let mut code_point: i32 = 0;

    for &cur_char in &input {
        if code_point_multiplier > 0 {
            code_point =
                code_point.wrapping_add(hex_to_int(cur_char)?.wrapping_mul(code_point_multiplier));
            code_point_multiplier = ((code_point_multiplier as u32) >> 4) as i32;
            if code_point_multiplier == 0 {
                output.push(code_point as u16);
                code_point = 0;
            }
        } else if last_char_was_escape_char {
            if cur_char == u16::from(b'u') {
                // found an escaped unicode character
                code_point_multiplier = 16 * 16 * 16;
            } else {
                // this character was escaped
                output.push(cur_char);
            }
            last_char_was_escape_char = false;
        } else if cur_char == u16::from(b'\\') {
            last_char_was_escape_char = true;
        } else {
            output.push(cur_char);
        }
    }

    if code_point_multiplier > 0 {
        return Err(ParseException::with_message("Truncated Unicode escape sequence.").error());
    }

    if last_char_was_escape_char {
        return Err(
            ParseException::with_message("Term can not end with escape character.").error(),
        );
    }

    Ok(String::from_utf16_lossy(&output))
}

/// Returns the numeric value of the hexadecimal character.
///
/// Equivalent to the package-private `hexToInt(char)`.
///
/// # Errors
///
/// `ParseException("Non-hex character in Unicode escape sequence: " + c)`.
pub(crate) fn hex_to_int(c: u16) -> Result<i32> {
    let c32 = i32::from(c);
    if (i32::from(b'0')..=i32::from(b'9')).contains(&c32) {
        Ok(c32 - i32::from(b'0'))
    } else if (i32::from(b'a')..=i32::from(b'f')).contains(&c32) {
        Ok(c32 - i32::from(b'a') + 10)
    } else if (i32::from(b'A')..=i32::from(b'F')).contains(&c32) {
        Ok(c32 - i32::from(b'A') + 10)
    } else {
        Err(ParseException::with_message(format!(
            "Non-hex character in Unicode escape sequence: {}",
            String::from_utf16_lossy(&[c])
        ))
        .error())
    }
}

/// Returns a String where those characters that QueryParser expects to be
/// escaped are escaped by a preceding `\`.
///
/// Equivalent to `QueryParserBase.escape(String)`.
pub fn escape(s: &str) -> String {
    let mut sb = String::with_capacity(s.len());
    for c in s.chars() {
        // These characters are part of the query syntax and must be escaped
        if matches!(
            c,
            '\\' | '+'
                | '-'
                | '!'
                | '('
                | ')'
                | ':'
                | '^'
                | '['
                | ']'
                | '"'
                | '{'
                | '}'
                | '~'
                | '*'
                | '?'
                | '|'
                | '&'
                | '/'
        ) {
            sb.push('\\');
        }
        sb.push(c);
    }
    sb
}

/// The `CommonQueryParserConfiguration` methods of Java's
/// `QueryParserBase`, for every classic parser. None of them fails; the
/// locale, the time zone and the rewrite method are stored as given, `None`
/// for Java's `null` (`QueryParserBase.java:262-293`), and the parser throws
/// the `NullPointerException` where it uses them.
impl<T: QueryParserBaseOps + ?Sized> CommonQueryParserConfiguration for T {
    fn set_allow_leading_wildcard(&mut self, allow_leading_wildcard: bool) -> Result<()> {
        self.query_parser_base_mut().allow_leading_wildcard = allow_leading_wildcard;
        Ok(())
    }

    fn set_enable_position_increments(&mut self, enabled: bool) -> Result<()> {
        self.query_parser_base_mut()
            .builder
            .set_enable_position_increments(enabled);
        Ok(())
    }

    fn get_enable_position_increments(&self) -> Result<bool> {
        Ok(self.query_builder().get_enable_position_increments())
    }

    fn set_multi_term_rewrite_method(
        &mut self,
        method: Option<Arc<dyn RewriteMethod>>,
    ) -> Result<()> {
        self.query_parser_base_mut().multi_term_rewrite_method = method;
        Ok(())
    }

    fn get_multi_term_rewrite_method(&self) -> Result<Option<Arc<dyn RewriteMethod>>> {
        Ok(self.query_parser_base().multi_term_rewrite_method.clone())
    }

    fn set_fuzzy_prefix_length(&mut self, fuzzy_prefix_length: i32) -> Result<()> {
        self.query_parser_base_mut().fuzzy_prefix_length = fuzzy_prefix_length;
        Ok(())
    }

    fn set_locale(&mut self, locale: Option<Locale>) -> Result<()> {
        self.query_parser_base_mut().locale = locale;
        Ok(())
    }

    fn get_locale(&self) -> Result<Option<Locale>> {
        Ok(self.query_parser_base().locale.clone())
    }

    fn set_time_zone(&mut self, time_zone: Option<TimeZone>) -> Result<()> {
        self.query_parser_base_mut().time_zone = time_zone;
        Ok(())
    }

    fn get_time_zone(&self) -> Result<Option<TimeZone>> {
        Ok(self.query_parser_base().time_zone.clone())
    }

    fn set_phrase_slop(&mut self, phrase_slop: i32) -> Result<()> {
        self.query_parser_base_mut().phrase_slop = phrase_slop;
        Ok(())
    }

    fn get_analyzer(&self) -> Result<Option<Arc<dyn Analyzer>>> {
        Ok(self.query_builder().get_analyzer().cloned())
    }

    fn get_allow_leading_wildcard(&self) -> Result<bool> {
        Ok(self.query_parser_base().allow_leading_wildcard)
    }

    fn get_fuzzy_min_sim(&self) -> Result<f32> {
        Ok(self.query_parser_base().fuzzy_min_sim)
    }

    fn get_fuzzy_prefix_length(&self) -> Result<i32> {
        Ok(self.query_parser_base().fuzzy_prefix_length)
    }

    fn get_phrase_slop(&self) -> Result<i32> {
        Ok(self.query_parser_base().phrase_slop)
    }

    fn set_fuzzy_min_sim(&mut self, fuzzy_min_sim: f32) -> Result<()> {
        self.query_parser_base_mut().fuzzy_min_sim = fuzzy_min_sim;
        Ok(())
    }

    fn set_date_resolution(&mut self, date_resolution: Option<Resolution>) -> Result<()> {
        self.query_parser_base_mut().date_resolution = date_resolution;
        Ok(())
    }
}
