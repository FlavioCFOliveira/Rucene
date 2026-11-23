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

//! Port of `org.apache.lucene.benchmark.quality.TestQualityRun`: test that
//! quality run does its job.
//!
//! NOTE: if the default scoring or StandardAnalyzer is changed, then this
//! test will not work correctly, as it does not dynamically generate its test
//! trec topics/qrels!

use std::io::Read;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::{directory_reader, IndexReader};
use rucene::search::IndexSearcher;
use rucene_analysis_common::jdk::charset::Charset;
use rucene_analysis_common::jdk::io::{BufferedReader, InputStreamReader};
use rucene_benchmark::jdk::PrintWriter;
use rucene_benchmark::quality::judge::Judge;
use rucene_benchmark::quality::quality_benchmark::QualityBenchmark;
use rucene_benchmark::quality::quality_query_parser::QualityQueryParser;
use rucene_benchmark::quality::quality_stats::QualityStats;
use rucene_benchmark::quality::trec::trec_judge::TrecJudge;
use rucene_benchmark::quality::trec::trec_topics_reader::TrecTopicsReader;
use rucene_benchmark::quality::utils::simple_qq_parser::SimpleQQParser;
use rucene_benchmark::quality::utils::submission_report::SubmissionReport;
use rucene_test_framework::util::lucene_test_case::VERBOSE;
use rucene_test_framework::util::LuceneTestCase;

use crate::benchmark_support::search_support::{ftest, set_up};
use crate::benchmark_support::{
    benchmark_class, copy_to_work_dir, exec_benchmark, get_work_dir_nio, get_work_dir_resource_path, resource_bytes,
};

/// The instance of `TestQualityRun`.
#[derive(Default)]
pub struct QualityRunTest;

/// `TestQualityRun`.
pub fn class_def() -> Arc<Class> {
    benchmark_class::<QualityRunTest>(
        "org.apache.lucene.benchmark.quality.TestQualityRun",
        vec![
            set_up(setup),
            ftest("testTrecQuality", test_trec_quality),
            ftest("testTrecTopicsReader", test_trec_topics_reader),
        ],
    )
}

fn setup(_f: &mut QualityRunTest) -> Result<()> {
    copy_to_work_dir("quality", "reuters.578.lines.txt.bz2")
}

/// `new BufferedReader(new InputStreamReader(getClass().getResourceAsStream(name), UTF_8))`.
fn resource_reader(name: &str) -> Result<BufferedReader<InputStreamReader<Box<dyn Read + Send>>>> {
    let stream: Box<dyn Read + Send> = Box::new(std::io::Cursor::new(resource_bytes("quality", name)));
    Ok(BufferedReader::new(InputStreamReader::with_charset(stream, Charset::for_name("UTF-8")?)))
}

fn test_trec_quality(_f: &mut QualityRunTest) -> Result<()> {
    // first create the partial reuters index
    create_reuters_index()?;

    let max_results = 1000;
    let doc_name_field = "doctitle"; // orig docID is in the linedoc format title

    let logger = if *VERBOSE { Some(Arc::new(PrintWriter::with_auto_flush(Box::new(std::io::stdout())))) } else { None };

    // prepare topics
    let q_reader = TrecTopicsReader::new();
    let qqs = q_reader.read_queries(resource_reader("trecTopics.txt")?)?;

    // prepare judge
    let judge = TrecJudge::new(resource_reader("trecQRels.txt")?)?;

    // validate topics & judgments match each other
    judge.validate_data(&qqs, logger.as_deref());

    let dir = LuceneTestCase::new_fs_directory(&get_work_dir_nio().resolve_str("index"));
    let reader = directory_reader::open(dir.as_directory())?;
    let searcher = Arc::new(IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?);

    let qq_parser: Arc<dyn QualityQueryParser> = Arc::new(SimpleQQParser::with_name("title", "body"));
    let mut qrun = QualityBenchmark::new(qqs, qq_parser, searcher, doc_name_field);
    let submit_log = if *VERBOSE { Some(SubmissionReport::new(logger.clone(), "TestRun")?) } else { None };
    qrun.set_max_results(max_results);
    let stats: Vec<QualityStats> = qrun
        .execute(Some(&judge as &dyn Judge), submit_log.as_ref(), logger.as_deref())?
        .into_iter()
        .map(|s| s.expect("stats"))
        .collect();

    // --------- verify by the way judgments were altered for this test:
    // for some queries, depending on m = qnum % 8
    // m==0: avg_precision and recall are hurt, by marking fake docs as relevant
    // m==1: precision_at_n and avg_precision are hurt, by unmarking relevant docs
    // m==2: all precision, precision_at_n and recall are hurt.
    // m>=3: these queries remain perfect
    for (i, s) in stats.iter().enumerate() {
        match i % 8 {
            0 => {
                Assert::assert_true_msg(&format!("avg-p should be hurt: {}", s.get_avp()), 1.0 > s.get_avp());
                Assert::assert_true_msg(&format!("recall should be hurt: {}", s.get_recall()), 1.0 > s.get_recall());
                for j in 1..=QualityStats::MAX_POINTS {
                    let p = s.get_precision_at(j)?;
                    crate::benchmark_support::assert_equals_delta_msg(&format!("p_at_{j} should be perfect: {p}"), 1.0, p, 1E-2);
                }
            }
            1 => {
                Assert::assert_true_msg("avg-p should be hurt", 1.0 > s.get_avp());
                crate::benchmark_support::assert_equals_delta_msg(
                    &format!("recall should be perfect: {}", s.get_recall()),
                    1.0,
                    s.get_recall(),
                    1E-2,
                );
                for j in 1..=QualityStats::MAX_POINTS {
                    let p = s.get_precision_at(j)?;
                    Assert::assert_true_msg(&format!("p_at_{j} should be hurt: {p}"), 1.0 > p);
                }
            }
            2 => {
                Assert::assert_true_msg(&format!("avg-p should be hurt: {}", s.get_avp()), 1.0 > s.get_avp());
                Assert::assert_true_msg(&format!("recall should be hurt: {}", s.get_recall()), 1.0 > s.get_recall());
                for j in 1..=QualityStats::MAX_POINTS {
                    let p = s.get_precision_at(j)?;
                    Assert::assert_true_msg(&format!("p_at_{j} should be hurt: {p}"), 1.0 > p);
                }
            }
            _ => {
                crate::benchmark_support::assert_equals_delta_msg(&format!("avg-p should be perfect: {}", s.get_avp()), 1.0, s.get_avp(), 1E-2);
                crate::benchmark_support::assert_equals_delta_msg(
                    &format!("recall should be perfect: {}", s.get_recall()),
                    1.0,
                    s.get_recall(),
                    1E-2,
                );
                for j in 1..=QualityStats::MAX_POINTS {
                    let p = s.get_precision_at(j)?;
                    crate::benchmark_support::assert_equals_delta_msg(&format!("p_at_{j} should be perfect: {p}"), 1.0, p, 1E-2);
                }
            }
        }
    }

    let avg = QualityStats::average(&stats)?;
    if let Some(logger) = &logger {
        avg.log(Some("Average statistis:"), 1, logger, Some("  "))?;
    }

    Assert::assert_true_msg(&format!("mean avg-p should be hurt: {}", avg.get_avp()), 1.0 > avg.get_avp());
    Assert::assert_true_msg(&format!("avg recall should be hurt: {}", avg.get_recall()), 1.0 > avg.get_recall());
    for j in 1..=QualityStats::MAX_POINTS {
        let p = avg.get_precision_at(j)?;
        Assert::assert_true_msg(&format!("avg p_at_{j} should be hurt: {p}"), 1.0 > p);
    }

    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_trec_topics_reader(_f: &mut QualityRunTest) -> Result<()> {
    // prepare topics
    let q_reader = TrecTopicsReader::new();
    let qqs = q_reader.read_queries(resource_reader("trecTopics.txt")?)?;

    Assert::assert_equals(20usize, qqs.len());

    let qq = &qqs[0];
    Assert::assert_equals_nullable(Some("statement months  total 1987"), qq.get_value("title"));
    Assert::assert_equals_nullable(Some("Topic 0 Description Line 1 Topic 0 Description Line 2"), qq.get_value("description"));
    Assert::assert_equals_nullable(Some("Topic 0 Narrative Line 1 Topic 0 Narrative Line 2"), qq.get_value("narrative"));

    let qq = &qqs[1];
    Assert::assert_equals_nullable(Some("agreed 15  against five"), qq.get_value("title"));
    Assert::assert_equals_nullable(Some("Topic 1 Description Line 1 Topic 1 Description Line 2"), qq.get_value("description"));
    Assert::assert_equals_nullable(Some("Topic 1 Narrative Line 1 Topic 1 Narrative Line 2"), qq.get_value("narrative"));

    let qq = &qqs[19];
    Assert::assert_equals_nullable(Some("20 while  common week"), qq.get_value("title"));
    Assert::assert_equals_nullable(Some("Topic 19 Description Line 1 Topic 19 Description Line 2"), qq.get_value("description"));
    Assert::assert_equals_nullable(Some("Topic 19 Narrative Line 1 Topic 19 Narrative Line 2"), qq.get_value("narrative"));
    Ok(())
}

fn create_reuters_index() -> Result<()> {
    // 1. alg definition
    let alg_lines = vec![
        "# ----- properties ".to_string(),
        "content.source=org.apache.lucene.benchmark.byTask.feeds.LineDocSource".to_string(),
        "analyzer=org.apache.lucene.analysis.classic.ClassicAnalyzer".to_string(),
        format!("docs.file={}", get_work_dir_resource_path("reuters.578.lines.txt.bz2")),
        "content.source.log.step=2500".to_string(),
        "doc.term.vector=false".to_string(),
        "content.source.forever=false".to_string(),
        "directory=FSDirectory".to_string(),
        "doc.stored=true".to_string(),
        "doc.tokenized=true".to_string(),
        "# ----- alg ".to_string(),
        "ResetSystemErase".to_string(),
        "CreateIndex".to_string(),
        "{ AddDoc } : *".to_string(),
        "CloseIndex".to_string(),
    ];
    // 2. execute the algorithm  (required in every "logic" test)
    exec_benchmark(&alg_lines)?;
    Ok(())
}
