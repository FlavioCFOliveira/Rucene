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

//! The types declared in `RepSumByNameTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::Arc;

use rucene::error::Result;

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::stats::report::Report;
use crate::by_task::stats::task_stats::TaskStats;
use crate::by_task::tasks::perf_task::{PerfTask, PerfTaskBase};
use crate::by_task::tasks::report_task::{self, ReportTask};

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.RepSumByNameTask`
/// (`public class RepSumByNameTask extends ReportTask`): report all statistics
/// aggregated by name.
///
/// Other side effects: None.
pub struct RepSumByNameTask {
    base: PerfTaskBase,
}

impl RepSumByNameTask {
    /// `RepSumByNameTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(RepSumByNameTask { base: PerfTaskBase::new(run_data, "RepSumByNameTask")? })
    }

    /// `reportSumByName(List<TaskStats> taskStats)`: report statistics as a
    /// string, aggregate for tasks named the same.
    ///
    /// # Errors
    ///
    /// The `IllegalStateException` of aggregating.
    pub fn report_sum_by_name(&self, task_stats: &[TaskStats]) -> Result<Report> {
        // aggregate by task name
        let (reported, p2) = ReportTask::aggregate(task_stats, |_| Ok(true), |s| s.get_task().get_name())?;
        // now generate report from secondary list p2
        Ok(ReportTask::gen_partial_report(self.base.get_run_data(), reported, &p2, task_stats.len() as i32))
    }
}

impl PerfTask for RepSumByNameTask {
    report_task::report_task_perf_methods!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(RepSumByNameTask { base: self.base.clone_base() }))
    }

    fn do_logic(&self) -> Result<i32> {
        let stats = ReportTask::snapshot(&self.base.get_run_data().get_points().task_stats());
        let rp = self.report_sum_by_name(&stats)?;
        println!();
        println!(
            "------------> Report Sum By (any) Name ({} about {} out of {})",
            rp.get_size(),
            rp.get_reported(),
            rp.get_out_of()
        );
        println!("{}", rp.get_text());
        println!();
        Ok(0)
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        matches!(simple_class_name, "RepSumByNameTask" | "ReportTask")
    }
}
