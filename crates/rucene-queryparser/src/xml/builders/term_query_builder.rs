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

//! Port of `TermQueryBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.xml.builders`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::Term;
use rucene::search::boost_query::BoostQuery;
use rucene::search::query::Query;
use rucene::search::term_query::TermQuery;

use crate::jdk::dom::Element;
use crate::xml::dom_utils::DOMUtils;
use crate::xml::query_builder::QueryBuilder;

/// Builder for `TermQuery`
///
/// Equivalent to `org.apache.lucene.queryparser.xml.builders.TermQueryBuilder`.
#[derive(Debug, Default)]
pub struct TermQueryBuilder;

impl TermQueryBuilder {
    /// Equivalent to `new TermQueryBuilder()`.
    pub fn new() -> Self {
        Self
    }
}

impl QueryBuilder for TermQueryBuilder {
    /// Equivalent to `getQuery(Element e)`: a `TermQuery` of the `fieldName`
    /// attribute (inherited) and the element's text, boosted by the `boost`
    /// attribute when it is not 1.
    fn get_query(&self, e: &Element) -> Result<Arc<dyn Query>> {
        let field = DOMUtils::get_attribute_with_inheritance_or_fail(e, "fieldName")?;
        let value = DOMUtils::get_non_blank_text_or_fail(e)?;
        let mut tq: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text(field, &value)));
        let boost = DOMUtils::get_attribute_float(e, "boost", 1.0)?;
        if boost != 1.0 {
            tq = Arc::new(BoostQuery::new(tq, boost)?);
        }
        Ok(tq)
    }
}
