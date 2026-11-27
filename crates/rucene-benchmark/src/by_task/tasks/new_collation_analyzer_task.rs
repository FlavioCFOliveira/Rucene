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

//! The types declared in `NewCollationAnalyzerTask.java`, of the Java
//! package `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::{Arc, Mutex};

use rucene::analysis::Analyzer;
use rucene::error::Result;
use rucene_analysis_common::collation::collation_key_analyzer::CollationKeyAnalyzer;
use rucene_analysis_common::jdk::collator::RuleBasedCollator;
use rucene_analysis_common::jdk::locale::Locale;
use rucene_analysis_icu::analysis::icu::icu_collation_key_analyzer::ICUCollationKeyAnalyzer;

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.NewCollationAnalyzerTask.Implementation`
/// (`public enum Implementation`): different Collation implementations:
/// currently limited to what is provided in the JDK and ICU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Implementation {
    /// `JDK("org.apache.lucene.collation.CollationKeyAnalyzer", "java.text.Collator")`.
    Jdk,
    /// `ICU("org.apache.lucene.analysis.icu.ICUCollationKeyAnalyzer", "com.ibm.icu.text.Collator")`.
    Icu,
}

impl Implementation {
    /// `className`.
    pub fn class_name(self) -> &'static str {
        match self {
            Implementation::Jdk => "org.apache.lucene.collation.CollationKeyAnalyzer",
            Implementation::Icu => "org.apache.lucene.analysis.icu.ICUCollationKeyAnalyzer",
        }
    }

    /// `collatorClassName`.
    pub fn collator_class_name(self) -> &'static str {
        match self {
            Implementation::Jdk => "java.text.Collator",
            Implementation::Icu => "com.ibm.icu.text.Collator",
        }
    }

    /// `name()`, as `toString()` prints it.
    pub fn name(self) -> &'static str {
        match self {
            Implementation::Jdk => "JDK",
            Implementation::Icu => "ICU",
        }
    }
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.NewCollationAnalyzerTask`
/// (`public class NewCollationAnalyzerTask extends PerfTask`): task to support
/// benchmarking collation.
///
/// * `NewCollationAnalyzer` with the default jdk impl
/// * `NewCollationAnalyzer(impl:icu)` specify an impl (jdk,icu)
pub struct NewCollationAnalyzerTask {
    base: PerfTaskBase,
    impl_: Mutex<Implementation>,
}

impl NewCollationAnalyzerTask {
    /// `NewCollationAnalyzerTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(NewCollationAnalyzerTask {
            base: PerfTaskBase::new(run_data, "NewCollationAnalyzerTask")?,
            impl_: Mutex::new(Implementation::Jdk),
        })
    }

    /// `createAnalyzer(Locale locale, Implementation impl)`: the analyzer
    /// class's `(Collator)` constructor over `Collator.getInstance(locale)`,
    /// both found by name: `CollationKeyAnalyzer` over
    /// `java.text.Collator.getInstance(locale)` for [`Implementation::Jdk`],
    /// `ICUCollationKeyAnalyzer` over `com.ibm.icu.text.Collator.getInstance(locale)`
    /// for [`Implementation::Icu`] (the reflective lookups resolve to these two
    /// pairs, the only ones the enum names).
    ///
    /// The `icu4j` port takes its `java.util.Locale` from `rucene-queryparser`'s
    /// emulation; the locale crosses over as its BCP 47 language tag, the form
    /// `NewLocaleTask` builds it from (`new Locale.Builder().setLanguageTag(tag)`),
    /// so the same `Locale.Builder` rebuilds it.
    ///
    /// # Errors
    ///
    /// The errors of loading the ICU collation data.
    pub(crate) fn create_analyzer(
        locale: &Locale,
        impl_: Implementation,
    ) -> Result<Arc<dyn Analyzer>> {
        match impl_ {
            Implementation::Jdk => {
                let collator = RuleBasedCollator::get_instance(locale);
                Ok(Arc::new(CollationKeyAnalyzer::new(&collator)))
            }
            Implementation::Icu => {
                let locale = rucene_queryparser::jdk::locale::Locale::builder_from_language_tag(
                    &locale.to_language_tag(),
                )?;
                let collator = icu4j::text::collator::get_instance_locale(Some(&locale))?;
                Ok(Arc::new(ICUCollationKeyAnalyzer::new(Arc::from(collator))))
            }
        }
    }
}

impl PerfTask for NewCollationAnalyzerTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(NewCollationAnalyzerTask {
            base: self.base.clone_base(),
            impl_: Mutex::new(*jdk::lock(&self.impl_)),
        }))
    }

    fn do_logic(&self) -> Result<i32> {
        let impl_ = *jdk::lock(&self.impl_);
        let result = (|| -> Result<()> {
            let Some(locale) = self.base.get_run_data().get_locale() else {
                return Err(jdk::runtime_exception(
                    "Locale must be set with the NewLocale task!",
                ));
            };
            let analyzer = Self::create_analyzer(&locale, impl_)?;
            self.base.get_run_data().set_analyzer(Some(analyzer));
            println!("Changed Analyzer to: {}({locale})", impl_.class_name());
            Ok(())
        })();
        result.map_err(|e| {
            jdk::runtime_exception_with_cause(
                format!("Error creating Analyzer: impl={}", impl_.name()),
                e,
            )
        })?;
        Ok(1)
    }

    fn set_params(&self, params: &str) -> Result<()> {
        perf_task::set_params(self, params)?;
        for param in params.split(',').filter(|s| !s.is_empty()) {
            let mut expr = param.split(':').filter(|s| !s.is_empty());
            let no_such = || {
                rucene::error::LuceneError::unchecked_with_message(
                    "java.util.NoSuchElementException",
                    None,
                )
            };
            let key = expr.next().ok_or_else(no_such)?;
            let value = expr.next().ok_or_else(no_such)?;
            // for now we only support the "impl" parameter.
            // TODO: add strength, decomposition, etc
            if key == "impl" {
                if value.eq_ignore_ascii_case("icu") {
                    *jdk::lock(&self.impl_) = Implementation::Icu;
                } else if value.eq_ignore_ascii_case("jdk") {
                    *jdk::lock(&self.impl_) = Implementation::Jdk;
                } else {
                    return Err(jdk::runtime_exception(format!("Unknown parameter {param}")));
                }
            } else {
                return Err(jdk::runtime_exception(format!("Unknown parameter {param}")));
            }
        }
        Ok(())
    }

    fn supports_params(&self) -> bool {
        true
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "NewCollationAnalyzerTask"
    }
}
