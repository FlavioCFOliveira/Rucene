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

//! Port of `MatchAllDocsQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.nodes`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::Result;

use super::query_node::{QueryNode, QueryNodeRef};
use super::query_node_impl::{QueryNodeImpl, QUERY_NODE_IMPL_CLASS};
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::JavaClass;

/// `org.apache.lucene.queryparser.flexible.core.nodes.MatchAllDocsQueryNode`.
pub static MATCH_ALL_DOCS_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.core.nodes.MatchAllDocsQueryNode",
    Some(&QUERY_NODE_IMPL_CLASS),
    &[],
);

/// A [`MatchAllDocsQueryNode`] indicates that a query node tree or subtree
/// will match all documents if executed in the index.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.core.nodes.MatchAllDocsQueryNode`.
#[derive(Debug)]
pub struct MatchAllDocsQueryNode {
    base: QueryNodeImpl,
}

impl MatchAllDocsQueryNode {
    /// Equivalent to `new MatchAllDocsQueryNode()`.
    pub fn new() -> Arc<Self> {
        Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: QueryNodeImpl::new(this.clone()),
        })
    }
}

impl QueryNode for MatchAllDocsQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        &self.base
    }

    fn java_class(&self) -> &'static JavaClass {
        &MATCH_ALL_DOCS_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_java_string(&self) -> Result<String> {
        Ok("<matchAllDocs field='*' term='*'/>".to_string())
    }

    fn to_query_string(
        &self,
        _escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        Ok(CharSequence::from("*:*"))
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.base.clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: self.base.cloned_state(this.clone(), clauses),
        }))
    }
}
