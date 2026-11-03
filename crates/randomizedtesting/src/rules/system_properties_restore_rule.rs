//! Port of `com.carrotsearch.randomizedtesting.rules.SystemPropertiesRestoreRule`
//! (randomizedtesting 2.8.1, `rules/SystemPropertiesRestoreRule.java`).

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use crate::jdk::system;
use crate::junit::{Description, Statement, TestRule};
use crate::randomized_runner::try_run;

/// Restores the system properties after the base statement to what they
/// were before it (`SystemPropertiesRestoreRule`).
///
/// **Divergence.** `System.getProperties()` (`SystemPropertiesRestoreRule.java:140`)
/// is the port's property table
/// (see [`crate::jdk::system`]): the properties set through it, not the
/// JVM's platform properties.
#[derive(Debug, Clone, Default)]
pub struct SystemPropertiesRestoreRule {
    ignored_properties: HashSet<String>,
    set_properties: Vec<(String, String)>,
}

impl SystemPropertiesRestoreRule {
    /// `new SystemPropertiesRestoreRule()`: restores every property.
    pub fn new() -> SystemPropertiesRestoreRule {
        SystemPropertiesRestoreRule::default()
    }

    /// `new SystemPropertiesRestoreRule(key, value)`: also sets `key` for
    /// the duration.
    pub fn with_property(key: &str, value: &str) -> SystemPropertiesRestoreRule {
        SystemPropertiesRestoreRule::with_properties(&[(key, value)])
    }

    /// `new SystemPropertiesRestoreRule(Map setProperties)`.
    pub fn with_properties(props: &[(&str, &str)]) -> SystemPropertiesRestoreRule {
        SystemPropertiesRestoreRule {
            ignored_properties: HashSet::new(),
            set_properties: props
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    /// `new SystemPropertiesRestoreRule(String... ignoredProperties)`.
    pub fn ignoring(ignored: &[&str]) -> SystemPropertiesRestoreRule {
        SystemPropertiesRestoreRule {
            ignored_properties: ignored.iter().map(|s| s.to_string()).collect(),
            set_properties: Vec::new(),
        }
    }
}

impl TestRule for SystemPropertiesRestoreRule {
    fn apply(&self, base: Statement, _d: &Arc<Description>) -> Statement {
        let me = self.clone();
        Statement::new(move || {
            let before = system_properties_as_map();
            let r = try_run(|| {
                for (k, v) in &me.set_properties {
                    system::set_property(k, v);
                }
                base.evaluate();
            });
            let after = system_properties_as_map();
            if after != before {
                restore(&before, after, &me.ignored_properties);
            }
            if let Err(t) = r {
                t.throw();
            }
        })
    }
}

/// `restore(before, after, ignoredKeys)`: clears the keys added since
/// `before` (unless ignored) and sets every key of `before` back.
///
/// Java checks the *value* of each `before` entry against the ignored keys
/// (`String key = e.getValue()`), so an ignored property is restored all
/// the same; the port keeps that.
pub(crate) fn restore(
    before: &BTreeMap<String, String>,
    mut after: BTreeMap<String, String>,
    ignored_keys: &HashSet<String>,
) {
    after.retain(|k, _| !before.contains_key(k));
    for key in after.keys() {
        if !ignored_keys.contains(key) {
            system::clear_property(key);
        }
    }
    for (k, v) in before {
        if !ignored_keys.contains(v) {
            system::set_property(k, v);
        }
    }
}

/// `systemPropertiesAsMap()`: the properties, sorted by key.
pub(crate) fn system_properties_as_map() -> BTreeMap<String, String> {
    system::properties_snapshot().into_iter().collect()
}
