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

//! Port of `FuzzyQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.nodes`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::Result;
use rucene::util::java_fmt::float_to_string;

use super::field_query_node::{FieldQueryNode, FIELD_QUERY_NODE_CLASS};
use super::fieldable_node::FieldableNode;
use super::query_node::{QueryNode, QueryNodeRef};
use super::query_node_impl::{Mutable, QueryNodeImpl};
use super::textable_query_node::TextableQueryNode;
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::jdk::char_sequence::{value_of, CharSequence};
use crate::jdk::class::JavaClass;

/// `org.apache.lucene.queryparser.flexible.core.nodes.FuzzyQueryNode`.
pub static FUZZY_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.core.nodes.FuzzyQueryNode",
    Some(&FIELD_QUERY_NODE_CLASS),
    &[],
);

/// A [`FuzzyQueryNode`] represents a element that contains
/// field/text/similarity tuple
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.core.nodes.FuzzyQueryNode`,
/// which extends `FieldQueryNode` (embedded).
#[derive(Debug)]
pub struct FuzzyQueryNode {
    base: FieldQueryNode,
    similarity: Mutable<f32>,
    prefix_length: Mutable<i32>,
}

impl FuzzyQueryNode {
    /// Equivalent to `new FuzzyQueryNode(CharSequence field, CharSequence
    /// term, float minSimilarity, int begin, int end)`.
    ///
    /// * `field` - Name of the field query will use.
    /// * `term` - Term token to use for building term for the query
    /// * `min_similarity` - similarity value
    /// * `begin` - position in the query string
    /// * `end` - position in the query string
    pub fn new(
        field: Option<CharSequence>,
        term: Option<CharSequence>,
        min_similarity: f32,
        begin: i32,
        end: i32,
    ) -> Arc<Self> {
        let node = Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: FieldQueryNode::fields(QueryNodeImpl::new(this.clone()), field, term, begin, end),
            similarity: Mutable::new(0.0),
            prefix_length: Mutable::new(0),
        });
        node.base.init();
        node.similarity.set(min_similarity);
        node.query_node_impl().set_leaf(true);
        node
    }

    /// Equivalent to `setPrefixLength(int prefixLength)`.
    pub fn set_prefix_length(&self, prefix_length: i32) {
        self.prefix_length.set(prefix_length);
    }

    /// Equivalent to `getPrefixLength()`.
    pub fn get_prefix_length(&self) -> i32 {
        self.prefix_length.get()
    }

    /// Equivalent to `setSimilarity(float similarity)`.
    pub fn set_similarity(&self, similarity: f32) {
        self.similarity.set(similarity);
    }

    /// Get the similarity value.
    ///
    /// Equivalent to `getSimilarity()`.
    pub fn get_similarity(&self) -> f32 {
        self.similarity.get()
    }
}

impl QueryNode for FuzzyQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        self.base.query_node_impl()
    }

    fn java_class(&self) -> &'static JavaClass {
        &FUZZY_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_query_string(&self, escaper: &dyn EscapeQuerySyntax) -> Result<CharSequence> {
        let field = self.base.get_field();
        let escaped = value_of(self.base.get_term_escaped(escaper)?.as_ref());
        let similarity = float_to_string(self.similarity.get());
        if self.query_node_impl().is_default_field(field.as_ref()) {
            Ok(CharSequence::from(format!("{escaped}~{similarity}")))
        } else {
            Ok(CharSequence::from(format!(
                "{}:{escaped}~{similarity}",
                value_of(field.as_ref())
            )))
        }
    }

    fn to_java_string(&self) -> Result<String> {
        Ok(format!(
            "<fuzzy field='{}' similarity='{}' term='{}'/>",
            value_of(self.base.get_field().as_ref()),
            float_to_string(self.similarity.get()),
            value_of(self.base.get_text().as_ref())
        ))
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.query_node_impl().clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: self
                .base
                .cloned(self.query_node_impl().cloned_state(this.clone(), clauses)),
            similarity: Mutable::new(self.similarity.get()),
            prefix_length: Mutable::new(self.prefix_length.get()),
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
