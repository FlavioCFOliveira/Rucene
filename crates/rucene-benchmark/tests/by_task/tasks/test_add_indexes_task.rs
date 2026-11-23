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

//! Port of `org.apache.lucene.benchmark.byTask.tasks.TestAddIndexesTask`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::store::ByteBuffersDirectory;
use rucene::util::Version;
use rucene_benchmark::by_task::perf_run_data::PerfRunData;
use rucene_benchmark::by_task::tasks::add_indexes_task::AddIndexesTask;
use rucene_benchmark::by_task::tasks::close_index_task::CloseIndexTask;
use rucene_benchmark::by_task::tasks::create_index_task::CreateIndexTask;
use rucene_benchmark::by_task::tasks::perf_task::PerfTask;
use rucene_benchmark::jdk::properties::Properties;
use rucene_test_framework::util::LuceneTestCase;

use crate::benchmark_support::search_support::{ftest, static_hook};
use crate::benchmark_support::{benchmark_class, run_data_of};

/// `private static Path testDir, inputDir`.
static INPUT_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// The instance of `TestAddIndexesTask`.
#[derive(Default)]
pub struct AddIndexesTaskTest;

/// `TestAddIndexesTask`.
pub fn class_def() -> Arc<Class> {
    benchmark_class::<AddIndexesTaskTest>(
        "org.apache.lucene.benchmark.byTask.tasks.TestAddIndexesTask",
        vec![
            static_hook(
                "beforeClassAddIndexesTaskTest",
                BeforeClass,
                before_class_add_indexes_task_test,
            ),
            static_hook(
                "afterClassAddIndexesTaskTest",
                AfterClass,
                after_class_add_indexes_task_test,
            ),
            ftest("testAddIndexesDefault", test_add_indexes_default),
            ftest("testAddIndexesDir", test_add_indexes_dir),
            ftest("testAddIndexesReader", test_add_indexes_reader),
        ],
    )
}

fn before_class_add_indexes_task_test() -> Result<()> {
    let test_dir = LuceneTestCase::create_temp_dir_prefix("addIndexesTask");
    let input_dir = test_dir.resolve_str("input");
    let tmp_dir = LuceneTestCase::new_fs_directory(&input_dir);
    let result = (|| -> Result<()> {
        let writer = IndexWriter::new(
            tmp_dir.as_directory(),
            IndexWriterConfig::with_analyzer(None),
        )?;
        for _ in 0..10 {
            writer.add_document(&Document::new())?;
        }
        writer.close()
    })();
    tmp_dir.close()?;
    result?;
    *INPUT_DIR.lock().unwrap() = Some(input_dir.as_std_path().to_path_buf());
    Ok(())
}

fn after_class_add_indexes_task_test() -> Result<()> {
    *INPUT_DIR.lock().unwrap() = None;
    Ok(())
}

fn create_perf_run_data() -> Result<Arc<PerfRunData>> {
    let input_dir = INPUT_DIR.lock().unwrap().clone().expect("beforeClass ran");
    let mut props = Properties::new();
    props.set_property("writer.version", Version::LATEST.to_string());
    props.set_property("print.props", "false"); // don't print anything
    props.set_property("directory", "ByteBuffersDirectory");
    props.set_property(
        AddIndexesTask::ADDINDEXES_INPUT_DIR,
        std::path::absolute(&input_dir)?
            .to_string_lossy()
            .into_owned(),
    );
    run_data_of(props)
}

fn assert_index(run_data: &PerfRunData) -> Result<()> {
    let task_dir = run_data.get_directory().expect("directory");
    // assertSame(ByteBuffersDirectory.class, taskDir.getClass())
    Assert::assert_true(
        task_dir
            .as_any()
            .is_some_and(|a| a.is::<ByteBuffersDirectory>()),
    );
    let r = directory_reader::open(task_dir)?;
    Assert::assert_equals(10, r.num_docs());
    r.close()
}

fn run(params: Option<&str>) -> Result<()> {
    let run_data = create_perf_run_data()?;
    CreateIndexTask::new(Arc::clone(&run_data))?.do_logic()?;
    let task = AddIndexesTask::new(Arc::clone(&run_data))?;
    task.setup()?;
    if let Some(params) = params {
        task.set_params(params)?;
    }
    task.do_logic()?;
    CloseIndexTask::new(Arc::clone(&run_data))?.do_logic()?;
    assert_index(&run_data)?;
    run_data.close()
}

fn test_add_indexes_default(_f: &mut AddIndexesTaskTest) -> Result<()> {
    run(None)
}

fn test_add_indexes_dir(_f: &mut AddIndexesTaskTest) -> Result<()> {
    run(Some("true"))
}

fn test_add_indexes_reader(_f: &mut AddIndexesTaskTest) -> Result<()> {
    run(Some("false"))
}
