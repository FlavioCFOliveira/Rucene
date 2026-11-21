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

//! Port of `QueryParserUtil.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard`.

use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::error::{LuceneError, Result};
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder;
use rucene::search::Query;

use super::standard_query_parser::StandardQueryParser;

/// This class defines utility methods to (help) parse query strings into
/// [`Query`] objects.
///
/// Equivalent to the final class
/// `org.apache.lucene.queryparser.flexible.standard.QueryParserUtil`, whose
/// methods are static; the overloads of `parse` are named by their
/// parameters.
#[derive(Debug)]
pub struct QueryParserUtil {
    _no_instances: (),
}

impl QueryParserUtil {
    /// Parses a query which searches on the fields specified.
    ///
    /// If x fields are specified, this effectively constructs: `(field1:query1)
    /// (field2:query2) (field3:query3)...(fieldx:queryx)`
    ///
    /// Equivalent to `parse(String[] queries, String[] fields, Analyzer
    /// analyzer)`.
    ///
    /// * `queries` - Queries strings to parse
    /// * `fields` - Fields to search on (`None` for Java's `null`)
    /// * `analyzer` - Analyzer to use
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` if the length of the queries array differs
    /// from the length of the fields array, and the `QueryNodeException` of
    /// a query that does not parse.
    pub fn parse_queries(
        queries: &[&str],
        fields: &[Option<&str>],
        analyzer: Option<Arc<dyn Analyzer>>,
    ) -> Result<Arc<dyn Query>> {
        if queries.len() != fields.len() {
            return Err(LuceneError::illegal_argument(
                "queries.length != fields.length",
            ));
        }
        let mut b_query = Builder::new();

        let mut qp = StandardQueryParser::new();
        qp.set_analyzer(analyzer)?;

        for i in 0..fields.len() {
            let q = qp.parse(queries[i], fields[i])?;

            if let Some(q) = q {
                // q never null, just being defensive
                b_query.add(q, Occur::SHOULD)?;
            }
        }
        Ok(Arc::new(b_query.build()))
    }

    /// Parses a query, searching on the fields specified. Use this if you
    /// need to specify certain fields as required, and others as prohibited.
    ///
    /// For the fields `filename`, `contents` and `description` with the flags
    /// `SHOULD`, `MUST` and `MUST_NOT`, it constructs the query
    /// `(filename:query) +(contents:query) -(description:query)`.
    ///
    /// Equivalent to `parse(String query, String[] fields,
    /// BooleanClause.Occur[] flags, Analyzer analyzer)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` if the length of the fields array differs
    /// from the length of the flags array, and the `QueryNodeException` of a
    /// query that does not parse.
    pub fn parse_with_flags(
        query: &str,
        fields: &[Option<&str>],
        flags: &[Occur],
        analyzer: Option<Arc<dyn Analyzer>>,
    ) -> Result<Arc<dyn Query>> {
        if fields.len() != flags.len() {
            return Err(LuceneError::illegal_argument(
                "fields.length != flags.length",
            ));
        }
        let mut b_query = Builder::new();

        let mut qp = StandardQueryParser::new();
        qp.set_analyzer(analyzer)?;

        for i in 0..fields.len() {
            let q = qp.parse(query, fields[i])?;

            if let Some(q) = q {
                // q never null, just being defensive
                b_query.add(q, flags[i])?;
            }
        }
        Ok(Arc::new(b_query.build()))
    }

    /// Parses a query, searching on the fields specified. Use this if you
    /// need to specify certain fields as required, and others as prohibited.
    ///
    /// For the queries `query1`, `query2` and `query3`, the fields
    /// `filename`, `contents` and `description` and the flags `SHOULD`,
    /// `MUST` and `MUST_NOT`, it constructs the query `(filename:query1)
    /// +(contents:query2) -(description:query3)`.
    ///
    /// Equivalent to `parse(String[] queries, String[] fields,
    /// BooleanClause.Occur[] flags, Analyzer analyzer)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` if the length of the queries, fields, and
    /// flags array differ, and the `QueryNodeException` of a query that does
    /// not parse.
    pub fn parse_queries_with_flags(
        queries: &[&str],
        fields: &[Option<&str>],
        flags: &[Occur],
        analyzer: Option<Arc<dyn Analyzer>>,
    ) -> Result<Arc<dyn Query>> {
        if !(queries.len() == fields.len() && queries.len() == flags.len()) {
            return Err(LuceneError::illegal_argument(
                "queries, fields, and flags array have have different length",
            ));
        }
        let mut b_query = Builder::new();

        let mut qp = StandardQueryParser::new();
        qp.set_analyzer(analyzer)?;

        for i in 0..fields.len() {
            let q = qp.parse(queries[i], fields[i])?;

            if let Some(q) = q {
                // q never null, just being defensive
                b_query.add(q, flags[i])?;
            }
        }
        Ok(Arc::new(b_query.build()))
    }

    /// Returns a String where those characters that TextParser expects to be
    /// escaped are escaped by a preceding `\`.
    ///
    /// Equivalent to `escape(String s)`.
    pub fn escape(s: &str) -> String {
        let mut sb = String::new();
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
}
