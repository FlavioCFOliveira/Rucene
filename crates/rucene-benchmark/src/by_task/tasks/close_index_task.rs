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

//! The types declared in `CloseIndexTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rucene::error::Result;

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.CloseIndexTask`
/// (`public class CloseIndexTask extends PerfTask`): close index writer.
///
/// Other side effects: index writer object in perfRunData is nullified.
///
/// Takes optional param "doWait": if false, then close(false) is called.
pub struct CloseIndexTask {
    base: PerfTaskBase,
    do_wait: AtomicBool,
}

impl CloseIndexTask {
    /// `CloseIndexTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(CloseIndexTask { base: PerfTaskBase::new(run_data, "CloseIndexTask")?, do_wait: AtomicBool::new(true) })
    }
}

impl PerfTask for CloseIndexTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(CloseIndexTask {
            base: self.base.clone_base(),
            do_wait: AtomicBool::new(self.do_wait.load(Ordering::SeqCst)),
        }))
    }

    fn do_logic(&self) -> Result<i32> {
        let run_data = self.base.get_run_data();
        if let Some(iw) = run_data.get_index_writer() {
            // If infoStream was set to output to a file, close it.
            iw.get_config().info_stream().close();
            if !self.do_wait.load(Ordering::SeqCst) {
                iw.commit()?;
                iw.rollback()?;
            } else {
                iw.close()?;
            }
            run_data.set_index_writer(None);
        }
        Ok(1)
    }

    fn set_params(&self, params: &str) -> Result<()> {
        perf_task::set_params(self, params)?;
        self.do_wait.store(jdk::parse_boolean(Some(params)), Ordering::SeqCst);
        Ok(())
    }

    fn supports_params(&self) -> bool {
        true
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "CloseIndexTask"
    }
}
