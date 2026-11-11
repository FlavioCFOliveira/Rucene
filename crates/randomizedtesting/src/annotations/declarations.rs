//! Port of the runner's annotations in
//! `com.carrotsearch.randomizedtesting.annotations` (randomizedtesting
//! 2.8.1): `Repeat`, `Seed`, `Seeds`, `Name`, `ParametersFactory`,
//! `TestCaseOrdering`, `TestCaseInstanceProvider`, `Listeners`,
//! `TestMethodProviders`, `SeedDecorators`, `TestContextRandomSupplier`,
//! `ReplicateOnEachVm`, `SuppressForbidden`, `ThreadLeakAction`,
//! `ThreadLeakFilters`, `ThreadLeakGroup`, `ThreadLeakLingering`,
//! `ThreadLeakScope`, `ThreadLeakZombies`, `Timeout` and `TimeoutSuite`
//! (one Java file each, `annotations/<Name>.java`).
//!
//! Each annotation is a Rust value with the Java members as fields and the
//! Java defaults in `Default`; a `Class<? extends X>` member is a factory
//! that makes the instance `clazz.newInstance()` would, with the class's
//! binary name for messages.

use std::any::Any;
use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use super::AnnotationType;
use crate::jdk::exception::Throwable;
use crate::jdk::reflect::{quote_member, JavaAnnotation, Method, Value};
use crate::junit::RunListener;
use crate::random_supplier::RandomSupplier;
use crate::seed_decorator::SeedDecorator;
use crate::test_method_provider::TestMethodProvider;
use crate::thread_filter::ThreadFilter;

/// `Class<? extends T>`: a class name and its `newInstance()`, which may
/// throw.
pub struct ClassRef<T: ?Sized> {
    /// `Class.getName()`.
    pub name: &'static str,
    make: Arc<dyn Fn() -> Result<Arc<T>, Throwable> + Send + Sync>,
}

impl<T: ?Sized> Clone for ClassRef<T> {
    fn clone(&self) -> Self {
        ClassRef {
            name: self.name,
            make: Arc::clone(&self.make),
        }
    }
}

impl<T: ?Sized> fmt::Debug for ClassRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.class", self.name)
    }
}

impl<T: ?Sized> ClassRef<T> {
    /// A class named `name` whose public no-argument constructor runs
    /// `make`.
    pub fn new(
        name: &'static str,
        make: impl Fn() -> Arc<T> + Send + Sync + 'static,
    ) -> ClassRef<T> {
        ClassRef {
            name,
            make: Arc::new(move || Ok(make())),
        }
    }

    /// A class whose `newInstance()` fails with `error` (for example one
    /// without a public no-argument constructor).
    pub fn failing(
        name: &'static str,
        error: impl Fn() -> Throwable + Send + Sync + 'static,
    ) -> ClassRef<T> {
        ClassRef {
            name,
            make: Arc::new(move || Err(error())),
        }
    }

    /// `newInstance()`.
    pub fn new_instance(&self) -> Result<Arc<T>, Throwable> {
        (self.make)()
    }

    /// `Class.toString()`: `class <name>`.
    pub fn class_to_string(&self) -> String {
        format!("class {}", self.name)
    }
}

macro_rules! annotation_type {
    ($name:ident, $java:literal, $simple:literal, $inherited:literal) => {
        impl $name {
            /// The annotation type.
            pub const TYPE: AnnotationType = AnnotationType::plain(
                concat!("com.carrotsearch.randomizedtesting.annotations.", $java),
                $simple,
                $inherited,
            );
        }
    };
}

macro_rules! java_annotation {
    ($name:ident, $static:ident, |$s:ident| $to_string:expr) => {
        static $static: AnnotationType = $name::TYPE;

        impl JavaAnnotation for $name {
            fn annotation_type(&self) -> &'static AnnotationType {
                &$static
            }
            fn to_java_string(&self) -> String {
                let $s = self;
                $to_string
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
        }
    };
}

fn prefix(simple: &str) -> String {
    format!("@com.carrotsearch.randomizedtesting.annotations.{simple}")
}

/// Port of `com.carrotsearch.randomizedtesting.annotations.Repeat`.
///
/// `@Repeat(iterations, useConstantSeed)`: repeats a test method (or every
/// test of a class) with the same or different seeds.
#[derive(Debug, Clone, Copy)]
pub struct Repeat {
    /// `iterations()`, default 1.
    pub iterations: i32,
    /// `useConstantSeed()`, default false.
    pub use_constant_seed: bool,
}

impl Default for Repeat {
    fn default() -> Self {
        Repeat {
            iterations: 1,
            use_constant_seed: false,
        }
    }
}

annotation_type!(Repeat, "Repeat", "Repeat", true);
java_annotation!(Repeat, REPEAT, |s| format!(
    "{}(iterations={}, useConstantSeed={})",
    prefix("Repeat"),
    s.iterations,
    s.use_constant_seed
));

/// Port of `com.carrotsearch.randomizedtesting.annotations.Seed`.
///
/// `@Seed(value)`: the seed (chain) of a suite or method; `random` for a
/// random one.
#[derive(Debug, Clone)]
pub struct Seed {
    /// `value()`, default `random`.
    pub value: String,
}

impl Seed {
    /// `@Seed(value)`.
    pub fn new(value: impl Into<String>) -> Seed {
        Seed {
            value: value.into(),
        }
    }
}

impl Default for Seed {
    fn default() -> Self {
        Seed::new("random")
    }
}

annotation_type!(Seed, "Seed", "Seed", true);
java_annotation!(Seed, SEED, |s| format!(
    "{}({})",
    prefix("Seed"),
    quote_member(&s.value)
));

/// Port of `com.carrotsearch.randomizedtesting.annotations.Seeds`.
///
/// `@Seeds({@Seed(...), ...})`: several seeds for one method.
#[derive(Debug, Clone, Default)]
pub struct Seeds {
    /// `value()`.
    pub value: Vec<Seed>,
}

impl Seeds {
    /// `@Seeds({@Seed(v1), @Seed(v2), …})`.
    pub fn new(values: &[&str]) -> Seeds {
        Seeds {
            value: values.iter().map(|v| Seed::new(*v)).collect(),
        }
    }
}

annotation_type!(Seeds, "Seeds", "Seeds", true);
java_annotation!(Seeds, SEEDS, |s| format!(
    "{}({{{}}})",
    prefix("Seeds"),
    s.value
        .iter()
        .map(|x| x.to_java_string())
        .collect::<Vec<_>>()
        .join(", ")
));

/// Port of `com.carrotsearch.randomizedtesting.annotations.Name`.
///
/// `@Name(value)`: the name of a constructor parameter in a parameterized
/// test's description. On the Rust side it is the name given to
/// [`Constructor::with_params`](crate::jdk::reflect::Constructor::with_params).
#[derive(Debug, Clone)]
pub struct Name {
    /// `value()`.
    pub value: String,
}

annotation_type!(Name, "Name", "Name", false);
java_annotation!(Name, NAME, |s| format!(
    "{}({})",
    prefix("Name"),
    quote_member(&s.value)
));

/// Port of `com.carrotsearch.randomizedtesting.annotations.ParametersFactory`.
///
/// `@ParametersFactory(argumentFormatting, shuffle)`: a static method
/// returning the constructor arguments of a parameterized suite.
#[derive(Debug, Clone)]
pub struct ParametersFactory {
    /// `argumentFormatting()`, default [`ParametersFactory::DEFAULT_FORMATTING`].
    pub argument_formatting: String,
    /// `shuffle()`, default true.
    pub shuffle: bool,
}

impl ParametersFactory {
    /// `ParametersFactory.DEFAULT_FORMATTING`.
    pub const DEFAULT_FORMATTING: &'static str = "default";
}

impl Default for ParametersFactory {
    fn default() -> Self {
        ParametersFactory {
            argument_formatting: ParametersFactory::DEFAULT_FORMATTING.to_string(),
            shuffle: true,
        }
    }
}

annotation_type!(
    ParametersFactory,
    "ParametersFactory",
    "ParametersFactory",
    false
);
java_annotation!(ParametersFactory, PARAMETERS_FACTORY, |s| format!(
    "{}(argumentFormatting={}, shuffle={})",
    prefix("ParametersFactory"),
    quote_member(&s.argument_formatting),
    s.shuffle
));

/// Port of `com.carrotsearch.randomizedtesting.TestMethodAndParams`.
///
/// `TestMethodAndParams`: a test method with its instance arguments, as
/// a [`TestCaseOrdering`] comparator sees it.
pub trait TestMethodAndParams {
    /// `getTestMethod()`.
    fn test_method(&self) -> &Arc<Method>;
    /// `getInstanceArguments()`.
    fn instance_arguments(&self) -> &[Value];
}

/// `Comparator<TestMethodAndParams>`.
pub type TestCaseComparator =
    Arc<dyn Fn(&dyn TestMethodAndParams, &dyn TestMethodAndParams) -> Ordering + Send + Sync>;

/// Port of `com.carrotsearch.randomizedtesting.annotations.TestCaseOrdering`.
///
/// `@TestCaseOrdering(comparatorClass)`: the order of test cases.
#[derive(Clone)]
pub struct TestCaseOrdering {
    /// `value()`: the comparator class.
    pub value: ClassRef<TestCaseComparatorBox>,
}

/// A comparator instance (`Comparator<TestMethodAndParams>`).
pub struct TestCaseComparatorBox(pub TestCaseComparator);

impl fmt::Debug for TestCaseOrdering {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@TestCaseOrdering({:?})", self.value)
    }
}

impl TestCaseOrdering {
    /// `TestCaseOrdering.AlphabeticOrder`: by test method name.
    pub fn alphabetic_order() -> TestCaseOrdering {
        TestCaseOrdering {
            value: ClassRef::new(
                "com.carrotsearch.randomizedtesting.annotations.TestCaseOrdering$AlphabeticOrder",
                || {
                    Arc::new(TestCaseComparatorBox(Arc::new(|a, b| {
                        jdk_string_compare(a.test_method().name(), b.test_method().name())
                    })))
                },
            ),
        }
    }
}

/// `String.compareTo`: by UTF-16 code units.
pub fn jdk_string_compare(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

annotation_type!(
    TestCaseOrdering,
    "TestCaseOrdering",
    "TestCaseOrdering",
    true
);
java_annotation!(TestCaseOrdering, TEST_CASE_ORDERING, |s| format!(
    "{}({}.class)",
    prefix("TestCaseOrdering"),
    s.value.name.replace('$', ".")
));

/// Port of `com.carrotsearch.randomizedtesting.annotations.TestCaseInstanceProvider.Type`.
///
/// `TestCaseInstanceProvider.Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InstanceProviderType {
    /// `INSTANCE_PER_TEST_METHOD` (the default).
    #[default]
    InstancePerTestMethod,
    /// `INSTANCE_PER_CONSTRUCTOR_ARGS`.
    InstancePerConstructorArgs,
}

/// Port of `com.carrotsearch.randomizedtesting.annotations.TestCaseInstanceProvider`.
///
/// `@TestCaseInstanceProvider(type)`.
#[derive(Debug, Clone, Copy, Default)]
pub struct TestCaseInstanceProvider {
    /// `value()`.
    pub value: InstanceProviderType,
}

annotation_type!(
    TestCaseInstanceProvider,
    "TestCaseInstanceProvider",
    "TestCaseInstanceProvider",
    true
);
java_annotation!(
    TestCaseInstanceProvider,
    TEST_CASE_INSTANCE_PROVIDER,
    |s| format!(
        "{}({})",
        prefix("TestCaseInstanceProvider"),
        match s.value {
            InstanceProviderType::InstancePerTestMethod => "INSTANCE_PER_TEST_METHOD",
            InstanceProviderType::InstancePerConstructorArgs => "INSTANCE_PER_CONSTRUCTOR_ARGS",
        }
    )
);

fn class_list<T: ?Sized>(v: &[ClassRef<T>]) -> String {
    v.iter()
        .map(|c| format!("{}.class", c.name.replace('$', ".")))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Port of `com.carrotsearch.randomizedtesting.annotations.Listeners`.
///
/// `@Listeners({listenerClasses})`: listeners the runner adds.
#[derive(Debug, Clone, Default)]
pub struct Listeners {
    /// `value()`.
    pub value: Vec<ClassRef<dyn RunListener>>,
}

annotation_type!(Listeners, "Listeners", "Listeners", false);
java_annotation!(Listeners, LISTENERS, |s| format!(
    "{}({{{}}})",
    prefix("Listeners"),
    class_list(&s.value)
));

/// Port of `com.carrotsearch.randomizedtesting.annotations.TestMethodProviders`.
///
/// `@TestMethodProviders({providerClasses})`.
#[derive(Debug, Clone, Default)]
pub struct TestMethodProviders {
    /// `value()`.
    pub value: Vec<ClassRef<dyn TestMethodProvider>>,
}

annotation_type!(
    TestMethodProviders,
    "TestMethodProviders",
    "TestMethodProviders",
    true
);
java_annotation!(TestMethodProviders, TEST_METHOD_PROVIDERS, |s| format!(
    "{}({{{}}})",
    prefix("TestMethodProviders"),
    class_list(&s.value)
));

/// Port of `com.carrotsearch.randomizedtesting.annotations.SeedDecorators`.
///
/// `@SeedDecorators({decoratorClasses})`.
#[derive(Debug, Clone, Default)]
pub struct SeedDecorators {
    /// `value()`.
    pub value: Vec<SeedDecoratorClass>,
}

/// `Class<? extends SeedDecorator>`: its name and `newInstance()`.
#[derive(Clone)]
pub struct SeedDecoratorClass {
    /// `Class.getName()`.
    pub name: &'static str,
    make: Arc<dyn Fn() -> Result<Box<dyn SeedDecorator>, Throwable> + Send + Sync>,
}

impl fmt::Debug for SeedDecoratorClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.class", self.name)
    }
}

impl SeedDecoratorClass {
    /// A decorator class whose public no-argument constructor runs `make`.
    pub fn new(
        name: &'static str,
        make: impl Fn() -> Box<dyn SeedDecorator> + Send + Sync + 'static,
    ) -> SeedDecoratorClass {
        SeedDecoratorClass {
            name,
            make: Arc::new(move || Ok(make())),
        }
    }

    /// A decorator class whose `newInstance()` fails with `error`.
    pub fn failing(
        name: &'static str,
        error: impl Fn() -> Throwable + Send + Sync + 'static,
    ) -> SeedDecoratorClass {
        SeedDecoratorClass {
            name,
            make: Arc::new(move || Err(error())),
        }
    }

    /// `MixWithSuiteName.class`.
    pub fn mix_with_suite_name() -> SeedDecoratorClass {
        SeedDecoratorClass::new(
            "com.carrotsearch.randomizedtesting.MixWithSuiteName",
            || Box::new(crate::MixWithSuiteName::new()),
        )
    }

    /// `newInstance()`.
    pub fn new_instance(&self) -> Result<Box<dyn SeedDecorator>, Throwable> {
        (self.make)()
    }
}

annotation_type!(SeedDecorators, "SeedDecorators", "SeedDecorators", false);
java_annotation!(SeedDecorators, SEED_DECORATORS, |s| format!(
    "{}({{{}}})",
    prefix("SeedDecorators"),
    s.value
        .iter()
        .map(|c| format!("{}.class", c.name.replace('$', ".")))
        .collect::<Vec<_>>()
        .join(", ")
));

/// Port of `com.carrotsearch.randomizedtesting.annotations.TestContextRandomSupplier`.
///
/// `@TestContextRandomSupplier(supplierClass)`.
#[derive(Debug, Clone)]
pub struct TestContextRandomSupplier {
    /// `value()`.
    pub value: ClassRef<dyn RandomSupplier>,
}

annotation_type!(
    TestContextRandomSupplier,
    "TestContextRandomSupplier",
    "TestContextRandomSupplier",
    true
);
java_annotation!(
    TestContextRandomSupplier,
    TEST_CONTEXT_RANDOM_SUPPLIER,
    |s| format!(
        "{}({}.class)",
        prefix("TestContextRandomSupplier"),
        s.value.name.replace('$', ".")
    )
);

/// Port of `com.carrotsearch.randomizedtesting.annotations.ReplicateOnEachVm`.
///
/// `@ReplicateOnEachVm`: a marker for the external runner.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReplicateOnEachVm;

annotation_type!(
    ReplicateOnEachVm,
    "ReplicateOnEachVm",
    "ReplicateOnEachVm",
    true
);
java_annotation!(ReplicateOnEachVm, REPLICATE_ON_EACH_VM, |_s| format!(
    "{}()",
    prefix("ReplicateOnEachVm")
));

/// Port of `com.carrotsearch.randomizedtesting.annotations.SuppressForbidden`.
///
/// `@SuppressForbidden(value)`: a marker for the forbidden-APIs checker.
///
/// **Divergence.** Java retains it in class files only
/// (`RetentionPolicy.CLASS`), so reflection never sees it; the port keeps
/// it as an ordinary value that the runner never reads.
#[derive(Debug, Clone, Default)]
pub struct SuppressForbidden {
    /// `value()`: the justification.
    pub value: String,
}

annotation_type!(
    SuppressForbidden,
    "SuppressForbidden",
    "SuppressForbidden",
    false
);
java_annotation!(SuppressForbidden, SUPPRESS_FORBIDDEN, |s| format!(
    "{}({})",
    prefix("SuppressForbidden"),
    quote_member(&s.value)
));

/// Port of `com.carrotsearch.randomizedtesting.annotations.ThreadLeakAction.Action`.
///
/// `ThreadLeakAction.Action`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Action {
    /// `WARN`: log the leak.
    Warn,
    /// `INTERRUPT`: interrupt the leaked threads.
    Interrupt,
}

/// Port of `com.carrotsearch.randomizedtesting.annotations.ThreadLeakAction`.
///
/// `@ThreadLeakAction({actions})`.
#[derive(Debug, Clone)]
pub struct ThreadLeakAction {
    /// `value()`, default `{WARN, INTERRUPT}`.
    pub value: Vec<Action>,
}

impl Default for ThreadLeakAction {
    fn default() -> Self {
        ThreadLeakAction {
            value: vec![Action::Warn, Action::Interrupt],
        }
    }
}

annotation_type!(
    ThreadLeakAction,
    "ThreadLeakAction",
    "ThreadLeakAction",
    true
);
java_annotation!(ThreadLeakAction, THREAD_LEAK_ACTION, |s| format!(
    "{}({{{}}})",
    prefix("ThreadLeakAction"),
    s.value
        .iter()
        .map(|a| match a {
            Action::Warn => "WARN",
            Action::Interrupt => "INTERRUPT",
        })
        .collect::<Vec<_>>()
        .join(", ")
));

/// Port of `com.carrotsearch.randomizedtesting.annotations.ThreadLeakFilters`.
///
/// `@ThreadLeakFilters(defaultFilters, filters)`.
#[derive(Debug, Clone)]
pub struct ThreadLeakFilters {
    /// `defaultFilters()`, default true.
    pub default_filters: bool,
    /// `filters()`, default empty.
    pub filters: Vec<ClassRef<dyn ThreadFilter>>,
}

impl Default for ThreadLeakFilters {
    fn default() -> Self {
        ThreadLeakFilters {
            default_filters: true,
            filters: Vec::new(),
        }
    }
}

annotation_type!(
    ThreadLeakFilters,
    "ThreadLeakFilters",
    "ThreadLeakFilters",
    true
);
java_annotation!(ThreadLeakFilters, THREAD_LEAK_FILTERS, |s| format!(
    "{}(defaultFilters={}, filters={{{}}})",
    prefix("ThreadLeakFilters"),
    s.default_filters,
    class_list(&s.filters)
));

/// Port of `com.carrotsearch.randomizedtesting.annotations.ThreadLeakGroup.Group`.
///
/// `ThreadLeakGroup.Group`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Group {
    /// `ALL`: every thread of the JVM.
    All,
    /// `MAIN` (the default): the threads of the runner's main group.
    #[default]
    Main,
    /// `TESTGROUP`: the threads of the suite's group.
    TestGroup,
}

/// Port of `com.carrotsearch.randomizedtesting.annotations.ThreadLeakGroup`.
///
/// `@ThreadLeakGroup(group)`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ThreadLeakGroup {
    /// `value()`.
    pub value: Group,
}

annotation_type!(ThreadLeakGroup, "ThreadLeakGroup", "ThreadLeakGroup", true);
java_annotation!(ThreadLeakGroup, THREAD_LEAK_GROUP, |s| format!(
    "{}({})",
    prefix("ThreadLeakGroup"),
    match s.value {
        Group::All => "ALL",
        Group::Main => "MAIN",
        Group::TestGroup => "TESTGROUP",
    }
));

/// Port of `com.carrotsearch.randomizedtesting.annotations.ThreadLeakLingering`.
///
/// `@ThreadLeakLingering(linger)`: how long to wait for leaked threads.
#[derive(Debug, Clone, Copy, Default)]
pub struct ThreadLeakLingering {
    /// `linger()` in milliseconds, default 0.
    pub linger: i32,
}

annotation_type!(
    ThreadLeakLingering,
    "ThreadLeakLingering",
    "ThreadLeakLingering",
    true
);
java_annotation!(ThreadLeakLingering, THREAD_LEAK_LINGERING, |s| format!(
    "{}(linger={})",
    prefix("ThreadLeakLingering"),
    s.linger
));

/// Port of `com.carrotsearch.randomizedtesting.annotations.ThreadLeakScope.Scope`.
///
/// `ThreadLeakScope.Scope`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    /// `TEST` (the default): check after each test and the suite.
    #[default]
    Test,
    /// `SUITE`: check after the suite only.
    Suite,
    /// `NONE`: no checks.
    None,
}

/// Port of `com.carrotsearch.randomizedtesting.annotations.ThreadLeakScope`.
///
/// `@ThreadLeakScope(scope)`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ThreadLeakScope {
    /// `value()`.
    pub value: Scope,
}

annotation_type!(ThreadLeakScope, "ThreadLeakScope", "ThreadLeakScope", true);
java_annotation!(ThreadLeakScope, THREAD_LEAK_SCOPE, |s| format!(
    "{}({})",
    prefix("ThreadLeakScope"),
    match s.value {
        Scope::Test => "TEST",
        Scope::Suite => "SUITE",
        Scope::None => "NONE",
    }
));

/// Port of `com.carrotsearch.randomizedtesting.annotations.ThreadLeakZombies.Consequence`.
///
/// `ThreadLeakZombies.Consequence`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Consequence {
    /// `CONTINUE` (deprecated): keep running tests.
    Continue,
    /// `IGNORE_REMAINING_TESTS` (the default).
    #[default]
    IgnoreRemainingTests,
}

/// Port of `com.carrotsearch.randomizedtesting.annotations.ThreadLeakZombies`.
///
/// `@ThreadLeakZombies(consequence)`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ThreadLeakZombies {
    /// `value()`.
    pub value: Consequence,
}

annotation_type!(
    ThreadLeakZombies,
    "ThreadLeakZombies",
    "ThreadLeakZombies",
    true
);
java_annotation!(ThreadLeakZombies, THREAD_LEAK_ZOMBIES, |s| format!(
    "{}({})",
    prefix("ThreadLeakZombies"),
    match s.value {
        Consequence::Continue => "CONTINUE",
        Consequence::IgnoreRemainingTests => "IGNORE_REMAINING_TESTS",
    }
));

/// Port of `com.carrotsearch.randomizedtesting.annotations.Timeout`.
///
/// `@Timeout(millis)`: a test's timeout.
#[derive(Debug, Clone, Copy)]
pub struct Timeout {
    /// `millis()`.
    pub millis: i32,
}

annotation_type!(Timeout, "Timeout", "Timeout", true);
java_annotation!(Timeout, TIMEOUT, |s| format!(
    "{}(millis={})",
    prefix("Timeout"),
    s.millis
));

/// Port of `com.carrotsearch.randomizedtesting.annotations.TimeoutSuite`.
///
/// `@TimeoutSuite(millis)`: a suite's timeout.
#[derive(Debug, Clone, Copy)]
pub struct TimeoutSuite {
    /// `millis()`.
    pub millis: i32,
}

annotation_type!(TimeoutSuite, "TimeoutSuite", "TimeoutSuite", true);
java_annotation!(TimeoutSuite, TIMEOUT_SUITE, |s| format!(
    "{}(millis={})",
    prefix("TimeoutSuite"),
    s.millis
));
