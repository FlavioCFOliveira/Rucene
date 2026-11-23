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

//! The types declared in `CommitTaxonomyIndexTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.CommitTaxonomyIndexTask`
/// (`public class CommitTaxonomyIndexTask extends PerfTask`): commits the Taxonomy Index.
pub struct CommitTaxonomyIndexTask {
    base: PerfTaskBase,
}

impl CommitTaxonomyIndexTask {
    /// `CommitTaxonomyIndexTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(CommitTaxonomyIndexTask {
            base: PerfTaskBase::new(run_data, "CommitTaxonomyIndexTask")?,
        })
    }
}

impl PerfTask for CommitTaxonomyIndexTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(CommitTaxonomyIndexTask {
            base: self.base.clone_base(),
        }))
    }

    fn do_logic(&self) -> Result<i32> {
        let run_data = self.base.get_run_data();
        match run_data.get_taxonomy_writer() {
            Some(taxonomy_writer) => {
                taxonomy_writer.commit()?;
            }
            None => {
                return Err(LuceneError::illegal_state(
                    "TaxonomyWriter is not currently open",
                ))
            }
        }
        Ok(1)
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "CommitTaxonomyIndexTask"
    }
}
