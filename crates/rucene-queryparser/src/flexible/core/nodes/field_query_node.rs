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

//! Port of `FieldQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.nodes`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::Result;

use super::field_value_pair_query_node::{
    FieldValuePairQueryNode, FIELD_VALUE_PAIR_QUERY_NODE_CLASS,
};
use super::fieldable_node::FieldableNode;
use super::query_node::{QueryNode, QueryNodeRef};
use super::query_node_impl::{Mutable, QueryNodeImpl, QUERY_NODE_IMPL_CLASS};
use super::textable_query_node::{TextableQueryNode, TEXTABLE_QUERY_NODE_CLASS};
use super::value_query_node::ValueQueryNode;
use crate::flexible::core::parser::escape_query_syntax::{EscapeQuerySyntax, Type};
use crate::jdk::char_sequence::{value_of, CharSequence};
use crate::jdk::class::JavaClass;
use crate::jdk::locale::Locale;

/// `org.apache.lucene.queryparser.flexible.core.nodes.FieldQueryNode`.
pub static FIELD_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.core.nodes.FieldQueryNode",
    Some(&QUERY_NODE_IMPL_CLASS),
    &[
        &FIELD_VALUE_PAIR_QUERY_NODE_CLASS,
        &TEXTABLE_QUERY_NODE_CLASS,
    ],
);

/// A [`FieldQueryNode`] represents a element that contains field/text tuple
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.core.nodes.FieldQueryNode`.
/// Its subclasses embed it and give it through
/// [`QueryNode::as_field_query_node`].
///
/// **Divergence.** Java's `toQueryString` returns the escaper's result,
/// `null` for a `null` text; the port returns the text `null` there, which is
/// what every Lucene caller renders it as (string concatenation).
#[derive(Debug)]
pub struct FieldQueryNode {
    base: QueryNodeImpl,
    /// The term's field
    field: Mutable<Option<CharSequence>>,
    /// The term's text.
    text: Mutable<Option<CharSequence>>,
    /// The term's begin position.
    begin: Mutable<i32>,
    /// The term's end position.
    end: Mutable<i32>,
    /// The term's position increment.
    position_increment: Mutable<i32>,
}

impl FieldQueryNode {
    /// The fields of a new node.
    pub(crate) fn fields(
        base: QueryNodeImpl,
        field: Option<CharSequence>,
        text: Option<CharSequence>,
        begin: i32,
        end: i32,
    ) -> Self {
        Self {
            base,
            field: Mutable::new(field),
            text: Mutable::new(text),
            begin: Mutable::new(begin),
            end: Mutable::new(end),
            position_increment: Mutable::new(0),
        }
    }

    /// The body of the constructor: a leaf.
    pub(crate) fn init(&self) {
        self.base.set_leaf(true);
    }

    /// Equivalent to `new FieldQueryNode(CharSequence field, CharSequence
    /// text, int begin, int end)`.
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
        let node = Arc::new_cyclic(|this: &Weak<Self>| {
            Self::fields(QueryNodeImpl::new(this.clone()), field, text, begin, end)
        });
        node.init();
        node
    }

    /// The fields of a clone (`cloneTree()` of `FieldQueryNode`), over the
    /// clone's `QueryNodeImpl` state.
    pub(crate) fn cloned(&self, base: QueryNodeImpl) -> Self {
        let clone = Self::fields(
            base,
            self.field.get(),
            self.text.get(),
            self.begin.get(),
            self.end.get(),
        );
        clone.position_increment.set(self.position_increment.get());
        clone
            .base
            .set_to_query_string_ignore_fields(self.base.to_query_string_ignore_fields());
        clone
    }

    /// Equivalent to the protected `getTermEscaped(EscapeQuerySyntax
    /// escaper)`: the text escaped as a term, in the default locale.
    ///
    /// # Errors
    ///
    /// The exceptions of the escaper.
    pub fn get_term_escaped(
        &self,
        escaper: &dyn EscapeQuerySyntax,
    ) -> Result<Option<CharSequence>> {
        escaper.escape(
            self.text.get().as_ref(),
            &Locale::get_default(),
            Type::Normal,
        )
    }

    /// Equivalent to the protected `getTermEscapeQuoted(EscapeQuerySyntax
    /// escaper)`: the text escaped as a quoted string, in the default
    /// locale.
    ///
    /// # Errors
    ///
    /// The exceptions of the escaper.
    pub fn get_term_escape_quoted(
        &self,
        escaper: &dyn EscapeQuerySyntax,
    ) -> Result<Option<CharSequence>> {
        escaper.escape(
            self.text.get().as_ref(),
            &Locale::get_default(),
            Type::String,
        )
    }

    /// Equivalent to `getTextAsString()`.
    pub fn get_text_as_string(&self) -> Option<String> {
        self.text.get().map(|t| t.to_string())
    }

    /// Equivalent to `getFieldAsString()`.
    pub fn get_field_as_string(&self) -> Option<String> {
        self.field.get().map(|f| f.to_string())
    }

    /// Equivalent to `getBegin()`.
    pub fn get_begin(&self) -> i32 {
        self.begin.get()
    }

    /// Equivalent to `setBegin(int begin)`.
    pub fn set_begin(&self, begin: i32) {
        self.begin.set(begin);
    }

    /// Equivalent to `getEnd()`.
    pub fn get_end(&self) -> i32 {
        self.end.get()
    }

    /// Equivalent to `setEnd(int end)`.
    pub fn set_end(&self, end: i32) {
        self.end.set(end);
    }

    /// Equivalent to `getField()`.
    pub fn get_field(&self) -> Option<CharSequence> {
        self.field.get()
    }

    /// Equivalent to `setField(CharSequence field)`.
    pub fn set_field(&self, field: Option<CharSequence>) {
        self.field.set(field);
    }

    /// Equivalent to `getPositionIncrement()`.
    pub fn get_position_increment(&self) -> i32 {
        self.position_increment.get()
    }

    /// Equivalent to `setPositionIncrement(int pi)`.
    pub fn set_position_increment(&self, pi: i32) {
        self.position_increment.set(pi);
    }

    /// Returns the term.
    ///
    /// Equivalent to `getText()`.
    pub fn get_text(&self) -> Option<CharSequence> {
        self.text.get()
    }

    /// Equivalent to `setText(CharSequence text)`.
    pub fn set_text(&self, text: Option<CharSequence>) {
        self.text.set(text);
    }

    /// Equivalent to `getValue()`: the text.
    pub fn get_value(&self) -> Option<CharSequence> {
        self.get_text()
    }

    /// Equivalent to `setValue(CharSequence value)`: the text.
    pub fn set_value(&self, value: Option<CharSequence>) {
        self.set_text(value);
    }

    /// `toQueryString` of `FieldQueryNode`.
    pub(crate) fn field_to_query_string(
        &self,
        escaper: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        let field = self.field.get();
        let escaped = value_of(self.get_term_escaped(escaper)?.as_ref());
        if self.base.is_default_field(field.as_ref()) {
            Ok(CharSequence::from(escaped))
        } else {
            Ok(CharSequence::from(format!(
                "{}:{escaped}",
                value_of(field.as_ref())
            )))
        }
    }
}

impl QueryNode for FieldQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        &self.base
    }

    fn java_class(&self) -> &'static JavaClass {
        &FIELD_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_query_string(&self, escaper: &dyn EscapeQuerySyntax) -> Result<CharSequence> {
        self.field_to_query_string(escaper)
    }

    fn to_java_string(&self) -> Result<String> {
        Ok(format!(
            "<field start='{}' end='{}' field='{}' text='{}'/>",
            self.begin.get(),
            self.end.get(),
            value_of(self.field.get().as_ref()),
            value_of(self.text.get().as_ref())
        ))
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.base.clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| {
            self.cloned(self.base.cloned_state(this.clone(), clauses))
        }))
    }

    fn as_fieldable_node(&self) -> Option<&dyn FieldableNode> {
        Some(self)
    }

    fn as_textable_query_node(&self) -> Option<&dyn TextableQueryNode> {
        Some(self)
    }

    fn as_field_query_node(&self) -> Option<&FieldQueryNode> {
        Some(self)
    }
}

impl FieldableNode for FieldQueryNode {
    fn get_field(&self) -> Result<Option<CharSequence>> {
        Ok(FieldQueryNode::get_field(self))
    }

    fn set_field(&self, field_name: Option<CharSequence>) -> Result<()> {
        FieldQueryNode::set_field(self, field_name);
        Ok(())
    }
}

impl TextableQueryNode for FieldQueryNode {
    fn get_text(&self) -> Option<CharSequence> {
        FieldQueryNode::get_text(self)
    }

    fn set_text(&self, text: Option<CharSequence>) {
        FieldQueryNode::set_text(self, text);
    }
}

impl ValueQueryNode<Option<CharSequence>> for FieldQueryNode {
    fn set_value(&self, value: Option<CharSequence>) {
        FieldQueryNode::set_value(self, value);
    }

    fn get_value(&self) -> Option<CharSequence> {
        FieldQueryNode::get_value(self)
    }
}

impl FieldValuePairQueryNode<Option<CharSequence>> for FieldQueryNode {}
