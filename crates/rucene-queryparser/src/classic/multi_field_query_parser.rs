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

//! The port of `MultiFieldQueryParser.java`, of the Java package
//! `org.apache.lucene.queryparser.classic`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::error::{LuceneError, Result};
use rucene::search::boolean_query::{self, BooleanQuery};
use rucene::search::boost_query::BoostQuery;
use rucene::search::multi_phrase_query::{self, MultiPhraseQuery};
use rucene::search::phrase_query::{self, PhraseQuery};
use rucene::search::{Occur, Query};
use rucene::util::query_builder::{QueryBuilder, QueryBuilderOps};

use super::query_parser::{self, QueryParser, QueryParserOps};
use super::query_parser_base::{self as base, QueryParserBase, QueryParserBaseOps};
use super::query_parser_constants::QueryParserConstants;
use crate::charstream::char_stream::CharStream;

/// A QueryParser which constructs queries to search multiple fields.
///
/// Equivalent to `org.apache.lucene.queryparser.classic.MultiFieldQueryParser`.
#[derive(Debug)]
pub struct MultiFieldQueryParser {
    /// The `QueryParser` state Java's class extends.
    parser: QueryParser,
    /// `protected String[] fields`.
    pub fields: Vec<String>,
    /// `protected Map<String, Float> boosts`, `None` for `null`. A field
    /// mapped to `null` has no boost, as a field absent from the map.
    pub boosts: Option<HashMap<String, f32>>,
}

impl MultiFieldQueryParser {
    /// Creates a MultiFieldQueryParser. Allows passing of a map with term to
    /// Boost, and the boost to apply to each term.
    ///
    /// It will, when parse(String query) is called, construct a query like
    /// this (assuming the query consists of two terms and you specify the
    /// two fields `title` and `body`):
    ///
    /// `(title:term1 body:term1) (title:term2 body:term2)`
    ///
    /// When setDefaultOperator(AND_OPERATOR) is set, the result will be:
    ///
    /// `+(title:term1 body:term1) +(title:term2 body:term2)`
    ///
    /// When you pass a boost (title=>5 body=>10) you can get
    ///
    /// `+(title:term1^5.0 body:term1^10.0) +(title:term2^5.0 body:term2^10.0)`
    ///
    /// In other words, all the query's terms must appear, but it doesn't
    /// matter in what fields they appear.
    ///
    /// Equivalent to `new MultiFieldQueryParser(String[], Analyzer,
    /// Map<String, Float>)`.
    ///
    /// # Errors
    ///
    /// As [`QueryParser::new`].
    pub fn with_boosts(
        fields: Vec<String>,
        analyzer: Option<Arc<dyn Analyzer>>,
        boosts: Option<HashMap<String, f32>>,
    ) -> Result<Self> {
        let mut parser = Self::new(fields, analyzer)?;
        parser.boosts = boosts;
        Ok(parser)
    }

    /// Creates a MultiFieldQueryParser.
    ///
    /// It will, when parse(String query) is called, construct a query like
    /// this (assuming the query consists of two terms and you specify the
    /// two fields `title` and `body`):
    ///
    /// `(title:term1 body:term1) (title:term2 body:term2)`
    ///
    /// Equivalent to `new MultiFieldQueryParser(String[], Analyzer)`.
    ///
    /// # Errors
    ///
    /// As [`QueryParser::new`].
    pub fn new(fields: Vec<String>, analyzer: Option<Arc<dyn Analyzer>>) -> Result<Self> {
        Ok(Self {
            parser: QueryParser::new(None, analyzer)?,
            fields,
            boosts: None,
        })
    }

    /// The boost of `field`, if the map gives one.
    fn boost_of(&self, field: &str) -> Option<f32> {
        self.boosts.as_ref().and_then(|b| b.get(field).copied())
    }

    /// Equivalent to the private `applySlop(Query, int)`.
    fn apply_slop(q: Arc<dyn Query>, slop: i32) -> Result<Arc<dyn Query>> {
        if let Some(pq) = q.as_any().downcast_ref::<PhraseQuery>() {
            let mut builder = phrase_query::Builder::new();
            builder.set_slop(slop);
            let terms = pq.get_terms();
            let positions = pq.get_positions();
            for i in 0..terms.len() {
                builder.add_at(terms[i].clone(), positions[i])?;
            }
            return Ok(Arc::new(builder.build()?));
        } else if let Some(mpq) = q.as_any().downcast_ref::<MultiPhraseQuery>() {
            if slop != mpq.get_slop() {
                return Ok(Arc::new(
                    multi_phrase_query::Builder::from_query(mpq)
                        .set_slop(slop)?
                        .build(),
                ));
            }
        } else if let Some(boost_query) = q.as_any().downcast_ref::<BoostQuery>() {
            let sub_query = Self::apply_slop(boost_query.get_query(), slop)?;
            return Ok(Arc::new(BoostQuery::new(
                sub_query,
                boost_query.get_boost(),
            )?));
        }
        Ok(q)
    }

    /// Equivalent to the private `applyBoost(Query, String)`.
    fn apply_boost(
        &self,
        q: Option<Arc<dyn Query>>,
        field: &str,
    ) -> Result<Option<Arc<dyn Query>>> {
        if let Some(boost) = self.boost_of(field) {
            // `new BoostQuery(q, boost)` on a `null` query.
            let q = crate::javacc::nn_owned(q)?;
            return Ok(Some(Arc::new(BoostQuery::new(q, boost)?)));
        }
        Ok(q)
    }

    /// Creates a multifield query.
    ///
    /// Equivalent to the protected `getMultiFieldQuery(List<Query>)`.
    ///
    /// # Errors
    ///
    /// The `TooManyClauses` of the boolean query builder.
    pub fn get_multi_field_query(
        &mut self,
        queries: Vec<Option<Arc<dyn Query>>>,
    ) -> Result<Option<Arc<dyn Query>>> {
        if queries.is_empty() {
            return Ok(None); // all clause words were filtered away by the analyzer.
        }
        let mut query = self.new_boolean_query();
        for sub in queries {
            // `query.add(sub, SHOULD)` on a `null` sub-query.
            query.add(crate::javacc::nn_owned(sub)?, Occur::SHOULD)?;
        }
        Ok(Some(Arc::new(query.build())))
    }

    /// Parses a query which searches on the fields specified.
    ///
    /// If x fields are specified, this effectively constructs:
    ///
    /// `(field1:query1) (field2:query2) (field3:query3)...(fieldx:queryx)`
    ///
    /// Equivalent to the static `parse(String[], String[], Analyzer)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of arrays of different lengths, and
    /// the `ParseException` of a query that does not parse.
    pub fn parse_queries(
        queries: &[&str],
        fields: &[&str],
        analyzer: Option<Arc<dyn Analyzer>>,
    ) -> Result<Arc<dyn Query>> {
        if queries.len() != fields.len() {
            return Err(LuceneError::illegal_argument(
                "queries.length != fields.length",
            ));
        }
        let mut b_query = boolean_query::Builder::new();
        for i in 0..fields.len() {
            let mut qp = QueryParser::new(Some(fields[i]), analyzer.clone())?;
            let q = qp.parse(queries[i])?;
            if is_not_empty(&q) {
                b_query.add(q, Occur::SHOULD)?;
            }
        }
        Ok(Arc::new(b_query.build()))
    }

    /// Parses a query, searching on the fields specified. Use this if you
    /// need to specify certain fields as required, and others as prohibited.
    ///
    /// Equivalent to the static `parse(String, String[],
    /// BooleanClause.Occur[], Analyzer)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of arrays of different lengths, and
    /// the `ParseException` of a query that does not parse.
    pub fn parse_with_flags(
        query: &str,
        fields: &[&str],
        flags: &[Occur],
        analyzer: Option<Arc<dyn Analyzer>>,
    ) -> Result<Arc<dyn Query>> {
        if fields.len() != flags.len() {
            return Err(LuceneError::illegal_argument(
                "fields.length != flags.length",
            ));
        }
        let mut b_query = boolean_query::Builder::new();
        for i in 0..fields.len() {
            let mut qp = QueryParser::new(Some(fields[i]), analyzer.clone())?;
            let q = qp.parse(query)?;
            if is_not_empty(&q) {
                b_query.add(q, flags[i])?;
            }
        }
        Ok(Arc::new(b_query.build()))
    }

    /// Parses a query, searching on the fields specified. Use this if you
    /// need to specify certain fields as required, and others as prohibited.
    ///
    /// Equivalent to the static `parse(String[], String[],
    /// BooleanClause.Occur[], Analyzer)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of arrays of different lengths, and
    /// the `ParseException` of a query that does not parse.
    pub fn parse_queries_with_flags(
        queries: &[&str],
        fields: &[&str],
        flags: &[Occur],
        analyzer: Option<Arc<dyn Analyzer>>,
    ) -> Result<Arc<dyn Query>> {
        if !(queries.len() == fields.len() && queries.len() == flags.len()) {
            return Err(LuceneError::illegal_argument(
                "queries, fields, and flags array have have different length",
            ));
        }
        let mut b_query = boolean_query::Builder::new();
        for i in 0..fields.len() {
            let mut qp = QueryParser::new(Some(fields[i]), analyzer.clone())?;
            let q = qp.parse(queries[i])?;
            if is_not_empty(&q) {
                b_query.add(q, flags[i])?;
            }
        }
        Ok(Arc::new(b_query.build()))
    }
}

/// `q != null && (!(q instanceof BooleanQuery) || ((BooleanQuery)
/// q).clauses().size() > 0)`: `parse` never returns `null`.
fn is_not_empty(q: &Arc<dyn Query>) -> bool {
    match q.as_any().downcast_ref::<BooleanQuery>() {
        Some(bq) => !bq.clauses().is_empty(),
        None => true,
    }
}

impl QueryParserConstants for MultiFieldQueryParser {}

impl QueryBuilderOps for MultiFieldQueryParser {
    fn query_builder(&self) -> &QueryBuilder {
        &self.parser.base.builder
    }
}

impl QueryParserOps for MultiFieldQueryParser {
    fn query_parser(&self) -> &QueryParser {
        &self.parser
    }

    fn query_parser_mut(&mut self) -> &mut QueryParser {
        &mut self.parser
    }
}

impl QueryParserBaseOps for MultiFieldQueryParser {
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
        let Some(field) = field else {
            let mut clauses: Vec<Arc<dyn Query>> = Vec::new();
            for i in 0..self.fields.len() {
                let f = self.fields[i].clone();
                let q = base::get_field_query(self, Some(&f), query_text, true)?;
                if let Some(mut q) = q {
                    // If the user passes a map of boosts
                    if let Some(boost) = self.boost_of(&f) {
                        // Get the boost from the map and apply them
                        q = Arc::new(BoostQuery::new(q, boost)?);
                    }
                    q = Self::apply_slop(q, slop)?;
                    clauses.push(q);
                }
            }
            if clauses.is_empty() {
                // happens for stopwords
                return Ok(None);
            }
            return self.get_multi_field_query(clauses.into_iter().map(Some).collect());
        };
        let q = base::get_field_query(self, Some(field), query_text, true)?;
        // `applySlop(q, slop)` on a `null` query: `null instanceof X` is
        // false, so `null` comes back.
        Ok(match q {
            Some(q) => Some(Self::apply_slop(q, slop)?),
            None => None,
        })
    }

    fn get_field_query(
        &mut self,
        field: Option<&str>,
        query_text: &str,
        quoted: bool,
    ) -> Result<Option<Arc<dyn Query>>> {
        let Some(field) = field else {
            let mut clauses: Vec<Arc<dyn Query>> = Vec::new();
            let mut field_queries: Vec<Option<Arc<dyn Query>>> =
                Vec::with_capacity(self.fields.len());
            let mut max_terms = 0usize;
            for i in 0..self.fields.len() {
                let f = self.fields[i].clone();
                let q = base::get_field_query(self, Some(&f), query_text, quoted)?;
                if let Some(q) = &q {
                    if let Some(bq) = q.as_any().downcast_ref::<BooleanQuery>() {
                        max_terms = max_terms.max(bq.clauses().len());
                    } else {
                        max_terms = max_terms.max(1);
                    }
                }
                field_queries.push(q);
            }
            for term_num in 0..max_terms {
                let mut term_clauses: Vec<Arc<dyn Query>> = Vec::new();
                for (i, field_query) in field_queries.iter().enumerate() {
                    if let Some(field_query) = field_query {
                        let mut q: Option<Arc<dyn Query>> = None;
                        if let Some(bq) = field_query.as_any().downcast_ref::<BooleanQuery>() {
                            let nested_clauses = bq.clauses();
                            if term_num < nested_clauses.len() {
                                q = Some(Arc::clone(nested_clauses[term_num].query()));
                            }
                        } else if term_num == 0 {
                            // e.g. TermQuery-s
                            q = Some(Arc::clone(field_query));
                        }
                        if let Some(mut q) = q {
                            if let Some(boost) = self.boost_of(&self.fields[i]) {
                                // Get the boost from the map and apply them
                                q = Arc::new(BoostQuery::new(q, boost)?);
                            }
                            term_clauses.push(q);
                        }
                    }
                }
                if max_terms > 1 {
                    if !term_clauses.is_empty() {
                        let mut builder = self.new_boolean_query();
                        for term_clause in term_clauses {
                            builder.add(term_clause, Occur::SHOULD)?;
                        }
                        clauses.push(Arc::new(builder.build()));
                    }
                } else {
                    clauses.extend(term_clauses);
                }
            }
            if clauses.is_empty() {
                // happens for stopwords
                return Ok(None);
            }
            return self.get_multi_field_query(clauses.into_iter().map(Some).collect());
        };
        base::get_field_query(self, Some(field), query_text, quoted)
    }

    fn get_fuzzy_query(
        &mut self,
        field: Option<&str>,
        term_str: &str,
        min_similarity: f32,
    ) -> Result<Option<Arc<dyn Query>>> {
        let Some(field) = field else {
            let mut clauses = Vec::new();
            for i in 0..self.fields.len() {
                let f = self.fields[i].clone();
                clauses.push(self.get_fuzzy_query(Some(&f), term_str, min_similarity)?);
            }
            return self.get_multi_field_query(clauses);
        };
        let q = base::get_fuzzy_query(self, Some(field), term_str, min_similarity)?;
        self.apply_boost(q, field)
    }

    fn get_prefix_query(
        &mut self,
        field: Option<&str>,
        term_str: &str,
    ) -> Result<Option<Arc<dyn Query>>> {
        let Some(field) = field else {
            let mut clauses = Vec::new();
            for i in 0..self.fields.len() {
                let f = self.fields[i].clone();
                clauses.push(self.get_prefix_query(Some(&f), term_str)?);
            }
            return self.get_multi_field_query(clauses);
        };
        let q = base::get_prefix_query(self, Some(field), term_str)?;
        self.apply_boost(q, field)
    }

    fn get_wildcard_query(
        &mut self,
        field: Option<&str>,
        term_str: &str,
    ) -> Result<Option<Arc<dyn Query>>> {
        let Some(field) = field else {
            let mut clauses = Vec::new();
            for i in 0..self.fields.len() {
                let f = self.fields[i].clone();
                clauses.push(self.get_wildcard_query(Some(&f), term_str)?);
            }
            return self.get_multi_field_query(clauses);
        };
        let q = base::get_wildcard_query(self, Some(field), term_str)?;
        self.apply_boost(q, field)
    }

    fn get_range_query(
        &mut self,
        field: Option<&str>,
        part1: Option<&str>,
        part2: Option<&str>,
        start_inclusive: bool,
        end_inclusive: bool,
    ) -> Result<Option<Arc<dyn Query>>> {
        let Some(field) = field else {
            let mut clauses = Vec::new();
            for i in 0..self.fields.len() {
                let f = self.fields[i].clone();
                clauses.push(self.get_range_query(
                    Some(&f),
                    part1,
                    part2,
                    start_inclusive,
                    end_inclusive,
                )?);
            }
            return self.get_multi_field_query(clauses);
        };
        let q = base::get_range_query(
            self,
            Some(field),
            part1,
            part2,
            start_inclusive,
            end_inclusive,
        )?;
        self.apply_boost(q, field)
    }

    fn get_regexp_query(
        &mut self,
        field: Option<&str>,
        term_str: &str,
    ) -> Result<Option<Arc<dyn Query>>> {
        let Some(field) = field else {
            let mut clauses = Vec::new();
            for i in 0..self.fields.len() {
                let f = self.fields[i].clone();
                clauses.push(self.get_regexp_query(Some(&f), term_str)?);
            }
            return self.get_multi_field_query(clauses);
        };
        let q = base::get_regexp_query(self, Some(field), term_str)?;
        self.apply_boost(q, field)
    }
}
