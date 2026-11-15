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

//! Port of `QueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.nodes`, and the ownership
//! model of the port's query node trees.
//!
//! # Ownership model
//!
//! Java's query node tree is a graph of shared, mutable objects: a node
//! keeps its children and its parent, processors hold nodes while they
//! rewrite the tree around them, `getChildren()` hands out the references,
//! and `QueryNodeImpl` compares nodes by identity. The port keeps that
//! model:
//!
//! * a node is shared as a [`QueryNodeRef`], an [`Arc`] of the trait object;
//!   cloning it copies the reference, and [`Arc::ptr_eq`] is Java's `==`;
//! * every method takes `&self`; the mutable state of a node lives behind
//!   locks held only for the duration of one field access, never across a
//!   call into another node, so a method may call any other node (its parent
//!   included) as Java's do;
//! * a node keeps its children strongly and its parent weakly, and holds a
//!   weak reference to itself (built with [`Arc::new_cyclic`]), through which
//!   `QueryNodeImpl` passes `this` to its children and dispatches its own
//!   virtual calls;
//! * Java's class hierarchy is kept by descriptors ([`JavaClass`]) and by
//!   views: [`QueryNode::as_any`] for a class without subclasses, and the
//!   `as_*` methods of this trait for the interfaces and for the classes
//!   that have subclasses (a subclass embeds the struct of its superclass and
//!   returns it). A view gives the members of that class or interface; the
//!   methods of [`QueryNode`] are always called on the node itself, which
//!   dispatches to the most derived override as Java does.
//!
//! **Divergence.** Java's collector keeps a parent alive while any child
//! references it; a strong parent link would make every tree a reference
//! cycle that Rust never frees. The parent link is therefore weak:
//! [`QueryNode::get_parent`] returns `None` (Java's `null`) once nothing
//! outside the tree holds the parent or one of its ancestors any more, where
//! Java would still return it. While the root of a tree is held, as it is
//! for the whole of parsing, processing and building, every parent is
//! returned as in Java.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use rucene::error::Result;
use rucene::util::java_collections::JavaHashMap;

use super::field_query_node::FieldQueryNode;
use super::fieldable_node::FieldableNode;
use super::modifier_query_node::ModifierQueryNode;
use super::query_node_impl::QueryNodeImpl;
use super::range_query_node::RangeQueryNode;
use super::textable_query_node::TextableQueryNode;
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::flexible::standard::nodes::abstract_range_query_node::AbstractRangeQueryNode;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::JavaClass;

/// A reference to a query node: Java's `QueryNode` reference.
pub type QueryNodeRef = Arc<dyn QueryNode>;

/// A `java.lang.Object` reference held as a tag value, or built by a
/// `QueryBuilder`; downcast it to the class it holds.
pub type JavaObjectRef = Arc<dyn Any + Send + Sync>;

/// `org.apache.lucene.queryparser.flexible.core.nodes.QueryNode`.
pub static QUERY_NODE_CLASS: JavaClass = JavaClass::interface(
    "org.apache.lucene.queryparser.flexible.core.nodes.QueryNode",
    &[],
);

/// A [`QueryNode`] is an interface implemented by all nodes on a QueryNode
/// tree.
///
/// Equivalent to the interface
/// `org.apache.lucene.queryparser.flexible.core.nodes.QueryNode`, together
/// with the overridable methods of `QueryNodeImpl`, whose state every node of
/// the port holds ([`query_node_impl`](QueryNode::query_node_impl)); the
/// provided methods are those of `QueryNodeImpl` (the `final` ones are not
/// overridden). See the module documentation for the ownership model.
///
/// **Divergence.** Java lets a class implement `QueryNode` without extending
/// `QueryNodeImpl`, but `QueryNodeImpl.add` casts every child to
/// `QueryNodeImpl` (`QueryNodeImpl.java:58`): such a node cannot be a child.
/// The port requires the `QueryNodeImpl` state of every node; no Lucene node
/// lacks it.
pub trait QueryNode: Any + Send + Sync + fmt::Debug {
    /// The `QueryNodeImpl` state of the node.
    fn query_node_impl(&self) -> &QueryNodeImpl;

    /// `getClass()`.
    fn java_class(&self) -> &'static JavaClass;

    /// The node, for a downcast to its class (`instanceof` of a class
    /// without subclasses).
    fn as_any(&self) -> &dyn Any;

    /// convert to a query string understood by the query parser
    ///
    /// Equivalent to `toQueryString(EscapeQuerySyntax escapeSyntaxParser)`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the class's implementation (a missing
    /// child, a failing number format).
    fn to_query_string(&self, escape_syntax_parser: &dyn EscapeQuerySyntax)
        -> Result<CharSequence>;

    /// for printing
    ///
    /// Equivalent to `toString()`: `QueryNodeImpl.toString()` is
    /// `Object.toString()`, the class name and the identity hash.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the class's implementation.
    fn to_java_string(&self) -> Result<String> {
        Ok(self.query_node_impl().object_to_string())
    }

    /// get Children nodes
    ///
    /// Equivalent to the final `getChildren()`: a copy of the children list,
    /// `None` (Java's `null`) for a leaf or a node without children list.
    fn get_children(&self) -> Option<Vec<QueryNodeRef>> {
        self.query_node_impl().get_children()
    }

    /// verify if a node is a Leaf node
    ///
    /// Equivalent to `isLeaf()`.
    fn is_leaf(&self) -> bool {
        self.query_node_impl().is_leaf_value()
    }

    /// verify if a node contains a tag
    ///
    /// Equivalent to `containsTag(String tagName)`.
    fn contains_tag(&self, tag_name: &str) -> bool {
        self.query_node_impl().contains_tag(tag_name)
    }

    /// Returns object stored under that tag name
    ///
    /// Equivalent to `getTag(String tagName)`.
    fn get_tag(&self, tag_name: &str) -> Option<JavaObjectRef> {
        self.query_node_impl().get_tag(tag_name)
    }

    /// Equivalent to `getParent()` (see the module documentation).
    fn get_parent(&self) -> Option<QueryNodeRef> {
        self.query_node_impl().get_parent()
    }

    /// Recursive clone the QueryNode tree. The tags are not copied to the new
    /// tree when you call the cloneTree() method.
    ///
    /// Equivalent to `cloneTree()`.
    ///
    /// # Errors
    ///
    /// Java's `CloneNotSupportedException`, and the exceptions of cloning a
    /// child.
    fn clone_tree(&self) -> Result<QueryNodeRef>;

    /// Add a new child to a non Leaf node
    ///
    /// Equivalent to the final `add(QueryNode child)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` with the `NODE_ACTION_NOT_SUPPORTED`
    /// message for a leaf or a node without children list.
    fn add(&self, child: QueryNodeRef) -> Result<()> {
        self.query_node_impl().add(child)
    }

    /// Equivalent to the final `add(List<QueryNode> children)`.
    ///
    /// # Errors
    ///
    /// As [`add`](QueryNode::add).
    fn add_all(&self, children: Vec<QueryNodeRef>) -> Result<()> {
        self.query_node_impl().add_all(children)
    }

    /// Reset the children of a node
    ///
    /// Equivalent to the final `set(List<QueryNode> children)`.
    ///
    /// # Errors
    ///
    /// For a leaf or a node without children list, what Java throws there:
    /// the `MissingResourceException` of a bundle that does not exist.
    fn set(&self, children: Vec<QueryNodeRef>) -> Result<()> {
        self.query_node_impl().set(children)
    }

    /// Associate the specified value with the specified tagName. If the tag
    /// name already exists, the old value is replaced. The tagName and value
    /// cannot be null. tagName will be converted to lowercase.
    ///
    /// Equivalent to `setTag(String tagName, Object value)`.
    fn set_tag(&self, tag_name: &str, value: JavaObjectRef) {
        self.query_node_impl().set_tag(tag_name, value);
    }

    /// Unset a tag. tagName will be converted to lowercase.
    ///
    /// Equivalent to `unsetTag(String tagName)`.
    fn unset_tag(&self, tag_name: &str) {
        self.query_node_impl().unset_tag(tag_name);
    }

    /// Returns a map containing all tags attached to this query node.
    ///
    /// Equivalent to `getTagMap()`: a copy of the tags, in Java's `HashMap`
    /// order.
    fn get_tag_map(&self) -> JavaHashMap<JavaObjectRef> {
        self.query_node_impl().get_tag_map()
    }

    /// Removes this query node from its parent.
    ///
    /// Equivalent to `removeFromParent()`.
    fn remove_from_parent(&self) {
        self.query_node_impl().remove_from_parent();
    }

    /// Remove a child node
    ///
    /// Equivalent to `removeChildren(QueryNode childNode)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for a node without children list.
    fn remove_children(&self, child_node: &QueryNodeRef) -> Result<()> {
        self.query_node_impl().remove_children(child_node)
    }

    /// The node as a `FieldableNode` (`instanceof FieldableNode`).
    fn as_fieldable_node(&self) -> Option<&dyn FieldableNode> {
        None
    }

    /// The node as a `TextableQueryNode` (`instanceof TextableQueryNode`).
    fn as_textable_query_node(&self) -> Option<&dyn TextableQueryNode> {
        None
    }

    /// The node as a `RangeQueryNode` (`instanceof RangeQueryNode`).
    fn as_range_query_node(&self) -> Option<&dyn RangeQueryNode> {
        None
    }

    /// The `FieldQueryNode` part of the node (`instanceof FieldQueryNode`).
    fn as_field_query_node(&self) -> Option<&FieldQueryNode> {
        None
    }

    /// The `ModifierQueryNode` part of the node (`instanceof
    /// ModifierQueryNode`).
    fn as_modifier_query_node(&self) -> Option<&ModifierQueryNode> {
        None
    }

    /// The `AbstractRangeQueryNode` part of the node (`instanceof
    /// AbstractRangeQueryNode`).
    fn as_abstract_range_query_node(&self) -> Option<&AbstractRangeQueryNode> {
        None
    }
}

/// `node instanceof class`: whether the class of `node` is `class`, a
/// subclass of it, or implements it.
pub fn instance_of(node: &dyn QueryNode, class: &JavaClass) -> bool {
    class.is_assignable_from(node.java_class())
}

/// The `ClassCastException` of the reference cast `(class) node` of a node
/// of another class, in the form `class <from> cannot be cast to class
/// <to>` the port's casts use.
pub fn class_cast(node: &dyn QueryNode, class: &JavaClass) -> rucene::error::LuceneError {
    rucene::error::LuceneError::unchecked(
        "java.lang.ClassCastException",
        format!(
            "class {} cannot be cast to class {}",
            node.java_class().get_name(),
            class.get_name()
        ),
    )
}

/// Java's `==` on two nullable node references.
pub fn same_node(a: Option<&QueryNodeRef>, b: Option<&QueryNodeRef>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    }
}
