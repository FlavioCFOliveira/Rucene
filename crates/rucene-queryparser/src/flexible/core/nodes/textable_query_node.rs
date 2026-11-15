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

//! Port of `TextableQueryNode.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.nodes`.

use crate::jdk::char_sequence::CharSequence;
use crate::jdk::class::JavaClass;

/// `org.apache.lucene.queryparser.flexible.core.nodes.TextableQueryNode`.
pub static TEXTABLE_QUERY_NODE_CLASS: JavaClass = JavaClass::interface(
    "org.apache.lucene.queryparser.flexible.core.nodes.TextableQueryNode",
    &[],
);

/// Interface for a node that has text as a [`CharSequence`].
///
/// Equivalent to the interface
/// `org.apache.lucene.queryparser.flexible.core.nodes.TextableQueryNode`; a
/// node gives it through
/// [`QueryNode::as_textable_query_node`](super::query_node::QueryNode::as_textable_query_node).
pub trait TextableQueryNode: Send + Sync {
    /// Equivalent to `getText()`; `None` for Java's `null`.
    fn get_text(&self) -> Option<CharSequence>;

    /// Equivalent to `setText(CharSequence text)`.
    fn set_text(&self, text: Option<CharSequence>);
}
