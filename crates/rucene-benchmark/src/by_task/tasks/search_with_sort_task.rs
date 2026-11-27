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

//! The types declared in `SearchWithSortTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::search::sort::{Sort, SortField, SortFieldType};

use crate::by_task::feeds::query_maker::QueryMaker;
use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::by_task::tasks::read_task::{self, query_maker_of, ReadTask, ReadTaskBase};
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.SearchWithSortTask`
/// (`public class SearchWithSortTask extends ReadTask`): does sort search on
/// specified field.
pub struct SearchWithSortTask {
    base: PerfTaskBase,
    read: ReadTaskBase,
    sort: Mutex<Option<Sort>>,
}

impl SearchWithSortTask {
    /// `SearchWithSortTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor or of the query maker.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        let base = PerfTaskBase::new(Arc::clone(&run_data), "SearchWithSortTask")?;
        let read = ReadTaskBase::new(true, || query_maker_of::<SearchWithSortTask>(&run_data))?;
        Ok(SearchWithSortTask {
            base,
            read,
            sort: Mutex::new(None),
        })
    }
}

/// `SortField.Type.valueOf(name)`.
fn sort_type_value_of(name: &str) -> Result<SortFieldType> {
    SortFieldType::parse(name).map_err(|e| match e.kind() {
        LuceneErrorKind::IllegalArgument { .. } => LuceneError::illegal_argument(format!(
            "No enum constant org.apache.lucene.search.SortField.Type.{name}"
        )),
        _ => e,
    })
}

impl PerfTask for SearchWithSortTask {
    read_task::read_task_perf_methods!("SearchWithSortTask");

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(SearchWithSortTask {
            base: self.base.clone_base(),
            read: self.read.clone_base(),
            sort: Mutex::new(jdk::lock(&self.sort).clone()),
        }))
    }

    /// SortFields: field:type,field:type[,noscore][,nomaxscore]
    ///
    /// If noscore is present, then we turn off score tracking in
    /// `TopFieldCollector`. If nomaxscore is present, then we turn off
    /// maxScore tracking in `TopFieldCollector`.
    ///
    /// name:string,page:int,subject:string
    fn set_params(&self, sort_field: &str) -> Result<()> {
        perf_task::set_params(self, sort_field)?;
        let fields = jdk::split(sort_field, ",");
        let mut sort_fields = Vec::with_capacity(fields.len());
        for field in &fields {
            let sort_field0 = if field == "doc" {
                SortField::FIELD_DOC.clone()
            } else if field == "score" {
                SortField::FIELD_SCORE.clone()
            } else {
                let Some(index) = field.rfind(':') else {
                    return Err(jdk::runtime_exception(
                        "You must specify the sort type ie page:int,subject:string",
                    ));
                };
                let field_name = &field[..index];
                let type_string = &field[1 + index..];
                SortField::new(
                    Some(field_name.to_string()),
                    sort_type_value_of(&type_string.to_uppercase())?,
                )?
            };
            sort_fields.push(sort_field0);
        }
        *jdk::lock(&self.sort) = Some(Sort::new_fields(sort_fields)?);
        Ok(())
    }

    fn supports_params(&self) -> bool {
        true
    }
}

impl ReadTask for SearchWithSortTask {
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

    fn get_sort(&self) -> Result<Option<Sort>> {
        match jdk::lock(&self.sort).clone() {
            None => Err(LuceneError::illegal_state("No sort field was set")),
            Some(sort) => Ok(Some(sort)),
        }
    }
}
