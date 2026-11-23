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

//! Port of `org.apache.lucene.benchmark.byTask.utils.TestConfig`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene_benchmark::by_task::utils::config::Config;
use rucene_benchmark::jdk::properties::Properties;

use crate::benchmark_support::core_support::{class, test};

/// `TestConfig`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.benchmark.byTask.utils.TestConfig",
        vec![test("testAbsolutePathNamesWindows", test_absolute_path_names_windows)],
    )
}

fn test_absolute_path_names_windows() -> Result<()> {
    let mut props = Properties::new();
    props.set_property("work.dir1", "c:\\temp");
    props.set_property("work.dir2", "c:/temp");
    let conf = Config::from_properties(props);
    Assert::assert_equals("c:\\temp", conf.get_str("work.dir1", Some(""))?.as_deref().unwrap());
    Assert::assert_equals("c:/temp", conf.get_str("work.dir2", Some(""))?.as_deref().unwrap());
    Ok(())
}
