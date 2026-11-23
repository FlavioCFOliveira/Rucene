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

//! Port of `org.apache.lucene.benchmark.byTask.tasks.TestPerfTask`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene_benchmark::by_task::perf_run_data::PerfRunData;
use rucene_benchmark::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use rucene_benchmark::jdk::properties::Properties;

use crate::benchmark_support::search_support::ftest;
use crate::benchmark_support::{benchmark_class, run_data_of};

/// `private static final class MyPerfTask extends PerfTask`.
struct MyPerfTask {
    base: PerfTaskBase,
}

impl MyPerfTask {
    fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(MyPerfTask { base: PerfTaskBase::new(run_data, "MyPerfTask")? })
    }

    /// `getLogStep()`.
    fn get_log_step(&self) -> i32 {
        self.base.log_step()
    }
}

impl PerfTask for MyPerfTask {
    fn base(&self) -> &PerfTaskBase {
        &self.base
    }

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(MyPerfTask { base: self.base.clone_base() }))
    }

    fn do_logic(&self) -> Result<i32> {
        Ok(0)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The instance of `TestPerfTask`.
#[derive(Default)]
pub struct PerfTaskTest;

/// `TestPerfTask`.
pub fn class_def() -> Arc<Class> {
    benchmark_class::<PerfTaskTest>(
        "org.apache.lucene.benchmark.byTask.tasks.TestPerfTask",
        vec![ftest("testLogStep", test_log_step)],
    )
}

fn create_perf_run_data(
    set_log_step: bool,
    log_step_val: i32,
    set_task_log_step: bool,
    task_log_step_val: i32,
) -> Result<Arc<PerfRunData>> {
    let mut props = Properties::new();
    if set_log_step {
        props.set_property("log.step", log_step_val.to_string());
    }
    if set_task_log_step {
        props.set_property("log.step.MyPerf", task_log_step_val.to_string());
    }
    props.set_property("directory", "ByteBuffersDirectory"); // no accidental FS dir.
    run_data_of(props)
}

fn do_log_step_test(
    set_log_step: bool,
    log_step_val: i32,
    set_task_log_step: bool,
    task_log_step_val: i32,
    exp_log_step_value: i32,
) -> Result<()> {
    let run_data = create_perf_run_data(set_log_step, log_step_val, set_task_log_step, task_log_step_val)?;
    let mpt = MyPerfTask::new(run_data)?;
    Assert::assert_equals(exp_log_step_value, mpt.get_log_step());
    Ok(())
}

fn test_log_step(_f: &mut PerfTaskTest) -> Result<()> {
    do_log_step_test(false, -1, false, -1, perf_task::DEFAULT_LOG_STEP)?;
    do_log_step_test(true, -1, false, -1, i32::MAX)?;
    do_log_step_test(true, 100, false, -1, 100)?;
    do_log_step_test(false, -1, true, -1, i32::MAX)?;
    do_log_step_test(false, -1, true, 100, 100)?;
    Ok(())
}
