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

//! The types declared in `SimpleQQParser.java`, of the Java package
//! `org.apache.lucene.benchmark.quality.utils`.

use std::sync::Arc;

use rucene::analysis::StandardAnalyzer;
use rucene::error::{LuceneError, Result};
use rucene::search::boolean_clause::Occur;
use rucene::search::query::Query;
use rucene::search::BooleanQueryBuilder;
use rucene_queryparser::classic::query_parser::QueryParser;
use rucene_queryparser::classic::query_parser_base;

use crate::jdk::{self, ThreadLocal};
use crate::quality::quality_query::QualityQuery;
use crate::quality::quality_query_parser::QualityQueryParser;

/// Equivalent to `org.apache.lucene.benchmark.quality.utils.SimpleQQParser`
/// (`public class SimpleQQParser implements QualityQueryParser`):
/// simplistic quality query parser. A Lucene query is created by passing the
/// value of the specified QualityQuery name-value pair(s) into a Lucene's
/// QueryParser using StandardAnalyzer.
pub struct SimpleQQParser {
    qq_names: Vec<String>,
    index_field: String,
    query_parser: ThreadLocal<QueryParser>,
}

impl SimpleQQParser {
    /// `SimpleQQParser(String[] qqNames, String indexField)`: `qq_names` are
    /// the name-value pairs of quality query to use for creating the query,
    /// `index_field` the corresponding index field.
    pub fn new(qq_names: Vec<String>, index_field: &str) -> Self {
        SimpleQQParser {
            qq_names,
            index_field: index_field.to_string(),
            query_parser: ThreadLocal::new(),
        }
    }

    /// `SimpleQQParser(String qqName, String indexField)`.
    pub fn with_name(qq_name: &str, index_field: &str) -> Self {
        Self::new(vec![qq_name.to_string()], index_field)
    }
}

impl QualityQueryParser for SimpleQQParser {
    fn parse(&self, qq: &QualityQuery) -> Result<Arc<dyn Query>> {
        let qp = match self.query_parser.get() {
            Some(qp) => qp,
            None => {
                let qp = QueryParser::new(
                    Some(&self.index_field),
                    Some(Arc::new(StandardAnalyzer::new())),
                )?;
                self.query_parser.set(Some(qp)).expect("just set")
            }
        };
        let mut qp = jdk::lock(&qp);
        let mut bq = BooleanQueryBuilder::new();
        for name in &self.qq_names {
            // `QueryParserBase.escape(null)` throws NullPointerException.
            let value = qq.get_value(name).ok_or_else(LuceneError::null_pointer)?;
            bq.add(
                query_parser_base::parse(&mut *qp, &query_parser_base::escape(value))?,
                Occur::SHOULD,
            )?;
        }
        Ok(Arc::new(bq.build()))
    }
}
