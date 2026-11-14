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

//! The port of `ExtendableQueryParser.java`, of the Java package
//! `org.apache.lucene.queryparser.ext`.

use std::sync::{Arc, LazyLock};

use rucene::analysis::Analyzer;
use rucene::error::Result;
use rucene::search::Query;
use rucene::util::query_builder::{QueryBuilder, QueryBuilderOps};

use super::extension_query::ExtensionQuery;
use super::extensions::Extensions;
use crate::charstream::char_stream::CharStream;
use crate::classic::query_parser::{self, QueryParser, QueryParserOps};
use crate::classic::query_parser_base::{self as base, QueryParserBase, QueryParserBaseOps};
use crate::classic::query_parser_constants::QueryParserConstants;

/// The default extension instance, shared by the parsers created without
/// one.
///
/// Equivalent to the private static `DEFAULT_EXTENSION`.
static DEFAULT_EXTENSION: LazyLock<Arc<Extensions>> = LazyLock::new(|| Arc::new(Extensions::new()));

/// The `ExtendableQueryParser` enables arbitrary query parser extension
/// based on a customizable field naming scheme. The lucene query syntax
/// allows implicit and explicit field definitions as query prefix followed
/// by a colon (`:`) character. The `ExtendableQueryParser` allows to
/// encode extension keys into the field symbol associated with a registered
/// instance of `ParserExtension`. A customizable separation character
/// separates the extension key from the actual field symbol. The
/// `ExtendableQueryParser` splits (see
/// [`Extensions::split_extension_field`]) the extension key from the field
/// symbol and tries to resolve the associated `ParserExtension`. If the
/// parser can't resolve the key or the field token does not contain a
/// separation character, `ExtendableQueryParser` yields the same behavior as
/// its super class `QueryParser`. Otherwise, if the key is associated with
/// a `ParserExtension` instance, the parser builds an instance of
/// `ExtensionQuery` to be processed by
/// [`ParserExtension::parse`](super::parser_extension::ParserExtension::parse).
/// If an extension field does not contain a field part the default field of
/// the query parser will be used.
///
/// Equivalent to `org.apache.lucene.queryparser.ext.ExtendableQueryParser`.
#[derive(Debug)]
pub struct ExtendableQueryParser {
    /// The `QueryParser` state Java's class extends.
    parser: QueryParser,
    default_field: Option<String>,
    extensions: Arc<Extensions>,
}

impl ExtendableQueryParser {
    /// Creates a new `ExtendableQueryParser` instance.
    ///
    /// Equivalent to `new ExtendableQueryParser(String, Analyzer)`.
    ///
    /// # Errors
    ///
    /// As [`QueryParser::new`].
    pub fn new(f: Option<&str>, a: Option<Arc<dyn Analyzer>>) -> Result<Self> {
        Self::with_extensions(f, a, Arc::clone(&DEFAULT_EXTENSION))
    }

    /// Creates a new `ExtendableQueryParser` instance.
    ///
    /// Equivalent to `new ExtendableQueryParser(String, Analyzer,
    /// Extensions)`.
    ///
    /// # Errors
    ///
    /// As [`QueryParser::new`].
    pub fn with_extensions(
        f: Option<&str>,
        a: Option<Arc<dyn Analyzer>>,
        ext: Arc<Extensions>,
    ) -> Result<Self> {
        Ok(Self {
            parser: QueryParser::new(f, a)?,
            default_field: f.map(str::to_string),
            extensions: ext,
        })
    }

    /// Returns the extension field delimiter character.
    ///
    /// Equivalent to `getExtensionFieldDelimiter()`.
    pub fn get_extension_field_delimiter(&self) -> char {
        self.extensions.get_extension_field_delimiter()
    }
}

impl QueryParserConstants for ExtendableQueryParser {}

impl QueryBuilderOps for ExtendableQueryParser {
    fn query_builder(&self) -> &QueryBuilder {
        &self.parser.base.builder
    }
}

impl QueryParserOps for ExtendableQueryParser {
    fn query_parser(&self) -> &QueryParser {
        &self.parser
    }

    fn query_parser_mut(&mut self) -> &mut QueryParser {
        &mut self.parser
    }
}

impl QueryParserBaseOps for ExtendableQueryParser {
    fn query_parser_base(&self) -> &QueryParserBase {
        &self.parser.base
    }

    fn query_parser_base_mut(&mut self) -> &mut QueryParserBase {
        &mut self.parser.base
    }

    fn re_init(&mut self, stream: Box<dyn CharStream>) {
        query_parser::re_init(self, stream);
    }

    fn top_level_query(&mut self, field: Option<&str>) -> Result<Option<Arc<dyn Query>>> {
        query_parser::top_level_query(self, field.map(str::to_string))
    }

    fn set_auto_generate_phrase_queries(&mut self, value: bool) -> Result<()> {
        query_parser::set_auto_generate_phrase_queries(self, value)
    }

    fn get_field_query(
        &mut self,
        field: Option<&str>,
        query_text: &str,
        quoted: bool,
    ) -> Result<Option<Arc<dyn Query>>> {
        let split_extension_field = self
            .extensions
            .split_extension_field(self.default_field.as_deref(), field)?;
        let extension = self
            .extensions
            .get_extension(split_extension_field.cud().as_deref());
        if let Some(extension) = extension {
            return extension.parse(ExtensionQuery::new(
                self,
                split_extension_field.cur().clone(),
                query_text.to_string(),
            ));
        }
        base::get_field_query(self, field, query_text, quoted)
    }
}
