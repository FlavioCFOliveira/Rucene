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

//! Port of `RangeQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.nodes`.

use rucene::error::Result;

use super::fieldable_node::FieldableNode;
use super::query_node::QueryNodeRef;
use crate::jdk::class::JavaClass;

/// `org.apache.lucene.queryparser.flexible.core.nodes.RangeQueryNode`.
pub static RANGE_QUERY_NODE_CLASS: JavaClass = JavaClass::interface(
    "org.apache.lucene.queryparser.flexible.core.nodes.RangeQueryNode",
    &[&super::fieldable_node::FIELDABLE_NODE_CLASS],
);

/// This interface should be implemented by a [`QueryNode`](super::query_node::QueryNode)
/// that represents some kind of range query.
///
/// Equivalent to the interface
/// `org.apache.lucene.queryparser.flexible.core.nodes.RangeQueryNode<T
/// extends FieldValuePairQueryNode<?>>`; the bounds are node references (the
/// type parameter is erased, as it is at run time in Java), and a node gives
/// the interface through
/// [`QueryNode::as_range_query_node`](super::query_node::QueryNode::as_range_query_node).
pub trait RangeQueryNode: FieldableNode {
    /// Equivalent to `getLowerBound()`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the class's implementation.
    fn get_lower_bound(&self) -> Result<QueryNodeRef>;

    /// Equivalent to `getUpperBound()`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the class's implementation.
    fn get_upper_bound(&self) -> Result<QueryNodeRef>;

    /// Equivalent to `isLowerInclusive()`.
    fn is_lower_inclusive(&self) -> bool;

    /// Equivalent to `isUpperInclusive()`.
    fn is_upper_inclusive(&self) -> bool;
}
