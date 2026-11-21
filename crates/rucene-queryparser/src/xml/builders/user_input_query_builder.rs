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

//! Port of `UserInputQueryBuilder.java`, of the Java package
//! `org.apache.lucene.queryparser.xml.builders`.

use std::sync::{Arc, Mutex, PoisonError};

use rucene::analysis::Analyzer;
use rucene::error::Result;
use rucene::search::boost_query::BoostQuery;
use rucene::search::query::Query;

use crate::classic::parse_exception::ParseException;
use crate::classic::query_parser::{QueryParser, QueryParserOps};
use crate::jdk::dom::Element;
use crate::xml::dom_utils::DOMUtils;
use crate::xml::parser_exception::ParserException;
use crate::xml::query_builder::QueryBuilder;

/// The parser shared between threads, as Java's `synchronized
/// (unSafeParser)` shares it.
pub type SharedQueryParser = Arc<Mutex<dyn QueryParserOps + Send>>;

/// The factory of [`UserInputQueryBuilder::create_query_parser`]: the
/// override a subclass gives Java's protected `createQueryParser`.
pub type QueryParserFactory = Arc<
    dyn Fn(Option<&str>, Option<Arc<dyn Analyzer>>) -> Result<Box<dyn QueryParserOps + Send>>
        + Send
        + Sync,
>;

/// UserInputQueryBuilder uses 1 of 2 strategies for thread-safe parsing: 1)
/// Synchronizing access to "parse" calls on a previously supplied
/// QueryParser or.. 2) creating a new QueryParser object for each parse
/// request
///
/// Equivalent to
/// `org.apache.lucene.queryparser.xml.builders.UserInputQueryBuilder`.
pub struct UserInputQueryBuilder {
    un_safe_parser: Option<SharedQueryParser>,
    analyzer: Option<Arc<dyn Analyzer>>,
    default_field: Option<String>,
    create_query_parser_override: Option<QueryParserFactory>,
}

impl std::fmt::Debug for UserInputQueryBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserInputQueryBuilder")
            .field("default_field", &self.default_field)
            .finish()
    }
}

impl UserInputQueryBuilder {
    /// This constructor has the disadvantage of not being able to change
    /// choice of default field name
    ///
    /// Equivalent to `new UserInputQueryBuilder(QueryParser parser)`: parser
    /// which has been configured with the desired default field name.
    pub fn with_parser(parser: SharedQueryParser) -> Self {
        Self {
            un_safe_parser: Some(parser),
            analyzer: None,
            default_field: None,
            create_query_parser_override: None,
        }
    }

    /// Equivalent to `new UserInputQueryBuilder(String defaultField,
    /// Analyzer analyzer)`.
    pub fn new(default_field: Option<String>, analyzer: Option<Arc<dyn Analyzer>>) -> Self {
        Self {
            un_safe_parser: None,
            analyzer,
            default_field,
            create_query_parser_override: None,
        }
    }

    /// The builder with an override of the protected `createQueryParser`, as
    /// a subclass of the Java class gives it.
    pub fn with_create_query_parser(mut self, factory: QueryParserFactory) -> Self {
        self.create_query_parser_override = Some(factory);
        self
    }

    /// Method to create a QueryParser - designed to be overridden
    ///
    /// Equivalent to the protected `createQueryParser(String fieldName,
    /// Analyzer analyzer)`: `new QueryParser(fieldName, analyzer)`, or the
    /// override.
    ///
    /// # Errors
    ///
    /// The exceptions of the parser's construction.
    pub fn create_query_parser(
        &self,
        field_name: Option<&str>,
        analyzer: Option<Arc<dyn Analyzer>>,
    ) -> Result<Box<dyn QueryParserOps + Send>> {
        match &self.create_query_parser_override {
            Some(factory) => factory(field_name, analyzer),
            None => Ok(Box::new(QueryParser::new(field_name, analyzer)?)),
        }
    }
}

impl QueryBuilder for UserInputQueryBuilder {
    /// Equivalent to `getQuery(Element e)`: the element's text parsed by the
    /// shared parser (under its lock) or by a new parser of the `fieldName`
    /// attribute (the default field otherwise), always wrapped in a
    /// `BoostQuery` of the `boost` attribute.
    ///
    /// # Errors
    ///
    /// `ParserException(e1.getMessage())` for the classic parser's
    /// `ParseException`.
    fn get_query(&self, e: &Element) -> Result<Arc<dyn Query>> {
        let text = DOMUtils::get_text(e);
        let result = (|| -> Result<Arc<dyn Query>> {
            let q = match &self.un_safe_parser {
                Some(un_safe_parser) => {
                    // synchronize on unsafe parser
                    let mut parser = un_safe_parser
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    parser.parse(&text)?
                }
                None => {
                    let field_name = DOMUtils::get_attribute_string(
                        e,
                        "fieldName",
                        self.default_field.as_deref(),
                    );
                    // Create new parser
                    let mut parser =
                        self.create_query_parser(field_name.as_deref(), self.analyzer.clone())?;
                    parser.parse(&text)?
                }
            };
            let boost = DOMUtils::get_attribute_float(e, "boost", 1.0)?;
            Ok(Arc::new(BoostQuery::new(q, boost)?))
        })();
        result.map_err(|e1| match ParseException::of(&e1) {
            Some(pe) => match pe.get_message() {
                Some(message) => ParserException::with_message(message).error(),
                None => ParserException::new().error(),
            },
            None => e1,
        })
    }
}
