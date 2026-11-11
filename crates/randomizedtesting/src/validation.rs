//! Port of `com.carrotsearch.randomizedtesting.Validation`
//! (randomizedtesting 2.8.1, `Validation.java`).

use crate::jdk::exception::runtime;
use crate::jdk::reflect::{modifier, Class, Method};

/// Validation utilities, in chained-call style (`Validation`).
pub(crate) struct Validation;

impl Validation {
    /// `checkThat(Method)`.
    pub(crate) fn check_method(m: &Method) -> MethodValidation<'_> {
        MethodValidation {
            m,
            description: format!("Method {}", m.name()),
        }
    }

    /// `checkThat(Class)`.
    pub(crate) fn check_class(c: &Class) -> ClassValidation<'_> {
        ClassValidation {
            clazz: c,
            description: format!("Class {}", c.name()),
        }
    }
}

/// `Class.toString()` of a parameter type: `int` for a primitive, `class
/// <name>` otherwise.
fn type_to_string(t: &str) -> String {
    match t {
        "boolean" | "byte" | "char" | "short" | "int" | "long" | "float" | "double" | "void" => {
            t.to_string()
        }
        _ => format!("class {t}"),
    }
}

/// `Validation.MethodValidation`.
pub(crate) struct MethodValidation<'a> {
    m: &'a Method,
    description: String,
}

#[allow(clippy::wrong_self_convention)] // Java's chained `isPublic()`-style checks.
impl MethodValidation<'_> {
    /// `describedAs(message)`.
    pub(crate) fn described_as(mut self, message: impl Into<String>) -> Self {
        self.description = message.into();
        self
    }

    /// `isPublic()`.
    pub(crate) fn is_public(self) -> Self {
        if self.m.get_modifiers() & modifier::PUBLIC == 0 {
            runtime(format!("{} should be public.", self.description));
        }
        self
    }

    /// `hasArgsCount(argsCount)`.
    pub(crate) fn has_args_count(self, args_count: usize) -> Self {
        if self.m.parameter_types().len() != args_count {
            let types: Vec<String> = self
                .m
                .parameter_types()
                .iter()
                .map(|t| type_to_string(t))
                .collect();
            runtime(format!(
                "{} should have {} parameters and has these: [{}]",
                self.description,
                args_count,
                types.join(", ")
            ));
        }
        self
    }

    /// `isStatic()`.
    pub(crate) fn is_static(self) -> Self {
        if self.m.get_modifiers() & modifier::STATIC == 0 {
            runtime(format!("{} should be static.", self.description));
        }
        self
    }

    /// `isNotStatic()`.
    pub(crate) fn is_not_static(self) -> Self {
        if self.m.get_modifiers() & modifier::STATIC != 0 {
            runtime(format!(
                "{} should be instance method (not static).",
                self.description
            ));
        }
        self
    }

    /// `hasReturnType(clazz)`: the erased return type is `class_name` (the
    /// only assignability the runner asks for, `Iterable`, is checked by
    /// name).
    pub(crate) fn has_return_type(self, class_name: &str) -> Self {
        let rt = self.m.raw_return_type();
        let assignable = rt == class_name
            || (class_name == "java.lang.Iterable"
                && matches!(
                    rt,
                    "java.util.Collection"
                        | "java.util.List"
                        | "java.util.ArrayList"
                        | "java.util.Set"
                        | "java.util.LinkedList"
                ));
        if !assignable {
            runtime(format!(
                "{} should have a return type assignable to: {}",
                self.description, class_name
            ));
        }
        self
    }
}

/// `Validation.ClassValidation`.
pub(crate) struct ClassValidation<'a> {
    clazz: &'a Class,
    description: String,
}

#[allow(clippy::wrong_self_convention)] // Java's chained `isPublic()`-style checks.
impl ClassValidation<'_> {
    /// `describedAs(message)`.
    pub(crate) fn described_as(mut self, message: impl Into<String>) -> Self {
        self.description = message.into();
        self
    }

    /// `isPublic()`.
    pub(crate) fn is_public(self) -> Self {
        if self.clazz.get_modifiers() & modifier::PUBLIC == 0 {
            runtime(format!("{} should be public.", self.description));
        }
        self
    }

    /// `isConcreteClass()`.
    pub(crate) fn is_concrete_class(self) -> Self {
        if self.clazz.get_modifiers() & modifier::INTERFACE != 0 {
            runtime(format!(
                "{} should be a conrete class (not an interface).",
                self.description
            ));
        }
        if self.clazz.get_modifiers() & modifier::ABSTRACT != 0 {
            runtime(format!(
                "{} should be a concrete class (not abstract).",
                self.description
            ));
        }
        self
    }

    /// `hasPublicNoArgsConstructor()`.
    #[allow(dead_code)] // Java declares it; the runner never calls it.
    pub(crate) fn has_public_no_args_constructor(self) {
        if !self
            .clazz
            .constructors()
            .iter()
            .any(|c| c.parameter_types().is_empty())
        {
            runtime(format!(
                "{} should have a public parameterless constructor.",
                self.description
            ));
        }
    }
}
