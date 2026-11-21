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

//! Port of `MultiFieldQueryNodeProcessor.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.processors`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::PoisonError;

use rucene::error::{LuceneError, Result};

use crate::flexible::core::nodes::fieldable_node::FIELDABLE_NODE_CLASS;
use crate::flexible::core::nodes::group_query_node::GroupQueryNode;
use crate::flexible::core::nodes::or_query_node::OrQueryNode;
use crate::flexible::core::nodes::query_node::{class_cast, QueryNodeRef};
use crate::flexible::core::processors::query_node_processor_impl::{
    QueryNodeProcessorImpl, QueryNodeProcessorImplOps,
};
use crate::flexible::standard::config::standard_query_config_handler::ConfigurationKeys;

/// This processor is used to expand terms so the query looks for the same
/// term in different fields. It also boosts a query based on its field.
///
/// This processor looks for every `FieldableNode` contained in the query
/// node tree. If a `FieldableNode` is found, it checks if there is a
/// `ConfigurationKeys.MULTI_FIELDS` defined in the `QueryConfigHandler`. If
/// there is, the `FieldableNode` is cloned N times and the clones are added
/// to a `BooleanQueryNode` together with the original node. N is defined by
/// the number of fields that it will be expanded to. The `BooleanQueryNode`
/// is returned.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.processors.MultiFieldQueryNodeProcessor`.
#[derive(Debug)]
pub struct MultiFieldQueryNodeProcessor {
    base: QueryNodeProcessorImpl,
    process_children: AtomicBool,
}

impl Default for MultiFieldQueryNodeProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiFieldQueryNodeProcessor {
    /// Equivalent to `new MultiFieldQueryNodeProcessor()`.
    pub fn new() -> Self {
        // empty constructor
        Self {
            base: QueryNodeProcessorImpl::new(),
            process_children: AtomicBool::new(true),
        }
    }
}

impl QueryNodeProcessorImplOps for MultiFieldQueryNodeProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        Ok(node)
    }

    fn process_children(&self, query_tree: &QueryNodeRef) -> Result<()> {
        if self.process_children.load(Ordering::SeqCst) {
            QueryNodeProcessorImpl::process_children(self, query_tree)
        } else {
            self.process_children.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    fn pre_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        if let Some(field_node) = node.as_fieldable_node() {
            self.process_children.store(false, Ordering::SeqCst);
            if field_node.get_field()?.is_none() {
                let fields = self
                    .get_query_config_handler()
                    .ok_or_else(LuceneError::null_pointer)?
                    .get(ConfigurationKeys::MULTI_FIELDS);
                let Some(fields) = fields else {
                    return Err(LuceneError::illegal_argument(
                        "StandardQueryConfigHandler.ConfigurationKeys.MULTI_FIELDS should be set on the QueryConfigHandler",
                    ));
                };
                let fields = fields
                    .read()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone();
                if !fields.is_empty() {
                    field_node.set_field(fields[0].clone())?;
                    if fields.len() == 1 {
                        return Ok(node);
                    } else {
                        let mut children: Vec<QueryNodeRef> = Vec::with_capacity(fields.len());
                        children.push(node.clone());
                        let mut field_node_ref = node.clone();
                        for field in fields.iter().skip(1) {
                            field_node_ref = field_node_ref.clone_tree()?;
                            field_node_ref
                                .as_fieldable_node()
                                .ok_or_else(|| {
                                    class_cast(field_node_ref.as_ref(), &FIELDABLE_NODE_CLASS)
                                })?
                                .set_field(field.clone())?;
                            children.push(field_node_ref.clone());
                        }
                        let or: QueryNodeRef = OrQueryNode::new(Some(children))?;
                        let group: QueryNodeRef = GroupQueryNode::new(Some(or))?;
                        return Ok(group);
                    }
                }
            }
        }
        Ok(node)
    }

    fn set_children_order(&self, children: Vec<QueryNodeRef>) -> Result<Vec<QueryNodeRef>> {
        Ok(children)
    }
}
