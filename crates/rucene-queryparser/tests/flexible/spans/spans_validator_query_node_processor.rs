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

//! Port of `SpansValidatorQueryNodeProcessor.java`.

use std::sync::Arc;

use rucene::error::Result;
use rucene_queryparser::flexible::core::messages::query_parser_messages::QueryParserMessages;
use rucene_queryparser::flexible::core::nodes::and_query_node::AND_QUERY_NODE_CLASS;
use rucene_queryparser::flexible::core::nodes::boolean_query_node::BOOLEAN_QUERY_NODE_CLASS;
use rucene_queryparser::flexible::core::nodes::field_query_node::FIELD_QUERY_NODE_CLASS;
use rucene_queryparser::flexible::core::nodes::query_node::{instance_of, QueryNodeRef};
use rucene_queryparser::flexible::core::processors::query_node_processor_impl::{
    QueryNodeProcessorImpl, QueryNodeProcessorImplOps,
};
use rucene_queryparser::flexible::core::query_node_exception::QueryNodeException;
use rucene_queryparser::flexible::messages::message_impl::MessageImpl;

/// Validates every query node in a query node tree. This processor will pass
/// fine if the query nodes are only `BooleanQueryNode`s, `OrQueryNode`s or
/// `FieldQueryNode`s, otherwise an exception will be thrown.
///
/// If they are `AndQueryNode` or an instance of anything else that
/// implements `FieldQueryNode` the exception will also be thrown.
#[derive(Debug, Default)]
pub struct SpansValidatorQueryNodeProcessor {
    base: QueryNodeProcessorImpl,
}

impl SpansValidatorQueryNodeProcessor {
    /// `new SpansValidatorQueryNodeProcessor()`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl QueryNodeProcessorImplOps for SpansValidatorQueryNodeProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        Ok(node)
    }

    fn pre_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        if !((instance_of(node.as_ref(), &BOOLEAN_QUERY_NODE_CLASS)
            && !instance_of(node.as_ref(), &AND_QUERY_NODE_CLASS))
            || std::ptr::eq(node.java_class(), &FIELD_QUERY_NODE_CLASS))
        {
            return Err(QueryNodeException::new(Arc::new(MessageImpl::new(
                QueryParserMessages::NODE_ACTION_NOT_SUPPORTED.get(),
            )))
            .error());
        }
        Ok(node)
    }

    fn set_children_order(&self, children: Vec<QueryNodeRef>) -> Result<Vec<QueryNodeRef>> {
        Ok(children)
    }
}
