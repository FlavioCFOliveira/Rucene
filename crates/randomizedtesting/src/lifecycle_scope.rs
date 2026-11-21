//! Port of `com.carrotsearch.randomizedtesting.LifecycleScope`
//! (randomizedtesting 2.8.1, `LifecycleScope.java`).

use std::fmt;

/// Lifecycle stages for tracking resources: `LifecycleScope`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LifecycleScope {
    /// `TEST`: a single test case, including all `@After` hooks.
    Test,
    /// `SUITE`: a single suite (class), including all `@AfterClass` hooks.
    Suite,
}

impl LifecycleScope {
    /// `name()`.
    pub fn name(&self) -> &'static str {
        match self {
            LifecycleScope::Test => "TEST",
            LifecycleScope::Suite => "SUITE",
        }
    }
}

impl fmt::Display for LifecycleScope {
    /// `Enum.toString()`: the constant's name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
