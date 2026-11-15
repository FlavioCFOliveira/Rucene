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

//! Port of `ModifierQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.nodes`.
use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::Result;

use super::boolean_query_node::BOOLEAN_QUERY_NODE_CLASS;
use super::query_node::{instance_of, QueryNode, QueryNodeRef};
use super::query_node_impl::{first_child, Mutable, QueryNodeImpl, QUERY_NODE_IMPL_CLASS};
use crate::flexible::core::messages::query_parser_messages::QueryParserMessages;
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::flexible::core::query_node_error::QueryNodeError;
use crate::flexible::messages::message_impl::MessageImpl;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::JavaClass;
use crate::jdk::object::JavaObject;

/// `org.apache.lucene.queryparser.flexible.core.nodes.ModifierQueryNode`.
pub static MODIFIER_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.core.nodes.ModifierQueryNode",
    Some(&QUERY_NODE_IMPL_CLASS),
    &[],
);

/// Modifier type: NONE, REQ, NOT
///
/// Equivalent to the enum
/// `org.apache.lucene.queryparser.flexible.core.nodes.ModifierQueryNode.Modifier`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Modifier {
    /// Java constant `MOD_NONE`.
    ModNone,
    /// Java constant `MOD_NOT`.
    ModNot,
    /// Java constant `MOD_REQ`.
    ModReq,
}

impl Modifier {
    /// Equivalent to `toString()`.
    pub fn to_java_string(self) -> &'static str {
        match self {
            Modifier::ModNone => "MOD_NONE",
            Modifier::ModNot => "MOD_NOT",
            Modifier::ModReq => "MOD_REQ",
        }
    }

    /// Equivalent to `toDigitString()`.
    pub fn to_digit_string(self) -> &'static str {
        match self {
            Modifier::ModNone => "",
            Modifier::ModNot => "-",
            Modifier::ModReq => "+",
        }
    }

    /// Equivalent to `toLargeString()`.
    pub fn to_large_string(self) -> &'static str {
        match self {
            Modifier::ModNone => "",
            Modifier::ModNot => "NOT ",
            Modifier::ModReq => "+",
        }
    }
}

/// A [`ModifierQueryNode`] indicates the modifier value (+,-,?,NONE) for
/// each term on the query string. For example "+t1 -t2 t3" will have a tree
/// of: `<BooleanQueryNode> <ModifierQueryNode modifier="MOD_REQ"> <t1/>
/// </ModifierQueryNode> <ModifierQueryNode modifier="MOD_NOT"> <t2/>
/// </ModifierQueryNode> <t3/> </BooleanQueryNode>`
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.core.nodes.ModifierQueryNode`.
/// Its subclasses embed it and give it through
/// [`QueryNode::as_modifier_query_node`].
#[derive(Debug)]
pub struct ModifierQueryNode {
    base: QueryNodeImpl,
    modifier: Mutable<Modifier>,
}

impl ModifierQueryNode {
    pub(crate) fn fields(base: QueryNodeImpl, modifier: Modifier) -> Self {
        Self {
            base,
            modifier: Mutable::new(modifier),
        }
    }

    /// The body of the constructor, after the `null` check.
    pub(crate) fn init(&self, query: QueryNodeRef, modifier: Modifier) -> Result<()> {
        self.base.allocate();
        self.base.set_leaf(false);
        self.base.add(query)?;
        self.modifier.set(modifier);
        Ok(())
    }

    /// The `QueryNodeError` of a `null` query.
    pub(crate) fn null_query() -> rucene::error::LuceneError {
        QueryNodeError::new(Arc::new(MessageImpl::with_args(
            QueryParserMessages::PARAMETER_VALUE_NOT_SUPPORTED.get(),
            vec![JavaObject::from("query"), JavaObject::from("null")],
        )))
        .error()
    }

    /// Used to store the modifier value on the original query string
    ///
    /// Equivalent to `new ModifierQueryNode(QueryNode query, Modifier mod)`.
    ///
    /// # Errors
    ///
    /// The `QueryNodeError` `PARAMETER_VALUE_NOT_SUPPORTED` for a `None`
    /// query.
    pub fn new(query: Option<QueryNodeRef>, modifier: Modifier) -> Result<Arc<Self>> {
        let query = query.ok_or_else(Self::null_query)?;
        let node = Arc::new_cyclic(|this: &Weak<Self>| {
            Self::fields(QueryNodeImpl::new(this.clone()), modifier)
        });
        node.init(query, modifier)?;
        Ok(node)
    }

    /// Equivalent to `getChild()`: `getChildren().get(0)`.
    ///
    /// # Errors
    ///
    /// The exceptions of a missing child (see [`first_child`]).
    pub fn get_child(&self) -> Result<QueryNodeRef> {
        match self.base.this() {
            Some(this) => first_child(this.as_ref()),
            None => first_child(self),
        }
    }

    /// Equivalent to `getModifier()`.
    pub fn get_modifier(&self) -> Modifier {
        self.modifier.get()
    }

    /// Equivalent to `setChild(QueryNode child)`.
    ///
    /// # Errors
    ///
    /// The exceptions of `set`.
    pub fn set_child(&self, child: QueryNodeRef) -> Result<()> {
        self.base.set(vec![child])
    }

    /// The fields of a clone, over the clone's state.
    pub(crate) fn cloned(&self, base: QueryNodeImpl) -> Self {
        Self::fields(base, self.modifier.get())
    }

    /// `toString()` of `ModifierQueryNode`.
    pub(crate) fn modifier_to_java_string(&self) -> Result<String> {
        Ok(format!(
            "<modifier operation='{}'>\n{}\n</modifier>",
            self.modifier.get().to_java_string(),
            self.get_child()?.to_java_string()?
        ))
    }

    /// `toQueryString(EscapeQuerySyntax)` of `ModifierQueryNode`.
    pub(crate) fn modifier_to_query_string(
        &self,
        escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        let child = self.get_child()?;
        let (left_parenthensis, right_parenthensis) =
            if instance_of(child.as_ref(), &MODIFIER_QUERY_NODE_CLASS) {
                ("(", ")")
            } else {
                ("", "")
            };
        let child_string = child.to_query_string(escape_syntax_parser)?;
        if instance_of(child.as_ref(), &BOOLEAN_QUERY_NODE_CLASS) {
            Ok(CharSequence::from(format!(
                "{}{left_parenthensis}{child_string}{right_parenthensis}",
                self.modifier.get().to_large_string()
            )))
        } else {
            Ok(CharSequence::from(format!(
                "{}{left_parenthensis}{child_string}{right_parenthensis}",
                self.modifier.get().to_digit_string()
            )))
        }
    }
}

impl QueryNode for ModifierQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        &self.base
    }

    fn java_class(&self) -> &'static JavaClass {
        &MODIFIER_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_java_string(&self) -> Result<String> {
        self.modifier_to_java_string()
    }

    fn to_query_string(
        &self,
        escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        self.modifier_to_query_string(escape_syntax_parser)
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.base.clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| {
            self.cloned(self.base.cloned_state(this.clone(), clauses))
        }))
    }

    fn as_modifier_query_node(&self) -> Option<&ModifierQueryNode> {
        Some(self)
    }
}
