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

//! Port of `org.apache.lucene.tests.util.RunListenerPrintReproduceInfo`.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception;
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{Description, Failure, JUnitResult, RunListener};
use randomizedtesting::{LifecycleScope, RandomizedContext};

use crate::util::lucene_test_case::{
    LuceneTestCase, RANDOM_MULTIPLIER, SUPPRESS_REPRODUCE_LINE, TEST_AWAITSFIX, TEST_CODEC,
    TEST_DIRECTORY, TEST_DOCVALUESFORMAT, TEST_LINE_DOCS_FILE, TEST_MONSTER, TEST_NIGHTLY,
    TEST_POSTINGSFORMAT, TEST_WEEKLY, VERBOSE,
};
use crate::util::lucene_test_case_rules::CLASS_ENV_RULE;

/// "A list of all test suite classes executed so far in this JVM (ehm, under
/// this class's classloader)."
static TEST_CLASSES_RUN: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The listener's fields.
struct State {
    /// "The currently executing scope."
    scope: Option<LifecycleScope>,
    /// "Current test failed."
    test_failed: bool,
    /// "Suite-level code (initialization, rule, hook) failed."
    suite_failed: bool,
    /// "A marker to print full env. diagnostics after the suite."
    print_diagnostics_after_class: bool,
    /// "true if we should skip the reproduce string (diagnostics are
    /// independent)"
    suppress_reproduce_line: bool,
}

/// Port of `RunListenerPrintReproduceInfo`: "a suite listener printing a
/// "reproduce string". This ensures test result events are always captured
/// properly even if exceptions happen at initialization or suite/ hooks
/// level."
///
/// **Divergences.**
/// * The reproduce line is the cargo invocation: Java's `gradlew test --tests
///   <target> -Dk=v...` is `env k=v... cargo test --tests <target>`, the same
///   options in the same order and with the same quoting, as the environment
///   variables the port reads its system properties from.
/// * The diagnostics line reports the JVM's heap (`Runtime.freeMemory()`,
///   `totalMemory()`), which a Rust process does not have: both are `0`.
///   `os.name` and `os.arch` default to the Rust target's, `java.vendor`,
///   `java.version` and `os.version` to `null`, and `file.encoding` to
///   `UTF-8` (Rust strings), when the property is unset.
pub struct RunListenerPrintReproduceInfo {
    state: Mutex<State>,
}

impl Default for RunListenerPrintReproduceInfo {
    fn default() -> Self {
        Self::new()
    }
}

impl RunListenerPrintReproduceInfo {
    /// `RunListenerPrintReproduceInfo.class.getName()`.
    pub const CLASS_NAME: &'static str =
        "org.apache.lucene.tests.util.RunListenerPrintReproduceInfo";

    /// `new RunListenerPrintReproduceInfo()`.
    pub fn new() -> RunListenerPrintReproduceInfo {
        RunListenerPrintReproduceInfo {
            state: Mutex::new(State {
                scope: None,
                test_failed: false,
                suite_failed: false,
                print_diagnostics_after_class: false,
                suppress_reproduce_line: false,
            }),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `stripTestNameAugmentations(methodName)`: `replaceAll("\\s*\\{.+?\\}",
    /// "")`, the iteration and seeds randomizedtesting appends to a test's
    /// name.
    pub fn strip_test_name_augmentations(method_name: &str) -> String {
        let chars: Vec<char> = method_name.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            // `\s*` then `\{.+?\}`: try a match at `i`.
            let mut j = i;
            while j < chars.len() && is_java_whitespace(chars[j]) {
                j += 1;
            }
            if j < chars.len() && chars[j] == '{' {
                // `.+?`: at least one character other than a line terminator,
                // up to the first `}` after it.
                let mut k = j + 2;
                let mut matched = None;
                if j + 1 < chars.len() && !is_line_terminator(chars[j + 1]) {
                    while k < chars.len() {
                        if chars[k] == '}' {
                            matched = Some(k);
                            break;
                        }
                        if is_line_terminator(chars[k]) {
                            break;
                        }
                        k += 1;
                    }
                }
                if let Some(end) = matched {
                    i = end + 1;
                    continue;
                }
            }
            out.push(chars[i]);
            i += 1;
        }
        out
    }

    /// `printDebuggingInformation()`: "print some useful debugging
    /// information about the environment."
    fn print_debugging_information() {
        {
            let env = CLASS_ENV_RULE.env();
            if env.initialized {
                let codec = env
                    .codec
                    .as_ref()
                    .map_or_else(|| "null".to_string(), |c| c.to_java_string());
                let sim = env
                    .similarity
                    .as_ref()
                    .map_or_else(|| "null".to_string(), |s| s.to_string());
                let locale = env
                    .locale
                    .as_ref()
                    .map_or_else(|| "null".to_string(), |l| l.to_language_tag());
                let timezone = env
                    .time_zone
                    .map_or_else(|| "(null)".to_string(), |t| t.get_id().to_string());
                system::err_println(&format!(
                    "NOTE: test params are: codec={codec}, sim={sim}, locale={locale}, \
                     timezone={timezone}"
                ));
            }
        }
        let prop = |key: &str, default: &str| {
            system::get_property(key).unwrap_or_else(|| default.to_string())
        };
        let cpus = std::thread::available_parallelism().map_or(1, |n| n.get());
        let threads = randomizedtesting::jdk::thread::ThreadGroup::main()
            .enumerate()
            .len();
        system::err_println(&format!(
            "NOTE: {} {} {}/{} {} {}/cpus={cpus},threads={threads},free=0,total=0",
            prop("os.name", std::env::consts::OS),
            prop("os.version", "null"),
            prop("os.arch", std::env::consts::ARCH),
            prop("java.vendor", "null"),
            prop("java.version", "null"),
            if cfg!(target_pointer_width = "64") {
                "(64-bit)"
            } else {
                "(32-bit)"
            },
        ));
        let run = TEST_CLASSES_RUN
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .join(", ");
        system::err_println(&format!("NOTE: All tests run in this JVM: [{run}]"));
    }

    /// The reproduce line `reportAdditionalFailureInfo(testName)` prints, for
    /// the suite simple name `test_class`.
    pub fn reproduce_line(test_class: &str, test_name: Option<&str>, runner_seed: &str) -> String {
        let mut opts = String::new();
        // Pass the master seed.
        add_vm_opt(&mut opts, "tests.seed", Some(runner_seed));
        // Test groups and multipliers.
        if *RANDOM_MULTIPLIER != LuceneTestCase::default_random_multiplier() {
            add_vm_opt(
                &mut opts,
                "tests.multiplier",
                Some(&RANDOM_MULTIPLIER.to_string()),
            );
        }
        if *TEST_NIGHTLY {
            add_vm_opt(&mut opts, LuceneTestCase::SYSPROP_NIGHTLY, Some("true"));
        }
        if *TEST_WEEKLY {
            add_vm_opt(&mut opts, LuceneTestCase::SYSPROP_WEEKLY, Some("true"));
        }
        if *TEST_MONSTER {
            add_vm_opt(&mut opts, LuceneTestCase::SYSPROP_MONSTER, Some("true"));
        }
        if *TEST_AWAITSFIX {
            add_vm_opt(&mut opts, LuceneTestCase::SYSPROP_AWAITSFIX, Some("true"));
        }
        // Codec, postings, directories.
        if TEST_CODEC.as_str() != "random" {
            add_vm_opt(&mut opts, "tests.codec", Some(&TEST_CODEC));
        }
        if TEST_POSTINGSFORMAT.as_str() != "random" {
            add_vm_opt(
                &mut opts,
                "tests.postingsformat",
                Some(&TEST_POSTINGSFORMAT),
            );
        }
        if TEST_DOCVALUESFORMAT.as_str() != "random" {
            add_vm_opt(
                &mut opts,
                "tests.docvaluesformat",
                Some(&TEST_DOCVALUESFORMAT),
            );
        }
        if TEST_DIRECTORY.as_str() != "random" {
            add_vm_opt(&mut opts, "tests.directory", Some(&TEST_DIRECTORY));
        }
        // Environment.
        if TEST_LINE_DOCS_FILE.as_str() != LuceneTestCase::DEFAULT_LINE_DOCS_FILE {
            add_vm_opt(&mut opts, "tests.linedocsfile", Some(&TEST_LINE_DOCS_FILE));
        }
        {
            let env = CLASS_ENV_RULE.env();
            if env.initialized {
                let locale = env.locale.as_ref().map(|l| l.to_language_tag());
                add_vm_opt(&mut opts, "tests.locale", locale.as_deref());
                if let Some(tz) = env.time_zone {
                    add_vm_opt(&mut opts, "tests.timezone", Some(tz.get_id()));
                }
            }
        }
        if LuceneTestCase::ASSERTS_ARE_ENABLED {
            add_vm_opt(&mut opts, "tests.asserts", Some("true"));
        } else {
            add_vm_opt(&mut opts, "tests.asserts", Some("false"));
        }
        let encoding = system::get_property("file.encoding").unwrap_or_else(|| "UTF-8".to_string());
        add_vm_opt(&mut opts, "tests.file.encoding", Some(&encoding));

        let mut b = String::from("NOTE: reproduce with: env");
        b.push_str(&opts);
        b.push_str(" cargo test ");
        // Figure out the test case name and method, if any.
        b.push_str("--tests ");
        b.push_str(test_class);
        if let Some(name) = test_name {
            b.push('.');
            b.push_str(name);
        }
        b
    }

    /// `reportAdditionalFailureInfo(testName)`.
    fn report_additional_failure_info(&self, test_name: Option<&str>) {
        if self.state().suppress_reproduce_line {
            return;
        }
        if TEST_LINE_DOCS_FILE.ends_with(LuceneTestCase::JENKINS_LARGE_LINE_DOCS_FILE) {
            system::err_println(&format!(
                "NOTE: large line-docs file was used in this run. You have to download it \
                 manually ('gradlew getEnWikiRandomLines') and use -P{}=... property to point \
                 to it.",
                TEST_LINE_DOCS_FILE.as_str()
            ));
        }
        let context = RandomizedContext::current();
        let test_class = match context.get_target_class_object() {
            Some(c) => c.simple_name().to_string(),
            None => exception::throw_without_message(exception::NULL_POINTER_EXCEPTION),
        };
        system::err_println(&Self::reproduce_line(
            &test_class,
            test_name,
            &context.get_runner_seed_as_string(),
        ));
    }
}

/// `addVmOpt(b, key, value)`: "append a VM option (-Dkey=value) to a
/// StringBuilder. Add quotes if spaces or other funky characters are
/// detected." (as ` key=value`, see the type's divergences).
pub fn add_vm_opt(b: &mut String, key: &str, value: Option<&str>) {
    let Some(v) = value else { return };
    b.push(' ');
    b.push_str(key);
    b.push('=');
    // Add simplistic quoting. This varies a lot from system to system and between
    // shells... ANT should have some code for doing it properly.
    if v.chars()
        .any(|c| is_regex_space(c) || c == '=' || c == '\'')
    {
        b.push('"');
        b.push_str(v);
        b.push('"');
    } else {
        b.push_str(v);
    }
}

/// The regex class `\s`: `[ \t\n\x0B\f\r]`.
fn is_regex_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r')
}

/// `\s` as `stripTestNameAugmentations`' pattern uses it.
fn is_java_whitespace(c: char) -> bool {
    is_regex_space(c)
}

/// The line terminators Java's `.` does not match.
fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

impl RunListener for RunListenerPrintReproduceInfo {
    /// `testRunStarted(description)`.
    fn test_run_started(&self, _description: &Arc<Description>) {
        let target = RandomizedContext::current().get_target_class_object();
        let mut s = self.state();
        s.suite_failed = false;
        s.test_failed = false;
        s.scope = Some(LifecycleScope::Suite);
        let Some(target_class) = target else {
            exception::throw_without_message(exception::NULL_POINTER_EXCEPTION)
        };
        s.suppress_reproduce_line = target_class.is_annotation_present(&SUPPRESS_REPRODUCE_LINE);
        TEST_CLASSES_RUN
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(target_class.simple_name().to_string());
    }

    /// `testStarted(description)`.
    fn test_started(&self, _description: &Arc<Description>) {
        let mut s = self.state();
        s.test_failed = false;
        s.scope = Some(LifecycleScope::Test);
    }

    /// `testFailure(failure)`.
    fn test_failure(&self, _failure: &Failure) {
        let mut s = self.state();
        if s.scope == Some(LifecycleScope::Test) {
            s.test_failed = true;
        } else {
            s.suite_failed = true;
        }
        s.print_diagnostics_after_class = true;
    }

    /// `testFinished(description)`.
    fn test_finished(&self, description: &Arc<Description>) {
        let test_failed = self.state().test_failed;
        if test_failed {
            let name = description
                .method_name()
                .map(|m| Self::strip_test_name_augmentations(&m));
            self.report_additional_failure_info(name.as_deref());
        }
        let mut s = self.state();
        s.scope = Some(LifecycleScope::Suite);
        s.test_failed = false;
    }

    /// `testRunFinished(result)`.
    fn test_run_finished(&self, _result: &JUnitResult) {
        let (print, suite_failed) = {
            let s = self.state();
            (s.print_diagnostics_after_class, s.suite_failed)
        };
        if print || *VERBOSE {
            Self::print_debugging_information();
        }
        if suite_failed {
            self.report_additional_failure_info(None);
        }
    }
}
