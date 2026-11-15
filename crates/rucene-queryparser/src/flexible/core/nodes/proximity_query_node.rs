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

//! Port of `ProximityQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.nodes`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::Result;

use super::boolean_query_node::{BooleanQueryNode, BOOLEAN_QUERY_NODE_CLASS};
use super::query_node::{QueryNode, QueryNodeRef};
use super::query_node_impl::{Mutable, QueryNodeImpl};
use crate::flexible::core::messages::query_parser_messages::QueryParserMessages;
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::flexible::core::query_node_error::QueryNodeError;
use crate::flexible::messages::message_impl::MessageImpl;
use crate::jdk::char_sequence::{value_of, CharSequence};
use crate::jdk::class::JavaClass;
use crate::jdk::object::JavaObject;

/// `org.apache.lucene.queryparser.flexible.core.nodes.ProximityQueryNode`.
pub static PROXIMITY_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.core.nodes.ProximityQueryNode",
    Some(&BOOLEAN_QUERY_NODE_CLASS),
    &[],
);

/// Distance condition: PARAGRAPH, SENTENCE, or NUMBER
///
/// Equivalent to the enum
/// `org.apache.lucene.queryparser.flexible.core.nodes.ProximityQueryNode.Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Type {
    /// Java constant `PARAGRAPH`.
    Paragraph,
    /// Java constant `SENTENCE`.
    Sentence,
    /// Java constant `NUMBER`.
    Number,
}

impl Type {
    /// Equivalent to the package-private abstract `toQueryString()` each
    /// constant implements.
    pub(crate) fn to_query_string(self) -> &'static str {
        match self {
            Type::Paragraph => "WITHIN PARAGRAPH",
            Type::Sentence => "WITHIN SENTENCE",
            Type::Number => "WITHIN",
        }
    }

    /// Equivalent to `toString()`, the constant's name.
    pub fn to_java_string(self) -> &'static str {
        match self {
            Type::Paragraph => "PARAGRAPH",
            Type::Sentence => "SENTENCE",
            Type::Number => "NUMBER",
        }
    }
}

/// utility class containing the distance condition and number
///
/// Equivalent to the static nested class
/// `org.apache.lucene.queryparser.flexible.core.nodes.ProximityQueryNode.ProximityType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProximityType {
    pub(crate) p_distance: i32,
    pub(crate) p_type: Type,
}

impl ProximityType {
    /// Equivalent to `new ProximityType(Type type)`: a distance of 0.
    pub fn new(type_: Type) -> Self {
        Self::with_distance(type_, 0)
    }

    /// Equivalent to `new ProximityType(Type type, int distance)`.
    pub fn with_distance(type_: Type, distance: i32) -> Self {
        Self {
            p_type: type_,
            p_distance: distance,
        }
    }
}

/// A [`ProximityQueryNode`] represents a query where the terms should meet
/// specific distance conditions. `(a b c) WITHIN [SENTENCE|PARAGRAPH|NUMBER]
/// [INORDER] ("a" "b" "c") WITHIN [SENTENCE|PARAGRAPH|NUMBER] [INORDER]`
///
/// TODO: Add this to the future standard Lucene parser/processor/builder
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.core.nodes.ProximityQueryNode`,
/// which extends `BooleanQueryNode` (embedded). It is not a
/// `FieldableNode`.
#[derive(Debug)]
pub struct ProximityQueryNode {
    base: BooleanQueryNode,
    proximity_type: Mutable<Type>,
    distance: Mutable<i32>,
    inorder: bool,
    field: Mutable<Option<CharSequence>>,
}

impl ProximityQueryNode {
    /// Equivalent to `new ProximityQueryNode(List<QueryNode> clauses,
    /// CharSequence field, Type type, int distance, boolean inorder)`: every
    /// `FieldQueryNode` clause is set to the field and ignores fields in
    /// `toQueryString`; the distance is kept for `Type.NUMBER` only.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for `None` clauses, and the `QueryNodeError`
    /// `PARAMETER_VALUE_NOT_SUPPORTED` for a `NUMBER` distance not above 0.
    pub fn new(
        clauses: Option<Vec<QueryNodeRef>>,
        field: Option<CharSequence>,
        type_: Type,
        distance: i32,
        inorder: bool,
    ) -> Result<Arc<Self>> {
        let node = Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: BooleanQueryNode::fields(QueryNodeImpl::new(this.clone())),
            proximity_type: Mutable::new(type_),
            distance: Mutable::new(-1),
            inorder,
            field: Mutable::new(None),
        });
        node.base.init(clauses.clone())?;
        node.query_node_impl().set_leaf(false);
        node.proximity_type.set(type_);
        node.field.set(field.clone());
        if type_ == Type::Number {
            if distance <= 0 {
                return Err(QueryNodeError::new(Arc::new(MessageImpl::with_args(
                    QueryParserMessages::PARAMETER_VALUE_NOT_SUPPORTED.get(),
                    vec![JavaObject::from("distance"), JavaObject::from(distance)],
                )))
                .error());
            } else {
                node.distance.set(distance);
            }
        }
        Self::clear_fields(clauses.as_deref(), field);
        Ok(node)
    }

    /// Equivalent to `new ProximityQueryNode(List<QueryNode> clauses,
    /// CharSequence field, Type type, boolean inorder)`: a distance of -1.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn without_distance(
        clauses: Option<Vec<QueryNodeRef>>,
        field: Option<CharSequence>,
        type_: Type,
        inorder: bool,
    ) -> Result<Arc<Self>> {
        Self::new(clauses, field, type_, -1, inorder)
    }

    /// Equivalent to the private `clearFields(List<QueryNode> nodes,
    /// CharSequence field)`.
    fn clear_fields(nodes: Option<&[QueryNodeRef]>, field: Option<CharSequence>) {
        let Some(nodes) = nodes else {
            return;
        };
        for clause in nodes {
            if let Some(fqn) = clause.as_field_query_node() {
                fqn.query_node_impl()
                    .set_to_query_string_ignore_fields(true);
                fqn.set_field(field.clone());
            }
        }
    }

    /// Equivalent to `getProximityType()`.
    pub fn get_proximity_type(&self) -> Type {
        self.proximity_type.get()
    }

    /// Equivalent to `getDistance()`: the distance, -1 when none.
    pub fn get_distance(&self) -> i32 {
        self.distance.get()
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

    /// Equivalent to `isInOrder()`.
    pub fn is_in_order(&self) -> bool {
        self.inorder
    }
}

impl QueryNode for ProximityQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        self.base.query_node_impl()
    }

    fn java_class(&self) -> &'static JavaClass {
        &PROXIMITY_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_java_string(&self) -> Result<String> {
        let distance = self.distance.get();
        let distance_str = if distance == -1 {
            String::new()
        } else {
            format!(" distance='{distance}'")
        };
        let head = format!(
            "<proximity field='{}' inorder='{}' type='{}'{distance_str}",
            value_of(self.field.get().as_ref()),
            self.inorder,
            self.proximity_type.get().to_java_string()
        );
        match self.get_children() {
            Some(children) if !children.is_empty() => {
                let mut sb = format!("{head}>");
                for child in &children {
                    sb.push('\n');
                    sb.push_str(&child.to_java_string()?);
                }
                sb.push_str("\n</proximity>");
                Ok(sb)
            }
            _ => Ok(format!("{head}/>")),
        }
    }

    fn to_query_string(
        &self,
        escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        let distance = self.distance.get();
        let within_str = format!(
            "{}{}{}",
            self.proximity_type.get().to_query_string(),
            if distance == -1 {
                String::new()
            } else {
                format!(" {distance}")
            },
            if self.inorder { " INORDER" } else { "" }
        );
        let mut sb = String::new();
        if let Some(children) = self.get_children().filter(|c| !c.is_empty()) {
            let mut filler = "";
            for child in &children {
                sb.push_str(filler);
                sb.push_str(&child.to_query_string(escape_syntax_parser)?.to_string());
                filler = " ";
            }
        }
        let field = self.field.get();
        if self.query_node_impl().is_default_field(field.as_ref()) {
            Ok(CharSequence::from(format!("( {sb} ) {within_str}")))
        } else {
            Ok(CharSequence::from(format!(
                "{}:(( {sb} ) {within_str})",
                value_of(field.as_ref())
            )))
        }
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.query_node_impl().clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: self.base.cloned(this.clone(), clauses),
            proximity_type: Mutable::new(self.proximity_type.get()),
            distance: Mutable::new(self.distance.get()),
            inorder: self.inorder,
            field: Mutable::new(self.field.get()),
        }))
    }
}
