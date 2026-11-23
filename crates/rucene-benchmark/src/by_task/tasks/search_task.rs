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

//! The types declared in `SearchTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::Arc;

use rucene::error::Result;

use crate::by_task::feeds::query_maker::QueryMaker;
use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{PerfTask, PerfTaskBase};
use crate::by_task::tasks::read_task::{self, query_maker_of, ReadTask, ReadTaskBase};

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.SearchTask`
/// (`public class SearchTask extends ReadTask`): search task.
///
/// Note: This task reuses the reader if it is already open. Otherwise a
/// reader is opened at start and closed at the end.
pub struct SearchTask {
    base: PerfTaskBase,
    read: ReadTaskBase,
}

impl SearchTask {
    /// `SearchTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor or of the query maker.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        let base = PerfTaskBase::new(Arc::clone(&run_data), "SearchTask")?;
        let read = ReadTaskBase::new(true, || query_maker_of::<SearchTask>(&run_data))?;
        Ok(SearchTask { base, read })
    }
}

impl PerfTask for SearchTask {
    read_task::read_task_perf_methods!("SearchTask");

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(SearchTask {
            base: self.base.clone_base(),
            read: self.read.clone_base(),
        }))
    }
}

impl ReadTask for SearchTask {
    fn read_base(&self) -> &ReadTaskBase {
        &self.read
    }

    fn get_query_maker(&self) -> Result<Option<Arc<dyn QueryMaker>>> {
        self.base.get_run_data().get_query_maker(self).map(Some)
    }

    fn with_retrieve(&self) -> bool {
        false
    }

    fn with_search(&self) -> bool {
        true
    }

    fn with_traverse(&self) -> bool {
        false
    }

    fn with_warm(&self) -> bool {
        false
    }
}
