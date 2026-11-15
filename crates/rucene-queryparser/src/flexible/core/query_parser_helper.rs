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

//! Port of `QueryParserHelper.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use super::builders::query_builder::QueryBuilder;
use super::config::query_config_handler::QueryConfigHandler;
use super::nodes::query_node::JavaObjectRef;
use super::parser::syntax_parser::SyntaxParser;
use super::processors::query_node_processor::QueryNodeProcessor;
use crate::jdk::char_sequence::CharSequence;

/// This class is a helper for the query parser framework, it does all the
/// three query parser phrases at once: text parsing, query processing and
/// query building.
///
/// It contains methods that allows the user to change the implementation
/// used on the three phases.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.core.QueryParserHelper`;
/// `None` stands for Java's `null` in its fields.
#[derive(Clone)]
pub struct QueryParserHelper {
    processor: Option<Arc<dyn QueryNodeProcessor>>,
    syntax_parser: Option<Arc<dyn SyntaxParser>>,
    builder: Option<Arc<dyn QueryBuilder>>,
    config: Option<Arc<QueryConfigHandler>>,
}

impl std::fmt::Debug for QueryParserHelper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryParserHelper")
            .field("config", &self.config)
            .finish()
    }
}

impl QueryParserHelper {
    /// Creates a query parser helper object using the specified
    /// configuration, text parser, processor and builder.
    ///
    /// Equivalent to `new QueryParserHelper(QueryConfigHandler
    /// queryConfigHandler, SyntaxParser syntaxParser, QueryNodeProcessor
    /// processor, QueryBuilder builder)`: the processor, when there is one,
    /// gets the configuration handler.
    pub fn new(
        query_config_handler: Option<Arc<QueryConfigHandler>>,
        syntax_parser: Option<Arc<dyn SyntaxParser>>,
        processor: Option<Arc<dyn QueryNodeProcessor>>,
        builder: Option<Arc<dyn QueryBuilder>>,
    ) -> Self {
        if let Some(processor) = &processor {
            processor.set_query_config_handler(query_config_handler.clone());
        }
        Self {
            processor,
            syntax_parser,
            builder,
            config: query_config_handler,
        }
    }

    /// Returns the processor object used to process the query node tree, it
    /// returns `null` if no processor is used.
    ///
    /// Equivalent to `getQueryNodeProcessor()`.
    pub fn get_query_node_processor(&self) -> Option<Arc<dyn QueryNodeProcessor>> {
        self.processor.clone()
    }

    /// Sets the processor that will be used to process the query node tree.
    /// If there is any [`QueryConfigHandler`] returned by
    /// [`get_query_config_handler`](Self::get_query_config_handler), it will
    /// be set on the processor. The argument can be `null`, which means that
    /// no processor will be used to process the query node tree.
    ///
    /// Equivalent to `setQueryNodeProcessor(QueryNodeProcessor processor)`.
    ///
    /// # Errors
    ///
    /// For `None`, the `NullPointerException` Java throws calling
    /// `setQueryConfigHandler` on the `null` it has just stored.
    pub fn set_query_node_processor(
        &mut self,
        processor: Option<Arc<dyn QueryNodeProcessor>>,
    ) -> Result<()> {
        self.processor = processor;
        let processor = self
            .processor
            .as_ref()
            .ok_or_else(LuceneError::null_pointer)?;
        processor.set_query_config_handler(self.get_query_config_handler());
        Ok(())
    }

    /// Sets the text parser that will be used to parse the query string, it
    /// cannot be `null`.
    ///
    /// Equivalent to `setSyntaxParser(SyntaxParser syntaxParser)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("textParser should not be null!")` for
    /// `None`.
    pub fn set_syntax_parser(
        &mut self,
        syntax_parser: Option<Arc<dyn SyntaxParser>>,
    ) -> Result<()> {
        let syntax_parser = syntax_parser
            .ok_or_else(|| LuceneError::illegal_argument("textParser should not be null!"))?;
        self.syntax_parser = Some(syntax_parser);
        Ok(())
    }

    /// The query builder that will be used to build an object from the query
    /// node tree. It cannot be `null`.
    ///
    /// Equivalent to `setQueryBuilder(QueryBuilder queryBuilder)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("queryBuilder should not be null!")` for
    /// `None`.
    pub fn set_query_builder(
        &mut self,
        query_builder: Option<Arc<dyn QueryBuilder>>,
    ) -> Result<()> {
        let query_builder = query_builder
            .ok_or_else(|| LuceneError::illegal_argument("queryBuilder should not be null!"))?;
        self.builder = Some(query_builder);
        Ok(())
    }

    /// Returns the query configuration handler, which is used during the
    /// query node tree processing. It can be `null`.
    ///
    /// Equivalent to `getQueryConfigHandler()`.
    pub fn get_query_config_handler(&self) -> Option<Arc<QueryConfigHandler>> {
        self.config.clone()
    }

    /// Returns the query builder used to build a object from the query node
    /// tree. The object produced by this builder is returned by
    /// [`parse`](Self::parse).
    ///
    /// Equivalent to `getQueryBuilder()`.
    pub fn get_query_builder(&self) -> Option<Arc<dyn QueryBuilder>> {
        self.builder.clone()
    }

    /// Returns the text parser used to build a query node tree from a query
    /// string. The default text parser instance returned by this method is a
    /// [`SyntaxParser`].
    ///
    /// Equivalent to `getSyntaxParser()`.
    pub fn get_syntax_parser(&self) -> Option<Arc<dyn SyntaxParser>> {
        self.syntax_parser.clone()
    }

    /// Sets the query configuration handler that will be used during query
    /// processing. It can be `null`. It's also set to the processor returned
    /// by [`get_query_node_processor`](Self::get_query_node_processor).
    ///
    /// Equivalent to `setQueryConfigHandler(QueryConfigHandler config)`.
    pub fn set_query_config_handler(&mut self, config: Option<Arc<QueryConfigHandler>>) {
        self.config = config.clone();
        if let Some(processor) = self.get_query_node_processor() {
            processor.set_query_config_handler(config);
        }
    }

    /// Parses a query string to an object, usually some query object.
    ///
    /// In this method the three phases are executed: 1st - the query string
    /// is parsed using the text parser returned by
    /// [`get_syntax_parser`](Self::get_syntax_parser), the result is a query
    /// node tree; 2nd - the query node tree is processed by the processor
    /// returned by [`get_query_node_processor`](Self::get_query_node_processor);
    /// 3th - a object is built from the query node tree using the builder
    /// returned by [`get_query_builder`](Self::get_query_builder).
    ///
    /// Equivalent to `parse(String query, String defaultField)`: the object
    /// built (`None` for Java's `null`).
    ///
    /// # Errors
    ///
    /// A `QueryNodeException` if something wrong happens along the three
    /// phases, and the `NullPointerException` of a missing syntax parser or
    /// builder.
    pub fn parse(&self, query: &str, default_field: Option<&str>) -> Result<Option<JavaObjectRef>> {
        let syntax_parser = self
            .get_syntax_parser()
            .ok_or_else(LuceneError::null_pointer)?;
        let default_field = default_field.map(CharSequence::from);
        let mut query_tree =
            syntax_parser.parse(&CharSequence::from(query), default_field.as_ref())?;
        if let Some(processor) = self.get_query_node_processor() {
            query_tree = processor.process(query_tree)?;
        }
        let builder = self
            .get_query_builder()
            .ok_or_else(LuceneError::null_pointer)?;
        builder.build(&query_tree)
    }
}
