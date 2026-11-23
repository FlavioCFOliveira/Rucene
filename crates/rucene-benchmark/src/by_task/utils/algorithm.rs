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

//! The types declared in `Algorithm.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.utils`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use crate::by_task::perf_run_data::PerfRunData;
use crate::by_task::tasks::perf_task::{run_and_maybe_stats, PerfTask};
use crate::by_task::tasks::task_sequence::TaskSequence;
use crate::by_task::utils::config::Config;
use crate::classes;
use crate::jdk::stream_tokenizer::{StreamTokenizer, TT_EOF, TT_NUMBER, TT_WORD};
use crate::jdk::{self, NEW_LINE};

/// `PerfTask.class.getPackage().getName()`.
const DEFAULT_TASK_PACKAGE: &str = "org.apache.lucene.benchmark.byTask.tasks";

/// Equivalent to `org.apache.lucene.benchmark.byTask.utils.Algorithm`
/// (`public class Algorithm implements AutoCloseable`): test algorithm, as
/// read from file.
pub struct Algorithm {
    sequence: Arc<TaskSequence>,
    task_packages: Vec<String>,
}

/// `(TaskSequence) prevTask`.
fn as_sequence(task: &Option<Arc<dyn PerfTask>>) -> Result<&TaskSequence> {
    let task = task.as_ref().ok_or_else(LuceneError::null_pointer)?;
    task.as_any().downcast_ref::<TaskSequence>().ok_or_else(|| {
        LuceneError::unchecked(
            "java.lang.ClassCastException",
            format!(
                "class {} cannot be cast to class org.apache.lucene.benchmark.byTask.tasks.TaskSequence",
                task.get_name()
            ),
        )
    })
}

impl Algorithm {
    /// `Algorithm(PerfRunData runData)`: read algorithm from file. Property
    /// examined: alt.tasks.packages == comma separated list of alternate
    /// package names where tasks would be searched for, when not found in
    /// the default package (that of `PerfTask`). If the same task class
    /// appears in more than one package, the package indicated first in
    /// this list will be used.
    ///
    /// # Errors
    ///
    /// If errors while parsing the algorithm.
    #[allow(unused_assignments)]
    pub fn new(run_data: Arc<PerfRunData>) -> Result<Algorithm> {
        let config = Arc::clone(run_data.get_config());
        let task_packages = Self::init_tasks_packages(&config)?;
        let alg_txt = config.get_algorithm_text().unwrap_or("").to_string();
        let mut sequence = TaskSequence::new(Arc::clone(&run_data), None, None, false)?;
        let mut curr_sequence = Arc::clone(&sequence);
        let mut prev_task: Option<Arc<dyn PerfTask>> = None;
        let mut stok = StreamTokenizer::new(&alg_txt);
        stok.comment_char('#' as i32);
        stok.eol_is_significant(false);
        stok.quote_char('"' as i32);
        stok.quote_char('\'' as i32);
        stok.ordinary_char('/' as i32);
        stok.ordinary_char('(' as i32);
        stok.ordinary_char(')' as i32);
        let mut colon_ok = false;
        let mut is_disable_count_next_task = false; // only for primitive tasks
        curr_sequence.set_depth(0);

        let algorithm = Algorithm { sequence: Arc::clone(&sequence), task_packages };

        while stok.next_token() != TT_EOF {
            match stok.ttype {
                TT_WORD => {
                    let s = stok.sval.clone().unwrap_or_default();
                    let task = algorithm.task_class(&config, &s, &run_data)?;
                    task.base().set_alg_line_num(stok.lineno());
                    task.base().set_disable_counting(is_disable_count_next_task);
                    is_disable_count_next_task = false;
                    curr_sequence.add_task(Arc::clone(&task));
                    if task.is_instance_of("RepSumByPrefTask") {
                        stok.next_token();
                        let prefix = stok.sval.clone();
                        match prefix {
                            Some(prefix) if !prefix.is_empty() => task.set_report_prefix(&prefix),
                            _ => return Err(jdk::exception(format!("named report prefix problem - {stok}"))),
                        }
                    }
                    // check for task param: '(' someParam ')'
                    stok.next_token();
                    if stok.ttype != '(' as i32 {
                        stok.push_back();
                    } else {
                        // get params, for tasks that supports them - allow recursive parenthetical expressions
                        stok.eol_is_significant(true); // Allow params tokenizer to keep track of line number
                        let mut params = String::new();
                        stok.next_token();
                        if stok.ttype != ')' as i32 {
                            let mut count = 1;
                            loop {
                                match stok.ttype {
                                    TT_NUMBER => {
                                        params.push_str(&jdk::double_to_string(stok.nval));
                                    }
                                    TT_WORD => {
                                        params.push_str(stok.sval.as_deref().unwrap_or("null"));
                                    }
                                    TT_EOF => {
                                        return Err(jdk::runtime_exception(format!("Unexpected EOF: - {stok}")));
                                    }
                                    t if t == '"' as i32 || t == '\'' as i32 => {
                                        let q = char::from_u32(t as u32).unwrap_or('"');
                                        params.push(q);
                                        // re-escape delimiters, if any
                                        params.push_str(
                                            &stok.sval.as_deref().unwrap_or("").replace(q, &format!("\\{q}")),
                                        );
                                        params.push(q);
                                    }
                                    t if t == '(' as i32 => {
                                        params.push('(');
                                        count += 1;
                                    }
                                    t if t == ')' as i32 => {
                                        count -= 1;
                                        if count >= 1 {
                                            // exclude final closing parenthesis
                                            params.push(')');
                                        } else {
                                            break;
                                        }
                                    }
                                    t => {
                                        params.push(char::from_u32(t as u32).unwrap_or('\u{FFFD}'));
                                    }
                                }
                                stok.next_token();
                            }
                        }
                        stok.eol_is_significant(false);
                        let prm = jdk::java_trim(&params).to_string();
                        if !prm.is_empty() {
                            task.set_params(&prm)?;
                        }
                    }

                    // ---------------------------------------
                    colon_ok = false;
                    prev_task = Some(task);
                }
                t => {
                    let c = char::from_u32(t as u32).unwrap_or('\u{FFFD}');
                    match c {
                        ':' => {
                            if !colon_ok {
                                return Err(jdk::exception(format!("colon unexpected: - {stok}")));
                            }
                            colon_ok = false;
                            // get repetitions number
                            stok.next_token();
                            if stok.ttype == '*' as i32 {
                                as_sequence(&prev_task)?.set_repetitions(TaskSequence::REPEAT_EXHAUST)?;
                            } else if stok.ttype != TT_NUMBER {
                                return Err(jdk::exception(format!("expected repetitions number or XXXs: - {stok}")));
                            } else {
                                let num = stok.nval;
                                stok.next_token();
                                if stok.ttype == TT_WORD && stok.sval.as_deref() == Some("s") {
                                    as_sequence(&prev_task)?.set_run_time(num)?;
                                } else {
                                    stok.push_back();
                                    as_sequence(&prev_task)?.set_repetitions(num as i32)?;
                                }
                            }
                            // check for rate specification (ops/min)
                            stok.next_token();
                            if stok.ttype != ':' as i32 {
                                stok.push_back();
                            } else {
                                // get rate number
                                stok.next_token();
                                if stok.ttype != TT_NUMBER {
                                    return Err(jdk::exception(format!("expected rate number: - {stok}")));
                                }
                                // check for unit - min or sec, sec is default
                                stok.next_token();
                                if stok.ttype != '/' as i32 {
                                    stok.push_back();
                                    as_sequence(&prev_task)?.set_rate(stok.nval as i32, false); // set rate per sec
                                } else {
                                    stok.next_token();
                                    if stok.ttype != TT_WORD {
                                        return Err(jdk::exception(format!(
                                            "expected rate unit: 'min' or 'sec' - {stok}"
                                        )));
                                    }
                                    let unit = stok.sval.as_deref().unwrap_or("").to_lowercase();
                                    if unit == "min" {
                                        as_sequence(&prev_task)?.set_rate(stok.nval as i32, true); // set rate per min
                                    } else if unit == "sec" {
                                        as_sequence(&prev_task)?.set_rate(stok.nval as i32, false); // set rate per sec
                                    } else {
                                        return Err(jdk::exception(format!(
                                            "expected rate unit: 'min' or 'sec' - {stok}"
                                        )));
                                    }
                                }
                            }
                            colon_ok = false;
                        }
                        '{' | '[' => {
                            // a sequence
                            // check for sequence name
                            let mut name = None;
                            stok.next_token();
                            if stok.ttype != '"' as i32 {
                                stok.push_back();
                            } else {
                                name = stok.sval.clone();
                                if stok.ttype != '"' as i32 || name.as_deref().is_none_or(str::is_empty) {
                                    return Err(jdk::exception(format!("sequence name problem - {stok}")));
                                }
                            }
                            // start the sequence
                            let seq2 =
                                TaskSequence::new(Arc::clone(&run_data), name.as_deref(), Some(&curr_sequence), c == '[')?;
                            curr_sequence.add_task(seq2.as_task());
                            curr_sequence = seq2;
                            colon_ok = false;
                        }
                        '&' => {
                            if curr_sequence.is_parallel() {
                                return Err(jdk::exception("Can only create background tasks within a serial task"));
                            }
                            stok.next_token();
                            let delta_pri = if stok.ttype != TT_NUMBER {
                                stok.push_back();
                                0
                            } else {
                                // priority
                                stok.nval as i32
                            };

                            match &prev_task {
                                None => return Err(jdk::exception("& was unexpected")),
                                Some(prev) if prev.base().get_run_in_background() => {
                                    return Err(jdk::exception("double & was unexpected"));
                                }
                                Some(prev) => prev.base().set_run_in_background(delta_pri),
                            }
                        }
                        '>' | '}' | ']' => {
                            if c == '>' {
                                curr_sequence.set_no_child_report();
                            }
                            // end sequence
                            colon_ok = true;
                            prev_task = Some(curr_sequence.as_task());
                            curr_sequence = match curr_sequence.get_parent() {
                                Some(parent) => parent,
                                None => {
                                    // Java's `currSequence` becomes null; the next use
                                    // throws NullPointerException.
                                    return Err(LuceneError::null_pointer());
                                }
                            };
                        }
                        '-' => {
                            is_disable_count_next_task = true;
                        }
                        _ => {}
                    } // switch(c)
                }
            } // switch(stok.ttype)
        }

        if !Arc::ptr_eq(&sequence, &curr_sequence) {
            return Err(jdk::exception("Unmatched sequences"));
        }

        // remove redundant top level enclosing sequences
        while sequence.is_collapsable() && sequence.get_repetitions() == 1 && sequence.get_rate() == 0 {
            let t = sequence.get_tasks();
            if t.len() == 1 {
                let p = Arc::clone(&t[0]);
                if p.as_any().is::<TaskSequence>() {
                    let seq = classes::downcast_sequence(&p);
                    sequence = seq;
                    continue;
                }
            }
            break;
        }
        Ok(Algorithm { sequence, task_packages: algorithm.task_packages })
    }

    /// `initTasksPackages(Config config)`.
    fn init_tasks_packages(config: &Config) -> Result<Vec<String>> {
        let alts = config.get_str("alt.tasks.packages", None)?;
        let dflt_pkg = DEFAULT_TASK_PACKAGE.to_string();
        let Some(alts) = alts else {
            return Ok(vec![dflt_pkg]);
        };
        let mut pkgs = vec![dflt_pkg];
        pkgs.extend(jdk::split(&alts, ","));
        Ok(pkgs)
    }

    /// `taskClass(Config config, String taskName)` and the reflective
    /// `getConstructor(PerfRunData.class).newInstance(runData)`.
    fn task_class(&self, _config: &Config, task_name: &str, run_data: &Arc<PerfRunData>) -> Result<Arc<dyn PerfTask>> {
        for pkg in &self.task_packages {
            let class_name = format!("{pkg}.{task_name}Task");
            if let Some(task) = classes::task_for_name(&class_name, run_data) {
                return task;
            }
            // failed in this package, might succeed in the next one...
        }
        // can only get here if failed to instantiate
        Err(classes::class_not_found(format!(
            "{task_name} not found in packages [{}]",
            self.task_packages.join(", ")
        )))
    }

    /// `execute()`: execute this algorithm.
    ///
    /// # Errors
    ///
    /// The exception of running or closing the tasks.
    pub fn execute(&self) -> Result<()> {
        let result = run_and_maybe_stats(&self.sequence.as_task(), true);
        let closed = self.sequence.as_task().close();
        result?;
        closed
    }

    /// `extractTasks()`: expert: for test purposes, return all tasks
    /// participating in this algorithm.
    pub fn extract_tasks(&self) -> Vec<Arc<dyn PerfTask>> {
        let mut res = Vec::new();
        self.extract_tasks_from(&mut res, Some(&self.sequence));
        res
    }

    /// `extractTasks(ArrayList<PerfTask> extrct, TaskSequence seq)`. Java
    /// walks `sequence.getTasks()` — the root's tasks — at every level, not
    /// `seq`'s; the port does the same.
    fn extract_tasks_from(&self, extrct: &mut Vec<Arc<dyn PerfTask>>, seq: Option<&Arc<TaskSequence>>) {
        let Some(seq) = seq else { return };
        extrct.push(seq.as_task());
        let t = self.sequence.get_tasks();
        for p in t {
            if p.as_any().is::<TaskSequence>() {
                let sub = classes::downcast_sequence(&p);
                self.extract_tasks_from(extrct, Some(&sub));
            } else {
                extrct.push(p);
            }
        }
    }

    /// `close()`.
    ///
    /// # Errors
    ///
    /// The exception of closing the tasks.
    pub fn close(&self) -> Result<()> {
        self.sequence.as_task().close()
    }
}

impl std::fmt::Display for Algorithm {
    /// `toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{NEW_LINE}", self.sequence.as_task().task_to_string())
    }
}
