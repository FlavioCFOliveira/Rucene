//! Port of `com.carrotsearch.randomizedtesting.SysGlobals`
//! (randomizedtesting 2.8.1, `SysGlobals.java`).

use std::sync::Mutex;

use crate::jdk::exception;
use crate::jdk::system;

/// `CHILDVM_SYSPROP_JVM_ID`.
pub const CHILDVM_SYSPROP_JVM_ID: &str = "junit4.childvm.id";
/// `CHILDVM_SYSPROP_JVM_COUNT`.
pub const CHILDVM_SYSPROP_JVM_COUNT: &str = "junit4.childvm.count";

const DEFAULT_PREFIX: &str = "tests";
const SYSPROP_PREFIX: &str = "tests.prefix";

/// Global names and constants of the runner's system properties,
/// rendered with a common prefix (`tests` unless `tests.prefix` says
/// otherwise): `SysGlobals`.
#[derive(Debug, Clone)]
pub struct SysGlobals {
    prefix: String,
    sysprop_stackfiltering: String,
    sysprop_random_seed: String,
    sysprop_iterations: String,
    sysprop_testclass: String,
    sysprop_testmethod: String,
    sysprop_testfilter: String,
    sysprop_killattempts: String,
    sysprop_killwait: String,
    sysprop_timeout: String,
    sysprop_timeout_suite: String,
    sysprop_append_seed: String,
    sysprop_asserts: String,
}

/// The singleton and the stack of the thread that initialized it.
static SINGLETON: Mutex<Option<(SysGlobals, String)>> = Mutex::new(None);

impl SysGlobals {
    fn new(prefix: &str) -> SysGlobals {
        let p = |name: &str| SysGlobals::prefix_with(prefix, name);
        SysGlobals {
            prefix: prefix.to_string(),
            sysprop_stackfiltering: p("stackfiltering"),
            sysprop_random_seed: p("seed"),
            sysprop_iterations: p("iters"),
            sysprop_testclass: p("class"),
            sysprop_testmethod: p("method"),
            sysprop_testfilter: p("filter"),
            sysprop_killattempts: p("killattempts"),
            sysprop_killwait: p("killwait"),
            sysprop_timeout: p("timeout"),
            sysprop_timeout_suite: p("timeoutSuite"),
            sysprop_append_seed: p("appendseed"),
            sysprop_asserts: p("asserts"),
        }
    }

    fn prefix_with(prefix: &str, property_name: &str) -> String {
        if prefix.is_empty() {
            property_name.to_string()
        } else {
            format!(
                "{prefix}{}{property_name}",
                if prefix.ends_with('.') { "" } else { "." }
            )
        }
    }

    fn singleton() -> SysGlobals {
        let existing = SINGLETON
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|(s, _)| s.clone());
        match existing {
            Some(s) => s,
            None => {
                let prefix =
                    system::get_property(SYSPROP_PREFIX).unwrap_or_else(|| DEFAULT_PREFIX.into());
                SysGlobals::initialize_with(&prefix)
            }
        }
    }

    /// `initializeWith(String prefix)`.
    ///
    /// # Panics
    ///
    /// `RuntimeException` when the singleton was already initialized with a
    /// different prefix.
    pub fn initialize_with(prefix: &str) -> SysGlobals {
        let mut guard = SINGLETON.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            *guard = Some((
                SysGlobals::new(prefix),
                std::backtrace::Backtrace::capture().to_string(),
            ));
        }
        let (singleton, _stack) = guard.as_ref().expect("initialized");
        if singleton.prefix != prefix {
            let message = format!(
                "A singleton has been initialized already with a different prefix: existing={}, \
                 attempted={}",
                singleton.prefix, prefix
            );
            drop(guard);
            exception::runtime(message);
        }
        singleton.clone()
    }

    /// `SYSPROP_PREFIX()`.
    pub fn sysprop_prefix() -> &'static str {
        SYSPROP_PREFIX
    }

    /// `CURRENT_PREFIX()`.
    pub fn current_prefix() -> String {
        SysGlobals::singleton().prefix
    }

    /// `SYSPROP_STACKFILTERING()`.
    pub fn sysprop_stackfiltering() -> String {
        SysGlobals::singleton().sysprop_stackfiltering
    }

    /// `SYSPROP_RANDOM_SEED()`: the master seed property (`tests.seed`).
    pub fn sysprop_random_seed() -> String {
        SysGlobals::singleton().sysprop_random_seed
    }

    /// `SYSPROP_ITERATIONS()`.
    pub fn sysprop_iterations() -> String {
        SysGlobals::singleton().sysprop_iterations
    }

    /// `SYSPROP_TESTCLASS()`.
    pub fn sysprop_testclass() -> String {
        SysGlobals::singleton().sysprop_testclass
    }

    /// `SYSPROP_TESTMETHOD()`.
    pub fn sysprop_testmethod() -> String {
        SysGlobals::singleton().sysprop_testmethod
    }

    /// `SYSPROP_TESTFILTER()`.
    pub fn sysprop_testfilter() -> String {
        SysGlobals::singleton().sysprop_testfilter
    }

    /// `SYSPROP_KILLATTEMPTS()`.
    pub fn sysprop_killattempts() -> String {
        SysGlobals::singleton().sysprop_killattempts
    }

    /// `SYSPROP_KILLWAIT()`.
    pub fn sysprop_killwait() -> String {
        SysGlobals::singleton().sysprop_killwait
    }

    /// `SYSPROP_TIMEOUT()`.
    pub fn sysprop_timeout() -> String {
        SysGlobals::singleton().sysprop_timeout
    }

    /// `SYSPROP_TIMEOUT_SUITE()`.
    pub fn sysprop_timeout_suite() -> String {
        SysGlobals::singleton().sysprop_timeout_suite
    }

    /// `SYSPROP_APPEND_SEED()`.
    pub fn sysprop_append_seed() -> String {
        SysGlobals::singleton().sysprop_append_seed
    }

    /// `SYSPROP_ASSERTS()`.
    pub fn sysprop_asserts() -> String {
        SysGlobals::singleton().sysprop_asserts
    }

    /// `prefixProperty(String propertyName)`: `propertyName` with the
    /// common prefix.
    pub fn prefix_property(property_name: &str) -> String {
        let s = SysGlobals::singleton();
        SysGlobals::prefix_with(&s.prefix, property_name)
    }
}
