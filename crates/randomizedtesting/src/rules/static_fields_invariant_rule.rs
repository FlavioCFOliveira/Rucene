//! Port of `com.carrotsearch.randomizedtesting.rules.StaticFieldsInvariantRule`
//! (randomizedtesting 2.8.1, `rules/StaticFieldsInvariantRule.java`).

use std::sync::Arc;

use super::{RamUsageEstimator, StatementAdapter};
use crate::jdk::exception::{runtime, Throwable};
use crate::jdk::reflect::{modifier, Class, Field, Value};
use crate::junit::{Description, Statement, TestRule};
use crate::randomized_context::RandomizedContext;
use crate::randomized_runner::try_run;

/// `junit.framework.AssertionFailedError`.
const ASSERTION_FAILED_ERROR: &str = "junit.framework.AssertionFailedError";

type Accept = Arc<dyn Fn(&Field) -> bool + Send + Sync>;

/// After the suite, fails when the static reference fields of the suite
/// class (and its superclasses) still hold more than a threshold of memory
/// (`StaticFieldsInvariantRule`). Measured with [`RamUsageEstimator`]
/// (see its divergences).
#[derive(Clone)]
pub struct StaticFieldsInvariantRule {
    leak_threshold: i64,
    count_superclasses: bool,
    accept: Accept,
}

impl StaticFieldsInvariantRule {
    /// `DEFAULT_LEAK_THRESHOLD`: 10 MiB.
    pub const DEFAULT_LEAK_THRESHOLD: i64 = 10 * 1024 * 1024;

    /// `new StaticFieldsInvariantRule()`.
    pub fn new() -> StaticFieldsInvariantRule {
        StaticFieldsInvariantRule::with_threshold(Self::DEFAULT_LEAK_THRESHOLD, true)
    }

    /// `new StaticFieldsInvariantRule(leakThresholdBytes, countSuperclasses)`.
    pub fn with_threshold(
        leak_threshold: i64,
        count_superclasses: bool,
    ) -> StaticFieldsInvariantRule {
        exception_classes();
        StaticFieldsInvariantRule {
            leak_threshold,
            count_superclasses,
            accept: Arc::new(|f| f.get_modifiers() & modifier::FINAL == 0),
        }
    }

    /// Overrides `accept(Field)` (by default, non-final fields count).
    pub fn with_accept(mut self, accept: impl Fn(&Field) -> bool + Send + Sync + 'static) -> Self {
        self.accept = Arc::new(accept);
        self
    }

    fn check(&self, errors: &mut Vec<Throwable>, d: &Description) {
        let test_class: Option<Arc<Class>> =
            try_run(|| RandomizedContext::current().get_target_class_object())
                .ok()
                .flatten()
                .or_else(|| d.test_class().cloned());
        let Some(test_class) = test_class else {
            runtime("Test class could not be acquired from the randomized context or the Description object.")
        };
        let mut entries: Vec<(Arc<Field>, Value, i64)> = Vec::new();
        let mut values: Vec<Value> = Vec::new();
        // Java loops while `countSuperclasses` holds, so without it no
        // field counts at all; the port keeps that.
        let mut c: Option<&Class> = Some(&test_class);
        while let Some(cls) = c {
            if !self.count_superclasses {
                break;
            }
            for field in cls.declared_fields() {
                if field.get_modifiers() & modifier::STATIC != 0
                    && !field.is_primitive()
                    && (self.accept)(field)
                {
                    let v = field.get(None);
                    if !matches!(v, Value::Null) {
                        entries.push((Arc::clone(field), v.clone(), 0));
                        values.push(v);
                    }
                }
            }
            c = cls.superclass().map(|s| s.as_ref());
        }
        let ram_usage = match RamUsageEstimator::size_of_all(&values) {
            Ok(r) => r,
            Err(ex) => {
                let mut b = String::from(
                    "Clean up static fields (in @AfterClass?) and null them, your test still has \
                     references to classes of which the sizes cannot be measured due to security \
                     restrictions or Java 9 module encapsulation:",
                );
                for (f, v, _) in &entries {
                    if RamUsageEstimator::size_of(v).is_err() {
                        b.push_str(&format!("\n  - {}", f.to_java_string()));
                    }
                }
                let mut err = Throwable::new(ASSERTION_FAILED_ERROR, b);
                err.set_cause(ex);
                errors.push(err);
                return;
            }
        };
        if ram_usage > self.leak_threshold {
            for e in entries.iter_mut() {
                e.2 = RamUsageEstimator::size_of(&e.1).unwrap_or(0);
            }
            entries.sort_by(|a, b| {
                b.2.cmp(&a.2).then_with(|| {
                    crate::annotations::jdk_string_compare(
                        &a.0.to_java_string(),
                        &b.0.to_java_string(),
                    )
                })
            });
            let mut b = format!(
                "Clean up static fields (in @AfterClass?), your test seems to hang on to \
                 approximately {} bytes (threshold is {}). Field reference sizes (counted \
                 individually):",
                group(ram_usage),
                group(self.leak_threshold)
            );
            for (f, _, used) in &entries {
                b.push_str(&format!(
                    "\n  - {} bytes, {}",
                    group(*used),
                    f.to_java_string()
                ));
            }
            errors.push(Throwable::new(ASSERTION_FAILED_ERROR, b));
        }
    }
}

impl Default for StaticFieldsInvariantRule {
    fn default() -> Self {
        StaticFieldsInvariantRule::new()
    }
}

fn exception_classes() {
    crate::jdk::exception::register_class(
        ASSERTION_FAILED_ERROR,
        crate::jdk::exception::ASSERTION_ERROR,
    );
}

/// `%,d` under `Locale.ROOT`: digits grouped by three with `,`.
fn group(v: i64) -> String {
    let s = v.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if v < 0 {
        format!("-{out}")
    } else {
        out
    }
}

impl TestRule for StaticFieldsInvariantRule {
    fn apply(&self, s: Statement, d: &Arc<Description>) -> Statement {
        let me = self.clone();
        let d = Arc::clone(d);
        StatementAdapter::new(s)
            .after_always(move |errors| me.check(errors, &d))
            .build()
    }
}
