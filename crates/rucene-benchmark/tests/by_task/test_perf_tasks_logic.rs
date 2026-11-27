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

//! Port of `org.apache.lucene.benchmark.byTask.TestPerfTasksLogic`: test
//! very simply that perf tasks - simple algorithms - are doing what they
//! should.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{Analyzer, StringCharReader, TermToBytesRefAttribute};
use rucene::error::Result;
use rucene::index::field_infos::FieldInfos;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_policy::{
    CodecReaderSupplier, LogMergePolicy, MergeContext, MergePolicy, MergeSpecification,
    MergeTrigger,
};
use rucene::index::merge_scheduler::{MergeScheduler, MergeSource, SerialMergeScheduler};
use rucene::index::multi_fields::MultiTerms;
use rucene::index::postings_enum::POSTINGS_ENUM_FREQS;
use rucene::index::segment_info::SegmentCommitInfo;
use rucene::index::segment_infos::SegmentInfos;
use rucene::index::{directory_reader, CodecReader, IndexReader};
use rucene::search::NO_MORE_DOCS;
use rucene_analysis_common::collation::collation_key_analyzer::CollationKeyAnalyzer;
use rucene_analysis_common::jdk::collator::RuleBasedCollator;
use rucene_analysis_common::jdk::locale::Locale;
use rucene_benchmark::by_task::feeds::doc_maker::DocMaker;
use rucene_benchmark::by_task::tasks::write_line_doc_task::WriteLineDocTask;
use rucene_benchmark::classes;
use rucene_test_framework::analysis::base_token_stream_test_case::BaseTokenStreamTestCase;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::lucene_test_case::SuppressCodecs;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::benchmark_support::search_support::{ftest, set_up};
use crate::benchmark_support::{
    benchmark_class_annotated, copy_to_work_dir, exec_benchmark, get_work_dir,
    get_work_dir_resource_path,
};
use crate::by_task::tasks::counting_search_test_task::{self as cst, CountingSearchTestTask};

const MY_MERGE_SCHEDULER: &str =
    "org.apache.lucene.benchmark.byTask.TestPerfTasksLogic$MyMergeScheduler";
const MY_MERGE_POLICY: &str = "org.apache.lucene.benchmark.byTask.TestPerfTasksLogic$MyMergePolicy";

/// The instance of `TestPerfTasksLogic`.
#[derive(Default)]
pub struct PerfTasksLogicTest;

/// `TestPerfTasksLogic`.
pub fn class_def() -> Arc<Class> {
    CountingSearchTestTask::register();
    classes::register_merge_scheduler_class(
        MY_MERGE_SCHEDULER,
        Arc::new(|| Arc::new(MyMergeScheduler::new()) as Arc<dyn MergeScheduler>),
    );
    classes::register_merge_policy_class(
        MY_MERGE_POLICY,
        Arc::new(|| Arc::new(MyMergePolicy::new()) as Arc<dyn MergePolicy>),
    );
    benchmark_class_annotated::<PerfTasksLogicTest>(
        "org.apache.lucene.benchmark.byTask.TestPerfTasksLogic",
        vec![
            set_up(setup),
            ftest("testIndexAndSearchTasks", test_index_and_search_tasks),
            ftest("testTimedSearchTask", test_timed_search_task),
            ftest("testBGSearchTaskThreads", test_bg_search_task_threads),
            ftest("testExhaustContentSource", test_exhaust_content_source),
            ftest("testDocMakerThreadSafety", test_doc_maker_thread_safety),
            ftest("testParallelDocMaker", test_parallel_doc_maker),
            ftest("testLineDocFile", test_line_doc_file),
            ftest("testReadTokens", test_read_tokens),
            ftest("testParallelExhausted", test_parallel_exhausted),
            ftest("testExhaustedLooped", test_exhausted_looped),
            ftest("testCloseIndexFalse", test_close_index_false),
            ftest("testMergeScheduler", test_merge_scheduler),
            ftest("testMergePolicy", test_merge_policy),
            ftest("testIndexWriterSettings", test_index_writer_settings),
            ftest("testIndexingWithFacets", test_indexing_with_facets),
            ftest("testForceMerge", test_force_merge),
            ftest("testDisableCounting", test_disable_counting),
            ftest("testLocale", test_locale),
            ftest("testCollator", test_collator),
            ftest("testShingleAnalyzer", test_shingle_analyzer),
            ftest("testAnalyzerFactory", test_analyzer_factory),
        ],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string(), "Direct".to_string()],
            })
        },
    )
}

fn setup(_f: &mut PerfTasksLogicTest) -> Result<()> {
    copy_to_work_dir("by_task", "reuters.first20.lines.txt")?;
    copy_to_work_dir("by_task", "test-mapping-ISOLatin1Accent-partial.txt")
}

fn s(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|l| l.to_string()).collect()
}

fn get_reuters20_lines_file() -> String {
    get_work_dir_resource_path("reuters.first20.lines.txt")
}

/// `new IndexWriter(dir, new IndexWriterConfig(new MockAnalyzer(random())).setOpenMode(OpenMode.APPEND)).close()`.
fn open_append_and_close(dir: Arc<dyn rucene::store::Directory>) -> Result<()> {
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*LuceneTestCase::random(),
    )) as Arc<dyn Analyzer>));
    iwc.set_open_mode(OpenMode::APPEND)?;
    let iw = IndexWriter::new(dir, iwc)?;
    iw.close()
}

fn num_docs(dir: Arc<dyn rucene::store::Directory>) -> Result<i32> {
    let ir = directory_reader::open(dir)?;
    let n = ir.num_docs();
    ir.close()?;
    Ok(n)
}

/// Test index creation logic.
fn test_index_and_search_tasks(_f: &mut PerfTasksLogicTest) -> Result<()> {
    let alg_lines = s(&[
        "ResetSystemErase",
        "CreateIndex",
        "{ AddDoc } : 1000",
        "ForceMerge(1)",
        "CloseIndex",
        "OpenReader",
        "{ CountingSearchTest } : 200",
        "CloseReader",
        "[ CountingSearchTest > : 70",
        "[ CountingSearchTest > : 9",
    ]);
    // 2. we test this value later
    cst::reset_num_searches();
    // 3. execute the algorithm  (required in every "logic" test)
    let benchmark = exec_benchmark(&alg_lines)?;
    // 4. test specific checks after the benchmark run completed.
    crate::benchmark_support::assert_equals_msg(
        "TestSearchTask was supposed to be called!",
        279,
        cst::counters().num_searches,
    );
    let dir = benchmark.get_run_data().get_directory().expect("directory");
    Assert::assert_true_msg(
        "Index does not exist?...!",
        directory_reader::index_exists(&*dir)?,
    );
    // now we should be able to open the index for write.
    open_append_and_close(Arc::clone(&dir))?;
    crate::benchmark_support::assert_equals_msg(
        "1000 docs were added to the index, this is what we expect to find!",
        1000,
        num_docs(dir)?,
    );
    Ok(())
}

/// Test timed sequence task.
fn test_timed_search_task(_f: &mut PerfTasksLogicTest) -> Result<()> {
    let alg_lines = s(&[
        "log.step=100000",
        "ResetSystemErase",
        "CreateIndex",
        "{ AddDoc } : 100",
        "ForceMerge(1)",
        "CloseIndex",
        "OpenReader",
        "{ CountingSearchTest } : .5s",
        "CloseReader",
    ]);
    cst::reset_num_searches();
    exec_benchmark(&alg_lines)?;
    let c = cst::counters();
    Assert::assert_true(c.num_searches > 0);
    let elapsed = (c.prev_last_nanos - c.start_nanos) / 1_000_000;
    Assert::assert_true_msg(&format!("elapsed time was {elapsed} ms"), elapsed <= 1500);
    Ok(())
}

// disabled until we fix BG thread prio -- this test
// causes build to hang
fn test_bg_search_task_threads(_f: &mut PerfTasksLogicTest) -> Result<()> {
    let alg_lines = s(&[
        "log.time.step.msec = 100",
        "log.step=100000",
        "ResetSystemErase",
        "CreateIndex",
        "{ AddDoc } : 1000",
        "ForceMerge(1)",
        "CloseIndex",
        "OpenReader",
        "{",
        "  [ \"XSearch\" { CountingSearchTest > : * ] : 2 &-1",
        "  Wait(0.5)",
        "}",
        "CloseReader",
        "RepSumByPref X",
    ]);
    cst::reset_num_searches();
    exec_benchmark(&alg_lines)?;

    // NOTE: cannot assert this, because on a super-slow
    // system, it could be after waiting 0.5 seconds that
    // the search threads hadn't yet succeeded in starting
    // up and then they start up and do no searching:
    // assertTrue(CountingSearchTestTask.numSearches > 0);
    Ok(())
}

/// Test Exhasting Doc Maker logic.
fn test_exhaust_content_source(_f: &mut PerfTasksLogicTest) -> Result<()> {
    let alg_lines = s(&[
        "# ----- properties ",
        "content.source=org.apache.lucene.benchmark.byTask.feeds.SingleDocSource",
        "content.source.log.step=1",
        "doc.term.vector=false",
        "content.source.forever=false",
        "directory=ByteBuffersDirectory",
        "doc.stored=false",
        "doc.tokenized=false",
        "# ----- alg ",
        "CreateIndex",
        "{ AddDoc } : * ",
        "ForceMerge(1)",
        "CloseIndex",
        "OpenReader",
        "{ CountingSearchTest } : 100",
        "CloseReader",
        "[ CountingSearchTest > : 30",
        "[ CountingSearchTest > : 9",
    ]);
    cst::reset_num_searches();
    let benchmark = exec_benchmark(&alg_lines)?;
    crate::benchmark_support::assert_equals_msg(
        "TestSearchTask was supposed to be called!",
        139,
        cst::counters().num_searches,
    );
    let dir = benchmark.get_run_data().get_directory().expect("directory");
    Assert::assert_true_msg(
        "Index does not exist?...!",
        directory_reader::index_exists(&*dir)?,
    );
    open_append_and_close(Arc::clone(&dir))?;
    crate::benchmark_support::assert_equals_msg(
        "1 docs were added to the index, this is what we expect to find!",
        1,
        num_docs(dir)?,
    );
    Ok(())
}

// LUCENE-1994: test thread safety of SortableSingleDocMaker
fn test_doc_maker_thread_safety(_f: &mut PerfTasksLogicTest) -> Result<()> {
    let alg_lines = s(&[
        "# ----- properties ",
        "content.source=org.apache.lucene.benchmark.byTask.feeds.SortableSingleDocSource",
        "doc.term.vector=false",
        "log.step.AddDoc=10000",
        "content.source.forever=true",
        "directory=ByteBuffersDirectory",
        "doc.reuse.fields=false",
        "doc.stored=true",
        "doc.tokenized=false",
        "doc.index.props=true",
        "# ----- alg ",
        "CreateIndex",
        "[ { AddDoc > : 250 ] : 4",
        "CloseIndex",
    ]);
    cst::reset_num_searches();
    let benchmark = exec_benchmark(&alg_lines)?;
    let r = directory_reader::open(benchmark.get_run_data().get_directory().expect("directory"))?;
    let max_doc = r.max_doc();
    Assert::assert_equals(1000, max_doc);
    let stored_fields = r.stored_fields()?;
    for i in 0..1000 {
        Assert::assert_true_msg(
            &format!("doc {i} has null country"),
            stored_fields.document(i)?.get_field("country").is_some(),
        );
    }
    r.close()?;
    Ok(())
}

/// Test Parallel Doc Maker logic (for LUCENE-940).
fn test_parallel_doc_maker(_f: &mut PerfTasksLogicTest) -> Result<()> {
    let alg_lines = vec![
        "# ----- properties ".to_string(),
        "content.source=org.apache.lucene.benchmark.byTask.feeds.LineDocSource".to_string(),
        format!("docs.file={}", get_reuters20_lines_file()),
        "content.source.log.step=3".to_string(),
        "doc.term.vector=false".to_string(),
        "content.source.forever=false".to_string(),
        "directory=FSDirectory".to_string(),
        "doc.stored=false".to_string(),
        "doc.tokenized=false".to_string(),
        "# ----- alg ".to_string(),
        "CreateIndex".to_string(),
        "[ { AddDoc } : * ] : 4 ".to_string(),
        "CloseIndex".to_string(),
    ];
    let benchmark = exec_benchmark(&alg_lines)?;
    let ndocs_expected = 20; // first 20 reuters docs.
    crate::benchmark_support::assert_equals_msg(
        "wrong number of docs in the index!",
        ndocs_expected,
        num_docs(benchmark.get_run_data().get_directory().expect("directory"))?,
    );
    Ok(())
}

/// Test WriteLineDoc and LineDocSource.
fn test_line_doc_file(_f: &mut PerfTasksLogicTest) -> Result<()> {
    let line_file = LuceneTestCase::create_temp_file_with("test.reuters.lines", ".txt")?;
    let line_file_abs = std::path::absolute(line_file.as_std_path())?
        .to_string_lossy()
        .replace('\\', "/");
    // We will call WriteLineDocs this many times
    const NUM_TRY_DOCS: i32 = 50;
    // Creates a line file with first 50 docs from SingleDocSource
    let alg_lines1 = vec![
        "# ----- properties ".to_string(),
        "content.source=org.apache.lucene.benchmark.byTask.feeds.SingleDocSource".to_string(),
        "content.source.forever=true".to_string(),
        format!("line.file.out={line_file_abs}"),
        "# ----- alg ".to_string(),
        format!("{{WriteLineDoc()}}:{NUM_TRY_DOCS}"),
    ];
    // Run algo
    exec_benchmark(&alg_lines1)?;
    let text = std::fs::read_to_string(line_file.as_std_path())?;
    let mut num_lines = 0;
    for line in text.lines() {
        if num_lines == 0 && line.starts_with(WriteLineDocTask::FIELDS_HEADER_INDICATOR) {
            continue; // do not count the header line as a doc
        }
        num_lines += 1;
    }
    crate::benchmark_support::assert_equals_msg(
        &format!(
            "did not see the right number of docs; should be {NUM_TRY_DOCS} but was {num_lines}"
        ),
        NUM_TRY_DOCS,
        num_lines,
    );
    // Index the line docs
    let alg_lines2 = vec![
        "# ----- properties ".to_string(),
        "analyzer=org.apache.lucene.analysis.core.WhitespaceAnalyzer".to_string(),
        "content.source=org.apache.lucene.benchmark.byTask.feeds.LineDocSource".to_string(),
        format!("docs.file={line_file_abs}"),
        "content.source.forever=false".to_string(),
        "doc.reuse.fields=false".to_string(),
        "ram.flush.mb=4".to_string(),
        "# ----- alg ".to_string(),
        "ResetSystemErase".to_string(),
        "CreateIndex".to_string(),
        "{AddDoc}: *".to_string(),
        "CloseIndex".to_string(),
    ];
    // Run algo
    let benchmark = exec_benchmark(&alg_lines2)?;
    // now we should be able to open the index for write.
    let dir = benchmark.get_run_data().get_directory().expect("directory");
    open_append_and_close(Arc::clone(&dir))?;
    let n = num_docs(dir)?;
    crate::benchmark_support::assert_equals_msg(
        &format!("{num_lines} lines were created but {n} docs are in the index"),
        num_lines,
        n,
    );
    Ok(())
}

/// Test ReadTokensTask.
fn test_read_tokens(_f: &mut PerfTasksLogicTest) -> Result<()> {
    // We will call ReadTokens on this many docs
    const NUM_DOCS: i32 = 20;
    // Read tokens from first NUM_DOCS docs from Reuters and
    // then build index from the same docs
    let alg_lines1 = vec![
        "# ----- properties ".to_string(),
        "analyzer=org.apache.lucene.analysis.core.WhitespaceAnalyzer".to_string(),
        "content.source=org.apache.lucene.benchmark.byTask.feeds.LineDocSource".to_string(),
        format!("docs.file={}", get_reuters20_lines_file()),
        "# ----- alg ".to_string(),
        format!("{{ReadTokens}}: {NUM_DOCS}"),
        "ResetSystemErase".to_string(),
        "CreateIndex".to_string(),
        format!("{{AddDoc}}: {NUM_DOCS}"),
        "CloseIndex".to_string(),
    ];
    // Run algo
    let benchmark = exec_benchmark(&alg_lines1)?;
    let stats = benchmark.get_run_data().get_points().task_stats();
    // Count how many tokens all ReadTokens saw
    let mut total_token_count1 = 0;
    for stat in &stats {
        let stat = stat.lock().unwrap();
        if stat.get_task().get_name() == "ReadTokens" {
            total_token_count1 += stat.get_count();
        }
    }
    // Separately count how many tokens are actually in the index:
    let reader =
        directory_reader::open(benchmark.get_run_data().get_directory().expect("directory"))?;
    Assert::assert_equals(NUM_DOCS, reader.num_docs());
    let reader_dyn: Arc<dyn IndexReader> = Arc::clone(&reader) as Arc<dyn IndexReader>;
    let mut total_token_count2 = 0;
    let fields = FieldInfos::get_indexed_fields(Arc::clone(&reader_dyn))?;
    for field_name in fields.iter() {
        if field_name == DocMaker::ID_FIELD
            || field_name == DocMaker::DATE_MSEC_FIELD
            || field_name == DocMaker::TIME_SEC_FIELD
        {
            continue;
        }
        let Some(terms) = MultiTerms::get_terms(&reader_dyn, field_name)? else {
            continue;
        };
        let mut terms_enum = terms.iterator()?;
        let mut docs = None;
        while terms_enum.next()?.is_some() {
            docs = TestUtil::docs_of(
                &*LuceneTestCase::random(),
                &mut *terms_enum,
                docs,
                POSTINGS_ENUM_FREQS,
            )?;
            let d = docs.as_mut().expect("docs");
            while d.next_doc()? != NO_MORE_DOCS {
                total_token_count2 += d.freq()?;
            }
        }
    }
    reader.close()?;
    // Make sure they are the same
    Assert::assert_equals(total_token_count1, total_token_count2);
    Ok(())
}

fn reuters_lines(extra_props: &[&str], alg: &[&str]) -> Vec<String> {
    let mut v = vec![
        "# ----- properties ".to_string(),
        "content.source=org.apache.lucene.benchmark.byTask.feeds.LineDocSource".to_string(),
        format!("docs.file={}", get_reuters20_lines_file()),
    ];
    v.extend(extra_props.iter().map(|l| l.to_string()));
    v.push("# ----- alg ".to_string());
    v.extend(alg.iter().map(|l| l.to_string()));
    v
}

/// Test that " {[AddDoc(4000)]: 4} : * " works corrcetly (for LUCENE-941).
fn test_parallel_exhausted(_f: &mut PerfTasksLogicTest) -> Result<()> {
    let alg_lines = reuters_lines(
        &[
            "content.source.log.step=3",
            "doc.term.vector=false",
            "content.source.forever=false",
            "directory=ByteBuffersDirectory",
            "doc.stored=false",
            "doc.tokenized=false",
            "task.max.depth.log=1",
        ],
        &[
            "CreateIndex",
            "{ [ AddDoc]: 4} : * ",
            "ResetInputs ",
            "{ [ AddDoc]: 4} : * ",
            "CloseIndex",
        ],
    );
    let benchmark = exec_benchmark(&alg_lines)?;
    let ndocs_expected = 2 * 20; // first 20 reuters docs.
    crate::benchmark_support::assert_equals_msg(
        "wrong number of docs in the index!",
        ndocs_expected,
        num_docs(benchmark.get_run_data().get_directory().expect("directory"))?,
    );
    Ok(())
}

/// Test that exhaust in loop works as expected (LUCENE-1115).
fn test_exhausted_looped(_f: &mut PerfTasksLogicTest) -> Result<()> {
    let alg_lines = reuters_lines(
        &[
            "content.source.log.step=3",
            "doc.term.vector=false",
            "content.source.forever=false",
            "directory=ByteBuffersDirectory",
            "doc.stored=false",
            "doc.tokenized=false",
            "task.max.depth.log=1",
        ],
        &[
            "{ \"Rounds\"",
            "  ResetSystemErase",
            "  CreateIndex",
            "  { \"AddDocs\"  AddDoc > : * ",
            "  CloseIndex",
            "} : 2",
        ],
    );
    let benchmark = exec_benchmark(&alg_lines)?;
    let ndocs_expected = 20; // first 20 reuters docs.
    crate::benchmark_support::assert_equals_msg(
        "wrong number of docs in the index!",
        ndocs_expected,
        num_docs(benchmark.get_run_data().get_directory().expect("directory"))?,
    );
    Ok(())
}

/// Test that we can close IndexWriter with argument "false".
fn test_close_index_false(_f: &mut PerfTasksLogicTest) -> Result<()> {
    let alg_lines = reuters_lines(
        &[
            "ram.flush.mb=-1",
            "max.buffered=2",
            "content.source.log.step=3",
            "doc.term.vector=false",
            "content.source.forever=false",
            "directory=ByteBuffersDirectory",
            "doc.stored=false",
            "doc.tokenized=false",
            "debug.level=1",
        ],
        &[
            "{ \"Rounds\"",
            "  ResetSystemErase",
            "  CreateIndex",
            "  { \"AddDocs\"  AddDoc > : * ",
            "  CloseIndex(false)",
            "} : 2",
        ],
    );
    let benchmark = exec_benchmark(&alg_lines)?;
    let ndocs_expected = 20; // first 20 reuters docs.
    crate::benchmark_support::assert_equals_msg(
        "wrong number of docs in the index!",
        ndocs_expected,
        num_docs(benchmark.get_run_data().get_directory().expect("directory"))?,
    );
    Ok(())
}

/// `public static class MyMergeScheduler extends SerialMergeScheduler`.
pub struct MyMergeScheduler {
    inner: SerialMergeScheduler,
    called: bool,
}

impl MyMergeScheduler {
    fn new() -> Self {
        MyMergeScheduler {
            inner: SerialMergeScheduler::new(),
            called: true,
        }
    }
}

impl fmt::Display for MyMergeScheduler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl MergeScheduler for MyMergeScheduler {
    fn merge(&self, merge_source: Arc<dyn MergeSource>, trigger: MergeTrigger) -> Result<()> {
        self.inner.merge(merge_source, trigger)
    }

    fn wrap_for_merge(
        &self,
        merge: &rucene::index::merge_policy::OneMerge,
        directory: Arc<dyn rucene::store::Directory>,
    ) -> Result<Arc<dyn rucene::store::Directory>> {
        self.inner.wrap_for_merge(merge, directory)
    }

    fn same_thread_executor(&self) -> &Arc<rucene::util::SameThreadExecutorService> {
        self.inner.same_thread_executor()
    }

    fn get_intra_merge_executor(
        &self,
        merge: &rucene::index::merge_policy::OneMerge,
    ) -> Result<Option<Arc<dyn rucene::search::Executor>>> {
        self.inner.get_intra_merge_executor(merge)
    }

    fn close(&self) -> Result<()> {
        self.inner.close()
    }

    fn initialize(
        &self,
        info_stream: Arc<dyn rucene::util::InfoStream>,
        directory: &Arc<dyn rucene::store::Directory>,
    ) -> Result<()> {
        self.inner.initialize(info_stream, directory)
    }

    fn set_info_stream(&self, info_stream: Arc<dyn rucene::util::InfoStream>) {
        self.inner.set_info_stream(info_stream)
    }

    fn get_info_stream(&self) -> Option<Arc<dyn rucene::util::InfoStream>> {
        self.inner.get_info_stream()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

/// Test that we can set merge scheduler".
fn test_merge_scheduler(_f: &mut PerfTasksLogicTest) -> Result<()> {
    let ms = format!("merge.scheduler={MY_MERGE_SCHEDULER}");
    let alg_lines = reuters_lines(
        &[
            "content.source.log.step=3",
            "doc.term.vector=false",
            "content.source.forever=false",
            "directory=ByteBuffersDirectory",
            &ms,
            "doc.stored=false",
            "doc.tokenized=false",
            "debug.level=1",
        ],
        &[
            "{ \"Rounds\"",
            "  ResetSystemErase",
            "  CreateIndex",
            "  { \"AddDocs\"  AddDoc > : * ",
            "} : 2",
        ],
    );
    let benchmark = exec_benchmark(&alg_lines)?;
    let writer = benchmark.get_run_data().get_index_writer().expect("writer");
    let scheduler = writer.get_config().merge_scheduler();
    Assert::assert_true_msg(
        "did not use the specified MergeScheduler",
        scheduler
            .as_any()
            .and_then(|a| a.downcast_ref::<MyMergeScheduler>())
            .expect("ClassCastException")
            .called,
    );
    writer.close()?;
    let ndocs_expected = 20; // first 20 reuters docs.
    crate::benchmark_support::assert_equals_msg(
        "wrong number of docs in the index!",
        ndocs_expected,
        num_docs(benchmark.get_run_data().get_directory().expect("directory"))?,
    );
    Ok(())
}

/// `public static class MyMergePolicy extends LogDocMergePolicy`.
pub struct MyMergePolicy {
    inner: LogMergePolicy,
    called: bool,
}

impl MyMergePolicy {
    fn new() -> Self {
        MyMergePolicy {
            inner: LogMergePolicy::by_doc_count(),
            called: true,
        }
    }
}

impl fmt::Display for MyMergePolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl MergePolicy for MyMergePolicy {
    fn find_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.inner
            .find_merges(merge_trigger, segment_infos, merge_context)
    }

    fn find_merges_for_readers(
        &self,
        readers: &[Arc<dyn CodecReader>],
    ) -> Result<Option<MergeSpecification>> {
        self.inner.find_merges_for_readers(readers)
    }

    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.inner.find_forced_merges(
            segment_infos,
            max_num_segments,
            segments_to_merge,
            merge_context,
        )
    }

    fn find_forced_deletes_merges(
        &self,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.inner
            .find_forced_deletes_merges(segment_infos, merge_context)
    }

    fn find_full_flush_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.inner
            .find_full_flush_merges(merge_trigger, segment_infos, merge_context)
    }

    fn use_compound_file(
        &self,
        infos: &SegmentInfos,
        merged_info: &SegmentCommitInfo,
        merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        self.inner
            .use_compound_file(infos, merged_info, merge_context)
    }

    fn size(&self, info: &SegmentCommitInfo, merge_context: &dyn MergeContext) -> Result<i64> {
        self.inner.size(info, merge_context)
    }

    fn max_full_flush_merge_size(&self) -> i64 {
        self.inner.max_full_flush_merge_size()
    }

    fn is_merged(
        &self,
        infos: &SegmentInfos,
        info: &SegmentCommitInfo,
        merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        self.inner.is_merged(infos, info, merge_context)
    }

    fn keep_fully_deleted_segment(
        &self,
        reader_supplier: &mut CodecReaderSupplier<'_>,
    ) -> Result<bool> {
        self.inner.keep_fully_deleted_segment(reader_supplier)
    }

    fn num_deletes_to_merge_default(&self, info: &SegmentCommitInfo, del_count: i32) -> i32 {
        self.inner.num_deletes_to_merge_default(info, del_count)
    }

    fn num_deletes_to_merge_with_reader(
        &self,
        info: &SegmentCommitInfo,
        del_count: i32,
        reader_supplier: &mut dyn FnMut() -> Result<Arc<dyn CodecReader>>,
    ) -> Result<i32> {
        self.inner
            .num_deletes_to_merge_with_reader(info, del_count, reader_supplier)
    }

    fn get_no_cfs_ratio(&self) -> f64 {
        self.inner.get_no_cfs_ratio()
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.inner.get_max_cfs_segment_size()
    }

    fn get_max_cfs_segment_size_mb(&self) -> f64 {
        self.inner.get_max_cfs_segment_size_mb()
    }

    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        self.inner.set_no_cfs_ratio(no_cfs_ratio)
    }

    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        self.inner.set_max_cfs_segment_size_mb(v)
    }

    fn assert_del_count(&self, del_count: i32, info: &SegmentCommitInfo) -> Result<bool> {
        self.inner.assert_del_count(del_count, info)
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

/// Test that we can set merge policy".
fn test_merge_policy(_f: &mut PerfTasksLogicTest) -> Result<()> {
    let mp = format!("merge.policy={MY_MERGE_POLICY}");
    let alg_lines = reuters_lines(
        &[
            "content.source.log.step=3",
            "ram.flush.mb=-1",
            "max.buffered=2",
            "doc.term.vector=false",
            "content.source.forever=false",
            "directory=ByteBuffersDirectory",
            &mp,
            "doc.stored=false",
            "doc.tokenized=false",
            "debug.level=1",
        ],
        &[
            "{ \"Rounds\"",
            "  ResetSystemErase",
            "  CreateIndex",
            "  { \"AddDocs\"  AddDoc > : * ",
            "} : 2",
        ],
    );
    let benchmark = exec_benchmark(&alg_lines)?;
    let writer = benchmark.get_run_data().get_index_writer().expect("writer");
    let policy = writer.get_config().merge_policy();
    Assert::assert_true_msg(
        "did not use the specified MergePolicy",
        policy
            .as_any()
            .and_then(|a| a.downcast_ref::<MyMergePolicy>())
            .expect("ClassCastException")
            .called,
    );
    writer.close()?;
    let ndocs_expected = 20; // first 20 reuters docs.
    crate::benchmark_support::assert_equals_msg(
        "wrong number of docs in the index!",
        ndocs_expected,
        num_docs(benchmark.get_run_data().get_directory().expect("directory"))?,
    );
    Ok(())
}

/// Test that IndexWriter settings stick.
fn test_index_writer_settings(_f: &mut PerfTasksLogicTest) -> Result<()> {
    let alg_lines = reuters_lines(
        &[
            "content.source.log.step=3",
            "ram.flush.mb=-1",
            "max.buffered=2",
            "compound=cmpnd:true:false",
            "doc.term.vector=vector:false:true",
            "content.source.forever=false",
            "directory=ByteBuffersDirectory",
            "doc.stored=false",
            "merge.factor=3",
            "doc.tokenized=false",
            "debug.level=1",
        ],
        &[
            "{ \"Rounds\"",
            "  ResetSystemErase",
            "  CreateIndex",
            "  { \"AddDocs\"  AddDoc > : * ",
            "  NewRound",
            "} : 2",
        ],
    );
    let benchmark = exec_benchmark(&alg_lines)?;
    let writer = benchmark.get_run_data().get_index_writer().expect("writer");
    Assert::assert_equals(2, writer.get_config().max_buffered_docs());
    Assert::assert_equals(
        rucene::index::index_writer_config::LiveIndexWriterConfig::DISABLE_AUTO_FLUSH,
        writer.get_config().ram_buffer_size_mb() as i32,
    );
    let mp = writer.get_config().merge_policy();
    Assert::assert_equals(
        3,
        mp.as_any()
            .and_then(|a| a.downcast_ref::<LogMergePolicy>())
            .expect("ClassCastException")
            .get_merge_factor(),
    );
    Assert::assert_equals_double_msg(None, 0.0, mp.get_no_cfs_ratio(), 0.0);
    writer.close()?;
    let dir = benchmark.get_run_data().get_directory().expect("directory");
    let reader = directory_reader::open(dir)?;
    let tfv = reader.term_vectors()?.get(0)?;
    Assert::assert_true(tfv.is_some());
    Assert::assert_true(tfv.unwrap().size() > 0);
    reader.close()?;
    Ok(())
}

/// Test indexing with facets tasks.
fn test_indexing_with_facets(_f: &mut PerfTasksLogicTest) -> Result<()> {
    // 1. alg definition (required in every "logic" test)
    let alg_lines = reuters_lines(
        &[
            "content.source.log.step=100",
            "content.source.forever=false",
            "directory=ByteBuffersDirectory",
            "doc.stored=false",
            "merge.factor=3",
            "doc.tokenized=false",
            "debug.level=1",
        ],
        &[
            "ResetSystemErase",
            "CreateIndex",
            "CreateTaxonomyIndex",
            "{ \"AddDocs\"  AddFacetedDoc > : * ",
            "CloseIndex",
            "CloseTaxonomyIndex",
            "OpenTaxonomyReader",
        ],
    );
    // 2. execute the algorithm  (required in every "logic" test)
    let benchmark = exec_benchmark(&alg_lines)?;
    let run_data = benchmark.get_run_data();
    Assert::assert_true_msg(
        "taxo writer was not properly closed",
        run_data.get_taxonomy_writer().is_none(),
    );
    let taxo_reader = run_data.get_taxonomy_reader()?;
    Assert::assert_true_msg("taxo reader was not opened", taxo_reader.is_some());
    let taxo_reader = taxo_reader.unwrap();
    Assert::assert_true_msg(
        "nothing was added to the taxnomy (expecting root and at least one addtional category)",
        taxo_reader.get_size()? > 1,
    );
    taxo_reader.close()?;
    Ok(())
}

/// Test that we can call forceMerge(maxNumSegments).
fn test_force_merge(_f: &mut PerfTasksLogicTest) -> Result<()> {
    let alg_lines = reuters_lines(
        &[
            "content.source.log.step=3",
            "ram.flush.mb=-1",
            "max.buffered=3",
            "doc.term.vector=false",
            "content.source.forever=false",
            "directory=ByteBuffersDirectory",
            "merge.policy=org.apache.lucene.index.LogDocMergePolicy",
            "doc.stored=false",
            "doc.tokenized=false",
            "debug.level=1",
        ],
        &[
            "{ \"Rounds\"",
            "  ResetSystemErase",
            "  CreateIndex",
            "  { \"AddDocs\"  AddDoc > : * ",
            "  ForceMerge(3)",
            "  CloseIndex()",
            "} : 2",
        ],
    );
    let benchmark = exec_benchmark(&alg_lines)?;
    let dir = benchmark.get_run_data().get_directory().expect("directory");
    let ndocs_expected = 20; // first 20 reuters docs.
    crate::benchmark_support::assert_equals_msg(
        "wrong number of docs in the index!",
        ndocs_expected,
        num_docs(Arc::clone(&dir))?,
    );
    let infos = SegmentInfos::read_latest_commit(dir)?;
    Assert::assert_equals(3usize, infos.size());
    Ok(())
}

/// Test disabling task count (LUCENE-1136).
fn test_disable_counting(_f: &mut PerfTasksLogicTest) -> Result<()> {
    do_test_disable_counting(true)?;
    do_test_disable_counting(false)
}

fn do_test_disable_counting(disable: bool) -> Result<()> {
    let alg_lines = disable_counting_lines(disable);
    let benchmark = exec_benchmark(&alg_lines)?;
    // 3. test the counters
    let n = if disable { 0 } else { 1 };
    let mut n_checked = 0;
    for stats in benchmark.get_run_data().get_points().task_stats() {
        let stats = stats.lock().unwrap();
        let task_name = stats.get_task().get_name();
        if task_name == "Rounds" {
            crate::benchmark_support::assert_equals_msg(
                "Wrong total count!",
                20 + 2 * n,
                stats.get_count(),
            );
            n_checked += 1;
        } else if task_name == "CreateIndex" {
            crate::benchmark_support::assert_equals_msg(
                "Wrong count for CreateIndex!",
                n,
                stats.get_count(),
            );
            n_checked += 1;
        } else if task_name == "CloseIndex" {
            crate::benchmark_support::assert_equals_msg(
                "Wrong count for CloseIndex!",
                n,
                stats.get_count(),
            );
            n_checked += 1;
        }
    }
    crate::benchmark_support::assert_equals_msg("Missing some tasks to check!", 3, n_checked);
    Ok(())
}

fn disable_counting_lines(disable: bool) -> Vec<String> {
    let dis = if disable { "-" } else { "" };
    let mut v = reuters_lines(
        &[
            "content.source.log.step=30",
            "doc.term.vector=false",
            "content.source.forever=false",
            "directory=ByteBuffersDirectory",
            "doc.stored=false",
            "doc.tokenized=false",
            "task.max.depth.log=1",
        ],
        &["{ \"Rounds\"", "  ResetSystemErase"],
    );
    v.push(format!("  {dis}CreateIndex")); // optionally disable counting here
    v.push("  { \"AddDocs\"  AddDoc > : * ".to_string());
    v.push(format!("  {dis}  CloseIndex")); // optionally disable counting here (with extra blanks)
    v.push("}".to_string());
    v.push("RepSumByName".to_string());
    v
}

/// Test that we can change the Locale in the runData, that it is parsed as
/// we expect.
fn test_locale(_f: &mut PerfTasksLogicTest) -> Result<()> {
    // empty Locale: clear it (null)
    let benchmark = exec_benchmark(&get_locale_config(""))?;
    Assert::assert_true(benchmark.get_run_data().get_locale().is_none());
    // ROOT locale
    let benchmark = exec_benchmark(&get_locale_config("ROOT"))?;
    Assert::assert_equals_nullable(
        Some(Locale::of_parts("", "", "", "")),
        benchmark.get_run_data().get_locale(),
    );
    // specify just a language
    let benchmark = exec_benchmark(&get_locale_config("de"))?;
    Assert::assert_equals_nullable(
        Some(Locale::builder_from_language_tag("de")?),
        benchmark.get_run_data().get_locale(),
    );
    // specify language + country
    let benchmark = exec_benchmark(&get_locale_config("en-US"))?;
    Assert::assert_equals_nullable(
        Some(Locale::builder_from_language_tag("en-US")?),
        benchmark.get_run_data().get_locale(),
    );
    Ok(())
}

fn get_locale_config(locale_param: &str) -> Vec<String> {
    let new_locale = format!("  NewLocale({locale_param})");
    reuters_lines(
        &[
            "content.source.log.step=3",
            "content.source.forever=false",
            "directory=ByteBuffersDirectory",
        ],
        &[
            "{ \"Rounds\"",
            "  ResetSystemErase",
            &new_locale,
            "  CreateIndex",
            "  { \"AddDocs\"  AddDoc > : * ",
            "  NewRound",
            "} : 1",
        ],
    )
}

/// Test that we can create CollationAnalyzers.
fn test_collator(_f: &mut PerfTasksLogicTest) -> Result<()> {
    for tag in ["ROOT", "de", "en-US", "nn-NO"] {
        let benchmark = exec_benchmark(&get_collator_config(tag, "impl:jdk"))?;
        let locale = if tag == "ROOT" {
            Locale::of_parts("", "", "", "")
        } else {
            Locale::builder_from_language_tag(tag)?
        };
        let expected = CollationKeyAnalyzer::new(&RuleBasedCollator::get_instance(&locale));
        assert_equal_collation(
            &expected,
            &*benchmark.get_run_data().get_analyzer().expect("analyzer"),
            "foobar",
        )?;
    }
    Ok(())
}

fn assert_equal_collation(a1: &dyn Analyzer, a2: &dyn Analyzer, text: &str) -> Result<()> {
    let ts1 = a1.token_stream("bogus", Box::new(StringCharReader::new(text)))?;
    let ts2 = a2.token_stream("bogus", Box::new(StringCharReader::new(text)))?;
    let mut ts1 = ts1.borrow_mut();
    let mut ts2 = ts2.borrow_mut();
    ts1.reset()?;
    ts2.reset()?;
    ts1.attribute_source_mut()
        .add_attribute_interface::<dyn TermToBytesRefAttribute>()?;
    ts2.attribute_source_mut()
        .add_attribute_interface::<dyn TermToBytesRefAttribute>()?;
    Assert::assert_true(ts1.increment_token()?);
    Assert::assert_true(ts2.increment_token()?);
    let bytes1 = ts1
        .attribute_source()
        .get_attribute_interface::<dyn TermToBytesRefAttribute>()
        .unwrap()
        .get_bytes_ref();
    let bytes2 = ts2
        .attribute_source()
        .get_attribute_interface::<dyn TermToBytesRefAttribute>()
        .unwrap()
        .get_bytes_ref();
    Assert::assert_equals(bytes1, bytes2);
    Assert::assert_false(ts1.increment_token()?);
    Assert::assert_false(ts2.increment_token()?);
    ts1.close()?;
    ts2.close()?;
    Ok(())
}

fn get_collator_config(locale_param: &str, collation_param: &str) -> Vec<String> {
    let new_locale = format!("  NewLocale({locale_param})");
    let new_collation = format!("  NewCollationAnalyzer({collation_param})");
    reuters_lines(
        &[
            "content.source.log.step=3",
            "content.source.forever=false",
            "directory=ByteBuffersDirectory",
        ],
        &[
            "{ \"Rounds\"",
            "  ResetSystemErase",
            &new_locale,
            &new_collation,
            "  CreateIndex",
            "  { \"AddDocs\"  AddDoc > : * ",
            "  NewRound",
            "} : 1",
        ],
    )
}

fn analyzes_to(
    benchmark: &rucene_benchmark::by_task::benchmark::Benchmark,
    text: &str,
    output: &[&str],
) -> Result<()> {
    BaseTokenStreamTestCase::assert_analyzes_to(
        &*benchmark.get_run_data().get_analyzer().expect("analyzer"),
        text,
        output,
    )
}

/// Test that we can create shingle analyzers using AnalyzerFactory.
fn test_shingle_analyzer(_f: &mut PerfTasksLogicTest) -> Result<()> {
    let text = "one,two,three, four five six";

    // StandardTokenizer, maxShingleSize, and outputUnigrams
    let benchmark = exec_benchmark(&get_analyzer_factory_config(
        "shingle-analyzer",
        "StandardTokenizer,ShingleFilter",
    ))?;
    {
        let analyzer = benchmark.get_run_data().get_analyzer().expect("analyzer");
        let ts = analyzer.token_stream("bogus", Box::new(StringCharReader::new(text)))?;
        ts.borrow_mut().close()?;
    }
    analyzes_to(
        &benchmark,
        text,
        &[
            "one",
            "one two",
            "two",
            "two three",
            "three",
            "three four",
            "four",
            "four five",
            "five",
            "five six",
            "six",
        ],
    )?;
    // StandardTokenizer, maxShingleSize = 3, and outputUnigrams = false
    let benchmark = exec_benchmark(&get_analyzer_factory_config(
        "shingle-analyzer",
        "StandardTokenizer,ShingleFilter(maxShingleSize:3,outputUnigrams:false)",
    ))?;
    analyzes_to(
        &benchmark,
        text,
        &[
            "one two",
            "one two three",
            "two three",
            "two three four",
            "three four",
            "three four five",
            "four five",
            "four five six",
            "five six",
        ],
    )?;
    // WhitespaceTokenizer, default maxShingleSize and outputUnigrams
    let benchmark = exec_benchmark(&get_analyzer_factory_config(
        "shingle-analyzer",
        "WhitespaceTokenizer,ShingleFilter",
    ))?;
    analyzes_to(
        &benchmark,
        text,
        &[
            "one,two,three,",
            "one,two,three, four",
            "four",
            "four five",
            "five",
            "five six",
            "six",
        ],
    )?;
    // WhitespaceTokenizer, maxShingleSize=3 and outputUnigrams=false
    let benchmark = exec_benchmark(&get_analyzer_factory_config(
        "shingle-factory",
        "WhitespaceTokenizer,ShingleFilter(outputUnigrams:false,maxShingleSize:3)",
    ))?;
    analyzes_to(
        &benchmark,
        text,
        &[
            "one,two,three, four",
            "one,two,three, four five",
            "four five",
            "four five six",
            "five six",
        ],
    )
}

fn get_analyzer_factory_config(name: &str, params: &str) -> Vec<String> {
    let single_quote_escaped_name = name.replace('\'', "\\'");
    vec![
        "content.source=org.apache.lucene.benchmark.byTask.feeds.LineDocSource".to_string(),
        format!("docs.file={}", get_reuters20_lines_file()),
        format!(
            "work.dir={}",
            std::path::absolute(get_work_dir())
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        ), // Fix Windows path
        "content.source.forever=false".to_string(),
        "directory=ByteBuffersDirectory".to_string(),
        format!("AnalyzerFactory(name:'{single_quote_escaped_name}', {params})"),
        format!("NewAnalyzer('{single_quote_escaped_name}')"),
        "CreateIndex".to_string(),
        "{ \"AddDocs\"  AddDoc > : * ".to_string(),
    ]
}

fn test_analyzer_factory(_f: &mut PerfTasksLogicTest) -> Result<()> {
    let text = "Fortieth, Quarantième, Cuadragésimo";
    let benchmark = exec_benchmark(&get_analyzer_factory_config(
        "ascii folded, pattern replaced, standard tokenized, downcased, bigrammed.'analyzer'",
        concat!(
            "positionIncrementGap:100,offsetGap:1111,",
            "MappingCharFilter(mapping:'test-mapping-ISOLatin1Accent-partial.txt'),",
            "PatternReplaceCharFilterFactory(pattern:'e(\\\\\\\\S*)m',replacement:\"$1xxx$1\"),",
            "StandardTokenizer,LowerCaseFilter,NGramTokenFilter(minGramSize:2,maxGramSize:2)"
        ),
    ))?;
    analyzes_to(
        &benchmark,
        text,
        &[
            "fo", "or", "rt", "ti", "ie", "et", "th", "qu", "ua", "ar", "ra", "an", "nt", "ti",
            "ix", "xx", "xx", "xe", "cu", "ua", "ad", "dr", "ra", "ag", "gs", "si", "ix", "xx",
            "xx", "xs", "si", "io",
        ],
    )
}
