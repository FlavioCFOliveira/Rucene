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

//! Port of `QueryBuilderFactory.java`, of the Java package
//! `org.apache.lucene.queryparser.xml`.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};

use rucene::error::Result;
use rucene::search::query::Query;

use super::parser_exception::ParserException;
use super::query_builder::QueryBuilder;
use crate::jdk::dom::Element;

/// Factory for [`QueryBuilder`]
///
/// Equivalent to `org.apache.lucene.queryparser.xml.QueryBuilderFactory`.
/// It is shared as an [`Arc`]: the builders that delegate to it hold it,
/// and see the builders added to it afterwards.
#[derive(Default)]
pub struct QueryBuilderFactory {
    /// The package-private `builders`.
    pub(crate) builders: RwLock<HashMap<String, Arc<dyn QueryBuilder>>>,
}

impl std::fmt::Debug for QueryBuilderFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("QueryBuilderFactory")
    }
}

impl QueryBuilderFactory {
    /// Equivalent to `new QueryBuilderFactory()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Equivalent to `addBuilder(String nodeName, QueryBuilder builder)`.
    pub fn add_builder(&self, node_name: &str, builder: Arc<dyn QueryBuilder>) {
        self.builders
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(node_name.to_string(), builder);
    }

    /// Equivalent to `getQueryBuilder(String nodeName)`.
    pub fn get_query_builder(&self, node_name: &str) -> Option<Arc<dyn QueryBuilder>> {
        self.builders
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(node_name)
            .cloned()
    }
}

impl QueryBuilder for QueryBuilderFactory {
    /// Equivalent to `getQuery(Element n)`: the builder registered for the
    /// element's name builds the query.
    ///
    /// # Errors
    ///
    /// `ParserException("No QueryObjectBuilder defined for node " + name)`
    /// without one, and what the builder throws.
    fn get_query(&self, n: &Element) -> Result<Arc<dyn Query>> {
        let name = n.get_node_name();
        let builder = self.get_query_builder(&name).ok_or_else(|| {
            ParserException::with_message(format!("No QueryObjectBuilder defined for node {name}"))
                .error()
        })?;
        builder.get_query(n)
    }
}
