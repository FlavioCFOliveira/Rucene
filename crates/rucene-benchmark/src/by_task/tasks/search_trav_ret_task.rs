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

//! The types declared in `SearchTravRetTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use rucene::error::Result;

use crate::by_task::feeds::query_maker::QueryMaker;
use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::by_task::tasks::read_task::{self, query_maker_of, ReadTask, ReadTaskBase};
use crate::by_task::tasks::search_trav_task::SearchTravState;

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.SearchTravRetTask`
/// (`public class SearchTravRetTask extends SearchTravTask`): search and
/// Traverse and Retrieve docs task.
///
/// Note: This task reuses the reader if it is already open. Otherwise a
/// reader is opened at start and closed at the end.
///
/// Takes optional param: traversal size (otherwise all results are
/// traversed).
///
/// Other side effects: counts additional 1 (record) for each traversed hit,
/// and 1 more for each retrieved (non null) document.
pub struct SearchTravRetTask {
    base: PerfTaskBase,
    read: ReadTaskBase,
    trav: SearchTravState,
}

impl SearchTravRetTask {
    /// `SearchTravRetTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor or of the query maker.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        let base = PerfTaskBase::new(Arc::clone(&run_data), "SearchTravRetTask")?;
        let read = ReadTaskBase::new(true, || query_maker_of::<SearchTravRetTask>(&run_data))?;
        Ok(SearchTravRetTask {
            base,
            read,
            trav: SearchTravState::new(),
        })
    }
}

impl PerfTask for SearchTravRetTask {
    read_task::read_task_perf_methods!("SearchTravRetTask", "SearchTravTask");

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(SearchTravRetTask {
            base: self.base.clone_base(),
            read: self.read.clone_base(),
            trav: self.trav.clone_state(),
        }))
    }

    fn set_params(&self, params: &str) -> Result<()> {
        perf_task::set_params(self, params)?;
        self.trav.set_params(params)
    }

    fn supports_params(&self) -> bool {
        true
    }
}

impl ReadTask for SearchTravRetTask {
    fn read_base(&self) -> &ReadTaskBase {
        &self.read
    }

    fn get_query_maker(&self) -> Result<Option<Arc<dyn QueryMaker>>> {
        self.base.get_run_data().get_query_maker(self).map(Some)
    }

    fn with_retrieve(&self) -> bool {
        true
    }

    fn with_search(&self) -> bool {
        true
    }

    fn with_traverse(&self) -> bool {
        true
    }

    fn with_warm(&self) -> bool {
        false
    }

    fn traversal_size(&self) -> i32 {
        self.trav.traversal_size.load(Ordering::SeqCst)
    }
}
