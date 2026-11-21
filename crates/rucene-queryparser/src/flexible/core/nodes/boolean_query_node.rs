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

//! Port of `BooleanQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.nodes`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, Result};

use super::group_query_node::GROUP_QUERY_NODE_CLASS;
use super::query_node::{instance_of, QueryNode, QueryNodeRef};
use super::query_node_impl::{QueryNodeImpl, QUERY_NODE_IMPL_CLASS};
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::JavaClass;

/// `org.apache.lucene.queryparser.flexible.core.nodes.BooleanQueryNode`.
pub static BOOLEAN_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.core.nodes.BooleanQueryNode",
    Some(&QUERY_NODE_IMPL_CLASS),
    &[],
);

/// A [`BooleanQueryNode`] represents a list of elements which do not have an
/// explicit boolean operator defined between them. It can be used to express
/// a boolean query that intends to use the default boolean operator.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.core.nodes.BooleanQueryNode`.
#[derive(Debug)]
pub struct BooleanQueryNode {
    base: QueryNodeImpl,
}

impl BooleanQueryNode {
    /// The fields of a new node (the constructor's body runs on the shared
    /// node, see [`init`](Self::init)).
    pub(crate) fn fields(base: QueryNodeImpl) -> Self {
        Self { base }
    }

    /// The body of `BooleanQueryNode(List<QueryNode> clauses)`: not a leaf,
    /// a new children list, set to the clauses.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for `None` clauses (`set(null)`).
    pub(crate) fn init(&self, clauses: Option<Vec<QueryNodeRef>>) -> Result<()> {
        self.base.set_leaf(false);
        self.base.allocate();
        self.base
            .set(clauses.ok_or_else(LuceneError::null_pointer)?)
    }

    /// Equivalent to `new BooleanQueryNode(List<QueryNode> clauses)`: the
    /// clauses become the children (and leave their parents).
    ///
    /// # Errors
    ///
    /// `NullPointerException` for `None` clauses.
    pub fn new(clauses: Option<Vec<QueryNodeRef>>) -> Result<Arc<Self>> {
        let node =
            Arc::new_cyclic(|this: &Weak<Self>| Self::fields(QueryNodeImpl::new(this.clone())));
        node.init(clauses)?;
        Ok(node)
    }

    /// `toString()` with `operation` as the operation name (`default`,
    /// `and`, `or`), as the subclasses write it.
    pub(crate) fn to_java_string_with(&self, operation: &str) -> Result<String> {
        let children = self.base.this().and_then(|t| t.get_children());
        match children {
            Some(children) if !children.is_empty() => {
                let mut sb = format!("<boolean operation='{operation}'>");
                for child in &children {
                    sb.push('\n');
                    sb.push_str(&child.to_java_string()?);
                }
                sb.push_str("\n</boolean>");
                Ok(sb)
            }
            _ => Ok(format!("<boolean operation='{operation}'/>")),
        }
    }

    /// `toQueryString(EscapeQuerySyntax)` with `filler` between the
    /// children, as the subclasses write it: parenthesized unless the node
    /// is the root or the child of a group.
    pub(crate) fn to_query_string_with(
        &self,
        escape_syntax_parser: &dyn EscapeQuerySyntax,
        filler: &str,
    ) -> Result<CharSequence> {
        let this = self.base.this();
        let children = this.as_ref().and_then(|t| t.get_children());
        let Some(children) = children.filter(|c| !c.is_empty()) else {
            return Ok(CharSequence::from(""));
        };
        let mut sb = String::new();
        let mut current_filler = "";
        for child in &children {
            sb.push_str(current_filler);
            sb.push_str(&child.to_query_string(escape_syntax_parser)?.to_string());
            current_filler = filler;
        }
        // in case is root or the parent is a group node avoid parenthesis
        let parent = this.as_ref().and_then(|t| t.get_parent());
        if parent.is_some_and(|p| instance_of(p.as_ref(), &GROUP_QUERY_NODE_CLASS))
            || self.base.is_root()
        {
            Ok(CharSequence::from(sb))
        } else {
            Ok(CharSequence::from(format!("( {sb} )")))
        }
    }

    /// The fields of a clone (`super.cloneTree()`), `this` being the clone
    /// and `clauses` its children.
    pub(crate) fn cloned(
        &self,
        this: Weak<dyn QueryNode>,
        clauses: Option<Vec<QueryNodeRef>>,
    ) -> Self {
        Self::fields(self.base.cloned_state(this, clauses))
    }
}

impl QueryNode for BooleanQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        &self.base
    }

    fn java_class(&self) -> &'static JavaClass {
        &BOOLEAN_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_java_string(&self) -> Result<String> {
        self.to_java_string_with("default")
    }

    fn to_query_string(
        &self,
        escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        self.to_query_string_with(escape_syntax_parser, " ")
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.base.clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| {
            self.cloned(this.clone(), clauses)
        }))
    }
}
