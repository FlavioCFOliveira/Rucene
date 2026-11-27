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

//! Port of `org.apache.lucene.benchmark.byTask.TestPerfTasksParse`: test
//! very simply that perf tasks are parsed as expected.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::Query;
use rucene_benchmark::by_task::benchmark::Benchmark;
use rucene_benchmark::by_task::feeds::abstract_query_maker::{
    AbstractQueryMaker, AbstractQueryMakerBase,
};
use rucene_benchmark::by_task::feeds::content_items_source::{
    ContentItemsSource, ContentItemsSourceBase,
};
use rucene_benchmark::by_task::feeds::content_source::ContentSource;
use rucene_benchmark::by_task::feeds::doc_data::DocData;
use rucene_benchmark::by_task::perf_run_data::PerfRunData;
use rucene_benchmark::by_task::tasks::task_sequence::TaskSequence;
use rucene_benchmark::by_task::utils::algorithm::Algorithm;
use rucene_benchmark::by_task::utils::config::Config;
use rucene_benchmark::classes;
use rucene_test_framework::util::lucene_test_case::SuppressSysoutChecks;
use rucene_test_framework::util::LuceneTestCase;

use crate::benchmark_support::conf_dir;
use crate::benchmark_support::core_support::{class_annotated, test};

const NEW_LINE: &str = rucene_benchmark::jdk::NEW_LINE;
const INDENT: &str = "  ";

/// `propPart`: properties in effect in all tests here.
fn prop_part() -> String {
    format!("{INDENT}directory=ByteBuffersDirectory{NEW_LINE}{INDENT}print.props=false{NEW_LINE}")
}

const MOCK_CONTENT_SOURCE: &str =
    "org.apache.lucene.benchmark.byTask.TestPerfTasksParse$MockContentSource";
const MOCK_QUERY_MAKER: &str =
    "org.apache.lucene.benchmark.byTask.TestPerfTasksParse$MockQueryMaker";

/// `public static class MockContentSource extends ContentSource`.
#[derive(Default)]
struct MockContentSource {
    base: ContentItemsSourceBase,
}

impl ContentItemsSource for MockContentSource {
    fn items_base(&self) -> &ContentItemsSourceBase {
        &self.base
    }

    fn java_simple_name(&self) -> &'static str {
        "MockContentSource"
    }

    fn close(&self) -> Result<()> {
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ContentSource for MockContentSource {
    fn get_next_doc_data(&self, _doc_data: &mut DocData) -> Result<()> {
        Ok(())
    }
}

/// `public static class MockQueryMaker extends AbstractQueryMaker`.
#[derive(Default)]
struct MockQueryMaker {
    base: AbstractQueryMakerBase,
}

impl AbstractQueryMaker for MockQueryMaker {
    fn aqm_base(&self) -> &AbstractQueryMakerBase {
        &self.base
    }

    fn java_class_name(&self) -> &'static str {
        MOCK_QUERY_MAKER
    }

    fn prepare_queries(&self) -> Result<Vec<Arc<dyn Query>>> {
        Ok(Vec::new())
    }
}

/// `TestPerfTasksParse`.
pub fn class_def() -> Arc<Class> {
    classes::register_content_source_class(
        MOCK_CONTENT_SOURCE,
        Arc::new(|| Arc::new(MockContentSource::default()) as Arc<dyn ContentSource>),
    );
    classes::register_query_maker_class(
        MOCK_QUERY_MAKER,
        Arc::new(|| {
            Arc::new(MockQueryMaker::default())
                as Arc<dyn rucene_benchmark::by_task::feeds::query_maker::QueryMaker>
        }),
    );
    class_annotated(
        "org.apache.lucene.benchmark.byTask.TestPerfTasksParse",
        vec![
            test(
                "testParseParallelTaskSequenceRepetition",
                test_parse_parallel_task_sequence_repetition,
            ),
            test(
                "testParseTaskSequenceRepetition",
                test_parse_task_sequence_repetition,
            ),
            test("testParseExamples", test_parse_examples),
        ],
        |b| {
            b.annotate(SuppressSysoutChecks {
                bug_url: "very noisy".to_string(),
            })
        },
    )
}

fn check_repetition(parsed_tasks: &str, parallel: bool) -> Result<()> {
    let task_str = "AddDoc";
    let benchmark = Benchmark::new(&(prop_part() + parsed_tasks))?;
    let alg = benchmark.get_algorithm();
    let alg_tasks = alg.extract_tasks();
    let mut found_add = false;
    for task in &alg_tasks {
        if task.task_to_string().contains(task_str) {
            found_add = true;
        }
        if let Some(seq) = task.as_any().downcast_ref::<TaskSequence>() {
            crate::benchmark_support::assert_equals_msg(
                &format!("repetions should be 1000 for {parsed_tasks}"),
                1000,
                seq.get_repetitions(),
            );
            if parallel {
                Assert::assert_true_msg(
                    &format!("sequence for {parsed_tasks} should be parallel!"),
                    seq.is_parallel(),
                );
            } else {
                Assert::assert_false_msg(
                    &format!("sequence for {parsed_tasks} should be sequential!"),
                    seq.is_parallel(),
                );
            }
        }
        Assert::assert_true_msg(
            &format!("Task {task_str} was not found in {alg}"),
            found_add,
        );
    }
    Ok(())
}

/// Test the repetiotion parsing for parallel tasks.
fn test_parse_parallel_task_sequence_repetition() -> Result<()> {
    check_repetition("[ AddDoc ] : 1000", true)
}

/// Test the repetiotion parsing for sequential tasks.
fn test_parse_task_sequence_repetition() -> Result<()> {
    check_repetition("{ AddDoc } : 1000", false)
}

/// Test the parsing of example scripts.
fn test_parse_examples() -> Result<()> {
    // Locate the folder with *.alg resources.
    let examples_dir = conf_dir();
    Assert::assert_true_msg(
        "Couldn't locate *.alg resources?",
        examples_dir.join("addIndexes.alg").exists(),
    );
    let mut found_files = false;
    // Files.newDirectoryStream(examplesDir, "*.alg"): directory order.
    for entry in std::fs::read_dir(&examples_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("alg") {
            continue;
        }
        let text = std::fs::read_to_string(&path)?;
        let config = Arc::new(Config::from_reader(&text)?);
        if let Some(content_source) = config.get_str("content.source", None)? {
            classes::content_source_for_name(&content_source)?;
        }
        let work_dir = LuceneTestCase::create_temp_dir_prefix("TestPerfTasksParse");
        config.set(
            "work.dir",
            &std::path::absolute(work_dir.as_std_path())?.to_string_lossy(),
        )?;
        config.set("content.source", MOCK_CONTENT_SOURCE)?;
        if let Some(dir) = config.get_str("content.source", None)? {
            classes::content_source_for_name(&dir)?;
        }
        config.set("directory", "ByteBuffersDirectory")?;
        if config.get_str("line.file.out", None)?.is_some() {
            let f = LuceneTestCase::create_temp_file_with("linefile", ".txt")?;
            config.set(
                "line.file.out",
                &std::path::absolute(f.as_std_path())?.to_string_lossy(),
            )?;
        }
        if let Some(query_maker) = config.get_str("query.maker", None)? {
            classes::check_query_maker_class(&query_maker)?;
            config.set("query.maker", MOCK_QUERY_MAKER)?;
        }
        let data = PerfRunData::new(config)?;
        Algorithm::new(data)?.close()?;
        found_files = true;
    }
    if !found_files {
        Assert::fail("could not find any .alg files!");
    }
    Ok(())
}
