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

//! The types declared in `TaskSequence.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use rucene::error::Result;

use crate::by_task::feeds::no_more_data_exception::NoMoreDataException;
use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, run_and_maybe_stats, PerfTask, PerfTaskBase, NEW_LINE};
use crate::constants::Constants;
use crate::jdk;

/// The mutable fields of `TaskSequence`.
#[derive(Clone)]
struct SeqState {
    tasks: Vec<Arc<dyn PerfTask>>,
    repetitions: i32,
    parallel: bool,
    let_child_report: bool,
    rate: i32,
    /// rate, if set, is, by default, be sec.
    per_min: bool,
    seq_name: String,
    tasks_array: Option<Vec<Arc<dyn PerfTask>>>,
    any_exhaustible_tasks: bool,
    /// true if we run for fixed time
    fixed_time: bool,
    /// how long to run for
    run_time_sec: f64,
    counts_by_time: Option<Vec<i32>>,
    running_parallel_tasks: Option<Vec<Arc<dyn PerfTask>>>,
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.TaskSequence`:
/// sequence of parallel or sequential tasks.
///
/// **Divergence.** Java raises the priority of a background task's thread
/// by its `&N` delta (`TaskSequence.java:166-167`); Rust's threads have no
/// priority, so the delta is kept (and printed) but not applied.
pub struct TaskSequence {
    base: PerfTaskBase,
    state: Mutex<SeqState>,
    parent: Option<Weak<TaskSequence>>,
    exhausted: AtomicBool,
    reset_exhausted: AtomicBool,
    /// to not collapse external sequence named in alg.
    collapsable: bool,
    log_by_time_msec: i64,
    me: Weak<TaskSequence>,
}

impl TaskSequence {
    /// `REPEAT_EXHAUST`.
    pub const REPEAT_EXHAUST: i32 = -2;

    /// `TaskSequence(PerfRunData runData, String name, TaskSequence parent,
    /// boolean parallel)`.
    ///
    /// # Errors
    ///
    /// The exception of reading the configuration.
    pub fn new(
        run_data: Arc<PerfRunData>,
        name: Option<&str>,
        parent: Option<&Arc<TaskSequence>>,
        parallel: bool,
    ) -> Result<Arc<TaskSequence>> {
        let base = PerfTaskBase::new(Arc::clone(&run_data), "TaskSequence")?;
        let collapsable = name.is_none();
        let name = name.map(str::to_string).unwrap_or_else(|| if parallel { "Par" } else { "Seq" }.to_string());
        base.set_name(name);
        let log_by_time_msec = i64::from(run_data.get_config().get_int("report.time.step.msec", 0)?);
        let seq = Arc::new_cyclic(|me| TaskSequence {
            base,
            state: Mutex::new(SeqState {
                tasks: Vec::new(),
                repetitions: 1,
                parallel,
                let_child_report: true,
                rate: 0,
                per_min: false,
                seq_name: String::new(),
                tasks_array: None,
                any_exhaustible_tasks: false,
                fixed_time: false,
                run_time_sec: 0.0,
                counts_by_time: None,
                running_parallel_tasks: None,
            }),
            parent: parent.map(Arc::downgrade),
            exhausted: AtomicBool::new(false),
            reset_exhausted: AtomicBool::new(false),
            collapsable,
            log_by_time_msec,
            me: me.clone(),
        });
        seq.set_sequence_name();
        Ok(seq)
    }

    fn lock(&self) -> MutexGuard<'_, SeqState> {
        jdk::lock(&self.state)
    }

    /// This sequence as an `Arc` of its own type.
    pub fn as_arc(&self) -> Arc<TaskSequence> {
        self.me.upgrade().expect("a TaskSequence is shared as an Arc")
    }

    /// This sequence as the shared task its parent holds.
    pub fn as_task(&self) -> Arc<dyn PerfTask> {
        self.me.upgrade().expect("a TaskSequence is shared as an Arc")
    }

    /// `initTasksArray()`.
    fn init_tasks_array(&self) {
        let mut s = self.lock();
        if s.tasks_array.is_none() {
            let tasks = s.tasks.clone();
            let mut any = s.any_exhaustible_tasks;
            for t in &tasks {
                any |= t.is_instance_of("ResetInputsTask");
                any |= t.as_any().is::<TaskSequence>();
            }
            s.any_exhaustible_tasks = any;
            s.tasks_array = Some(tasks);
        }
        if !s.parallel && self.log_by_time_msec != 0 && !s.let_child_report {
            s.counts_by_time = Some(vec![0; 1]);
        }
    }

    /// `isParallel()`.
    pub fn is_parallel(&self) -> bool {
        self.lock().parallel
    }

    /// `getRepetitions()`.
    pub fn get_repetitions(&self) -> i32 {
        self.lock().repetitions
    }

    /// `setRunTime(double sec)`.
    ///
    /// # Errors
    ///
    /// None.
    pub fn set_run_time(&self, sec: f64) -> Result<()> {
        let mut s = self.lock();
        s.run_time_sec = sec;
        s.fixed_time = true;
        Ok(())
    }

    /// `setRepetitions(int repetitions)`.
    ///
    /// # Errors
    ///
    /// `Exception("REPEAT_EXHAUST is not allowed for parallel tasks")`.
    pub fn set_repetitions(&self, repetitions: i32) -> Result<()> {
        {
            let mut s = self.lock();
            s.fixed_time = false;
            s.repetitions = repetitions;
            if repetitions == Self::REPEAT_EXHAUST && s.parallel {
                return Err(jdk::exception("REPEAT_EXHAUST is not allowed for parallel tasks"));
            }
        }
        self.set_sequence_name();
        Ok(())
    }

    /// `getParent()`.
    pub fn get_parent(&self) -> Option<Arc<TaskSequence>> {
        self.parent.as_ref().and_then(Weak::upgrade)
    }

    /// `doSerialTasks()`.
    fn do_serial_tasks(&self) -> Result<i32> {
        if self.lock().rate > 0 {
            return self.do_serial_tasks_with_rate();
        }
        self.init_tasks_array();
        let mut count = 0;
        let (run_time_sec, tasks_array, let_child_report) = {
            let s = self.lock();
            (s.run_time_sec, s.tasks_array.clone().unwrap_or_default(), s.let_child_report)
        };
        let run_time = (run_time_sec * 1000.0) as i64;
        let t0 = jdk::current_time_millis();
        // Background tasks run on threads of their own: an exception in the
        // foreground leaves them running, as Java's threads keep running.
        (|| -> Result<()> {
            let mut bg_tasks: Option<Vec<(Arc<dyn PerfTask>, std::thread::JoinHandle<i32>)>> = None;
            let mut k = 0;
            loop {
                {
                    let s = self.lock();
                    let go = s.fixed_time
                        || (s.repetitions == Self::REPEAT_EXHAUST && !self.exhausted.load(Ordering::SeqCst))
                        || k < s.repetitions;
                    if !go {
                        break;
                    }
                }
                if self.base.is_stop_now() {
                    break;
                }
                for task in &tasks_array {
                    if task.base().get_run_in_background() {
                        let bg = Arc::clone(task);
                        let handle = std::thread::Builder::new()
                            .name(jdk::next_thread_name())
                            .spawn(move || run_background_task(&bg, let_child_report))
                            .map_err(rucene::error::LuceneError::from)?;
                        bg_tasks.get_or_insert_with(Vec::new).push((Arc::clone(task), handle));
                    } else {
                        match run_and_maybe_stats(task, let_child_report) {
                            Ok(inc) => {
                                count += inc;
                                self.add_count_by_time(t0, inc);
                                if self.lock().any_exhaustible_tasks {
                                    self.update_exhausted(&**task);
                                }
                            }
                            Err(e) if NoMoreDataException::is(&e) => {
                                self.exhausted.store(true, Ordering::SeqCst);
                            }
                            Err(e) => return Err(e),
                        }
                    }
                }
                let fixed_time = self.lock().fixed_time;
                if fixed_time && jdk::current_time_millis() - t0 > run_time {
                    self.lock().repetitions = k + 1;
                    break;
                }
                k += 1;
            }
            if let Some(bg_tasks) = bg_tasks {
                for (task, _) in &bg_tasks {
                    task.stop_now();
                }
                for (_, handle) in bg_tasks {
                    count += handle.join().unwrap_or(0);
                }
            }
            Ok(())
        })()?;
        let counts_by_time = self.lock().counts_by_time.clone();
        if let Some(counts_by_time) = counts_by_time {
            if let Some(stats) = self.base.get_run_data().get_points().get_current_stats() {
                jdk::lock(&stats).set_counts_by_time(counts_by_time, self.log_by_time_msec);
            }
        }
        self.base.set_stop_now(false);
        Ok(count)
    }

    /// The `countsByTime[slot] += inc` of a serial run.
    fn add_count_by_time(&self, t0: i64, inc: i32) {
        let mut s = self.lock();
        if let Some(counts) = s.counts_by_time.as_mut() {
            let slot = ((jdk::current_time_millis() - t0) / self.log_by_time_msec) as usize;
            if slot >= counts.len() {
                *counts = rucene::util::ArrayUtil::grow_int(std::mem::take(counts), 1 + slot);
            }
            counts[slot] += inc;
        }
    }

    /// `doSerialTasksWithRate()`.
    fn do_serial_tasks_with_rate(&self) -> Result<i32> {
        self.init_tasks_array();
        let (rate, per_min, tasks_array, let_child_report) = {
            let s = self.lock();
            (s.rate, s.per_min, s.tasks_array.clone().unwrap_or_default(), s.let_child_report)
        };
        let delay_step = i64::from(if per_min { 60000 } else { 1000 } / rate);
        let mut next_start_time = jdk::current_time_millis();
        let mut count = 0;
        let t0 = jdk::current_time_millis();
        let mut k = 0;
        loop {
            {
                let s = self.lock();
                let go = (s.repetitions == Self::REPEAT_EXHAUST && !self.exhausted.load(Ordering::SeqCst))
                    || k < s.repetitions;
                if !go {
                    break;
                }
            }
            if self.base.is_stop_now() {
                break;
            }
            for task in &tasks_array {
                while !self.base.is_stop_now() {
                    let wait_more = next_start_time - jdk::current_time_millis();
                    if wait_more > 0 {
                        // TODO: better to use condition to notify
                        std::thread::sleep(Duration::from_millis(1));
                    } else {
                        break;
                    }
                }
                if self.base.is_stop_now() {
                    break;
                }
                next_start_time += delay_step; // this aims at avarage rate.
                match run_and_maybe_stats(task, let_child_report) {
                    Ok(inc) => {
                        count += inc;
                        self.add_count_by_time(t0, inc);
                        if self.lock().any_exhaustible_tasks {
                            self.update_exhausted(&**task);
                        }
                    }
                    Err(e) if NoMoreDataException::is(&e) => {
                        self.exhausted.store(true, Ordering::SeqCst);
                    }
                    Err(e) => return Err(e),
                }
            }
            k += 1;
        }
        self.base.set_stop_now(false);
        Ok(count)
    }

    /// `updateExhausted(PerfTask task)`: update state regarding exhaustion.
    fn update_exhausted(&self, task: &dyn PerfTask) {
        if task.is_instance_of("ResetInputsTask") {
            self.exhausted.store(false, Ordering::SeqCst);
            self.reset_exhausted.store(true, Ordering::SeqCst);
        } else if let Some(t) = task.as_any().downcast_ref::<TaskSequence>() {
            if t.reset_exhausted.load(Ordering::SeqCst) {
                self.exhausted.store(false, Ordering::SeqCst);
                self.reset_exhausted.store(true, Ordering::SeqCst);
                t.reset_exhausted.store(false, Ordering::SeqCst);
            } else if t.exhausted.load(Ordering::SeqCst) {
                self.exhausted.store(true, Ordering::SeqCst);
            }
        }
    }

    /// `doParallelTasks()`.
    fn do_parallel_tasks(&self) -> Result<i32> {
        let run_data = Arc::clone(self.base.get_run_data());
        let stats = run_data.get_points().get_current_stats();
        self.init_tasks_array();
        let (repetitions, tasks_array, let_child_report, any_exhaustible, rate, per_min) = {
            let s = self.lock();
            (
                s.repetitions,
                s.tasks_array.clone().unwrap_or_default(),
                s.let_child_report,
                s.any_exhaustible_tasks,
                s.rate,
                s.per_min,
            )
        };
        let n = (repetitions.max(0) as usize) * tasks_array.len();
        // Get number of parallel threads from algo file and set it to use in ReuersContentSource.java's
        // docCountArrInit()
        run_data.get_config().set_num_threads(n as i32);
        // prepare threads
        let mut t: Vec<(String, Arc<dyn PerfTask>)> = Vec::with_capacity(n);
        let mut index = 0;
        for _ in 0..repetitions {
            for task in &tasks_array {
                let task = task.clone_task()?;
                // Setting unique ThreadName with index value which is used in ReuersContentSource.java's
                // getNextDocData().
                t.push((format!("{}-{index}", Constants::PARALLEL_TASK_THREAD_NAME_PREFIX), task));
                index += 1;
            }
        }
        self.lock().running_parallel_tasks = Some(t.iter().map(|(_, task)| Arc::clone(task)).collect());
        let counts: Vec<AtomicI32> = (0..t.len()).map(|_| AtomicI32::new(0)).collect();
        std::thread::scope(|scope| -> Result<()> {
            // run threads
            let mut handles = Vec::with_capacity(t.len());
            let delay_step = if rate > 0 { i64::from(if per_min { 60000 } else { 1000 } / rate) } else { 0 };
            let mut next_start_time = jdk::current_time_millis();
            for (i, (name, task)) in t.iter().enumerate() {
                if rate > 0 {
                    // startlThreadsWithRate: run threads with rate
                    let wait_more = next_start_time - jdk::current_time_millis();
                    if wait_more > 0 {
                        std::thread::sleep(Duration::from_millis(wait_more as u64));
                    }
                    next_start_time += delay_step; // this aims at average rate of starting threads.
                }
                let count = &counts[i];
                let thread_name = name.clone();
                let handle = std::thread::Builder::new()
                    .name(name.clone())
                    .spawn_scoped(scope, move || {
                        // ParallelTask.run()
                        match run_and_maybe_stats(task, let_child_report) {
                            Ok(n) => {
                                if any_exhaustible {
                                    self.update_exhausted(&**task);
                                }
                                count.fetch_add(n, Ordering::SeqCst);
                            }
                            Err(e) if NoMoreDataException::is(&e) => {
                                self.exhausted.store(true, Ordering::SeqCst);
                            }
                            Err(e) => {
                                // `throw new RuntimeException(e)`: the thread dies
                                // with the default uncaught-exception report.
                                eprintln!(
                                    "Exception in thread \"{thread_name}\" {}",
                                    jdk::runtime_exception_from(e)
                                );
                            }
                        }
                    })
                    .map_err(rucene::error::LuceneError::from)?;
                handles.push(handle);
            }
            if self.base.is_stop_now() {
                for (_, task) in &t {
                    task.stop_now();
                }
            }
            // wait for all threads to complete
            for handle in handles {
                let _ = handle.join();
            }
            Ok(())
        })?;
        let mut count = 0;
        for (i, (_, task)) in t.iter().enumerate() {
            count += counts[i].load(Ordering::SeqCst);
            if let Some(sub) = task.as_any().downcast_ref::<TaskSequence>() {
                let sub_counts = sub.lock().counts_by_time.clone();
                if let Some(sub_counts) = sub_counts {
                    let mut s = self.lock();
                    let counts = s.counts_by_time.get_or_insert_with(|| vec![0; sub_counts.len()]);
                    if counts.len() < sub_counts.len() {
                        *counts = rucene::util::ArrayUtil::grow_int(std::mem::take(counts), sub_counts.len());
                    }
                    for (j, c) in sub_counts.iter().enumerate() {
                        counts[j] += c;
                    }
                }
            }
        }
        let counts_by_time = self.lock().counts_by_time.clone();
        if let (Some(counts_by_time), Some(stats)) = (counts_by_time, stats) {
            jdk::lock(&stats).set_counts_by_time(counts_by_time, self.log_by_time_msec);
        }
        // return total count
        Ok(count)
    }

    /// `addTask(PerfTask task)`.
    pub fn add_task(&self, task: Arc<dyn PerfTask>) {
        task.base().set_depth(self.base.get_depth() + 1);
        self.lock().tasks.push(task);
    }

    /// `setNoChildReport()`: execute child tasks in a way that they do not
    /// report their time separately.
    pub fn set_no_child_report(&self) {
        let tasks = {
            let mut s = self.lock();
            s.let_child_report = false;
            s.tasks.clone()
        };
        for task in tasks {
            if let Some(seq) = task.as_any().downcast_ref::<TaskSequence>() {
                seq.set_no_child_report();
            }
        }
    }

    /// `getRate()`: the rate per min: how many operations should be
    /// performed in a minute. If 0 this has no effect.
    pub fn get_rate(&self) -> i32 {
        let s = self.lock();
        if s.per_min {
            s.rate
        } else {
            60 * s.rate
        }
    }

    /// `setRate(int rate, boolean perMin)`.
    pub fn set_rate(&self, rate: i32, per_min: bool) {
        {
            let mut s = self.lock();
            s.rate = rate;
            s.per_min = per_min;
        }
        self.set_sequence_name();
    }

    /// `setSequenceName()`.
    fn set_sequence_name(&self) {
        let base_name = self.base.get_name();
        let mut s = self.lock();
        let mut seq_name = base_name;
        if s.repetitions == Self::REPEAT_EXHAUST {
            seq_name.push_str("_Exhaust");
        } else if s.repetitions > 1 {
            seq_name.push_str(&format!("_{}", s.repetitions));
        }
        if s.rate > 0 {
            seq_name.push_str(&format!("_{}{}", s.rate, if s.per_min { "/min" } else { "/sec" }));
        }
        if s.parallel && !seq_name.to_lowercase().contains("par") {
            seq_name.push_str("_Par");
        }
        s.seq_name = seq_name;
    }

    /// `getTasks()`.
    pub fn get_tasks(&self) -> Vec<Arc<dyn PerfTask>> {
        self.lock().tasks.clone()
    }

    /// `isCollapsable()`: true if can be collapsed in case it is outermost
    /// sequence.
    pub fn is_collapsable(&self) -> bool {
        self.collapsable
    }

    /// `setDepth(int)` (inherited).
    pub fn set_depth(&self, depth: i32) {
        self.base.set_depth(depth);
    }
}

/// `RunBackgroundTask.run()`: runs `task` in a background thread; an
/// exception kills the thread with the default uncaught-exception report.
fn run_background_task(task: &Arc<dyn PerfTask>, let_child_report: bool) -> i32 {
    match run_and_maybe_stats(task, let_child_report) {
        Ok(count) => count,
        Err(e) => {
            eprintln!(
                "Exception in thread \"{}\" {}",
                jdk::current_thread_name(),
                jdk::runtime_exception_from(e)
            );
            0
        }
    }
}

impl PerfTask for TaskSequence {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        let mut state = self.lock().clone();
        let mut tasks = Vec::with_capacity(state.tasks.len());
        for task in &state.tasks {
            tasks.push(task.clone_task()?);
        }
        state.tasks = tasks;
        let base = self.base.clone_base();
        let res: Arc<TaskSequence> = Arc::new_cyclic(|me| TaskSequence {
            base,
            state: Mutex::new(state),
            parent: self.parent.clone(),
            exhausted: AtomicBool::new(self.exhausted.load(Ordering::SeqCst)),
            reset_exhausted: AtomicBool::new(self.reset_exhausted.load(Ordering::SeqCst)),
            collapsable: self.collapsable,
            log_by_time_msec: self.log_by_time_msec,
            me: me.clone(),
        });
        Ok(res)
    }

    fn close(&self) -> Result<()> {
        self.init_tasks_array();
        let tasks = self.lock().tasks_array.clone().unwrap_or_default();
        for task in tasks {
            task.close()?;
        }
        self.base.get_run_data().get_doc_maker().close()
    }

    fn do_logic(&self) -> Result<i32> {
        self.exhausted.store(false, Ordering::SeqCst);
        self.reset_exhausted.store(false, Ordering::SeqCst);
        if self.is_parallel() {
            self.do_parallel_tasks()
        } else {
            self.do_serial_tasks()
        }
    }

    fn stop_now(&self) {
        self.base.stop_now();
        // Forwards top request to children
        let running = self.lock().running_parallel_tasks.clone();
        if let Some(running) = running {
            for t in running {
                t.stop_now();
            }
        }
    }

    fn get_name(&self) -> String {
        self.lock().seq_name.clone() // override to include more info
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "TaskSequence"
    }

    fn task_to_string(&self) -> String {
        let padd = self.base.get_padding();
        let mut sb = perf_task::task_to_string(self);
        let s = self.lock().clone();
        sb.push_str(if s.parallel { " [" } else { " {" });
        sb.push_str(NEW_LINE);
        for task in &s.tasks {
            sb.push_str(&task.task_to_string());
            sb.push_str(NEW_LINE);
        }
        sb.push_str(&padd);
        sb.push_str(if !s.let_child_report {
            ">"
        } else if s.parallel {
            "]"
        } else {
            "}"
        });
        if s.fixed_time {
            let format = rucene_queryparser::jdk::number_format::get_number_instance(
                &rucene_queryparser::jdk::locale::Locale::root(),
            ).expect("Locale.ROOT has a number format");
            sb.push(' ');
            sb.push_str(&format.format_double(s.run_time_sec).unwrap_or_default());
            sb.push('s');
        } else if s.repetitions > 1 {
            sb.push_str(&format!(" * {}", s.repetitions));
        } else if s.repetitions == Self::REPEAT_EXHAUST {
            sb.push_str(" * EXHAUST");
        }
        if s.rate > 0 {
            sb.push_str(&format!(",  rate: {}/{}", s.rate, if s.per_min { "min" } else { "sec" }));
        }
        if self.base.get_run_in_background() {
            sb.push_str(" &");
            let x = self.base.get_background_delta_priority();
            if x != 0 {
                sb.push_str(&x.to_string());
            }
        }
        sb
    }
}
