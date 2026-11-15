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

//! Port of `SpanNearBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.xml.builders`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::query::Query;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_query::SpanQuery;

use crate::jdk::dom::ELEMENT_NODE;

use super::span_builder_base::SpanBuilderBase;
use super::span_query_builder::SpanQueryBuilder;
use crate::jdk::dom::Element;
use crate::xml::dom_utils::DOMUtils;
use crate::xml::query_builder::QueryBuilder;

/// Builder for `SpanNearQuery`
///
/// Equivalent to `org.apache.lucene.queryparser.xml.builders.SpanNearBuilder`, which extends `SpanBuilderBase`.
pub struct SpanNearBuilder {
    factory: Arc<dyn SpanQueryBuilder>,
}

impl std::fmt::Debug for SpanNearBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SpanNearBuilder")
    }
}

impl SpanNearBuilder {
    /// Equivalent to `new SpanNearBuilder(SpanQueryBuilder factory)`.
    pub fn new(factory: Arc<dyn SpanQueryBuilder>) -> Self {
        Self { factory }
    }
}

impl QueryBuilder for SpanNearBuilder {
    fn get_query(&self, e: &Element) -> Result<Arc<dyn Query>> {
        SpanBuilderBase::get_query(self, e)
    }
}

impl SpanQueryBuilder for SpanNearBuilder {
    /// Equivalent to `getSpanQuery(Element e)`: a `SpanNearQuery` of the
    /// child elements, with the `slop` attribute and the `inOrder` one
    /// (false by default).
    ///
    /// # Errors
    ///
    /// The `NumberFormatException` of an empty or absent `slop`, and what
    /// the child builders throw.
    fn get_span_query(&self, e: &Element) -> Result<Arc<dyn SpanQuery>> {
        let slop_string = DOMUtils::get_attribute_or_fail(e, "slop")?;
        let slop = crate::javacc::parse_int(&slop_string)?;
        let in_order = DOMUtils::get_attribute_boolean(e, "inOrder", false);
        let mut spans: Vec<Arc<dyn SpanQuery>> = Vec::new();
        let mut kid = e.get_first_child();
        while let Some(k) = kid {
            if k.get_node_type() == ELEMENT_NODE {
                spans.push(self.factory.get_span_query(&Element::cast(&k)?)?);
            }
            kid = k.get_next_sibling();
        }
        Ok(Arc::new(SpanNearQuery::new(spans, slop, in_order)?))
    }
}
