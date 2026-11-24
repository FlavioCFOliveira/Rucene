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

//! The types declared in `ReportTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use rucene::error::Result;

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::stats::report::Report;
use crate::by_task::stats::task_stats::{TaskStats, TaskStatsRef};
use crate::by_task::utils::format::Format;
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.ReportTask`
/// (`public abstract class ReportTask extends PerfTask`): report (abstract)
/// task - all report tasks extend this task.
///
/// Its `shouldNeverLogAtStart()` and `shouldNotRecordStats()` answer `true`
/// in every report task; the report-generating members are the associated
/// functions below, given the task's run data.
pub struct ReportTask;

/// A `LinkedHashMap<String, TaskStats>`: insertion-ordered.
pub type LinkedTaskStats = Vec<(String, TaskStats)>;

impl ReportTask {
    /// `newline`.
    pub const NEWLINE: &'static str = jdk::NEW_LINE;
    /// `OP`: get a textual summary of the benchmark results, average from
    /// all test runs.
    pub const OP: &'static str = "Operation  ";
    /// `ROUND`.
    pub const ROUND: &'static str = " round";
    /// `RUNCNT`.
    pub const RUNCNT: &'static str = "   runCnt";
    /// `RECCNT`.
    pub const RECCNT: &'static str = "   recsPerRun";
    /// `RECSEC`.
    pub const RECSEC: &'static str = "        rec/s";
    /// `ELAPSED`.
    pub const ELAPSED: &'static str = "  elapsedSec";
    /// `USEDMEM`.
    pub const USEDMEM: &'static str = "    avgUsedMem";
    /// `TOTMEM`.
    pub const TOTMEM: &'static str = "    avgTotalMem";
    /// `COLS`.
    pub const COLS: [&'static str; 6] = [
        Self::RUNCNT,
        Self::RECCNT,
        Self::RECSEC,
        Self::ELAPSED,
        Self::USEDMEM,
        Self::TOTMEM,
    ];

    /// `tableTitle(String longestOp)`: compute a title line for a report
    /// table; `longest_op` is the longest op name in the table.
    pub fn table_title(run_data: &PerfRunData, longest_op: &str) -> String {
        let mut sb = String::new();
        sb.push_str(&Format::format_str(Self::OP, longest_op));
        sb.push_str(Self::ROUND);
        sb.push_str(&run_data.get_config().get_cols_names_for_vals_by_round());
        for col in Self::COLS {
            sb.push_str(col);
        }
        sb
    }

    /// `longestOp(Iterable<TaskStats> taskStats)`: find the longest op name
    /// out of completed tasks.
    pub fn longest_op<'a>(task_stats: impl IntoIterator<Item = &'a TaskStats>) -> String {
        let mut longest = Self::OP.to_string();
        for stat in task_stats {
            if stat.get_elapsed() >= 0 {
                // consider only tasks that ended
                let name = stat.get_task().get_name();
                if jdk::utf16_len(&name) > jdk::utf16_len(&longest) {
                    longest = name;
                }
            }
        }
        longest
    }

    /// `taskReportLine(String longestOp, TaskStats stat)`: compute a report
    /// line for the given task stat.
    pub fn task_report_line(run_data: &PerfRunData, longest_op: &str, stat: &TaskStats) -> String {
        let task = stat.get_task();
        let mut sb = String::new();
        sb.push_str(&Format::format_str(&task.get_name(), longest_op));
        let round = if stat.get_round() >= 0 {
            stat.get_round().to_string()
        } else {
            "-".to_string()
        };
        sb.push_str(&Format::format_padd_left(&round, Self::ROUND));
        sb.push_str(
            &run_data
                .get_config()
                .get_cols_values_for_vals_by_round(stat.get_round()),
        );
        sb.push_str(&Format::format_int(stat.get_num_runs(), Self::RUNCNT));
        sb.push_str(&Format::format_int(
            stat.get_count() / stat.get_num_runs(),
            Self::RECCNT,
        ));
        let elapsed = if stat.get_elapsed() > 0 {
            stat.get_elapsed()
        } else {
            1
        }; // assume at least 1ms
        sb.push_str(&Format::format_float(
            2,
            (f64::from(stat.get_count()) * 1000.0 / elapsed as f64) as f32,
            Self::RECSEC,
        ));
        sb.push_str(&Format::format_float(
            2,
            stat.get_elapsed() as f32 / 1000.0,
            Self::ELAPSED,
        ));
        sb.push_str(&Format::format_float(
            0,
            stat.get_max_used_mem() as f32 / stat.get_num_runs() as f32,
            Self::USEDMEM,
        ));
        sb.push_str(&Format::format_float(
            0,
            stat.get_max_tot_mem() as f32 / stat.get_num_runs() as f32,
            Self::TOTMEM,
        ));
        sb
    }

    /// `genPartialReport(int reported, LinkedHashMap<String, TaskStats>
    /// partOfTasks, int totalSize)`.
    pub fn gen_partial_report(
        run_data: &PerfRunData,
        reported: i32,
        part_of_tasks: &LinkedTaskStats,
        total_size: i32,
    ) -> Report {
        let longet_op = Self::longest_op(part_of_tasks.iter().map(|(_, s)| s));
        let mut first = true;
        let mut sb = String::new();
        sb.push_str(&Self::table_title(run_data, &longet_op));
        sb.push_str(Self::NEWLINE);
        let mut line_num = 0;
        for (_, stat) in part_of_tasks {
            if !first {
                sb.push_str(Self::NEWLINE);
            }
            first = false;
            let mut line = Self::task_report_line(run_data, &longet_op, stat);
            line_num += 1;
            if part_of_tasks.len() > 2 && line_num % 2 == 0 {
                line = line.replace("   ", " - ");
            }
            sb.push_str(&line);
            if let Some(by_time) = stat.get_counts_by_time() {
                sb.push_str(Self::NEWLINE);
                let end = by_time.iter().rposition(|&c| c != 0);
                if let Some(end) = end {
                    sb.push_str("  by time:");
                    for c in &by_time[..end] {
                        sb.push(' ');
                        sb.push_str(&c.to_string());
                    }
                }
            }
        }
        let reptxt = if reported == 0 {
            "No Matching Entries Were Found!".to_string()
        } else {
            sb
        };
        Report::new(reptxt, part_of_tasks.len() as i32, reported, total_size)
    }

    /// The `TaskStats` values of `points.taskStats()`, copied out of their
    /// locks.
    pub fn snapshot(task_stats: &[TaskStatsRef]) -> Vec<TaskStats> {
        task_stats.iter().map(|s| jdk::lock(s).clone()).collect()
    }

    /// The aggregation loop the summing reports share: every ended stat
    /// whose name passes `filter` is added to the entry `key(stat)` (a
    /// clone of the first one, `stat2.add(stat1)` for the next ones).
    /// Returns `(reported, p2)`.
    ///
    /// # Errors
    ///
    /// The `IllegalStateException` of `TaskStats.add`.
    pub fn aggregate(
        task_stats: &[TaskStats],
        filter: impl Fn(&TaskStats) -> Result<bool>,
        key: impl Fn(&TaskStats) -> String,
    ) -> Result<(i32, LinkedTaskStats)> {
        let mut reported = 0;
        let mut p2: LinkedTaskStats = Vec::new();
        for stat1 in task_stats {
            if stat1.get_elapsed() >= 0 && filter(stat1)? {
                reported += 1;
                let name = key(stat1);
                match p2.iter_mut().find(|(k, _)| *k == name) {
                    None => p2.push((name, stat1.clone())),
                    Some((_, stat2)) => stat2.add(stat1)?,
                }
            }
        }
        Ok((reported, p2))
    }

    /// `prefix` of a by-prefix report: `name.startsWith(prefix)`, Java's
    /// `NullPointerException` when no prefix was set.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for a `null` prefix.
    pub fn starts_with(name: &str, prefix: Option<&str>) -> Result<bool> {
        match prefix {
            Some(p) => Ok(name.starts_with(p)),
            None => Err(rucene::error::LuceneError::null_pointer()),
        }
    }
}

/// The `PerfTask` members every report task shares: `shouldNeverLogAtStart()`
/// and `shouldNotRecordStats()` answer `true`.
macro_rules! report_task_perf_methods {
    () => {
        fn base(&self) -> &$crate::by_task::tasks::perf_task::PerfTaskBase {
            &self.base
        }

        fn as_any(&self) -> &dyn ::std::any::Any {
            self
        }

        fn should_never_log_at_start(&self) -> bool {
            true
        }

        fn should_not_record_stats(&self) -> bool {
            true
        }
    };
}
pub(crate) use report_task_perf_methods;
