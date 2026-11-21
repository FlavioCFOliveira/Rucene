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

//! Port of `CorePlusExtensionsParser.java`, of the Java package
//! `org.apache.lucene.queryparser.xml`.

use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::error::Result;
use rucene::search::query::Query;
use rucene_queries::spans::span_query::SpanQuery;

use super::builders::fuzzy_like_this_query_builder::FuzzyLikeThisQueryBuilder;
use super::builders::span_query_builder::SpanQueryBuilder;
use super::builders::user_input_query_builder::SharedQueryParser;
use super::core_parser::{CoreParser, CoreParserOps};
use super::core_plus_queries_parser::CorePlusQueriesParser;
use super::query_builder::QueryBuilder;
use crate::jdk::dom::Element;

/// Assembles a QueryBuilder which uses Query objects from Lucene's
/// `sandbox` and `queries` modules in addition to core queries.
///
/// Equivalent to `org.apache.lucene.queryparser.xml.CorePlusExtensionsParser`,
/// which extends `CorePlusQueriesParser` (embedded).
#[derive(Debug)]
pub struct CorePlusExtensionsParser {
    base: CorePlusQueriesParser,
}

impl CorePlusExtensionsParser {
    /// Construct an XML parser that uses a single instance QueryParser for
    /// handling UserQuery tags - all parse operations are synchronized on
    /// this parser
    ///
    /// Equivalent to `new CorePlusExtensionsParser(Analyzer analyzer,
    /// QueryParser parser)`.
    pub fn with_parser(
        analyzer: Option<Arc<dyn Analyzer>>,
        parser: Option<SharedQueryParser>,
    ) -> Self {
        Self::with_all(None, analyzer, parser)
    }

    /// Constructs an XML parser that creates a QueryParser for each UserQuery
    /// request.
    ///
    /// Equivalent to `new CorePlusExtensionsParser(String defaultField,
    /// Analyzer analyzer)`.
    pub fn new(default_field: Option<String>, analyzer: Option<Arc<dyn Analyzer>>) -> Self {
        Self::with_all(default_field, analyzer, None)
    }

    /// Equivalent to the private `CorePlusExtensionsParser(String
    /// defaultField, Analyzer analyzer, QueryParser parser)`: the builders
    /// of `CorePlusQueriesParser`, and `FuzzyLikeThisQuery`.
    fn with_all(
        default_field: Option<String>,
        analyzer: Option<Arc<dyn Analyzer>>,
        parser: Option<SharedQueryParser>,
    ) -> Self {
        let base = CorePlusQueriesParser::with_all(default_field, analyzer.clone(), parser);
        base.core_parser().query_factory.add_builder(
            "FuzzyLikeThisQuery",
            Arc::new(FuzzyLikeThisQueryBuilder::new(analyzer)),
        );
        Self { base }
    }
}

impl QueryBuilder for CorePlusExtensionsParser {
    fn get_query(&self, e: &Element) -> Result<Arc<dyn Query>> {
        self.base.get_query(e)
    }
}

impl SpanQueryBuilder for CorePlusExtensionsParser {
    fn get_span_query(&self, e: &Element) -> Result<Arc<dyn SpanQuery>> {
        self.base.get_span_query(e)
    }
}

impl CoreParserOps for CorePlusExtensionsParser {
    fn core_parser(&self) -> &CoreParser {
        self.base.core_parser()
    }
}
