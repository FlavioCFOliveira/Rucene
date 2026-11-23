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

//! The types declared in `QualityBenchmark.java`, of the Java package
//! `org.apache.lucene.benchmark.quality`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::{query_to_string, Query};
use rucene::search::top_docs::TopDocs;

use crate::jdk::{self, PrintWriter};
use crate::quality::judge::Judge;
use crate::quality::quality_query::QualityQuery;
use crate::quality::quality_query_parser::QualityQueryParser;
use crate::quality::quality_stats::QualityStats;
use crate::quality::utils::doc_name_extractor::DocNameExtractor;
use crate::quality::utils::submission_report::SubmissionReport;

/// Equivalent to `org.apache.lucene.benchmark.quality.QualityBenchmark`:
/// main entry point for running a quality benchmark.
///
/// There are two main configurations for running a quality benchmark:
///
/// * Against existing judgements.
/// * For submission (e.g. for a contest).
///
/// The first configuration requires a non null [`Judge`]. The second
/// configuration requires a non null [`SubmissionReport`].
pub struct QualityBenchmark {
    /// Quality Queries that this quality benchmark would execute.
    pub quality_queries: Vec<QualityQuery>,
    /// Parser for turning QualityQueries into Lucene Queries.
    pub qq_parser: Arc<dyn QualityQueryParser>,
    /// Index to be searched.
    pub searcher: Arc<IndexSearcher>,
    /// index field to extract doc name for each search result; used for
    /// judging the results.
    pub doc_name_field: String,
    /// maximal number of queries that this quality benchmark runs. Default:
    /// maxint. Useful for debugging.
    max_queries: i32,
    /// maximal number of results to collect for each query. Default: 1000.
    max_results: i32,
}

impl QualityBenchmark {
    /// `QualityBenchmark(QualityQuery[] qqs, QualityQueryParser qqParser,
    /// IndexSearcher searcher, String docNameField)`: `doc_name_field` is the
    /// name of field containing the document name. This allows to extract
    /// the doc name for search results, and is important for judging the
    /// results.
    pub fn new(
        qqs: Vec<QualityQuery>,
        qq_parser: Arc<dyn QualityQueryParser>,
        searcher: Arc<IndexSearcher>,
        doc_name_field: &str,
    ) -> Self {
        QualityBenchmark {
            quality_queries: qqs,
            qq_parser,
            searcher,
            doc_name_field: doc_name_field.to_string(),
            max_queries: i32::MAX,
            max_results: 1000,
        }
    }

    /// `execute(Judge judge, SubmissionReport submitRep, PrintWriter
    /// qualityLog)`: run the quality benchmark. `judge` can tell if a
    /// certain result doc is relevant for a certain quality query; if
    /// `None`, no judgements would be made (usually for a submission run).
    /// A submission report is created if `submit_rep` is not `None`. If
    /// `quality_log` is not `None`, quality run data would be printed for
    /// each query. Returns the QualityStats of each quality query that was
    /// executed (`None` where no judge judged it).
    ///
    /// # Errors
    ///
    /// If quality benchmark failed to run.
    pub fn execute(
        &self,
        judge: Option<&dyn Judge>,
        submit_rep: Option<&SubmissionReport>,
        quality_log: Option<&PrintWriter>,
    ) -> Result<Vec<Option<QualityStats>>> {
        let n_queries = (self.max_queries.max(0) as usize).min(self.quality_queries.len());
        let mut stats: Vec<Option<QualityStats>> = vec![None; n_queries];
        for (i, qq) in self.quality_queries[..n_queries].iter().enumerate() {
            // generate query
            let q = self.qq_parser.parse(qq)?;
            // search with this query
            let t1 = jdk::current_time_millis();
            let td = self.searcher.search(Arc::clone(&q), self.max_results)?;
            let search_time = jdk::current_time_millis() - t1;
            // most likely we either submit or judge, but check both
            if let Some(judge) = judge {
                stats[i] = Some(self.analyze_query_results(qq, &q, &td, judge, quality_log, search_time)?);
            }
            if let Some(submit_rep) = submit_rep {
                submit_rep.report(qq, &td, &self.doc_name_field, &self.searcher)?;
            }
        }
        if let Some(submit_rep) = submit_rep {
            submit_rep.flush();
        }
        Ok(stats)
    }

    /// `analyzeQueryResults(...)`: analyze/judge results for a single quality
    /// query; optionally log them.
    fn analyze_query_results(
        &self,
        qq: &QualityQuery,
        q: &Arc<dyn Query>,
        td: &TopDocs,
        judge: &dyn Judge,
        logger: Option<&PrintWriter>,
        search_time: i64,
    ) -> Result<QualityStats> {
        let mut stts = QualityStats::new(f64::from(judge.max_recall(qq)), search_time);
        let sd = &td.score_docs;
        // extraction of first doc name we measure also construction of
        // doc name extractor, just in case.
        let mut t1 = jdk::current_time_millis();
        let xt = DocNameExtractor::new(&self.doc_name_field);
        let stored_fields = self.searcher.stored_fields()?;
        for (i, score_doc) in sd.iter().enumerate() {
            let doc_name = xt.doc_name(&*stored_fields, score_doc.doc)?;
            let doc_name_extract_time = jdk::current_time_millis() - t1;
            t1 = jdk::current_time_millis();
            let is_relevant = judge.is_relevant(doc_name.as_deref(), qq);
            stts.add_result(i as i32 + 1, is_relevant, doc_name_extract_time)?;
        }
        if let Some(logger) = logger {
            logger.println(&format!("{}  -  {}", qq.get_query_id(), query_to_string(&**q)));
            stts.log(Some(&format!("{} Stats:", qq.get_query_id())), 1, logger, Some("  "))?;
        }
        Ok(stts)
    }

    /// `getMaxQueries()`: the maximum number of quality queries to run.
    /// Useful at debugging.
    pub fn get_max_queries(&self) -> i32 {
        self.max_queries
    }

    /// `setMaxQueries(int)`: set the maximum number of quality queries to
    /// run. Useful at debugging.
    pub fn set_max_queries(&mut self, max_queries: i32) {
        self.max_queries = max_queries;
    }

    /// `getMaxResults()`: the maximum number of results to collect for each
    /// quality query.
    pub fn get_max_results(&self) -> i32 {
        self.max_results
    }

    /// `setMaxResults(int)`: set the maximum number of results to collect for
    /// each quality query.
    pub fn set_max_results(&mut self, max_results: i32) {
        self.max_results = max_results;
    }
}
