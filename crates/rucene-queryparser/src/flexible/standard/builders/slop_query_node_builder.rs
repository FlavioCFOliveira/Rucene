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

//! Port of `SlopQueryNodeBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.builders`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::search::multi_phrase_query::{self, MultiPhraseQuery};
use rucene::search::phrase_query::{self, PhraseQuery};
use rucene::search::Query;

use super::standard_query_builder::{built_query, cast, query_class_cast, StandardQueryBuilder};
use crate::flexible::core::nodes::query_node::QueryNodeRef;
use crate::flexible::core::nodes::slop_query_node::{SlopQueryNode, SLOP_QUERY_NODE_CLASS};

/// This builder basically reads the [`Query`] object set on the
/// `SlopQueryNode` child using `QueryTreeBuilder.QUERY_TREE_BUILDER_TAGID`
/// and applies the slop value defined in the `SlopQueryNode`.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.builders.SlopQueryNodeBuilder`.
#[derive(Debug, Default)]
pub struct SlopQueryNodeBuilder;

impl SlopQueryNodeBuilder {
    /// Equivalent to `new SlopQueryNodeBuilder()`.
    pub fn new() -> Self {
        // empty constructor
        Self
    }
}

impl StandardQueryBuilder for SlopQueryNodeBuilder {
    fn build(&self, query_node: &QueryNodeRef) -> Result<Option<Arc<dyn Query>>> {
        let phrase_slop_node: &SlopQueryNode = cast(query_node, &SLOP_QUERY_NODE_CLASS)?;
        let mut query = built_query(phrase_slop_node.get_child()?.as_ref())?;
        let pq = query
            .as_ref()
            .and_then(|q| q.as_any().downcast_ref::<PhraseQuery>());
        if let Some(pq) = pq {
            let mut builder = phrase_query::Builder::new();
            builder.set_slop(phrase_slop_node.get_value());
            let terms = pq.get_terms();
            let positions = pq.get_positions();
            for i in 0..terms.len() {
                builder.add_at(terms[i].clone(), positions[i])?;
            }
            query = Some(Arc::new(builder.build()?));
        } else {
            // `(MultiPhraseQuery) query`, then `mpq.getSlop()`
            let q = query.as_ref().ok_or_else(LuceneError::null_pointer)?;
            let mpq = q
                .as_any()
                .downcast_ref::<MultiPhraseQuery>()
                .ok_or_else(|| {
                    query_class_cast(q.as_ref(), "org.apache.lucene.search.MultiPhraseQuery")
                })?;
            let slop = phrase_slop_node.get_value();
            if slop != mpq.get_slop() {
                let mut builder = multi_phrase_query::Builder::from_query(mpq);
                builder.set_slop(slop)?;
                query = Some(Arc::new(builder.build()));
            }
        }
        Ok(query)
    }
}
