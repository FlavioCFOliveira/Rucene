//! Port of `com.carrotsearch.randomizedtesting.rules.SystemPropertiesInvariantRule`
//! (randomizedtesting 2.8.1, `rules/SystemPropertiesInvariantRule.java`).

use std::collections::{BTreeMap, HashSet};
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

/// `collectErrorMessage(before, after)`: the missing, new and changed keys,
/// each list in Java's `TreeSet<String>` order (`String.compareTo`, UTF-16
/// code units).
fn collect_error_message(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> String {
    let new_keys = java_sorted(after.keys().filter(|k| !before.contains_key(*k)).collect());
    let missing = java_sorted(before.keys().filter(|k| !after.contains_key(*k)).collect());
    let different = java_sorted(
        before
            .keys()
            .filter(|k| after.contains_key(*k) && before.get(*k) != after.get(*k))
            .collect(),
    );
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

/// `v` sorted as a Java `TreeSet<String>` iterates.
fn java_sorted(mut v: Vec<&String>) -> Vec<&String> {
    v.sort_by(|a, b| crate::annotations::jdk_string_compare(a, b));
    v
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::collect_error_message;

    /// Keys sort as Java strings: a supplementary character (high surrogate
    /// `D800..DBFF`) sorts before `U+FFFD`, unlike UTF-8 byte order.
    #[test]
    fn keys_are_listed_in_java_string_order() {
        let before = BTreeMap::new();
        let after: BTreeMap<String, String> = [("\u{FFFD}", "a"), ("\u{10000}", "b")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        assert_eq!(
            collect_error_message(&before, &after),
            "New keys:\n  \u{10000}=b\n  \u{FFFD}=a\n"
        );
    }
}
