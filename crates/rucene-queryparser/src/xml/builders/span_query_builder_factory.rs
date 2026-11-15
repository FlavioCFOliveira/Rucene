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

//! Port of `SpanQueryBuilderFactory.java`, of the Java package
//! `org.apache.lucene.queryparser.xml.builders`.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};

use rucene::error::Result;
use rucene::search::query::Query;
use rucene_queries::spans::span_query::SpanQuery;

use super::span_query_builder::SpanQueryBuilder;
use crate::jdk::dom::Element;
use crate::xml::parser_exception::ParserException;
use crate::xml::query_builder::QueryBuilder;

/// Factory for [`SpanQueryBuilder`]s
///
/// Equivalent to
/// `org.apache.lucene.queryparser.xml.builders.SpanQueryBuilderFactory`.
/// It is shared as an [`Arc`]: the builders that delegate to it hold it.
#[derive(Default)]
pub struct SpanQueryBuilderFactory {
    builders: RwLock<HashMap<String, Arc<dyn SpanQueryBuilder>>>,
}

impl std::fmt::Debug for SpanQueryBuilderFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SpanQueryBuilderFactory")
    }
}

impl SpanQueryBuilderFactory {
    /// Equivalent to `new SpanQueryBuilderFactory()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Equivalent to `addBuilder(String nodeName, SpanQueryBuilder builder)`.
    pub fn add_builder(&self, node_name: &str, builder: Arc<dyn SpanQueryBuilder>) {
        self.builders
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(node_name.to_string(), builder);
    }
}

impl QueryBuilder for SpanQueryBuilderFactory {
    fn get_query(&self, e: &Element) -> Result<Arc<dyn Query>> {
        let q: Arc<dyn Query> = self.get_span_query(e)?;
        Ok(q)
    }
}

impl SpanQueryBuilder for SpanQueryBuilderFactory {
    /// Equivalent to `getSpanQuery(Element e)`: the builder registered for
    /// the element's name builds the span query.
    ///
    /// # Errors
    ///
    /// `ParserException("No SpanQueryObjectBuilder defined for node " +
    /// name)` without one, and what the builder throws.
    fn get_span_query(&self, e: &Element) -> Result<Arc<dyn SpanQuery>> {
        let name = e.get_node_name();
        let builder = self
            .builders
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&name)
            .cloned()
            .ok_or_else(|| {
                ParserException::with_message(format!(
                    "No SpanQueryObjectBuilder defined for node {name}"
                ))
                .error()
            })?;
        builder.get_span_query(e)
    }
}
