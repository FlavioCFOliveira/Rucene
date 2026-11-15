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

//! Port of `MatchAllDocsQueryBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.xml.builders`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::query::Query;

use crate::jdk::dom::Element;
use crate::xml::query_builder::QueryBuilder;

/// Builder for `MatchAllDocsQuery`
///
/// Equivalent to
/// `org.apache.lucene.queryparser.xml.builders.MatchAllDocsQueryBuilder`.
#[derive(Debug, Default)]
pub struct MatchAllDocsQueryBuilder;

impl MatchAllDocsQueryBuilder {
    /// Equivalent to `new MatchAllDocsQueryBuilder()`.
    pub fn new() -> Self {
        Self
    }
}

impl QueryBuilder for MatchAllDocsQueryBuilder {
    /// Equivalent to `getQuery(Element e)`: `MatchAllDocsQuery.INSTANCE`.
    fn get_query(&self, _e: &Element) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(MatchAllDocsQuery::instance()))
    }
}
