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

//! Port of `ConstantScoreQueryBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.xml.builders`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::boost_query::BoostQuery;
use rucene::search::constant_score_query::ConstantScoreQuery;
use rucene::search::query::Query;

use crate::jdk::dom::Element;
use crate::xml::dom_utils::DOMUtils;
use crate::xml::query_builder::QueryBuilder;
use crate::xml::query_builder_factory::QueryBuilderFactory;

/// Builder for `ConstantScoreQuery`
///
/// Equivalent to
/// `org.apache.lucene.queryparser.xml.builders.ConstantScoreQueryBuilder`.
#[derive(Debug)]
pub struct ConstantScoreQueryBuilder {
    query_factory: Arc<QueryBuilderFactory>,
}

impl ConstantScoreQueryBuilder {
    /// Equivalent to `new ConstantScoreQueryBuilder(QueryBuilderFactory
    /// queryFactory)`.
    pub fn new(query_factory: Arc<QueryBuilderFactory>) -> Self {
        Self { query_factory }
    }
}

impl QueryBuilder for ConstantScoreQueryBuilder {
    fn get_query(&self, e: &Element) -> Result<Arc<dyn Query>> {
        let query_elem = DOMUtils::get_first_child_or_fail(e)?;
        let mut q: Arc<dyn Query> = Arc::new(ConstantScoreQuery::new(
            self.query_factory.get_query(&query_elem)?,
        ));
        let boost = DOMUtils::get_attribute_float(e, "boost", 1.0)?;
        if boost != 1.0 {
            q = Arc::new(BoostQuery::new(q, boost)?);
        }
        Ok(q)
    }
}
