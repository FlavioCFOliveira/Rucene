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

//! Port of `QueryNodeOperation.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.util`.

use rucene::error::{LuceneError, Result};

use crate::flexible::core::nodes::and_query_node::{AndQueryNode, AND_QUERY_NODE_CLASS};
use crate::flexible::core::nodes::query_node::{instance_of, QueryNodeRef};
use crate::flexible::core::query_node_error::QueryNodeError;

/// Allow joining 2 QueryNode Trees, into one.
///
/// Equivalent to the final class
/// `org.apache.lucene.queryparser.flexible.core.util.QueryNodeOperation`.
#[derive(Debug)]
pub struct QueryNodeOperation {
    _no_instances: (),
}

/// Equivalent to the private nested enum
/// `org.apache.lucene.queryparser.flexible.core.util.QueryNodeOperation.ANDOperation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ANDOperation {
    /// Java constant `BOTH`.
    Both,
    /// Java constant `Q1`.
    Q1,
    /// Java constant `Q2`.
    Q2,
    /// Java constant `NONE`.
    None,
}

impl QueryNodeOperation {
    /// perform a logical and of 2 QueryNode trees. if q1 and q2 are
    /// ANDQueryNode nodes it uses head Node from q1 and adds the children of
    /// q2 to q1 if q1 is a AND node and q2 is not, add q2 as a child of the
    /// head node of q1 if q2 is a AND node and q1 is not, add q1 as a child
    /// of the head node of q2 if q1 and q2 are not ANDQueryNode nodes, create
    /// a AND node and make q1 and q2 children of that node if q1 or q2 is
    /// null it returns the not null node if q1 = q2 = null it returns null
    ///
    /// Equivalent to `QueryNodeOperation.logicalAnd(QueryNode q1, QueryNode
    /// q2)`.
    ///
    /// # Errors
    ///
    /// The `QueryNodeError` wrapping a `CloneNotSupportedException` of
    /// `cloneTree`, and the exceptions of adding a child.
    pub fn logical_and(
        q1: Option<QueryNodeRef>,
        q2: Option<QueryNodeRef>,
    ) -> Result<Option<QueryNodeRef>> {
        let q1 = match q1 {
            None => return Ok(q2),
            Some(q1) => q1,
        };
        let q2 = match q2 {
            None => return Ok(Some(q1)),
            Some(q2) => q2,
        };
        let q1_and = instance_of(q1.as_ref(), &AND_QUERY_NODE_CLASS);
        let q2_and = instance_of(q2.as_ref(), &AND_QUERY_NODE_CLASS);
        let op = if q1_and && q2_and {
            ANDOperation::Both
        } else if q1_and {
            ANDOperation::Q1
        } else if q2_and {
            ANDOperation::Q2
        } else {
            ANDOperation::None
        };
        let clone_tree = |q: &QueryNodeRef| -> Result<QueryNodeRef> {
            q.clone_tree().map_err(|e| {
                if is_clone_not_supported(&e) {
                    QueryNodeError::with_cause(e).error()
                } else {
                    e
                }
            })
        };
        let result: QueryNodeRef = match op {
            ANDOperation::None => {
                let children = vec![clone_tree(&q1)?, clone_tree(&q2)?];
                AndQueryNode::new(Some(children))?
            }
            ANDOperation::Q1 => {
                let result = clone_tree(&q1)?;
                result.add(clone_tree(&q2)?)?;
                result
            }
            ANDOperation::Q2 => {
                let result = clone_tree(&q2)?;
                result.add(clone_tree(&q1)?)?;
                result
            }
            ANDOperation::Both => {
                let result = clone_tree(&q1)?;
                let children = clone_tree(&q2)?
                    .get_children()
                    .ok_or_else(LuceneError::null_pointer)?;
                result.add_all(children)?;
                result
            }
        };
        Ok(Some(result))
    }
}

/// Whether `error` is a `java.lang.CloneNotSupportedException`.
fn is_clone_not_supported(error: &LuceneError) -> bool {
    matches!(
        error.kind(),
        rucene::error::LuceneErrorKind::Unchecked { class, .. } if *class == "java.lang.CloneNotSupportedException"
    )
}
