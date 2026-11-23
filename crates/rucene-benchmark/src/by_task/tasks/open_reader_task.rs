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

//! The types declared in `OpenReaderTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::index_commit::IndexCommit;
use rucene::store::Directory;

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.OpenReaderTask`
/// (`public class OpenReaderTask extends PerfTask`): open an index reader.
///
/// Other side effects: index reader object in perfRunData is set.
///
/// Optional params commitUserData eg. OpenReader(false,commit1)
pub struct OpenReaderTask {
    base: PerfTaskBase,
    commit_user_data: Mutex<Option<String>>,
}

impl OpenReaderTask {
    /// `USER_DATA`.
    pub const USER_DATA: &'static str = "userData";

    /// `OpenReaderTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(OpenReaderTask {
            base: PerfTaskBase::new(run_data, "OpenReaderTask")?,
            commit_user_data: Mutex::new(None),
        })
    }

    /// `findIndexCommit(Directory dir, String userData)`.
    ///
    /// # Errors
    ///
    /// `IOException("index does not contain commit with userData: " +
    /// userData)`, or the exception of listing the commits.
    pub fn find_index_commit(
        dir: Arc<dyn Directory>,
        user_data: &str,
    ) -> Result<Arc<dyn IndexCommit>> {
        let commits = directory_reader::list_commits(dir)?;
        for ic in commits {
            let map = ic.get_user_data()?;
            if map.get(Self::USER_DATA).is_some_and(|ud| ud == user_data) {
                return Ok(ic);
            }
        }
        Err(jdk::io_exception(format!(
            "index does not contain commit with userData: {user_data}"
        )))
    }
}

impl PerfTask for OpenReaderTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(OpenReaderTask {
            base: self.base.clone_base(),
            commit_user_data: Mutex::new(jdk::lock(&self.commit_user_data).clone()),
        }))
    }

    fn do_logic(&self) -> Result<i32> {
        let run_data = self.base.get_run_data();
        let dir = run_data
            .get_directory()
            .ok_or_else(LuceneError::null_pointer)?;
        let commit_user_data = jdk::lock(&self.commit_user_data).clone();
        let r = match commit_user_data {
            Some(user_data) => {
                directory_reader::open_with_commit(&*Self::find_index_commit(dir, &user_data)?)?
            }
            None => directory_reader::open(dir)?,
        };
        run_data.set_index_reader(Some(Arc::clone(&r)))?;
        // We transfer reference to the run data
        r.dec_ref()?;
        Ok(1)
    }

    fn set_params(&self, params: &str) -> Result<()> {
        perf_task::set_params(self, params)?;
        let split = jdk::split(params, ",");
        if let Some(first) = split.first() {
            *jdk::lock(&self.commit_user_data) = Some(first.clone());
        }
        Ok(())
    }

    fn supports_params(&self) -> bool {
        true
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "OpenReaderTask"
    }
}
