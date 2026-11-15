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

//! Port of `SyntaxParser.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.parser`.

use rucene::error::Result;

use crate::flexible::core::nodes::query_node::QueryNodeRef;
use crate::jdk::char_sequence::CharSequence;

/// A parser needs to implement [`SyntaxParser`] interface
///
/// Equivalent to the interface
/// `org.apache.lucene.queryparser.flexible.core.parser.SyntaxParser`.
pub trait SyntaxParser: Send + Sync {
    /// Equivalent to `parse(CharSequence query, CharSequence field)`: the
    /// query node tree of `query`, `field` being the default field (`None`
    /// for Java's `null`).
    ///
    /// # Errors
    ///
    /// A `QueryNodeParseException`.
    fn parse(&self, query: &CharSequence, field: Option<&CharSequence>) -> Result<QueryNodeRef>;
}
