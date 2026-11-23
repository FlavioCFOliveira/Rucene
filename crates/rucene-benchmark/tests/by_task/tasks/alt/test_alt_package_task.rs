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

//! Port of `org.apache.lucene.benchmark.byTask.tasks.alt.TestAltPackageTask`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene_test_framework::util::java_exceptions::expect_throws;

use super::alt_test_task::AltTestTask;
use crate::benchmark_support::search_support::ftest;
use crate::benchmark_support::{benchmark_class, exec_benchmark};

/// `this.getClass().getPackage().getName()`.
const PACKAGE: &str = "org.apache.lucene.benchmark.byTask.tasks.alt";

/// The instance of `TestAltPackageTask`.
#[derive(Default)]
pub struct AltPackageTaskTest;

/// `TestAltPackageTask`.
pub fn class_def() -> Arc<Class> {
    AltTestTask::register();
    benchmark_class::<AltPackageTaskTest>(
        "org.apache.lucene.benchmark.byTask.tasks.alt.TestAltPackageTask",
        vec![ftest("testWithoutAlt", test_without_alt), ftest("testWithAlt", test_with_alt)],
    )
}

/// Benchmark should fail loading the algorithm when alt is not specified.
fn test_without_alt(_f: &mut AltPackageTaskTest) -> Result<()> {
    expect_throws("java.lang.Exception", || exec_benchmark(&alt_alg(false)));
    Ok(())
}

/// Benchmark should be able to load the algorithm when alt is specified.
fn test_with_alt(_f: &mut AltPackageTaskTest) -> Result<()> {
    let bm = exec_benchmark(&alt_alg(true))?;
    let _points = bm.get_run_data().get_points();
    Assert::assert_true(true); // assertNotNull(bm), assertNotNull(points): non-null by type
    Ok(())
}

fn alt_alg(allow_alt: bool) -> Vec<String> {
    let alt_task = "{ AltTest }".to_string();
    if allow_alt {
        return vec![format!("alt.tasks.packages = {PACKAGE}"), alt_task];
    }
    vec![alt_task]
}
