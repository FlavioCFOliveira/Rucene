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

//! The types declared in `SimpleQueryMaker.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::terms::Term;
use rucene::search::boolean_clause::Occur;
use rucene::search::query::Query;
use rucene::search::term_query::TermQuery;
use rucene::search::BooleanQueryBuilder;
use rucene_queryparser::classic::query_parser::QueryParser;
use rucene_queryparser::classic::query_parser_base;

use crate::by_task::feeds::abstract_query_maker::{AbstractQueryMaker, AbstractQueryMakerBase};
use crate::by_task::feeds::doc_maker::DocMaker;
use crate::by_task::tasks::new_analyzer_task::NewAnalyzerTask;

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.SimpleQueryMaker`
/// (`public class SimpleQueryMaker extends AbstractQueryMaker`): a
/// QueryMaker that makes queries for a collection created using
/// `SingleDocSource`.
#[derive(Debug, Default)]
pub struct SimpleQueryMaker {
    base: AbstractQueryMakerBase,
}

impl SimpleQueryMaker {
    /// `new SimpleQueryMaker()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `SimpleQueryMaker.prepareQueries()`: prepare the queries for this
    /// test. Extending classes can override this method for preparing
    /// different queries.
    ///
    /// # Errors
    ///
    /// If cannot prepare the queries.
    pub(crate) fn prepare_simple_queries(
        base: &AbstractQueryMakerBase,
    ) -> Result<Vec<Arc<dyn Query>>> {
        let config = base
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

        let mut qp = QueryParser::new(Some(DocMaker::BODY_FIELD), Some(anlzr))?;
        let mut qq: Vec<Arc<dyn Query>> = Vec::new();
        let q1: Arc<dyn Query> =
            Arc::new(TermQuery::new(Term::from_text(DocMaker::ID_FIELD, "doc2")));
        qq.push(Arc::clone(&q1));
        let q2: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text(
            DocMaker::BODY_FIELD,
            "simple",
        )));
        qq.push(Arc::clone(&q2));
        let mut bq = BooleanQueryBuilder::new();
        bq.add(q1, Occur::MUST)?;
        bq.add(q2, Occur::MUST)?;
        qq.push(Arc::new(bq.build()));
        for s in [
            "synthetic body",
            "\"synthetic body\"",
            "synthetic text",
            "\"synthetic text\"",
            "\"synthetic text\"~3",
            "zoom*",
            "synth*",
        ] {
            qq.push(query_parser_base::parse(&mut qp, s)?);
        }
        Ok(qq)
    }
}

impl AbstractQueryMaker for SimpleQueryMaker {
    fn aqm_base(&self) -> &AbstractQueryMakerBase {
        &self.base
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.benchmark.byTask.feeds.SimpleQueryMaker"
    }

    fn prepare_queries(&self) -> Result<Vec<Arc<dyn Query>>> {
        Self::prepare_simple_queries(&self.base)
    }
}
