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

//! Port of `QueryNodeImpl.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.nodes`.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use rucene::error::{LuceneError, Result};
use rucene::util::java_collections::JavaHashMap;

use super::query_node::{JavaObjectRef, QueryNode, QueryNodeRef};
use crate::flexible::core::messages::query_parser_messages::QueryParserMessages;
use crate::flexible::core::util::string_utils::StringUtils;
use crate::flexible::messages::nls::NLS;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::{JavaClass, CLONEABLE, OBJECT};
use crate::jdk::locale::Locale;
use crate::jdk::resource_bundle::ResourceBundle;
use crate::jdk::string::to_lower_case_str;

/// `org.apache.lucene.queryparser.flexible.core.nodes.QueryNodeImpl`.
pub static QUERY_NODE_IMPL_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.core.nodes.QueryNodeImpl",
    Some(&OBJECT),
    &[&super::query_node::QUERY_NODE_CLASS, &CLONEABLE],
);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A field of a Java object that its methods read and write through a
/// shared reference: the lock is held for one access only.
#[derive(Debug, Default)]
pub struct Mutable<T>(Mutex<T>);

impl<T: Clone> Mutable<T> {
    /// A field holding `value`.
    pub fn new(value: T) -> Self {
        Self(Mutex::new(value))
    }

    /// Reads the field.
    pub fn get(&self) -> T {
        lock(&self.0).clone()
    }

    /// Writes the field.
    pub fn set(&self, value: T) {
        *lock(&self.0) = value;
    }
}

/// A default implementation of QueryNode
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queryparser.flexible.core.nodes.QueryNodeImpl`: the
/// state every node holds (leaf flag, tags, children list, parent and
/// `toQueryStringIgnoreFields`) and the methods that act on it; the
/// [`QueryNode`] trait's provided methods call them. See
/// [`query_node`](super::query_node) for the ownership model.
pub struct QueryNodeImpl {
    /// The node this state belongs to (`this`).
    this: Weak<dyn QueryNode>,
    is_leaf: AtomicBool,
    tags: Mutex<JavaHashMap<JavaObjectRef>>,
    clauses: Mutex<Option<Vec<QueryNodeRef>>>,
    parent: Mutex<Option<Weak<dyn QueryNode>>>,
    to_query_string_ignore_fields: AtomicBool,
}

impl fmt::Debug for QueryNodeImpl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QueryNodeImpl")
            .field("is_leaf", &self.is_leaf.load(Ordering::Relaxed))
            .field("children", &lock(&self.clauses).as_ref().map(Vec::len))
            .finish()
    }
}

impl QueryNodeImpl {
    /// index default field
    ///
    /// Equivalent to `QueryNodeImpl.PLAINTEXT_FIELD_NAME`.
    pub const PLAINTEXT_FIELD_NAME: &'static str = "_plain";

    /// The state of a new node, `this` being the node (the weak reference
    /// [`Arc::new_cyclic`] gives): a leaf, without tags, children list or
    /// parent.
    pub fn new(this: Weak<dyn QueryNode>) -> Self {
        Self {
            this,
            is_leaf: AtomicBool::new(true),
            tags: Mutex::new(JavaHashMap::new()),
            clauses: Mutex::new(None),
            parent: Mutex::new(None),
            to_query_string_ignore_fields: AtomicBool::new(false),
        }
    }

    /// The node this state belongs to.
    pub fn this(&self) -> Option<QueryNodeRef> {
        self.this.upgrade()
    }

    /// `isLeaf()` dispatched to the node (it may override it).
    fn dispatch_is_leaf(&self) -> bool {
        match self.this() {
            Some(this) => this.is_leaf(),
            None => self.is_leaf_value(),
        }
    }

    /// Equivalent to the protected `allocate()`: a new empty children list,
    /// or the current one cleared.
    pub fn allocate(&self) {
        let mut clauses = lock(&self.clauses);
        match clauses.as_mut() {
            None => *clauses = Some(Vec::new()),
            Some(list) => list.clear(),
        }
    }

    /// The `IllegalArgumentException` of `add`: the localized
    /// `NODE_ACTION_NOT_SUPPORTED` message.
    fn node_action_not_supported() -> LuceneError {
        match NLS::get_localized_message(
            QueryParserMessages::NODE_ACTION_NOT_SUPPORTED
                .get()
                .as_deref(),
        ) {
            Ok(message) => LuceneError::illegal_argument(message),
            Err(e) => e,
        }
    }

    /// Equivalent to the final `add(QueryNode child)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` with the `NODE_ACTION_NOT_SUPPORTED`
    /// message for a leaf or a node without children list.
    pub fn add(&self, child: QueryNodeRef) -> Result<()> {
        if self.dispatch_is_leaf() {
            return Err(Self::node_action_not_supported());
        }
        {
            let mut clauses = lock(&self.clauses);
            match clauses.as_mut() {
                None => {
                    drop(clauses);
                    return Err(Self::node_action_not_supported());
                }
                Some(list) => list.push(Arc::clone(&child)),
            }
        }
        child.query_node_impl().set_parent(self.this());
        Ok(())
    }

    /// Equivalent to the final `add(List<QueryNode> children)`.
    ///
    /// # Errors
    ///
    /// As [`add`](Self::add).
    pub fn add_all(&self, children: Vec<QueryNodeRef>) -> Result<()> {
        if self.dispatch_is_leaf() || lock(&self.clauses).is_none() {
            return Err(Self::node_action_not_supported());
        }
        for child in children {
            self.add(child)?;
        }
        Ok(())
    }

    /// Equivalent to the default `isLeaf()`: the leaf flag.
    pub fn is_leaf_value(&self) -> bool {
        self.is_leaf.load(Ordering::SeqCst)
    }

    /// Equivalent to the final `set(List<QueryNode> children)`: the
    /// children are taken from their parents, the current children lose
    /// their parent, and `children` become the children.
    ///
    /// # Errors
    ///
    /// For a leaf or a node without children list, what
    /// `QueryNodeImpl.java:78-83` throws: it looks the message up in the
    /// bundle `org.apache.lucene.queryParser.messages.QueryParserMessages`,
    /// which does not exist, so the `MissingResourceException` of
    /// `ResourceBundle.getBundle`.
    pub fn set(&self, children: Vec<QueryNodeRef>) -> Result<()> {
        if self.dispatch_is_leaf() || lock(&self.clauses).is_none() {
            let bundle = ResourceBundle::get_bundle(
                "org.apache.lucene.queryParser.messages.QueryParserMessages",
                &Locale::get_default(),
            )?;
            let message = bundle.get_object(Some("Q0008E.NODE_ACTION_NOT_SUPPORTED"))?;
            return Err(LuceneError::illegal_argument(message));
        }
        // reset parent value
        for child in &children {
            child.remove_from_parent();
        }
        let existing_children = self.get_children().ok_or_else(LuceneError::null_pointer)?;
        for existing_child in existing_children {
            existing_child.remove_from_parent();
        }
        // allocate new children list
        self.allocate();
        // add new children and set parent
        self.add_all(children)
    }

    /// The children list of a clone: each child cloned (`cloneTree()`), in
    /// order; `None` without children list.
    ///
    /// # Errors
    ///
    /// The exceptions of cloning a child.
    pub fn clone_children(&self) -> Result<Option<Vec<QueryNodeRef>>> {
        let clauses = lock(&self.clauses).clone();
        match clauses {
            None => Ok(None),
            Some(list) => {
                let mut local_clauses = Vec::with_capacity(list.len());
                for clause in list {
                    local_clauses.push(clause.clone_tree()?);
                }
                Ok(Some(local_clauses))
            }
        }
    }

    /// The state of a clone, `this` being the clone: `super.clone()` (a
    /// shallow copy, the parent included), with new empty tags and the
    /// children list [`clone_children`](Self::clone_children) built.
    pub fn cloned_state(
        &self,
        this: Weak<dyn QueryNode>,
        clauses: Option<Vec<QueryNodeRef>>,
    ) -> QueryNodeImpl {
        QueryNodeImpl {
            this,
            is_leaf: AtomicBool::new(self.is_leaf_value()),
            // Reset all tags
            tags: Mutex::new(JavaHashMap::new()),
            clauses: Mutex::new(clauses),
            parent: Mutex::new(lock(&self.parent).clone()),
            to_query_string_ignore_fields: AtomicBool::new(self.to_query_string_ignore_fields()),
        }
    }

    /// Equivalent to the protected `setLeaf(boolean isLeaf)`.
    pub fn set_leaf(&self, is_leaf: bool) {
        self.is_leaf.store(is_leaf, Ordering::SeqCst);
    }

    /// Equivalent to the final `getChildren()`.
    pub fn get_children(&self) -> Option<Vec<QueryNodeRef>> {
        if self.dispatch_is_leaf() {
            return None;
        }
        lock(&self.clauses).clone()
    }

    /// The tag key: `tagName.toLowerCase(Locale.ROOT)`.
    fn tag_key(tag_name: &str) -> String {
        to_lower_case_str(tag_name, &Locale::root())
    }

    /// Equivalent to `setTag(String tagName, Object value)`.
    pub fn set_tag(&self, tag_name: &str, value: JavaObjectRef) {
        lock(&self.tags).insert(Self::tag_key(tag_name), value);
    }

    /// Equivalent to `unsetTag(String tagName)`.
    pub fn unset_tag(&self, tag_name: &str) {
        lock(&self.tags).remove(&Self::tag_key(tag_name));
    }

    /// Equivalent to `containsTag(String tagName)`.
    pub fn contains_tag(&self, tag_name: &str) -> bool {
        lock(&self.tags).contains_key(&Self::tag_key(tag_name))
    }

    /// Equivalent to `getTag(String tagName)`.
    pub fn get_tag(&self, tag_name: &str) -> Option<JavaObjectRef> {
        lock(&self.tags).get(&Self::tag_key(tag_name)).cloned()
    }

    /// Equivalent to the private `setParent(QueryNode parent)`: a new parent
    /// takes the node from its old one first.
    fn set_parent(&self, parent: Option<QueryNodeRef>) {
        let same = {
            let current = lock(&self.parent);
            match (current.as_ref(), parent.as_ref()) {
                (None, None) => true,
                (Some(w), Some(p)) => w.as_ptr() as *const () == Arc::as_ptr(p) as *const (),
                _ => false,
            }
        };
        if !same {
            match self.this() {
                Some(this) => this.remove_from_parent(),
                None => self.remove_from_parent(),
            }
            *lock(&self.parent) = parent.as_ref().map(Arc::downgrade);
        }
    }

    /// Equivalent to `getParent()` (see the ownership model of
    /// [`query_node`](super::query_node)).
    pub fn get_parent(&self) -> Option<QueryNodeRef> {
        lock(&self.parent).as_ref().and_then(Weak::upgrade)
    }

    /// Equivalent to the protected `isRoot()`: `getParent() == null`.
    pub fn is_root(&self) -> bool {
        match self.this() {
            Some(this) => this.get_parent().is_none(),
            None => self.get_parent().is_none(),
        }
    }

    /// The protected field `toQueryStringIgnoreFields`.
    pub fn to_query_string_ignore_fields(&self) -> bool {
        self.to_query_string_ignore_fields.load(Ordering::SeqCst)
    }

    /// Writes the protected field `toQueryStringIgnoreFields`.
    pub fn set_to_query_string_ignore_fields(&self, value: bool) {
        self.to_query_string_ignore_fields
            .store(value, Ordering::SeqCst);
    }

    /// Equivalent to the protected `isDefaultField(CharSequence fld)`.
    pub fn is_default_field(&self, fld: Option<&CharSequence>) -> bool {
        if self.to_query_string_ignore_fields() {
            return true;
        }
        let Some(fld) = fld else {
            return true;
        };
        StringUtils::to_string(Some(fld)).as_deref() == Some(Self::PLAINTEXT_FIELD_NAME)
    }

    /// `QueryNodeImpl.toString()`, `Object.toString()`: the class name of the
    /// node and its identity hash, as
    /// [`java_object_to_string`](rucene::index::index_reader::java_object_to_string)
    /// renders it.
    pub fn object_to_string(&self) -> String {
        match self.this() {
            Some(this) => rucene::index::index_reader::java_object_to_string(
                this.java_class().get_name(),
                &*this,
            ),
            None => rucene::index::index_reader::java_object_to_string(
                QUERY_NODE_IMPL_CLASS.get_name(),
                self,
            ),
        }
    }

    /// Equivalent to `getTagMap()`: a copy of the tags.
    pub fn get_tag_map(&self) -> JavaHashMap<JavaObjectRef> {
        lock(&self.tags).clone()
    }

    /// Equivalent to `removeChildren(QueryNode childNode)`: every occurrence
    /// of the child (by identity) leaves the children list, then the child
    /// leaves its parent.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for a node without children list.
    pub fn remove_children(&self, child_node: &QueryNodeRef) -> Result<()> {
        {
            let mut clauses = lock(&self.clauses);
            let list = clauses.as_mut().ok_or_else(LuceneError::null_pointer)?;
            list.retain(|c| !Arc::ptr_eq(c, child_node));
        }
        child_node.remove_from_parent();
        Ok(())
    }

    /// Equivalent to `removeFromParent()`.
    pub fn remove_from_parent(&self) {
        let parent = lock(&self.parent).take();
        if let Some(parent) = parent {
            if let (Some(parent), Some(this)) = (parent.upgrade(), self.this()) {
                // A parent has a children list: it holds this node.
                let _ = parent.remove_children(&this);
            }
        }
    }
}

/// `getChildren().get(0)` of a node: its first child.
///
/// # Errors
///
/// `NullPointerException` without children list, and the
/// `IndexOutOfBoundsException` of `ArrayList.get` without children.
pub fn first_child(node: &dyn QueryNode) -> Result<QueryNodeRef> {
    let children = node.get_children().ok_or_else(LuceneError::null_pointer)?;
    children.first().cloned().ok_or_else(|| {
        LuceneError::unchecked(
            "java.lang.IndexOutOfBoundsException",
            format!("Index 0 out of bounds for length {}", children.len()),
        )
    })
}

/// `getValueString()` of the nodes holding a number: `Float f =
/// Float.valueOf(value); if (f == f.longValue()) return "" + f.longValue();
/// else return "" + f;`.
pub fn float_value_string(f: f32) -> String {
    let l = f as i64;
    if f == l as f32 {
        l.to_string()
    } else {
        rucene::util::java_fmt::float_to_string(f)
    }
}
