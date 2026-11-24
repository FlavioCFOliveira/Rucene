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

//! The types declared in `Points.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.stats`.

use std::sync::{Arc, Mutex};

use crate::by_task::stats::task_stats::{TaskStats, TaskStatsRef};
use crate::by_task::tasks::perf_task::PerfTask;
use crate::by_task::utils::config::Config;
use crate::jdk;

#[derive(Default)]
struct State {
    /// stat points ordered by their start time.
    points: Vec<TaskStatsRef>,
    next_task_run_num: i32,
    current_stats: Option<TaskStatsRef>,
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.stats.Points`: test run
/// data points collected as the test proceeds.
#[derive(Default)]
pub struct Points {
    state: Mutex<State>,
}

impl Points {
    /// `Points(Config config)`: create a Points statistics object.
    pub fn new(_config: &Config) -> Self {
        Self::default()
    }

    /// `taskStats()`: the current task stats. the actual task stats are
    /// returned, so caller should not modify this task stats.
    pub fn task_stats(&self) -> Vec<TaskStatsRef> {
        jdk::lock(&self.state).points.clone()
    }

    /// `markTaskStart(PerfTask task, int round)`: mark that a task is
    /// starting. Create a task stats for it and store it as a point.
    pub fn mark_task_start(&self, task: Arc<dyn PerfTask>, round: i32) -> TaskStatsRef {
        let mut s = jdk::lock(&self.state);
        let run_num = s.next_task_run_num;
        s.next_task_run_num += 1;
        let stats = Arc::new(Mutex::new(TaskStats::new(task, run_num, round)));
        s.current_stats = Some(Arc::clone(&stats));
        s.points.push(Arc::clone(&stats));
        stats
    }

    /// `getCurrentStats()`.
    pub fn get_current_stats(&self) -> Option<TaskStatsRef> {
        jdk::lock(&self.state).current_stats.clone()
    }

    /// `markTaskEnd(TaskStats stats, int count)`: mark the end of a task.
    pub fn mark_task_end(&self, stats: &TaskStatsRef, count: i32) {
        let s = jdk::lock(&self.state);
        let mut stats = jdk::lock(stats);
        let num_parallel_tasks = s.next_task_run_num - 1 - stats.get_task_run_num();
        // note: if the stats were cleared, might be that this stats object is
        // no longer in points, but this is just ok.
        stats.mark_end(num_parallel_tasks, count);
    }

    /// `clearData()`: clear all data, prepare for more tests.
    pub fn clear_data(&self) {
        jdk::lock(&self.state).points.clear();
    }
}
