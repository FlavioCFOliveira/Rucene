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

//! The port of `SimpleQueryParser.java`, of the Java package
//! `org.apache.lucene.queryparser.simple`.

use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::Term;
use rucene::search::boolean_clause::BooleanClause;
use rucene::search::boolean_query::{self, BooleanQuery};
use rucene::search::boost_query::BoostQuery;
use rucene::search::fuzzy_query::FuzzyQuery;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::match_no_docs_query::MatchNoDocsQuery;
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::{Occur, Query};
use rucene::util::automaton::levenshtein_automata::MAXIMUM_SUPPORTED_DISTANCE;
use rucene::util::query_builder::{QueryBuilder, QueryBuilderOps};

/// Enables `AND` operator (+).
pub const AND_OPERATOR: i32 = 1 << 0;
/// Enables `NOT` operator (-).
pub const NOT_OPERATOR: i32 = 1 << 1;
/// Enables `OR` operator (|).
pub const OR_OPERATOR: i32 = 1 << 2;
/// Enables `PREFIX` operator (*).
pub const PREFIX_OPERATOR: i32 = 1 << 3;
/// Enables `PHRASE` operator (").
pub const PHRASE_OPERATOR: i32 = 1 << 4;
/// Enables `PRECEDENCE` operators: `(` and `)`.
pub const PRECEDENCE_OPERATORS: i32 = 1 << 5;
/// Enables `ESCAPE` operator (\).
pub const ESCAPE_OPERATOR: i32 = 1 << 6;
/// Enables `WHITESPACE` operators: ' ' '\n' '\r' '\t'.
pub const WHITESPACE_OPERATOR: i32 = 1 << 7;
/// Enables `FUZZY` operators: (~) on single terms.
pub const FUZZY_OPERATOR: i32 = 1 << 8;
/// Enables `NEAR` operators: (~) on phrases.
pub const NEAR_OPERATOR: i32 = 1 << 9;

/// The UTF-16 unit of an ASCII character.
const fn u(c: char) -> u16 {
    c as u16
}

/// SimpleQueryParser is used to parse human readable query syntax.
///
/// The main idea behind this parser is that a person should be able to type
/// whatever they want to represent a query, and this parser will do its best
/// to interpret what to search for no matter how poorly composed the request
/// may be. Tokens are considered to be any of a term, phrase, or subquery
/// for the operations described below. Whitespace including ' ' '\n' '\r'
/// and '\t' and certain operators may be used to delimit tokens ( ) + | " .
///
/// Any errors in query syntax will be ignored and the parser will attempt
/// to decipher what it can; however, this may mean odd or unexpected results.
///
/// The operators are: `+` (AND), `|` (OR), `-` (NOT), `"` (phrase), `*` at
/// the end of terms (prefix), `(` and `)` (precedence), `~N` at the end of
/// terms (fuzzy) and of phrases (near slop), and `\` (escape).
///
/// Equivalent to `org.apache.lucene.queryparser.simple.SimpleQueryParser`.
/// Java's `Map<String, Float> weights` is iterated in the map's own order;
/// here the caller gives the entries in that order.
#[derive(Debug)]
pub struct SimpleQueryParser {
    /// The `QueryBuilder` state Java's class extends.
    builder: QueryBuilder,
    /// Map of fields to query against with their weights.
    pub weights: Vec<(String, f32)>,
    /// Flags to the parser (to turn features on/off).
    pub flags: i32,
    default_operator: Occur,
}

impl SimpleQueryParser {
    /// Creates a new parser searching over a single field.
    ///
    /// Equivalent to `new SimpleQueryParser(Analyzer, String)`.
    pub fn new(analyzer: Option<Arc<dyn Analyzer>>, field: &str) -> Self {
        Self::with_weights(analyzer, vec![(field.to_string(), 1.0)])
    }

    /// Creates a new parser searching over multiple fields with different
    /// weights.
    ///
    /// Equivalent to `new SimpleQueryParser(Analyzer, Map<String, Float>)`.
    pub fn with_weights(analyzer: Option<Arc<dyn Analyzer>>, weights: Vec<(String, f32)>) -> Self {
        Self::with_flags(analyzer, weights, -1)
    }

    /// Creates a new parser with custom flags used to enable/disable certain
    /// features.
    ///
    /// Equivalent to `new SimpleQueryParser(Analyzer, Map<String, Float>,
    /// int)`.
    pub fn with_flags(
        analyzer: Option<Arc<dyn Analyzer>>,
        weights: Vec<(String, f32)>,
        flags: i32,
    ) -> Self {
        Self {
            builder: QueryBuilder::new(analyzer),
            weights,
            flags,
            default_operator: Occur::SHOULD,
        }
    }

    /// The `QueryBuilder` state, for its setters.
    pub fn query_builder_mut(&mut self) -> &mut QueryBuilder {
        &mut self.builder
    }
}

impl QueryBuilderOps for SimpleQueryParser {
    fn query_builder(&self) -> &QueryBuilder {
        &self.builder
    }
}

impl SimpleQueryParserOps for SimpleQueryParser {
    fn simple_query_parser(&self) -> &SimpleQueryParser {
        self
    }

    fn simple_query_parser_mut(&mut self) -> &mut SimpleQueryParser {
        self
    }
}

/// The parsing state of one (sub)query: the characters, the shared
/// buffer, the range being parsed, the pending operations and the tree
/// built so far.
///
/// Equivalent to the package-private
/// `org.apache.lucene.queryparser.simple.SimpleQueryParser.State`. Java's
/// nested states share the outer state's `data` and `buffer` arrays; here
/// they borrow them.
pub(crate) struct State<'a> {
    /// The characters in the query string, as UTF-16 units.
    data: &'a [u16],
    /// A temporary buffer used to reduce necessary allocations.
    buffer: &'a mut [u16],
    index: usize,
    length: usize,
    current_operation: Option<Occur>,
    previous_operation: Option<Occur>,
    not: i32,
    top: Option<Arc<dyn Query>>,
}

impl<'a> State<'a> {
    /// Equivalent to the constructor `State(char[], char[], int, int)`.
    fn new(data: &'a [u16], buffer: &'a mut [u16], index: usize, length: usize) -> Self {
        Self {
            data,
            buffer,
            index,
            length,
            current_operation: None,
            previous_operation: None,
            not: 0,
            top: None,
        }
    }
}

/// The behaviour of Java's `SimpleQueryParser`: `parse`, the protected
/// factory methods a subclass overrides, and the private parsing steps.
///
/// Equivalent to the methods of
/// `org.apache.lucene.queryparser.simple.SimpleQueryParser`.
pub trait SimpleQueryParserOps: QueryBuilderOps {
    /// The state of the parser.
    fn simple_query_parser(&self) -> &SimpleQueryParser;

    /// The state of the parser, for writing.
    fn simple_query_parser_mut(&mut self) -> &mut SimpleQueryParser;

    /// Parses the query text and returns parsed query.
    ///
    /// Equivalent to `parse(String)`.
    ///
    /// # Errors
    ///
    /// The errors of the analysis and of the query construction.
    fn parse(&mut self, query_text: &str) -> Result<Arc<dyn Query>> {
        // `queryText.trim()`: the characters up to U+0020.
        if query_text.trim_matches(|c: char| c <= '\u{20}') == "*" {
            return Ok(Arc::new(MatchAllDocsQuery::instance()));
        }

        let data: Vec<u16> = query_text.encode_utf16().collect();
        let mut buffer: Vec<u16> = vec![0; data.len()];

        let length = data.len();
        let mut state = State::new(&data, &mut buffer, 0, length);
        parse_sub_query(self, &mut state)?;
        match state.top {
            None => Ok(Arc::new(MatchNoDocsQuery::new(
                "empty string passed to query parser",
            ))),
            Some(top) => Ok(top),
        }
    }

    /// Factory method to generate a standard query (no phrase or prefix
    /// operators).
    ///
    /// Equivalent to the protected `newDefaultQuery(String)`.
    ///
    /// # Errors
    ///
    /// The errors of the analysis and of the query construction.
    fn new_default_query(&mut self, text: &str) -> Result<Option<Arc<dyn Query>>> {
        let mut bq = boolean_query::Builder::new();
        let weights = self.simple_query_parser().weights.clone();
        let default_operator = self.simple_query_parser().default_operator;
        for (key, boost) in &weights {
            let q = self.create_boolean_query_with_operator(Some(key), text, default_operator)?;
            if let Some(mut q) = q {
                if *boost != 1.0 {
                    q = Arc::new(BoostQuery::new(q, *boost)?);
                }
                bq.add(q, Occur::SHOULD)?;
            }
        }
        Ok(self.simplify(bq.build()))
    }

    /// Factory method to generate a fuzzy query.
    ///
    /// Equivalent to the protected `newFuzzyQuery(String, int)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalization and of the query construction.
    fn new_fuzzy_query(&mut self, text: &str, fuzziness: i32) -> Result<Option<Arc<dyn Query>>> {
        let mut bq = boolean_query::Builder::new();
        let weights = self.simple_query_parser().weights.clone();
        for (field_name, boost) in &weights {
            let analyzer = crate::javacc::nn(self.query_builder().get_analyzer())?;
            let term = analyzer.normalize(field_name, text)?;
            let mut q: Arc<dyn Query> = Arc::new(FuzzyQuery::with_max_edits(
                Term::new(field_name.clone(), term),
                fuzziness,
            )?);
            if *boost != 1.0 {
                q = Arc::new(BoostQuery::new(q, *boost)?);
            }
            bq.add(q, Occur::SHOULD)?;
        }
        Ok(self.simplify(bq.build()))
    }

    /// Factory method to generate a phrase query with slop.
    ///
    /// Equivalent to the protected `newPhraseQuery(String, int)`.
    ///
    /// # Errors
    ///
    /// The errors of the analysis and of the query construction.
    fn new_phrase_query(&mut self, text: &str, slop: i32) -> Result<Option<Arc<dyn Query>>> {
        let mut bq = boolean_query::Builder::new();
        let weights = self.simple_query_parser().weights.clone();
        for (key, boost) in &weights {
            let q = self.create_phrase_query_with_slop(Some(key), text, slop)?;
            if let Some(mut q) = q {
                if *boost != 1.0 {
                    q = Arc::new(BoostQuery::new(q, *boost)?);
                }
                bq.add(q, Occur::SHOULD)?;
            }
        }
        Ok(self.simplify(bq.build()))
    }

    /// Factory method to generate a prefix query.
    ///
    /// Equivalent to the protected `newPrefixQuery(String)`.
    ///
    /// # Errors
    ///
    /// The errors of the normalization and of the query construction.
    fn new_prefix_query(&mut self, text: &str) -> Result<Option<Arc<dyn Query>>> {
        let mut bq = boolean_query::Builder::new();
        let weights = self.simple_query_parser().weights.clone();
        for (field_name, boost) in &weights {
            let analyzer = crate::javacc::nn(self.query_builder().get_analyzer())?;
            let term = analyzer.normalize(field_name, text)?;
            let mut q: Arc<dyn Query> =
                Arc::new(PrefixQuery::new(Term::new(field_name.clone(), term))?);
            if *boost != 1.0 {
                q = Arc::new(BoostQuery::new(q, *boost)?);
            }
            bq.add(q, Occur::SHOULD)?;
        }
        Ok(self.simplify(bq.build()))
    }

    /// Helper to simplify boolean queries with 0 or 1 clause.
    ///
    /// Equivalent to the protected `simplify(BooleanQuery)`.
    fn simplify(&mut self, bq: BooleanQuery) -> Option<Arc<dyn Query>> {
        match bq.clauses().len() {
            0 => None,
            1 => Some(Arc::clone(bq.clauses()[0].query())),
            _ => Some(Arc::new(bq)),
        }
    }

    /// Returns the implicit operator setting, which will be either `SHOULD`
    /// or `MUST`.
    ///
    /// Equivalent to `getDefaultOperator()`.
    fn get_default_operator(&self) -> Occur {
        self.simple_query_parser().default_operator
    }

    /// Sets the implicit operator setting, which must be either `SHOULD` or
    /// `MUST`.
    ///
    /// Equivalent to `setDefaultOperator(BooleanClause.Occur)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for any other operator.
    fn set_default_operator(&mut self, operator: Occur) -> Result<()> {
        if operator != Occur::SHOULD && operator != Occur::MUST {
            return Err(LuceneError::illegal_argument(
                "invalid operator: only SHOULD or MUST are allowed",
            ));
        }
        self.simple_query_parser_mut().default_operator = operator;
        Ok(())
    }
}

/// `flags & flag != 0`.
fn has<P: SimpleQueryParserOps + ?Sized>(this: &P, flag: i32) -> bool {
    (this.simple_query_parser().flags & flag) != 0
}

/// Equivalent to the private `parseSubQuery(State)`.
fn parse_sub_query<P: SimpleQueryParserOps + ?Sized>(
    this: &mut P,
    state: &mut State<'_>,
) -> Result<()> {
    while state.index < state.length {
        let c = state.data[state.index];
        if c == u('(') && has(this, PRECEDENCE_OPERATORS) {
            // the beginning of a subquery has been found
            consume_sub_query(this, state)?;
        } else if c == u(')') && has(this, PRECEDENCE_OPERATORS) {
            // this is an extraneous character so it is ignored
            state.index += 1;
        } else if c == u('"') && has(this, PHRASE_OPERATOR) {
            // the beginning of a phrase has been found
            consume_phrase(this, state)?;
        } else if c == u('+') && has(this, AND_OPERATOR) {
            // an and operation has been explicitly set
            // if an operation has already been set this one is ignored
            // if a term (or phrase or subquery) has not been found yet the
            // operation is also ignored since there is no previous
            // term (or phrase or subquery) to and with
            if state.current_operation.is_none() && state.top.is_some() {
                state.current_operation = Some(Occur::MUST);
            }

            state.index += 1;
        } else if c == u('|') && has(this, OR_OPERATOR) {
            // an or operation has been explicitly set
            // if an operation has already been set this one is ignored
            // if a term (or phrase or subquery) has not been found yet the
            // operation is also ignored since there is no previous
            // term (or phrase or subquery) to or with
            if state.current_operation.is_none() && state.top.is_some() {
                state.current_operation = Some(Occur::SHOULD);
            }

            state.index += 1;
        } else if c == u('-') && has(this, NOT_OPERATOR) {
            // a not operator has been found, so increase the not count
            // two not operators in a row negate each other
            state.not += 1;
            state.index += 1;

            // continue so the not operator is not reset
            // before the next character is determined
            continue;
        } else if (c == u(' ') || c == u('\t') || c == u('\n') || c == u('\r'))
            && has(this, WHITESPACE_OPERATOR)
        {
            // ignore any whitespace found as it may have already been
            // used a delimiter across a term (or phrase or subquery)
            // or is simply extraneous
            state.index += 1;
        } else {
            // the beginning of a token has been found
            consume_token(this, state)?;
        }

        // reset the not operator as even whitespace is not allowed when
        // specifying the not operation for a term (or phrase or subquery)
        state.not = 0;
    }
    Ok(())
}

/// Equivalent to the private `consumeSubQuery(State)`.
fn consume_sub_query<P: SimpleQueryParserOps + ?Sized>(
    this: &mut P,
    state: &mut State<'_>,
) -> Result<()> {
    debug_assert!(has(this, PRECEDENCE_OPERATORS));
    state.index += 1;
    let start = state.index;
    let mut precedence = 1;
    let mut escaped = false;

    while state.index < state.length {
        if !escaped {
            let c = state.data[state.index];
            if c == u('\\') && has(this, ESCAPE_OPERATOR) {
                // an escape character has been found so
                // whatever character is next will become
                // part of the subquery unless the escape
                // character is the last one in the data
                escaped = true;
                state.index += 1;

                continue;
            } else if c == u('(') {
                // increase the precedence as there is a
                // subquery in the current subquery
                precedence += 1;
            } else if c == u(')') {
                precedence -= 1;

                if precedence == 0 {
                    // this should be the end of the subquery
                    // all characters found will used for
                    // creating the subquery
                    break;
                }
            }
        }

        escaped = false;
        state.index += 1;
    }

    if state.index == state.length {
        // a closing parenthesis was never found so the opening
        // parenthesis is considered extraneous and will be ignored
        state.index = start;
    } else if state.index == start {
        // a closing parenthesis was found immediately after the opening
        // parenthesis so the current operation is reset since it would
        // have been applied to this subquery
        state.current_operation = None;

        state.index += 1;
    } else {
        // a complete subquery has been found and is recursively parsed by
        // starting over with a new state object
        let top = {
            let mut sub_state = State::new(state.data, &mut *state.buffer, start, state.index);
            parse_sub_query(this, &mut sub_state)?;
            sub_state.top
        };
        build_query_tree(this, state, top)?;

        state.index += 1;
    }
    Ok(())
}

/// Equivalent to the private `consumePhrase(State)`.
fn consume_phrase<P: SimpleQueryParserOps + ?Sized>(
    this: &mut P,
    state: &mut State<'_>,
) -> Result<()> {
    debug_assert!(has(this, PHRASE_OPERATOR));
    state.index += 1;
    let start = state.index;
    let mut copied = 0usize;
    let mut escaped = false;
    let mut has_slop = false;

    while state.index < state.length {
        if !escaped {
            let c = state.data[state.index];
            if c == u('\\') && has(this, ESCAPE_OPERATOR) {
                // an escape character has been found so
                // whatever character is next will become
                // part of the phrase unless the escape
                // character is the last one in the data
                escaped = true;
                state.index += 1;

                continue;
            } else if c == u('"') {
                // if there are still characters after the closing ", check
                // for a tilde
                if state.length > (state.index + 1)
                    && state.data[state.index + 1] == u('~')
                    && has(this, NEAR_OPERATOR)
                {
                    state.index += 1;
                    // check for characters after the tilde
                    if state.length > (state.index + 1) {
                        has_slop = true;
                    }
                    break;
                } else {
                    // this should be the end of the phrase
                    // all characters found will used for
                    // creating the phrase query
                    break;
                }
            }
        }

        escaped = false;
        state.buffer[copied] = state.data[state.index];
        copied += 1;
        state.index += 1;
    }

    if state.index == state.length {
        // a closing double quote was never found so the opening
        // double quote is considered extraneous and will be ignored
        state.index = start;
    } else if state.index == start {
        // a closing double quote was found immediately after the opening
        // double quote so the current operation is reset since it would
        // have been applied to this phrase
        state.current_operation = None;

        state.index += 1;
    } else {
        // a complete phrase has been found and is parsed through
        // through the analyzer from the given field
        let phrase = String::from_utf16_lossy(&state.buffer[..copied]);
        let branch = if has_slop {
            let slop = parse_fuzziness(this, state);
            this.new_phrase_query(&phrase, slop)?
        } else {
            this.new_phrase_query(&phrase, 0)?
        };
        build_query_tree(this, state, branch)?;

        state.index += 1;
    }
    Ok(())
}

/// Equivalent to the private `consumeToken(State)`.
fn consume_token<P: SimpleQueryParserOps + ?Sized>(
    this: &mut P,
    state: &mut State<'_>,
) -> Result<()> {
    let mut copied = 0usize;
    let mut escaped = false;
    let mut prefix = false;
    let mut fuzzy = false;

    while state.index < state.length {
        if !escaped {
            let c = state.data[state.index];
            if c == u('\\') && has(this, ESCAPE_OPERATOR) {
                // an escape character has been found so
                // whatever character is next will become
                // part of the term unless the escape
                // character is the last one in the data
                escaped = true;
                prefix = false;
                state.index += 1;

                continue;
            } else if token_finished(this, state) {
                // this should be the end of the term
                // all characters found will used for
                // creating the term query
                break;
            } else if copied > 0 && c == u('~') && has(this, FUZZY_OPERATOR) {
                fuzzy = true;
                break;
            }

            // wildcard tracks whether or not the last character
            // was a '*' operator that hasn't been escaped
            // there must be at least one valid character before
            // searching for a prefixed set of terms
            prefix = copied > 0 && c == u('*') && has(this, PREFIX_OPERATOR);
        }

        escaped = false;
        state.buffer[copied] = state.data[state.index];
        copied += 1;
        state.index += 1;
    }

    if copied > 0 {
        let branch = if fuzzy && has(this, FUZZY_OPERATOR) {
            let token = String::from_utf16_lossy(&state.buffer[..copied]);
            let mut fuzziness = parse_fuzziness(this, state);
            // edit distance has a maximum, limit to the maximum supported
            fuzziness = fuzziness.min(MAXIMUM_SUPPORTED_DISTANCE);
            if fuzziness == 0 {
                this.new_default_query(&token)?
            } else {
                this.new_fuzzy_query(&token, fuzziness)?
            }
        } else if prefix {
            // if a term is found with a closing '*' it is considered to be a
            // prefix query and will have prefix added as an option
            let token = String::from_utf16_lossy(&state.buffer[..copied - 1]);
            this.new_prefix_query(&token)?
        } else {
            // a standard term has been found so it will be run through
            // the entire analysis chain from the specified schema field
            let token = String::from_utf16_lossy(&state.buffer[..copied]);
            this.new_default_query(&token)?
        };

        build_query_tree(this, state, branch)?;
    }
    Ok(())
}

/// Adds a clause to the end of a copy of `bq`.
///
/// Equivalent to the private static `addClause(BooleanQuery, Query,
/// BooleanClause.Occur)`.
fn add_clause(bq: &BooleanQuery, query: Arc<dyn Query>, occur: Occur) -> Result<BooleanQuery> {
    let mut new_bq = boolean_query::Builder::new();
    new_bq.set_minimum_number_should_match(bq.get_minimum_number_should_match());
    for clause in bq.clauses() {
        new_bq.add_clause(clause.clone())?;
    }
    new_bq.add_clause(BooleanClause::new(query, occur))?;
    Ok(new_bq.build())
}

/// buildQueryTree should be called after a term, phrase, or subquery is
/// consumed to be added to our existing query tree; this method will only
/// add to the existing tree if the branch contained in state is not null.
///
/// Equivalent to the private `buildQueryTree(State, Query)`.
fn build_query_tree<P: SimpleQueryParserOps + ?Sized>(
    this: &mut P,
    state: &mut State<'_>,
    branch: Option<Arc<dyn Query>>,
) -> Result<()> {
    let Some(mut branch) = branch else {
        return Ok(());
    };
    // modify our branch to a BooleanQuery wrapper for not
    // this is necessary any time a term, phrase, or subquery is negated
    if state.not % 2 == 1 {
        let mut nq = boolean_query::Builder::new();
        nq.add(branch, Occur::MUST_NOT)?;
        nq.add(Arc::new(MatchAllDocsQuery::instance()), Occur::SHOULD)?;
        branch = Arc::new(nq.build());
    }

    // first term (or phrase or subquery) found and will begin our query tree
    match state.top.clone() {
        None => state.top = Some(branch),
        Some(top) => {
            // more than one term (or phrase or subquery) found
            // set currentOperation to the default if no other operation is
            // explicitly set
            if state.current_operation.is_none() {
                state.current_operation = Some(this.simple_query_parser().default_operator);
            }
            let current_operation = state.current_operation.expect("INVARIANT: set above");

            // operational change requiring a new parent node
            // this occurs if the previous operation is not the same as
            // current operation because the previous operation must be
            // evaluated separately to preserve the proper precedence and the
            // current operation will take over as the top of the tree
            let mut top = top;
            if state.previous_operation != state.current_operation {
                let mut bq = boolean_query::Builder::new();
                bq.add(top, current_operation)?;
                top = Arc::new(bq.build());
            }

            // reset all of the state for reuse
            // `(BooleanQuery) state.top`
            let Some(bq) = top.as_any().downcast_ref::<BooleanQuery>() else {
                return Err(LuceneError::from(LuceneErrorKind::Unchecked {
                    class: "java.lang.ClassCastException",
                    message: Some(format!(
                        "class {} cannot be cast to class org.apache.lucene.search.BooleanQuery",
                        top.java_class_name()
                    )),
                    cause: None,
                }));
            };
            state.top = Some(Arc::new(add_clause(bq, branch, current_operation)?));
            state.previous_operation = state.current_operation;
        }
    }

    // reset the current operation as it was intended to be applied to
    // the incoming term (or phrase or subquery) even if branch was null
    // due to other possible errors
    state.current_operation = None;
    Ok(())
}

/// Helper parsing fuzziness from parsing state.
///
/// Equivalent to the private `parseFuzziness(State)`: the slop that
/// follows `~`, 2 for none, 0 for a number that does not parse or is
/// negative.
fn parse_fuzziness<P: SimpleQueryParserOps + ?Sized>(this: &P, state: &mut State<'_>) -> i32 {
    let mut slop_text: Vec<u16> = Vec::with_capacity(state.length);

    if state.data[state.index] == u('~') {
        while state.index < state.length {
            state.index += 1;
            // it's possible that the ~ was at the end, so check after
            // incrementing to make sure we don't go out of bounds
            if state.index < state.length {
                if token_finished(this, state) {
                    break;
                }
                slop_text.push(state.data[state.index]);
            }
        }
        let mut fuzziness = 0;
        let fuzzy_string = String::from_utf16_lossy(&slop_text);
        if fuzzy_string.is_empty() {
            // Use automatic fuzziness, ~2
            fuzziness = 2;
        } else if let Ok(value) = crate::javacc::parse_int(&fuzzy_string) {
            fuzziness = value;
        }
        // swallow number format exceptions parsing fuzziness

        // negative -> 0
        if fuzziness < 0 {
            fuzziness = 0;
        }
        return fuzziness;
    }
    0
}

/// Helper returning true if the state has reached the end of token.
///
/// Equivalent to the private `tokenFinished(State)`.
fn token_finished<P: SimpleQueryParserOps + ?Sized>(this: &P, state: &State<'_>) -> bool {
    let c = state.data[state.index];
    (c == u('"') && has(this, PHRASE_OPERATOR))
        || (c == u('|') && has(this, OR_OPERATOR))
        || (c == u('+') && has(this, AND_OPERATOR))
        || (c == u('(') && has(this, PRECEDENCE_OPERATORS))
        || (c == u(')') && has(this, PRECEDENCE_OPERATORS))
        || ((c == u(' ') || c == u('\t') || c == u('\n') || c == u('\r'))
            && has(this, WHITESPACE_OPERATOR))
}
