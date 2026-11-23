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

//! The types declared in `RepAllTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::Arc;

use rucene::error::Result;

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::stats::report::Report;
use crate::by_task::stats::task_stats::TaskStats;
use crate::by_task::tasks::perf_task::{PerfTask, PerfTaskBase};
use crate::by_task::tasks::report_task::{self, ReportTask};

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.RepAllTask`
/// (`public class RepAllTask extends ReportTask`): report all statistics with no
/// aggregations.
///
/// Other side effects: None.
pub struct RepAllTask {
    base: PerfTaskBase,
}

impl RepAllTask {
    /// `RepAllTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(RepAllTask { base: PerfTaskBase::new(run_data, "RepAllTask")? })
    }

    /// `reportAll(List<TaskStats> taskStats)`: report detailed statistics as
    /// a string.
    pub fn report_all(&self, task_stats: &[TaskStats]) -> Report {
        let run_data = self.base.get_run_data();
        let longest_op = ReportTask::longest_op(task_stats);
        let mut first = true;
        let mut sb = String::new();
        sb.push_str(&ReportTask::table_title(run_data, &longest_op));
        sb.push_str(ReportTask::NEWLINE);
        let mut reported = 0;
        for stat in task_stats {
            if stat.get_elapsed() >= 0 {
                // consider only tasks that ended
                if !first {
                    sb.push_str(ReportTask::NEWLINE);
                }
                first = false;
                let mut line = ReportTask::task_report_line(run_data, &longest_op, stat);
                reported += 1;
                if task_stats.len() > 2 && reported % 2 == 0 {
                    line = line.replace("   ", " - ");
                }
                sb.push_str(&line);
            }
        }
        let reptxt = if reported == 0 { "No Matching Entries Were Found!".to_string() } else { sb };
        Report::new(reptxt, reported, reported, task_stats.len() as i32)
    }
}

impl PerfTask for RepAllTask {
    report_task::report_task_perf_methods!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(RepAllTask { base: self.base.clone_base() }))
    }

    fn do_logic(&self) -> Result<i32> {
        let stats = ReportTask::snapshot(&self.base.get_run_data().get_points().task_stats());
        let rp = self.report_all(&stats);
        println!();
        println!("------------> Report All ({} out of {})", rp.get_size(), rp.get_out_of());
        println!("{}", rp.get_text());
        println!();
        Ok(0)
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        matches!(simple_class_name, "RepAllTask" | "ReportTask")
    }
}
