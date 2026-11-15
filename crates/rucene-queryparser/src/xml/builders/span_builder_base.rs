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

//! Port of `SpanBuilderBase.java`, of the Java package
//! `org.apache.lucene.queryparser.xml.builders`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::boost_query::BoostQuery;
use rucene::search::query::Query;

use super::span_query_builder::SpanQueryBuilder;
use crate::jdk::dom::Element;
use crate::xml::dom_utils::DOMUtils;

/// Base class for building [`SpanQuery`](rucene_queries::spans::span_query::SpanQuery)s
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queryparser.xml.builders.SpanBuilderBase`: its one
/// member, `getQuery`, is [`get_query`](Self::get_query), which the span
/// builders' `QueryBuilder` implementations call.
#[derive(Debug)]
pub struct SpanBuilderBase {
    _no_instances: (),
}

impl SpanBuilderBase {
    /// Equivalent to `getQuery(Element e)`: the span query of the builder,
    /// boosted by the `boost` attribute when it is not 1.
    ///
    /// # Errors
    ///
    /// The `NumberFormatException` of the boost, and what the builder
    /// throws.
    pub fn get_query(
        this: &(impl SpanQueryBuilder + ?Sized),
        e: &Element,
    ) -> Result<Arc<dyn Query>> {
        let boost = DOMUtils::get_attribute_float(e, "boost", 1.0)?;
        let q = this.get_span_query(e)?;
        let q: Arc<dyn Query> = q;
        if boost == 1.0 {
            return Ok(q);
        }
        Ok(Arc::new(BoostQuery::new(q, boost)?))
    }
}
