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

//! Port of `org.apache.lucene.benchmark.byTask.tasks.TestCommitIndexTask`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::segment_infos::SegmentInfos;
use rucene::util::Version;
use rucene_benchmark::by_task::perf_run_data::PerfRunData;
use rucene_benchmark::by_task::tasks::close_index_task::CloseIndexTask;
use rucene_benchmark::by_task::tasks::commit_index_task::CommitIndexTask;
use rucene_benchmark::by_task::tasks::create_index_task::CreateIndexTask;
use rucene_benchmark::by_task::tasks::open_reader_task::OpenReaderTask;
use rucene_benchmark::by_task::tasks::perf_task::PerfTask;
use rucene_benchmark::jdk::properties::Properties;

use crate::benchmark_support::search_support::ftest;
use crate::benchmark_support::{benchmark_class, run_data_of};

/// The instance of `TestCommitIndexTask`.
#[derive(Default)]
pub struct CommitIndexTaskTest;

/// `TestCommitIndexTask`.
pub fn class_def() -> Arc<Class> {
    benchmark_class::<CommitIndexTaskTest>(
        "org.apache.lucene.benchmark.byTask.tasks.TestCommitIndexTask",
        vec![ftest("testNoParams", test_no_params), ftest("testCommitData", test_commit_data)],
    )
}

fn create_perf_run_data() -> Result<Arc<PerfRunData>> {
    let mut props = Properties::new();
    props.set_property("writer.version", Version::LATEST.to_string());
    props.set_property("print.props", "false"); // don't print anything
    props.set_property("directory", "ByteBuffersDirectory");
    run_data_of(props)
}

fn test_no_params(_f: &mut CommitIndexTaskTest) -> Result<()> {
    let run_data = create_perf_run_data()?;
    CreateIndexTask::new(Arc::clone(&run_data))?.do_logic()?;
    CommitIndexTask::new(Arc::clone(&run_data))?.do_logic()?;
    CloseIndexTask::new(Arc::clone(&run_data))?.do_logic()?;
    Ok(())
}

fn test_commit_data(_f: &mut CommitIndexTaskTest) -> Result<()> {
    let run_data = create_perf_run_data()?;
    CreateIndexTask::new(Arc::clone(&run_data))?.do_logic()?;
    let task = CommitIndexTask::new(Arc::clone(&run_data))?;
    task.set_params("params")?;
    task.do_logic()?;
    let infos = SegmentInfos::read_latest_commit(run_data.get_directory().expect("directory"))?;
    Assert::assert_equals_nullable(
        Some("params"),
        infos.user_data().get(OpenReaderTask::USER_DATA).map(String::as_str),
    );
    CloseIndexTask::new(Arc::clone(&run_data))?.do_logic()?;
    Ok(())
}
