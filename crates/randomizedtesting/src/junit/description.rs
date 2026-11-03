//! `org.junit.runner.Description`, rebuilt from its documented behaviour.

use std::fmt;
use std::sync::{Arc, Mutex};

use crate::jdk::exception::illegal_argument;
use crate::jdk::reflect::{find_annotation, AnnotationRef, Class, JavaAnnotation};

/// A description of a test or of a suite of tests (`Description`): a
/// display name, a unique id (equality), the test class, annotations and
/// child descriptions.
///
/// Shared as `Arc<Description>`; two descriptions are equal when their
/// unique ids are, as JUnit's `equals` states.
pub struct Description {
    display_name: String,
    unique_id: String,
    test_class: Option<Arc<Class>>,
    annotations: Vec<AnnotationRef>,
    children: Mutex<Vec<Arc<Description>>>,
}

impl fmt::Debug for Description {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display_name)
    }
}

impl fmt::Display for Description {
    /// `toString()`: the display name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display_name)
    }
}

impl PartialEq for Description {
    fn eq(&self, other: &Description) -> bool {
        self.unique_id == other.unique_id
    }
}

impl Eq for Description {}

impl Description {
    fn make(
        test_class: Option<Arc<Class>>,
        display_name: String,
        unique_id: String,
        annotations: Vec<AnnotationRef>,
    ) -> Arc<Description> {
        if display_name.is_empty() {
            illegal_argument("The display name must not be empty.");
        }
        Arc::new(Description {
            display_name,
            unique_id,
            test_class,
            annotations,
            children: Mutex::new(Vec::new()),
        })
    }

    /// `createSuiteDescription(String name, Annotation... annotations)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`name must have non-zero length`) for an
    /// empty name.
    pub fn create_suite_description(
        name: &str,
        annotations: Vec<AnnotationRef>,
    ) -> Arc<Description> {
        if name.is_empty() {
            illegal_argument("name must have non-zero length");
        }
        Description::make(None, name.to_string(), name.to_string(), annotations)
    }

    /// `createSuiteDescription(Class<?> testClass)`: named after the class,
    /// with its annotations.
    pub fn create_suite_description_for_class(class: &Arc<Class>) -> Arc<Description> {
        Description::make(
            Some(Arc::clone(class)),
            class.name().to_string(),
            class.name().to_string(),
            class.annotations(),
        )
    }

    /// `createTestDescription(Class<?> clazz, String name, Annotation...)`:
    /// `name(className)`.
    pub fn create_test_description(
        class: &Arc<Class>,
        name: &str,
        annotations: Vec<AnnotationRef>,
    ) -> Arc<Description> {
        let display = format!("{}({})", name, class.name());
        Description::make(
            Some(Arc::clone(class)),
            display.clone(),
            display,
            annotations,
        )
    }

    /// `createTestDescription(String className, String name, Annotation...)`.
    pub fn create_test_description_named(class_name: &str, name: &str) -> Arc<Description> {
        let display = format!("{name}({class_name})");
        Description::make(None, display.clone(), display, Vec::new())
    }

    /// `getDisplayName()`.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// `addChild(description)`.
    pub fn add_child(&self, description: Arc<Description>) {
        self.children
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(description);
    }

    /// `getChildren()`: a copy of the children list (clearing it does not
    /// change this description).
    pub fn children(&self) -> Vec<Arc<Description>> {
        self.children
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// `isTest()`: no children.
    pub fn is_test(&self) -> bool {
        self.children
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }

    /// `isSuite()`: `!isTest()`.
    pub fn is_suite(&self) -> bool {
        !self.is_test()
    }

    /// `testCount()`: the number of leaf descriptions.
    pub fn test_count(&self) -> usize {
        if self.is_test() {
            return 1;
        }
        self.children().iter().map(|c| c.test_count()).sum()
    }

    /// `childlessCopy()`: the same class, display name and annotations (the
    /// unique id becomes the display name).
    pub fn childless_copy(&self) -> Arc<Description> {
        Description::make(
            self.test_class.clone(),
            self.display_name.clone(),
            self.display_name.clone(),
            self.annotations.clone(),
        )
    }

    /// `getAnnotations()`.
    pub fn annotations(&self) -> &[AnnotationRef] {
        &self.annotations
    }

    /// `getAnnotation(A.class)`.
    pub fn get_annotation<A: JavaAnnotation>(&self) -> Option<&A> {
        find_annotation(&self.annotations)
    }

    /// `getTestClass()`: the class the description was created with.
    ///
    /// **Divergence.** JUnit also loads the class named by `getClassName()`
    /// (`Class.forName`) when the description has none; a Rust program
    /// cannot load a class by name, so such a description has no class.
    pub fn test_class(&self) -> Option<&Arc<Class>> {
        self.test_class.as_ref()
    }

    /// The `method(class)` split of the display name, as JUnit's pattern
    /// `([\s\S]*)\((.*)\)` matches it: the text up to the last `(` and the
    /// text between it and a final `)`, which holds no line terminator.
    fn method_and_class(&self) -> Option<(&str, &str)> {
        let s = self.display_name.as_str();
        let body = s.strip_suffix(')')?;
        let open = body.rfind('(')?;
        let class = &body[open + 1..];
        if class
            .chars()
            .any(|c| matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'))
        {
            return None;
        }
        Some((&body[..open], class))
    }

    /// `getClassName()`: the test class's name, else the class part of the
    /// display name, else the display name.
    pub fn class_name(&self) -> Option<String> {
        if let Some(c) = &self.test_class {
            return Some(c.name().to_string());
        }
        Some(
            self.method_and_class()
                .map(|(_, c)| c.to_string())
                .unwrap_or_else(|| self.display_name.clone()),
        )
    }

    /// `getMethodName()`: the method part of the display name, or `None`.
    pub fn method_name(&self) -> Option<String> {
        self.method_and_class().map(|(m, _)| m.to_string())
    }
}
