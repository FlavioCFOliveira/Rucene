//! randomizedtesting 2.8.1's own test suite (`randomized-runner/src/test`,
//! package `com.carrotsearch.randomizedtesting`), ported (task #330) and run
//! by the ported `RandomizedRunner` through the ported `JUnitCore`
//! (`harness = false`).
//!
//! Each Java test class is a module of the same name under the module of its
//! Java package, declaring the class (and its nested classes) with the
//! suite-declaration builder of `randomizedtesting::jdk::reflect`, member by
//! member. The driver runs every ported class, as Maven's surefire runs the
//! Java ones, and compares per class the run, failure, ignored and
//! assumption-failure counts and every failure's test, exception class and
//! message against `tests/fixtures/self_tests.tsv`, which the Java classes
//! wrote under JUnit 4.13.1 (`tests/fixtures/oracle/SelfTestsOracle.java`).
//!
//! Masked on both sides before comparing: stack frames (`\tat …`,
//! `\t... N more`), random seeds (hexadecimal runs of 8 to 16 digits in
//! brackets), thread ids and the unnamed module's identity hash.
//!
//! `SELF_TESTS_FILTER=<substring>[,<substring>...]` runs only the classes
//! whose name contains one of them.
//!
//! **Divergences.**
//! * JUnit runs a class without `@RunWith` under its own
//!   `BlockJUnit4ClassRunner`; the port has only `RandomizedRunner`
//!   (`Request::a_class`), so the outer classes and the plain nested ones run
//!   under it. For the classes ported here the counts agree; the one class
//!   whose assertion compares JUnit's order with the runner's
//!   (`inheritance.JUnitAnnotationPropagation`, `JUnitAnnotationPropagation.java:67`)
//!   is listed in its module.
//! * `@RunWith(Suite.class) @SuiteClasses({…})` (JUnit's `Suite` runner) is
//!   the list of its member classes, run in order in one run.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{Failure, JUnitCore, RunListener};

mod contracts;
mod generators;
mod inheritance;
mod print_event_listener;
mod rules;
mod support;
mod timeouts;
mod utils;
mod with_nested_test_class;

mod test_biased_numbers;
mod test_child_test_group_threads;
mod test_class_method_filtering;
mod test_context_random;
mod test_custom_method_provider;
mod test_custom_test_context_supplier;
mod test_expected;
mod test_failure_propagation_compatibility;
mod test_filter_expression_parser;
mod test_formatting_random_seeds;
mod test_glob_filter;
mod test_hook_method_order_with_exceptions;
mod test_hooks_with_empty_test_set;
mod test_ignored_run_count;
mod test_instantiation_time;
mod test_iterations_annotation;
mod test_j9_sys_threads;
mod test_junit3_method_provider;
mod test_listeners_annotation;
mod test_mac_sys_threads;
mod test_many_threads_oom;
mod test_nightly_mode;
mod test_out_of_scope_random_use;
mod test_overlapping_method_providers;
mod test_overriding_default_exception_handler;
mod test_parameterized;
mod test_parameterized_shuffles_order;
mod test_random;
mod test_random_in_static_initializer;
mod test_random_numbers;
mod test_randomized_context;
mod test_randomized_test;
mod test_repeat_test_with_complex_description;
mod test_resource_disposal;
mod test_rules;
mod test_seed_decorator;
mod test_seed_fixing;
mod test_seed_fixing_with_properties;
mod test_seed_parameter_optional;
mod test_seed_repeatable;
mod test_set_seed_locked;
mod test_stack_augmentation;
mod test_target_method;
mod test_test_case_instance_providers;
mod test_test_case_ordering;
mod test_test_filtering;
mod test_test_groups;
mod test_test_groups_on_overrides;
mod test_thread_name_contains_test_name;
mod test_uncaught_exceptions_duplicated;
mod test_validation;

/// What a test class is run as: its Java name and the classes JUnit runs for
/// it (one, or a `Suite`'s members).
pub type Entry = (&'static str, fn() -> Vec<Arc<Class>>);

const P: &str = "com.carrotsearch.randomizedtesting.";

/// Classes whose Java outcome the port cannot reproduce, with the declared
/// divergence: reported next to the Java counts, not counted as
/// differences.
const DECLARED_DIVERGENT: &[(&str, &str)] = &[(
    "inheritance.JUnitAnnotationPropagation",
    "compares JUnit's BlockJUnit4ClassRunner with RandomizedRunner; the port has only the latter",
)];

/// Every ported test class, by Java name.
fn registry() -> Vec<Entry> {
    let mut v: Vec<Entry> = Vec::new();
    v.extend(contracts::entries());
    v.extend(generators::entries());
    v.extend(inheritance::entries());
    v.extend(rules::entries());
    v.extend(timeouts::entries());
    v.extend([
        (
            "TestBiasedNumbers",
            test_biased_numbers::classes as fn() -> Vec<Arc<Class>>,
        ),
        (
            "TestChildTestGroupThreads",
            test_child_test_group_threads::classes,
        ),
        (
            "TestClassMethodFiltering",
            test_class_method_filtering::classes,
        ),
        ("TestContextRandom", test_context_random::classes),
        (
            "TestCustomMethodProvider",
            test_custom_method_provider::classes,
        ),
        (
            "TestCustomTestContextSupplier",
            test_custom_test_context_supplier::classes,
        ),
        ("TestExpected", test_expected::classes),
        (
            "TestFailurePropagationCompatibility",
            test_failure_propagation_compatibility::classes,
        ),
        (
            "TestFilterExpressionParser",
            test_filter_expression_parser::classes,
        ),
        (
            "TestFormattingRandomSeeds",
            test_formatting_random_seeds::classes,
        ),
        ("TestGlobFilter", test_glob_filter::classes),
        (
            "TestHookMethodOrderWithExceptions",
            test_hook_method_order_with_exceptions::classes,
        ),
        (
            "TestHooksWithEmptyTestSet",
            test_hooks_with_empty_test_set::classes,
        ),
        ("TestIgnoredRunCount", test_ignored_run_count::classes),
        ("TestInstantiationTime", test_instantiation_time::classes),
        (
            "TestIterationsAnnotation",
            test_iterations_annotation::classes,
        ),
        ("TestJ9SysThreads", test_j9_sys_threads::classes),
        (
            "TestJUnit3MethodProvider",
            test_junit3_method_provider::classes,
        ),
        (
            "TestListenersAnnotation",
            test_listeners_annotation::classes,
        ),
        ("TestMacSysThreads", test_mac_sys_threads::classes),
        ("TestManyThreadsOOM", test_many_threads_oom::classes),
        ("TestNightlyMode", test_nightly_mode::classes),
        (
            "TestOutOfScopeRandomUse",
            test_out_of_scope_random_use::classes,
        ),
        (
            "TestOverlappingMethodProviders",
            test_overlapping_method_providers::classes,
        ),
        (
            "TestOverridingDefaultExceptionHandler",
            test_overriding_default_exception_handler::classes,
        ),
        ("TestParameterized", test_parameterized::classes),
        (
            "TestParameterizedShufflesOrder",
            test_parameterized_shuffles_order::classes,
        ),
        ("TestRandom", test_random::classes),
        (
            "TestRandomInStaticInitializer",
            test_random_in_static_initializer::classes,
        ),
        ("TestRandomNumbers", test_random_numbers::classes),
        ("TestRandomizedContext", test_randomized_context::classes),
        ("TestRandomizedTest", test_randomized_test::classes),
        (
            "TestRepeatTestWithComplexDescription",
            test_repeat_test_with_complex_description::classes,
        ),
        ("TestResourceDisposal", test_resource_disposal::classes),
        ("TestRules", test_rules::classes),
        ("TestSeedDecorator", test_seed_decorator::classes),
        ("TestSeedFixing", test_seed_fixing::classes),
        (
            "TestSeedFixingWithProperties",
            test_seed_fixing_with_properties::classes,
        ),
        (
            "TestSeedParameterOptional",
            test_seed_parameter_optional::classes,
        ),
        ("TestSeedRepeatable", test_seed_repeatable::classes),
        ("TestSetSeedLocked", test_set_seed_locked::classes),
        ("TestStackAugmentation", test_stack_augmentation::classes),
        ("TestTargetMethod", test_target_method::classes),
        (
            "TestTestCaseInstanceProviders",
            test_test_case_instance_providers::classes,
        ),
        ("TestTestCaseOrdering", test_test_case_ordering::classes),
        ("TestTestFiltering", test_test_filtering::classes),
        ("TestTestGroups", test_test_groups::classes),
        (
            "TestTestGroupsOnOverrides",
            test_test_groups_on_overrides::classes,
        ),
        (
            "TestThreadNameContainsTestName",
            test_thread_name_contains_test_name::classes,
        ),
        (
            "TestUncaughtExceptionsDuplicated",
            test_uncaught_exceptions_duplicated::classes,
        ),
        ("TestValidation", test_validation::classes),
    ]);
    v
}

/// Escapes as the oracle's `esc`.
fn esc(s: Option<&str>) -> String {
    match s {
        None => "<null>".to_string(),
        Some(s) => s
            .replace('\\', "\\\\")
            .replace('\n', "\\n")
            .replace('\t', "\\t"),
    }
}

/// Records failures and assumption failures as the oracle does.
struct Recorder {
    class: String,
    failures: Mutex<Vec<String>>,
    assumptions: Mutex<usize>,
}

impl RunListener for Recorder {
    fn test_failure(&self, f: &Failure) {
        let t = f.exception();
        self.failures
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(format!(
                "F\t{}\t{}\t{}\t{}",
                self.class,
                esc(Some(f.description().display_name())),
                t.class_name(),
                esc(t.message())
            ));
    }
    fn test_assumption_failure(&self, _f: &Failure) {
        *self.assumptions.lock().unwrap_or_else(|e| e.into_inner()) += 1;
    }
}

fn run_entry(name: &str, classes: &[Arc<Class>]) -> Vec<String> {
    let recorder = Arc::new(Recorder {
        class: name.to_string(),
        failures: Mutex::new(Vec::new()),
        assumptions: Mutex::new(0),
    });
    let mut core = JUnitCore::new();
    core.add_listener(recorder.clone());
    let r = core.run_all(classes);
    let mut lines = vec![format!(
        "C\t{name}\t{}\t{}\t{}\t{}",
        r.run_count(),
        r.failure_count(),
        r.ignore_count(),
        *recorder
            .assumptions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    )];
    lines.extend(
        recorder
            .failures
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned(),
    );
    lines
}

/// Masks the parts of an escaped failure line that differ between runs and
/// between a JVM and the port.
fn mask(line: &str) -> String {
    // Escaped segments: drop stack frames and "... N more".
    let segments: Vec<&str> = line.split("\\n").collect();
    let kept: Vec<&str> = segments
        .into_iter()
        .filter(|s| {
            let t = s.trim_start_matches("\\t");
            !(s.starts_with("\\t") && (t.starts_with("at ") || t.starts_with("... ")))
        })
        .collect();
    let mut s = kept.join("\\n");
    s = mask_hex_runs(&s);
    for prefix in ["Thread[#", "id="] {
        s = support::mask_digits(&s, prefix);
    }
    support::mask_hex_after(&s, "unnamed module @")
}

/// Replaces each bracketed run of 8 to 16 hexadecimal digits (a seed, alone
/// or in a `[A:B]` chain) with `S`.
fn mask_hex_runs(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        let n = b[i..].iter().take_while(|c| c.is_ascii_hexdigit()).count();
        let prev = if i == 0 { b' ' } else { b[i - 1] };
        let next = b.get(i + n).copied().unwrap_or(b' ');
        if (8..=16).contains(&n) && (prev == b'[' || prev == b':') && (next == b']' || next == b':')
        {
            out.push('S');
            i += n;
        } else if n > 0 {
            out.push_str(&s[i..i + n]);
            i += n;
        } else {
            let c = s[i..].chars().next().unwrap_or(' ');
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

fn fixture() -> BTreeMap<String, Vec<String>> {
    let path = format!(
        "{}/tests/fixtures/self_tests.tsv",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for l in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let class = l.split('\t').nth(1).unwrap_or("").to_string();
        m.entry(class).or_default().push(l.to_string());
    }
    m
}

fn main() {
    randomizedtesting::jdk::system::set_property("file.encoding", "UTF-8");
    let filter = std::env::var("SELF_TESTS_FILTER").ok();
    let expected = fixture();
    let registry: BTreeMap<String, fn() -> Vec<Arc<Class>>> = registry()
        .into_iter()
        .map(|(n, f)| (format!("{P}{n}"), f))
        .collect();
    let mut differences = 0;
    let mut report = Vec::new();
    for (class, java) in &expected {
        if filter
            .as_ref()
            .is_some_and(|f| !f.split(',').any(|p| class.contains(p)))
        {
            continue;
        }
        let Some(make) = registry.get(class) else {
            report.push(format!("NOT PORTED {class}"));
            differences += 1;
            continue;
        };
        let rust = run_entry(class, &make());
        let mut j: Vec<String> = java.iter().map(|l| mask(l)).collect();
        let mut r: Vec<String> = rust.iter().map(|l| mask(l)).collect();
        j[1..].sort();
        r[1..].sort();
        let java_counts = java[0].split('\t').skip(2).collect::<Vec<_>>().join(",");
        let rust_counts = rust[0].split('\t').skip(2).collect::<Vec<_>>().join(",");
        let declared = DECLARED_DIVERGENT
            .iter()
            .find(|(n, _)| format!("{P}{n}") == *class);
        if let Some((_, why)) = declared.filter(|_| j != r) {
            report.push(format!(
                "DIVERGENT (declared) {class}  java[run,fail,ign,assume]={java_counts}  rust={rust_counts}: {why}"
            ));
        } else if j == r {
            report.push(format!(
                "ok   {class}  java[run,fail,ign,assume]={java_counts}  rust={rust_counts}"
            ));
        } else {
            differences += 1;
            report.push(format!(
                "DIFF {class}  java[run,fail,ign,assume]={java_counts}  rust={rust_counts}"
            ));
            for i in 0..j.len().max(r.len()) {
                let x = j.get(i).map(String::as_str).unwrap_or("<none>");
                let y = r.get(i).map(String::as_str).unwrap_or("<none>");
                if x != y {
                    report.push(format!("    java: {x}\n    rust: {y}"));
                }
            }
        }
    }
    for k in registry.keys() {
        if !expected.contains_key(k) {
            report.push(format!("EXTRA (no oracle row) {k}"));
            differences += 1;
        }
    }
    println!("\n==== randomizedtesting self tests: Java (JUnit 4.13.1) vs Rust");
    for l in &report {
        println!("{l}");
    }
    if differences > 0 {
        println!("self tests: {differences} difference(s)");
        std::process::exit(1);
    }
    println!("self tests: every class equals its Java counterpart");
}
