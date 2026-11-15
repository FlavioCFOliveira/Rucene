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

//! The port of `SimpleTerm.java`, of the Java package
//! `org.apache.lucene.queryparser.surround.query`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{IndexReader, Term};
use rucene::search::Query;

use super::basic_query_factory::BasicQueryFactory;
use super::distance_sub_query::DistanceSubQuery;
use super::simple_term_rewrite_query::SimpleTermRewriteQuery;
use super::span_near_clause_factory::SpanNearClauseFactory;
use super::srnd_query::SrndQuery;

/// Base class for queries that expand to sets of simple terms.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queryparser.surround.query.SimpleTerm`, which
/// implements `DistanceSubQuery`.
pub trait SimpleTerm: SrndQuery + DistanceSubQuery {
    /// Equivalent to the package-private `isQuoted()`.
    fn is_quoted(&self) -> bool;

    /// Equivalent to `getQuote()`.
    fn get_quote(&self) -> String {
        "\"".to_string()
    }

    /// Equivalent to `getFieldOperator()`.
    fn get_field_operator(&self) -> String {
        "/".to_string()
    }

    /// Equivalent to the abstract `toStringUnquoted()`.
    fn to_string_unquoted(&self) -> String;

    /// Override for prefix query.
    ///
    /// Equivalent to the protected `suffixToString(StringBuilder)`.
    fn suffix_to_string(&self, r: &mut String) {
        let _ = r;
    }

    /// Equivalent to the abstract `visitMatchingTerms(IndexReader, String,
    /// MatchingTermVisitor)`.
    ///
    /// # Errors
    ///
    /// The `IOException` of the index access and the errors of the
    /// visitor.
    fn visit_matching_terms(
        &self,
        reader: &Arc<dyn IndexReader>,
        field_name: &str,
        mtv: &mut dyn MatchingTermVisitor,
    ) -> Result<()>;
}

/// Callback to visit each matching term during "rewrite" in
/// `visit_matching_term`.
///
/// Equivalent to `org.apache.lucene.queryparser.surround.query.SimpleTerm.MatchingTermVisitor`.
pub trait MatchingTermVisitor {
    /// Equivalent to `visitMatchingTerm(Term)`.
    ///
    /// # Errors
    ///
    /// The `IOException` of the visitor.
    fn visit_matching_term(&mut self, t: Term) -> Result<()>;
}

/// `SimpleTerm.toString()`, the override of [`SrndQuery::srnd_to_string`]
/// every simple term delegates to.
pub fn to_string<T: SimpleTerm + ?Sized>(t: &T) -> String {
    let mut r = String::new();
    if t.is_quoted() {
        r.push_str(&t.get_quote());
    }
    r.push_str(&t.to_string_unquoted());
    if t.is_quoted() {
        r.push_str(&t.get_quote());
    }
    t.suffix_to_string(&mut r);
    t.weight_to_string(&mut r);
    r
}

/// `SimpleTerm.distanceSubQueryNotAllowed()`: `null`.
pub fn distance_sub_query_not_allowed<T: SimpleTerm + ?Sized>(t: &T) -> Option<String> {
    let _ = t;
    None
}

/// `SimpleTerm.addSpanQueries(SpanNearClauseFactory)`: every matching term,
/// with this term's weight.
///
/// # Errors
///
/// As [`SimpleTerm::visit_matching_terms`].
pub fn add_span_queries<T: SimpleTerm + ?Sized>(
    t: &T,
    sncf: &mut SpanNearClauseFactory,
) -> Result<()> {
    struct Visitor<'a> {
        sncf: &'a mut SpanNearClauseFactory,
        weight: f32,
    }
    impl MatchingTermVisitor for Visitor<'_> {
        fn visit_matching_term(&mut self, term: Term) -> Result<()> {
            self.sncf.add_term_weighted(term, self.weight)
        }
    }
    let reader = Arc::clone(sncf.get_index_reader());
    let field_name = sncf.get_field_name().to_string();
    let weight = t.get_weight();
    t.visit_matching_terms(&reader, &field_name, &mut Visitor { sncf, weight })
}

/// `SimpleTerm.makeLuceneQueryFieldNoBoost(String, BasicQueryFactory)`: a
/// `SimpleTermRewriteQuery` over this term.
///
/// # Errors
///
/// The `NullPointerException` of a `null` field name.
pub fn make_lucene_query_field_no_boost<T: SimpleTerm + 'static>(
    this: Arc<T>,
    field_name: Option<&str>,
    qf: &Arc<BasicQueryFactory>,
) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(SimpleTermRewriteQuery::new(this, field_name, qf)?))
}
