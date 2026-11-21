//! Port of `com.carrotsearch.randomizedtesting.ReproduceErrorMessageBuilder`
//! (randomizedtesting 2.8.1, `ReproduceErrorMessageBuilder.java`).

use std::sync::Arc;

use crate::jdk::system;
use crate::junit::Description;
use crate::randomized_context::RandomizedContext;
use crate::randomized_runner::{method_name, RandomizedRunner};
use crate::sys_globals::SysGlobals;

/// Builds the "reproduce with" message: `-Dname=value` options
/// (`ReproduceErrorMessageBuilder`).
#[derive(Debug, Default)]
pub struct ReproduceErrorMessageBuilder {
    b: String,
}

impl ReproduceErrorMessageBuilder {
    /// `new ReproduceErrorMessageBuilder()`.
    pub fn new() -> ReproduceErrorMessageBuilder {
        ReproduceErrorMessageBuilder::default()
    }

    /// `new ReproduceErrorMessageBuilder(StringBuilder)`: continues `text`.
    pub fn with_text(text: impl Into<String>) -> ReproduceErrorMessageBuilder {
        ReproduceErrorMessageBuilder { b: text.into() }
    }

    /// The text built so far.
    pub fn text(&self) -> &str {
        &self.b
    }

    /// The text built so far, owned.
    pub fn into_text(self) -> String {
        self.b
    }

    /// `appendAllOpts(description)`: the master seed (from the current
    /// context), the test class and method, the runner's properties, the
    /// group options and the environment settings.
    pub fn append_all_opts(&mut self, description: &Description) -> &mut Self {
        let ctx = {
            let _quiet = crate::jdk::exception::QuietPanics::new();
            std::panic::catch_unwind(RandomizedContext::current).ok()
        };
        match &ctx {
            Some(c) => {
                self.append_opt(
                    &SysGlobals::sysprop_random_seed(),
                    &c.get_runner_seed_as_string(),
                );
            }
            None => RandomizedRunner::logger()
                .warning("No context available when dumping reproduce options?"),
        }
        if let Some(c) = description.class_name() {
            self.append_opt(&SysGlobals::sysprop_testclass(), &c);
        }
        if description.method_name().is_some() {
            self.append_opt(&SysGlobals::sysprop_testmethod(), &method_name(description));
        }
        self.append_runner_properties();
        self.append_test_group_options(ctx.as_ref());
        self.append_environment_settings();
        self
    }

    /// `appendEnvironmentSettings()`: `file.encoding` and `user.timezone`
    /// when set.
    pub fn append_environment_settings(&mut self) -> &mut Self {
        for name in ["file.encoding", "user.timezone"] {
            if let Some(v) = system::get_property(name) {
                if !crate::jdk::lang::trim(&v).is_empty() {
                    self.append_opt(name, &v);
                }
            }
        }
        self
    }

    /// `appendTestGroupOptions(ctx)`.
    pub fn append_test_group_options(&mut self, ctx: Option<&Arc<RandomizedContext>>) -> &mut Self {
        if let Some(c) = ctx {
            c.get_group_evaluator().append_group_filtering_options(self);
        }
        self
    }

    /// `appendRunnerProperties()`: the prefix, then the runner properties
    /// that are set.
    pub fn append_runner_properties(&mut self) -> &mut Self {
        self.append_opt(SysGlobals::sysprop_prefix(), &SysGlobals::current_prefix());
        for name in [
            SysGlobals::sysprop_stackfiltering(),
            SysGlobals::sysprop_iterations(),
            SysGlobals::sysprop_killattempts(),
            SysGlobals::sysprop_killwait(),
            SysGlobals::sysprop_timeout(),
        ] {
            if let Some(v) = system::get_property(&name) {
                self.append_opt(&name, &v);
            }
        }
        self
    }

    /// `appendOpt(sysPropName, value)`: ` -Dname=value`.
    pub fn append_opt(&mut self, sys_prop_name: &str, value: &str) -> &mut Self {
        if !self.b.is_empty() {
            self.b.push(' ');
        }
        self.b.push_str("-D");
        self.b.push_str(sys_prop_name);
        self.b.push('=');
        self.b.push_str(value);
        self
    }
}
