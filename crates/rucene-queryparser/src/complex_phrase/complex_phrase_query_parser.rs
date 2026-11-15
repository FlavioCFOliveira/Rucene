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

//! The port of `ComplexPhraseQueryParser.java`, of the Java package
//! `org.apache.lucene.queryparser.complexPhrase`.

use std::any::Any;
use std::sync::{Arc, Mutex, PoisonError};

use rucene::analysis::Analyzer;
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::Term;
use rucene::search::boolean_clause::BooleanClause;
use rucene::search::boolean_query::{self, BooleanQuery};
use rucene::search::boost_query::BoostQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_no_docs_query::MatchNoDocsQuery;
use rucene::search::multi_term_query::scoring_boolean_rewrite;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::synonym_query::SynonymQuery;
use rucene::search::{Occur, Query, TermQuery};
use rucene::util::java_collections::java_string_hash_code;
use rucene::util::query_builder::{QueryBuilder, QueryBuilderOps};
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_not_query::SpanNotQuery;
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;

use crate::charstream::char_stream::CharStream;
use crate::classic::parse_exception::ParseException;
use crate::classic::query_parser::{self, QueryParser, QueryParserOps};
use crate::classic::query_parser_base::{self as base, QueryParserBase, QueryParserBaseOps};
use crate::classic::query_parser_constants::QueryParserConstants;
use crate::flexible::standard::common_query_parser_configuration::CommonQueryParserConfiguration;

/// QueryParser which permits complex phrase query syntax eg "(john
/// jon jonathan~) peters*".
///
/// Performs potentially multiple passes over Query text to parse any nested
/// logic in PhraseQueries. - First pass takes any PhraseQuery content between
/// quotes and stores for subsequent pass. All other query content is parsed
/// as normal - Second pass parses any stored PhraseQuery content, checking
/// all embedded clauses are referring to the same field and therefore can be
/// rewritten as Span queries. All PhraseQuery clauses are expressed as
/// ComplexPhraseQuery objects.
///
/// This could arguably be done in one pass using a new QueryParser but here
/// I am working within the constraints of the existing parser as a base
/// class. This currently simply feeds all phrase content through an analyzer
/// to select phrase terms - any "special" syntax such as * ~ * etc are not
/// given special status.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.complexPhrase.ComplexPhraseQueryParser`.
#[derive(Debug)]
pub struct ComplexPhraseQueryParser {
    /// The `QueryParser` state Java's class extends.
    parser: QueryParser,
    complex_phrases: Option<Vec<Arc<ComplexPhraseQuery>>>,
    is_pass2_resolving_phrases: bool,
    in_order: bool,
    current_phrase_query: Option<Arc<ComplexPhraseQuery>>,
}

impl ComplexPhraseQueryParser {
    /// Equivalent to `new ComplexPhraseQueryParser(String, Analyzer)`.
    ///
    /// # Errors
    ///
    /// As [`QueryParser::new`].
    pub fn new(f: Option<&str>, a: Option<Arc<dyn Analyzer>>) -> Result<Self> {
        Ok(Self {
            parser: QueryParser::new(f, a)?,
            complex_phrases: None,
            is_pass2_resolving_phrases: false,
            in_order: true,
            current_phrase_query: None,
        })
    }

    /// When `in_order` is true, the search terms must exists in the
    /// documents as the same order as in query.
    ///
    /// Equivalent to `setInOrder(boolean)`.
    pub fn set_in_order(&mut self, in_order: bool) {
        self.in_order = in_order;
    }

    /// Helper method used to report on any clauses that appear in query
    /// syntax.
    ///
    /// Equivalent to the private `checkPhraseClauseIsForSameField(String)`.
    fn check_phrase_clause_is_for_same_field(&self, field: Option<&str>) -> Result<()> {
        let field = crate::javacc::nn_str(field)?;
        let current = crate::javacc::nn(self.current_phrase_query.as_ref())?;
        if field != current.field {
            return Err(ParseException::with_message(format!(
                "Cannot have clause for field \"{field}\" nested in phrase  for field \"{}\"",
                current.field
            ))
            .error());
        }
        Ok(())
    }
}

impl QueryParserConstants for ComplexPhraseQueryParser {}

impl QueryBuilderOps for ComplexPhraseQueryParser {
    fn query_builder(&self) -> &QueryBuilder {
        &self.parser.base.builder
    }

    /// There is No "getTermQuery throws ParseException" method to override
    /// so unfortunately need to throw a runtime exception here if a term for
    /// another field is embedded in phrase query.
    fn new_term_query(&self, term: &Term, boost: f32) -> Result<Arc<dyn Query>> {
        if self.is_pass2_resolving_phrases {
            if let Err(pe) = self.check_phrase_clause_is_for_same_field(Some(term.field())) {
                if ParseException::of(&pe).is_some() {
                    return Err(LuceneError::from(LuceneErrorKind::Unchecked {
                        class: "java.lang.RuntimeException",
                        message: Some("Error parsing complex phrase".to_string()),
                        cause: Some(Arc::new(pe)),
                    }));
                }
                return Err(pe);
            }
        }
        // `super.newTermQuery(term, boost)`: `QueryBuilder`'s own body.
        self.query_builder().new_term_query(term, boost)
    }
}

impl QueryParserOps for ComplexPhraseQueryParser {
    fn query_parser(&self) -> &QueryParser {
        &self.parser
    }

    fn query_parser_mut(&mut self) -> &mut QueryParser {
        &mut self.parser
    }
}

impl QueryParserBaseOps for ComplexPhraseQueryParser {
    fn query_parser_base(&self) -> &QueryParserBase {
        &self.parser.base
    }

    fn query_parser_base_mut(&mut self) -> &mut QueryParserBase {
        &mut self.parser.base
    }

    fn re_init(&mut self, stream: Box<dyn CharStream>) {
        query_parser::re_init(self, stream);
    }

    fn top_level_query(&mut self, field: Option<&str>) -> Result<Option<Arc<dyn Query>>> {
        query_parser::top_level_query(self, field.map(str::to_string))
    }

    fn set_auto_generate_phrase_queries(&mut self, value: bool) -> Result<()> {
        query_parser::set_auto_generate_phrase_queries(self, value)
    }

    fn get_field_query_with_slop(
        &mut self,
        field: Option<&str>,
        query_text: &str,
        slop: i32,
    ) -> Result<Option<Arc<dyn Query>>> {
        let cpq = Arc::new(ComplexPhraseQuery::new(
            field,
            Some(query_text),
            slop,
            self.in_order,
        )?);
        // add to list of phrases to be parsed once we are through with this
        // pass
        crate::javacc::nn_mut(self.complex_phrases.as_mut())?.push(Arc::clone(&cpq));
        Ok(Some(cpq))
    }

    fn parse(&mut self, query: &str) -> Result<Arc<dyn Query>> {
        if self.is_pass2_resolving_phrases {
            let old_method = self.get_multi_term_rewrite_method()?;
            // Temporarily force BooleanQuery rewrite so that Parser will
            // generate visible collection of terms which we can convert into
            // SpanQueries. ConstantScoreRewrite mode produces an opaque
            // ConstantScoreQuery object which cannot be interrogated for terms
            // in the same way a BooleanQuery can. QueryParser is not
            // guaranteed threadsafe anyway so this temporary state change
            // should not present an issue
            self.set_multi_term_rewrite_method(Some(scoring_boolean_rewrite()))?;
            let result = base::parse(self, query);
            self.set_multi_term_rewrite_method(old_method)?;
            return result;
        }

        // First pass - parse the top-level query recording any PhraseQuerys
        // which will need to be resolved
        self.complex_phrases = Some(Vec::new());
        let q = base::parse(self, query)?;

        // Perform second pass, using this QueryParser to parse any nested
        // PhraseQueries with different set of syntax restrictions (i.e. all
        // fields must be same)
        self.is_pass2_resolving_phrases = true;
        let result = (|| -> Result<()> {
            // `for (Iterator<ComplexPhraseQuery> iterator =
            // complexPhrases.iterator(); iterator.hasNext(); )`: the
            // `ArrayList` iterator fails fast with a
            // `ConcurrentModificationException` once the list grew during
            // the iteration.
            let expected = crate::javacc::nn(self.complex_phrases.as_ref())?.len();
            let mut cursor = 0usize;
            while cursor != crate::javacc::nn(self.complex_phrases.as_ref())?.len() {
                let phrases = crate::javacc::nn(self.complex_phrases.as_ref())?;
                if phrases.len() != expected {
                    return Err(LuceneError::from(LuceneErrorKind::Unchecked {
                        class: "java.util.ConcurrentModificationException",
                        message: None,
                        cause: None,
                    }));
                }
                let current = Arc::clone(&phrases[cursor]);
                cursor += 1;
                self.current_phrase_query = Some(Arc::clone(&current));
                // in each phrase, now parse the contents between quotes as a
                // separate parse operation
                current.parse_phrase_elements(self)?;
            }
            Ok(())
        })();
        self.is_pass2_resolving_phrases = false;
        result?;
        Ok(q)
    }

    fn get_wildcard_query(
        &mut self,
        field: Option<&str>,
        term_str: &str,
    ) -> Result<Option<Arc<dyn Query>>> {
        if self.is_pass2_resolving_phrases {
            self.check_phrase_clause_is_for_same_field(field)?;
        }
        base::get_wildcard_query(self, field, term_str)
    }

    fn get_range_query(
        &mut self,
        field: Option<&str>,
        part1: Option<&str>,
        part2: Option<&str>,
        start_inclusive: bool,
        end_inclusive: bool,
    ) -> Result<Option<Arc<dyn Query>>> {
        if self.is_pass2_resolving_phrases {
            self.check_phrase_clause_is_for_same_field(field)?;
        }
        base::get_range_query(self, field, part1, part2, start_inclusive, end_inclusive)
    }

    fn new_range_query(
        &mut self,
        field: Option<&str>,
        part1: Option<&str>,
        part2: Option<&str>,
        start_inclusive: bool,
        end_inclusive: bool,
    ) -> Result<Arc<dyn Query>> {
        let original_rewrite_method = self.get_multi_term_rewrite_method()?;
        if self.is_pass2_resolving_phrases {
            self.set_multi_term_rewrite_method(Some(scoring_boolean_rewrite()))?;
        }
        let result =
            base::new_range_query(self, field, part1, part2, start_inclusive, end_inclusive);
        self.set_multi_term_rewrite_method(original_rewrite_method)?;
        result
    }

    fn get_fuzzy_query(
        &mut self,
        field: Option<&str>,
        term_str: &str,
        min_similarity: f32,
    ) -> Result<Option<Arc<dyn Query>>> {
        if self.is_pass2_resolving_phrases {
            self.check_phrase_clause_is_for_same_field(field)?;
        }
        base::get_fuzzy_query(self, field, term_str, min_similarity)
    }
}

/// Used to handle the query content in between quotes and produced Span-based
/// interpretations of the clauses.
///
/// Equivalent to the package-private
/// `org.apache.lucene.queryparser.complexPhrase.ComplexPhraseQueryParser.ComplexPhraseQuery`.
/// Its one-element `contents` array, which the parser fills after the query
/// is built, is a [`Mutex`].
#[derive(Debug)]
pub(crate) struct ComplexPhraseQuery {
    pub(crate) field: String,
    pub(crate) phrased_query_string_contents: String,
    pub(crate) slop_factor: i32,
    in_order: bool,
    contents: Mutex<Option<Arc<dyn Query>>>,
}

/// The text of the dummy span term that matches nothing.
const DUMMY_CLAUSE: &str = "Dummy clause because no terms found - must match nothing";

impl ComplexPhraseQuery {
    /// Equivalent to `new ComplexPhraseQuery(String, String, int,
    /// boolean)`.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of `Objects.requireNonNull` for a `null`
    /// field or contents.
    pub fn new(
        field: Option<&str>,
        phrased_query_string_contents: Option<&str>,
        slop_factor: i32,
        in_order: bool,
    ) -> Result<Self> {
        Ok(Self {
            field: crate::javacc::nn_str(field)?.to_string(),
            phrased_query_string_contents: crate::javacc::nn_str(phrased_query_string_contents)?
                .to_string(),
            slop_factor,
            in_order,
            contents: Mutex::new(None),
        })
    }

    /// `contents[0]`.
    fn contents(&self) -> Option<Arc<dyn Query>> {
        self.contents
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Called by ComplexPhraseQueryParser for each phrase after the main
    /// parse thread is through.
    ///
    /// Equivalent to the protected `parsePhraseElements(ComplexPhraseQueryParser)`.
    ///
    /// # Errors
    ///
    /// The `ParseException` of the phrase contents.
    pub(crate) fn parse_phrase_elements(&self, qp: &mut ComplexPhraseQueryParser) -> Result<()> {
        // TODO ensure that field-sensitivity is preserved ie the query
        // string below is parsed as
        // field+":("+phrasedQueryStringContents+")"
        // but this will need code in rewrite to unwrap the first layer of
        // boolean query

        let old_default_parser_field = qp.query_parser_base().field.clone();
        // temporarily set the QueryParser to be parsing the default field for
        // this phrase e.g author:"fred* smith"
        qp.query_parser_base_mut().field = Some(self.field.clone());
        let parsed = qp.parse(&self.phrased_query_string_contents);
        qp.query_parser_base_mut().field = old_default_parser_field;
        let parsed = parsed?;
        *self.contents.lock().unwrap_or_else(PoisonError::into_inner) = Some(parsed);
        Ok(())
    }

    /// A span term on the phrase's field that matches nothing.
    fn dummy_clause(&self) -> Arc<dyn SpanQuery> {
        Arc::new(SpanTermQuery::new(Term::new(
            self.field.clone(),
            rucene::util::BytesRef::from_char_sequence(DUMMY_CLAUSE),
        )))
    }

    /// `"Unknown query type \"" + q.getClass().getName() + "\" found in
    /// phrase query string \"" + contents + "\""`.
    fn unknown_query_type(&self, q: &dyn Query) -> LuceneError {
        LuceneError::illegal_argument(format!(
            "Unknown query type \"{}\" found in phrase query string \"{}\"",
            q.java_class_name(),
            self.phrased_query_string_contents
        ))
    }

    /// Equivalent to the private `convert(SynonymQuery)`.
    fn convert(qc: &SynonymQuery) -> Result<BooleanQuery> {
        let mut bqb = boolean_query::Builder::new();
        for t in qc.get_terms() {
            bqb.add_clause(BooleanClause::new(
                Arc::new(TermQuery::new(t)),
                Occur::SHOULD,
            ))?;
        }
        Ok(bqb.build())
    }

    /// Equivalent to the private `addComplexPhraseClause(List<SpanQuery>,
    /// BooleanQuery)`.
    fn add_complex_phrase_clause(
        &self,
        span_clauses: &mut Vec<Arc<dyn SpanQuery>>,
        qc: &BooleanQuery,
    ) -> Result<()> {
        let mut ors: Vec<Arc<dyn SpanQuery>> = Vec::new();
        let mut nots: Vec<Arc<dyn SpanQuery>> = Vec::new();

        // For all clauses e.g. one* two~
        for clause in qc.clauses() {
            let mut child_query = Arc::clone(clause.query());
            while let Some(bq) = child_query.as_any().downcast_ref::<BoostQuery>() {
                child_query = bq.get_query();
            }

            // select the list to which we will add these options
            let chosen_list = if clause.occur() == Occur::MUST_NOT {
                &mut nots
            } else {
                &mut ors
            };

            if let Some(tq) = child_query.as_any().downcast_ref::<TermQuery>() {
                let stq: Arc<dyn SpanQuery> = Arc::new(SpanTermQuery::new(tq.get_term().clone()));
                chosen_list.push(stq);
            } else if let Some(cbq) = child_query.as_any().downcast_ref::<BooleanQuery>() {
                self.add_complex_phrase_clause(chosen_list, cbq)?;
            } else if child_query.as_any().is::<MatchNoDocsQuery>() {
                // Insert fake term e.g. phrase query was for "Fred Smithe*"
                // and there were no "Smithe*" terms - need to prevent match
                // on just "Fred".
                chosen_list.push(self.dummy_clause());
            } else {
                // TODO alternatively could call extract terms here?
                return Err(LuceneError::illegal_argument(format!(
                    "Unknown query type:{}",
                    child_query.java_class_name()
                )));
            }
        }
        if ors.is_empty() {
            return Ok(());
        }
        let soq: Arc<dyn SpanQuery> = Arc::new(SpanOrQuery::new(ors)?);
        if nots.is_empty() {
            span_clauses.push(soq);
        } else {
            let snqs: Arc<dyn SpanQuery> = Arc::new(SpanOrQuery::new(nots)?);
            let snq: Arc<dyn SpanQuery> = Arc::new(SpanNotQuery::new(soq, snqs)?);
            span_clauses.push(snq);
        }
        Ok(())
    }
}

impl Query for ComplexPhraseQuery {
    fn to_query_string(&self, field: &str) -> String {
        let mut sb = String::new();
        if self.field != field {
            sb.push_str(&self.field);
            sb.push(':');
        }
        sb.push('"');
        sb.push_str(&self.phrased_query_string_contents);
        sb.push('"');
        if self.slop_factor != 0 {
            sb.push('~');
            sb.push_str(&self.slop_factor.to_string());
        }
        sb
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        visitor.visit_leaf(self);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        let contents = self.contents();

        // ArrayList spanClauses = new ArrayList();
        if let Some(c) = &contents {
            if c.as_any().is::<TermQuery>()
                || c.is_multi_term_query()
                || c.as_any().is::<SynonymQuery>()
            {
                return Ok(Some(Arc::clone(c)));
            }
        }
        // Build a sequence of Span clauses arranged in a SpanNear - child
        // clauses can be complex Booleans e.g. nots and ors etc
        let mut num_negatives = 0;
        // `contents.getClass().getName()` on a `null` contents.
        let contents = crate::javacc::nn_owned(contents)?;
        let Some(bq) = contents.as_any().downcast_ref::<BooleanQuery>() else {
            return Err(self.unknown_query_type(contents.as_ref()));
        };
        let mut all_span_clauses: Vec<Arc<dyn SpanQuery>> = Vec::with_capacity(bq.clauses().len());
        // For all clauses e.g. one* two~
        for clause in bq.clauses() {
            // HashSet bclauseterms=new HashSet();
            // Rewrite this clause e.g one* becomes (one OR onerous)
            let mut qc = index_searcher.rewrite(Arc::clone(clause.query()))?;
            if clause.occur() == Occur::MUST_NOT {
                num_negatives += 1;
            }

            while let Some(boost) = qc.as_any().downcast_ref::<BoostQuery>() {
                qc = boost.get_query();
            }

            let boolean = qc.as_any().downcast_ref::<BooleanQuery>();
            let synonym = qc.as_any().downcast_ref::<SynonymQuery>();
            if boolean.is_some() || synonym.is_some() {
                let mut sc: Vec<Arc<dyn SpanQuery>> = Vec::new();
                let converted;
                let boolean_clause = match boolean {
                    Some(b) => b,
                    None => {
                        converted = Self::convert(synonym.expect("INVARIANT: checked above"))?;
                        &converted
                    }
                };
                self.add_complex_phrase_clause(&mut sc, boolean_clause)?;
                if !sc.is_empty() {
                    all_span_clauses.push(Arc::clone(&sc[0]));
                } else {
                    // Insert fake term e.g. phrase query was for "Fred
                    // Smithe*" and there were no "Smithe*" terms - need to
                    // prevent match on just "Fred".
                    all_span_clauses.push(self.dummy_clause());
                }
            } else if qc.as_any().is::<MatchNoDocsQuery>() {
                // Insert fake term e.g. phrase query was for "Fred Smithe*"
                // and there were no "Smithe*" terms - need to prevent match
                // on just "Fred".
                all_span_clauses.push(self.dummy_clause());
            } else if let Some(tq) = qc.as_any().downcast_ref::<TermQuery>() {
                all_span_clauses.push(Arc::new(SpanTermQuery::new(tq.get_term().clone())));
            } else {
                return Err(self.unknown_query_type(qc.as_ref()));
            }
        }
        if num_negatives == 0 {
            // The simple case - no negative elements in phrase
            return Ok(Some(Arc::new(SpanNearQuery::new(
                all_span_clauses,
                self.slop_factor,
                self.in_order,
            )?)));
        }
        // Complex case - we have mixed positives and negatives in the
        // sequence. Need to return a SpanNotQuery
        let mut positive_clauses: Vec<Arc<dyn SpanQuery>> = Vec::new();
        for (i, clause) in bq.clauses().iter().enumerate() {
            if clause.occur() != Occur::MUST_NOT {
                positive_clauses.push(Arc::clone(&all_span_clauses[i]));
            }
        }

        let include: Arc<dyn SpanQuery> = if positive_clauses.len() == 1 {
            Arc::clone(&positive_clauses[0]) // only one positive clause
        } else {
            // need to increase slop factor based on gaps introduced by
            // negatives
            Arc::new(SpanNearQuery::new(
                positive_clauses,
                self.slop_factor + num_negatives,
                self.in_order,
            )?)
        };
        // Use sequence of positive and negative values as the exclude.
        let exclude: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(
            all_span_clauses,
            self.slop_factor,
            self.in_order,
        )?);
        Ok(Some(Arc::new(SpanNotQuery::new(include, exclude)?)))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other.as_any().downcast_ref::<Self>().is_some_and(|other| {
                self.field == other.field
                    && self.phrased_query_string_contents == other.phrased_query_string_contents
                    && self.slop_factor == other.slop_factor
                    && self.in_order == other.in_order
            })
    }

    fn query_hash(&self) -> i32 {
        let prime: i32 = 31;
        let mut result = self.class_hash();
        result = prime
            .wrapping_mul(result)
            .wrapping_add(java_string_hash_code(&self.field));
        result = prime
            .wrapping_mul(result)
            .wrapping_add(java_string_hash_code(&self.phrased_query_string_contents));
        result = prime.wrapping_mul(result).wrapping_add(self.slop_factor);
        result = prime
            .wrapping_mul(result)
            .wrapping_add(if self.in_order { 1 } else { 0 });
        result
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queryparser.complexPhrase.ComplexPhraseQueryParser$ComplexPhraseQuery"
    }
}
