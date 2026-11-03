//! Port of `com.carrotsearch.randomizedtesting.RandomizedRunner`
//! (randomizedtesting 2.8.1, `RandomizedRunner.java`), with its nested
//! `TestCandidate`, `UncaughtException`, `QueueUncaughtExceptionsHandler`,
//! `TestMethodExecution`, `SameInstanceProvider` and `NewInstanceProvider`,
//! and `InstanceProvider` (`InstanceProvider.java`).
//!
//! A suite is a declared [`Class`] (see [`crate::jdk::reflect`]); the runner
//! reads its declaration where Java reflects, and runs it with the hook,
//! rule, seed, repetition, filtering, grouping, thread-leak and timeout
//! semantics of randomizedtesting 2.8.1. A test target with `harness =
//! false` calls [`run_main`] (or [`JUnitCore`](crate::junit::JUnitCore)) from
//! its `main`.
//!
//! **Divergences.**
//! * `detectContainer()` (`RandomizedRunner.java:427`) inspects the JVM's
//!   stack for an Eclipse or IDEA
//!   launcher; a Rust test has no such launcher, so the container is always
//!   `UNKNOWN`.
//! * The suite's static initializer is the declared
//!   [`ClassBuilder::static_init`](crate::jdk::reflect::ClassBuilder::static_init)
//!   (`Class.forName(name, true, loader)`, `RandomizedRunner.java:631`).
//! * A test method's exception arrives as a panic; the runner converts it
//!   to a [`Throwable`] (see that type) and augments its stack trace with
//!   the seed frame as Java does (`RandomizedRunner.java:1891`).
//! * The security-manager branches (`AccessController`,
//!   `SecurityException`, e.g. `RandomizedRunner.java:1089`) have no Rust
//!   counterpart and are omitted.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use crate::annotations::{
    InstanceProviderType, Listeners, ParametersFactory, Repeat, Seed, SeedDecorators, Seeds,
    TestCaseInstanceProvider, TestCaseOrdering, TestContextRandomSupplier, TestMethodAndParams,
    TestMethodProviders, Timeout,
};
use crate::class_model::ClassModel;
use crate::classes::Classes;
use crate::glob_filter::{ClassGlobFilter, MethodGlobFilter};
use crate::group_evaluator::GroupEvaluator;
use crate::jdk::exception::{self, StackTraceElement, Throwable};
use crate::jdk::format::format as java_format;
use crate::jdk::logging::{Level, Logger};
use crate::jdk::random::JdkRandom;
use crate::jdk::reflect::{
    modifier, AnnotationRef, AnnotationType, Class, Constructor, Instance, Method, Value,
};
use crate::jdk::system;
use crate::jdk::thread::{Thread, ThreadGroup, UncaughtExceptionHandler};
use crate::jdk::util::{hash_iteration_order, long_hash_code, shuffle};
use crate::junit::{
    After, AfterClass, Assert, Before, BeforeClass, ClassRule, Description, Failure, Filter,
    FrameworkMethod, Ignore, InitializationError, JUnitResult, ListenerRef, Notifier, Rule,
    RunListener, RunNotifier, Statement, Test, ASSUMPTION_VIOLATED, ASSUMPTION_VIOLATED_INTERNAL,
    NO_TESTS_REMAIN,
};
use crate::lifecycle_scope::LifecycleScope;
use crate::murmur_hash3::MurmurHash3;
use crate::random_supplier::{default_supplier, RandomSupplier};
use crate::randomized_context::RandomizedContext;
use crate::randomized_test::RandomizedTest;
use crate::randomness::Randomness;
use crate::rules::StatementAdapter;
use crate::runner_container::RunnerContainer;
use crate::runner_thread_group::RunnerThreadGroup;
use crate::seed_decorator::SeedDecorator;
use crate::seed_utils::SeedUtils;
use crate::sys_globals::{SysGlobals, CHILDVM_SYSPROP_JVM_COUNT, CHILDVM_SYSPROP_JVM_ID};
use crate::test_method_provider::{JUnit4MethodProvider, TestMethodProvider};
use crate::thread_leak_control::ThreadLeakControl;
use crate::threads::Threads;
use crate::trace_formatting::TraceFormatting;
use crate::validation::Validation;

/// `AUGMENTED_SEED_PACKAGE`: the fake package of the stack frame the runner
/// adds to a test's exception to carry the seeds.
pub const AUGMENTED_SEED_PACKAGE: &str = "__randomizedtesting";
/// `DEFAULT_TIMEOUT`: no test timeout.
pub const DEFAULT_TIMEOUT: i32 = 0;
/// `DEFAULT_TIMEOUT_SUITE`: no suite timeout.
pub const DEFAULT_TIMEOUT_SUITE: i32 = 0;
/// `DEFAULT_KILLATTEMPTS`: interrupt attempts on leaked threads.
pub const DEFAULT_KILLATTEMPTS: i32 = 5;
/// `DEFAULT_KILLWAIT`: milliseconds between interrupt attempts.
pub const DEFAULT_KILLWAIT: i32 = 500;
/// `DEFAULT_ITERATIONS`.
pub const DEFAULT_ITERATIONS: i32 = 1;

/// `ThreadLeakError`.
pub(crate) const THREAD_LEAK_ERROR: &str = crate::thread_leak_error::ThreadLeakError::CLASS;
/// `UncaughtExceptionError`.
pub(crate) const UNCAUGHT_EXCEPTION_ERROR: &str =
    crate::uncaught_exception_error::UncaughtExceptionError::CLASS;
/// `ResourceDisposalError`.
pub(crate) const RESOURCE_DISPOSAL_ERROR: &str =
    "com.carrotsearch.randomizedtesting.ResourceDisposalError";

fn register_runner_throwables() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        exception::register_class(THREAD_LEAK_ERROR, exception::ERROR);
        exception::register_class(UNCAUGHT_EXCEPTION_ERROR, exception::ERROR);
        exception::register_class(RESOURCE_DISPOSAL_ERROR, exception::ERROR);
        exception::register_class(
            crate::filter_expression_parser::SYNTAX_EXCEPTION,
            exception::RUNTIME_EXCEPTION,
        );
    });
}

/// `DEFAULT_STACK_FILTERS`.
const DEFAULT_STACK_FILTERS: &[&str] = &[
    "org.junit.",
    "junit.framework.",
    "sun.",
    "java.lang.reflect.",
    "com.carrotsearch.randomizedtesting.",
];

/// `sequencer`: distinguishes the seeds of runners created in quick
/// succession.
static SEQUENCER: AtomicI64 = AtomicI64::new(0);

/// `zombieMarker`: set once a suite leaves zombie threads.
pub(crate) static ZOMBIE_MARKER: AtomicBool = AtomicBool::new(false);

/// `mainThreadGroup`: the group of the thread that first used the runner.
pub(crate) static MAIN_THREAD_GROUP: LazyLock<Arc<ThreadGroup>> = LazyLock::new(|| {
    Thread::current()
        .thread_group()
        .unwrap_or_else(|| Arc::clone(ThreadGroup::main()))
});

/// Port of `com.carrotsearch.randomizedtesting.InstanceProvider`.
///
/// Creates instances of the suite class (`InstanceProvider`).
pub(crate) trait InstanceProvider: Send + Sync {
    /// `newInstance()`: throws what the constructor throws.
    fn new_instance(&self) -> Instance;
}

/// `NewInstanceProvider`: a new instance per call.
struct NewInstanceProvider {
    constructor: Constructor,
    args: Vec<Value>,
}

impl InstanceProvider for NewInstanceProvider {
    fn new_instance(&self) -> Instance {
        self.constructor.new_instance(&self.args)
    }
}

/// `SameInstanceProvider`: one instance, created on the first call.
struct SameInstanceProvider {
    delegate: NewInstanceProvider,
    instance: Mutex<Option<Instance>>,
}

impl InstanceProvider for SameInstanceProvider {
    fn new_instance(&self) -> Instance {
        let mut i = self.instance.lock().unwrap_or_else(|e| e.into_inner());
        if i.is_none() {
            *i = Some(self.delegate.new_instance());
        }
        Arc::clone(i.as_ref().expect("instance"))
    }
}

/// A test candidate (`TestCandidate`): one execution of a test method with
/// its seed and description.
pub struct TestCandidate {
    /// `seed`.
    pub seed: i64,
    /// `description`.
    pub description: Arc<Description>,
    /// `method`.
    pub method: Arc<Method>,
    instance_provider: Arc<dyn InstanceProvider>,
    suite_class: Arc<Class>,
}

impl TestCandidate {
    /// `getTestClass()`: the suite class.
    pub fn test_class(&self) -> &Arc<Class> {
        &self.suite_class
    }
}

/// `UncaughtException`: an uncaught exception and its thread.
#[derive(Clone)]
pub(crate) struct UncaughtException {
    pub(crate) thread_name: String,
    pub(crate) error: Throwable,
}

/// Queues uncaught exceptions (`QueueUncaughtExceptionsHandler`).
#[derive(Default)]
pub(crate) struct QueueUncaughtExceptionsHandler {
    state: Mutex<(Vec<UncaughtException>, bool)>,
}

impl QueueUncaughtExceptionsHandler {
    fn new() -> QueueUncaughtExceptionsHandler {
        QueueUncaughtExceptionsHandler {
            state: Mutex::new((Vec::new(), true)),
        }
    }

    /// `stopReporting()`.
    pub(crate) fn stop_reporting(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).1 = false;
    }

    /// `resumeReporting()`.
    pub(crate) fn resume_reporting(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).1 = true;
    }

    /// `getUncaughtAndClear()`.
    pub(crate) fn get_uncaught_and_clear(&self) -> Vec<UncaughtException> {
        std::mem::take(&mut self.state.lock().unwrap_or_else(|e| e.into_inner()).0)
    }
}

impl UncaughtExceptionHandler for QueueUncaughtExceptionsHandler {
    fn uncaught_exception(&self, t: &Thread, e: Throwable) {
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if !s.1 {
                return;
            }
            s.0.push(UncaughtException {
                thread_name: Threads::thread_name(t),
                error: e.clone(),
            });
        }
        Logger::get_logger("RunnerThreadGroup").log(
            Level::Warning,
            &format!("Uncaught exception in thread: {t}"),
            Some(&e),
        );
    }
}

/// The runner's state, shared with its context and threads.
pub(crate) struct RunnerInner {
    suite_class: Arc<Class>,
    pub(crate) runner_randomness: Randomness,
    test_case_randomness_override: Option<Randomness>,
    iterations_override: Option<i32>,
    test_candidates: Mutex<Vec<Arc<TestCandidate>>>,
    suite_description: Mutex<Arc<Description>>,
    pub(crate) runner_thread_group: Mutex<Option<Arc<ThreadGroup>>>,
    auto_listeners: Mutex<Vec<ListenerRef>>,
    append_seed_parameter: bool,
    traces: TraceFormatting,
    container_runner: RunnerContainer,
    pub(crate) handler: Mutex<Option<Arc<QueueUncaughtExceptionsHandler>>>,
    class_model: ClassModel,
    random_supplier: Arc<dyn RandomSupplier>,
    shuffled_methods_cache: Mutex<HashMap<&'static str, Vec<Arc<Method>>>>,
    restore_properties: Mutex<Vec<(String, Option<String>)>>,
    pub(crate) group_evaluator: Mutex<Arc<GroupEvaluator>>,
}

/// A JUnit runner for randomized test cases with predictable and repeatable
/// randomness (`RandomizedRunner`).
///
/// Supports `@BeforeClass`, `@Before`, `@Test`, `@After`, `@AfterClass`,
/// `@Rule` and `@ClassRule`; hooks of a superclass run before (after, for
/// `@After*`) those of a subclass, and hooks declared in the same class run
/// in an order derived from the master seed. A test must not leave threads
/// behind, and uncaught exceptions of any thread fail it.
#[derive(Clone)]
pub struct RandomizedRunner {
    pub(crate) inner: Arc<RunnerInner>,
}

/// `RandomizedRunner.logger`.
static LOGGER: LazyLock<Logger> = LazyLock::new(|| Logger::get_logger("RandomizedRunner"));

/// `emptyToNull(value)`: the trimmed value; `None` when blank.
pub(crate) fn empty_to_null(value: Option<String>) -> Option<String> {
    value
        .map(|v| crate::jdk::lang::trim(&v).to_string())
        .filter(|v| !v.is_empty())
}

/// Runs `f`, converting a panic into its [`Throwable`].
pub(crate) fn try_run<T>(f: impl FnOnce() -> T) -> Result<T, Throwable> {
    catch_unwind(AssertUnwindSafe(f)).map_err(Throwable::from_panic)
}

/// `MultipleFailureException.assertEmpty(errors)`: nothing for no error,
/// the error itself for one, a `MultipleFailureException` for more.
pub(crate) fn assert_empty(mut errors: Vec<Throwable>) {
    match errors.len() {
        0 => {}
        1 => errors.remove(0).throw(),
        _ => Throwable::multiple_failures(errors).throw(),
    }
}

/// `isAssumptionViolated(t)`.
pub(crate) fn is_assumption_violated(t: &Throwable) -> bool {
    t.is_instance(ASSUMPTION_VIOLATED) || t.is_instance(ASSUMPTION_VIOLATED_INTERNAL)
}

/// `methodName(description)`: the method name without its parameterized
/// part (`replaceAll("\\s?\\{.+\\}", "")`).
pub fn method_name(description: &Description) -> String {
    let name = description.method_name().unwrap_or_default();
    strip_braced(&name)
}

/// `replaceAll("\\s?\\{.+\\}", "")`.
fn strip_braced(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let is_space = |ch: char| matches!(ch, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r');
    let is_term = |ch: char| matches!(ch, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}');
    // The end of `{.+}` starting at `open`: the last `}` reachable without
    // crossing a line terminator, at least one character after `open`.
    let close_after = |open: usize| -> Option<usize> {
        let mut last = None;
        let mut k = open + 1;
        while k < c.len() && !is_term(c[k]) {
            if c[k] == '}' && k > open + 1 {
                last = Some(k);
            }
            k += 1;
        }
        last
    };
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        let mut matched = None;
        if is_space(c[i]) && i + 1 < c.len() && c[i + 1] == '{' {
            if let Some(end) = close_after(i + 1) {
                matched = Some(end);
            }
        }
        if matched.is_none() && c[i] == '{' {
            if let Some(end) = close_after(i) {
                matched = Some(end);
            }
        }
        match matched {
            Some(end) => i = end + 1,
            None => {
                out.push(c[i]);
                i += 1;
            }
        }
    }
    out
}

/// `augmentStackTrace(e, seeds)`: inserts the frame
/// `__randomizedtesting.SeedInfo.seed(<chain>:0)` first, unless the
/// throwable already carries that chain; without seeds, the chain of the
/// current context's randomness stack.
pub(crate) fn augment_stack_trace(mut e: Throwable, seeds: &[&Randomness]) -> Throwable {
    let chain = if seeds.is_empty() {
        let all = RandomizedContext::current().get_randomnesses();
        let refs: Vec<&Randomness> = all.iter().collect();
        SeedUtils::format_seed_chain(&refs)
    } else {
        SeedUtils::format_seed_chain(seeds)
    };
    if seed_from_throwable(&e).as_deref() == Some(chain.as_str()) {
        return e;
    }
    let mut stack = e.stack_trace().to_vec();
    stack.insert(
        0,
        StackTraceElement::new(
            format!("{AUGMENTED_SEED_PACKAGE}.SeedInfo"),
            "seed",
            Some(chain),
            0,
        ),
    );
    e.set_stack_trace(stack);
    e
}

/// `seedFromThrowable(t)`: the seed chains of the augmented frames of `t`
/// and its causes, joined with `, `; `None` without any.
pub fn seed_from_throwable(t: &Throwable) -> Option<String> {
    let mut b = String::new();
    let mut cur = Some(t);
    while let Some(x) = cur {
        for s in x.stack_trace() {
            if s.declaring_class.starts_with(AUGMENTED_SEED_PACKAGE) {
                if !b.is_empty() {
                    b.push_str(", ");
                }
                b.push_str(s.file_name.as_deref().unwrap_or("null"));
            }
        }
        cur = x.cause();
    }
    (!b.is_empty()).then_some(b)
}

/// `getAnnotationsFromClassHierarchy(clazz, annotation)`: the annotation as
/// each class of the hierarchy reports it, superclass first, an inherited
/// annotation once.
fn annotations_from_class_hierarchy(clazz: &Class, ty: &AnnotationType) -> Vec<AnnotationRef> {
    let mut anns: Vec<AnnotationRef> = Vec::new();
    let mut c: Option<&Class> = Some(clazz);
    while let Some(cls) = c {
        if let Some(ann) = cls.get_annotation_ref(ty) {
            let seen = anns.iter().any(|a| Arc::ptr_eq(a, &ann));
            if !(ty.inherited && seen) {
                anns.push(ann);
            }
        }
        c = cls.superclass().map(|s| s.as_ref());
    }
    anns.reverse();
    anns
}

fn downcast<A: 'static>(a: &AnnotationRef) -> &A {
    a.as_any()
        .downcast_ref::<A>()
        .expect("annotation of the requested type")
}

static BEFORE: AnnotationType = Before::TYPE;
static AFTER: AnnotationType = After::TYPE;
static BEFORE_CLASS: AnnotationType = BeforeClass::TYPE;
static AFTER_CLASS: AnnotationType = AfterClass::TYPE;
static PARAMETERS_FACTORY: AnnotationType = ParametersFactory::TYPE;
static RULE: AnnotationType = Rule::TYPE;
static CLASS_RULE: AnnotationType = ClassRule::TYPE;

/// `TestMethodExecution`: a test method with its constructor arguments.
struct TestMethodExecution {
    params: Vec<Value>,
    method: Arc<Method>,
    arg_formatting_template: String,
    instance_provider: Arc<dyn InstanceProvider>,
}

impl TestMethodAndParams for TestMethodExecution {
    fn test_method(&self) -> &Arc<Method> {
        &self.method
    }
    fn instance_arguments(&self) -> &[Value] {
        &self.params
    }
}

/// Downcasts a JUnit runner to the randomized runner, when it is one.
pub(crate) fn as_randomized_runner(r: &dyn crate::junit::Runner) -> Option<&RandomizedRunner> {
    (r as &dyn std::any::Any).downcast_ref::<RandomizedRunner>()
}

impl RandomizedRunner {
    /// `RandomizedRunner.logger`.
    pub fn logger() -> &'static Logger {
        &LOGGER
    }

    /// `zombieMarker.set(value)`: the marker `ThreadLeakControl` sets when a
    /// thread could not be terminated. Package-private in Java, where the
    /// runner's own tests (`WithNestedTestClass.after`) reset it.
    pub fn set_zombie_marker(value: bool) {
        ZOMBIE_MARKER.store(value, Ordering::Release);
    }

    /// `hasZombieThreads()`: whether a suite left zombie threads with
    /// `IGNORE_REMAINING_TESTS`.
    pub fn has_zombie_threads() -> bool {
        ZOMBIE_MARKER.load(Ordering::Acquire)
    }

    /// `new RandomizedRunner(testClass)`.
    ///
    /// # Errors
    ///
    /// The `InitializationError` (or the throwable Java's constructor
    /// throws) when the suite is invalid: a malformed `tests.seed` or
    /// `tests.iters`, a validation failure, a failing provider or factory.
    pub fn new(test_class: &Arc<Class>) -> Result<RandomizedRunner, InitializationError> {
        register_runner_throwables();
        // Java's static initializer runs on the thread that loads the class.
        LazyLock::force(&MAIN_THREAD_GROUP);
        let _quiet = exception::QuietPanics::new();
        try_run(|| RandomizedRunner::construct(test_class)).map_err(InitializationError::new)
    }

    fn construct(test_class: &Arc<Class>) -> RandomizedRunner {
        let append_seed_parameter =
            RandomizedTest::system_property_as_boolean(&SysGlobals::sysprop_append_seed(), false);
        let traces = if RandomizedTest::system_property_as_boolean(
            &SysGlobals::sysprop_stackfiltering(),
            true,
        ) {
            TraceFormatting::with_filters(DEFAULT_STACK_FILTERS)
        } else {
            TraceFormatting::new()
        };
        let suite_class = Arc::clone(test_class);
        let class_model = ClassModel::new(&suite_class);
        let container_runner = RunnerContainer::Unknown;

        // Seed decorators.
        let mut decorators: Vec<Arc<dyn SeedDecorator>> = Vec::new();
        for ann in annotations_from_class_hierarchy(&suite_class, &SeedDecorators::TYPE) {
            for clazz in &downcast::<SeedDecorators>(&ann).value {
                let made = clazz.new_instance().and_then(|mut d| {
                    try_run(|| {
                        d.initialize(suite_class.name());
                        d
                    })
                });
                match made {
                    Ok(d) => decorators.push(Arc::from(d)),
                    Err(t) => Throwable::with_cause(
                        exception::RUNTIME_EXCEPTION,
                        Some(format!(
                            "Could not initialize suite class: {} because its @SeedDecorators \
                             contains non-instantiable: {}",
                            suite_class.name(),
                            clazz.name
                        )),
                        t,
                    )
                    .throw(),
                }
            }
        }

        let random_supplier = RandomizedRunner::determine_random_supplier(&suite_class);

        let random_seed = MurmurHash3::hash_long(
            SEQUENCER
                .fetch_add(1, Ordering::Relaxed)
                .wrapping_add(system::nano_time()),
        );
        let global_seed = empty_to_null(system::get_property(&SysGlobals::sysprop_random_seed()));
        let mut test_case_randomness_override = None;
        let initial_seed = if let Some(global_seed) = global_seed {
            let chain = SeedUtils::parse_seed_chain(&global_seed);
            if chain.is_empty() || chain.len() > 2 {
                exception::illegal_argument(format!(
                    "Invalid system property {} specification: {}",
                    SysGlobals::sysprop_random_seed(),
                    global_seed
                ));
            }
            if chain.len() > 1 {
                test_case_randomness_override = Some(Randomness::new(
                    chain[1],
                    Arc::clone(&random_supplier),
                    Vec::new(),
                ));
            }
            chain[0]
        } else if suite_class.is_annotation_present(&Seed::TYPE) {
            let seed = suite_class.get_annotation::<Seed>().expect("@Seed");
            seed_from_annot(&seed, random_seed)[0]
        } else {
            random_seed
        };
        let runner_randomness =
            Randomness::new(initial_seed, Arc::clone(&random_supplier), decorators);

        let iterations_override =
            if empty_to_null(system::get_property(&SysGlobals::sysprop_iterations())).is_some() {
                let v =
                    RandomizedTest::system_property_as_int(&SysGlobals::sysprop_iterations(), 0);
                if v < 1 {
                    exception::illegal_argument(format!(
                        "System property {} must be >= 1: {}",
                        SysGlobals::sysprop_iterations(),
                        v
                    ));
                }
                Some(v)
            } else {
                None
            };

        let inner = Arc::new(RunnerInner {
            suite_class: Arc::clone(&suite_class),
            runner_randomness,
            test_case_randomness_override,
            iterations_override,
            test_candidates: Mutex::new(Vec::new()),
            suite_description: Mutex::new(Description::create_suite_description_for_class(
                &suite_class,
            )),
            runner_thread_group: Mutex::new(None),
            auto_listeners: Mutex::new(Vec::new()),
            append_seed_parameter,
            traces,
            container_runner,
            handler: Mutex::new(None),
            class_model,
            random_supplier,
            shuffled_methods_cache: Mutex::new(HashMap::new()),
            restore_properties: Mutex::new(Vec::new()),
            group_evaluator: Mutex::new(Arc::new(GroupEvaluator::new())),
        });
        let runner = RandomizedRunner { inner };

        // Java wraps the rest in InitializationError; both reach JUnit's
        // error runner with the same cause.
        runner.validate_target();
        let suite_description = Description::create_suite_description_for_class(&suite_class);
        *runner
            .inner
            .suite_description
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Arc::clone(&suite_description);
        let candidates = runner.collect_test_candidates(&suite_description);
        let candidate_annotations: Vec<Vec<AnnotationRef>> = candidates
            .iter()
            .map(|c| {
                let mut v = c.suite_class.annotations();
                v.extend(c.method.annotations().iter().cloned());
                v
            })
            .collect();
        *runner
            .inner
            .test_candidates
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = candidates;
        *runner
            .inner
            .group_evaluator
            .lock()
            .unwrap_or_else(|e| e.into_inner()) =
            Arc::new(GroupEvaluator::from_candidates(&candidate_annotations));

        if let Some(m) = empty_to_null(system::get_property(&SysGlobals::sysprop_testmethod())) {
            let _ = runner.filter(&MethodGlobFilter::new(&m));
        }
        if let Some(c) = empty_to_null(system::get_property(&SysGlobals::sysprop_testclass())) {
            let suite_filter = ClassGlobFilter::new(&c);
            let d = runner.description();
            if !suite_filter.should_run(&d) {
                // `getChildren()` returns a copy in JUnit 4.13, so clearing it
                // leaves the description as it is.
                runner
                    .inner
                    .test_candidates
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clear();
            }
        }
        runner
    }

    /// `determineRandomSupplier(testClass)`: the last
    /// `@TestContextRandomSupplier` of the hierarchy, else the default.
    fn determine_random_supplier(test_class: &Class) -> Arc<dyn RandomSupplier> {
        let anns = annotations_from_class_hierarchy(test_class, &TestContextRandomSupplier::TYPE);
        match anns.last() {
            None => default_supplier(),
            Some(a) => {
                let clazz = &downcast::<TestContextRandomSupplier>(a).value;
                match clazz.new_instance() {
                    Ok(s) => s,
                    Err(e) => Throwable::with_cause(
                        exception::ILLEGAL_ARGUMENT_EXCEPTION,
                        Some(format!(
                            "Could not instantiate random supplier of class: {}",
                            clazz.class_to_string()
                        )),
                        e,
                    )
                    .throw(),
                }
            }
        }
    }

    /// The suite class.
    pub fn suite_class(&self) -> &Arc<Class> {
        &self.inner.suite_class
    }

    /// `getDescription()`: the current (filtered) description tree.
    pub fn description(&self) -> Arc<Description> {
        Arc::clone(
            &self
                .inner
                .suite_description
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        )
    }

    /// `getTraceFormatting()`.
    pub fn get_trace_formatting(&self) -> &TraceFormatting {
        &self.inner.traces
    }

    /// The runner's master randomness (`runnerRandomness`).
    pub(crate) fn runner_randomness(&self) -> &Randomness {
        &self.inner.runner_randomness
    }

    /// `groupEvaluator`.
    pub fn group_evaluator(&self) -> Arc<GroupEvaluator> {
        Arc::clone(
            &self
                .inner
                .group_evaluator
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        )
    }

    /// `filter(filter)`: keeps the candidates the filter accepts (by full
    /// description or by plain method name) and prunes the description.
    ///
    /// # Errors
    ///
    /// `NoTestsRemainException` when no candidate remains.
    pub fn filter(&self, filter: &dyn Filter) -> Result<(), Throwable> {
        let candidates = self
            .inner
            .test_candidates
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let filtered = apply_filters(&self.inner.suite_class, &candidates, &[filter]);
        let permitted: Vec<Arc<Description>> = filtered
            .iter()
            .map(|c| Arc::clone(&c.description))
            .collect();
        let pruned = prune(&self.description(), &permitted);
        *self
            .inner
            .suite_description
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = pruned;
        let empty = filtered.is_empty();
        *self
            .inner
            .test_candidates
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = filtered;
        if empty {
            return Err(Throwable::with_message(NO_TESTS_REMAIN, None));
        }
        Ok(())
    }

    /// `run(notifier)`: runs every test and hook, with the child-VM
    /// properties set for the duration.
    pub fn run(&self, notifier: &Arc<RunNotifier>) {
        register_runner_throwables();
        let _quiet = exception::QuietPanics::new();
        self.process_system_properties();
        let r = try_run(|| self.run_suite(notifier));
        self.restore_system_properties();
        if let Err(t) = r {
            t.throw()
        }
    }

    fn restore_system_properties(&self) {
        let props = std::mem::take(
            &mut *self
                .inner
                .restore_properties
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        );
        for (k, v) in props {
            match v {
                None => {
                    system::clear_property(&k);
                }
                Some(v) => {
                    system::set_property(&k, &v);
                }
            }
        }
    }

    fn process_system_properties(&self) {
        let jvm_count = system::get_property(CHILDVM_SYSPROP_JVM_COUNT);
        let jvm_id = system::get_property(CHILDVM_SYSPROP_JVM_ID);
        if empty_to_null(jvm_count.clone()).is_none() && empty_to_null(jvm_id.clone()).is_none() {
            system::set_property(CHILDVM_SYSPROP_JVM_COUNT, "1");
            system::set_property(CHILDVM_SYSPROP_JVM_ID, "0");
            let mut r = self
                .inner
                .restore_properties
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            r.push((CHILDVM_SYSPROP_JVM_COUNT.to_string(), jvm_count));
            r.push((CHILDVM_SYSPROP_JVM_ID.to_string(), jvm_id));
        }
    }

    /// `runSuite(notifier)`: installs the uncaught-exception handler and
    /// runs the suite in its own thread group and thread.
    fn run_suite(&self, notifier: &Arc<RunNotifier>) {
        let previous = Thread::get_default_uncaught_exception_handler();
        let handler = Arc::new(QueueUncaughtExceptionsHandler::new());
        *self.inner.handler.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&handler));
        let handler_dyn: Arc<dyn UncaughtExceptionHandler> = handler.clone();
        Thread::set_default_uncaught_exception_handler(Some(Arc::clone(&handler_dyn)));

        let simple = Classes::simple_name(self.inner.suite_class.name()).to_string();
        let group = RunnerThreadGroup::new(&format!("TGRP-{simple}"));
        *self
            .inner
            .runner_thread_group
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&group));

        let me = self.clone();
        let n = Arc::clone(notifier);
        let tg = Arc::clone(&group);
        let name = format!(
            "SUITE-{}-seed#{}",
            simple,
            SeedUtils::format_seed_chain(&[&self.inner.runner_randomness])
        );
        let runner = Thread::with_group(&group, name, move || {
            let r = try_run(|| {
                if let Err(t) = me.inner.suite_class.initialize() {
                    t.throw();
                }
                let context = RandomizedContext::create_for_runner(&tg, &me);
                me.run_suite_in_context(&context, &n);
                context.dispose();
            });
            if let Err(t) = r {
                n.fire_test_failure(Failure::new(me.description(), t));
            }
        });
        runner.start();
        let _ = runner.join();

        let current = Thread::get_default_uncaught_exception_handler();
        let same = current
            .as_ref()
            .is_some_and(|c| Thread::same_handler(c, &handler_dyn));
        if !same {
            let what = match &current {
                None => "(null)".to_string(),
                Some(_) => "class <handler>".to_string(),
            };
            notifier.fire_test_failure(Failure::new(
                self.description(),
                Throwable::new(
                    exception::RUNTIME_EXCEPTION,
                    format!(
                        "Suite replaced Thread.defaultUncaughtExceptionHandler. It's better not \
                         to touch it. Or at least revert it to what it was before. Current: {what}"
                    ),
                ),
            ));
        }
        Thread::set_default_uncaught_exception_handler(previous);
        *self
            .inner
            .runner_thread_group
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        *self.inner.handler.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// `runSuite(context, notifier)`: the suite's statement chain, under
    /// the runner thread group.
    fn run_suite_in_context(&self, context: &Arc<RandomizedContext>, notifier: &Arc<RunNotifier>) {
        let result = JUnitResult::new();
        let accounting = result.create_listener();
        notifier.add_listener(Arc::clone(&accounting));

        let class_randomness = self.inner.runner_randomness.clone_for(&Thread::current());
        context.push(class_randomness);
        let suite_description = self.description();
        let r = try_run(|| {
            self.subscribe_listeners(notifier);
            for l in self.auto_listeners() {
                if let Err(e) = try_run(|| l.test_run_started(&suite_description)) {
                    LOGGER.log(
                        Level::Severe,
                        "Panic: RunListener hook shouldn't throw exceptions.",
                        Some(&e),
                    );
                }
            }
            let tests: Vec<Arc<TestCandidate>> = self
                .inner
                .test_candidates
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            if !tests.is_empty() {
                let ignored = self.determine_ignored_tests(&tests);
                if ignored.len() == tests.len() {
                    for c in &tests {
                        if ignored
                            .iter()
                            .any(|(x, report)| Arc::ptr_eq(x, c) && *report)
                        {
                            self.report_as_ignored(notifier.as_ref(), c);
                        }
                    }
                } else {
                    let tlc = ThreadLeakControl::new(Arc::clone(notifier), self.clone());
                    let mut s = self.run_tests_statement(
                        tlc.notifier(),
                        tests.clone(),
                        ignored,
                        Arc::clone(&tlc),
                    );
                    s = self.with_class_befores(s);
                    s = self.with_class_afters(s);
                    s = self.with_class_rules(s);
                    s = with_close_context_resources(s, LifecycleScope::Suite);
                    s = tlc.for_suite(s, Arc::clone(&suite_description));
                    if let Err(t) = try_run(|| s.evaluate()) {
                        let t = augment_stack_trace(t, &[&self.inner.runner_randomness]);
                        if is_assumption_violated(&t) {
                            notifier.fire_test_assumption_failed(Failure::new(
                                Arc::clone(&suite_description),
                                t,
                            ));
                            for c in &tests {
                                notifier.fire_test_ignored(&c.description);
                            }
                        } else {
                            fire_test_failure(notifier.as_ref(), &suite_description, t);
                        }
                    }
                }
            }
        });
        if let Err(t) = r {
            notifier.fire_test_failure(Failure::new(Arc::clone(&suite_description), t));
        }
        for l in self.auto_listeners() {
            if let Err(e) = try_run(|| l.test_run_finished(&result)) {
                LOGGER.log(
                    Level::Severe,
                    "Panic: RunListener hook shouldn't throw exceptions.",
                    Some(&e),
                );
            }
        }
        notifier.remove_listener(&accounting);
        self.unsubscribe_listeners(notifier);
        context.pop_and_destroy();
    }

    fn auto_listeners(&self) -> Vec<ListenerRef> {
        self.inner
            .auto_listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// `determineIgnoredTests(tests)`: the candidates to skip, each with
    /// whether to report it.
    fn determine_ignored_tests(
        &self,
        tests: &[Arc<TestCandidate>],
    ) -> Vec<(Arc<TestCandidate>, bool)> {
        let ge = self.group_evaluator();
        let mut ignored: Vec<(Arc<TestCandidate>, bool)> = Vec::new();
        let mut put = |c: &Arc<TestCandidate>, v: bool| match ignored
            .iter_mut()
            .find(|(x, _)| Arc::ptr_eq(x, c))
        {
            Some(slot) => slot.1 = v,
            None => ignored.push((Arc::clone(c), v)),
        };
        for c in tests {
            if c.method.is_annotation_present(&Ignore::TYPE) {
                put(c, true);
            }
            if self.is_test_filtered(&ge, c) {
                if matches!(
                    self.inner.container_runner,
                    RunnerContainer::Eclipse | RunnerContainer::Idea
                ) || !ge.has_filtering_expression()
                {
                    put(c, true);
                } else {
                    put(c, false);
                }
            }
        }
        ignored
    }

    fn ignore_reason(&self, ge: &GroupEvaluator, c: &TestCandidate) -> Option<String> {
        let class_anns = self.inner.suite_class.annotations();
        ge.get_ignore_reason(&[c.method.annotations(), &class_anns])
    }

    fn is_test_filtered(&self, ge: &GroupEvaluator, c: &TestCandidate) -> bool {
        self.ignore_reason(ge, c).is_some()
    }

    /// `runTestsStatement(...)`: runs each candidate in turn, under its own
    /// thread name and randomness.
    fn run_tests_statement(
        &self,
        notifier: Arc<dyn Notifier>,
        tests: Vec<Arc<TestCandidate>>,
        ignored: Vec<(Arc<TestCandidate>, bool)>,
        tlc: Arc<ThreadLeakControl>,
    ) -> Statement {
        let me = self.clone();
        Statement::new(move || {
            for c in &tests {
                if tlc.is_timed_out() {
                    break;
                }
                let test_thread_name = format!(
                    "TEST-{}.{}-seed#{}",
                    Classes::simple_name(me.inner.suite_class.name()),
                    c.method.name(),
                    SeedUtils::format_seed_chain(&[&me.inner.runner_randomness])
                );
                let current_thread = Thread::current();
                let restore_name = current_thread.name();
                let current = RandomizedContext::current();
                let r = try_run(|| {
                    current_thread.set_name(test_thread_name.clone());
                    current.push(Randomness::new(
                        c.seed,
                        Arc::clone(&me.inner.random_supplier),
                        Vec::new(),
                    ));
                    current.set_target_method(Some(c.method.name()));
                    match ignored.iter().find(|(x, _)| Arc::ptr_eq(x, c)) {
                        Some((_, report)) => {
                            if *report {
                                me.report_as_ignored(notifier.as_ref(), c);
                            }
                        }
                        None => me.run_single_test(notifier.as_ref(), c, &tlc),
                    }
                });
                current_thread.set_name(restore_name);
                current.set_target_method(None);
                current.pop_and_destroy();
                if let Err(t) = r {
                    t.throw();
                }
            }
        })
    }

    /// `reportAsIgnored(notifier, ge, c)`: `@Ignore`d tests are reported
    /// ignored; group-filtered ones as started, assumption-failed and
    /// finished.
    fn report_as_ignored(&self, notifier: &dyn Notifier, c: &TestCandidate) {
        if c.method.is_annotation_present(&Ignore::TYPE) {
            notifier.fire_test_ignored(&c.description);
            return;
        }
        let ge = self.group_evaluator();
        if let Some(reason) = self.ignore_reason(&ge, c) {
            notifier.fire_test_started(&c.description);
            notifier.fire_test_assumption_failed(Failure::new(
                Arc::clone(&c.description),
                Throwable::new(ASSUMPTION_VIOLATED, reason),
            ));
            notifier.fire_test_finished(&c.description);
        }
    }

    /// `withClassBefores(s)`.
    fn with_class_befores(&self, s: Statement) -> Statement {
        let me = self.clone();
        let befores = self.get_shuffled_methods(&BEFORE_CLASS);
        Statement::new(move || {
            if let Err(t) = try_run(|| {
                for m in &befores {
                    m.invoke(None, &[]);
                }
            }) {
                augment_stack_trace(t, &[&me.inner.runner_randomness]).throw();
            }
            s.evaluate();
        })
    }

    /// `withClassAfters(s)`: every `@AfterClass` runs; the errors are
    /// collected.
    fn with_class_afters(&self, s: Statement) -> Statement {
        let me = self.clone();
        let afters = self.get_shuffled_methods(&AFTER_CLASS);
        Statement::new(move || {
            let mut errors = Vec::new();
            if let Err(t) = try_run(|| s.evaluate()) {
                errors.push(augment_stack_trace(t, &[&me.inner.runner_randomness]));
            }
            for m in &afters {
                if let Err(t) = try_run(|| m.invoke(None, &[])) {
                    errors.push(augment_stack_trace(t, &[&me.inner.runner_randomness]));
                }
            }
            assert_empty(errors);
        })
    }

    /// `withClassRules(s)`.
    fn with_class_rules(&self, mut s: Statement) -> Statement {
        let d = self.description();
        for v in self.get_annotated_field_values(None, &CLASS_RULE) {
            if let Value::TestRule(rule) = v {
                s = rule.apply(s, &d);
            }
        }
        s
    }

    /// `runSingleTest(notifier, c, threadLeakControl)`.
    fn run_single_test(
        &self,
        notifier: &dyn Notifier,
        c: &Arc<TestCandidate>,
        tlc: &Arc<ThreadLeakControl>,
    ) {
        notifier.fire_test_started(&c.description);
        let r = try_run(|| {
            let instance = c.instance_provider.new_instance();
            let method = Arc::clone(&c.method);
            let inst = Arc::clone(&instance);
            let mut s = Statement::new(move || {
                method.invoke(Some(&inst), &[]);
            });
            s = wrap_expected_exceptions(s, c);
            s = self.wrap_before_and_afters(s, &instance);
            s = self.wrap_method_rules(s, c, &instance);
            s = with_close_context_resources(s, LifecycleScope::Test);
            s = tlc.for_test(s, Arc::clone(c));
            s.evaluate();
        });
        if let Err(e) = r {
            let e = augment_stack_trace(e, &[]);
            if is_assumption_violated(&e) {
                notifier.fire_test_assumption_failed(Failure::new(Arc::clone(&c.description), e));
            } else {
                fire_test_failure(notifier, &c.description, e);
            }
        }
        notifier.fire_test_finished(&c.description);
    }

    /// `wrapBeforeAndAfters(s, c, instance)`: the first failing `@Before`
    /// stops the rest; every `@After` runs.
    fn wrap_before_and_afters(&self, mut s: Statement, instance: &Instance) -> Statement {
        let befores = self.get_shuffled_methods(&BEFORE);
        if !befores.is_empty() {
            let after_befores = s;
            let inst = Arc::clone(instance);
            s = Statement::new(move || {
                for m in &befores {
                    m.invoke(Some(&inst), &[]);
                }
                after_befores.evaluate();
            });
        }
        let afters = self.get_shuffled_methods(&AFTER);
        if !afters.is_empty() {
            let before_afters = s;
            let inst = Arc::clone(instance);
            s = Statement::new(move || {
                let mut cumulative = Vec::new();
                if let Err(t) = try_run(|| before_afters.evaluate()) {
                    cumulative.push(t);
                }
                for m in &afters {
                    if let Err(t) = try_run(|| m.invoke(Some(&inst), &[])) {
                        cumulative.push(t);
                    }
                }
                match cumulative.len() {
                    0 => {}
                    1 => cumulative.remove(0).throw(),
                    _ => Throwable::multiple_failures_of_class(
                        "org.junit.runners.model.MultipleFailureException",
                        cumulative,
                    )
                    .throw(),
                }
            });
        }
        s
    }

    /// `wrapMethodRules(s, c, instance)`: `MethodRule`s first, then
    /// `TestRule`s.
    fn wrap_method_rules(
        &self,
        mut s: Statement,
        c: &TestCandidate,
        instance: &Instance,
    ) -> Statement {
        let fm = FrameworkMethod::new(Arc::clone(&c.method));
        let values = self.get_annotated_field_values(Some(instance), &RULE);
        for v in &values {
            if let Value::MethodRule(rule) = v {
                s = rule.apply(s, &fm, instance);
            }
        }
        for v in &values {
            if let Value::TestRule(rule) = v {
                s = rule.apply(s, &c.description);
            }
        }
        s
    }

    /// `getAnnotatedFieldValues(test, annotationClass, valueClass)`: the
    /// annotated fields of the class hierarchy (a field shadowed by a
    /// subclass field of the same name is skipped, as JUnit's `TestClass`
    /// does), each class's fields sorted by name and shuffled with the
    /// master seed, subclass first.
    fn get_annotated_field_values(
        &self,
        test: Option<&Instance>,
        ann: &AnnotationType,
    ) -> Vec<Value> {
        // JUnit's `TestClass.getAnnotatedFields`.
        let mut annotated: Vec<Arc<crate::jdk::reflect::Field>> = Vec::new();
        let mut c: Option<&Class> = Some(&self.inner.suite_class);
        while let Some(cls) = c {
            let mut fields: Vec<_> = cls.declared_fields().to_vec();
            fields.sort_by(|a, b| crate::annotations::jdk_string_compare(a.name(), b.name()));
            for f in fields {
                if f.is_annotation_present(ann) && !annotated.iter().any(|x| x.name() == f.name()) {
                    annotated.push(f);
                }
            }
            c = cls.superclass().map(|s| s.as_ref());
        }
        // Split by class; sort and shuffle each class's fields.
        let mut by_class: Vec<(String, Vec<Arc<crate::jdk::reflect::Field>>)> = Vec::new();
        for f in annotated {
            match by_class.iter_mut().find(|(n, _)| n == f.declaring_class()) {
                Some((_, v)) => v.push(f),
                None => by_class.push((f.declaring_class().to_string(), vec![f])),
            }
        }
        for (_, fields) in by_class.iter_mut() {
            fields.sort_by(|a, b| crate::annotations::jdk_string_compare(a.name(), b.name()));
            shuffle(
                fields,
                &JdkRandom::new(self.inner.runner_randomness.get_seed()),
            );
        }
        let mut ordered = Vec::new();
        let mut c: Option<&Class> = Some(&self.inner.suite_class);
        while let Some(cls) = c {
            if let Some((_, v)) = by_class.iter().find(|(n, _)| n == cls.name()) {
                ordered.extend(v.iter().cloned());
            }
            c = cls.superclass().map(|s| s.as_ref());
        }
        ordered
            .iter()
            .map(|f| f.get(test))
            .filter(|v| matches!(v, Value::TestRule(_) | Value::MethodRule(_)))
            .collect()
    }

    /// `subscribeListeners(notifier)`: instantiates the `@Listeners` of the
    /// hierarchy and adds them.
    fn subscribe_listeners(&self, notifier: &Arc<RunNotifier>) {
        for ann in annotations_from_class_hierarchy(&self.inner.suite_class, &Listeners::TYPE) {
            for clazz in &downcast::<Listeners>(&ann).value {
                match clazz.new_instance() {
                    Ok(l) => {
                        self.inner
                            .auto_listeners
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(Arc::clone(&l));
                        notifier.add_listener(l);
                    }
                    Err(t) => Throwable::with_cause(
                        exception::RUNTIME_EXCEPTION,
                        Some(format!(
                            "Could not initialize suite class: {} because its @Listener is not \
                             instantiable: {}",
                            self.inner.suite_class.name(),
                            clazz.name
                        )),
                        t,
                    )
                    .throw(),
                }
            }
        }
    }

    fn unsubscribe_listeners(&self, notifier: &Arc<RunNotifier>) {
        for l in self.auto_listeners() {
            notifier.remove_listener(&l);
        }
    }

    /// `getShuffledMethods(ann)`: the annotated leaf methods, each class's
    /// run shuffled with the master seed; reversed (superclass first) for
    /// `@Before` and `@BeforeClass`.
    fn get_shuffled_methods(&self, ann: &'static AnnotationType) -> Vec<Arc<Method>> {
        if let Some(m) = self
            .inner
            .shuffled_methods_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(ann.name)
        {
            return m.clone();
        }
        let mut methods: Vec<Arc<Method>> = self
            .inner
            .class_model
            .get_annotated_leaf_methods(ann)
            .into_iter()
            .map(|mm| Arc::clone(&mm.element))
            .collect();
        let rnd = JdkRandom::new(self.inner.runner_randomness.get_seed());
        let mut i = 0;
        while i < methods.len() {
            let mut j = i + 1;
            while j < methods.len() && methods[i].declaring_class() == methods[j].declaring_class()
            {
                j += 1;
            }
            if j - i > 1 {
                shuffle(&mut methods[i..j], &rnd);
            }
            i = j;
        }
        if ann.name == BEFORE.name || ann.name == BEFORE_CLASS.name {
            methods.reverse();
        }
        self.inner
            .shuffled_methods_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(ann.name, methods.clone());
        methods
    }

    /// `collectTestCandidates(classDescription)`: every candidate, with
    /// iterations, seeds and parameters expanded, in the master seed's
    /// order; adds their descriptions to the suite description.
    fn collect_test_candidates(
        &self,
        suite_description: &Arc<Description>,
    ) -> Vec<Arc<TestCandidate>> {
        let suite_class = &self.inner.suite_class;
        let providers: Vec<Arc<dyn TestMethodProvider>> = match suite_class
            .get_annotation::<TestMethodProviders>()
        {
            Some(ann) => ann
                .value
                .iter()
                .map(|c| match c.new_instance() {
                    Ok(p) => p,
                    Err(e) => Throwable::with_cause(
                        exception::RUNTIME_EXCEPTION,
                        Some("TestMethodProviders classes could not be instantiated.".to_string()),
                        e,
                    )
                    .throw(),
                })
                .collect(),
            None => vec![Arc::new(JUnit4MethodProvider)],
        };

        let mut all: Vec<Arc<Method>> = Vec::new();
        for p in &providers {
            for m in p.get_test_methods(suite_class, &self.inner.class_model) {
                if !all.iter().any(|x| Arc::ptr_eq(x, &m)) {
                    all.push(m);
                }
            }
        }
        all.sort_by(|a, b| {
            crate::annotations::jdk_string_compare(&a.to_generic_string(), &b.to_generic_string())
        });
        self.validate_test_methods(&all);
        shuffle(
            &mut all,
            &JdkRandom::new(self.inner.runner_randomness.get_seed()),
        );

        let constructors = suite_class.constructors();
        if constructors.len() != 1 {
            exception::runtime(format!(
                "There must be exactly one constructor: {}",
                constructors.len()
            ));
        }
        let constructor = constructors[0].clone();
        let mut test_cases = self.collect_method_executions(&constructor, &all);

        if let Some(order) = suite_class.get_annotation::<TestCaseOrdering>() {
            match order.value.new_instance() {
                Ok(cmp) => {
                    let cmp = Arc::clone(&cmp.0);
                    test_cases.sort_by(|a, b| cmp(a, b));
                }
                Err(e) => Throwable::with_cause(
                    exception::RUNTIME_EXCEPTION,
                    Some("Could not sort test methods.".to_string()),
                    e,
                )
                .throw(),
            }
        }

        let mut description_repetitions: HashMap<String, i32> = HashMap::new();
        let mut all_tests: Vec<Arc<TestCandidate>> = Vec::new();
        let mut same_method_variants: Vec<(Arc<Method>, Vec<Arc<TestCandidate>>)> = Vec::new();
        for tc in &test_cases {
            let variants = self.collect_candidates_for_method(&mut description_repetitions, tc);
            all_tests.extend(variants.iter().cloned());
            match same_method_variants
                .iter_mut()
                .find(|(m, _)| Arc::ptr_eq(m, &tc.method))
            {
                Some((_, v)) => v.extend(variants),
                None => same_method_variants.push((Arc::clone(&tc.method), variants)),
            }
        }
        for (m, candidates) in &same_method_variants {
            if candidates.len() > 1 {
                let parent = Description::create_suite_description(m.name(), Vec::new());
                suite_description.add_child(Arc::clone(&parent));
                for c in candidates {
                    parent.add_child(Arc::clone(&c.description));
                }
            } else {
                suite_description.add_child(Arc::clone(&candidates[0].description));
            }
        }
        all_tests
    }

    /// `collectCandidatesForMethod(...)`.
    fn collect_candidates_for_method(
        &self,
        description_repetitions: &mut HashMap<String, i32>,
        tc: &TestMethodExecution,
    ) -> Vec<Arc<TestCandidate>> {
        let method = &tc.method;
        let fixed_seed = self.is_constant_seed_for_all_iterations(method);
        let iterations = self.determine_method_iteration_count(method);
        let seeds = self.determine_method_seeds(method);
        let mut template = tc.arg_formatting_template.clone();
        if iterations > 1 || seeds.len() > 1 || self.inner.append_seed_parameter {
            template.push_str(&format!(" seed=%{}$s", tc.params.len() + 1));
        }
        let mut candidates = Vec::new();
        for &test_seed in &seeds {
            for i in 0..iterations {
                let this_seed = if fixed_seed {
                    test_seed
                } else {
                    test_seed ^ MurmurHash3::hash_long(i64::from(i))
                };
                let mut args = tc.params.clone();
                let chain = SeedUtils::format_seed_chain(&[
                    &self.inner.runner_randomness,
                    &Randomness::new(
                        this_seed,
                        Arc::clone(&self.inner.random_supplier),
                        Vec::new(),
                    ),
                ]);
                args.push(Value::Str(chain));
                let mut formatted = java_format(&template, &args);
                let key = format!("{}::{}", method.name(), formatted);
                let cnt = 1 + description_repetitions.get(&key).copied().unwrap_or(0);
                description_repetitions.insert(key, cnt);
                if cnt > 1 {
                    formatted.push_str(&format!(" #{cnt}"));
                }
                let trimmed = crate::jdk::lang::trim(&formatted).to_string();
                if !trimmed.is_empty() {
                    formatted = if self.inner.container_runner == RunnerContainer::Idea {
                        format!(" [{trimmed}]")
                    } else {
                        format!(" {{{trimmed}}}")
                    };
                }
                let description = Description::create_suite_description(
                    &format!(
                        "{}{}({})",
                        method.name(),
                        formatted,
                        self.inner.suite_class.name()
                    ),
                    method.annotations().to_vec(),
                );
                candidates.push(Arc::new(TestCandidate {
                    seed: this_seed,
                    description,
                    method: Arc::clone(method),
                    instance_provider: Arc::clone(&tc.instance_provider),
                    suite_class: Arc::clone(&self.inner.suite_class),
                }));
            }
        }
        candidates
    }

    /// `collectMethodExecutions(constructor, testMethods)`.
    fn collect_method_executions(
        &self,
        constructor: &Constructor,
        test_methods: &[Arc<Method>],
    ) -> Vec<TestMethodExecution> {
        let mut test_cases = Vec::new();
        let mut template = create_default_argument_formatting(constructor);
        let factory_methods: Vec<Arc<Method>> = self
            .inner
            .class_model
            .get_annotated_leaf_methods(&PARAMETERS_FACTORY)
            .into_iter()
            .map(|mm| Arc::clone(&mm.element))
            .collect();
        if factory_methods.is_empty() {
            let provider = self.get_instance_provider(constructor, Vec::new());
            for m in test_methods {
                test_cases.push(TestMethodExecution {
                    params: Vec::new(),
                    method: Arc::clone(m),
                    arg_formatting_template: template.clone(),
                    instance_provider: Arc::clone(&provider),
                });
            }
            return test_cases;
        }
        for factory in &factory_methods {
            Validation::check_method(factory).is_static().is_public();
            if !is_iterable(factory.raw_return_type()) {
                exception::runtime(format!(
                    "@ParametersFactory annotated methods must be public, static and returning \
                     Iterable<Object[]>:{}",
                    factory.to_java_string()
                ));
            }
            let pf = factory
                .get_annotation::<ParametersFactory>()
                .expect("@ParametersFactory");
            if pf.argument_formatting != ParametersFactory::DEFAULT_FORMATTING {
                template = pf.argument_formatting.clone();
            }
            let returned = match try_run(|| factory.invoke(None, &[])) {
                Ok(v) => v,
                Err(t) => {
                    if is_assumption_violated(&t) {
                        return Vec::new();
                    }
                    t.throw()
                }
            };
            let items: Vec<Value> = match returned {
                Value::List(v) | Value::Array(v) => v,
                Value::Null => Throwable::with_cause(
                    exception::RUNTIME_EXCEPTION,
                    Some(format!(
                        "Error collecting parameters from: {}",
                        factory.to_java_string()
                    )),
                    Throwable::with_message(exception::NULL_POINTER_EXCEPTION, None),
                )
                .throw(),
                other => vec![other],
            };
            let mut args: Vec<Vec<Value>> = Vec::new();
            for o in items {
                match o {
                    Value::Array(a) => args.push(a),
                    other => Throwable::with_cause(
                        exception::RUNTIME_EXCEPTION,
                        Some(format!(
                            "Error collecting parameters from: {}",
                            factory.to_java_string()
                        )),
                        Throwable::new(
                            exception::RUNTIME_EXCEPTION,
                            format!(
                                "Expected Object[] for each set of constructor arguments: {other}"
                            ),
                        ),
                    )
                    .throw(),
                }
            }
            if pf.shuffle {
                shuffle(
                    &mut args,
                    &JdkRandom::new(self.inner.runner_randomness.get_seed()),
                );
            }
            for constructor_args in args {
                let provider = self.get_instance_provider(constructor, constructor_args.clone());
                for m in test_methods {
                    test_cases.push(TestMethodExecution {
                        params: constructor_args.clone(),
                        method: Arc::clone(m),
                        arg_formatting_template: template.clone(),
                        instance_provider: Arc::clone(&provider),
                    });
                }
            }
        }
        test_cases
    }

    /// `getInstanceProvider(constructor, args)`.
    fn get_instance_provider(
        &self,
        constructor: &Constructor,
        args: Vec<Value>,
    ) -> Arc<dyn InstanceProvider> {
        let ty = self
            .inner
            .suite_class
            .get_annotation::<TestCaseInstanceProvider>()
            .map(|a| a.value)
            .unwrap_or_default();
        let new = NewInstanceProvider {
            constructor: constructor.clone(),
            args,
        };
        match ty {
            InstanceProviderType::InstancePerConstructorArgs => Arc::new(SameInstanceProvider {
                delegate: new,
                instance: Mutex::new(None),
            }),
            InstanceProviderType::InstancePerTestMethod => Arc::new(new),
        }
    }

    /// `isConstantSeedForAllIterations(method)`.
    fn is_constant_seed_for_all_iterations(&self, method: &Method) -> bool {
        if self.inner.test_case_randomness_override.is_some() {
            return true;
        }
        if let Some(r) = method.get_annotation::<Repeat>() {
            return r.use_constant_seed;
        }
        if let Some(r) = self.inner.suite_class.get_annotation::<Repeat>() {
            return r.use_constant_seed;
        }
        false
    }

    /// `determineMethodIterationCount(method)`: `tests.iters`, the method's
    /// `@Repeat`, the class's `@Repeat`, or 1.
    fn determine_method_iteration_count(&self, method: &Method) -> i32 {
        if let Some(i) = self.inner.iterations_override {
            return i;
        }
        if let Some(r) = method.get_annotation::<Repeat>() {
            return r.iterations;
        }
        if let Some(r) = self.inner.suite_class.get_annotation::<Repeat>() {
            return r.iterations;
        }
        DEFAULT_ITERATIONS
    }

    /// `determineMethodSeeds(method)`: the method's initial seeds, in the
    /// iteration order of Java's `HashSet<Long>`.
    fn determine_method_seeds(&self, method: &Arc<Method>) -> Vec<i64> {
        if let Some(o) = &self.inner.test_case_randomness_override {
            return vec![o.get_seed()];
        }
        let random_seed = self.inner.runner_randomness.get_seed()
            ^ MurmurHash3::hash_long(i64::from(crate::jdk::lang::string_hash_code(method.name())));
        let mut seeds: Vec<i64> = Vec::new();
        if let Some(seed) = method.get_annotation::<Seed>() {
            seeds.extend(seed_from_annot(seed, random_seed));
        }
        if let Some(seeds_value) = self.inner.class_model.get_annotation::<Seeds>(method, true) {
            for s in &seeds_value.value {
                if s.value == "random" {
                    seeds.push(random_seed);
                } else {
                    seeds.extend(SeedUtils::parse_seed_chain(&s.value));
                }
            }
        }
        if seeds.is_empty() {
            if let Some(seed) = self.inner.suite_class.get_annotation::<Seed>() {
                if seed.value != "random" {
                    let chain = SeedUtils::parse_seed_chain(&seed.value);
                    if chain.len() > 1 {
                        seeds.push(chain[1]);
                    }
                }
            }
        }
        if seeds.is_empty() {
            seeds.push(random_seed);
        }
        let keyed: Vec<(i64, i32)> = seeds.iter().map(|&s| (s, long_hash_code(s))).collect();
        hash_iteration_order(&keyed, |a, b| a == b)
            .into_iter()
            .map(|i| seeds[i])
            .collect()
    }

    /// `validateTestMethods(testMethods)`.
    fn validate_test_methods(&self, test_methods: &[Arc<Method>]) {
        let suite = &self.inner.suite_class;
        let mut parents: Vec<&str> = Vec::new();
        let mut c: Option<&Class> = Some(suite);
        while let Some(cls) = c {
            parents.push(cls.name());
            c = cls.superclass().map(|s| s.as_ref());
        }
        for method in test_methods {
            if !parents.contains(&method.declaring_class()) {
                exception::illegal_argument(format!(
                    "Test method does not belong to test suite class hierarchy: class {}#{}",
                    method.declaring_class(),
                    method.name()
                ));
            }
            Validation::check_method(method)
                .described_as(format!("Test method {}#{}", suite.name(), method.name()))
                .is_public()
                .is_not_static()
                .has_args_count(0);
            let test_ann = self.inner.class_model.get_annotation::<Test>(method, true);
            if test_ann.is_some_and(|t| t.timeout > 0)
                && self
                    .inner
                    .class_model
                    .is_annotation_present::<Timeout>(method, true)
            {
                exception::illegal_argument(format!(
                    "Conflicting @Test(timeout=...) and @Timeout annotations in: {}#{}",
                    suite.name(),
                    method.name()
                ));
            }
            if let Some(seed) = self.inner.class_model.get_annotation::<Seed>(method, true) {
                let r = try_run(|| {
                    if seed.value != "random" {
                        let chain = SeedUtils::parse_seed_chain(&seed.value);
                        if chain.len() > 1 {
                            exception::illegal_argument(
                                "@Seed on methods must contain one seed only (no runner seed).",
                            );
                        }
                    }
                });
                if let Err(e) = r {
                    if e.is_instance(exception::ILLEGAL_ARGUMENT_EXCEPTION) {
                        exception::runtime(format!(
                            "@Seed annotation invalid on method {}, in class {}: {}",
                            method.name(),
                            suite.name(),
                            e.message().unwrap_or("null")
                        ));
                    }
                    e.throw();
                }
            }
        }
    }

    /// `validateTarget()`: the suite class, its constructor, its parameter
    /// factories and its hooks follow the runner's rules.
    fn validate_target(&self) {
        let suite = &self.inner.suite_class;
        Validation::check_class(suite)
            .described_as(format!("Suite class {}", suite.name()))
            .is_public()
            .is_concrete_class();
        let constructors = suite.constructors();
        if constructors.len() != 1 || constructors[0].get_modifiers() & modifier::PUBLIC == 0 {
            exception::runtime(format!(
                "A test class is expected to have one public constructor  (parameterless or with \
                 types matching static @{}-annotated method's output): {}",
                PARAMETERS_FACTORY_CLASS,
                suite.name()
            ));
        }
        if !constructors[0].parameter_types().is_empty() {
            let factories = self
                .inner
                .class_model
                .get_annotated_leaf_methods(&PARAMETERS_FACTORY);
            if factories.is_empty() {
                exception::runtime(format!(
                    "A test class with a parameterized constructor is expected  to have a static \
                     @{}-annotated method: {}",
                    PARAMETERS_FACTORY_CLASS,
                    suite.name()
                ));
            }
            for mm in factories {
                Validation::check_method(&mm.element)
                    .described_as(format!(
                        "@ParametersFactory method {}#{}",
                        suite.name(),
                        mm.element.name()
                    ))
                    .is_static()
                    .is_public()
                    .has_args_count(0)
                    .has_return_type("java.lang.Iterable");
            }
        }
        let checks: [(&AnnotationType, &str, bool); 4] = [
            (&BEFORE_CLASS, "@BeforeClass", true),
            (&AFTER_CLASS, "@AfterClass", true),
            (&BEFORE, "@Before", false),
            (&AFTER, "@After", false),
        ];
        for (ann, label, is_static) in checks {
            for mm in self.inner.class_model.get_annotated_leaf_methods(ann) {
                let v = Validation::check_method(&mm.element).described_as(format!(
                    "{} method {}#{}",
                    label,
                    suite.name(),
                    mm.element.name()
                ));
                let v = if is_static {
                    v.is_static()
                } else {
                    v.is_not_static()
                };
                v.has_args_count(0);
            }
        }
    }
}

/// `ParametersFactory.class.toString()`.
const PARAMETERS_FACTORY_CLASS: &str =
    "interface com.carrotsearch.randomizedtesting.annotations.ParametersFactory";

fn is_iterable(raw_return_type: &str) -> bool {
    matches!(
        raw_return_type,
        "java.lang.Iterable"
            | "java.util.Collection"
            | "java.util.List"
            | "java.util.ArrayList"
            | "java.util.Set"
            | "java.util.LinkedList"
    )
}

/// `seedFromAnnot(element, randomSeed)`.
fn seed_from_annot(seed: &Seed, random_seed: i64) -> Vec<i64> {
    if seed.value == "random" {
        vec![random_seed]
    } else {
        SeedUtils::parse_seed_chain(&seed.value)
    }
}

/// `createDefaultArgumentFormatting(constructor)`: `p0=%s p1=%s …`, a
/// parameter's `@Name` in place of `pN`.
fn create_default_argument_formatting(constructor: &Constructor) -> String {
    let mut b = String::new();
    for i in 0..constructor.parameter_types().len() {
        let name = constructor
            .parameter_name(i)
            .map(str::to_string)
            .unwrap_or_else(|| format!("p{i}"));
        if i > 0 {
            b.push(' ');
        }
        b.push_str(&name);
        b.push_str("=%s");
    }
    b
}

/// `applyFilters(suiteClass, testCandidates, testFilters)`: a candidate
/// runs when every filter accepts its full description or its plain
/// `method(class)` description.
fn apply_filters(
    suite_class: &Arc<Class>,
    candidates: &[Arc<TestCandidate>],
    filters: &[&dyn Filter],
) -> Vec<Arc<TestCandidate>> {
    candidates
        .iter()
        .filter(|c| {
            filters.iter().all(|f| {
                f.should_run(&c.description)
                    || f.should_run(&Description::create_test_description(
                        suite_class,
                        c.method.name(),
                        Vec::new(),
                    ))
            })
        })
        .cloned()
        .collect()
}

/// `prune(suite, permitted)`: a copy of the tree with only the permitted
/// tests and the suites that still hold children.
fn prune(suite: &Arc<Description>, permitted: &[Arc<Description>]) -> Arc<Description> {
    if suite.is_suite() {
        let mut retained = Vec::new();
        for child in suite.children() {
            if child.is_suite() {
                let d = prune(&child, permitted);
                if !child.children().is_empty() {
                    retained.push(d);
                }
            } else if permitted.iter().any(|p| Arc::ptr_eq(p, &child)) {
                retained.push(child);
            }
        }
        let copy = suite.childless_copy();
        for d in retained {
            copy.add_child(d);
        }
        return copy;
    }
    Arc::clone(suite)
}

/// `fireTestFailure(notifier, description, t)`: a
/// `MultipleFailureException` is reported failure by failure.
fn fire_test_failure(notifier: &dyn Notifier, description: &Arc<Description>, t: Throwable) {
    if t.is_multiple_failure() {
        for nested in t.failures() {
            fire_test_failure(notifier, description, nested.clone());
        }
    } else {
        notifier.fire_test_failure(Failure::new(Arc::clone(description), t));
    }
}

/// `wrapExpectedExceptions(s, c)`: with `@Test(expected = X)`, passes only
/// when the test throws an `X`.
fn wrap_expected_exceptions(s: Statement, c: &TestCandidate) -> Statement {
    let Some(ann) = c.method.get_annotation::<Test>() else {
        return s;
    };
    let Some(expected) = ann.expected else {
        return s;
    };
    Statement::new(move || {
        if let Err(t) = try_run(|| s.evaluate()) {
            if !t.is_instance(expected) {
                t.throw();
            }
            return;
        }
        Assert::fail(&format!(
            "Expected an exception but the test passed: {expected}"
        ));
    })
}

/// `withCloseContextResources(s, scope)`: closes the context's resources
/// of `scope` after `s`, a failure to close becoming a
/// `ResourceDisposalError`.
fn with_close_context_resources(s: Statement, scope: LifecycleScope) -> Statement {
    StatementAdapter::new(s)
        .after_always(move |errors| {
            RandomizedContext::current().close_resources(
                |info: crate::closeable_resource_info::CloseableResourceInfo| {
                    if let Err(e) = info.get_resource().close() {
                        let mut err = crate::resource_disposal_error::ResourceDisposalError::new(
                            format!(
                                "Resource in scope {} failed to close. Resource was registered \
                                 from thread {}, registration stack trace below.",
                                info.get_scope().name(),
                                info.get_thread_name()
                            ),
                            Box::new(e),
                        );
                        err.set_stack_trace(std::sync::Arc::clone(info.get_allocation_stack()));
                        errors.push(err.to_throwable());
                    }
                },
                scope,
            );
        })
        .build()
}

impl crate::junit::Runner for RandomizedRunner {
    fn description(&self) -> Arc<Description> {
        RandomizedRunner::description(self)
    }

    fn run(&self, notifier: &Arc<RunNotifier>) {
        RandomizedRunner::run(self, notifier)
    }
}

/// Runs `classes` with [`JUnitCore`](crate::junit::JUnitCore) as a test
/// target's `main` would: prints each failure (with its reproduce line, as
/// [`ReproduceInfoPrinter`](crate::listeners::ReproduceInfoPrinter) does)
/// and a summary, and exits with status 1 when any test failed.
pub fn run_main(classes: &[Arc<Class>]) {
    struct Summary;
    impl RunListener for Summary {
        fn test_failure(&self, f: &Failure) {
            system::err_println(&format!("FAILED {}: {}", f.test_header(), f.exception()));
        }
    }
    let mut failed = 0;
    let mut run = 0;
    for c in classes {
        let mut core = crate::junit::JUnitCore::new();
        core.add_listener(Arc::new(Summary));
        core.add_listener(Arc::new(crate::listeners::ReproduceInfoPrinter));
        let r = core.run_class(c);
        failed += r.failure_count();
        run += r.run_count();
    }
    system::out_println(&format!("tests run: {run}, failures: {failed}"));
    if failed > 0 {
        std::process::exit(1);
    }
}
