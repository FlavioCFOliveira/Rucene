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

//! Port of `AbstractRangeQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.nodes`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, Result};

use crate::flexible::core::nodes::field_value_pair_query_node::FIELD_VALUE_PAIR_QUERY_NODE_CLASS;
use crate::flexible::core::nodes::fieldable_node::FieldableNode;
use crate::flexible::core::nodes::query_node::{instance_of, QueryNode, QueryNodeRef};
use crate::flexible::core::nodes::query_node_impl::{
    Mutable, QueryNodeImpl, QUERY_NODE_IMPL_CLASS,
};
use crate::flexible::core::nodes::range_query_node::{RangeQueryNode, RANGE_QUERY_NODE_CLASS};
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::flexible::core::util::string_utils::StringUtils;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::JavaClass;

/// `org.apache.lucene.queryparser.flexible.standard.nodes.AbstractRangeQueryNode`.
pub static ABSTRACT_RANGE_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.standard.nodes.AbstractRangeQueryNode",
    Some(&QUERY_NODE_IMPL_CLASS),
    &[&RANGE_QUERY_NODE_CLASS],
);

/// The cast `(T)` of a bound to `FieldValuePairQueryNode`, the erasure of
/// the class's type parameter, and its `getField()`.
fn bound_field(bound: &QueryNodeRef) -> Result<Option<CharSequence>> {
    match bound.as_fieldable_node() {
        Some(fieldable) if instance_of(bound.as_ref(), &FIELD_VALUE_PAIR_QUERY_NODE_CLASS) => {
            fieldable.get_field()
        }
        _ => Err(class_cast(bound.as_ref())),
    }
}

/// The `ClassCastException` of a bound that is not a
/// `FieldValuePairQueryNode`.
fn class_cast(bound: &dyn QueryNode) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.ClassCastException",
        format!(
            "class {} cannot be cast to class {}",
            bound.java_class().get_name(),
            FIELD_VALUE_PAIR_QUERY_NODE_CLASS.get_name()
        ),
    )
}

/// This class should be extended by nodes intending to represent range
/// queries.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.standard.nodes.AbstractRangeQueryNode<T
/// extends FieldValuePairQueryNode<?>>`, a class with a protected
/// constructor that its subclasses embed and give through
/// [`QueryNode::as_abstract_range_query_node`]. The bounds are node
/// references (the type parameter is erased, as at run time in Java); a
/// bound used as a `FieldValuePairQueryNode` that is not one is the
/// `ClassCastException` Java's cast throws.
#[derive(Debug)]
pub struct AbstractRangeQueryNode {
    base: QueryNodeImpl,
    lower_inclusive: Mutable<bool>,
    upper_inclusive: Mutable<bool>,
}

impl AbstractRangeQueryNode {
    pub(crate) fn fields(base: QueryNodeImpl) -> Self {
        Self {
            base,
            lower_inclusive: Mutable::new(false),
            upper_inclusive: Mutable::new(false),
        }
    }

    /// The body of the protected constructor: not a leaf, an empty children
    /// list.
    pub(crate) fn init(&self) {
        self.base.set_leaf(false);
        self.base.allocate();
    }

    /// Equivalent to the protected `AbstractRangeQueryNode()`: a node
    /// without bounds.
    pub fn new() -> Arc<Self> {
        let node =
            Arc::new_cyclic(|this: &Weak<Self>| Self::fields(QueryNodeImpl::new(this.clone())));
        node.init();
        node
    }

    /// The fields of a clone (a shallow copy), over the clone's state.
    pub(crate) fn cloned(&self, base: QueryNodeImpl) -> Self {
        Self {
            base,
            lower_inclusive: Mutable::new(self.lower_inclusive.get()),
            upper_inclusive: Mutable::new(self.upper_inclusive.get()),
        }
    }

    /// Returns the field associated with this node: that of the lower
    /// bound, else that of the upper bound.
    ///
    /// Equivalent to `getField()`.
    ///
    /// # Errors
    ///
    /// The exceptions of a missing bound, and the `ClassCastException` of a
    /// bound that is not a `FieldValuePairQueryNode`.
    pub fn get_field(&self) -> Result<Option<CharSequence>> {
        let lower = self.get_lower_bound()?;
        let _upper = self.get_upper_bound()?;
        // A child is never null: the lower bound gives the field.
        bound_field(&lower)
    }

    /// Sets the field associated with this node.
    ///
    /// Equivalent to `setField(CharSequence fieldName)`: set on both bounds.
    ///
    /// # Errors
    ///
    /// As [`get_field`](Self::get_field).
    pub fn set_field(&self, field_name: Option<CharSequence>) -> Result<()> {
        let lower = self.get_lower_bound()?;
        let upper = self.get_upper_bound()?;
        for bound in [lower, upper] {
            match bound.as_fieldable_node() {
                Some(fieldable)
                    if instance_of(bound.as_ref(), &FIELD_VALUE_PAIR_QUERY_NODE_CLASS) =>
                {
                    fieldable.set_field(field_name.clone())?;
                }
                _ => return Err(class_cast(bound.as_ref())),
            }
        }
        Ok(())
    }

    /// Returns the child at `index` (`(T) getChildren().get(index)`).
    fn child(&self, index: usize) -> Result<QueryNodeRef> {
        let children = self.get_children().ok_or_else(LuceneError::null_pointer)?;
        match children.get(index) {
            Some(child) => Ok(Arc::clone(child)),
            None => Err(LuceneError::unchecked(
                "java.lang.IndexOutOfBoundsException",
                format!("Index {index} out of bounds for length {}", children.len()),
            )),
        }
    }

    /// Returns the lower bound node.
    ///
    /// Equivalent to `getLowerBound()`: the first child.
    ///
    /// # Errors
    ///
    /// The `IndexOutOfBoundsException` of a node without bounds.
    pub fn get_lower_bound(&self) -> Result<QueryNodeRef> {
        self.child(0)
    }

    /// Returns the upper bound node.
    ///
    /// Equivalent to `getUpperBound()`: the second child.
    ///
    /// # Errors
    ///
    /// The `IndexOutOfBoundsException` of a node without bounds.
    pub fn get_upper_bound(&self) -> Result<QueryNodeRef> {
        self.child(1)
    }

    /// Returns whether the lower bound is inclusive or exclusive.
    ///
    /// Equivalent to `isLowerInclusive()`.
    pub fn is_lower_inclusive(&self) -> bool {
        self.lower_inclusive.get()
    }

    /// Returns whether the upper bound is inclusive or exclusive.
    ///
    /// Equivalent to `isUpperInclusive()`.
    pub fn is_upper_inclusive(&self) -> bool {
        self.upper_inclusive.get()
    }

    /// Sets the lower and upper bounds.
    ///
    /// Equivalent to `setBounds(T lower, T upper, boolean lowerInclusive,
    /// boolean upperInclusive)`: nothing unless both bounds are given; they
    /// must have the same field.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("lower and upper bounds should have the
    /// same field name!")`, the `ClassCastException` of a bound that is not
    /// a `FieldValuePairQueryNode`, and the exceptions of `set`.
    pub fn set_bounds(
        &self,
        lower: Option<QueryNodeRef>,
        upper: Option<QueryNodeRef>,
        lower_inclusive: bool,
        upper_inclusive: bool,
    ) -> Result<()> {
        if let (Some(lower), Some(upper)) = (lower, upper) {
            let lower_field = StringUtils::to_string(bound_field(&lower)?.as_ref());
            let upper_field = StringUtils::to_string(bound_field(&upper)?.as_ref());
            if (upper_field.is_some() || lower_field.is_some())
                && ((upper_field.is_some() && upper_field != lower_field)
                    || lower_field != upper_field)
            {
                return Err(LuceneError::illegal_argument(
                    "lower and upper bounds should have the same field name!",
                ));
            }
            self.lower_inclusive.set(lower_inclusive);
            self.upper_inclusive.set(upper_inclusive);
            self.base.set(vec![lower, upper])?;
        }
        Ok(())
    }

    /// `toQueryString` of `AbstractRangeQueryNode`.
    pub(crate) fn range_to_query_string(
        &self,
        escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        let mut sb = String::new();
        let lower = self.get_lower_bound()?;
        let upper = self.get_upper_bound()?;
        sb.push(if self.lower_inclusive.get() { '[' } else { '{' });
        // A child is never null (Java's "..." for a null bound cannot occur).
        sb.push_str(&lower.to_query_string(escape_syntax_parser)?.to_string());
        sb.push(' ');
        sb.push_str(&upper.to_query_string(escape_syntax_parser)?.to_string());
        sb.push(if self.upper_inclusive.get() { ']' } else { '}' });
        Ok(CharSequence::from(sb))
    }

    /// `toString()` of `AbstractRangeQueryNode`, for the node's class.
    pub(crate) fn range_to_java_string(&self, class: &JavaClass) -> Result<String> {
        let canonical = class.get_canonical_name();
        let node_string = |node: QueryNodeRef| -> Result<String> { node.to_java_string() };
        let mut sb = format!("<{canonical}");
        sb.push_str(&format!(" lowerInclusive={}", self.is_lower_inclusive()));
        sb.push_str(&format!(" upperInclusive={}", self.is_upper_inclusive()));
        sb.push_str(">\n\t");
        sb.push_str(&node_string(self.get_upper_bound()?)?);
        sb.push_str("\n\t");
        sb.push_str(&node_string(self.get_lower_bound()?)?);
        sb.push('\n');
        sb.push_str(&format!("</{canonical}>\n"));
        Ok(sb)
    }
}

impl QueryNode for AbstractRangeQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        &self.base
    }

    fn java_class(&self) -> &'static JavaClass {
        &ABSTRACT_RANGE_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_query_string(
        &self,
        escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        self.range_to_query_string(escape_syntax_parser)
    }

    fn to_java_string(&self) -> Result<String> {
        self.range_to_java_string(&ABSTRACT_RANGE_QUERY_NODE_CLASS)
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.base.clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| {
            self.cloned(self.base.cloned_state(this.clone(), clauses))
        }))
    }

    fn as_fieldable_node(&self) -> Option<&dyn FieldableNode> {
        Some(self)
    }

    fn as_range_query_node(&self) -> Option<&dyn RangeQueryNode> {
        Some(self)
    }

    fn as_abstract_range_query_node(&self) -> Option<&AbstractRangeQueryNode> {
        Some(self)
    }
}

impl FieldableNode for AbstractRangeQueryNode {
    fn get_field(&self) -> Result<Option<CharSequence>> {
        AbstractRangeQueryNode::get_field(self)
    }

    fn set_field(&self, field_name: Option<CharSequence>) -> Result<()> {
        AbstractRangeQueryNode::set_field(self, field_name)
    }
}

impl RangeQueryNode for AbstractRangeQueryNode {
    fn get_lower_bound(&self) -> Result<QueryNodeRef> {
        AbstractRangeQueryNode::get_lower_bound(self)
    }

    fn get_upper_bound(&self) -> Result<QueryNodeRef> {
        AbstractRangeQueryNode::get_upper_bound(self)
    }

    fn is_lower_inclusive(&self) -> bool {
        AbstractRangeQueryNode::is_lower_inclusive(self)
    }

    fn is_upper_inclusive(&self) -> bool {
        AbstractRangeQueryNode::is_upper_inclusive(self)
    }
}
