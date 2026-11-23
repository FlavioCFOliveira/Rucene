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

//! Port of `org.apache.lucene.benchmark.byTask.tasks.TestCreateIndexTask`.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::Version;
use rucene_benchmark::by_task::perf_run_data::PerfRunData;
use rucene_benchmark::by_task::tasks::close_index_task::CloseIndexTask;
use rucene_benchmark::by_task::tasks::create_index_task::CreateIndexTask;
use rucene_benchmark::by_task::tasks::perf_task::PerfTask;
use rucene_benchmark::jdk::properties::Properties;
use rucene_benchmark::jdk::system;

use crate::benchmark_support::search_support::ftest;
use crate::benchmark_support::{benchmark_class, get_work_dir, run_data_of};

/// The instance of `TestCreateIndexTask`.
#[derive(Default)]
pub struct CreateIndexTaskTest;

/// `TestCreateIndexTask`.
pub fn class_def() -> Arc<Class> {
    benchmark_class::<CreateIndexTaskTest>(
        "org.apache.lucene.benchmark.byTask.tasks.TestCreateIndexTask",
        vec![
            ftest("testInfoStream_SystemOutErr", test_info_stream_system_out_err),
            ftest("testInfoStream_File", test_info_stream_file),
            ftest("testNoMergePolicy", test_no_merge_policy),
            ftest("testNoMergeScheduler", test_no_merge_scheduler),
            ftest("testNoDeletionPolicy", test_no_deletion_policy),
        ],
    )
}

fn create_perf_run_data(info_stream_value: Option<&str>) -> Result<Arc<PerfRunData>> {
    let mut props = Properties::new();
    props.set_property("writer.version", Version::LATEST.to_string());
    props.set_property("print.props", "false"); // don't print anything
    props.set_property("directory", "ByteBuffersDirectory");
    if let Some(info_stream_value) = info_stream_value {
        props.set_property("writer.info.stream", info_stream_value);
    }
    run_data_of(props)
}

/// `new ByteArrayOutputStream()` installed as a `System.out`/`System.err`.
fn baos() -> (Arc<Mutex<Vec<u8>>>, system::PrintSink) {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let sink_buf = Arc::clone(&buf);
    let sink: system::PrintSink = Arc::new(move |b: &[u8]| sink_buf.lock().unwrap().extend_from_slice(b));
    (buf, sink)
}

fn test_info_stream_system_out_err(_f: &mut CreateIndexTaskTest) -> Result<()> {
    let (buf, sink) = baos();
    let cur_out = system::set_out(Some(sink));
    let result = (|| -> Result<()> {
        let run_data = create_perf_run_data(Some("SystemOut"))?;
        let cit = CreateIndexTask::new(Arc::clone(&run_data))?;
        cit.do_logic()?;
        CloseIndexTask::new(Arc::clone(&run_data))?.do_logic()?;
        Assert::assert_true(!buf.lock().unwrap().is_empty());
        Ok(())
    })();
    system::set_out(cur_out);
    result?;

    let (buf, sink) = baos();
    let cur_err = system::set_err(Some(sink));
    let result = (|| -> Result<()> {
        let run_data = create_perf_run_data(Some("SystemErr"))?;
        let cit = CreateIndexTask::new(Arc::clone(&run_data))?;
        cit.do_logic()?;
        CloseIndexTask::new(Arc::clone(&run_data))?.do_logic()?;
        Assert::assert_true(!buf.lock().unwrap().is_empty());
        Ok(())
    })();
    system::set_err(cur_err);
    result
}

fn test_info_stream_file(_f: &mut CreateIndexTaskTest) -> Result<()> {
    let out_file = get_work_dir().join("infoStreamTest");
    let path = std::path::absolute(&out_file)?.to_string_lossy().into_owned();
    let run_data = create_perf_run_data(Some(&path))?;
    CreateIndexTask::new(Arc::clone(&run_data))?.do_logic()?;
    CloseIndexTask::new(Arc::clone(&run_data))?.do_logic()?;
    Assert::assert_true(std::fs::metadata(&out_file)?.len() > 0);
    Ok(())
}

fn test_no_merge_policy(_f: &mut CreateIndexTaskTest) -> Result<()> {
    let run_data = create_perf_run_data(None)?;
    run_data.get_config().set("merge.policy", "org.apache.lucene.index.NoMergePolicy")?;
    CreateIndexTask::new(Arc::clone(&run_data))?.do_logic()?;
    CloseIndexTask::new(Arc::clone(&run_data))?.do_logic()?;
    Ok(())
}

fn test_no_merge_scheduler(_f: &mut CreateIndexTaskTest) -> Result<()> {
    let run_data = create_perf_run_data(None)?;
    run_data.get_config().set("merge.scheduler", "org.apache.lucene.index.NoMergeScheduler")?;
    CreateIndexTask::new(Arc::clone(&run_data))?.do_logic()?;
    CloseIndexTask::new(Arc::clone(&run_data))?.do_logic()?;
    Ok(())
}

fn test_no_deletion_policy(_f: &mut CreateIndexTaskTest) -> Result<()> {
    let run_data = create_perf_run_data(None)?;
    run_data.get_config().set("deletion.policy", "org.apache.lucene.index.NoDeletionPolicy")?;
    CreateIndexTask::new(Arc::clone(&run_data))?.do_logic()?;
    CloseIndexTask::new(Arc::clone(&run_data))?.do_logic()?;
    Ok(())
}
