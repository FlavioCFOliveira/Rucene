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

//! Port of `FieldableNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.nodes`.

use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::JavaClass;

/// `org.apache.lucene.queryparser.flexible.core.nodes.FieldableNode`.
pub static FIELDABLE_NODE_CLASS: JavaClass = JavaClass::interface(
    "org.apache.lucene.queryparser.flexible.core.nodes.FieldableNode",
    &[&super::query_node::QUERY_NODE_CLASS],
);

/// A query node implements [`FieldableNode`] interface to indicate that its
/// children and itself are associated to a specific field.
///
/// If it has any children which also implements this interface, it must
/// ensure the children are associated to the same field.
///
/// Equivalent to the interface
/// `org.apache.lucene.queryparser.flexible.core.nodes.FieldableNode`. It
/// extends `QueryNode`: a node gives it through
/// [`QueryNode::as_fieldable_node`](super::query_node::QueryNode::as_fieldable_node),
/// and the `QueryNode` methods are called on the node itself.
pub trait FieldableNode: Send + Sync {
    /// Returns the field associated to the node and every node under it.
    ///
    /// Equivalent to `getField()`; `None` for Java's `null`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the class's implementation (a missing
    /// child).
    fn get_field(&self) -> rucene::error::Result<Option<CharSequence>>;

    /// Associates the node to a field.
    ///
    /// Equivalent to `setField(CharSequence fieldName)`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the class's implementation.
    fn set_field(&self, field_name: Option<CharSequence>) -> rucene::error::Result<()>;
}
