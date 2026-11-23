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

//! The types declared in `CommitIndexTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::{Arc, Mutex};

use rucene::error::Result;

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::open_reader_task::OpenReaderTask;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.CommitIndexTask`
/// (`public class CommitIndexTask extends PerfTask`): commits the
/// IndexWriter.
pub struct CommitIndexTask {
    base: PerfTaskBase,
    /// `commitUserData`: a one-entry map.
    commit_user_data: Mutex<Option<Vec<(String, String)>>>,
}

impl CommitIndexTask {
    /// `CommitIndexTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(CommitIndexTask { base: PerfTaskBase::new(run_data, "CommitIndexTask")?, commit_user_data: Mutex::new(None) })
    }
}

impl PerfTask for CommitIndexTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(CommitIndexTask {
            base: self.base.clone_base(),
            commit_user_data: Mutex::new(jdk::lock(&self.commit_user_data).clone()),
        }))
    }

    fn supports_params(&self) -> bool {
        true
    }

    fn set_params(&self, params: &str) -> Result<()> {
        perf_task::set_params(self, params)?;
        *jdk::lock(&self.commit_user_data) = Some(vec![(OpenReaderTask::USER_DATA.to_string(), params.to_string())]);
        Ok(())
    }

    fn do_logic(&self) -> Result<i32> {
        if let Some(iw) = self.base.get_run_data().get_index_writer() {
            if let Some(commit_user_data) = jdk::lock(&self.commit_user_data).clone() {
                iw.set_live_commit_data(commit_user_data)?;
            }
            iw.commit()?;
        }
        Ok(1)
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "CommitIndexTask"
    }
}
