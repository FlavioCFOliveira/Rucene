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

//! Port of `GroupQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.nodes`.
use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::Result;

use super::query_node::{QueryNode, QueryNodeRef};
use super::query_node_impl::{first_child, QueryNodeImpl, QUERY_NODE_IMPL_CLASS};
use crate::flexible::core::messages::query_parser_messages::QueryParserMessages;
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::flexible::core::query_node_error::QueryNodeError;
use crate::flexible::messages::message_impl::MessageImpl;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::JavaClass;
use crate::jdk::object::JavaObject;

/// `org.apache.lucene.queryparser.flexible.core.nodes.GroupQueryNode`.
pub static GROUP_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.core.nodes.GroupQueryNode",
    Some(&QUERY_NODE_IMPL_CLASS),
    &[],
);

/// A [`GroupQueryNode`] represents a location where the original user typed
/// real parenthesis on the query string. This class is useful for
/// queryparser explain functionality.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.core.nodes.GroupQueryNode`.
#[derive(Debug)]
pub struct GroupQueryNode {
    base: QueryNodeImpl,
}

impl GroupQueryNode {
    /// This QueryNode is used to identify parenthesis on the original query
    /// string
    ///
    /// Equivalent to `new GroupQueryNode(QueryNode query)`.
    ///
    /// # Errors
    ///
    /// The `QueryNodeError` `PARAMETER_VALUE_NOT_SUPPORTED` for a `None`
    /// query.
    pub fn new(query: Option<QueryNodeRef>) -> Result<Arc<Self>> {
        let Some(query) = query else {
            return Err(QueryNodeError::new(Arc::new(MessageImpl::with_args(
                QueryParserMessages::PARAMETER_VALUE_NOT_SUPPORTED.get(),
                vec![JavaObject::from("query"), JavaObject::from("null")],
            )))
            .error());
        };
        let node = Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: QueryNodeImpl::new(this.clone()),
        });
        node.base.allocate();
        node.base.set_leaf(false);
        node.base.add(query)?;
        Ok(node)
    }

    /// Equivalent to `getChild()`: `getChildren().get(0)`.
    ///
    /// # Errors
    ///
    /// The exceptions of a missing child (see [`first_child`]).
    pub fn get_child(&self) -> Result<QueryNodeRef> {
        first_child(self)
    }

    /// Equivalent to `setChild(QueryNode child)`.
    ///
    /// # Errors
    ///
    /// The exceptions of `set`.
    pub fn set_child(&self, child: QueryNodeRef) -> Result<()> {
        self.set(vec![child])
    }
}

impl QueryNode for GroupQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        &self.base
    }

    fn java_class(&self) -> &'static JavaClass {
        &GROUP_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_java_string(&self) -> Result<String> {
        Ok(format!(
            "<group>\n{}\n</group>",
            self.get_child()?.to_java_string()?
        ))
    }

    fn to_query_string(
        &self,
        escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        let child = self.get_child()?;
        Ok(CharSequence::from(format!(
            "( {} )",
            child.to_query_string(escape_syntax_parser)?
        )))
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.base.clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: self.base.cloned_state(this.clone(), clauses),
        }))
    }
}
