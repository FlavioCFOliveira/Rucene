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

//! The types declared in `CloseReaderTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.CloseReaderTask`
/// (`public class CloseReaderTask extends PerfTask`): close index reader.
///
/// Other side effects: index reader in perfRunData is nullified.
///
/// This would cause read related tasks to reopen their own reader.
pub struct CloseReaderTask {
    base: PerfTaskBase,
}

impl CloseReaderTask {
    /// `CloseReaderTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(CloseReaderTask {
            base: PerfTaskBase::new(run_data, "CloseReaderTask")?,
        })
    }
}

impl PerfTask for CloseReaderTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(CloseReaderTask {
            base: self.base.clone_base(),
        }))
    }

    fn do_logic(&self) -> Result<i32> {
        let run_data = self.base.get_run_data();
        let reader = run_data.get_index_reader()?;
        run_data.set_index_reader(None)?;
        let reader = reader.ok_or_else(LuceneError::null_pointer)?;
        if reader.get_ref_count() != 1 {
            println!(
                "WARNING: CloseReader: reference count is currently {}",
                reader.get_ref_count()
            );
        }
        reader.dec_ref()?;
        Ok(1)
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "CloseReaderTask"
    }
}
