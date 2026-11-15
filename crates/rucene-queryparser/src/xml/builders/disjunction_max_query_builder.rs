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

//! Port of `DisjunctionMaxQueryBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.xml.builders`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::boost_query::BoostQuery;
use rucene::search::disjunction_max_query::DisjunctionMaxQuery;
use rucene::search::query::Query;

use crate::jdk::dom::Element;
use crate::xml::dom_utils::DOMUtils;
use crate::xml::query_builder::QueryBuilder;

/// Builder for `DisjunctionMaxQuery`
///
/// Equivalent to
/// `org.apache.lucene.queryparser.xml.builders.DisjunctionMaxQueryBuilder`.
pub struct DisjunctionMaxQueryBuilder {
    factory: Arc<dyn QueryBuilder>,
}

impl std::fmt::Debug for DisjunctionMaxQueryBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DisjunctionMaxQueryBuilder")
    }
}

impl DisjunctionMaxQueryBuilder {
    /// Equivalent to `new DisjunctionMaxQueryBuilder(QueryBuilder factory)`.
    pub fn new(factory: Arc<dyn QueryBuilder>) -> Self {
        Self { factory }
    }
}

impl QueryBuilder for DisjunctionMaxQueryBuilder {
    /// Equivalent to `getQuery(Element e)`: a `DisjunctionMaxQuery` of every
    /// child element, with the `tieBreaker` and `boost` attributes.
    fn get_query(&self, e: &Element) -> Result<Arc<dyn Query>> {
        let tie_breaker = DOMUtils::get_attribute_float(e, "tieBreaker", 0.0)?;
        let mut disjuncts: Vec<Arc<dyn Query>> = Vec::new();
        let nl = e.get_child_nodes();
        let nl_len = nl.get_length();
        for i in 0..nl_len {
            if let Some(query_elem) = nl.item(i).and_then(|node| node.as_element()) {
                // all elements are disjuncts.
                let q = self.factory.get_query(&query_elem)?;
                disjuncts.push(q);
            }
        }
        let mut q: Arc<dyn Query> = Arc::new(DisjunctionMaxQuery::new(disjuncts, tie_breaker)?);
        let boost = DOMUtils::get_attribute_float(e, "boost", 1.0)?;
        if boost != 1.0 {
            q = Arc::new(BoostQuery::new(q, boost)?);
        }
        Ok(q)
    }
}
