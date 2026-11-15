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

//! Port of `SpanNotBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.xml.builders`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::query::Query;
use rucene_queries::spans::span_not_query::SpanNotQuery;
use rucene_queries::spans::span_query::SpanQuery;

use super::span_builder_base::SpanBuilderBase;
use super::span_query_builder::SpanQueryBuilder;
use crate::jdk::dom::Element;
use crate::xml::dom_utils::DOMUtils;
use crate::xml::query_builder::QueryBuilder;

/// Builder for `SpanNotQuery`
///
/// Equivalent to `org.apache.lucene.queryparser.xml.builders.SpanNotBuilder`, which extends `SpanBuilderBase`.
pub struct SpanNotBuilder {
    factory: Arc<dyn SpanQueryBuilder>,
}

impl std::fmt::Debug for SpanNotBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SpanNotBuilder")
    }
}

impl SpanNotBuilder {
    /// Equivalent to `new SpanNotBuilder(SpanQueryBuilder factory)`.
    pub fn new(factory: Arc<dyn SpanQueryBuilder>) -> Self {
        Self { factory }
    }
}

impl QueryBuilder for SpanNotBuilder {
    fn get_query(&self, e: &Element) -> Result<Arc<dyn Query>> {
        SpanBuilderBase::get_query(self, e)
    }
}

impl SpanQueryBuilder for SpanNotBuilder {
    /// Equivalent to `getSpanQuery(Element e)`: a `SpanNotQuery` of the
    /// first child of `Include` and the first child of `Exclude`.
    fn get_span_query(&self, e: &Element) -> Result<Arc<dyn SpanQuery>> {
        let include_elem = DOMUtils::get_child_by_tag_or_fail(e, "Include")?;
        let include_elem = DOMUtils::get_first_child_or_fail(&include_elem)?;
        let exclude_elem = DOMUtils::get_child_by_tag_or_fail(e, "Exclude")?;
        let exclude_elem = DOMUtils::get_first_child_or_fail(&exclude_elem)?;
        let include = self.factory.get_span_query(&include_elem)?;
        let exclude = self.factory.get_span_query(&exclude_elem)?;
        Ok(Arc::new(SpanNotQuery::new(include, exclude)?))
    }
}
