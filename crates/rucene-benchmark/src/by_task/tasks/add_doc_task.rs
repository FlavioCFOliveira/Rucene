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

//! The types declared in `AddDocTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use rucene::document::Document;
use rucene::error::{LuceneError, Result};

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::jdk;

/// The fields of `AddDocTask`, which `AddFacetedDocTask` inherits.
pub struct AddDocState {
    doc_size: AtomicI32,
    /// `doc`: volatile data passed between setup(), doLogic(), tearDown().
    /// the doc is created at setup() and added at doLogic().
    pub doc: Mutex<Option<Document>>,
}

impl AddDocState {
    /// `docSize = 0`, `doc = null`.
    pub fn new() -> Self {
        AddDocState {
            doc_size: AtomicI32::new(0),
            doc: Mutex::new(None),
        }
    }

    /// `super.clone()`: the clone shares the document object.
    pub fn clone_state(&self) -> Self {
        let doc = jdk::lock(&self.doc).as_ref().map(|d| {
            let mut copy = Document::new();
            for f in d.get_fields() {
                copy.add_shared(Arc::clone(f));
            }
            copy
        });
        AddDocState {
            doc_size: AtomicI32::new(self.doc_size.load(Ordering::SeqCst)),
            doc: Mutex::new(doc),
        }
    }

    /// `AddDocTask.setup()`.
    ///
    /// # Errors
    ///
    /// The exception of the doc maker.
    pub fn setup(&self, run_data: &PerfRunData) -> Result<()> {
        let doc_maker = run_data.get_doc_maker();
        let doc_size = self.doc_size.load(Ordering::SeqCst);
        let doc = if doc_size > 0 {
            doc_maker.make_document_of_size(doc_size)?
        } else {
            doc_maker.make_document()?
        };
        *jdk::lock(&self.doc) = Some(doc);
        Ok(())
    }

    /// `AddDocTask.doLogic()`.
    ///
    /// # Errors
    ///
    /// The exception of `addDocument`.
    pub fn do_logic(&self, run_data: &PerfRunData) -> Result<i32> {
        let writer = run_data
            .get_index_writer()
            .ok_or_else(LuceneError::null_pointer)?;
        let doc = jdk::lock(&self.doc);
        writer.add_document(doc.as_ref())?;
        Ok(1)
    }

    /// `AddDocTask.setParams(String)` after `super.setParams`.
    ///
    /// # Errors
    ///
    /// `NumberFormatException`.
    pub fn set_params(&self, params: &str) -> Result<()> {
        self.doc_size
            .store(jdk::parse_float(params)? as i32, Ordering::SeqCst);
        Ok(())
    }
}

impl Default for AddDocState {
    fn default() -> Self {
        Self::new()
    }
}

/// `AddDocTask.getLogMessage(int)`: `String.format(Locale.ROOT, "added %9d
/// docs", recsCount)`.
pub(crate) fn add_doc_log_message(recs_count: i32) -> String {
    format!("added {recs_count:>9} docs")
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.AddDocTask`
/// (`public class AddDocTask extends PerfTask`): add a document, optionally
/// of a certain size.
///
/// Other side effects: none.
///
/// Takes optional param: document size.
pub struct AddDocTask {
    base: PerfTaskBase,
    add: AddDocState,
}

impl AddDocTask {
    /// `AddDocTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(AddDocTask {
            base: PerfTaskBase::new(run_data, "AddDocTask")?,
            add: AddDocState::new(),
        })
    }
}

impl PerfTask for AddDocTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(AddDocTask {
            base: self.base.clone_base(),
            add: self.add.clone_state(),
        }))
    }

    fn setup(&self) -> Result<()> {
        self.add.setup(self.base.get_run_data())
    }

    fn tear_down(&self) -> Result<()> {
        *jdk::lock(&self.add.doc) = None;
        perf_task::tear_down(self)
    }

    fn get_log_message(&self, recs_count: i32) -> String {
        add_doc_log_message(recs_count)
    }

    fn do_logic(&self) -> Result<i32> {
        self.add.do_logic(self.base.get_run_data())
    }

    /// Set the params (docSize only): docSize, or 0 for no limit.
    fn set_params(&self, params: &str) -> Result<()> {
        perf_task::set_params(self, params)?;
        self.add.set_params(params)
    }

    fn supports_params(&self) -> bool {
        true
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "AddDocTask"
    }
}
