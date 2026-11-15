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

//! Port of `SpanOrBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.xml.builders`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::query::Query;
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_query::SpanQuery;

use crate::jdk::dom::ELEMENT_NODE;

use super::span_builder_base::SpanBuilderBase;
use super::span_query_builder::SpanQueryBuilder;
use crate::jdk::dom::Element;
use crate::xml::query_builder::QueryBuilder;

/// Builder for `SpanOrQuery`
///
/// Equivalent to `org.apache.lucene.queryparser.xml.builders.SpanOrBuilder`, which extends `SpanBuilderBase`.
pub struct SpanOrBuilder {
    factory: Arc<dyn SpanQueryBuilder>,
}

impl std::fmt::Debug for SpanOrBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SpanOrBuilder")
    }
}

impl SpanOrBuilder {
    /// Equivalent to `new SpanOrBuilder(SpanQueryBuilder factory)`.
    pub fn new(factory: Arc<dyn SpanQueryBuilder>) -> Self {
        Self { factory }
    }
}

impl QueryBuilder for SpanOrBuilder {
    fn get_query(&self, e: &Element) -> Result<Arc<dyn Query>> {
        SpanBuilderBase::get_query(self, e)
    }
}

impl SpanQueryBuilder for SpanOrBuilder {
    /// Equivalent to `getSpanQuery(Element e)`: a `SpanOrQuery` of the child
    /// elements.
    fn get_span_query(&self, e: &Element) -> Result<Arc<dyn SpanQuery>> {
        let mut clauses_list: Vec<Arc<dyn SpanQuery>> = Vec::new();
        let mut kid = e.get_first_child();
        while let Some(k) = kid {
            if k.get_node_type() == ELEMENT_NODE {
                let clause = self.factory.get_span_query(&Element::cast(&k)?)?;
                clauses_list.push(clause);
            }
            kid = k.get_next_sibling();
        }
        Ok(Arc::new(SpanOrQuery::new(clauses_list)?))
    }
}
