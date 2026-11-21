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

//! Port of `PointRangeQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.nodes`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, Result};

use super::abstract_range_query_node::{AbstractRangeQueryNode, ABSTRACT_RANGE_QUERY_NODE_CLASS};
use super::point_query_node::PointQueryNode;
use crate::flexible::core::nodes::fieldable_node::FieldableNode;
use crate::flexible::core::nodes::query_node::{QueryNode, QueryNodeRef};
use crate::flexible::core::nodes::query_node_impl::{Mutable, QueryNodeImpl};
use crate::flexible::core::nodes::range_query_node::RangeQueryNode;
use crate::flexible::core::parser::escape_query_syntax::EscapeQuerySyntax;
use crate::flexible::standard::config::points_config::PointsConfig;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::{JavaClass, DOUBLE, FLOAT, INTEGER, LONG};
use crate::jdk::format::JavaNumber;

/// `org.apache.lucene.queryparser.flexible.standard.nodes.PointRangeQueryNode`.
pub static POINT_RANGE_QUERY_NODE_CLASS: JavaClass = JavaClass::class(
    "org.apache.lucene.queryparser.flexible.standard.nodes.PointRangeQueryNode",
    Some(&ABSTRACT_RANGE_QUERY_NODE_CLASS),
    &[],
);

/// The class of a `Number`, `getClass()`.
fn number_class(n: &JavaNumber) -> &'static JavaClass {
    match n {
        JavaNumber::Integer(_) => &INTEGER,
        JavaNumber::Long(_) => &LONG,
        JavaNumber::Float(_) => &FLOAT,
        JavaNumber::Double(_) => &DOUBLE,
    }
}

/// This query node represents a range query composed by [`PointQueryNode`]
/// bounds, which means the bound values are `Number`s.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.standard.nodes.PointRangeQueryNode`,
/// which extends `AbstractRangeQueryNode<PointQueryNode>` (embedded).
#[derive(Debug)]
pub struct PointRangeQueryNode {
    base: AbstractRangeQueryNode,
    numeric_config: Mutable<Option<Arc<PointsConfig>>>,
}

impl PointRangeQueryNode {
    /// Constructs a [`PointRangeQueryNode`] object using the given
    /// [`PointQueryNode`] as its bounds and [`PointsConfig`].
    ///
    /// Equivalent to `new PointRangeQueryNode(PointQueryNode lower,
    /// PointQueryNode upper, boolean lowerInclusive, boolean upperInclusive,
    /// PointsConfig numericConfig)`.
    ///
    /// # Errors
    ///
    /// The exceptions of [`set_bounds`](Self::set_bounds).
    pub fn new(
        lower: Option<Arc<PointQueryNode>>,
        upper: Option<Arc<PointQueryNode>>,
        lower_inclusive: bool,
        upper_inclusive: bool,
        numeric_config: Option<Arc<PointsConfig>>,
    ) -> Result<Arc<Self>> {
        let node = Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: AbstractRangeQueryNode::fields(QueryNodeImpl::new(this.clone())),
            numeric_config: Mutable::new(None),
        });
        node.base.init();
        node.set_bounds(
            lower,
            upper,
            lower_inclusive,
            upper_inclusive,
            numeric_config,
        )?;
        Ok(node)
    }

    /// Sets the upper and lower bounds of this range query node and the
    /// [`PointsConfig`] associated with these bounds.
    ///
    /// Equivalent to `setBounds(PointQueryNode lower, PointQueryNode upper,
    /// boolean lowerInclusive, boolean upperInclusive, PointsConfig
    /// pointsConfig)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("pointsConfig must not be null!")`, the
    /// `IllegalArgumentException` of a bound value whose class is not the
    /// configuration's type, and the exceptions of
    /// [`AbstractRangeQueryNode::set_bounds`].
    pub fn set_bounds(
        &self,
        lower: Option<Arc<PointQueryNode>>,
        upper: Option<Arc<PointQueryNode>>,
        lower_inclusive: bool,
        upper_inclusive: bool,
        points_config: Option<Arc<PointsConfig>>,
    ) -> Result<()> {
        let points_config = points_config
            .ok_or_else(|| LuceneError::illegal_argument("pointsConfig must not be null!"))?;
        let lower_number_type = lower
            .as_ref()
            .and_then(|l| l.get_value())
            .map(|v| number_class(&v));
        let upper_number_type = upper
            .as_ref()
            .and_then(|u| u.get_value())
            .map(|v| number_class(&v));
        if let Some(lower_number_type) = lower_number_type {
            if lower_number_type != points_config.get_type() {
                return Err(LuceneError::illegal_argument(format!(
                    "lower value's type should be the same as numericConfig type: {} != {}",
                    lower_number_type.to_java_string(),
                    points_config.get_type().to_java_string()
                )));
            }
        }
        if let Some(upper_number_type) = upper_number_type {
            if upper_number_type != points_config.get_type() {
                return Err(LuceneError::illegal_argument(format!(
                    "upper value's type should be the same as numericConfig type: {} != {}",
                    upper_number_type.to_java_string(),
                    points_config.get_type().to_java_string()
                )));
            }
        }
        self.base.set_bounds(
            lower.map(|l| l as QueryNodeRef),
            upper.map(|u| u as QueryNodeRef),
            lower_inclusive,
            upper_inclusive,
        )?;
        self.numeric_config.set(Some(points_config));
        Ok(())
    }

    /// Returns the [`PointsConfig`] associated with the lower and upper
    /// bounds.
    ///
    /// Equivalent to `getPointsConfig()`.
    pub fn get_points_config(&self) -> Option<Arc<PointsConfig>> {
        self.numeric_config.get()
    }

    /// Writes the public field `numericConfig`.
    pub fn set_numeric_config(&self, numeric_config: Option<Arc<PointsConfig>>) {
        self.numeric_config.set(numeric_config);
    }

    /// The `AbstractRangeQueryNode` this node extends.
    pub fn abstract_range_query_node(&self) -> &AbstractRangeQueryNode {
        &self.base
    }
}

impl QueryNode for PointRangeQueryNode {
    fn query_node_impl(&self) -> &QueryNodeImpl {
        self.base.query_node_impl()
    }

    fn java_class(&self) -> &'static JavaClass {
        &POINT_RANGE_QUERY_NODE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_query_string(
        &self,
        escape_syntax_parser: &dyn EscapeQuerySyntax,
    ) -> Result<CharSequence> {
        self.base.range_to_query_string(escape_syntax_parser)
    }

    /// Equivalent to `PointRangeQueryNode.toString()`.
    fn to_java_string(&self) -> Result<String> {
        let numeric_config = self
            .numeric_config
            .get()
            .ok_or_else(LuceneError::null_pointer)?;
        let mut sb = String::from("<pointRange lowerInclusive='");
        sb.push_str(&self.base.is_lower_inclusive().to_string());
        sb.push_str("' upperInclusive='");
        sb.push_str(&self.base.is_upper_inclusive().to_string());
        sb.push_str("' type='");
        sb.push_str(numeric_config.get_type().get_simple_name());
        sb.push_str("'>\n");
        sb.push_str(&self.base.get_lower_bound()?.to_java_string()?);
        sb.push('\n');
        sb.push_str(&self.base.get_upper_bound()?.to_java_string()?);
        sb.push('\n');
        sb.push_str("</pointRange>");
        Ok(sb)
    }

    fn clone_tree(&self) -> Result<QueryNodeRef> {
        let clauses = self.query_node_impl().clone_children()?;
        Ok(Arc::new_cyclic(|this: &Weak<Self>| Self {
            base: self
                .base
                .cloned(self.query_node_impl().cloned_state(this.clone(), clauses)),
            numeric_config: Mutable::new(self.numeric_config.get()),
        }))
    }

    fn as_fieldable_node(&self) -> Option<&dyn FieldableNode> {
        Some(&self.base)
    }

    fn as_range_query_node(&self) -> Option<&dyn RangeQueryNode> {
        Some(&self.base)
    }

    fn as_abstract_range_query_node(&self) -> Option<&AbstractRangeQueryNode> {
        Some(&self.base)
    }
}
