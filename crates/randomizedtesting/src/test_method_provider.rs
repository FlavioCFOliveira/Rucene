//! Port of `com.carrotsearch.randomizedtesting.TestMethodProvider`,
//! `AnnotatedMethodProvider`, `JUnit4MethodProvider` and
//! `JUnit3MethodProvider` (randomizedtesting 2.8.1).

use std::sync::Arc;

use crate::annotations::AnnotationType;
use crate::class_model::ClassModel;
use crate::jdk::reflect::{Class, Method};
use crate::junit::Test;

/// Responsible for providing individual test instances and their
/// descriptions: `TestMethodProvider`.
pub trait TestMethodProvider: Send + Sync {
    /// `getTestMethods(suiteClass, suiteClassModel)`: every method that is
    /// a test, valid or not (the runner validates them).
    fn get_test_methods(
        &self,
        suite_class: &Class,
        suite_class_model: &ClassModel,
    ) -> Vec<Arc<Method>>;
}

/// Methods annotated with a given annotation: `AnnotatedMethodProvider`.
#[derive(Debug, Clone, Copy)]
pub struct AnnotatedMethodProvider {
    annotation: &'static AnnotationType,
}

impl AnnotatedMethodProvider {
    /// `new AnnotatedMethodProvider(annotation)`.
    pub fn new(annotation: &'static AnnotationType) -> AnnotatedMethodProvider {
        AnnotatedMethodProvider { annotation }
    }
}

static TEST_TYPE: AnnotationType = Test::TYPE;

impl TestMethodProvider for AnnotatedMethodProvider {
    /// The leaf methods with the annotation; for `@Test`, which JUnit
    /// treats as inherited, the most specific override of each.
    fn get_test_methods(&self, _suite_class: &Class, model: &ClassModel) -> Vec<Arc<Method>> {
        let methods = model.get_annotated_leaf_methods(self.annotation);
        if self.annotation.name == TEST_TYPE.name {
            methods
                .into_iter()
                .map(|mut mm| {
                    while let Some(d) = model.down(mm) {
                        mm = d;
                    }
                    Arc::clone(&mm.element)
                })
                .collect()
        } else {
            methods
                .into_iter()
                .map(|mm| Arc::clone(&mm.element))
                .collect()
        }
    }
}

/// `@Test`-annotated methods: `JUnit4MethodProvider`, the default.
#[derive(Debug, Clone, Copy, Default)]
pub struct JUnit4MethodProvider;

impl TestMethodProvider for JUnit4MethodProvider {
    fn get_test_methods(&self, suite_class: &Class, model: &ClassModel) -> Vec<Arc<Method>> {
        AnnotatedMethodProvider::new(&TEST_TYPE).get_test_methods(suite_class, model)
    }
}

/// Methods named `test*`, the most specific override of each:
/// `JUnit3MethodProvider`.
#[derive(Debug, Clone, Copy, Default)]
pub struct JUnit3MethodProvider;

impl TestMethodProvider for JUnit3MethodProvider {
    fn get_test_methods(&self, _suite_class: &Class, model: &ClassModel) -> Vec<Arc<Method>> {
        model
            .methods()
            .iter()
            .filter(|mm| model.down(mm).is_none() && mm.element.name().starts_with("test"))
            .map(|mm| Arc::clone(&mm.element))
            .collect()
    }
}
