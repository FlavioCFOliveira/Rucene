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

//! The types declared in `RepSumByPrefTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::{Arc, Mutex};

use rucene::error::Result;

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::stats::report::Report;
use crate::by_task::stats::task_stats::TaskStats;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::by_task::tasks::report_task::{self, ReportTask};
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.RepSumByPrefTask`
/// (`public class RepSumByPrefTask extends ReportTask`): report by-name-prefix
/// statistics aggregated by name.
///
/// Other side effects: None.
pub struct RepSumByPrefTask {
    base: PerfTaskBase,
    /// `prefix`.
    prefix: Mutex<Option<String>>,
}

impl RepSumByPrefTask {
    /// `RepSumByPrefTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(RepSumByPrefTask {
            base: PerfTaskBase::new(run_data, "RepSumByPrefTask")?,
            prefix: Mutex::new(None),
        })
    }

    fn prefix(&self) -> Option<String> {
        jdk::lock(&self.prefix).clone()
    }

    /// `setPrefix(String prefix)`.
    pub fn set_prefix(&self, prefix: Option<String>) {
        *jdk::lock(&self.prefix) = prefix;
    }

    /// `reportSumByPrefix(List<TaskStats> taskStats)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` without a prefix; the `IllegalStateException`
    /// of aggregating.
    pub fn report_sum_by_prefix(&self, task_stats: &[TaskStats]) -> Result<Report> {
        let prefix = self.prefix();
        // aggregate by task name
        let (reported, p2) = ReportTask::aggregate(
            task_stats,
            |s| ReportTask::starts_with(&s.get_task().get_name(), prefix.as_deref()), // only ended tasks with proper name
            |s| s.get_task().get_name(),
        )?;
        // now generate report from secondary list p2
        Ok(ReportTask::gen_partial_report(
            self.base.get_run_data(),
            reported,
            &p2,
            task_stats.len() as i32,
        ))
    }
}

impl PerfTask for RepSumByPrefTask {
    report_task::report_task_perf_methods!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(RepSumByPrefTask {
            base: self.base.clone_base(),
            prefix: Mutex::new(self.prefix()),
        }))
    }

    fn do_logic(&self) -> Result<i32> {
        let stats = ReportTask::snapshot(&self.base.get_run_data().get_points().task_stats());
        let rp = self.report_sum_by_prefix(&stats)?;
        println!();
        println!(
            "------------> Report Sum By Prefix ({}) ({} about {} out of {})",
            self.prefix().as_deref().unwrap_or("null"),
            rp.get_size(),
            rp.get_reported(),
            rp.get_out_of()
        );
        println!("{}", rp.get_text());
        println!();
        Ok(0)
    }

    fn set_report_prefix(&self, prefix: &str) {
        self.set_prefix(Some(prefix.to_string()));
    }

    /// Equivalent to `RepSumByPrefTask.toString()`.
    fn task_to_string(&self) -> String {
        format!(
            "{} {}",
            perf_task::task_to_string(self),
            self.prefix().as_deref().unwrap_or("null")
        )
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        matches!(simple_class_name, "RepSumByPrefTask" | "ReportTask")
    }
}
