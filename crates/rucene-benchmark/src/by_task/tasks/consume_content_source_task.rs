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

//! The types declared in `ConsumeContentSourceTask.java`, of the Java
//! package `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use crate::by_task::feeds::content_source::ContentSource;
use crate::by_task::feeds::doc_data::DocData;
use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::jdk::ThreadLocal;

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.ConsumeContentSourceTask`
/// (`public class ConsumeContentSourceTask extends PerfTask`): consumes a
/// [`ContentSource`].
///
/// **Divergence.** Java's first call on a thread passes the thread-local's
/// `null` to `getNextDocData`, which throws `NullPointerException` where the
/// source first touches the `DocData` (`ConsumeContentSourceTask.java:43`).
/// Rust has no `null` `DocData`: the first call passes a fresh one and
/// raises the `NullPointerException` after the source returns, so the
/// source's work after that first touch also happens.
pub struct ConsumeContentSourceTask {
    base: PerfTaskBase,
    source: Arc<dyn ContentSource>,
    dd: Arc<ThreadLocal<DocData>>,
}

impl ConsumeContentSourceTask {
    /// `ConsumeContentSourceTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        let source = run_data.get_content_source();
        Ok(ConsumeContentSourceTask {
            base: PerfTaskBase::new(run_data, "ConsumeContentSourceTask")?,
            source,
            dd: Arc::new(ThreadLocal::new()),
        })
    }
}

impl PerfTask for ConsumeContentSourceTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(ConsumeContentSourceTask {
            base: self.base.clone_base(),
            source: Arc::clone(&self.source),
            dd: Arc::clone(&self.dd),
        }))
    }

    fn get_log_message(&self, recs_count: i32) -> String {
        format!("read {recs_count} documents from the content source")
    }

    fn do_logic(&self) -> Result<i32> {
        match self.dd.get() {
            Some(dd) => {
                let mut dd = crate::jdk::lock(&dd);
                self.source.get_next_doc_data(&mut dd)?;
            }
            None => {
                let mut fresh = DocData::new();
                self.source.get_next_doc_data(&mut fresh)?;
                return Err(LuceneError::null_pointer());
            }
        }
        Ok(1)
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "ConsumeContentSourceTask"
    }
}
