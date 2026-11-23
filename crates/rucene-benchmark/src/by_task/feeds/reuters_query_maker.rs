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

//! The types declared in `ReutersQueryMaker.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::error::Result;
use rucene::index::terms::Term;
use rucene::search::query::Query;
use rucene::search::wildcard_query::WildcardQuery;
use rucene_queries::spans::span_first_query::SpanFirstQuery;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_queryparser::classic::query_parser::QueryParser;
use rucene_queryparser::classic::query_parser_base;

use crate::by_task::feeds::abstract_query_maker::{AbstractQueryMaker, AbstractQueryMakerBase};
use crate::by_task::feeds::doc_maker::DocMaker;
use crate::by_task::tasks::new_analyzer_task::NewAnalyzerTask;

/// `STANDARD_QUERIES`.
const STANDARD_QUERIES: [&str; 11] = [
    // Start with some short queries
    "Salomon",
    "Comex",
    "night trading",
    "Japan Sony",
    // Try some Phrase Queries
    "\"Sony Japan\"",
    "\"food needs\"~3",
    "\"World Bank\"^2 AND Nigeria",
    "\"World Bank\" -Nigeria",
    "\"Ford Credit\"~5",
    // Try some longer queries
    "airline Europe Canada destination",
    concat!(
        "Long term pressure by trade ",
        "ministers is necessary if the current Uruguay round of talks on ",
        "the General Agreement on Trade and Tariffs (GATT) is to ",
        "succeed"
    ),
];

/// An element of the `List<Object>` `createQueries` takes: a query
/// expression or a prebuilt query.
pub(crate) enum QueryItem {
    /// A `String` to parse.
    Text(String),
    /// A prebuilt `Query`.
    Query(Arc<dyn Query>),
}

/// The span queries `getPrebuiltQueries` of the Reuters and Enwiki makers
/// share; `wildcard` is the last query.
pub(crate) fn span_prebuilt_queries(
    field: &str,
    wildcard: Arc<dyn Query>,
) -> Result<Vec<Arc<dyn Query>>> {
    let term = |text: &str| Term::from_text(field.to_string(), text);
    // be wary of unanalyzed text
    Ok(vec![
        Arc::new(SpanFirstQuery::new(
            Arc::new(SpanTermQuery::new(term("ford"))),
            5,
        )),
        Arc::new(SpanNearQuery::new(
            vec![
                Arc::new(SpanTermQuery::new(term("night"))) as Arc<dyn SpanQuery>,
                Arc::new(SpanTermQuery::new(term("trading"))),
            ],
            4,
            false,
        )?),
        Arc::new(SpanNearQuery::new(
            vec![
                Arc::new(SpanFirstQuery::new(
                    Arc::new(SpanTermQuery::new(term("ford"))),
                    10,
                )) as Arc<dyn SpanQuery>,
                Arc::new(SpanTermQuery::new(term("credit"))),
            ],
            10,
            false,
        )?),
        wildcard,
    ])
}

/// `createQueries(List<Object> qs, Analyzer a)`: parse the strings
/// containing Lucene queries; an expression that fails to parse is printed
/// and skipped.
///
/// # Errors
///
/// The exception of building the parser.
pub(crate) fn create_queries(
    qs: Vec<QueryItem>,
    a: Arc<dyn Analyzer>,
) -> Result<Vec<Arc<dyn Query>>> {
    let mut qp = QueryParser::new(Some(DocMaker::BODY_FIELD), Some(a))?;
    let mut queries: Vec<Arc<dyn Query>> = Vec::new();
    for query in qs {
        let q = match query {
            QueryItem::Text(s) => query_parser_base::parse(&mut qp, &s),
            QueryItem::Query(q) => Ok(q),
        };
        match q {
            Ok(q) => queries.push(q),
            Err(e) => eprintln!("{e}"),
        }
    }
    Ok(queries)
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.ReutersQueryMaker`
/// (`public class ReutersQueryMaker extends AbstractQueryMaker`): a
/// QueryMaker that makes queries devised manually (by Grant Ingersoll) for
/// searching in the Reuters collection.
#[derive(Debug, Default)]
pub struct ReutersQueryMaker {
    base: AbstractQueryMakerBase,
}

impl ReutersQueryMaker {
    /// `new ReutersQueryMaker()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `getPrebuiltQueries(String field)`.
    fn get_prebuilt_queries(field: &str) -> Result<Vec<Arc<dyn Query>>> {
        span_prebuilt_queries(
            field,
            Arc::new(WildcardQuery::new(Term::from_text(
                field.to_string(),
                "fo*",
            ))?),
        )
    }
}

impl AbstractQueryMaker for ReutersQueryMaker {
    fn aqm_base(&self) -> &AbstractQueryMakerBase {
        &self.base
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.benchmark.byTask.feeds.ReutersQueryMaker"
    }

    fn prepare_queries(&self) -> Result<Vec<Arc<dyn Query>>> {
        let config = self
            .base
            .config()
            .ok_or_else(rucene::error::LuceneError::null_pointer)?;
        // analyzer (default is standard analyzer)
        let anlzr = NewAnalyzerTask::create_analyzer(
            &config
                .get_str(
                    "analyzer",
                    Some("org.apache.lucene.analysis.standard.StandardAnalyzer"),
                )?
                .unwrap_or_default(),
        )?;

        let mut query_list: Vec<QueryItem> = STANDARD_QUERIES
            .iter()
            .map(|s| QueryItem::Text(s.to_string()))
            .collect();
        query_list.extend(
            Self::get_prebuilt_queries(DocMaker::BODY_FIELD)?
                .into_iter()
                .map(QueryItem::Query),
        );
        create_queries(query_list, anlzr)
    }
}
