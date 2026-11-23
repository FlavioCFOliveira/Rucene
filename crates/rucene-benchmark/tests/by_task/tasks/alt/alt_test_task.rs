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

//! Port of `org.apache.lucene.benchmark.byTask.tasks.alt.AltTestTask`.

use std::sync::Arc;

use rucene::error::Result;
use rucene_benchmark::by_task::perf_run_data::PerfRunData;
use rucene_benchmark::by_task::tasks::perf_task::{PerfTask, PerfTaskBase};
use rucene_benchmark::classes;

/// `AltTestTask`: [`PerfTask`] which does nothing, but is in a different
/// package.
pub struct AltTestTask {
    base: PerfTaskBase,
}

/// The binary name of `AltTestTask`.
pub const CLASS_NAME: &str = "org.apache.lucene.benchmark.byTask.tasks.alt.AltTestTask";

impl AltTestTask {
    /// `AltTestTask(PerfRunData runData)`.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(AltTestTask {
            base: PerfTaskBase::new(run_data, "AltTestTask")?,
        })
    }

    /// Puts the class on the "classpath" (`Class.forName` finds it).
    pub fn register() {
        classes::register_task_class(
            CLASS_NAME,
            Arc::new(|run_data| Ok(Arc::new(AltTestTask::new(run_data)?) as Arc<dyn PerfTask>)),
        );
    }
}

impl PerfTask for AltTestTask {
    fn base(&self) -> &PerfTaskBase {
        &self.base
    }

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(AltTestTask {
            base: self.base.clone_base(),
        }))
    }

    fn do_logic(&self) -> Result<i32> {
        Ok(0)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
