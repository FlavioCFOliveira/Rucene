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

//! The types declared in `Config.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.utils`.

use std::sync::{Mutex, MutexGuard, OnceLock};

use rucene::error::{LuceneError, Result};
use rucene::util::java_collections::JavaHashMap;

use crate::by_task::utils::format::Format;
use crate::jdk::properties::Properties;
use crate::jdk::{self, NEW_LINE};

/// The arrays `valByRound` holds, by their Java type.
#[derive(Clone, Debug)]
enum ValByRound {
    /// `String[]`.
    Str(Vec<String>),
    /// `int[]`.
    Int(Vec<i32>),
    /// `double[]`.
    Double(Vec<f64>),
    /// `boolean[]`.
    Bool(Vec<bool>),
}

impl ValByRound {
    /// The JVM's binary name of the array class.
    fn class_name(&self) -> &'static str {
        match self {
            ValByRound::Str(_) => "[Ljava.lang.String;",
            ValByRound::Int(_) => "[I",
            ValByRound::Double(_) => "[D",
            ValByRound::Bool(_) => "[Z",
        }
    }

    /// The `ClassCastException` of `(target) valByRound.get(name)`.
    fn class_cast(&self, target: &'static str) -> LuceneError {
        let source = self.class_name();
        LuceneError::unchecked(
            "java.lang.ClassCastException",
            format!(
                "class {source} cannot be cast to class {target} ({source} and {target} are in module java.base of loader 'bootstrap')"
            ),
        )
    }
}

/// The mutable state of a [`Config`].
#[derive(Debug)]
struct ConfigState {
    round_number: i32,
    props: Properties,
    val_by_round: JavaHashMap<ValByRound>,
    col_for_val_by_round: JavaHashMap<String>,
    num_threads: i32,
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.utils.Config`: perf run
/// configuration properties.
///
/// Numeric property containing ":", e.g. "10:100:5" is interpreted as array
/// of numeric values. It is extracted once, on first use, and maintain a
/// round number to return the appropriate value.
///
/// The config property "work.dir" tells where is the root of docs data dirs
/// and indexes dirs. It is set to either of: value supplied for it in the
/// alg file; otherwise, value of System property "benchmark.work.dir";
/// otherwise, "work".
///
/// A `Config` is shared by every task and source of a run (as Java shares
/// the object), so its state sits behind a lock and every method takes
/// `&self`.
#[derive(Debug)]
pub struct Config {
    state: Mutex<ConfigState>,
    algorithm_text: Option<String>,
}

/// `DEFAULT_PRINT_PROPS`: `System.getProperty("tests.verbose", "true")`.
fn default_print_props() -> &'static str {
    static V: OnceLock<String> = OnceLock::new();
    V.get_or_init(|| {
        rucene::util::system_property("tests.verbose").unwrap_or_else(|| "true".to_string())
    })
}

/// `BufferedReader.readLine()` over a whole text: lines end at `\n`, `\r`
/// or `\r\n`.
pub(crate) fn read_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    let mut pending = false;
    while let Some(c) = chars.next() {
        pending = true;
        match c {
            '\n' => {
                lines.push(std::mem::take(&mut cur));
                pending = false;
            }
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                lines.push(std::mem::take(&mut cur));
                pending = false;
            }
            c => cur.push(c),
        }
    }
    if pending {
        lines.push(cur);
    }
    lines
}

/// `StringTokenizer(s, ":")` tokens: the non-empty runs between colons.
fn colon_tokens(s: &str) -> impl Iterator<Item = &str> {
    s.split(':').filter(|t| !t.is_empty())
}

impl Config {
    /// `Config(Reader algReader)`: read both algorithm and config
    /// properties.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of a malformed `\uxxxx` escape in the
    /// properties.
    pub fn from_reader(alg_reader: &str) -> Result<Config> {
        // read alg file to array of lines
        let lines = read_lines(alg_reader);
        let mut last_config_line = 0;
        for (i, line) in lines.iter().enumerate() {
            if line.find('=').is_some_and(|p| p > 0) {
                last_config_line = i + 1;
            }
        }
        // copy props lines to string
        let mut sb = String::new();
        for line in &lines[..last_config_line] {
            sb.push_str(line);
            sb.push_str(NEW_LINE);
        }
        // read props from string
        let mut props = Properties::new();
        props.load(&sb)?;
        // make sure work dir is set properly
        if !props.contains_key("work.dir") {
            props.set_property(
                "work.dir",
                rucene::util::system_property("benchmark.work.dir")
                    .unwrap_or_else(|| "work".to_string()),
            );
        }
        // copy algorithm lines
        let mut alg = String::new();
        for line in &lines[last_config_line..] {
            alg.push_str(line);
            alg.push_str(NEW_LINE);
        }
        let config = Config::new_state(props, Some(alg));
        config.maybe_print_props();
        Ok(config)
    }

    /// `Config(Properties props)`: create config without algorithm - useful
    /// for a programmatic perf test.
    pub fn from_properties(props: Properties) -> Config {
        let config = Config::new_state(props, None);
        config.maybe_print_props();
        config
    }

    fn new_state(props: Properties, algorithm_text: Option<String>) -> Config {
        Config {
            state: Mutex::new(ConfigState {
                round_number: 0,
                props,
                val_by_round: JavaHashMap::new(),
                col_for_val_by_round: JavaHashMap::new(),
                num_threads: 1,
            }),
            algorithm_text,
        }
    }

    fn lock(&self) -> MutexGuard<'_, ConfigState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn maybe_print_props(&self) {
        let print = {
            let state = self.lock();
            jdk::parse_boolean(
                state
                    .props
                    .get_property("print.props")
                    .or(Some(default_print_props())),
            )
        };
        if print {
            self.print_props();
        }
    }

    /// `setNumThreads(int)`.
    pub fn set_num_threads(&self, num_threads: i32) {
        self.lock().num_threads = num_threads;
    }

    /// `getNumThreads()`.
    pub fn get_num_threads(&self) -> i32 {
        self.lock().num_threads
    }

    /// `printProps()`.
    fn print_props(&self) {
        let state = self.lock();
        println!("------------> config properties:");
        let mut prop_keys: Vec<&str> = state.props.keys().collect();
        prop_keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        for prop_name in prop_keys {
            println!(
                "{} = {}",
                prop_name,
                state.props.get_property(prop_name).unwrap_or("null")
            );
        }
        println!("-------------------------------");
    }

    /// The by-round values of `name`, extracted on first use: `None` when
    /// the property is not by round. `parse` turns the text after the
    /// column name into the array.
    fn by_round(
        state: &mut ConfigState,
        name: &str,
        sval: &str,
        parse: impl FnOnce(&str) -> Result<ValByRound>,
    ) -> Result<()> {
        // first time this prop is extracted by round
        let k = sval.find(':').expect("checked by the caller");
        let col_name = &sval[..k];
        let rest = &sval[k + 1..];
        state
            .col_for_val_by_round
            .insert(name.to_string(), col_name.to_string());
        let vals = parse(rest)?;
        state.val_by_round.insert(name.to_string(), vals);
        Ok(())
    }

    /// `get(String name, String dflt)`: return a string property.
    ///
    /// # Errors
    ///
    /// `ClassCastException` when `name` was first read by round as another
    /// type.
    pub fn get_str(&self, name: &str, dflt: Option<&str>) -> Result<Option<String>> {
        let mut state = self.lock();
        let round = state.round_number;
        if let Some(vals) = state.val_by_round.get(name) {
            return match vals {
                ValByRound::Str(vals) => Ok(Some(vals[round as usize % vals.len()].clone())),
                other => Err(other.class_cast("[Ljava.lang.String;")),
            };
        }
        // done if not by round
        let Some(sval) = state.props.get_property(name).or(dflt).map(str::to_string) else {
            return Ok(None);
        };
        if !sval.contains(':') {
            return Ok(Some(sval));
        } else if sval.contains(":\\") || sval.contains(":/") {
            // this previously messed up absolute path names on Windows. Assuming
            // there is no real value that starts with \ or /
            return Ok(Some(sval));
        }
        Self::by_round(&mut state, name, &sval, |s| {
            Ok(ValByRound::Str(prop_to_string_array(s)))
        })?;
        let Some(ValByRound::Str(vals)) = state.val_by_round.get(name) else {
            unreachable!()
        };
        Ok(Some(vals[round as usize % vals.len()].clone()))
    }

    /// `get(String name, int dflt)`: return an int property. If the property
    /// contain ":", e.g. "10:100:5", it is interpreted as array of ints. It
    /// is extracted once, on first call to get() it, and a by-round-value is
    /// returned.
    ///
    /// # Errors
    ///
    /// `NumberFormatException` for a value that is not an int;
    /// `ClassCastException` when `name` was first read by round as another
    /// type.
    pub fn get_int(&self, name: &str, dflt: i32) -> Result<i32> {
        let mut state = self.lock();
        let round = state.round_number;
        if let Some(vals) = state.val_by_round.get(name) {
            return match vals {
                ValByRound::Int(vals) => Ok(vals[round as usize % vals.len()]),
                other => Err(other.class_cast("[I")),
            };
        }
        let sval = state
            .props
            .get_property(name)
            .map(str::to_string)
            .unwrap_or_else(|| dflt.to_string());
        if !sval.contains(':') {
            return jdk::parse_int(&sval);
        }
        Self::by_round(&mut state, name, &sval, |s| {
            Ok(ValByRound::Int(prop_to_int_array(s)?))
        })?;
        let Some(ValByRound::Int(vals)) = state.val_by_round.get(name) else {
            unreachable!()
        };
        Ok(vals[round as usize % vals.len()])
    }

    /// `get(String name, double dflt)`: return a double property, by round
    /// as [`get_int`](Self::get_int).
    ///
    /// # Errors
    ///
    /// As [`get_int`](Self::get_int).
    pub fn get_double(&self, name: &str, dflt: f64) -> Result<f64> {
        let mut state = self.lock();
        let round = state.round_number;
        if let Some(vals) = state.val_by_round.get(name) {
            return match vals {
                ValByRound::Double(vals) => Ok(vals[round as usize % vals.len()]),
                other => Err(other.class_cast("[D")),
            };
        }
        let sval = state
            .props
            .get_property(name)
            .map(str::to_string)
            .unwrap_or_else(|| jdk::double_to_string(dflt));
        if !sval.contains(':') {
            return jdk::parse_double(&sval);
        }
        Self::by_round(&mut state, name, &sval, |s| {
            Ok(ValByRound::Double(prop_to_double_array(s)?))
        })?;
        let Some(ValByRound::Double(vals)) = state.val_by_round.get(name) else {
            unreachable!()
        };
        Ok(vals[round as usize % vals.len()])
    }

    /// `get(String name, boolean dflt)`: return a boolean property, by
    /// round as [`get_int`](Self::get_int).
    ///
    /// # Errors
    ///
    /// `ClassCastException` when `name` was first read by round as another
    /// type.
    pub fn get_bool(&self, name: &str, dflt: bool) -> Result<bool> {
        let mut state = self.lock();
        let round = state.round_number;
        if let Some(vals) = state.val_by_round.get(name) {
            return match vals {
                ValByRound::Bool(vals) => Ok(vals[round as usize % vals.len()]),
                other => Err(other.class_cast("[Z")),
            };
        }
        let sval = state
            .props
            .get_property(name)
            .map(str::to_string)
            .unwrap_or_else(|| dflt.to_string());
        if !sval.contains(':') {
            return Ok(jdk::parse_boolean(Some(&sval)));
        }
        Self::by_round(&mut state, name, &sval, |s| {
            Ok(ValByRound::Bool(prop_to_boolean_array(s)))
        })?;
        let Some(ValByRound::Bool(vals)) = state.val_by_round.get(name) else {
            unreachable!()
        };
        Ok(vals[round as usize % vals.len()])
    }

    /// `set(String name, String value)`: set a property. Note: once a
    /// multiple values property is set, it can no longer be modified.
    ///
    /// # Errors
    ///
    /// `Exception("Cannot modify a multi value property!")`.
    pub fn set(&self, name: &str, value: &str) -> Result<()> {
        let mut state = self.lock();
        if state.val_by_round.get(name).is_some() {
            return Err(jdk::exception("Cannot modify a multi value property!"));
        }
        state.props.set_property(name, value);
        Ok(())
    }

    /// `newRound()`: increment the round number, for config values that are
    /// extracted by round number. Returns the new round number.
    pub fn new_round(&self) -> i32 {
        let mut state = self.lock();
        state.round_number += 1;
        let round_number = state.round_number;
        let mut sb = format!("--> Round {}-->{}", round_number - 1, round_number);
        // log changes in values
        if !state.val_by_round.is_empty() {
            sb.push_str(": ");
            for (name, a) in state.val_by_round.iter() {
                let (v1, v2) = match a {
                    ValByRound::Int(ai) => {
                        let n1 = (round_number - 1) as usize % ai.len();
                        let n2 = round_number as usize % ai.len();
                        (ai[n1].to_string(), ai[n2].to_string())
                    }
                    ValByRound::Double(ad) => {
                        let n1 = (round_number - 1) as usize % ad.len();
                        let n2 = round_number as usize % ad.len();
                        (jdk::double_to_string(ad[n1]), jdk::double_to_string(ad[n2]))
                    }
                    ValByRound::Str(ad) => {
                        let n1 = (round_number - 1) as usize % ad.len();
                        let n2 = round_number as usize % ad.len();
                        (ad[n1].clone(), ad[n2].clone())
                    }
                    ValByRound::Bool(ab) => {
                        let n1 = (round_number - 1) as usize % ab.len();
                        let n2 = round_number as usize % ab.len();
                        (ab[n1].to_string(), ab[n2].to_string())
                    }
                };
                sb.push_str(&format!("  {name}:{v1}-->{v2}"));
            }
        }
        println!();
        println!("{sb}");
        println!();
        round_number
    }

    /// `getColsNamesForValsByRound()`: names of params set by round, for
    /// reports title.
    pub fn get_cols_names_for_vals_by_round(&self) -> String {
        let state = self.lock();
        if state.col_for_val_by_round.is_empty() {
            return String::new();
        }
        let mut sb = String::new();
        for col_name in state.col_for_val_by_round.values() {
            sb.push(' ');
            sb.push_str(col_name);
        }
        sb
    }

    /// `getColsValuesForValsByRound(int roundNum)`: values of params set by
    /// round, for reports lines.
    pub fn get_cols_values_for_vals_by_round(&self, round_num: i32) -> String {
        let state = self.lock();
        if state.col_for_val_by_round.is_empty() {
            return String::new();
        }
        let mut sb = String::new();
        for (val_by_round_name, col_name) in state.col_for_val_by_round.iter() {
            let template = format!(" {col_name}");
            if round_num < 0 {
                // just append blanks
                sb.push_str(&Format::format_padd_left("-", &template));
            } else {
                // append actual values, for that round
                match state.val_by_round.get(val_by_round_name) {
                    Some(ValByRound::Int(ai)) => {
                        let n = round_num as usize % ai.len();
                        sb.push_str(&Format::format_int(ai[n], &template));
                    }
                    Some(ValByRound::Double(ad)) => {
                        let n = round_num as usize % ad.len();
                        sb.push_str(&Format::format_double(2, ad[n], &template));
                    }
                    Some(ValByRound::Str(ad)) => {
                        let n = round_num as usize % ad.len();
                        sb.push_str(&ad[n]);
                    }
                    Some(ValByRound::Bool(ab)) => {
                        let n = round_num as usize % ab.len();
                        sb.push_str(&Format::format_padd_left(&ab[n].to_string(), &template));
                    }
                    None => unreachable!("every by-round column has its values"),
                }
            }
        }
        sb
    }

    /// `getRoundNumber()`.
    pub fn get_round_number(&self) -> i32 {
        self.lock().round_number
    }

    /// `getAlgorithmText()`.
    pub fn get_algorithm_text(&self) -> Option<&str> {
        self.algorithm_text.as_deref()
    }
}

/// `propToStringArray(String s)`.
fn prop_to_string_array(s: &str) -> Vec<String> {
    if !s.contains(':') {
        return vec![s.to_string()];
    }
    colon_tokens(s).map(str::to_string).collect()
}

/// `propToIntArray(String s)`: extract properties to array, e.g. for
/// "10:100:5" return int[]{10,100,5}.
fn prop_to_int_array(s: &str) -> Result<Vec<i32>> {
    if !s.contains(':') {
        return Ok(vec![jdk::parse_int(s)?]);
    }
    colon_tokens(s).map(jdk::parse_int).collect()
}

/// `propToDoubleArray(String s)`.
fn prop_to_double_array(s: &str) -> Result<Vec<f64>> {
    if !s.contains(':') {
        return Ok(vec![jdk::parse_double(s)?]);
    }
    colon_tokens(s).map(jdk::parse_double).collect()
}

/// `propToBooleanArray(String s)`.
fn prop_to_boolean_array(s: &str) -> Vec<bool> {
    if !s.contains(':') {
        return vec![jdk::parse_boolean(Some(s))];
    }
    colon_tokens(s)
        .map(|t| jdk::parse_boolean(Some(t)))
        .collect()
}
