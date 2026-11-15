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

//! Port of `SynonymQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.nodes`.
use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::Result;

use crate::flexible::core::nodes::boolean_query_node::{
    BooleanQueryNode, BOOLEAN_QUERY_NODE_CLASS,
};
use crate::flexible::core::nodes::query_node::{QueryNode, QueryNodeRef};
use crate::flexible::core::nodes::query_node_impl::QueryNodeImpl;
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::JavaClass;

/// `org.apache.lucene.queryparser.flexible.standard.nodes.SynonymQueryNode`.
pub static SYNONYM_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.standard.nodes.SynonymQueryNode",
    Some(&BOOLEAN_QUERY_NODE_CLASS),
    &[],
);

/// QueryNode for clauses that are synonym of each other.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.standard.nodes.SynonymQueryNode`,
/// which extends `BooleanQueryNode` (embedded).
#[derive(Debug)]
pub struct SynonymQueryNode {
    base: BooleanQueryNode,
}

impl SynonymQueryNode {
    /// Equivalent to `new SynonymQueryNode(List<QueryNode> clauses)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for `None` clauses.
    pub fn new(clauses: Option<Vec<QueryNodeRef>>) -> Result<Arc<Self>> {
        let node = Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: BooleanQueryNode::fields(QueryNodeImpl::new(this.clone())),
        });
        node.base.init(clauses)?;
        Ok(node)
    }
}

impl QueryNode for SynonymQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        self.base.query_node_impl()
    }

    fn java_class(&self) -> &'static JavaClass {
        &SYNONYM_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_java_string(&self) -> Result<String> {
        self.base.to_java_string_with("default")
    }

    fn to_query_string(
        &self,
        escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        self.base.to_query_string_with(escape_syntax_parser, " ")
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.query_node_impl().clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: self.base.cloned(this.clone(), clauses),
        }))
    }
}
