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

//! Port of `org.apache.lucene.benchmark.byTask.tasks.TestSearchWithSortTask`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::sort::SortFieldType;
use rucene_benchmark::by_task::tasks::perf_task::PerfTask;
use rucene_benchmark::by_task::tasks::read_task::ReadTask;
use rucene_benchmark::by_task::tasks::search_with_sort_task::SearchWithSortTask;
use rucene_benchmark::jdk::properties::Properties;

use crate::benchmark_support::search_support::ftest;
use crate::benchmark_support::{benchmark_class, run_data_of};

/// The instance of `TestSearchWithSortTask`.
#[derive(Default)]
pub struct SearchWithSortTaskTest;

/// `TestSearchWithSortTask`.
pub fn class_def() -> Arc<Class> {
    benchmark_class::<SearchWithSortTaskTest>(
        "org.apache.lucene.benchmark.byTask.tasks.TestSearchWithSortTask",
        vec![ftest("testSetParams_docField", test_set_params_doc_field)],
    )
}

fn test_set_params_doc_field(_f: &mut SearchWithSortTaskTest) -> Result<()> {
    let task = SearchWithSortTask::new(run_data_of(Properties::new())?)?;
    task.set_params("doc")?;
    let sort = task.get_sort()?.expect("sort set");
    Assert::assert_equals(SortFieldType::Doc, sort.fields()[0].field_type());
    Ok(())
}
