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

//! Port of `SpanPositionRangeBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.xml.builders`.

use std::sync::Arc;

use rucene::error::LuceneError;
use rucene::error::Result;
use rucene::search::query::Query;
use rucene_queries::spans::span_position_range_query::SpanPositionRangeQuery;
use rucene_queries::spans::span_query::SpanQuery;

use super::span_builder_base::SpanBuilderBase;
use super::span_query_builder::SpanQueryBuilder;
use crate::jdk::dom::Element;
use crate::xml::dom_utils::DOMUtils;
use crate::xml::query_builder::QueryBuilder;

/// Builder for `SpanPositionRangeQuery`
///
/// Equivalent to `org.apache.lucene.queryparser.xml.builders.SpanPositionRangeBuilder`, which extends `SpanBuilderBase`.
pub struct SpanPositionRangeBuilder {
    factory: Arc<dyn SpanQueryBuilder>,
}

impl std::fmt::Debug for SpanPositionRangeBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SpanPositionRangeBuilder")
    }
}

impl SpanPositionRangeBuilder {
    /// Equivalent to `new SpanPositionRangeBuilder(SpanQueryBuilder factory)`.
    pub fn new(factory: Arc<dyn SpanQueryBuilder>) -> Self {
        Self { factory }
    }
}

impl QueryBuilder for SpanPositionRangeBuilder {
    fn get_query(&self, e: &Element) -> Result<Arc<dyn Query>> {
        SpanBuilderBase::get_query(self, e)
    }
}

impl SpanQueryBuilder for SpanPositionRangeBuilder {
    /// Equivalent to `getSpanQuery(Element e)`: a `SpanPositionRangeQuery`
    /// of the first child element, between the `start` and `end` attributes
    /// (1 by default).
    fn get_span_query(&self, e: &Element) -> Result<Arc<dyn SpanQuery>> {
        let start = DOMUtils::get_attribute_int(e, "start", 1)?;
        let end = DOMUtils::get_attribute_int(e, "end", 1)?;
        let child = DOMUtils::get_first_child_element(e).ok_or_else(LuceneError::null_pointer)?;
        let q = self.factory.get_span_query(&child)?;
        Ok(Arc::new(SpanPositionRangeQuery::new(q, start, end)))
    }
}
