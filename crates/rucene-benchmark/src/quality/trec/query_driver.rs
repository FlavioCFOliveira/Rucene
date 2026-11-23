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

//! The types declared in `QueryDriver.java`, of the Java package
//! `org.apache.lucene.benchmark.quality.trec`.

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_reader::IndexReader;
use rucene::search::index_searcher::IndexSearcher;
use rucene::store::{Directory, FSDirectory};
use rucene::util::java_collections::JavaHashSet;

use crate::jdk::{self, PrintWriter};
use crate::quality::judge::Judge;
use crate::quality::quality_benchmark::QualityBenchmark;
use crate::quality::quality_stats::QualityStats;
use crate::quality::trec::trec_judge::TrecJudge;
use crate::quality::trec::trec_topics_reader::TrecTopicsReader;
use crate::quality::utils::simple_qq_parser::SimpleQQParser;
use crate::quality::utils::submission_report::SubmissionReport;

/// Equivalent to `org.apache.lucene.benchmark.quality.trec.QueryDriver`:
/// command-line tool for doing a TREC evaluation run.
pub struct QueryDriver;

impl QueryDriver {
    /// `main(String[] args)`.
    ///
    /// # Errors
    ///
    /// The exception of reading the inputs or running the benchmark.
    pub fn main(args: &[String]) -> Result<()> {
        if args.len() < 4 || args.len() > 5 {
            eprintln!("Usage: QueryDriver <topicsFile> <qrelsFile> <submissionFile> <indexDir> [querySpec]");
            eprintln!("topicsFile: input file containing queries");
            eprintln!("qrelsFile: input file containing relevance judgements");
            eprintln!("submissionFile: output submission file for trec_eval");
            eprintln!("indexDir: index directory");
            eprintln!(
                "querySpec: string composed of fields to use in query consisting of T=title,D=description,N=narrative:"
            );
            eprintln!("\texample: TD (query on Title + Description). The default is T (title only)");
            std::process::exit(1);
        }

        let topics_file = PathBuf::from(&args[0]);
        let qrels_file = PathBuf::from(&args[1]);
        let submission_file = PathBuf::from(&args[2]);
        let submit_log = SubmissionReport::new(
            Some(Arc::new(PrintWriter::new(Box::new(BufWriter::new(File::create(&submission_file)?))))),
            "lucene",
        )?;
        let field_spec = if args.len() == 5 { args[4].as_str() } else { "T" }; // default to Title-only if not specified.

        // --- Use try-with-resources for FSDirectory and IndexReader ---
        let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(PathBuf::from(&args[3]))?);
        let reader: Arc<dyn IndexReader> = match directory_reader::open(Arc::clone(&dir)) {
            Ok(r) => r,
            Err(e) => {
                let _ = dir.close();
                return Err(e);
            }
        };
        let body = (|| -> Result<()> {
            let searcher = Arc::new(IndexSearcher::new(Arc::clone(&reader))?);

            let max_results = 1000;
            let doc_name_field = "docname";

            let logger = PrintWriter::with_auto_flush(Box::new(std::io::stdout()));

            // use trec utilities to read trec topics into quality queries
            let q_reader = TrecTopicsReader::new();
            let qqs = q_reader.read_queries(jdk::new_buffered_reader_utf8(&topics_file)?)?;

            // prepare judge, with trec utilities that read from a QRels file
            let judge = TrecJudge::new(jdk::new_buffered_reader_utf8(&qrels_file)?)?;

            // validate topics & judgments match each other
            judge.validate_data(&qqs, Some(&logger));

            let mut field_set: JavaHashSet = JavaHashSet::new();
            if field_spec.contains('T') {
                field_set.insert("title".to_string());
            }
            if field_spec.contains('D') {
                field_set.insert("description".to_string());
            }
            if field_spec.contains('N') {
                field_set.insert("narrative".to_string());
            }

            // set the parsing of quality queries into Lucene queries.
            let qq_parser = Arc::new(SimpleQQParser::new(field_set.iter().cloned().collect(), "body"));

            // run the benchmark
            let mut qrun = QualityBenchmark::new(qqs, qq_parser, searcher, doc_name_field);
            qrun.set_max_results(max_results);
            let stats = qrun.execute(Some(&judge), Some(&submit_log), Some(&logger))?;

            // print an avarage sum of the results
            let stats: Vec<QualityStats> = stats.into_iter().flatten().collect();
            let avg = QualityStats::average(&stats)?;
            avg.log(Some("SUMMARY"), 2, &logger, Some("  "))
        })();
        let closed_reader = reader.close();
        let closed_dir = dir.close();
        body?;
        closed_reader?;
        closed_dir
    }
}
