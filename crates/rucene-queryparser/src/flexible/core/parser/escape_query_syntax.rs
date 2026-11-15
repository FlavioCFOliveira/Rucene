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

//! Port of `EscapeQuerySyntax.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.parser`.

use rucene::error::Result;

use crate::jdk::char_sequence::CharSequence;
use crate::jdk::locale::Locale;

/// A parser needs to implement [`EscapeQuerySyntax`] to allow the
/// QueryNode to escape the queries, when the toQueryString method is called.
///
/// Equivalent to the interface
/// `org.apache.lucene.queryparser.flexible.core.parser.EscapeQuerySyntax`.
pub trait EscapeQuerySyntax: Send + Sync {
    /// Equivalent to `escape(CharSequence text, Locale locale, Type type)`:
    /// the escaped text (`None` for Java's `null`, which a `null` text
    /// gives).
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the implementation.
    fn escape(
        &self,
        text: Option<&CharSequence>,
        locale: &Locale,
        type_: Type,
    ) -> Result<Option<CharSequence>>;
}

/// Type of escaping: String for escaping syntax, NORMAL for escaping
/// reserved words (like AND) in terms
///
/// Equivalent to the enum
/// `org.apache.lucene.queryparser.flexible.core.parser.EscapeQuerySyntax.Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Type {
    /// Java constant `STRING`.
    String,
    /// Java constant `NORMAL`.
    Normal,
}
