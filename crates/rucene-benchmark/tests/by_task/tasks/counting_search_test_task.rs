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

//! Port of `org.apache.lucene.benchmark.byTask.tasks.CountingSearchTestTask`.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use rucene::error::Result;
use rucene_benchmark::by_task::feeds::query_maker::QueryMaker;
use rucene_benchmark::by_task::perf_run_data::PerfRunData;
use rucene_benchmark::by_task::tasks::perf_task::{PerfTask, PerfTaskBase};
use rucene_benchmark::by_task::tasks::read_task::{self, query_maker_of, ReadTask, ReadTaskBase};
use rucene_benchmark::classes;

/// The binary name of `CountingSearchTestTask`.
pub const CLASS_NAME: &str = "org.apache.lucene.benchmark.byTask.tasks.CountingSearchTestTask";

/// The static fields `numSearches`, `startNanos`, `lastNanos`,
/// `prevLastNanos`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Counters {
    pub num_searches: i32,
    pub start_nanos: i64,
    pub last_nanos: i64,
    pub prev_last_nanos: i64,
}

static COUNTERS: Mutex<Counters> = Mutex::new(Counters {
    num_searches: 0,
    start_nanos: 0,
    last_nanos: 0,
    prev_last_nanos: 0,
});

/// The static fields.
pub fn counters() -> Counters {
    *COUNTERS.lock().unwrap_or_else(|e| e.into_inner())
}

/// `CountingSearchTestTask.numSearches = 0`.
pub fn reset_num_searches() {
    COUNTERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .num_searches = 0;
}

/// `System.nanoTime()`.
fn nano_time() -> i64 {
    static ORIGIN: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
    ORIGIN.elapsed().as_nanos() as i64
}

/// `CountingSearchTestTask extends SearchTask`: test search task which
/// counts number of searches.
pub struct CountingSearchTestTask {
    base: PerfTaskBase,
    read: ReadTaskBase,
}

impl CountingSearchTestTask {
    /// `CountingSearchTestTask(PerfRunData runData)`.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        let base = PerfTaskBase::new(Arc::clone(&run_data), "CountingSearchTestTask")?;
        let read = ReadTaskBase::new(true, || query_maker_of::<CountingSearchTestTask>(&run_data))?;
        Ok(CountingSearchTestTask { base, read })
    }

    /// Puts the class on the "classpath".
    pub fn register() {
        classes::register_task_class(
            CLASS_NAME,
            Arc::new(|run_data| {
                Ok(Arc::new(CountingSearchTestTask::new(run_data)?) as Arc<dyn PerfTask>)
            }),
        );
    }

    /// `incrNumSearches()` (`private static synchronized`).
    fn incr_num_searches() {
        let mut c = COUNTERS.lock().unwrap_or_else(|e| e.into_inner());
        c.prev_last_nanos = c.last_nanos;
        c.last_nanos = nano_time();
        if 0 == c.num_searches {
            c.start_nanos = c.last_nanos;
            c.prev_last_nanos = c.last_nanos;
        }
        c.num_searches += 1;
    }

    /// `getElapsedMillis()` (no caller in the Lucene tests either).
    #[allow(dead_code)]
    pub fn get_elapsed_millis(&self) -> i64 {
        let c = counters();
        (c.last_nanos - c.start_nanos) / 1_000_000
    }
}

impl PerfTask for CountingSearchTestTask {
    fn base(&self) -> &PerfTaskBase {
        &self.base
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(CountingSearchTestTask {
            base: self.base.clone_base(),
            read: self.read.clone_base(),
        }))
    }

    fn do_logic(&self) -> Result<i32> {
        let res = read_task::do_logic(self)?;
        Self::incr_num_searches();
        Ok(res)
    }

    fn setup(&self) -> Result<()> {
        read_task::setup(self)
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        matches!(
            simple_class_name,
            "CountingSearchTestTask" | "SearchTask" | "ReadTask"
        )
    }
}

impl ReadTask for CountingSearchTestTask {
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
