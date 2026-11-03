//! Port of `com.carrotsearch.randomizedtesting.RandomizedContext`
//! (randomizedtesting 2.8.1, `RandomizedContext.java`).

use std::backtrace::Backtrace;
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::annotations::NIGHTLY;
use crate::closeable_resource_info::CloseableResourceInfo;
use crate::group_evaluator::GroupEvaluator;
use crate::jdk::exception::{self, illegal_argument, illegal_state};
use crate::jdk::io::Closeable;
use crate::jdk::random::Random;
use crate::jdk::thread::{State, Thread, ThreadGroup, WeakThread};
use crate::lifecycle_scope::LifecycleScope;
use crate::object_procedure::ObjectProcedure;
use crate::randomness::Randomness;
use crate::seed_utils::SeedUtils;
use crate::threads::Threads;

/// `RandomizedRunner.class.toString()`, which the context's messages print.
const RANDOMIZED_RUNNER_CLASS: &str = "class com.carrotsearch.randomizedtesting.RandomizedRunner";

/// Per-thread resources (`PerThreadResources`): the stack of randomness,
/// pushed per lifecycle phase (suite, then test). The front is the top.
#[derive(Default)]
struct PerThreadResources {
    randomnesses: VecDeque<Randomness>,
}

/// One entry of `WeakHashMap<Thread, PerThreadResources>`.
struct PerThreadEntry {
    thread: WeakThread,
    resources: PerThreadResources,
}

/// The state guarded by Java's `_contextLock`.
#[derive(Default)]
struct ContextState {
    per_thread_resources: Vec<PerThreadEntry>,
    disposable_resources: BTreeMap<LifecycleScope, Vec<CloseableResourceInfo>>,
}

impl ContextState {
    /// `WeakHashMap` drops the entries of collected threads; so does this,
    /// on every access.
    fn expunge(&mut self) {
        self.per_thread_resources
            .retain(|e| e.thread.get().is_some());
    }

    fn entry(&mut self, thread: &Thread) -> Option<&mut PerThreadResources> {
        self.expunge();
        self.per_thread_resources
            .iter_mut()
            .find(|e| e.thread.is(thread))
            .map(|e| &mut e.resources)
    }

    fn contains(&mut self, thread: &Thread) -> bool {
        self.entry(thread).is_some()
    }

    fn put(&mut self, thread: &Thread, resources: PerThreadResources) {
        self.expunge();
        self.per_thread_resources.retain(|e| !e.thread.is(thread));
        self.per_thread_resources.push(PerThreadEntry {
            thread: thread.downgrade(),
            resources,
        });
    }
}

/// Java's `_globalLock` and `contexts`
/// (`IdentityHashMap<ThreadGroup, RandomizedContext>`).
static CONTEXTS: Mutex<Vec<(Arc<ThreadGroup>, Arc<RandomizedContext>)>> = Mutex::new(Vec::new());

fn contexts() -> MutexGuard<'static, Vec<(Arc<ThreadGroup>, Arc<RandomizedContext>)>> {
    CONTEXTS.lock().unwrap_or_else(|e| e.into_inner())
}

fn find_context(
    contexts: &[(Arc<ThreadGroup>, Arc<RandomizedContext>)],
    group: &Arc<ThreadGroup>,
) -> Option<Arc<RandomizedContext>> {
    contexts
        .iter()
        .find(|(g, _)| Arc::ptr_eq(g, group))
        .map(|(_, c)| Arc::clone(c))
}

#[allow(dead_code)] // Read by `create`, which RandomizedRunner (a later task) calls.
static NEXT_IDENTITY: AtomicU64 = AtomicU64::new(1);

/// The context of a test suite's execution (hooks and tests) under the
/// runner: `RandomizedContext`.
///
/// A context is bound to a thread group; every thread of that group (or of a
/// descendant group) finds it through [`current`](Self::current) and gets its
/// own stack of [`Randomness`], starting with a clone of the runner's.
///
/// **Divergences.**
/// * Java's `create(ThreadGroup, Class<?>, RandomizedRunner)` keeps the
///   runner and reads its `runnerRandomness` and `groupEvaluator` fields;
///   the runner is a later task, so `create` takes those two
///   values, and `getRunner()` is not provided until the runner exists.
/// * The suite class is its binary name (`Class.getName()`) and the target
///   method its name (`Method.getName()`): Rust has no `Class` or `Method`
///   objects.
/// * Threads, groups and their weak references are the emulation of
///   [`crate::jdk::thread`]; the context's `toString()` identity hash is a
///   process-wide counter, not the JVM's identity hash.
pub struct RandomizedContext {
    identity: u64,
    /// Java's `_contextLock` and the state it guards.
    state: Mutex<ContextState>,
    #[allow(dead_code)] // Read by `dispose`, which RandomizedRunner (a later task) calls.
    thread_group: Arc<ThreadGroup>,
    suite_class: String,
    /// `runner.runnerRandomness`.
    runner_randomness: Randomness,
    /// `runner.groupEvaluator`.
    group_evaluator: GroupEvaluator,
    /// Java's `volatile boolean disposed`.
    disposed: AtomicBool,
    current_method: Mutex<Option<String>>,
}

impl RandomizedContext {
    fn lock(&self) -> MutexGuard<'_, ContextState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `getTargetClass()`: the suite's binary class name.
    ///
    /// # Panics
    ///
    /// `IllegalStateException` once the context is disposed.
    pub fn get_target_class(&self) -> &str {
        self.check_disposed();
        &self.suite_class
    }

    /// `getRunnerSeed()` (package-private): the runner's master seed.
    pub(crate) fn get_runner_seed(&self) -> i64 {
        self.runner_randomness.get_seed()
    }

    /// `getRunnerSeedAsString()`: the master seed, formatted.
    ///
    /// # Panics
    ///
    /// `IllegalStateException` once the context is disposed.
    pub fn get_runner_seed_as_string(&self) -> String {
        self.check_disposed();
        SeedUtils::format_seed(self.get_runner_seed())
    }

    /// `getRandomness()`: the top of the calling thread's stack (`None`
    /// where Java returns `null`).
    ///
    /// # Panics
    ///
    /// `IllegalStateException` once the context is disposed;
    /// `NullPointerException` when the calling thread was never registered
    /// with the context (Java dereferences a missing entry).
    pub fn get_randomness(&self) -> Option<Randomness> {
        self.with_per_thread(|r| r.randomnesses.front().cloned())
    }

    /// `getRandomnesses()` (package-private): the calling thread's whole
    /// stack, the most recent (current) randomness last.
    #[allow(dead_code)] // Package-private: RandomizedRunner (a later task) calls it.
    pub(crate) fn get_randomnesses(&self) -> Vec<Randomness> {
        self.with_per_thread(|r| r.randomnesses.iter().rev().cloned().collect())
    }

    /// `getRandom()`: `getRandomness().getRandom()`.
    ///
    /// # Panics
    ///
    /// As [`get_randomness`](Self::get_randomness), and
    /// `NullPointerException` when the stack is empty.
    pub fn get_random(&self) -> Arc<dyn Random> {
        match self.get_randomness() {
            Some(r) => r.get_random(),
            None => exception::throw_without_message(exception::NULL_POINTER_EXCEPTION),
        }
    }

    /// `isNightly()`: whether the `@Nightly` group is enabled.
    ///
    /// # Panics
    ///
    /// `IllegalStateException` once the context is disposed.
    pub fn is_nightly(&self) -> bool {
        self.check_disposed();
        self.get_group_evaluator().is_group_enabled(&NIGHTLY)
    }

    /// `current()`: the context of the calling thread.
    ///
    /// # Panics
    ///
    /// `IllegalStateException` when the thread runs outside any context.
    pub fn current() -> Arc<RandomizedContext> {
        RandomizedContext::context(&Thread::current())
    }

    /// `closeAtEnd(resource, scope)`: closes `resource` at the end of
    /// `scope`; a failure to close fails the test or suite. Returns
    /// `resource`.
    pub fn close_at_end<T: Closeable + 'static>(
        &self,
        resource: Arc<T>,
        scope: LifecycleScope,
    ) -> Arc<T> {
        let mut state = self.lock();
        let current = Thread::current();
        state
            .disposable_resources
            .entry(scope)
            .or_default()
            .push(CloseableResourceInfo::new(
                resource.clone(),
                scope,
                &current,
                Arc::new(Backtrace::capture()),
            ));
        resource
    }

    /// `getGroupEvaluator()`.
    pub fn get_group_evaluator(&self) -> &GroupEvaluator {
        &self.group_evaluator
    }

    /// `runWithPrivateRandomness(Randomness, Callable)`: pushes
    /// `randomness`, runs `callable` and pops and destroys `randomness`
    /// before returning, also when `callable` panics.
    pub fn run_with_private_randomness<T>(
        &self,
        randomness: Randomness,
        callable: impl FnOnce() -> T,
    ) -> T {
        self.push(randomness);
        struct PopOnExit<'a>(&'a RandomizedContext);
        impl Drop for PopOnExit<'_> {
            fn drop(&mut self) {
                self.0.pop_and_destroy();
            }
        }
        let _pop = PopOnExit(self);
        callable()
    }

    /// `runWithPrivateRandomness(long seed, Callable)`: as
    /// [`run_with_private_randomness`](Self::run_with_private_randomness)
    /// with a new randomness of `seed`, the current randomness's supplier
    /// and decorators.
    ///
    /// # Panics
    ///
    /// `NullPointerException` when the calling thread has no randomness.
    pub fn run_with_private_randomness_seed<T>(
        &self,
        seed: i64,
        callable: impl FnOnce() -> T,
    ) -> T {
        let Some(randomness) = self.get_randomness() else {
            exception::throw_without_message(exception::NULL_POINTER_EXCEPTION)
        };
        let prv = Randomness::new(
            seed,
            randomness.get_random_supplier(),
            randomness.get_decorators(),
        );
        self.run_with_private_randomness(prv, callable)
    }

    /// `closeResources(consumer, scope)` (package-private): hands every
    /// resource registered for `scope`, in registration order, to
    /// `consumer`, and forgets them.
    #[allow(dead_code)] // Package-private: RandomizedRunner (a later task) calls it.
    pub(crate) fn close_resources(
        &self,
        mut consumer: impl ObjectProcedure<CloseableResourceInfo>,
        scope: LifecycleScope,
    ) {
        let resources = self.lock().disposable_resources.remove(&scope);
        for info in resources.into_iter().flatten() {
            consumer.apply(info);
        }
    }

    /// `context(Thread)` (package-private): the context of `thread`'s group
    /// or its nearest ancestor with one; a thread seen for the first time
    /// gets a clone of the runner's randomness.
    pub(crate) fn context(thread: &Thread) -> Arc<RandomizedContext> {
        let Some(mut current_group) = thread.thread_group() else {
            illegal_state(format!(
                "No context for a terminated thread: {}",
                Threads::thread_name(thread)
            ))
        };
        let contexts = contexts();
        let context = loop {
            match find_context(&contexts, &current_group) {
                None if current_group.parent().is_some() => {
                    current_group = Arc::clone(current_group.parent().expect("parent"));
                }
                found => break found,
            }
        };
        let Some(context) = context else {
            drop(contexts);
            illegal_state(format!(
                "No context information for thread: {}. Is this thread running under a {} \
                 runner context? Add @RunWith({}.class) to your test class. Make sure your code \
                 accesses random contexts within @BeforeClass and @AfterClass boundary (for \
                 example, static test class initializers are not permitted to access random \
                 contexts).",
                Threads::thread_name(thread),
                RANDOMIZED_RUNNER_CLASS,
                RANDOMIZED_RUNNER_CLASS
            ))
        };
        {
            let mut state = context.lock();
            if !state.contains(thread) {
                let mut resources = PerThreadResources::default();
                resources
                    .randomnesses
                    .push_front(context.runner_randomness.clone_for(thread));
                state.put(thread, resources);
            }
        }
        context
    }

    /// `create(ThreadGroup, Class<?>, RandomizedRunner)` (package-private):
    /// a new context bound to `tg`, registered with an empty stack for the
    /// calling thread, which must belong to `tg`.
    #[allow(dead_code)] // Package-private: RandomizedRunner (a later task) calls it.
    pub(crate) fn create(
        tg: &Arc<ThreadGroup>,
        suite_class: &str,
        runner_randomness: Randomness,
        group_evaluator: GroupEvaluator,
    ) -> Arc<RandomizedContext> {
        let me = Thread::current();
        exception::java_assert_no_message(me.thread_group().is_some_and(|g| Arc::ptr_eq(&g, tg)));
        let mut contexts = contexts();
        let ctx = Arc::new(RandomizedContext {
            identity: NEXT_IDENTITY.fetch_add(1, Ordering::Relaxed),
            state: Mutex::new(ContextState::default()),
            thread_group: Arc::clone(tg),
            suite_class: suite_class.to_string(),
            runner_randomness,
            group_evaluator,
            disposed: AtomicBool::new(false),
            current_method: Mutex::new(None),
        });
        contexts.retain(|(g, _)| !Arc::ptr_eq(g, tg));
        contexts.push((Arc::clone(tg), Arc::clone(&ctx)));
        ctx.lock().put(&me, PerThreadResources::default());
        ctx
    }

    /// `dispose()` (package-private): unregisters the context and
    /// invalidates every randomness it handed out.
    ///
    /// # Panics
    ///
    /// `IllegalStateException` when already disposed.
    #[allow(dead_code)] // Package-private: RandomizedRunner (a later task) calls it.
    pub(crate) fn dispose(&self) {
        let mut contexts = contexts();
        self.check_disposed();
        self.disposed.store(true, Ordering::Release);
        contexts.retain(|(g, _)| !Arc::ptr_eq(g, &self.thread_group));
        let mut state = self.lock();
        state.expunge();
        for entry in &state.per_thread_resources {
            for randomness in &entry.resources.randomnesses {
                randomness.destroy();
            }
        }
    }

    /// `push(Randomness)` (package-private): a new top of the calling
    /// thread's stack.
    pub(crate) fn push(&self, rnd: Randomness) {
        self.with_per_thread(|r| r.randomnesses.push_front(rnd));
    }

    /// `popAndDestroy()` (package-private): pops the calling thread's top
    /// randomness and destroys it.
    ///
    /// # Panics
    ///
    /// `NoSuchElementException` when the stack is empty, as
    /// `ArrayDeque.pop()` throws.
    pub(crate) fn pop_and_destroy(&self) {
        match self.with_per_thread(|r| r.randomnesses.pop_front()) {
            Some(r) => r.destroy(),
            None => exception::throw_without_message("java.util.NoSuchElementException"),
        }
    }

    /// `getPerThread()`, with the entry's use inside the context lock.
    fn with_per_thread<T>(&self, f: impl FnOnce(&mut PerThreadResources) -> T) -> T {
        self.check_disposed();
        let current = Thread::current();
        let mut state = self.lock();
        match state.entry(&current) {
            Some(r) => f(r),
            None => {
                drop(state);
                exception::throw_without_message(exception::NULL_POINTER_EXCEPTION)
            }
        }
    }

    /// `checkDisposed()`.
    fn check_disposed(&self) {
        if self.disposed.load(Ordering::Acquire) {
            illegal_state(format!(
                "Context disposed: {} for thread: {}",
                self,
                Thread::current()
            ));
        }
    }

    /// `cloneFor(Thread)` (package-private): copies the calling thread's
    /// randomness stack to `t`, a not yet started thread of the same group.
    ///
    /// # Panics
    ///
    /// `IllegalStateException` or `IllegalArgumentException` with Java's
    /// messages when `t` is started, has no group, is in another group, has
    /// no context, already has one, or the calling thread has none.
    #[allow(dead_code)] // Package-private: RandomizedRunner (a later task) calls it.
    pub(crate) fn clone_for(t: &Thread) {
        if t.state() != State::New {
            illegal_state(format!(
                "The thread to share context with is not in NEW state: {t}"
            ));
        }
        let Some(t_group) = t.thread_group() else {
            illegal_state(format!("No thread group for thread: {t}"))
        };
        let me = Thread::current();
        if !me.thread_group().is_some_and(|g| Arc::ptr_eq(&g, &t_group)) {
            illegal_argument("Both threads must share the thread group.");
        }
        let contexts = contexts();
        let Some(context) = find_context(&contexts, &t_group) else {
            drop(contexts);
            illegal_state(format!("No context information for thread: {t}"))
        };
        let mut state = context.lock();
        if state.contains(t) {
            drop(state);
            drop(contexts);
            illegal_state(format!("Context already initialized for thread: {t}"));
        }
        let cloned: Option<VecDeque<Randomness>> = state
            .entry(&me)
            .map(|r| r.randomnesses.iter().map(|x| x.clone_for(t)).collect());
        let Some(randomnesses) = cloned else {
            drop(state);
            drop(contexts);
            illegal_state(format!("Context not initialized for thread: {me}"))
        };
        state.put(t, PerThreadResources { randomnesses });
    }

    /// `setTargetMethod(Method)` (package-private).
    #[allow(dead_code)] // Package-private: RandomizedRunner (a later task) calls it.
    pub(crate) fn set_target_method(&self, method: Option<&str>) {
        *self
            .current_method
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = method.map(str::to_string);
    }

    /// `getTargetMethod()`: the name of the test method being executed, or
    /// `None` in the static (suite) context.
    ///
    /// # Panics
    ///
    /// `IllegalStateException` once the context is disposed.
    pub fn get_target_method(&self) -> Option<String> {
        self.check_disposed();
        self.current_method
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl fmt::Display for RandomizedContext {
    /// `Object.toString()`: `com.carrotsearch.randomizedtesting.RandomizedContext@<hash>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "com.carrotsearch.randomizedtesting.RandomizedContext@{:x}",
            self.identity
        )
    }
}
