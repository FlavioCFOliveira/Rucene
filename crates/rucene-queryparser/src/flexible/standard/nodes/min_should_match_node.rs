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

//! Port of `MinShouldMatchNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.nodes`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::Result;

use crate::flexible::core::nodes::group_query_node::GroupQueryNode;
use crate::flexible::core::nodes::query_node::{QueryNode, QueryNodeRef};
use crate::flexible::core::nodes::query_node_impl::{QueryNodeImpl, QUERY_NODE_IMPL_CLASS};
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::JavaClass;

/// `org.apache.lucene.queryparser.flexible.standard.nodes.MinShouldMatchNode`.
pub static MIN_SHOULD_MATCH_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.standard.nodes.MinShouldMatchNode",
    Some(&QUERY_NODE_IMPL_CLASS),
    &[],
);

/// Node that represents a minimum-should-match restriction on a
/// [`GroupQueryNode`].
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.standard.nodes.MinShouldMatchNode`,
/// whose `public final` fields are [`min_should_match`](Self::min_should_match)
/// and [`group_query_node`](Self::group_query_node). It does not override
/// `toString()` (`Object.toString()`) or `cloneTree()` (a clone keeps the
/// original group in its field and a clone of it as its child).
#[derive(Debug)]
pub struct MinShouldMatchNode {
    base: QueryNodeImpl,
    /// `public final int minShouldMatch`.
    pub min_should_match: i32,
    /// `public final GroupQueryNode groupQueryNode`.
    pub group_query_node: Option<Arc<GroupQueryNode>>,
}

impl MinShouldMatchNode {
    /// Equivalent to `new MinShouldMatchNode(int minShouldMatch,
    /// GroupQueryNode groupQueryNode)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of adding a `None` group.
    pub fn new(
        min_should_match: i32,
        group_query_node: Option<Arc<GroupQueryNode>>,
    ) -> Result<Arc<Self>> {
        let node = Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: QueryNodeImpl::new(this.clone()),
            min_should_match,
            group_query_node: group_query_node.clone(),
        });
        node.base.set_leaf(false);
        node.base.allocate();
        match group_query_node {
            Some(group) => node.base.add(group)?,
            None => {
                // `add(null)`: the `IllegalArgumentException` of a null child.
                let message = crate::flexible::messages::nls::NLS::get_localized_message(
                    crate::flexible::core::messages::query_parser_messages::QueryParserMessages::NODE_ACTION_NOT_SUPPORTED
                        .get()
                        .as_deref(),
                )?;
                return Err(rucene::error::LuceneError::illegal_argument(message));
            }
        }
        Ok(node)
    }
}

impl QueryNode for MinShouldMatchNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        &self.base
    }

    fn java_class(&self) -> &'static JavaClass {
        &MIN_SHOULD_MATCH_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// Equivalent to `toQueryString(EscapeQuerySyntax)`: the group's query
    /// string, `@` and the minimum.
    ///
    /// # Errors
    ///
    /// `NullPointerException` without a group, and the exceptions of the
    /// group's query string.
    fn to_query_string(
        &self,
        escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        let group = self
            .group_query_node
            .as_ref()
            .ok_or_else(rucene::error::LuceneError::null_pointer)?;
        Ok(CharSequence::from(format!(
            "{}@{}",
            group.to_query_string(escape_syntax_parser)?,
            self.min_should_match
        )))
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.base.clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: self.base.cloned_state(this.clone(), clauses),
            min_should_match: self.min_should_match,
            group_query_node: self.group_query_node.clone(),
        }))
    }
}
