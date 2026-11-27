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

//! The types declared in `WaitTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rucene::error::Result;

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.WaitTask`
/// (`public class WaitTask extends PerfTask`): simply waits for the
/// specified (via the parameter) amount of time. For example Wait(30s)
/// waits for 30 seconds. This is useful with background tasks to control
/// how long the tasks run.
///
/// You can specify h, m, or s (hours, minutes, seconds) as the trailing time
/// unit. No unit is interpreted as seconds.
pub struct WaitTask {
    base: PerfTaskBase,
    wait_time_sec: Mutex<f64>,
}

impl WaitTask {
    /// `WaitTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(WaitTask {
            base: PerfTaskBase::new(run_data, "WaitTask")?,
            wait_time_sec: Mutex::new(0.0),
        })
    }
}

impl PerfTask for WaitTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(WaitTask {
            base: self.base.clone_base(),
            wait_time_sec: Mutex::new(*jdk::lock(&self.wait_time_sec)),
        }))
    }

    fn set_params(&self, params: &str) -> Result<()> {
        perf_task::set_params(self, params)?;
        let (multiplier, number) = if let Some(p) = params.strip_suffix('s') {
            (1.0, p)
        } else if let Some(p) = params.strip_suffix('m') {
            (60.0, p)
        } else if let Some(p) = params.strip_suffix('h') {
            (3600.0, p)
        } else {
            // Assume seconds
            (1.0, params)
        };
        *jdk::lock(&self.wait_time_sec) = jdk::parse_double(number)? * multiplier;
        Ok(())
    }

    fn do_logic(&self) -> Result<i32> {
        let millis = (1000.0 * *jdk::lock(&self.wait_time_sec)) as i64;
        if millis < 0 {
            return Err(rucene::error::LuceneError::illegal_argument(
                "timeout value is negative",
            ));
        }
        std::thread::sleep(Duration::from_millis(millis as u64));
        Ok(0)
    }

    fn supports_params(&self) -> bool {
        true
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "WaitTask"
    }
}
