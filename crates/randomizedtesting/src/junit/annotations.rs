//! The JUnit 4 annotations the runner reads: `@Test`, `@Before`, `@After`,
//! `@BeforeClass`, `@AfterClass`, `@Ignore`, `@Rule` and `@ClassRule`.

use std::any::Any;

use crate::jdk::reflect::{quote_member, AnnotationType, JavaAnnotation};

macro_rules! marker {
    ($(#[$doc:meta])* $name:ident, $ty:ident, $java:literal, $simple:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, Default)]
        pub struct $name;

        impl $name {
            /// The annotation type.
            pub const TYPE: AnnotationType = AnnotationType::plain($java, $simple, false);
        }

        static $ty: AnnotationType = $name::TYPE;

        impl JavaAnnotation for $name {
            fn annotation_type(&self) -> &'static AnnotationType {
                &$ty
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
        }
    };
}

marker!(
    /// `@org.junit.Before`.
    Before, BEFORE, "org.junit.Before", "Before"
);
marker!(
    /// `@org.junit.After`.
    After, AFTER, "org.junit.After", "After"
);
marker!(
    /// `@org.junit.BeforeClass`.
    BeforeClass, BEFORE_CLASS, "org.junit.BeforeClass", "BeforeClass"
);
marker!(
    /// `@org.junit.AfterClass`.
    AfterClass, AFTER_CLASS, "org.junit.AfterClass", "AfterClass"
);
marker!(
    /// `@org.junit.Rule`.
    Rule, RULE, "org.junit.Rule", "Rule"
);
marker!(
    /// `@org.junit.ClassRule`.
    ClassRule, CLASS_RULE, "org.junit.ClassRule", "ClassRule"
);

/// `@org.junit.Test(expected = …, timeout = …)`.
#[derive(Debug, Clone, Copy)]
pub struct Test {
    /// `expected()`: the binary name of the expected throwable class, or
    /// `None` for `Test.None`.
    pub expected: Option<&'static str>,
    /// `timeout()` in milliseconds; `0` for none.
    pub timeout: i64,
}

impl Test {
    /// The annotation type.
    pub const TYPE: AnnotationType = AnnotationType::plain("org.junit.Test", "Test", false);

    /// `@Test`.
    pub const fn new() -> Test {
        Test {
            expected: None,
            timeout: 0,
        }
    }

    /// `@Test(expected = class)`.
    pub const fn expected(class: &'static str) -> Test {
        Test {
            expected: Some(class),
            timeout: 0,
        }
    }

    /// `@Test(timeout = millis)`.
    pub const fn timeout(millis: i64) -> Test {
        Test {
            expected: None,
            timeout: millis,
        }
    }
}

impl Default for Test {
    fn default() -> Self {
        Test::new()
    }
}

static TEST: AnnotationType = Test::TYPE;

impl JavaAnnotation for Test {
    fn annotation_type(&self) -> &'static AnnotationType {
        &TEST
    }
    fn to_java_string(&self) -> String {
        format!(
            "@org.junit.Test(expected={}.class, timeout={}L)",
            self.expected
                .map(|c| c.replace('$', "."))
                .unwrap_or_else(|| "org.junit.Test.None".to_string()),
            self.timeout
        )
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `@org.junit.Ignore(value)`.
#[derive(Debug, Clone, Default)]
pub struct Ignore {
    /// `value()`: the reason.
    pub value: String,
}

impl Ignore {
    /// The annotation type.
    pub const TYPE: AnnotationType = AnnotationType::plain("org.junit.Ignore", "Ignore", false);

    /// `@Ignore`.
    pub fn new() -> Ignore {
        Ignore::default()
    }

    /// `@Ignore(reason)`.
    pub fn because(reason: impl Into<String>) -> Ignore {
        Ignore {
            value: reason.into(),
        }
    }
}

static IGNORE: AnnotationType = Ignore::TYPE;

impl JavaAnnotation for Ignore {
    fn annotation_type(&self) -> &'static AnnotationType {
        &IGNORE
    }
    fn to_java_string(&self) -> String {
        format!("@org.junit.Ignore({})", quote_member(&self.value))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
