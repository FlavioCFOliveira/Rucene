//! Port of `com.carrotsearch.randomizedtesting.rules.RequireAssertionsRule`
//! (randomizedtesting 2.8.1, `rules/RequireAssertionsRule.java`).

use std::sync::{Arc, LazyLock};

use super::TestRuleAdapter;
use crate::jdk::exception::{Throwable, EXCEPTION};
use crate::jdk::reflect::Class;
use crate::randomized_test::RandomizedTest;
use crate::sys_globals::SysGlobals;

/// `TEST_ASSERTS_ENABLED`: `tests.asserts`, default true.
static TEST_ASSERTS_ENABLED: LazyLock<bool> = LazyLock::new(|| {
    RandomizedTest::system_property_as_boolean(&SysGlobals::sysprop_asserts(), true)
});

/// Requires the assertion state to match `tests.asserts`
/// (`RequireAssertionsRule`), as a [`TestRuleAdapter`].
///
/// **Divergence.** Java reads `targetClass.desiredAssertionStatus()` (the
/// JVM's `-ea`, `RequireAssertionsRule.java:48`); the port reads whether Rust debug assertions are on, the
/// switch [`java_assert`](crate::jdk::exception::java_assert) follows.
pub struct RequireAssertionsRule {
    target_class: Arc<Class>,
}

impl RequireAssertionsRule {
    /// `TEST_ASSERTS_ENABLED`.
    pub fn test_asserts_enabled() -> bool {
        *TEST_ASSERTS_ENABLED
    }

    /// `new RequireAssertionsRule(targetClass)`.
    pub fn new(target_class: Arc<Class>) -> RequireAssertionsRule {
        RequireAssertionsRule { target_class }
    }
}

impl TestRuleAdapter for RequireAssertionsRule {
    fn before(&self) {
        let asserts_enabled = cfg!(debug_assertions);
        if asserts_enabled != *TEST_ASSERTS_ENABLED {
            let mut msg = format!(
                "Assertion state mismatch on {}: ",
                self.target_class.simple_name()
            );
            if asserts_enabled {
                msg.push_str("-ea was specified");
            } else {
                msg.push_str("-ea was not specified");
            }
            msg.push_str(&format!(" but -Dtests.asserts={}", *TEST_ASSERTS_ENABLED));
            crate::jdk::system::err_println(&msg);
            Throwable::new(EXCEPTION, msg).throw();
        }
    }
}
