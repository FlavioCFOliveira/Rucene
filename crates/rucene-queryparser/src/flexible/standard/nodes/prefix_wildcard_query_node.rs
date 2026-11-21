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

//! Port of `PrefixWildcardQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.nodes`.
use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::Result;

use super::wildcard_query_node::{WildcardQueryNode, WILDCARD_QUERY_NODE_CLASS};
use crate::flexible::core::nodes::field_query_node::FieldQueryNode;
use crate::flexible::core::nodes::fieldable_node::FieldableNode;
use crate::flexible::core::nodes::query_node::{QueryNode, QueryNodeRef};
use crate::flexible::core::nodes::query_node_impl::QueryNodeImpl;
use crate::flexible::core::nodes::textable_query_node::TextableQueryNode;
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::jdk::char_sequence::{value_of, CharSequence};
use crate::jdk::class::JavaClass;

/// `org.apache.lucene.queryparser.flexible.standard.nodes.PrefixWildcardQueryNode`.
pub static PREFIX_WILDCARD_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.standard.nodes.PrefixWildcardQueryNode",
    Some(&WILDCARD_QUERY_NODE_CLASS),
    &[],
);

/// A [`PrefixWildcardQueryNode`] represents wildcardquery that matches
/// abc* or *. This does not apply to phrases, this is a special case on the
/// original lucene parser. TODO: refactor the code to remove this special
/// case from the parser. and probably do it on a Processor
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.standard.nodes.PrefixWildcardQueryNode`,
/// which extends `WildcardQueryNode` (embedded).
#[derive(Debug)]
pub struct PrefixWildcardQueryNode {
    base: WildcardQueryNode,
}

impl PrefixWildcardQueryNode {
    /// Equivalent to `new PrefixWildcardQueryNode(CharSequence field,
    /// CharSequence text, int begin, int end)`.
    ///
    /// * `field` - field name
    /// * `text` - value including the wildcard
    /// * `begin` - position in the query string
    /// * `end` - position in the query string
    pub fn new(
        field: Option<CharSequence>,
        text: Option<CharSequence>,
        begin: i32,
        end: i32,
    ) -> Arc<Self> {
        let node = Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: WildcardQueryNode::fields(
                QueryNodeImpl::new(this.clone()),
                field,
                text,
                begin,
                end,
            ),
        });
        node.base.init();
        node
    }

    /// Equivalent to `new PrefixWildcardQueryNode(FieldQueryNode fqn)`: the
    /// field, text, begin and end of `fqn`.
    pub fn from_field_query_node(fqn: &FieldQueryNode) -> Arc<Self> {
        Self::new(
            fqn.get_field(),
            fqn.get_text(),
            fqn.get_begin(),
            fqn.get_end(),
        )
    }
}

impl QueryNode for PrefixWildcardQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        self.base.query_node_impl()
    }

    fn java_class(&self) -> &'static JavaClass {
        &PREFIX_WILDCARD_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_query_string(&self, _escaper: &dyn EscapeQuerySyntax) -> Result<CharSequence> {
        Ok(self.base.wildcard_to_query_string())
    }

    fn to_java_string(&self) -> Result<String> {
        let fqn = self.base.field_query_node();
        Ok(format!(
            "<prefixWildcard field='{}' term='{}'/>",
            value_of(fqn.get_field().as_ref()),
            value_of(fqn.get_text().as_ref())
        ))
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.query_node_impl().clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: self
                .base
                .cloned(self.query_node_impl().cloned_state(this.clone(), clauses)),
        }))
    }

    fn as_fieldable_node(&self) -> Option<&dyn FieldableNode> {
        Some(self.base.field_query_node())
    }

    fn as_textable_query_node(&self) -> Option<&dyn TextableQueryNode> {
        Some(self.base.field_query_node())
    }

    fn as_field_query_node(&self) -> Option<&FieldQueryNode> {
        Some(self.base.field_query_node())
    }
}
