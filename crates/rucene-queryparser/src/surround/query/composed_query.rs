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

//! The port of `ComposedQuery.java`, of the Java package
//! `org.apache.lucene.queryparser.surround.query`.

use std::sync::Arc;

use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::search::Query;

use super::basic_query_factory::BasicQueryFactory;
use super::srnd_query::{SrndQuery, SrndQueryState};

/// The fields of Java's abstract `ComposedQuery`.
#[derive(Debug, Clone)]
pub struct ComposedQueryState {
    pub(crate) srnd: SrndQueryState,
    /// `protected String opName`.
    pub(crate) op_name: String,
    /// `protected List<SrndQuery> queries`.
    pub(crate) queries: Vec<Arc<dyn SrndQuery>>,
    operator_infix: bool,
}

impl ComposedQueryState {
    /// Equivalent to `new ComposedQuery(List<SrndQuery>, boolean, String)`.
    ///
    /// # Errors
    ///
    /// The `AssertionError("Too few subqueries")` of `recompose` for fewer
    /// than two queries.
    pub fn new(qs: Vec<Arc<dyn SrndQuery>>, operator_infix: bool, op_name: &str) -> Result<Self> {
        Ok(Self {
            srnd: SrndQueryState::new(),
            op_name: op_name.to_string(),
            queries: recompose(qs)?,
            operator_infix,
        })
    }
}

/// Equivalent to the protected `recompose(List<SrndQuery>)`.
fn recompose(queries: Vec<Arc<dyn SrndQuery>>) -> Result<Vec<Arc<dyn SrndQuery>>> {
    if queries.len() < 2 {
        return Err(LuceneError::from(LuceneErrorKind::AssertionError(
            "Too few subqueries".to_string(),
        )));
    }
    Ok(queries)
}

/// Base class for composite queries (such as AND/OR/NOT).
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queryparser.surround.query.ComposedQuery`.
pub trait ComposedQuery: SrndQuery {
    /// The fields of the composed query.
    fn composed(&self) -> &ComposedQueryState;

    /// Equivalent to `getOperatorName()`.
    fn get_operator_name(&self) -> String {
        self.composed().op_name.clone()
    }

    /// Equivalent to `getSubQueriesIterator()`.
    fn get_sub_queries_iterator(&self) -> std::slice::Iter<'_, Arc<dyn SrndQuery>> {
        self.composed().queries.iter()
    }

    /// Equivalent to `getNrSubQueries()`.
    fn get_nr_sub_queries(&self) -> i32 {
        self.composed().queries.len() as i32
    }

    /// Equivalent to `getSubQuery(int)`.
    fn get_sub_query(&self, qn: i32) -> Arc<dyn SrndQuery> {
        Arc::clone(&self.composed().queries[qn as usize])
    }

    /// Equivalent to `isOperatorInfix()`; else prefix operator.
    fn is_operator_infix(&self) -> bool {
        self.composed().operator_infix
    }

    /// Equivalent to `makeLuceneSubQueriesField(String, BasicQueryFactory)`.
    ///
    /// # Errors
    ///
    /// The errors of the sub-query construction.
    fn make_lucene_sub_queries_field(
        &self,
        fn_: Option<&str>,
        qf: &Arc<BasicQueryFactory>,
    ) -> Result<Vec<Arc<dyn Query>>> {
        let mut lucene_sub_queries = Vec::new();
        for sq in self.get_sub_queries_iterator() {
            lucene_sub_queries.push(Arc::clone(sq).make_lucene_query_field(fn_, qf)?);
        }
        Ok(lucene_sub_queries)
    }

    /// Override for different spacing.
    ///
    /// Equivalent to the protected `getPrefixSeparator()`.
    fn get_prefix_separator(&self) -> String {
        ", ".to_string()
    }

    /// Equivalent to the protected `getBracketOpen()`.
    fn get_bracket_open(&self) -> String {
        "(".to_string()
    }

    /// Equivalent to the protected `getBracketClose()`.
    fn get_bracket_close(&self) -> String {
        ")".to_string()
    }

    /// Brackets are possibly redundant in the result.
    ///
    /// Equivalent to the protected `infixToString(StringBuilder)`.
    fn infix_to_string(&self, r: &mut String) {
        let mut sqi = self.get_sub_queries_iterator();
        r.push_str(&self.get_bracket_open());
        if let Some(first) = sqi.next() {
            r.push_str(&first.srnd_to_string());
            for next in sqi {
                r.push(' ');
                r.push_str(&self.get_operator_name()); /* infix operator */
                r.push(' ');
                r.push_str(&next.srnd_to_string());
            }
        }
        r.push_str(&self.get_bracket_close());
    }

    /// Equivalent to the protected `prefixToString(StringBuilder)`.
    fn prefix_to_string(&self, r: &mut String) {
        let mut sqi = self.get_sub_queries_iterator();
        r.push_str(&self.get_operator_name()); /* prefix operator */
        r.push_str(&self.get_bracket_open());
        if let Some(first) = sqi.next() {
            r.push_str(&first.srnd_to_string());
            for next in sqi {
                r.push_str(&self.get_prefix_separator());
                r.push_str(&next.srnd_to_string());
            }
        }
        r.push_str(&self.get_bracket_close());
    }
}

/// `ComposedQuery.toString()`, the override of
/// [`SrndQuery::srnd_to_string`] every composed query delegates to.
pub fn to_string<C: ComposedQuery + ?Sized>(c: &C) -> String {
    let mut r = String::new();
    if c.is_operator_infix() {
        c.infix_to_string(&mut r);
    } else {
        c.prefix_to_string(&mut r);
    }
    c.weight_to_string(&mut r);
    r
}

/// `ComposedQuery.isFieldsSubQueryAcceptable()`: at least one subquery
/// should be acceptable.
pub fn is_fields_sub_query_acceptable<C: ComposedQuery + ?Sized>(c: &C) -> bool {
    c.get_sub_queries_iterator()
        .any(|sq| sq.is_fields_sub_query_acceptable())
}
