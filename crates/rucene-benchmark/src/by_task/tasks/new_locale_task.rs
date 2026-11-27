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

//! The types declared in `NewLocaleTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::{Arc, Mutex};

use rucene::error::Result;
use rucene_analysis_common::jdk::locale::Locale;

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.NewLocaleTask`
/// (`public class NewLocaleTask extends PerfTask`): set a `java.util.Locale`
/// for use in benchmarking.
///
/// Locales can be specified as BCP-47 language tag or as ROOT or empty
/// string.
///
/// * `de`: Language "de"
/// * `en-US`: Language "en", country "US"
/// * `ROOT`: The root (language-agnostic) Locale
/// * <empty string>: Erase the Locale (null)
pub struct NewLocaleTask {
    base: PerfTaskBase,
    tag: Mutex<Option<String>>,
}

impl NewLocaleTask {
    /// `NewLocaleTask(PerfRunData runData)`: create a new `Locale` and set
    /// it in the run data for use by all future tasks.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(NewLocaleTask {
            base: PerfTaskBase::new(run_data, "NewLocaleTask")?,
            tag: Mutex::new(None),
        })
    }

    /// `createLocale(String tag)`.
    ///
    /// # Errors
    ///
    /// The `IllformedLocaleException` of `Locale.Builder.setLanguageTag`.
    pub(crate) fn create_locale(tag: Option<&str>) -> Result<Option<Locale>> {
        let Some(tag) = tag.filter(|t| !t.is_empty()) else {
            return Ok(None);
        };
        if tag.eq_ignore_ascii_case("ROOT") {
            return Ok(Some(Locale::of_parts("", "", "", "")));
        }
        Locale::builder_from_language_tag(tag).map(Some)
    }
}

impl PerfTask for NewLocaleTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(NewLocaleTask {
            base: self.base.clone_base(),
            tag: Mutex::new(jdk::lock(&self.tag).clone()),
        }))
    }

    fn do_logic(&self) -> Result<i32> {
        let tag = jdk::lock(&self.tag).clone();
        let locale = Self::create_locale(tag.as_deref())?;
        self.base.get_run_data().set_locale(locale.clone());
        let shown = match &locale {
            None => "null".to_string(),
            // `locale.getDisplayName(Locale.ENGLISH)` is empty exactly when
            // the language, script, country and variant are all empty.
            Some(l)
                if l.get_language().is_empty()
                    && l.get_script().is_empty()
                    && l.get_country().is_empty()
                    && l.get_variant().is_empty() =>
            {
                "root locale".to_string()
            }
            Some(l) => l.to_string(),
        };
        println!("Changed Locale to: {shown}");
        Ok(1)
    }

    fn set_params(&self, params: &str) -> Result<()> {
        perf_task::set_params(self, params)?;
        *jdk::lock(&self.tag) = Some(params.to_string());
        Ok(())
    }

    fn supports_params(&self) -> bool {
        true
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "NewLocaleTask"
    }
}
