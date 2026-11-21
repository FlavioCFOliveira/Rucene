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

//! Port of `org.apache.lucene.tests.util.VerifyTestClassNamingConvention`.

use std::sync::Arc;

use randomizedtesting::jdk::exception::{self, Throwable};
use randomizedtesting::junit::Description;
use randomizedtesting::RandomizedContext;

use crate::util::abstract_before_after_rule::AbstractBeforeAfterRule;
use crate::util::test_rule_ignore_test_suites::TestRuleIgnoreTestSuites;

/// Port of `VerifyTestClassNamingConvention`: "enforce test naming
/// convention."
///
/// `LuceneTestCase` installs it with the package prefix `org.apache.lucene`
/// and the pattern `(.+\.)(Test)([^.]+)`; Java's `Pattern` is the matcher
/// [`lucene_convention_matches`], and `toString()` its source.
pub struct VerifyTestClassNamingConvention {
    package_prefix: String,
    naming_convention: String,
    matches: fn(&str) -> bool,
}

/// `Pattern.compile("(.+\\.)(Test)([^.]+)").matcher(name).matches()`: a
/// non-empty prefix ending in `.`, then `Test`, then a non-empty simple name
/// without `.`. Java's `.` matches any character but a line terminator.
pub fn lucene_convention_matches(name: &str) -> bool {
    let units: Vec<char> = name.chars().collect();
    // `(.+\.)` is greedy but must leave `Test[^.]+`: the last `.` decides,
    // as `[^.]+` cannot contain one.
    let Some(last_dot) = units.iter().rposition(|&c| c == '.') else {
        return false;
    };
    let line_terminator = |c: char| matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}');
    let prefix = &units[..last_dot];
    let rest = &units[last_dot + 1..];
    !prefix.is_empty()
        && !prefix.iter().any(|&c| line_terminator(c))
        && rest.len() > 4
        && rest[..4] == ['T', 'e', 's', 't']
}

impl VerifyTestClassNamingConvention {
    /// `new VerifyTestClassNamingConvention(packagePrefix, namingConvention)`.
    pub fn new(
        package_prefix: &str,
        naming_convention: &str,
        matches: fn(&str) -> bool,
    ) -> VerifyTestClassNamingConvention {
        VerifyTestClassNamingConvention {
            package_prefix: package_prefix.to_string(),
            naming_convention: naming_convention.to_string(),
            matches,
        }
    }

    /// The rule `LuceneTestCase` installs.
    pub fn lucene() -> VerifyTestClassNamingConvention {
        Self::new(
            "org.apache.lucene",
            "(.+\\.)(Test)([^.]+)",
            lucene_convention_matches,
        )
    }
}

impl AbstractBeforeAfterRule for VerifyTestClassNamingConvention {
    /// `before()`.
    fn before(&self, _description: &Arc<Description>) {
        if TestRuleIgnoreTestSuites::is_running_nested() {
            // Ignore nested test suites that test the test framework itself.
            return;
        }
        let suite_name = RandomizedContext::current().get_target_class().to_string();
        if suite_name.starts_with(&self.package_prefix) && !(self.matches)(&suite_name) {
            Throwable::new(
                exception::ASSERTION_ERROR,
                format!(
                    "{} suite must follow {} naming convention: {suite_name}",
                    self.package_prefix, self.naming_convention
                ),
            )
            .throw();
        }
    }
}
