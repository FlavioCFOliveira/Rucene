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

//! Port of `AnyQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.nodes`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::Result;

use super::and_query_node::{AndQueryNode, AND_QUERY_NODE_CLASS};
use super::query_node::{QueryNode, QueryNodeRef};
use super::query_node_impl::{Mutable, QueryNodeImpl};
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::jdk::char_sequence::{value_of, CharSequence};
use crate::jdk::class::JavaClass;

/// `org.apache.lucene.queryparser.flexible.core.nodes.AnyQueryNode`.
pub static ANY_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.core.nodes.AnyQueryNode",
    Some(&AND_QUERY_NODE_CLASS),
    &[],
);

/// A [`AnyQueryNode`] represents an ANY operator performed on a list of
/// nodes.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.core.nodes.AnyQueryNode`,
/// which extends `AndQueryNode` (embedded). It is not a `FieldableNode`.
#[derive(Debug)]
pub struct AnyQueryNode {
    base: AndQueryNode,
    field: Mutable<Option<CharSequence>>,
    minimum_matching_elements: Mutable<i32>,
}

impl AnyQueryNode {
    /// Equivalent to `new AnyQueryNode(List<QueryNode> clauses, CharSequence
    /// field, int minimumMatchingElements)`: every `FieldQueryNode` clause
    /// is set to the field and ignores fields in `toQueryString`.
    ///
    /// # Errors
    ///
    /// The exceptions of `AndQueryNode(List<QueryNode>)`.
    pub fn new(
        clauses: Option<Vec<QueryNodeRef>>,
        field: Option<CharSequence>,
        minimum_matching_elements: i32,
    ) -> Result<Arc<Self>> {
        let node = Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: AndQueryNode::fields(QueryNodeImpl::new(this.clone())),
            field: Mutable::new(None),
            minimum_matching_elements: Mutable::new(0),
        });
        node.base.init(clauses.clone())?;
        node.field.set(field.clone());
        node.minimum_matching_elements
            .set(minimum_matching_elements);
        if let Some(clauses) = clauses {
            for clause in clauses {
                if let Some(fqn) = clause.as_field_query_node() {
                    fqn.query_node_impl()
                        .set_to_query_string_ignore_fields(true);
                    fqn.set_field(field.clone());
                }
            }
        }
        Ok(node)
    }

    /// Equivalent to `getMinimumMatchingElements()`.
    pub fn get_minimum_matching_elements(&self) -> i32 {
        self.minimum_matching_elements.get()
    }

    /// Equivalent to `getField()`: the field, `None` for Java's `null`.
    pub fn get_field(&self) -> Option<CharSequence> {
        self.field.get()
    }

    /// Equivalent to `getFieldAsString()`.
    pub fn get_field_as_string(&self) -> Option<String> {
        self.field.get().map(|f| f.to_string())
    }

    /// Equivalent to `setField(CharSequence field)`.
    pub fn set_field(&self, field: Option<CharSequence>) {
        self.field.set(field);
    }
}

impl QueryNode for AnyQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        self.base.query_node_impl()
    }

    fn java_class(&self) -> &'static JavaClass {
        &ANY_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.query_node_impl().clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: self.base.cloned(this.clone(), clauses),
            field: Mutable::new(self.field.get()),
            minimum_matching_elements: Mutable::new(self.minimum_matching_elements.get()),
        }))
    }

    fn to_java_string(&self) -> Result<String> {
        let field = value_of(self.field.get().as_ref());
        let n = self.minimum_matching_elements.get();
        match self.get_children() {
            Some(children) if !children.is_empty() => {
                let mut sb = format!("<any field='{field}'  matchelements={n}>");
                for clause in &children {
                    sb.push('\n');
                    sb.push_str(&clause.to_java_string()?);
                }
                sb.push_str("\n</any>");
                Ok(sb)
            }
            _ => Ok(format!("<any field='{field}'  matchelements={n}/>")),
        }
    }

    fn to_query_string(
        &self,
        escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        let any_str = format!("ANY {}", self.minimum_matching_elements.get());
        let mut sb = String::new();
        if let Some(children) = self.get_children().filter(|c| !c.is_empty()) {
            let mut filler = "";
            for clause in &children {
                sb.push_str(filler);
                sb.push_str(&clause.to_query_string(escape_syntax_parser)?.to_string());
                filler = " ";
            }
        }
        let field = self.field.get();
        if self.query_node_impl().is_default_field(field.as_ref()) {
            Ok(CharSequence::from(format!("( {sb} ) {any_str}")))
        } else {
            Ok(CharSequence::from(format!(
                "{}:(( {sb} ) {any_str})",
                value_of(field.as_ref())
            )))
        }
    }
}
