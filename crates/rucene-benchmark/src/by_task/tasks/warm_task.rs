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

//! The types declared in `WarmTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::Arc;

use rucene::error::Result;

use crate::by_task::feeds::query_maker::QueryMaker;
use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{PerfTask, PerfTaskBase};
use crate::by_task::tasks::read_task::{self, ReadTask, ReadTaskBase};

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.WarmTask`
/// (`public class WarmTask extends ReadTask`): warm reader task: retrieve
/// all reader documents.
///
/// Note: This task reuses the reader if it is already open. Otherwise a
/// reader is opened at start and closed at the end.
///
/// Other side effects: counts additional 1 (record) for each retrieved (non
/// null) document.
pub struct WarmTask {
    base: PerfTaskBase,
    read: ReadTaskBase,
}

impl WarmTask {
    /// `WarmTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        let base = PerfTaskBase::new(run_data, "WarmTask")?;
        let read = ReadTaskBase::new(false, || Ok(None))?;
        Ok(WarmTask { base, read })
    }
}

impl PerfTask for WarmTask {
    read_task::read_task_perf_methods!("WarmTask");

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(WarmTask { base: self.base.clone_base(), read: self.read.clone_base() }))
    }
}

impl ReadTask for WarmTask {
    fn read_base(&self) -> &ReadTaskBase {
        &self.read
    }

    fn get_query_maker(&self) -> Result<Option<Arc<dyn QueryMaker>>> {
        Ok(None) // not required for this task.
    }

    fn with_retrieve(&self) -> bool {
        false
    }

    fn with_search(&self) -> bool {
        false
    }

    fn with_traverse(&self) -> bool {
        false
    }

    fn with_warm(&self) -> bool {
        true
    }
}
