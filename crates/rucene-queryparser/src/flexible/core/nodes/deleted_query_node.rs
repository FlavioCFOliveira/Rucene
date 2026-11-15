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

//! Port of `DeletedQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.nodes`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::Result;

use super::query_node::{QueryNode, QueryNodeRef};
use super::query_node_impl::{QueryNodeImpl, QUERY_NODE_IMPL_CLASS};
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::JavaClass;

/// `org.apache.lucene.queryparser.flexible.core.nodes.DeletedQueryNode`.
pub static DELETED_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.core.nodes.DeletedQueryNode",
    Some(&QUERY_NODE_IMPL_CLASS),
    &[],
);

/// A [`DeletedQueryNode`] represents a node that was deleted from the query
/// node tree. It can be removed from the tree using the
/// `RemoveDeletedQueryNodesProcessor` processor.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.core.nodes.DeletedQueryNode`.
#[derive(Debug)]
pub struct DeletedQueryNode {
    base: QueryNodeImpl,
}

impl DeletedQueryNode {
    pub(crate) fn fields(base: QueryNodeImpl) -> Self {
        Self { base }
    }

    /// Equivalent to `new DeletedQueryNode()`.
    pub fn new() -> Arc<Self> {
        Arc::new_cyclic(|this: &Weak<Self>| Self::fields(QueryNodeImpl::new(this.clone())))
    }
}

impl QueryNode for DeletedQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        &self.base
    }

    fn java_class(&self) -> &'static JavaClass {
        &DELETED_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_query_string(&self, _escaper: &dyn EscapeQuerySyntax) -> Result<CharSequence> {
        Ok(CharSequence::from("[DELETEDCHILD]"))
    }

    fn to_java_string(&self) -> Result<String> {
        Ok("<deleted/>".to_string())
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.base.clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| {
            Self::fields(self.base.cloned_state(this.clone(), clauses))
        }))
    }
}
