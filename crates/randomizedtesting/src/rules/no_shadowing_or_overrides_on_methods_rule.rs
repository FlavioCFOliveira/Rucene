//! Port of `com.carrotsearch.randomizedtesting.rules.NoShadowingOrOverridesOnMethodsRule`,
//! `NoClassHooksShadowingRule` and `NoInstanceHooksOverridesRule`
//! (randomizedtesting 2.8.1).

use std::sync::Arc;

use crate::annotations::AnnotationType;
use crate::class_model::ClassModel;
use crate::jdk::exception::{runtime, Throwable};
use crate::jdk::reflect::{Class, Method};
use crate::junit::{After, AfterClass, Before, BeforeClass, Description, Statement, TestRule};
use crate::randomized_context::RandomizedContext;
use crate::randomized_runner::try_run;

/// Fails when methods annotated with any of the given annotations shadow
/// or override each other (`NoShadowingOrOverridesOnMethodsRule`).
#[derive(Clone)]
pub struct NoShadowingOrOverridesOnMethodsRule {
    annotations: Vec<&'static AnnotationType>,
    verify: Arc<dyn Fn(&Method) -> bool + Send + Sync>,
}

impl NoShadowingOrOverridesOnMethodsRule {
    /// `new NoShadowingOrOverridesOnMethodsRule(annotations...)`.
    pub fn new(annotations: Vec<&'static AnnotationType>) -> NoShadowingOrOverridesOnMethodsRule {
        NoShadowingOrOverridesOnMethodsRule {
            annotations,
            verify: Arc::new(|_| true),
        }
    }

    /// Overrides `verify(Method)`: `false` skips a method.
    pub fn with_verify(mut self, verify: impl Fn(&Method) -> bool + Send + Sync + 'static) -> Self {
        self.verify = Arc::new(verify);
        self
    }

    /// `validate(clazz)`.
    ///
    /// # Panics
    ///
    /// `RuntimeException` listing the overriding or shadowing methods.
    pub fn validate(&self, clazz: &Class) {
        let model = ClassModel::new(clazz);
        for ann in &self.annotations {
            self.check_no_shadows_or_overrides(&model, ann);
        }
    }

    fn check_no_shadows_or_overrides(&self, model: &ClassModel, ann: &AnnotationType) {
        let mut b = String::new();
        for mm in model.get_annotated_leaf_methods(ann) {
            if (self.verify)(&mm.element) && (model.down(mm).is_some() || model.up(mm).is_some()) {
                b.push_str(&format!(
                    "Methods annotated with @{} shadow or override each other:\n",
                    ann.name
                ));
                let mut m = mm;
                while let Some(u) = model.up(m) {
                    m = u;
                }
                let mut cur = Some(m);
                while let Some(x) = cur {
                    b.push_str("  - ");
                    if x.element.is_annotation_present(ann) {
                        b.push('@');
                        b.push_str(ann.simple_name);
                        b.push(' ');
                    }
                    b.push_str(&x.element.to_java_string());
                    b.push('\n');
                    cur = model.down(x);
                }
            }
        }
        if !b.is_empty() {
            runtime(format!(
                "There are overridden methods annotated with {}. These methods would not be \
                 executed by JUnit and need to manually chain themselves which can lead to \
                 maintenance problems. Consider using different method names or make hook \
                 methods private.\n{}",
                ann.name,
                crate::jdk::lang::trim(&b)
            ));
        }
    }
}

impl TestRule for NoShadowingOrOverridesOnMethodsRule {
    /// Validates the context's target class (else the description's) before
    /// the base statement.
    fn apply(&self, base: Statement, description: &Arc<Description>) -> Statement {
        let me = self.clone();
        let d = Arc::clone(description);
        Statement::new(move || {
            let test_class = try_run(|| RandomizedContext::current().get_target_class_object())
                .ok()
                .flatten()
                .or_else(|| d.test_class().cloned());
            match test_class {
                Some(c) => me.validate(&c),
                None => {
                    Throwable::with_message(crate::jdk::exception::NULL_POINTER_EXCEPTION, None)
                        .throw()
                }
            }
            base.evaluate();
        })
    }
}

static BEFORE_CLASS: AnnotationType = BeforeClass::TYPE;
static AFTER_CLASS: AnnotationType = AfterClass::TYPE;
static BEFORE: AnnotationType = Before::TYPE;
static AFTER: AnnotationType = After::TYPE;

/// Forbids `@BeforeClass` / `@AfterClass` shadowing
/// (`NoClassHooksShadowingRule`).
pub struct NoClassHooksShadowingRule;

impl NoClassHooksShadowingRule {
    /// `new NoClassHooksShadowingRule()`.
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> NoShadowingOrOverridesOnMethodsRule {
        NoShadowingOrOverridesOnMethodsRule::new(vec![&BEFORE_CLASS, &AFTER_CLASS])
    }
}

/// Forbids `@Before` / `@After` overrides (`NoInstanceHooksOverridesRule`).
pub struct NoInstanceHooksOverridesRule;

impl NoInstanceHooksOverridesRule {
    /// `new NoInstanceHooksOverridesRule()`.
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> NoShadowingOrOverridesOnMethodsRule {
        NoShadowingOrOverridesOnMethodsRule::new(vec![&BEFORE, &AFTER])
    }
}
