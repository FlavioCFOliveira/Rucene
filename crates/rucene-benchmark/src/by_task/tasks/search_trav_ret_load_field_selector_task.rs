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

//! The types declared in `SearchTravRetLoadFieldSelectorTask.java`, of the
//! Java package `org.apache.lucene.benchmark.byTask.tasks`.

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use rucene::document::{Document, DocumentStoredFieldVisitor};
use rucene::error::Result;
use rucene::index::index_reader::StoredFields;

use crate::by_task::feeds::query_maker::QueryMaker;
use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{PerfTask, PerfTaskBase};
use crate::by_task::tasks::read_task::{self, query_maker_of, ReadTask, ReadTaskBase};
use crate::by_task::tasks::search_trav_task::SearchTravState;
use crate::jdk;

/// Equivalent to
/// `org.apache.lucene.benchmark.byTask.tasks.SearchTravRetLoadFieldSelectorTask`
/// (`public class SearchTravRetLoadFieldSelectorTask extends
/// SearchTravTask`): search and Traverse and Retrieve docs task using a
/// FieldVisitor loading only the requested fields.
///
/// Note: This task reuses the reader if it is already open. Otherwise a
/// reader is opened at start and closed at the end.
///
/// Takes optional param: comma separated list of Fields to load.
///
/// Other side effects: counts additional 1 (record) for each traversed hit,
/// and 1 more for each retrieved (non null) document.
pub struct SearchTravRetLoadFieldSelectorTask {
    base: PerfTaskBase,
    read: ReadTaskBase,
    trav: SearchTravState,
    /// `fieldsToLoad`.
    fields_to_load: Mutex<Option<HashSet<String>>>,
}

impl SearchTravRetLoadFieldSelectorTask {
    /// `SearchTravRetLoadFieldSelectorTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor or of the query maker.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        let base = PerfTaskBase::new(Arc::clone(&run_data), "SearchTravRetLoadFieldSelectorTask")?;
        let read = ReadTaskBase::new(true, || {
            query_maker_of::<SearchTravRetLoadFieldSelectorTask>(&run_data)
        })?;
        Ok(SearchTravRetLoadFieldSelectorTask {
            base,
            read,
            trav: SearchTravState::new(),
            fields_to_load: Mutex::new(None),
        })
    }
}

impl PerfTask for SearchTravRetLoadFieldSelectorTask {
    read_task::read_task_perf_methods!("SearchTravRetLoadFieldSelectorTask", "SearchTravTask");

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(SearchTravRetLoadFieldSelectorTask {
            base: self.base.clone_base(),
            read: self.read.clone_base(),
            trav: self.trav.clone_state(),
            fields_to_load: Mutex::new(jdk::lock(&self.fields_to_load).clone()),
        }))
    }

    fn set_params(&self, params: &str) -> Result<()> {
        self.base.store_params(params); // cannot just call super.setParams(), b/c its params differ.
        let fields: HashSet<String> = params
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        *jdk::lock(&self.fields_to_load) = Some(fields);
        Ok(())
    }

    fn supports_params(&self) -> bool {
        true
    }
}

impl ReadTask for SearchTravRetLoadFieldSelectorTask {
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

    fn retrieve_doc(&self, stored_fields: &dyn StoredFields, id: i32) -> Result<Option<Document>> {
        let fields = jdk::lock(&self.fields_to_load).clone();
        match fields {
            None => stored_fields.document(id).map(Some),
            Some(fields) => {
                let mut visitor = DocumentStoredFieldVisitor::with_fields(fields);
                stored_fields.document_with_visitor(id, &mut visitor)?;
                Ok(Some(visitor.into_document()))
            }
        }
    }
}
