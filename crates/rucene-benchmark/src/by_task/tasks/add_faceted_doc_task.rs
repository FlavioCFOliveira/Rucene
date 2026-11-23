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

//! The types declared in `AddFacetedDocTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::{Arc, Mutex};

use rucene::error::Result;
use rucene_facet::facet_field::FacetField;
use rucene_facet::facets_config::FacetsConfig;

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::add_doc_task::{add_doc_log_message, AddDocState};
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.AddFacetedDocTask`
/// (`public class AddFacetedDocTask extends AddDocTask`): add a faceted
/// document.
///
/// Config properties:
///
/// * **with.facets**=<tells whether to actually add any facets to the
///   document| Default: true>. This config property allows to easily
///   compare the performance of adding docs with and without facets. Note
///   that facets are created even when this is false, just that they are not
///   added to the document (nor to the taxonomy).
///
/// See [`AddDocTask`](crate::by_task::tasks::add_doc_task::AddDocTask) for
/// general document parameters and configuration.
///
/// Makes use of the `FacetSource` in effect - see [`PerfRunData`] for facet
/// source settings.
pub struct AddFacetedDocTask {
    base: PerfTaskBase,
    add: AddDocState,
    config: Mutex<Option<Arc<FacetsConfig>>>,
}

impl AddFacetedDocTask {
    /// `AddFacetedDocTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(AddFacetedDocTask {
            base: PerfTaskBase::new(run_data, "AddFacetedDocTask")?,
            add: AddDocState::new(),
            config: Mutex::new(None),
        })
    }
}

impl PerfTask for AddFacetedDocTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(AddFacetedDocTask {
            base: self.base.clone_base(),
            add: self.add.clone_state(),
            config: Mutex::new(jdk::lock(&self.config).clone()),
        }))
    }

    fn setup(&self) -> Result<()> {
        self.add.setup(self.base.get_run_data())?;
        let mut config = jdk::lock(&self.config);
        if config.is_none() {
            let with_facets = self.base.get_run_data().get_config().get_bool("with.facets", true)?;
            if with_facets {
                let facets_source = self.base.get_run_data().get_facet_source();
                let c = FacetsConfig::new();
                facets_source.configure(&c);
                *config = Some(Arc::new(c));
            }
        }
        Ok(())
    }

    fn tear_down(&self) -> Result<()> {
        *jdk::lock(&self.add.doc) = None;
        perf_task::tear_down(self)
    }

    fn get_log_message(&self, recs_count: i32) -> String {
        if jdk::lock(&self.config).is_none() {
            return add_doc_log_message(recs_count);
        }
        format!("{} with facets", add_doc_log_message(recs_count))
    }

    fn do_logic(&self) -> Result<i32> {
        let config = jdk::lock(&self.config).clone();
        if let Some(config) = config {
            let run_data = self.base.get_run_data();
            let mut facets: Vec<FacetField> = Vec::new();
            run_data.get_facet_source().get_next_facets(&mut facets)?;
            let mut doc = jdk::lock(&self.add.doc);
            let d = doc.as_mut().ok_or_else(rucene::error::LuceneError::null_pointer)?;
            for ff in facets {
                d.add(ff);
            }
            let taxonomy_writer = run_data.get_taxonomy_writer();
            let built = config.build_with_taxonomy(taxonomy_writer.as_deref(), d)?;
            *doc = Some(built);
        }
        self.add.do_logic(self.base.get_run_data())
    }

    fn set_params(&self, params: &str) -> Result<()> {
        perf_task::set_params(self, params)?;
        self.add.set_params(params)
    }

    fn supports_params(&self) -> bool {
        true
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        matches!(simple_class_name, "AddFacetedDocTask" | "AddDocTask")
    }
}
