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

//! Port of `CorePlusQueriesParser.java`, of the Java package
//! `org.apache.lucene.queryparser.xml`.

use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::error::Result;
use rucene::search::query::Query;
use rucene_queries::spans::span_query::SpanQuery;

use super::builders::like_this_query_builder::LikeThisQueryBuilder;
use super::builders::span_query_builder::SpanQueryBuilder;
use super::builders::user_input_query_builder::SharedQueryParser;
use super::core_parser::{CoreParser, CoreParserOps};
use super::query_builder::QueryBuilder;
use crate::jdk::dom::Element;

/// Assembles a QueryBuilder which uses Query objects from Lucene's `queries`
/// module in addition to core queries.
///
/// Equivalent to `org.apache.lucene.queryparser.xml.CorePlusQueriesParser`,
/// which extends `CoreParser` (embedded).
#[derive(Debug)]
pub struct CorePlusQueriesParser {
    base: CoreParser,
}

impl CorePlusQueriesParser {
    /// Construct an XML parser that uses a single instance QueryParser for
    /// handling UserQuery tags - all parse operations are synchronized on
    /// this parser
    ///
    /// Equivalent to `new CorePlusQueriesParser(Analyzer analyzer,
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
    /// Equivalent to `new CorePlusQueriesParser(String defaultField,
    /// Analyzer analyzer)`.
    pub fn new(default_field: Option<String>, analyzer: Option<Arc<dyn Analyzer>>) -> Self {
        Self::with_all(default_field, analyzer, None)
    }

    /// Equivalent to the protected `CorePlusQueriesParser(String
    /// defaultField, Analyzer analyzer, QueryParser parser)`: the core
    /// builders, and `LikeThisQuery` over the field `contents`.
    pub fn with_all(
        default_field: Option<String>,
        analyzer: Option<Arc<dyn Analyzer>>,
        parser: Option<SharedQueryParser>,
    ) -> Self {
        let base = CoreParser::with_all(default_field, analyzer.clone(), parser);
        let fields = vec!["contents".to_string()];
        base.query_factory.add_builder(
            "LikeThisQuery",
            Arc::new(LikeThisQueryBuilder::new(analyzer, Some(fields))),
        );
        Self { base }
    }

    /// The `CoreParser` this parser extends.
    pub fn core_parser(&self) -> &CoreParser {
        &self.base
    }
}

impl QueryBuilder for CorePlusQueriesParser {
    fn get_query(&self, e: &Element) -> Result<Arc<dyn Query>> {
        self.base.get_query(e)
    }
}

impl SpanQueryBuilder for CorePlusQueriesParser {
    fn get_span_query(&self, e: &Element) -> Result<Arc<dyn SpanQuery>> {
        self.base.get_span_query(e)
    }
}

impl CoreParserOps for CorePlusQueriesParser {
    fn core_parser(&self) -> &CoreParser {
        &self.base
    }
}
