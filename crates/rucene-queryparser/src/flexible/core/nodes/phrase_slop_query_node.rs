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

//! Port of `PhraseSlopQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.nodes`.
use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::Result;

use super::fieldable_node::{FieldableNode, FIELDABLE_NODE_CLASS};
use super::query_node::{QueryNode, QueryNodeRef};
use super::query_node_impl::{
    first_child, float_value_string, Mutable, QueryNodeImpl, QUERY_NODE_IMPL_CLASS,
};
use crate::flexible::core::messages::query_parser_messages::QueryParserMessages;
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::flexible::core::query_node_error::QueryNodeError;
use crate::flexible::messages::message_impl::MessageImpl;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::JavaClass;
use crate::jdk::object::JavaObject;

/// `org.apache.lucene.queryparser.flexible.core.nodes.PhraseSlopQueryNode`.
pub static PHRASE_SLOP_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.core.nodes.PhraseSlopQueryNode",
    Some(&QUERY_NODE_IMPL_CLASS),
    &[&FIELDABLE_NODE_CLASS],
);

/// Query node for `PhraseQuery`'s slop factor.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.core.nodes.PhraseSlopQueryNode`.
#[derive(Debug)]
pub struct PhraseSlopQueryNode {
    base: QueryNodeImpl,
    value: Mutable<i32>,
}

impl PhraseSlopQueryNode {
    /// Equivalent to `new PhraseSlopQueryNode(QueryNode query, int value)`.
    ///
    /// * `query` - the query to which the slop applies
    /// * `value` - slop value
    ///
    /// # Errors
    ///
    /// The `QueryNodeError` `NODE_ACTION_NOT_SUPPORTED` for a `None` query.
    pub fn new(query: Option<QueryNodeRef>, value: i32) -> Result<Arc<Self>> {
        let Some(query) = query else {
            return Err(QueryNodeError::new(Arc::new(MessageImpl::with_args(
                QueryParserMessages::NODE_ACTION_NOT_SUPPORTED.get(),
                vec![JavaObject::from("query"), JavaObject::from("null")],
            )))
            .error());
        };
        let node = Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: QueryNodeImpl::new(this.clone()),
            value: Mutable::new(value),
        });
        node.base.set_leaf(false);
        node.base.allocate();
        node.base.add(query)?;
        Ok(node)
    }

    /// Equivalent to `getChild()`: `getChildren().get(0)`.
    ///
    /// # Errors
    ///
    /// The exceptions of a missing child (see [`first_child`]).
    pub fn get_child(&self) -> Result<QueryNodeRef> {
        first_child(self)
    }

    /// Equivalent to `getValue()`.
    pub fn get_value(&self) -> i32 {
        self.value.get()
    }

    /// Equivalent to the private `getValueString()`.
    fn get_value_string(&self) -> String {
        float_value_string(self.value.get() as f32)
    }
}

impl QueryNode for PhraseSlopQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        &self.base
    }

    fn java_class(&self) -> &'static JavaClass {
        &PHRASE_SLOP_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_java_string(&self) -> Result<String> {
        Ok(format!(
            "<phraseslop value='{}'>\n{}\n</phraseslop>",
            self.get_value_string(),
            self.get_child()?.to_java_string()?
        ))
    }

    fn to_query_string(
        &self,
        escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        let child = self.get_child()?;
        Ok(CharSequence::from(format!(
            "{}~{}",
            child.to_query_string(escape_syntax_parser)?,
            self.get_value_string()
        )))
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.base.clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: self.base.cloned_state(this.clone(), clauses),
            value: Mutable::new(self.value.get()),
        }))
    }

    fn as_fieldable_node(&self) -> Option<&dyn FieldableNode> {
        Some(self)
    }
}

impl FieldableNode for PhraseSlopQueryNode {
    fn get_field(&self) -> Result<Option<CharSequence>> {
        let child = self.get_child()?;
        match child.as_fieldable_node() {
            Some(fieldable) => fieldable.get_field(),
            None => Ok(None),
        }
    }

    fn set_field(&self, field_name: Option<CharSequence>) -> Result<()> {
        let child = self.get_child()?;
        if let Some(fieldable) = child.as_fieldable_node() {
            fieldable.set_field(field_name)?;
        }
        Ok(())
    }
}
