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

//! The types declared in `SearchWithCollectorTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};
use rucene::search::top_score_doc_collector_manager::TopScoreDocCollectorManager;

use crate::by_task::feeds::query_maker::QueryMaker;
use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{PerfTask, PerfTaskBase};
use crate::by_task::tasks::read_task::{
    self, query_maker_of, AnyCollectorManager, ErasedCollectorManager, ReadTask, ReadTaskBase,
};
use crate::classes;
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.SearchWithCollectorTask`
/// (`public class SearchWithCollectorTask extends SearchTask`): does search
/// w/ a custom collector.
pub struct SearchWithCollectorTask {
    base: PerfTaskBase,
    read: ReadTaskBase,
    /// `clnName`.
    cln_name: Mutex<Option<String>>,
}

impl SearchWithCollectorTask {
    /// `SearchWithCollectorTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor or of the query maker.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        let base = PerfTaskBase::new(Arc::clone(&run_data), "SearchWithCollectorTask")?;
        let read = ReadTaskBase::new(true, || {
            query_maker_of::<SearchWithCollectorTask>(&run_data)
        })?;
        Ok(SearchWithCollectorTask {
            base,
            read,
            cln_name: Mutex::new(None),
        })
    }
}

impl PerfTask for SearchWithCollectorTask {
    fn base(&self) -> &PerfTaskBase {
        &self.base
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn do_logic(&self) -> Result<i32> {
        read_task::do_logic(self)
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        matches!(
            simple_class_name,
            "SearchWithCollectorTask" | "SearchTask" | "ReadTask"
        )
    }

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(SearchWithCollectorTask {
            base: self.base.clone_base(),
            read: self.read.clone_base(),
            cln_name: Mutex::new(jdk::lock(&self.cln_name).clone()),
        }))
    }

    fn setup(&self) -> Result<()> {
        read_task::setup(self)?;
        // check to make sure either the doc is being stored
        let config = Arc::clone(self.base.get_run_data().get_config());
        if config.get_str("collector.class", None)?.is_some() {
            return Err(LuceneError::illegal_argument(
                "collector.class is no longer supported as a config parameter, use collector.manager.class instead to provide a CollectorManager class name",
            ));
        }
        *jdk::lock(&self.cln_name) = config.get_str("collector.manager.class", Some(""))?;
        Ok(())
    }
}

impl ReadTask for SearchWithCollectorTask {
    fn read_base(&self) -> &ReadTaskBase {
        &self.read
    }

    fn with_collector(&self) -> bool {
        true
    }

    fn create_collector_manager(&self) -> Result<Box<dyn AnyCollectorManager>> {
        let cln_name = jdk::lock(&self.cln_name)
            .clone()
            .ok_or_else(LuceneError::null_pointer)?;
        if cln_name.eq_ignore_ascii_case("topScoreDoc") {
            Ok(Box::new(ErasedCollectorManager(
                TopScoreDocCollectorManager::new(self.num_hits(), None, i32::MAX)?,
            )))
        } else if !cln_name.is_empty() {
            classes::collector_manager_for_name(&cln_name)
        } else {
            read_task::create_collector_manager(self)
        }
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
