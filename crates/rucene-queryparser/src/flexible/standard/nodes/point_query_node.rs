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

//! Port of `PointQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.nodes`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, Result};

use crate::flexible::core::nodes::field_value_pair_query_node::{
    FieldValuePairQueryNode, FIELD_VALUE_PAIR_QUERY_NODE_CLASS,
};
use crate::flexible::core::nodes::fieldable_node::FieldableNode;
use crate::flexible::core::nodes::query_node::{QueryNode, QueryNodeRef};
use crate::flexible::core::nodes::query_node_impl::{
    Mutable, QueryNodeImpl, QUERY_NODE_IMPL_CLASS,
};
use crate::flexible::core::nodes::value_query_node::ValueQueryNode;
use crate::flexible::core::parser::escape_query_syntax::{EscapeQuerySyntax, Type};
use crate::jdk::char_sequence::{value_of, CharSequence};
use crate::jdk::class::JavaClass;
use crate::jdk::format::JavaNumber;
use crate::jdk::locale::Locale;
use crate::jdk::number_format::NumberFormat;

/// `org.apache.lucene.queryparser.flexible.standard.nodes.PointQueryNode`.
pub static POINT_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.standard.nodes.PointQueryNode",
    Some(&QUERY_NODE_IMPL_CLASS),
    &[&FIELD_VALUE_PAIR_QUERY_NODE_CLASS],
);

/// This query node represents a field query that holds a point value. It is
/// similar to `FieldQueryNode`, however the `getValue()` returns a `Number`.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.standard.nodes.PointQueryNode`.
#[derive(Debug)]
pub struct PointQueryNode {
    base: QueryNodeImpl,
    number_format: Mutable<Option<Arc<dyn NumberFormat>>>,
    field: Mutable<Option<CharSequence>>,
    value: Mutable<Option<JavaNumber>>,
}

impl PointQueryNode {
    /// Creates a [`PointQueryNode`] object using the given field, `Number`
    /// value and [`NumberFormat`] used to convert the value to `String`.
    ///
    /// Equivalent to `new PointQueryNode(CharSequence field, Number value,
    /// NumberFormat numberFormat)`.
    pub fn new(
        field: Option<CharSequence>,
        value: Option<JavaNumber>,
        number_format: Option<Arc<dyn NumberFormat>>,
    ) -> Arc<Self> {
        let node = Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: QueryNodeImpl::new(this.clone()),
            number_format: Mutable::new(None),
            field: Mutable::new(None),
            value: Mutable::new(None),
        });
        node.set_number_format(number_format);
        node.set_field(field);
        node.set_value(value);
        node
    }

    /// Returns the field associated with this node.
    ///
    /// Equivalent to `getField()`.
    pub fn get_field(&self) -> Option<CharSequence> {
        self.field.get()
    }

    /// Sets the field associated with this node.
    ///
    /// Equivalent to `setField(CharSequence fieldName)`.
    pub fn set_field(&self, field_name: Option<CharSequence>) {
        self.field.set(field_name);
    }

    /// `numberFormat.format(this.value)`: `Format.format(Object)` of the
    /// value.
    fn format_value(&self) -> Result<String> {
        let format = self
            .number_format
            .get()
            .ok_or_else(LuceneError::null_pointer)?;
        match self.value.get() {
            Some(value) => format.format(value),
            None => Err(LuceneError::illegal_argument(
                "Cannot format given Object as a Number",
            )),
        }
    }

    /// This method is used to get the value converted to `String` and
    /// escaped using the given [`EscapeQuerySyntax`].
    ///
    /// Equivalent to the protected `getTermEscaped(EscapeQuerySyntax
    /// escaper)`: the formatted value escaped as a term, in `Locale.ROOT`.
    ///
    /// # Errors
    ///
    /// The exceptions of the number format and of the escaper.
    pub fn get_term_escaped(
        &self,
        escaper: &dyn EscapeQuerySyntax,
    ) -> Result<Option<CharSequence>> {
        escaper.escape(
            Some(&CharSequence::from(self.format_value()?)),
            &Locale::root(),
            Type::Normal,
        )
    }

    /// Sets the [`NumberFormat`] used to convert the value to `String`.
    ///
    /// Equivalent to `setNumberFormat(NumberFormat format)`.
    pub fn set_number_format(&self, format: Option<Arc<dyn NumberFormat>>) {
        self.number_format.set(format);
    }

    /// Returns the [`NumberFormat`] used to convert the value to `String`.
    ///
    /// Equivalent to `getNumberFormat()`.
    pub fn get_number_format(&self) -> Option<Arc<dyn NumberFormat>> {
        self.number_format.get()
    }

    /// Returns the numeric value as `Number`.
    ///
    /// Equivalent to `getValue()`.
    pub fn get_value(&self) -> Option<JavaNumber> {
        self.value.get()
    }

    /// Sets the numeric value.
    ///
    /// Equivalent to `setValue(Number value)`.
    pub fn set_value(&self, value: Option<JavaNumber>) {
        self.value.set(value);
    }
}

impl QueryNode for PointQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        &self.base
    }

    fn java_class(&self) -> &'static JavaClass {
        &POINT_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_query_string(
        &self,
        escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        let field = self.field.get();
        let escaped = value_of(self.get_term_escaped(escape_syntax_parser)?.as_ref());
        if self.base.is_default_field(field.as_ref()) {
            Ok(CharSequence::from(escaped))
        } else {
            Ok(CharSequence::from(format!(
                "{}:{escaped}",
                value_of(field.as_ref())
            )))
        }
    }

    fn to_java_string(&self) -> Result<String> {
        Ok(format!(
            "<numeric field='{}' number='{}'/>",
            value_of(self.field.get().as_ref()),
            self.format_value()?
        ))
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.base.clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: self.base.cloned_state(this.clone(), clauses),
            number_format: Mutable::new(self.number_format.get()),
            field: Mutable::new(self.field.get()),
            value: Mutable::new(self.value.get()),
        }))
    }

    fn as_fieldable_node(&self) -> Option<&dyn FieldableNode> {
        Some(self)
    }
}

impl FieldableNode for PointQueryNode {
    fn get_field(&self) -> Result<Option<CharSequence>> {
        Ok(PointQueryNode::get_field(self))
    }

    fn set_field(&self, field_name: Option<CharSequence>) -> Result<()> {
        PointQueryNode::set_field(self, field_name);
        Ok(())
    }
}

impl ValueQueryNode<Option<JavaNumber>> for PointQueryNode {
    fn set_value(&self, value: Option<JavaNumber>) {
        PointQueryNode::set_value(self, value);
    }

    fn get_value(&self) -> Option<JavaNumber> {
        PointQueryNode::get_value(self)
    }
}

impl FieldValuePairQueryNode<Option<JavaNumber>> for PointQueryNode {}
