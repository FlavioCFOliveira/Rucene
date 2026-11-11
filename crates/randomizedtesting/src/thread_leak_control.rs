//! Port of `com.carrotsearch.randomizedtesting.ThreadLeakControl`
//! (randomizedtesting 2.8.1, `ThreadLeakControl.java`), with
//! `ThreadLeakError` and `UncaughtExceptionError`: thread-leak detection,
//! test and suite timeouts, and uncaught exceptions as failures.
//!
//! **Divergences.**
//! * Leaked threads are interrupted (`ThreadLeakControl.java:793`) through
//!   the interruption model of
//!   [`crate::jdk::thread`]: a thread that does not block in that module
//!   (or check its flag) cannot be interrupted, as a Java thread spinning
//!   without checking `isInterrupted` cannot.
//! * A thread's stack trace is empty (see [`crate::jdk::thread`]), so the
//!   leak messages show `at (empty stack)` where Java lists frames, and the
//!   timeout warnings use the thread list instead of a JVM thread dump
//!   (`ThreadLeakControl.java:704`).
//! * The set of threads is Java's `HashSet<Thread>`, whose iteration order
//!   (identity hashes) is not reproducible (`ThreadLeakControl.java:732`,
//!   `:782`); the port iterates in start order.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use crate::annotations::{
    Action, Consequence, Group, Scope, ThreadLeakAction, ThreadLeakFilters, ThreadLeakGroup,
    ThreadLeakLingering, ThreadLeakScope, ThreadLeakZombies, Timeout, TimeoutSuite,
};
use crate::deadline_clock::DeadlineClock;
use crate::jdk::exception::{self, Throwable};
use crate::jdk::lang::parse_int;
use crate::jdk::reflect::{find_annotation, AnnotationRef, Class, JavaAnnotation};
use crate::jdk::system;
use crate::jdk::thread::Thread;
use crate::junit::{
    Description, Failure, Notifier, RunNotifier, Statement, Test, ASSUMPTION_VIOLATED,
};
use crate::lifecycle_scope::LifecycleScope;
use crate::randomized_context::RandomizedContext;
use crate::randomized_runner::{
    assert_empty, augment_stack_trace, try_run, RandomizedRunner, TestCandidate, UncaughtException,
    DEFAULT_KILLATTEMPTS, DEFAULT_KILLWAIT, DEFAULT_TIMEOUT, DEFAULT_TIMEOUT_SUITE,
    MAIN_THREAD_GROUP, ZOMBIE_MARKER,
};
use crate::randomized_test::RandomizedTest;
use crate::sys_globals::SysGlobals;
use crate::thread_filter::ThreadFilter;
use crate::thread_leak_error::ThreadLeakError;
use crate::threads::Threads;
use crate::uncaught_exception_error::UncaughtExceptionError;

/// `DefaultAnnotationValues`: the annotations' defaults.
static DEFAULTS: LazyLock<Vec<AnnotationRef>> = LazyLock::new(|| {
    vec![
        Arc::new(ThreadLeakScope::default()),
        Arc::new(ThreadLeakAction::default()),
        Arc::new(ThreadLeakLingering::default()),
        Arc::new(ThreadLeakZombies::default()),
        Arc::new(ThreadLeakFilters::default()),
        Arc::new(ThreadLeakGroup::default()),
    ]
});

/// One element of an annotation chain (`AnnotatedElement`).
enum Element<'a> {
    Method(&'a crate::jdk::reflect::Method),
    Class(&'a Class),
    Defaults,
}

/// `firstAnnotated(clazz, elements...)`: the annotation of the first
/// element that has one (the defaults always do).
fn first_annotated<A: JavaAnnotation + Clone>(elements: &[Element<'_>]) -> A {
    for e in elements {
        let found = match e {
            Element::Method(m) => m.get_annotation::<A>().cloned(),
            Element::Class(c) => c.get_annotation::<A>(),
            Element::Defaults => find_annotation::<A>(&DEFAULTS).cloned(),
        };
        if let Some(a) = found {
            return a;
        }
    }
    exception::runtime("default annotation value must be within elements.")
}

/// `TimeoutValue`: a timeout property, with `!` giving it precedence over
/// annotations.
struct TimeoutValue {
    timeout_override: i32,
    global_timeout_first: bool,
}

impl TimeoutValue {
    fn new(sysprop: &str, default_value: i32) -> TimeoutValue {
        let mut value =
            system::get_property(sysprop).filter(|v| !crate::jdk::lang::trim(v).is_empty());
        let mut global_first = false;
        match &mut value {
            Some(v) => {
                global_first = v.len() > 1
                    && v.ends_with('!')
                    && v[..v.len() - 1].chars().all(|c| c.is_ascii_digit());
                *v = v.replace('!', "");
            }
            None => value = Some(default_value.to_string()),
        }
        let v = value.expect("timeout value");
        let timeout_override = match parse_int(&v) {
            Some(i) => i,
            None => exception::throw(
                "java.lang.NumberFormatException",
                format!("For input string: \"{v}\""),
            ),
        };
        TimeoutValue {
            timeout_override,
            global_timeout_first: global_first,
        }
    }

    fn get_timeout(&self, value: Option<i32>) -> i32 {
        if self.global_timeout_first {
            self.timeout_override
        } else {
            value.unwrap_or(self.timeout_override)
        }
    }
}

/// `ThisThreadFilter`.
struct ThisThreadFilter(Thread);

impl ThreadFilter for ThisThreadFilter {
    fn reject(&self, t: &Thread) -> bool {
        self.0 == *t
    }
}

/// `KnownSystemThread`: the JVM's system threads.
struct KnownSystemThread;

impl ThreadFilter for KnownSystemThread {
    fn reject(&self, t: &Thread) -> bool {
        if let Some(g) = t.thread_group() {
            if g.name() == "system" && g.parent().is_none() {
                return true;
            }
        }
        let name = t.name();
        matches!(
            name.as_str(),
            "JUnit4-serializer-daemon"
                | "JFR request timer"
                | "YJPAgent-Telemetry"
                | "MemoryPoolMXBean notification dispatcher"
                | "AWT-AppKit"
                | "process reaper"
        ) || name.contains("Poller SunPKCS11")
    }
}

/// `or(filters...)`.
struct OrFilter(Vec<Arc<dyn ThreadFilter>>);

impl ThreadFilter for OrFilter {
    fn reject(&self, t: &Thread) -> bool {
        self.0.iter().any(|f| f.reject(t))
    }
}

/// The sub-notifier that stops passing events after a suite timeout
/// (`SubNotifier`).
struct SubNotifier {
    target: Arc<RunNotifier>,
    lock: Arc<Mutex<()>>,
    state: Mutex<SubState>,
}

#[derive(Default)]
struct SubState {
    stop_requested: bool,
    test_in_progress: Option<Arc<Description>>,
}

impl SubNotifier {
    fn state(&self) -> std::sync::MutexGuard<'_, SubState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Notifier for SubNotifier {
    fn fire_test_started(&self, description: &Arc<Description>) {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if self.state().stop_requested {
            return;
        }
        self.target.fire_test_started(description);
        self.state().test_in_progress = Some(Arc::clone(description));
    }

    fn fire_test_failure(&self, failure: Failure) {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if self.state().stop_requested {
            return;
        }
        self.target.fire_test_failure(failure);
    }

    fn fire_test_assumption_failed(&self, failure: Failure) {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if self.state().stop_requested {
            return;
        }
        self.target.fire_test_assumption_failed(failure);
    }

    fn fire_test_ignored(&self, description: &Arc<Description>) {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if self.state().stop_requested {
            return;
        }
        self.state().test_in_progress = None;
        self.target.fire_test_ignored(description);
    }

    fn fire_test_finished(&self, description: &Arc<Description>) {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if self.state().stop_requested {
            return;
        }
        self.state().test_in_progress = None;
        self.target.fire_test_finished(description);
    }
}

/// Everything about thread leak control (`ThreadLeakControl`,
/// package-private).
pub(crate) struct ThreadLeakControl {
    kill_attempts: i32,
    kill_wait: i32,
    target_notifier: Arc<RunNotifier>,
    expected_suite_state: Vec<Thread>,
    notifier_lock: Arc<Mutex<()>>,
    sub_notifier: Arc<SubNotifier>,
    test_timeout: TimeoutValue,
    suite_timeout: TimeoutValue,
    builtin_filters: Vec<Arc<dyn ThreadFilter>>,
    suite_filters: Mutex<Arc<dyn ThreadFilter>>,
    runner: RandomizedRunner,
    suite_timed_out: AtomicBool,
    thread_leak_group: Mutex<ThreadLeakGroup>,
}

/// `StatementRunner`: runs a statement and keeps its error and completion.
#[derive(Default)]
struct StatementRunner {
    error: Mutex<Option<Throwable>>,
    completed: AtomicBool,
}

impl StatementRunner {
    fn run(&self, s: &Statement) {
        if let Err(t) = try_run(|| s.evaluate()) {
            *self.error.lock().unwrap_or_else(|e| e.into_inner()) = Some(t);
        }
        self.completed.store(true, Ordering::Release);
    }
}

fn empty_stack(mut t: Throwable) -> Throwable {
    t.set_stack_trace(Vec::new());
    t
}

impl ThreadLeakControl {
    /// `new ThreadLeakControl(notifier, runner)`.
    pub(crate) fn new(
        notifier: Arc<RunNotifier>,
        runner: RandomizedRunner,
    ) -> Arc<ThreadLeakControl> {
        let lock = Arc::new(Mutex::new(()));
        let kill_attempts = RandomizedTest::system_property_as_int(
            &SysGlobals::sysprop_killattempts(),
            DEFAULT_KILLATTEMPTS,
        );
        let kill_wait = RandomizedTest::system_property_as_int(
            &SysGlobals::sysprop_killwait(),
            DEFAULT_KILLWAIT,
        );
        let test_timeout = TimeoutValue::new(&SysGlobals::sysprop_timeout(), DEFAULT_TIMEOUT);
        let suite_timeout =
            TimeoutValue::new(&SysGlobals::sysprop_timeout_suite(), DEFAULT_TIMEOUT_SUITE);
        let builtin: Vec<Arc<dyn ThreadFilter>> = vec![
            Arc::new(ThisThreadFilter(Thread::current())),
            Arc::new(KnownSystemThread),
        ];
        Arc::new(ThreadLeakControl {
            kill_attempts,
            kill_wait,
            target_notifier: Arc::clone(&notifier),
            expected_suite_state: Threads::get_all_threads(),
            notifier_lock: Arc::clone(&lock),
            sub_notifier: Arc::new(SubNotifier {
                target: notifier,
                lock,
                state: Mutex::new(SubState::default()),
            }),
            test_timeout,
            suite_timeout,
            builtin_filters: builtin,
            suite_filters: Mutex::new(Arc::new(OrFilter(Vec::new()))),
            runner,
            suite_timed_out: AtomicBool::new(false),
            thread_leak_group: Mutex::new(ThreadLeakGroup::default()),
        })
    }

    fn suite_filters(&self) -> Arc<dyn ThreadFilter> {
        Arc::clone(&self.suite_filters.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// `checkZombies()`.
    fn check_zombies() {
        if RandomizedRunner::has_zombie_threads() {
            Throwable::new(
                ASSUMPTION_VIOLATED,
                "Leaked background threads present (zombies).",
            )
            .throw();
        }
    }

    /// `forSuite(s, suiteDescription)`.
    pub(crate) fn for_suite(
        self: &Arc<Self>,
        s: Statement,
        suite_description: Arc<Description>,
    ) -> Statement {
        let suite_class = RandomizedContext::current()
            .get_target_class_object()
            .expect("the context's suite class");
        let timeout = self.determine_suite_timeout(&suite_class);
        let me = Arc::clone(self);
        Statement::new(move || {
            ThreadLeakControl::check_zombies();
            *me.thread_leak_group
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = first_annotated::<ThreadLeakGroup>(&[
                Element::Class(&suite_class),
                Element::Defaults,
            ]);
            let mut errors = Vec::new();
            let filters = me.instantiate_filters(&mut errors, &suite_class);
            *me.suite_filters.lock().unwrap_or_else(|e| e.into_inner()) = filters;
            assert_empty(std::mem::take(&mut errors));

            let sr = Arc::new(StatementRunner::default());
            let timed_out = me.fork_timeouting_task(&sr, &s, timeout, &mut errors);
            {
                let _g = me.notifier_lock.lock().unwrap_or_else(|e| e.into_inner());
                if timed_out {
                    me.suite_timed_out.store(true, Ordering::Release);
                    RandomizedRunner::logger().warning(&format!(
                        "Suite execution timed out: {}{}",
                        suite_description,
                        me.format_thread_stacks_full()
                    ));
                    me.sub_notifier.state().stop_requested = true;
                }
            }
            if timed_out {
                let in_progress = me.sub_notifier.state().test_in_progress.clone();
                if let Some(t) = in_progress {
                    me.target_notifier.fire_test_failure(Failure::new(
                        Arc::clone(&t),
                        augment_stack_trace(
                            empty_stack(Throwable::new(
                                exception::EXCEPTION,
                                "Test abandoned because suite timeout was reached.",
                            )),
                            &[],
                        ),
                    ));
                    crate::junit::Notifier::fire_test_finished(me.target_notifier.as_ref(), &t);
                }
                errors.push(augment_stack_trace(
                    empty_stack(Throwable::new(
                        exception::EXCEPTION,
                        format!("Suite timeout exceeded (>= {timeout} msec)."),
                    )),
                    &[],
                ));
            }
            let chain = [Element::Class(&suite_class), Element::Defaults];
            let expected = me.refilter(&me.expected_suite_state, &me.suite_filters());
            let mut leak_errors = Vec::new();
            me.check_thread_leaks(
                &expected,
                if timed_out {
                    &mut leak_errors
                } else {
                    &mut errors
                },
                LifecycleScope::Suite,
                &suite_description,
                &chain,
            );
            me.process_uncaught(&mut errors, me.handler_uncaught());
            assert_empty(errors);
        })
    }

    fn handler_uncaught(&self) -> Vec<UncaughtException> {
        self.runner
            .inner
            .handler
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|h| h.get_uncaught_and_clear())
            .unwrap_or_default()
    }

    /// `forTest(s, c)`.
    pub(crate) fn for_test(self: &Arc<Self>, s: Statement, c: Arc<TestCandidate>) -> Statement {
        let timeout = self.determine_test_timeout(&c);
        let me = Arc::clone(self);
        Statement::new(move || {
            ThreadLeakControl::check_zombies();
            let sr = Arc::new(StatementRunner::default());
            let mut errors = Vec::new();
            let before_test_state = me.get_threads(&[me.suite_filters()]);
            let timed_out = me.fork_timeouting_task(&sr, &s, timeout, &mut errors);
            if me.suite_timed_out.load(Ordering::Acquire) {
                return;
            }
            if timed_out {
                RandomizedRunner::logger().warning(&format!(
                    "Test execution timed out: {}{}",
                    c.description,
                    me.format_thread_stacks_full()
                ));
                errors.push(augment_stack_trace(
                    empty_stack(Throwable::new(
                        exception::EXCEPTION,
                        format!("Test timeout exceeded (>= {timeout} msec)."),
                    )),
                    &[],
                ));
            }
            let chain = [
                Element::Method(&c.method),
                Element::Class(c.test_class()),
                Element::Defaults,
            ];
            let mut leak_errors = Vec::new();
            me.check_thread_leaks(
                &before_test_state,
                if timed_out {
                    &mut leak_errors
                } else {
                    &mut errors
                },
                LifecycleScope::Test,
                &c.description,
                &chain,
            );
            me.process_uncaught(&mut errors, me.handler_uncaught());
            assert_empty(errors);
        })
    }

    /// `refilter(in, f)`.
    fn refilter(&self, threads: &[Thread], f: &Arc<dyn ThreadFilter>) -> Vec<Thread> {
        threads.iter().filter(|t| !f.reject(t)).cloned().collect()
    }

    /// `instantiateFilters(errors, suiteClass)`.
    fn instantiate_filters(
        &self,
        errors: &mut Vec<Throwable>,
        suite_class: &Class,
    ) -> Arc<dyn ThreadFilter> {
        let ann =
            first_annotated::<ThreadLeakFilters>(&[Element::Class(suite_class), Element::Defaults]);
        let mut filters: Vec<Arc<dyn ThreadFilter>> = Vec::new();
        for c in &ann.filters {
            match c.new_instance() {
                Ok(f) => filters.push(f),
                Err(t) => errors.push(t),
            }
        }
        if ann.default_filters {
            filters.extend(self.builtin_filters.iter().cloned());
        }
        Arc::new(OrFilter(filters))
    }

    /// `processUncaught(errors, uncaughtList)`.
    fn process_uncaught(&self, errors: &mut Vec<Throwable>, uncaught: Vec<UncaughtException>) {
        for e in uncaught {
            errors.push(empty_stack(UncaughtExceptionError::new(
                format!(
                    "Captured an uncaught exception in thread: {}",
                    e.thread_name
                ),
                e.error,
            )));
        }
    }

    /// `checkThreadLeaks(expectedState, errors, scope, description, chain)`.
    fn check_thread_leaks(
        &self,
        expected_state: &[Thread],
        errors: &mut Vec<Throwable>,
        scope: LifecycleScope,
        description: &Description,
        chain: &[Element<'_>],
    ) {
        let ann_scope = first_annotated::<ThreadLeakScope>(chain);
        if ann_scope.value == Scope::None {
            return;
        }
        if ann_scope.value == Scope::Suite && scope == LifecycleScope::Test {
            return;
        }
        let linger = first_annotated::<ThreadLeakLingering>(chain).linger;
        let leaked = |me: &Self| -> Vec<Thread> {
            me.get_threads(&[me.suite_filters()])
                .into_iter()
                .filter(|t| !expected_state.contains(t))
                .collect()
        };
        let mut threads = leaked(self);
        if linger > 0 && !threads.is_empty() {
            let clock = DeadlineClock::millis(i64::from(linger));
            RandomizedRunner::logger().warning(&format!(
                "Will linger awaiting termination of {} leaked thread(s).",
                threads.len()
            ));
            loop {
                if Thread::try_sleep(100).is_err() {
                    RandomizedRunner::logger().warning("Lingering interrupted.");
                    break;
                }
                threads = leaked(self);
                if threads.is_empty() || clock.is_after_deadline() {
                    break;
                }
            }
        }
        if threads.is_empty() {
            return;
        }
        let with_traces = leaked(self);
        if with_traces.is_empty() {
            return;
        }
        let mut message = format!(
            "{} thread{} leaked from {} scope at {}: ",
            with_traces.len(),
            if with_traces.len() == 1 { "" } else { "s" },
            scope,
            description
        );
        message.push_str(&format_thread_stacks(&with_traces));
        errors.push(augment_stack_trace(
            empty_stack(ThreadLeakError::new(message.clone())),
            &[],
        ));
        let actions: BTreeSet<Action> = first_annotated::<ThreadLeakAction>(chain)
            .value
            .into_iter()
            .collect();
        if actions.contains(&Action::Warn) {
            RandomizedRunner::logger().severe(&message);
        }
        let mut zombies = Vec::new();
        if actions.contains(&Action::Interrupt) {
            zombies = self.try_to_interrupt_all(errors, with_traces);
        }
        if !zombies.is_empty() {
            match first_annotated::<ThreadLeakZombies>(chain).value {
                Consequence::Continue => {}
                Consequence::IgnoreRemainingTests => ZOMBIE_MARKER.store(true, Ordering::Release),
            }
        }
    }

    /// `formatThreadStacksFull()`: the port has no JVM thread dump, so this
    /// is `formatThreadStacks(getThreadsWithTraces())`, Java's fallback.
    fn format_thread_stacks_full(&self) -> String {
        format_thread_stacks(&self.get_threads(&[]))
    }

    /// `getThreads(filters...)`: the live threads of the leak group, minus
    /// the rejected ones.
    fn get_threads(&self, filters: &[Arc<dyn ThreadFilter>]) -> Vec<Thread> {
        let group = self
            .thread_leak_group
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .value;
        let threads = match group {
            Group::All => Threads::get_all_threads(),
            Group::Main => Threads::get_threads(&MAIN_THREAD_GROUP),
            Group::TestGroup => match self
                .runner
                .inner
                .runner_thread_group
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
            {
                Some(g) => Threads::get_threads(&g),
                None => Vec::new(),
            },
        };
        let filter = OrFilter(filters.to_vec());
        threads
            .into_iter()
            .filter(|t| t.is_alive() && !filter.reject(t))
            .collect()
    }

    /// `tryToInterruptAll(errors, threads)`: interrupts the threads up to
    /// `killAttempts` times; returns those still alive (zombies).
    fn try_to_interrupt_all(
        &self,
        errors: &mut Vec<Throwable>,
        threads: Vec<Thread>,
    ) -> Vec<Thread> {
        RandomizedRunner::logger().info(&format!(
            "Starting to interrupt leaked threads:{}",
            thread_names(&threads)
        ));
        let handler = self
            .runner
            .inner
            .handler
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(h) = &handler {
            h.stop_reporting();
        }
        let mut ordered = threads;
        let mut interrupt_attempts = self.kill_attempts;
        let me = Thread::current();
        let restore_priority = me.priority();
        loop {
            let mut all_dead = true;
            me.set_priority(Thread::MAX_PRIORITY);
            for t in &ordered {
                t.interrupt();
            }
            let clock = DeadlineClock::millis(i64::from(self.kill_wait));
            let mut interrupted = false;
            let mut i = 0;
            while i < ordered.len() {
                if ordered[i].is_alive() {
                    all_dead = false;
                    let wait = clock.time_until_deadline_millis().max(1);
                    if join(&ordered[i], wait, &mut Thread::try_sleep).is_err() {
                        interrupted = true;
                        break;
                    }
                    i += 1;
                } else {
                    ordered.remove(i);
                }
            }
            if interrupted {
                interrupt_attempts = 0;
            }
            interrupt_attempts -= 1;
            if all_dead || interrupt_attempts <= 0 {
                break;
            }
        }
        me.set_priority(restore_priority);
        let zombies: Vec<Thread> = ordered.into_iter().filter(|t| t.is_alive()).collect();
        if zombies.is_empty() {
            RandomizedRunner::logger().info("All leaked threads terminated.");
        } else {
            let message = format!(
                "There are still zombie threads that couldn't be terminated:{}",
                format_thread_stacks(&zombies)
            );
            RandomizedRunner::logger().severe(&message);
            errors.push(augment_stack_trace(
                empty_stack(ThreadLeakError::new(message)),
                &[],
            ));
        }
        if let Some(h) = &handler {
            h.resume_reporting();
        }
        zombies
    }

    /// `forkTimeoutingTask(r, timeout, errors)`: runs the statement here,
    /// or with a timeout in a `-worker` thread; returns whether it timed
    /// out.
    fn fork_timeouting_task(
        &self,
        r: &Arc<StatementRunner>,
        s: &Statement,
        timeout: i32,
        errors: &mut Vec<Throwable>,
    ) -> bool {
        if timeout == 0 {
            r.run(s);
        } else {
            let owner = Thread::current();
            let done = Arc::new(AtomicBool::new(false));
            let (r2, s2, d2, o2) = (Arc::clone(r), s.clone(), Arc::clone(&done), owner.clone());
            let t = Thread::new(format!("{}-worker", owner.name()), move || {
                r2.run(&s2);
                d2.store(true, Ordering::Release);
                o2.unpark();
            });
            RandomizedContext::clone_for(&t);
            t.start();
            let t2 = t.clone();
            let _ = join(&t, i64::from(timeout), &mut |millis| {
                Thread::park_nanos(millis.saturating_mul(1_000_000));
                if done.load(Ordering::Acquire) {
                    let _ = t2.join();
                }
                Ok(())
            });
        }
        let timed_out = !r.completed.load(Ordering::Acquire);
        if let Some(e) = r.error.lock().unwrap_or_else(|e| e.into_inner()).take() {
            errors.push(e);
        }
        timed_out
    }

    /// `isTimedOut()`.
    pub(crate) fn is_timed_out(&self) -> bool {
        self.suite_timed_out.load(Ordering::Acquire)
    }

    /// `notifier()`: the notifier for the instance-scope statements.
    pub(crate) fn notifier(&self) -> Arc<dyn Notifier> {
        self.sub_notifier.clone()
    }

    /// `determineTimeout(suiteClass)`.
    fn determine_suite_timeout(&self, suite_class: &Class) -> i32 {
        let ann = suite_class.get_annotation::<TimeoutSuite>();
        self.suite_timeout.get_timeout(ann.map(|a| a.millis))
    }

    /// `determineTimeout(c)`: the class's `@Timeout`, then `@Test(timeout)`,
    /// then the method's `@Timeout`.
    fn determine_test_timeout(&self, c: &TestCandidate) -> i32 {
        let mut timeout: Option<i32> = None;
        if let Some(a) = c.test_class().get_annotation::<Timeout>() {
            timeout = Some(a.millis);
        }
        if let Some(t) = c.method.get_annotation::<Test>() {
            if t.timeout > 0 {
                timeout = Some(t.timeout.min(i64::from(i32::MAX)) as i32);
            }
        }
        if let Some(a) = c.method.get_annotation::<Timeout>() {
            timeout = Some(a.millis);
        }
        self.test_timeout.get_timeout(timeout)
    }
}

/// `join(t, millis, cond)`: waits for `t` at most `millis`, in slices of at
/// most 250 ms passed to `cond`.
///
/// # Panics
///
/// `IllegalArgumentException` (`Timeout must be positive: N`) for a
/// non-positive timeout.
fn join(
    t: &Thread,
    millis: i64,
    cond: &mut dyn FnMut(i64) -> Result<(), Throwable>,
) -> Result<(), Throwable> {
    if millis <= 0 {
        exception::illegal_argument(format!("Timeout must be positive: {millis}"));
    }
    let clock = DeadlineClock::millis(millis);
    while t.is_alive() {
        let until = clock.time_until_deadline_millis();
        if until > 0 {
            cond(until.min(250))?;
        } else {
            break;
        }
    }
    Ok(())
}

/// `formatThreadStacks(threads)`: each thread numbered, with its (empty)
/// stack.
fn format_thread_stacks(threads: &[Thread]) -> String {
    let mut message = String::new();
    for (i, t) in threads.iter().enumerate() {
        message.push_str(&format!("\n  {:>2}) {}", i + 1, Threads::thread_name(t)));
        let stack = t.get_stack_trace();
        if stack.is_empty() {
            message.push_str("\n        at (empty stack)");
        } else {
            for ste in stack {
                message.push_str(&format!("\n        at {ste}"));
            }
        }
    }
    message
}

/// `threadNames(threads)`.
fn thread_names(threads: &[Thread]) -> String {
    threads
        .iter()
        .enumerate()
        .map(|(i, t)| format!("\n  {:>2}) {}", i + 1, Threads::thread_name(t)))
        .collect()
}
