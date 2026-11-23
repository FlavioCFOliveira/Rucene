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

//! The types declared in `SetPropTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::{Arc, Mutex};

use rucene::error::Result;

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.SetPropTask`
/// (`public class SetPropTask extends PerfTask`): set a performance test
/// configuration property. A property may have a single value, or a
/// sequence of values, separated by ":". If a sequence of values is
/// specified, each time a new round starts, the next (cyclic) value is
/// taken.
///
/// Other side effects: none.
///
/// Takes mandatory param: "name,value" pair.
pub struct SetPropTask {
    base: PerfTaskBase,
    /// `(name, value)`.
    state: Mutex<(Option<String>, Option<String>)>,
}

impl SetPropTask {
    /// `SetPropTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(SetPropTask { base: PerfTaskBase::new(run_data, "SetPropTask")?, state: Mutex::new((None, None)) })
    }
}

impl PerfTask for SetPropTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(SetPropTask { base: self.base.clone_base(), state: Mutex::new(jdk::lock(&self.state).clone()) }))
    }

    fn do_logic(&self) -> Result<i32> {
        let (name, value) = jdk::lock(&self.state).clone();
        match (name, value) {
            (Some(name), Some(value)) => {
                self.base.get_run_data().get_config().set(&name, &value)?;
                Ok(0)
            }
            (name, value) => Err(jdk::exception(format!(
                "{} - undefined name or value: name={} value={}",
                self.get_name(),
                name.as_deref().unwrap_or("null"),
                value.as_deref().unwrap_or("null")
            ))),
        }
    }

    /// Set the params (property name and value), separated by ','.
    fn set_params(&self, params: &str) -> Result<()> {
        perf_task::set_params(self, params)?;
        // `params.indexOf(',')` is -1 without a comma: `substring(0, -1)`
        // throws StringIndexOutOfBoundsException.
        let Some(k) = params.find(',') else {
            return Err(rucene::error::LuceneError::unchecked(
                "java.lang.StringIndexOutOfBoundsException",
                format!("begin 0, end -1, length {}", jdk::utf16_len(params)),
            ));
        };
        let name = jdk::java_trim(&params[..k]).to_string();
        let value = jdk::java_trim(&params[k + 1..]).to_string();
        *jdk::lock(&self.state) = (Some(name), Some(value));
        Ok(())
    }

    fn supports_params(&self) -> bool {
        true
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "SetPropTask"
    }
}
