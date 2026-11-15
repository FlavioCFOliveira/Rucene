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

//! Port of `OpaqueQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.nodes`.
use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::Result;

use super::query_node::{QueryNode, QueryNodeRef};
use super::query_node_impl::{Mutable, QueryNodeImpl, QUERY_NODE_IMPL_CLASS};
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::jdk::char_sequence::{value_of, CharSequence};
use crate::jdk::class::JavaClass;

/// `org.apache.lucene.queryparser.flexible.core.nodes.OpaqueQueryNode`.
pub static OPAQUE_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.core.nodes.OpaqueQueryNode",
    Some(&QUERY_NODE_IMPL_CLASS),
    &[],
);

/// A [`OpaqueQueryNode`] is used for specify values that are not supposed to
/// be parsed by the parser. For example: and XPATH query in the middle of a
/// query string `a b @xpath:'/bookstore/book[1]/title' c d`
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.core.nodes.OpaqueQueryNode`.
#[derive(Debug)]
pub struct OpaqueQueryNode {
    base: QueryNodeImpl,
    schema: Mutable<Option<CharSequence>>,
    value: Mutable<Option<CharSequence>>,
}

impl OpaqueQueryNode {
    /// Equivalent to `new OpaqueQueryNode(CharSequence schema, CharSequence
    /// value)`.
    ///
    /// * `schema` - schema identifier
    /// * `value` - value that was not parsed
    pub fn new(schema: Option<CharSequence>, value: Option<CharSequence>) -> Arc<Self> {
        let node = Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: QueryNodeImpl::new(this.clone()),
            schema: Mutable::new(None),
            value: Mutable::new(None),
        });
        node.base.set_leaf(true);
        node.schema.set(schema);
        node.value.set(value);
        node
    }

    /// Equivalent to `getSchema()`: the schema.
    pub fn get_schema(&self) -> Option<CharSequence> {
        self.schema.get()
    }

    /// Equivalent to `getValue()`: the value.
    pub fn get_value(&self) -> Option<CharSequence> {
        self.value.get()
    }
}

impl QueryNode for OpaqueQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        &self.base
    }

    fn java_class(&self) -> &'static JavaClass {
        &OPAQUE_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_java_string(&self) -> Result<String> {
        Ok(format!(
            "<opaque schema='{}' value='{}'/>",
            value_of(self.schema.get().as_ref()),
            value_of(self.value.get().as_ref())
        ))
    }

    fn to_query_string(
        &self,
        _escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        Ok(CharSequence::from(format!(
            "@{}:'{}'",
            value_of(self.schema.get().as_ref()),
            value_of(self.value.get().as_ref())
        )))
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.base.clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: self.base.cloned_state(this.clone(), clauses),
            schema: Mutable::new(self.schema.get()),
            value: Mutable::new(self.value.get()),
        }))
    }
}
