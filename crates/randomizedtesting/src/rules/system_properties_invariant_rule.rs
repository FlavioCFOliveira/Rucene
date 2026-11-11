//! Port of `com.carrotsearch.randomizedtesting.rules.SystemPropertiesInvariantRule`
//! (randomizedtesting 2.8.1, `rules/SystemPropertiesInvariantRule.java`).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use super::system_properties_restore_rule::{restore, system_properties_as_map};
use crate::jdk::exception::{Throwable, ASSERTION_ERROR};
use crate::junit::{Description, Statement, TestRule};
use crate::randomized_runner::{assert_empty, try_run};

/// Fails when the base statement leaves the system properties changed, and
/// restores them (`SystemPropertiesInvariantRule`).
///
/// **Divergence.** As [`SystemPropertiesRestoreRule`](super::SystemPropertiesRestoreRule):
/// the properties are the port's property table.
#[derive(Debug, Clone, Default)]
pub struct SystemPropertiesInvariantRule {
    ignored_properties: HashSet<String>,
}

impl SystemPropertiesInvariantRule {
    /// `new SystemPropertiesInvariantRule()`: every property counts.
    pub fn new() -> SystemPropertiesInvariantRule {
        SystemPropertiesInvariantRule::default()
    }

    /// `new SystemPropertiesInvariantRule(String... ignoredProperties)`.
    pub fn ignoring(ignored: &[&str]) -> SystemPropertiesInvariantRule {
        SystemPropertiesInvariantRule {
            ignored_properties: ignored.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl TestRule for SystemPropertiesInvariantRule {
    fn apply(&self, s: Statement, _d: &Arc<Description>) -> Statement {
        let me = self.clone();
        Statement::new(move || {
            let mut before = system_properties_as_map();
            let mut errors = Vec::new();
            if let Err(t) = try_run(|| s.evaluate()) {
                errors.push(t);
            }
            let mut after = system_properties_as_map();
            before.retain(|k, _| !me.ignored_properties.contains(k));
            after.retain(|k, _| !me.ignored_properties.contains(k));
            if after != before {
                errors.push(Throwable::new(
                    ASSERTION_ERROR,
                    format!(
                        "System properties invariant violated.\n{}",
                        collect_error_message(&before, &after)
                    ),
                ));
            }
            restore(&before, after, &me.ignored_properties);
            assert_empty(errors);
        })
    }
}

/// `collectErrorMessage(before, after)`: the missing, new and changed keys.
fn collect_error_message(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> String {
    let new_keys: BTreeSet<&String> = after.keys().filter(|k| !before.contains_key(*k)).collect();
    let missing: BTreeSet<&String> = before.keys().filter(|k| !after.contains_key(*k)).collect();
    let different: BTreeSet<&String> = before
        .keys()
        .filter(|k| after.contains_key(*k) && before.get(*k) != after.get(*k))
        .collect();
    let mut b = String::new();
    if !missing.is_empty() {
        b.push_str("Missing keys:\n");
        for k in missing {
            b.push_str(&format!("  {}={}\n", k, before[k]));
        }
    }
    if !new_keys.is_empty() {
        b.push_str("New keys:\n");
        for k in new_keys {
            b.push_str(&format!("  {}={}\n", k, after[k]));
        }
    }
    if !different.is_empty() {
        b.push_str("Different values:\n");
        for k in different {
            b.push_str(&format!("  [old]{}={}\n", k, before[k]));
            b.push_str(&format!("  [new]{}={}\n", k, after[k]));
        }
    }
    b
}
