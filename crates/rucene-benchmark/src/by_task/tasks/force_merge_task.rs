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

//! The types declared in `ForceMergeTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.ForceMergeTask`
/// (`public class ForceMergeTask extends PerfTask`): runs forceMerge on the
/// index.
///
/// Other side effects: none.
pub struct ForceMergeTask {
    base: PerfTaskBase,
    max_num_segments: AtomicI32,
}

impl ForceMergeTask {
    /// `ForceMergeTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(ForceMergeTask {
            base: PerfTaskBase::new(run_data, "ForceMergeTask")?,
            max_num_segments: AtomicI32::new(-1),
        })
    }
}

impl PerfTask for ForceMergeTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(ForceMergeTask {
            base: self.base.clone_base(),
            max_num_segments: AtomicI32::new(self.max_num_segments.load(Ordering::SeqCst)),
        }))
    }

    fn do_logic(&self) -> Result<i32> {
        let max_num_segments = self.max_num_segments.load(Ordering::SeqCst);
        if max_num_segments == -1 {
            return Err(LuceneError::illegal_state(
                "required argument (maxNumSegments) was not specified",
            ));
        }
        let iw = self
            .base
            .get_run_data()
            .get_index_writer()
            .ok_or_else(LuceneError::null_pointer)?;
        iw.force_merge(max_num_segments)?;
        // System.out.println("forceMerge called");
        Ok(1)
    }

    fn set_params(&self, params: &str) -> Result<()> {
        perf_task::set_params(self, params)?;
        self.max_num_segments
            .store(jdk::parse_double(params)? as i32, Ordering::SeqCst);
        Ok(())
    }

    fn supports_params(&self) -> bool {
        true
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "ForceMergeTask"
    }
}
