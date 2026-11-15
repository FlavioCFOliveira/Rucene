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

//! Port of `MultiTermRewriteMethodProcessor.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.processors`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::search::multi_term_query::RewriteMethod;

use crate::flexible::core::nodes::query_node::{instance_of, JavaObjectRef, QueryNodeRef};
use crate::flexible::core::processors::query_node_processor_impl::{
    QueryNodeProcessorImpl, QueryNodeProcessorImplOps,
};
use crate::flexible::standard::config::standard_query_config_handler::ConfigurationKeys;
use crate::flexible::standard::nodes::abstract_range_query_node::ABSTRACT_RANGE_QUERY_NODE_CLASS;
use crate::flexible::standard::nodes::regexp_query_node::REGEXP_QUERY_NODE_CLASS;
use crate::flexible::standard::nodes::wildcard_query_node::WILDCARD_QUERY_NODE_CLASS;

/// This processor instates the default `MultiTermQuery.RewriteMethod`,
/// `MultiTermQuery.CONSTANT_SCORE_BLENDED_REWRITE`, for multi-term query
/// nodes.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.processors.MultiTermRewriteMethodProcessor`.
#[derive(Debug, Default)]
pub struct MultiTermRewriteMethodProcessor {
    base: QueryNodeProcessorImpl,
}

impl MultiTermRewriteMethodProcessor {
    /// Equivalent to `MultiTermRewriteMethodProcessor.TAG_ID`.
    pub const TAG_ID: &'static str = "MultiTermRewriteMethodConfiguration";

    /// Equivalent to the implicit `new MultiTermRewriteMethodProcessor()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// A rewrite method as the `Object` the [`TAG_ID`](Self::TAG_ID) tag
    /// holds: the `Arc<dyn RewriteMethod>` itself.
    pub fn rewrite_method_object(rewrite_method: Arc<dyn RewriteMethod>) -> JavaObjectRef {
        Arc::new(rewrite_method)
    }

    /// The cast `(MultiTermQuery.RewriteMethod) obj` of the
    /// [`TAG_ID`](Self::TAG_ID) tag the builders read.
    ///
    /// **Divergence.** Java's `ClassCastException` names the class of the
    /// object; an object held as `Any` keeps no Java class, so the message
    /// names the target class only. Only code outside Lucene tags a node with
    /// another object.
    ///
    /// # Errors
    ///
    /// The `ClassCastException` of an object that is not a rewrite method.
    pub fn rewrite_method_of(obj: &JavaObjectRef) -> Result<Arc<dyn RewriteMethod>> {
        obj.downcast_ref::<Arc<dyn RewriteMethod>>()
            .cloned()
            .ok_or_else(|| {
                LuceneError::unchecked(
                    "java.lang.ClassCastException",
                    "cannot be cast to class org.apache.lucene.search.MultiTermQuery$RewriteMethod",
                )
            })
    }
}

impl QueryNodeProcessorImplOps for MultiTermRewriteMethodProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        // set setMultiTermRewriteMethod for WildcardQueryNode and
        // PrefixWildcardQueryNode
        if instance_of(node.as_ref(), &WILDCARD_QUERY_NODE_CLASS)
            || instance_of(node.as_ref(), &ABSTRACT_RANGE_QUERY_NODE_CLASS)
            || instance_of(node.as_ref(), &REGEXP_QUERY_NODE_CLASS)
        {
            let rewrite_method = self
                .get_query_config_handler()
                .ok_or_else(LuceneError::null_pointer)?
                .get(ConfigurationKeys::MULTI_TERM_REWRITE_METHOD);
            let Some(rewrite_method) = rewrite_method else {
                // This should not happen, this configuration is set in the
                // StandardQueryConfigHandler
                return Err(LuceneError::illegal_argument(
                    "StandardQueryConfigHandler.ConfigurationKeys.MULTI_TERM_REWRITE_METHOD should be set on the QueryConfigHandler",
                ));
            };
            // use a TAG to take the value to the Builder
            node.set_tag(
                MultiTermRewriteMethodProcessor::TAG_ID,
                Self::rewrite_method_object(rewrite_method),
            );
        }
        Ok(node)
    }

    fn pre_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        Ok(node)
    }

    fn set_children_order(&self, children: Vec<QueryNodeRef>) -> Result<Vec<QueryNodeRef>> {
        Ok(children)
    }
}
