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

//! Port of `SpanTermBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.xml.builders`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::Term;
use rucene::search::query::Query;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;

use super::span_builder_base::SpanBuilderBase;
use super::span_query_builder::SpanQueryBuilder;
use crate::jdk::dom::Element;
use crate::xml::dom_utils::DOMUtils;
use crate::xml::query_builder::QueryBuilder;

/// Builder for `SpanTermQuery`
///
/// Equivalent to `org.apache.lucene.queryparser.xml.builders.SpanTermBuilder`, which extends `SpanBuilderBase`.
pub struct SpanTermBuilder {}

impl std::fmt::Debug for SpanTermBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SpanTermBuilder")
    }
}

impl Default for SpanTermBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SpanTermBuilder {
    /// Equivalent to `new SpanTermBuilder()`.
    pub fn new() -> Self {
        Self {}
    }
}

impl QueryBuilder for SpanTermBuilder {
    fn get_query(&self, e: &Element) -> Result<Arc<dyn Query>> {
        SpanBuilderBase::get_query(self, e)
    }
}

impl SpanQueryBuilder for SpanTermBuilder {
    /// Equivalent to `getSpanQuery(Element e)`: a `SpanTermQuery` of the
    /// `fieldName` attribute (inherited) and the element's text.
    fn get_span_query(&self, e: &Element) -> Result<Arc<dyn SpanQuery>> {
        let field_name = DOMUtils::get_attribute_with_inheritance_or_fail(e, "fieldName")?;
        let value = DOMUtils::get_non_blank_text_or_fail(e)?;
        Ok(Arc::new(SpanTermQuery::new(Term::from_text(
            field_name, &value,
        ))))
    }
}
