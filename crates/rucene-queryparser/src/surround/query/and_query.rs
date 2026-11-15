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

//! The port of `AndQuery.java`, of the Java package
//! `org.apache.lucene.queryparser.surround.query`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::search::{Occur, Query};

use super::basic_query_factory::BasicQueryFactory;
use super::composed_query::{self, ComposedQuery, ComposedQueryState};
use super::srnd_boolean_query::SrndBooleanQuery;
use super::srnd_query::{SrndQuery, SrndQueryState};

/// Factory for conjunctions.
///
/// Equivalent to `org.apache.lucene.queryparser.surround.query.AndQuery`.
#[derive(Debug, Clone)]
pub struct AndQuery {
    composed: ComposedQueryState,
}

impl AndQuery {
    /// Equivalent to `new AndQuery(List<SrndQuery>, boolean, String)`.
    ///
    /// # Errors
    ///
    /// As [`ComposedQueryState::new`].
    pub fn new(queries: Vec<Arc<dyn SrndQuery>>, inf: bool, op_name: &str) -> Result<Self> {
        Ok(Self {
            composed: ComposedQueryState::new(queries, inf, op_name)?,
        })
    }
}

impl ComposedQuery for AndQuery {
    fn composed(&self) -> &ComposedQueryState {
        &self.composed
    }
}

impl SrndQuery for AndQuery {
    fn srnd_state(&self) -> &SrndQueryState {
        &self.composed.srnd
    }

    fn srnd_to_string(&self) -> String {
        composed_query::to_string(self)
    }

    fn is_fields_sub_query_acceptable(&self) -> bool {
        composed_query::is_fields_sub_query_acceptable(self)
    }

    fn clone_srnd(&self) -> Arc<dyn SrndQuery> {
        Arc::new(self.clone())
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queryparser.surround.query.AndQuery"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn make_lucene_query_field_no_boost(
        self: Arc<Self>,
        field_name: Option<&str>,
        qf: &Arc<BasicQueryFactory>,
    ) -> Result<Arc<dyn Query>> {
        SrndBooleanQuery::make_boolean_query(
            /* subqueries can be individually boosted */
            &self.make_lucene_sub_queries_field(field_name, qf)?,
            Occur::MUST,
        )
    }
}
