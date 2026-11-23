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

//! The types declared in `UpdateDocTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use rucene::document::Document;
use rucene::error::{LuceneError, Result};
use rucene::index::terms::Term;

use crate::by_task::feeds::doc_maker::DocMaker;
use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.UpdateDocTask`
/// (`public class UpdateDocTask extends PerfTask`): update a document, using
/// IndexWriter.updateDocument, optionally with of a certain size.
///
/// Other side effects: none.
///
/// Takes optional param: document size.
pub struct UpdateDocTask {
    base: PerfTaskBase,
    doc_size: AtomicI32,
    /// volatile data passed between setup(), doLogic(), tearDown().
    doc: Mutex<Option<Document>>,
}

impl UpdateDocTask {
    /// `UpdateDocTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(UpdateDocTask {
            base: PerfTaskBase::new(run_data, "UpdateDocTask")?,
            doc_size: AtomicI32::new(0),
            doc: Mutex::new(None),
        })
    }
}

impl PerfTask for UpdateDocTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        let doc = jdk::lock(&self.doc).as_ref().map(|d| {
            let mut copy = Document::new();
            for f in d.get_fields() {
                copy.add_shared(Arc::clone(f));
            }
            copy
        });
        Ok(Arc::new(UpdateDocTask {
            base: self.base.clone_base(),
            doc_size: AtomicI32::new(self.doc_size.load(Ordering::SeqCst)),
            doc: Mutex::new(doc),
        }))
    }

    fn setup(&self) -> Result<()> {
        let doc_maker = self.base.get_run_data().get_doc_maker();
        let doc_size = self.doc_size.load(Ordering::SeqCst);
        let doc = if doc_size > 0 {
            doc_maker.make_document_of_size(doc_size)?
        } else {
            doc_maker.make_document()?
        };
        *jdk::lock(&self.doc) = Some(doc);
        Ok(())
    }

    fn tear_down(&self) -> Result<()> {
        *jdk::lock(&self.doc) = None;
        perf_task::tear_down(self)
    }

    fn do_logic(&self) -> Result<i32> {
        let doc = jdk::lock(&self.doc);
        let d = doc.as_ref().ok_or_else(LuceneError::null_pointer)?;
        let Some(doc_id) = d.get(DocMaker::ID_FIELD) else {
            return Err(LuceneError::illegal_state(
                "document must define the docid field",
            ));
        };
        let iw = self
            .base
            .get_run_data()
            .get_index_writer()
            .ok_or_else(LuceneError::null_pointer)?;
        iw.update_document(Term::from_text(DocMaker::ID_FIELD, &doc_id), Some(d))?;
        Ok(1)
    }

    fn get_log_message(&self, recs_count: i32) -> String {
        format!("updated {recs_count} docs")
    }

    /// Set the params (docSize only): docSize, or 0 for no limit.
    fn set_params(&self, params: &str) -> Result<()> {
        perf_task::set_params(self, params)?;
        self.doc_size
            .store(jdk::parse_float(params)? as i32, Ordering::SeqCst);
        Ok(())
    }

    fn supports_params(&self) -> bool {
        true
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "UpdateDocTask"
    }
}
