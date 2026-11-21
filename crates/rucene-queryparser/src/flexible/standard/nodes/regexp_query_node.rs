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

//! Port of `RegexpQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.nodes`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, Result};
use rucene::util::BytesRef;

use crate::flexible::core::nodes::fieldable_node::{FieldableNode, FIELDABLE_NODE_CLASS};
use crate::flexible::core::nodes::query_node::{QueryNode, QueryNodeRef};
use crate::flexible::core::nodes::query_node_impl::{
    Mutable, QueryNodeImpl, QUERY_NODE_IMPL_CLASS,
};
use crate::flexible::core::nodes::textable_query_node::{
    TextableQueryNode, TEXTABLE_QUERY_NODE_CLASS,
};
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::jdk::char_sequence::{value_of, CharSequence};
use crate::jdk::class::JavaClass;

/// `org.apache.lucene.queryparser.flexible.standard.nodes.RegexpQueryNode`.
pub static REGEXP_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.standard.nodes.RegexpQueryNode",
    Some(&QUERY_NODE_IMPL_CLASS),
    &[&TEXTABLE_QUERY_NODE_CLASS, &FIELDABLE_NODE_CLASS],
);

/// A [`RegexpQueryNode`] represents `RegexpQuery` query Examples: /[a-z]|[0-9]/
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.standard.nodes.RegexpQueryNode`.
#[derive(Debug)]
pub struct RegexpQueryNode {
    base: QueryNodeImpl,
    text: Mutable<Option<CharSequence>>,
    field: Mutable<Option<CharSequence>>,
}

impl RegexpQueryNode {
    /// Equivalent to `new RegexpQueryNode(CharSequence field, CharSequence
    /// text, int begin, int end)`: the text is `text.subSequence(begin,
    /// end)`.
    ///
    /// * `field` - field name
    /// * `text` - value that contains a regular expression
    /// * `begin` - position in the query string
    /// * `end` - position in the query string
    ///
    /// # Errors
    ///
    /// The `IndexOutOfBoundsException` of `subSequence`.
    pub fn new(
        field: Option<CharSequence>,
        text: &CharSequence,
        begin: usize,
        end: usize,
    ) -> Result<Arc<Self>> {
        let text = text.sub_sequence(begin, end)?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: QueryNodeImpl::new(this.clone()),
            text: Mutable::new(Some(text)),
            field: Mutable::new(field),
        }))
    }

    /// Equivalent to `new RegexpQueryNode(CharSequence field, CharSequence
    /// text)`: the whole text.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_text(field: Option<CharSequence>, text: &CharSequence) -> Result<Arc<Self>> {
        Self::new(field, text, 0, text.length())
    }

    /// Equivalent to `textToBytesRef()`: `new BytesRef(text)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for a `null` text.
    pub fn text_to_bytes_ref(&self) -> Result<BytesRef> {
        let text = self.text.get().ok_or_else(LuceneError::null_pointer)?;
        Ok(BytesRef::from_char_sequence(&text.to_string()))
    }

    /// Equivalent to `getText()`.
    pub fn get_text(&self) -> Option<CharSequence> {
        self.text.get()
    }

    /// Equivalent to `setText(CharSequence text)`.
    pub fn set_text(&self, text: Option<CharSequence>) {
        self.text.set(text);
    }

    /// Equivalent to `getField()`.
    pub fn get_field(&self) -> Option<CharSequence> {
        self.field.get()
    }

    /// Equivalent to `getFieldAsString()`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for a `null` field.
    pub fn get_field_as_string(&self) -> Result<String> {
        Ok(self
            .field
            .get()
            .ok_or_else(LuceneError::null_pointer)?
            .to_string())
    }

    /// Equivalent to `setField(CharSequence field)`.
    pub fn set_field(&self, field: Option<CharSequence>) {
        self.field.set(field);
    }
}

impl QueryNode for RegexpQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        &self.base
    }

    fn java_class(&self) -> &'static JavaClass {
        &REGEXP_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_java_string(&self) -> Result<String> {
        Ok(format!(
            "<regexp field='{}' term='{}'/>",
            value_of(self.field.get().as_ref()),
            value_of(self.text.get().as_ref())
        ))
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.base.clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: self.base.cloned_state(this.clone(), clauses),
            text: Mutable::new(self.text.get()),
            field: Mutable::new(self.field.get()),
        }))
    }

    fn to_query_string(
        &self,
        _escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        let field = self.field.get();
        let text = value_of(self.text.get().as_ref());
        if self.base.is_default_field(field.as_ref()) {
            Ok(CharSequence::from(format!("/{text}/")))
        } else {
            Ok(CharSequence::from(format!(
                "{}:/{text}/",
                value_of(field.as_ref())
            )))
        }
    }

    fn as_fieldable_node(&self) -> Option<&dyn FieldableNode> {
        Some(self)
    }

    fn as_textable_query_node(&self) -> Option<&dyn TextableQueryNode> {
        Some(self)
    }
}

impl TextableQueryNode for RegexpQueryNode {
    fn get_text(&self) -> Option<CharSequence> {
        RegexpQueryNode::get_text(self)
    }

    fn set_text(&self, text: Option<CharSequence>) {
        RegexpQueryNode::set_text(self, text);
    }
}

impl FieldableNode for RegexpQueryNode {
    fn get_field(&self) -> Result<Option<CharSequence>> {
        Ok(RegexpQueryNode::get_field(self))
    }

    fn set_field(&self, field_name: Option<CharSequence>) -> Result<()> {
        RegexpQueryNode::set_field(self, field_name);
        Ok(())
    }
}
