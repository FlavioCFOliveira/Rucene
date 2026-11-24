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

//! The types declared in `NewRoundTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::Arc;

use rucene::error::Result;

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.NewRoundTask`
/// (`public class NewRoundTask extends PerfTask`): increment the counter for properties maintained by Round Number.
///
/// Other side effects: if there are props by round number, log value change.
pub struct NewRoundTask {
    base: PerfTaskBase,
}

impl NewRoundTask {
    /// `NewRoundTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(NewRoundTask {
            base: PerfTaskBase::new(run_data, "NewRoundTask")?,
        })
    }
}

impl PerfTask for NewRoundTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(NewRoundTask {
            base: self.base.clone_base(),
        }))
    }

    fn do_logic(&self) -> Result<i32> {
        self.base.get_run_data().get_config().new_round();
        Ok(0)
    }

    fn should_not_record_stats(&self) -> bool {
        true
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        matches!(simple_class_name, "NewRoundTask")
    }
}
