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

//! The types declared in `OpenIndexTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};
use rucene::index::index_writer_config::{LiveIndexWriterConfig, OpenMode};
use rucene::index::merge_policy::DEFAULT_MERGE_FACTOR;

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::create_index_task::CreateIndexTask;
use crate::by_task::tasks::open_reader_task::OpenReaderTask;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.OpenIndexTask`
/// (`public class OpenIndexTask extends PerfTask`): open an index writer.
///
/// Other side effects: index writer object in perfRunData is set.
///
/// Relevant properties: `merge.factor, max.buffered, max.field.length,
/// ram.flush.mb [default 0]`.
///
/// Accepts a param specifying the commit point as previously saved with
/// CommitIndexTask. If you specify this, it rolls the index back to that
/// commit on opening the IndexWriter.
pub struct OpenIndexTask {
    base: PerfTaskBase,
    commit_user_data: Mutex<Option<String>>,
}

impl OpenIndexTask {
    /// `DEFAULT_MAX_BUFFERED`.
    pub const DEFAULT_MAX_BUFFERED: i32 = LiveIndexWriterConfig::DEFAULT_MAX_BUFFERED_DOCS;
    /// `DEFAULT_MERGE_PFACTOR`.
    pub const DEFAULT_MERGE_PFACTOR: i32 = DEFAULT_MERGE_FACTOR;
    /// `DEFAULT_RAM_FLUSH_MB`: `(int) IndexWriterConfig.DEFAULT_RAM_BUFFER_SIZE_MB`.
    pub const DEFAULT_RAM_FLUSH_MB: f64 =
        LiveIndexWriterConfig::DEFAULT_RAM_BUFFER_SIZE_MB as i32 as f64;

    /// `OpenIndexTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(OpenIndexTask {
            base: PerfTaskBase::new(run_data, "OpenIndexTask")?,
            commit_user_data: Mutex::new(None),
        })
    }
}

impl PerfTask for OpenIndexTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(OpenIndexTask {
            base: self.base.clone_base(),
            commit_user_data: Mutex::new(jdk::lock(&self.commit_user_data).clone()),
        }))
    }

    fn do_logic(&self) -> Result<i32> {
        let run_data = self.base.get_run_data();
        let config = Arc::clone(run_data.get_config());
        let commit_user_data = jdk::lock(&self.commit_user_data).clone();
        let ic = match commit_user_data {
            Some(user_data) => Some(OpenReaderTask::find_index_commit(
                run_data
                    .get_directory()
                    .ok_or_else(LuceneError::null_pointer)?,
                &user_data,
            )?),
            None => None,
        };
        let writer = CreateIndexTask::configure_writer(&config, run_data, OpenMode::APPEND, ic)?;
        run_data.set_index_writer(Some(writer));
        Ok(1)
    }

    fn set_params(&self, params: &str) -> Result<()> {
        perf_task::set_params(self, params)?;
        // specifies which commit point to open
        *jdk::lock(&self.commit_user_data) = Some(params.to_string());
        Ok(())
    }

    fn supports_params(&self) -> bool {
        true
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "OpenIndexTask"
    }
}
