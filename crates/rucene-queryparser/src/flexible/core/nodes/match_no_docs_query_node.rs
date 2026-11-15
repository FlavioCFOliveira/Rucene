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

//! Port of `MatchNoDocsQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.nodes`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::Result;

use super::deleted_query_node::{DeletedQueryNode, DELETED_QUERY_NODE_CLASS};
use super::query_node::{QueryNode, QueryNodeRef};
use super::query_node_impl::QueryNodeImpl;
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::JavaClass;

/// `org.apache.lucene.queryparser.flexible.core.nodes.MatchNoDocsQueryNode`.
pub static MATCH_NO_DOCS_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.core.nodes.MatchNoDocsQueryNode",
    Some(&DELETED_QUERY_NODE_CLASS),
    &[],
);

/// A [`MatchNoDocsQueryNode`] indicates that a query node tree or subtree
/// will not match any documents if executed in the index.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.core.nodes.MatchNoDocsQueryNode`,
/// which extends `DeletedQueryNode` (embedded).
#[derive(Debug)]
pub struct MatchNoDocsQueryNode {
    base: DeletedQueryNode,
}

impl MatchNoDocsQueryNode {
    /// Equivalent to `new MatchNoDocsQueryNode()`.
    pub fn new() -> Arc<Self> {
        Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: DeletedQueryNode::fields(QueryNodeImpl::new(this.clone())),
        })
    }
}

impl QueryNode for MatchNoDocsQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        self.base.query_node_impl()
    }

    fn java_class(&self) -> &'static JavaClass {
        &MATCH_NO_DOCS_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_query_string(&self, escaper: &dyn EscapeQuerySyntax) -> Result<CharSequence> {
        self.base.to_query_string(escaper)
    }

    fn to_java_string(&self) -> Result<String> {
        Ok("<matchNoDocsQueryNode/>".to_string())
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.query_node_impl().clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: DeletedQueryNode::fields(
                self.query_node_impl().cloned_state(this.clone(), clauses),
            ),
        }))
    }
}
