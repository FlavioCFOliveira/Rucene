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

//! The types declared in `PerfTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use rucene::error::{LuceneError, Result};

use crate::by_task::perf_run_data::PerfRunData;
use crate::jdk;

/// `DEFAULT_LOG_STEP`.
pub(crate) const DEFAULT_LOG_STEP: i32 = 1000;

/// `NEW_LINE`.
pub(crate) const NEW_LINE: &str = jdk::NEW_LINE;

/// The fields of `PerfTask`.
#[derive(Clone, Debug)]
struct BaseState {
    name: String,
    depth: i32,
    log_step: i32,
    log_step_count: i32,
    max_depth_log_start: i32,
    disable_counting: bool,
    params: Option<String>,
    run_in_background: bool,
    delta_pri: i32,
    /// The first line of this task's definition in the alg file
    alg_line_num: i32,
}

/// The state and the non-virtual members of
/// `org.apache.lucene.benchmark.byTask.tasks.PerfTask`, which every task
/// embeds.
pub struct PerfTaskBase {
    run_data: Arc<PerfRunData>,
    state: Mutex<BaseState>,
    /// `stopNow` (`protected volatile boolean`).
    stop_now: AtomicBool,
}

impl PerfTaskBase {
    /// `PerfTask(PerfRunData runData)` of a task whose class's simple name
    /// is `simple_class_name`: the name drops a trailing `"Task"`, and the
    /// logging properties are read from the config.
    ///
    /// # Errors
    ///
    /// The `NumberFormatException`/`ClassCastException` of reading the
    /// properties.
    pub fn new(run_data: Arc<PerfRunData>, simple_class_name: &str) -> Result<Self> {
        let name = simple_class_name.strip_suffix("Task").unwrap_or(simple_class_name).to_string();
        let config = run_data.get_config();
        let max_depth_log_start = config.get_int("task.max.depth.log", 0)?;
        let mut log_step_att = "log.step".to_string();
        let task_log_step_att = format!("log.step.{name}");
        if config.get_str(&task_log_step_att, None)?.is_some() {
            log_step_att = task_log_step_att;
        }
        // It's important to read this from Config, to support vals-by-round.
        let mut log_step = config.get_int(&log_step_att, DEFAULT_LOG_STEP)?;
        // To avoid the check 'if (logStep > 0)' in tearDown(). This effectively
        // turns logging off.
        if log_step <= 0 {
            log_step = i32::MAX;
        }
        Ok(PerfTaskBase {
            run_data,
            state: Mutex::new(BaseState {
                name,
                depth: 0,
                log_step,
                log_step_count: 0,
                max_depth_log_start,
                disable_counting: false,
                params: None,
                run_in_background: false,
                delta_pri: 0,
                alg_line_num: 0,
            }),
            stop_now: AtomicBool::new(false),
        })
    }

    fn lock(&self) -> MutexGuard<'_, BaseState> {
        jdk::lock(&self.state)
    }

    /// `super.clone()`: a copy of every field.
    pub fn clone_base(&self) -> PerfTaskBase {
        PerfTaskBase {
            run_data: Arc::clone(&self.run_data),
            state: Mutex::new(self.lock().clone()),
            stop_now: AtomicBool::new(self.stop_now.load(Ordering::SeqCst)),
        }
    }

    /// `setRunInBackground(int deltaPri)`.
    pub fn set_run_in_background(&self, delta_pri: i32) {
        let mut s = self.lock();
        s.run_in_background = true;
        s.delta_pri = delta_pri;
    }

    /// `getRunInBackground()`.
    pub fn get_run_in_background(&self) -> bool {
        self.lock().run_in_background
    }

    /// `getBackgroundDeltaPriority()`.
    pub fn get_background_delta_priority(&self) -> i32 {
        self.lock().delta_pri
    }

    /// `PerfTask.stopNow()`: `stopNow = true`.
    pub fn stop_now(&self) {
        self.stop_now.store(true, Ordering::SeqCst);
    }

    /// `stopNow` (the field).
    pub fn is_stop_now(&self) -> bool {
        self.stop_now.load(Ordering::SeqCst)
    }

    /// `stopNow = value` (the field).
    pub fn set_stop_now(&self, value: bool) {
        self.stop_now.store(value, Ordering::SeqCst);
    }

    /// `PerfTask.getName()`: the name, then `(params)` when there are
    /// params.
    pub fn get_name(&self) -> String {
        let s = self.lock();
        match &s.params {
            None => s.name.clone(),
            Some(params) => format!("{}({params})", s.name),
        }
    }

    /// `setName(String)`.
    pub fn set_name(&self, name: impl Into<String>) {
        self.lock().name = name.into();
    }

    /// `getRunData()`.
    pub fn get_run_data(&self) -> &Arc<PerfRunData> {
        &self.run_data
    }

    /// `getDepth()`.
    pub fn get_depth(&self) -> i32 {
        self.lock().depth
    }

    /// `setDepth(int)`.
    pub fn set_depth(&self, depth: i32) {
        self.lock().depth = depth;
    }

    /// `getPadding()`: a blank string padding for printing this task
    /// indented by its depth.
    pub fn get_padding(&self) -> String {
        " ".repeat((4 * self.get_depth()).max(0) as usize)
    }

    /// `getMaxDepthLogStart()`.
    pub fn get_max_depth_log_start(&self) -> i32 {
        self.lock().max_depth_log_start
    }

    /// `logStep`.
    pub fn log_step(&self) -> i32 {
        self.lock().log_step
    }

    /// `logStep = value`.
    pub fn set_log_step(&self, log_step: i32) {
        self.lock().log_step = log_step;
    }

    /// `params`.
    pub fn get_params(&self) -> Option<String> {
        self.lock().params.clone()
    }

    /// `this.params = params`.
    pub fn store_params(&self, params: &str) {
        self.lock().params = Some(params.to_string());
    }

    /// `isDisableCounting()`.
    pub fn is_disable_counting(&self) -> bool {
        self.lock().disable_counting
    }

    /// `setDisableCounting(boolean)`.
    pub fn set_disable_counting(&self, disable_counting: bool) {
        self.lock().disable_counting = disable_counting;
    }

    /// `setAlgLineNum(int)`.
    pub fn set_alg_line_num(&self, alg_line_num: i32) {
        self.lock().alg_line_num = alg_line_num;
    }

    /// `getAlgLineNum()`.
    pub fn get_alg_line_num(&self) -> i32 {
        self.lock().alg_line_num
    }

    /// `++logStepCount % logStep == 0`, with the new count.
    fn next_log_step(&self) -> (bool, i32) {
        let mut s = self.lock();
        s.log_step_count += 1;
        (s.log_step_count % s.log_step == 0, s.log_step_count)
    }
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.PerfTask`
/// (`public abstract class PerfTask implements Cloneable`): an abstract task
/// to be tested for performance.
///
/// Every performance task implements this trait, and provides its own
/// [`do_logic`](Self::do_logic) method, which performs the actual task.
/// Tasks performing some work that should be measured for the task, can
/// override [`setup`](Self::setup) and/or [`tear_down`](Self::tear_down)
/// and place that work there.
///
/// Relevant properties: `task.max.depth.log`. Also supports the following
/// logging attributes:
///
/// * log.step - specifies how often to log messages about the current
///   running task. Default is 1000 `doLogic()` invocations. Set to -1 to
///   disable logging.
/// * log.step.[class Task Name] - specifies the same as 'log.step', only for
///   a particular task name. For example, log.step.AddDoc will be applied
///   only for `AddDocTask`. It's a way to control per task logging settings.
///   If you want to omit logging for any other task, include log.step=-1.
///   The syntax is "log.step." together with the Task's 'short' name (i.e.,
///   without the 'Task' part).
///
/// A task is shared (`Arc<dyn PerfTask>`) by its sequence and by the thread
/// that runs it in the background, as Java shares the object, so its
/// methods take `&self` and its state sits behind locks.
pub trait PerfTask: Send + Sync + Any {
    /// The embedded `PerfTask` state.
    fn base(&self) -> &PerfTaskBase;

    /// `clone()`: tasks having non primitive data structures copy them;
    /// otherwise parallel running of a task sequence might not run
    /// correctly.
    ///
    /// # Errors
    ///
    /// `CloneNotSupportedException` never in this port; a task's clone may
    /// fail on building its state.
    fn clone_task(&self) -> Result<Arc<dyn PerfTask>>;

    /// `close()`.
    ///
    /// # Errors
    ///
    /// The exception of closing the task's resources.
    fn close(&self) -> Result<()> {
        Ok(())
    }

    /// `doLogic()`: perform the task once (ignoring repetitions
    /// specification) Return number of work items done by this task. For
    /// indexing that can be number of docs added. For warming that can be
    /// number of scanned items, etc.
    ///
    /// # Errors
    ///
    /// The exception of the task's work.
    fn do_logic(&self) -> Result<i32>;

    /// `getName()`.
    fn get_name(&self) -> String {
        self.base().get_name()
    }

    /// `getLogMessage(int recsCount)`.
    fn get_log_message(&self, recs_count: i32) -> String {
        format!("processed {recs_count} records")
    }

    /// `shouldNeverLogAtStart()`: tasks that should never log at start can
    /// override this.
    fn should_never_log_at_start(&self) -> bool {
        false
    }

    /// `shouldNotRecordStats()`: tasks that should not record statistics can
    /// override this.
    fn should_not_record_stats(&self) -> bool {
        false
    }

    /// `setup()`: task setup work that should not be measured for that
    /// specific task. By default it does nothing, but tasks can implement
    /// this, moving work from `doLogic()` to this method. Only the work done
    /// in `doLogic()` is measured for this task. Notice that higher level
    /// (sequence) tasks containing this task would then measure larger time
    /// than the sum of their contained tasks.
    ///
    /// # Errors
    ///
    /// The exception of the setup work.
    fn setup(&self) -> Result<()> {
        Ok(())
    }

    /// `tearDown()`: task tearDown work that should not be measured for that
    /// specific task. The base implementation, [`tear_down`], logs every
    /// `logStep` runs.
    ///
    /// # Errors
    ///
    /// The exception of the tear-down work.
    fn tear_down(&self) -> Result<()> {
        tear_down(self)
    }

    /// `supportsParams()`: sub classes that support parameters must override
    /// this method to return true.
    fn supports_params(&self) -> bool {
        false
    }

    /// `setParams(String params)`: set the params of this task. The base
    /// implementation is [`set_params`].
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` for tasks not supporting command
    /// line parameters.
    fn set_params(&self, params: &str) -> Result<()> {
        set_params(self, params)
    }

    /// `stopNow()`.
    fn stop_now(&self) {
        self.base().stop_now();
    }

    /// `toString()`. The base implementation is [`task_to_string`].
    fn task_to_string(&self) -> String {
        task_to_string(self)
    }

    /// `this instanceof C` for the task class whose simple name is
    /// `simple_class_name`: true for the task's class and its task
    /// superclasses (`ResetSystemEraseTask` is a `ResetSystemSoftTask` and a
    /// `ResetInputsTask`).
    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        let _ = simple_class_name;
        false
    }

    /// `((RepSumByPrefTask) task).setPrefix(prefix)`, for a task that
    /// [`is_instance_of`](Self::is_instance_of) `RepSumByPrefTask`; a no-op
    /// for every other task.
    fn set_report_prefix(&self, prefix: &str) {
        let _ = prefix;
    }

    /// This task as `Any`, for `instanceof` and `getClass()`.
    fn as_any(&self) -> &dyn Any;
}

/// `PerfTask.tearDown()`: logs `getLogMessage(logStepCount)` every
/// `logStep` runs.
///
/// # Errors
///
/// None.
pub fn tear_down<T: PerfTask + ?Sized>(task: &T) -> Result<()> {
    let (log, count) = task.base().next_log_step();
    if log {
        let time = (jdk::current_time_millis() - task.base().get_run_data().get_start_time_millis()) as f64 / 1000.0;
        println!(
            "{:>7} sec --> {} {}",
            rucene::util::java_fmt::format_fixed(time, 2),
            jdk::current_thread_name(),
            task.get_log_message(count)
        );
    }
    Ok(())
}

/// `PerfTask.setParams(String params)`.
///
/// # Errors
///
/// `UnsupportedOperationException(getName() + " does not support command
/// line parameters.")` when the task does not support params.
pub fn set_params<T: PerfTask + ?Sized>(task: &T, params: &str) -> Result<()> {
    if !task.supports_params() {
        return Err(LuceneError::unsupported_operation(format!(
            "{} does not support command line parameters.",
            task.get_name()
        )));
    }
    task.base().store_params(params);
    Ok(())
}

/// `PerfTask.toString()`.
pub fn task_to_string<T: PerfTask + ?Sized>(task: &T) -> String {
    let padd = task.base().get_padding();
    let mut sb = padd;
    if task.base().is_disable_counting() {
        sb.push('-');
    }
    sb.push_str(&task.get_name());
    if task.base().get_run_in_background() {
        sb.push_str(" &");
        let x = task.base().get_background_delta_priority();
        if x != 0 {
            sb.push_str(&x.to_string());
        }
    }
    sb
}

/// `PerfTask.runAndMaybeStats(boolean reportStats)` (a `final` method): run
/// the task, record statistics. Returns the number of work items done by
/// this task.
///
/// # Errors
///
/// The exception of `setup`, `doLogic` or `tearDown`.
pub fn run_and_maybe_stats(task: &Arc<dyn PerfTask>, report_stats: bool) -> Result<i32> {
    let base = task.base();
    if !report_stats || task.should_not_record_stats() {
        task.setup()?;
        let mut count = task.do_logic()?;
        count = if base.is_disable_counting() { 0 } else { count };
        task.tear_down()?;
        return Ok(count);
    }
    if report_stats && base.get_depth() <= base.get_max_depth_log_start() && !task.should_never_log_at_start() {
        println!("------------> starting task: {}", task.get_name());
    }
    task.setup()?;
    let run_data = base.get_run_data();
    let pnts = run_data.get_points();
    let ts = pnts.mark_task_start(Arc::clone(task), run_data.get_config().get_round_number());
    let mut count = task.do_logic()?;
    count = if base.is_disable_counting() { 0 } else { count };
    pnts.mark_task_end(&ts, count);
    task.tear_down()?;
    Ok(count)
}

/// Implements [`PerfTask::clone_task`] for a task whose only state besides
/// its base is a `Mutex` named `state` over a `Clone` value, and the
/// [`PerfTask::base`]/[`PerfTask::as_any`] accessors.
macro_rules! perf_task_plumbing {
    () => {
        fn base(&self) -> &$crate::by_task::tasks::perf_task::PerfTaskBase {
            &self.base
        }

        fn as_any(&self) -> &dyn ::std::any::Any {
            self
        }
    };
}
pub(crate) use perf_task_plumbing;
