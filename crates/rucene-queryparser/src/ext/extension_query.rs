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

//! The port of `ExtensionQuery.java`, of the Java package
//! `org.apache.lucene.queryparser.ext`.

use crate::classic::query_parser::QueryParserOps;

/// `ExtensionQuery` holds all query components extracted from the
/// original query string like the query field and the extension query
/// string.
///
/// Equivalent to the record `org.apache.lucene.queryparser.ext.ExtensionQuery`:
/// `top_level_parser` is the parser that created this query, `field` the
/// query field (`None` for `null`) and `raw_query_string` the extension
/// query string. Java's record holds a reference to the parser; this one
/// borrows it for the duration of
/// [`ParserExtension::parse`](super::parser_extension::ParserExtension::parse).
pub struct ExtensionQuery<'a> {
    top_level_parser: &'a mut dyn QueryParserOps,
    field: Option<String>,
    raw_query_string: String,
}

impl<'a> ExtensionQuery<'a> {
    /// Creates a new `ExtensionQuery`.
    ///
    /// Equivalent to the canonical constructor `ExtensionQuery(QueryParser,
    /// String, String)`.
    pub fn new(
        top_level_parser: &'a mut dyn QueryParserOps,
        field: Option<String>,
        raw_query_string: String,
    ) -> Self {
        Self {
            top_level_parser,
            field,
            raw_query_string,
        }
    }

    /// Equivalent to the record accessor `topLevelParser()`.
    pub fn top_level_parser(&mut self) -> &mut dyn QueryParserOps {
        &mut *self.top_level_parser
    }

    /// Equivalent to the record accessor `field()`.
    pub fn field(&self) -> Option<&str> {
        self.field.as_deref()
    }

    /// Equivalent to the record accessor `rawQueryString()`.
    pub fn raw_query_string(&self) -> &str {
        &self.raw_query_string
    }
}

impl std::fmt::Debug for ExtensionQuery<'_> {
    /// The record's components but the parser.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtensionQuery")
            .field("field", &self.field)
            .field("raw_query_string", &self.raw_query_string)
            .finish_non_exhaustive()
    }
}
