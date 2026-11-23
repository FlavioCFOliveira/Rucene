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

//! The types declared in `NewAnalyzerTask.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.tasks`.

use std::sync::{Arc, Mutex};

use rucene::analysis::{Analyzer, CharArraySet};
use rucene::error::{LuceneError, Result};
use rucene_analysis_common::analysis::core::stop_analyzer::StopAnalyzer;

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{self, PerfTask, PerfTaskBase};
use crate::classes;
use crate::jdk::stream_tokenizer::{StreamTokenizer, TT_EOF, TT_WORD};
use crate::jdk;

#[derive(Clone, Default)]
struct State {
    analyzer_names: Vec<String>,
    current: usize,
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.tasks.NewAnalyzerTask`
/// (`public class NewAnalyzerTask extends PerfTask`): create a new
/// [`Analyzer`] and set it in the run data for use by all future tasks.
pub struct NewAnalyzerTask {
    base: PerfTaskBase,
    state: Mutex<State>,
}

impl NewAnalyzerTask {
    /// `NewAnalyzerTask(PerfRunData runData)`.
    ///
    /// # Errors
    ///
    /// The exception of the `PerfTask` constructor.
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Self> {
        Ok(NewAnalyzerTask { base: PerfTaskBase::new(run_data, "NewAnalyzerTask")?, state: Mutex::new(State::default()) })
    }

    /// `createAnalyzer(String className)`: the analyzer class's public
    /// no-argument constructor, or `StopAnalyzer(CharArraySet.EMPTY_SET)`.
    ///
    /// # Errors
    ///
    /// `ClassNotFoundException` for an unknown class; `ClassCastException`
    /// for a class that is not an analyzer.
    pub fn create_analyzer(class_name: &str) -> Result<Arc<dyn Analyzer>> {
        if class_name == "org.apache.lucene.analysis.core.StopAnalyzer" {
            return Ok(Arc::new(StopAnalyzer::new(CharArraySet::empty_set())));
        }
        classes::analyzer_for_name(class_name)
    }

    fn create_named(&self, analyzer_name: &mut String) -> Result<Arc<dyn Analyzer>> {
        if analyzer_name.is_empty() {
            *analyzer_name = "org.apache.lucene.analysis.standard.StandardAnalyzer".to_string();
        }
        // First, lookup analyzerName as a named analyzer factory
        let factory = self.base.get_run_data().get_analyzer_factories().get(analyzer_name.as_str()).cloned();
        if let Some(factory) = factory {
            return Ok(factory.create());
        }
        if analyzer_name.contains('.') {
            if analyzer_name.starts_with("standard.") {
                *analyzer_name = format!("org.apache.lucene.analysis.{analyzer_name}");
            }
            Self::create_analyzer(analyzer_name)
        } else {
            // No package
            // Attempt to instantiate a core analyzer
            let core_class_name = format!("org.apache.lucene.analysis.core.{analyzer_name}");
            match Self::create_analyzer(&core_class_name) {
                Ok(analyzer) => {
                    *analyzer_name = core_class_name;
                    Ok(analyzer)
                }
                Err(e) if classes::is_class_not_found(&e) => {
                    // If not a core analyzer, try the base analysis package
                    *analyzer_name = format!("org.apache.lucene.analysis.{analyzer_name}");
                    Self::create_analyzer(analyzer_name)
                }
                Err(e) => Err(e),
            }
        }
    }
}

impl PerfTask for NewAnalyzerTask {
    perf_task::perf_task_plumbing!();

    fn clone_task(&self) -> Result<Arc<dyn PerfTask>> {
        Ok(Arc::new(NewAnalyzerTask { base: self.base.clone_base(), state: Mutex::new(jdk::lock(&self.state).clone()) }))
    }

    fn do_logic(&self) -> Result<i32> {
        let mut analyzer_name: Option<String> = None;
        let result = (|| -> Result<()> {
            let name = {
                let mut s = jdk::lock(&self.state);
                if s.current >= s.analyzer_names.len() {
                    s.current = 0;
                }
                let i = s.current;
                s.current += 1;
                // `analyzerNames.get(current++)` throws IndexOutOfBoundsException
                // on an empty list.
                s.analyzer_names.get(i).cloned().ok_or_else(|| {
                    LuceneError::unchecked("java.lang.IndexOutOfBoundsException", format!("Index {i} out of bounds for length 0"))
                })?
            };
            analyzer_name = Some(name.clone());
            let mut name = name;
            let analyzer = self.create_named(&mut name);
            analyzer_name = Some(name);
            self.base.get_run_data().set_analyzer(Some(analyzer?));
            Ok(())
        })();
        if let Err(e) = result {
            return Err(jdk::runtime_exception_with_cause(
                format!("Error creating Analyzer: {}", analyzer_name.as_deref().unwrap_or("null")),
                e,
            ));
        }
        Ok(1)
    }

    /// Set the params (analyzerName only), Comma-separate list of Analyzer
    /// class names. If the Analyzer lives in org.apache.lucene.analysis, the
    /// name can be shortened by dropping the o.a.l.a part of the Fully
    /// Qualified Class Name.
    ///
    /// Analyzer names may also refer to previously defined
    /// AnalyzerFactory's.
    ///
    /// Example Declaration: `{"NewAnalyzer" NewAnalyzer(WhitespaceAnalyzer,
    /// SimpleAnalyzer, StopAnalyzer, standard.StandardAnalyzer) >`
    ///
    /// Example AnalyzerFactory usage:
    ///
    /// ```text
    /// -AnalyzerFactory(name:'whitespace tokenized',WhitespaceTokenizer)
    /// -NewAnalyzer('whitespace tokenized')
    /// ```
    fn set_params(&self, params: &str) -> Result<()> {
        perf_task::set_params(self, params)?;
        let mut stok = StreamTokenizer::new(params);
        stok.quote_char('"' as i32);
        stok.quote_char('\'' as i32);
        stok.eol_is_significant(false);
        stok.ordinary_char(',' as i32);
        let result = (|| -> Result<()> {
            while stok.next_token() != TT_EOF {
                match stok.ttype {
                    t if t == ',' as i32 => {
                        // Do nothing
                    }
                    t if t == '\'' as i32 || t == '"' as i32 || t == TT_WORD => {
                        jdk::lock(&self.state).analyzer_names.push(stok.sval.clone().unwrap_or_default());
                    }
                    _ => {
                        return Err(jdk::runtime_exception(format!("Unexpected token: {stok}")));
                    }
                }
            }
            Ok(())
        })();
        match result {
            Ok(()) => Ok(()),
            Err(e) if jdk::get_message(&e).is_some_and(|m| m.starts_with("Line #")) => Err(e),
            Err(e) => Err(jdk::runtime_exception_with_cause(
                format!("Line #{}: ", stok.lineno() + self.base.get_alg_line_num()),
                e,
            )),
        }
    }

    fn supports_params(&self) -> bool {
        true
    }

    fn is_instance_of(&self, simple_class_name: &str) -> bool {
        simple_class_name == "NewAnalyzerTask"
    }
}
