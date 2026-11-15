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

//! Port of `BoostQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.nodes`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::Result;

use super::query_node::{QueryNode, QueryNodeRef};
use super::query_node_impl::{float_value_string, Mutable, QueryNodeImpl, QUERY_NODE_IMPL_CLASS};
use crate::flexible::core::messages::query_parser_messages::QueryParserMessages;
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::flexible::core::query_node_error::QueryNodeError;
use crate::flexible::messages::message_impl::MessageImpl;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::JavaClass;
use crate::jdk::object::JavaObject;

/// `org.apache.lucene.queryparser.flexible.core.nodes.BoostQueryNode`.
pub static BOOST_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.core.nodes.BoostQueryNode",
    Some(&QUERY_NODE_IMPL_CLASS),
    &[],
);

/// A [`BoostQueryNode`] boosts the QueryNode tree which is under this node.
/// So, it must only and always have one child.
///
/// The boost value may vary from 0.0 to 1.0.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.core.nodes.BoostQueryNode`.
#[derive(Debug)]
pub struct BoostQueryNode {
    base: QueryNodeImpl,
    value: Mutable<f32>,
}

impl BoostQueryNode {
    /// Constructs a boost node
    ///
    /// Equivalent to `new BoostQueryNode(QueryNode query, float value)`.
    ///
    /// * `query` - the query to be boosted
    /// * `value` - the boost value, it may vary from 0.0 to 1.0
    ///
    /// # Errors
    ///
    /// The `QueryNodeError` `NODE_ACTION_NOT_SUPPORTED` for a `None` query.
    pub fn new(query: Option<QueryNodeRef>, value: f32) -> Result<Arc<Self>> {
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

    /// Returns the single child which this node boosts.
    ///
    /// Equivalent to `getChild()`: `None` (Java's `null`) without children.
    pub fn get_child(&self) -> Option<QueryNodeRef> {
        self.get_children()?.first().cloned()
    }

    /// Returns the boost value. It may vary from 0.0 to 1.0.
    ///
    /// Equivalent to `getValue()`.
    pub fn get_value(&self) -> f32 {
        self.value.get()
    }

    /// Returns the boost value parsed to a string.
    ///
    /// Equivalent to the private `getValueString()`.
    fn get_value_string(&self) -> String {
        float_value_string(self.value.get())
    }
}

impl QueryNode for BoostQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        &self.base
    }

    fn java_class(&self) -> &'static JavaClass {
        &BOOST_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// Equivalent to `toString()`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` without a child.
    fn to_java_string(&self) -> Result<String> {
        let child = self
            .get_child()
            .ok_or_else(rucene::error::LuceneError::null_pointer)?;
        Ok(format!(
            "<boost value='{}'>\n{}\n</boost>",
            self.get_value_string(),
            child.to_java_string()?
        ))
    }

    fn to_query_string(
        &self,
        escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        let Some(child) = self.get_child() else {
            return Ok(CharSequence::from(""));
        };
        Ok(CharSequence::from(format!(
            "{}^{}",
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
}
