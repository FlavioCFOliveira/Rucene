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

//! The types declared in `TaskStats.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.stats`.

use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};

use crate::by_task::tasks::perf_task::PerfTask;
use crate::jdk::{self, Runtime};

/// A `TaskStats` object as Java shares it: the one in `Points` and the
/// reference the running task marks the end on.
pub type TaskStatsRef = Arc<Mutex<TaskStats>>;

/// Equivalent to `org.apache.lucene.benchmark.byTask.stats.TaskStats`
/// (`public class TaskStats implements Cloneable`): statistics for a task
/// run. The same task can run more than once, but, if that task records
/// statistics, each run would create its own TaskStats.
///
/// The memory figures are those of [`Runtime`], a declared divergence.
#[derive(Clone)]
pub struct TaskStats {
    /// task for which data was collected
    task: Arc<dyn PerfTask>,
    /// round in which task run started
    round: i32,
    /// task start time
    start: i64,
    /// task elapsed time. elapsed >= 0 indicates run completion!
    elapsed: i64,
    /// max tot mem during task
    max_tot_mem: i64,
    /// max used mem during task
    max_used_mem: i64,
    /// serial run number of this task run in the perf run
    task_run_num: i32,
    /// number of other tasks that started to run while this task was still
    /// running
    num_parallel_tasks: i32,
    /// number of work items done by this task. For indexing that can be
    /// number of docs added. For warming that can be number of scanned
    /// items, etc. For repeating tasks, this is a sum over repetitions.
    count: i32,
    /// Number of similar tasks aggregated into this record. Used when
    /// summing up on few runs/instances of similar tasks.
    num_runs: i32,
    counts_by_time: Option<Vec<i32>>,
    counts_by_time_step_msec: i64,
}

impl TaskStats {
    /// `TaskStats(PerfTask task, int taskRunNum, int round)`: create a run
    /// data for a task that is starting now. To be called from `Points`.
    pub(crate) fn new(task: Arc<dyn PerfTask>, task_run_num: i32, round: i32) -> Self {
        let max_tot_mem = Runtime::total_memory();
        let max_used_mem = max_tot_mem - Runtime::free_memory();
        TaskStats {
            task,
            round,
            start: jdk::current_time_millis(),
            elapsed: -1,
            max_tot_mem,
            max_used_mem,
            task_run_num,
            num_parallel_tasks: 0,
            count: 0,
            num_runs: 1,
            counts_by_time: None,
            counts_by_time_step_msec: 0,
        }
    }

    /// `markEnd(int numParallelTasks, int count)`: mark the end of a task.
    pub(crate) fn mark_end(&mut self, num_parallel_tasks: i32, count: i32) {
        self.elapsed = jdk::current_time_millis() - self.start;
        let tot_mem = Runtime::total_memory();
        if tot_mem > self.max_tot_mem {
            self.max_tot_mem = tot_mem;
        }
        let used_mem = tot_mem - Runtime::free_memory();
        if used_mem > self.max_used_mem {
            self.max_used_mem = used_mem;
        }
        self.num_parallel_tasks = num_parallel_tasks;
        self.count = count;
    }

    /// `setCountsByTime(int[] counts, long msecStep)`.
    pub fn set_counts_by_time(&mut self, counts: Vec<i32>, msec_step: i64) {
        self.counts_by_time = Some(counts);
        self.counts_by_time_step_msec = msec_step;
    }

    /// `getCountsByTime()`.
    pub fn get_counts_by_time(&self) -> Option<&[i32]> {
        self.counts_by_time.as_deref()
    }

    /// `getCountsByTimeStepMSec()`.
    pub fn get_counts_by_time_step_msec(&self) -> i64 {
        self.counts_by_time_step_msec
    }

    /// `getTaskRunNum()`: the task run number.
    pub fn get_task_run_num(&self) -> i32 {
        self.task_run_num
    }

    /// `getCount()`: count of work items done by this task.
    pub fn get_count(&self) -> i32 {
        self.count
    }

    /// `getElapsed()`: elapsed time in millis.
    pub fn get_elapsed(&self) -> i64 {
        self.elapsed
    }

    /// `getMaxTotMem()`.
    pub fn get_max_tot_mem(&self) -> i64 {
        self.max_tot_mem
    }

    /// `getMaxUsedMem()`.
    pub fn get_max_used_mem(&self) -> i64 {
        self.max_used_mem
    }

    /// `getNumParallelTasks()`.
    pub fn get_num_parallel_tasks(&self) -> i32 {
        self.num_parallel_tasks
    }

    /// `getTask()`.
    pub fn get_task(&self) -> &Arc<dyn PerfTask> {
        &self.task
    }

    /// `getNumRuns()`.
    pub fn get_num_runs(&self) -> i32 {
        self.num_runs
    }

    /// `add(TaskStats stat2)`: add data from another stat, for aggregation.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` when the by-time counts differ in step or
    /// length.
    pub fn add(&mut self, stat2: &TaskStats) -> Result<()> {
        self.num_runs += stat2.get_num_runs();
        self.elapsed += stat2.get_elapsed();
        self.max_tot_mem += stat2.get_max_tot_mem();
        self.max_used_mem += stat2.get_max_used_mem();
        self.count += stat2.get_count();
        if self.round != stat2.round {
            self.round = -1; // no meaning if aggregating tasks of different round.
        }
        if let (Some(counts), Some(counts2)) = (self.counts_by_time.as_mut(), stat2.counts_by_time.as_ref()) {
            if self.counts_by_time_step_msec != stat2.counts_by_time_step_msec {
                return Err(LuceneError::illegal_state("different by-time msec step"));
            }
            if counts.len() != counts2.len() {
                return Err(LuceneError::illegal_state("different by-time msec count"));
            }
            for (c, c2) in counts.iter_mut().zip(counts2) {
                *c += c2;
            }
        }
        Ok(())
    }

    /// `getRound()`: the round number.
    pub fn get_round(&self) -> i32 {
        self.round
    }
}

impl std::fmt::Display for TaskStats {
    /// `toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {} {}", self.task.get_name(), self.count, self.elapsed)
    }
}
