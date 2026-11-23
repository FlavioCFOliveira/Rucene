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

//! The types declared in `PrintReaderTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::open_reader_task::OpenReaderTask;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.PrintReaderTask`
/// (`public class PrintReaderTask extends PerfTask`): opens a reader and
/// prints basic statistics.
pub struct PrintReaderTask {
    base: PerfTaskBase,
    user_data: Mutex<Option<String>>,
}

impl PrintReaderTask {
    /// `PrintReaderTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(PrintReaderTask { base: PerfTaskBase::new(run_data, "PrintReaderTask")?, user_data: Mutex::new(None) })
    }
}

impl PerfTask for PrintReaderTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(PrintReaderTask {
            base: self.base.clone_base(),
            user_data: Mutex::new(jdk::lock(&self.user_data).clone()),
        }))
    }

    fn set_params(&self, params: &str) -> Result<()> {
        perf_task::set_params(self, params)?;
        *jdk::lock(&self.user_data) = Some(params.to_string());
        Ok(())
    }

    fn supports_params(&self) -> bool {
        true
    }

    fn do_logic(&self) -> Result<i32> {
        let dir = self.base.get_run_data().get_directory().ok_or_else(LuceneError::null_pointer)?;
        let user_data = jdk::lock(&self.user_data).clone();
        let r = match user_data {
            None => directory_reader::open(dir)?,
            Some(user_data) => directory_reader::open_with_commit(&*OpenReaderTask::find_index_commit(dir, &user_data)?)?,
        };
        println!("--> numDocs:{} dels:{}", r.num_docs(), r.num_deleted_docs());
        r.close()?;
        Ok(1)
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "PrintReaderTask"
    }
}
