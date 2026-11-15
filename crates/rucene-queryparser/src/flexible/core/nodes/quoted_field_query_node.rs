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

//! Port of `QuotedFieldQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.nodes`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::Result;

use super::field_query_node::{FieldQueryNode, FIELD_QUERY_NODE_CLASS};
use super::fieldable_node::FieldableNode;
use super::query_node::{QueryNode, QueryNodeRef};
use super::query_node_impl::QueryNodeImpl;
use super::textable_query_node::TextableQueryNode;
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::jdk::char_sequence::{value_of, CharSequence};
use crate::jdk::class::JavaClass;

/// `org.apache.lucene.queryparser.flexible.core.nodes.QuotedFieldQueryNode`.
pub static QUOTED_FIELD_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.core.nodes.QuotedFieldQueryNode",
    Some(&FIELD_QUERY_NODE_CLASS),
    &[],
);

/// A [`QuotedFieldQueryNode`] represents phrase query. Example: "life is
/// great"
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.core.nodes.QuotedFieldQueryNode`,
/// which extends `FieldQueryNode` (embedded).
#[derive(Debug)]
pub struct QuotedFieldQueryNode {
    base: FieldQueryNode,
}

impl QuotedFieldQueryNode {
    /// Equivalent to `new QuotedFieldQueryNode(CharSequence field,
    /// CharSequence text, int begin, int end)`.
    ///
    /// * `field` - field name
    /// * `text` - value
    /// * `begin` - position in the query string
    /// * `end` - position in the query string
    pub fn new(
        field: Option<CharSequence>,
        text: Option<CharSequence>,
        begin: i32,
        end: i32,
    ) -> Arc<Self> {
        let node = Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: FieldQueryNode::fields(QueryNodeImpl::new(this.clone()), field, text, begin, end),
        });
        node.base.init();
        node
    }
}

impl QueryNode for QuotedFieldQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        self.base.query_node_impl()
    }

    fn java_class(&self) -> &'static JavaClass {
        &QUOTED_FIELD_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_query_string(&self, escaper: &dyn EscapeQuerySyntax) -> Result<CharSequence> {
        let field = self.base.get_field();
        let quoted = value_of(self.base.get_term_escape_quoted(escaper)?.as_ref());
        if self.query_node_impl().is_default_field(field.as_ref()) {
            Ok(CharSequence::from(format!("\"{quoted}\"")))
        } else {
            Ok(CharSequence::from(format!(
                "{}:\"{quoted}\"",
                value_of(field.as_ref())
            )))
        }
    }

    fn to_java_string(&self) -> Result<String> {
        Ok(format!(
            "<quotedfield start='{}' end='{}' field='{}' term='{}'/>",
            self.base.get_begin(),
            self.base.get_end(),
            value_of(self.base.get_field().as_ref()),
            value_of(self.base.get_text().as_ref())
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
        Some(&self.base)
    }

    fn as_textable_query_node(&self) -> Option<&dyn TextableQueryNode> {
        Some(&self.base)
    }

    fn as_field_query_node(&self) -> Option<&FieldQueryNode> {
        Some(&self.base)
    }
}
