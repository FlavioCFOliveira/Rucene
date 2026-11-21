//! Port of `com.carrotsearch.randomizedtesting.rules` (randomizedtesting
//! 2.8.1, `rules/*.java`): statement and rule adapters, the hook shadowing
//! and override checks, the assertion requirement, the static-field and
//! system-property invariants, and the memory estimator they use.

mod no_shadowing_or_overrides_on_methods_rule;
mod ram_usage_estimator;
mod require_assertions_rule;
mod statement_adapter;
mod static_fields_invariant_rule;
mod system_properties_invariant_rule;
mod system_properties_restore_rule;
mod test_rule_adapter;

pub use no_shadowing_or_overrides_on_methods_rule::{
    NoClassHooksShadowingRule, NoInstanceHooksOverridesRule, NoShadowingOrOverridesOnMethodsRule,
};
pub use ram_usage_estimator::{JvmFeature, RamUsageEstimator};
pub use require_assertions_rule::RequireAssertionsRule;
pub use statement_adapter::StatementAdapter;
pub use static_fields_invariant_rule::StaticFieldsInvariantRule;
pub use system_properties_invariant_rule::SystemPropertiesInvariantRule;
pub use system_properties_restore_rule::SystemPropertiesRestoreRule;
pub use test_rule_adapter::{TestRuleAdapter, TestRuleAdapterRule};
