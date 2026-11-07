/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! The class declaration of `org.apache.lucene.tests.util.LuceneTestCase`
//! (task #348): its annotations, its class rule chain (`classRules`), its test
//! rule chain (`ruleChain`) and its hooks, `LuceneTestCase.java:262-279` and
//! `:560-725`, `:1877-1940`.
//!
//! A suite extends the declaration with
//! `Class::builder(name).extends(&LuceneTestCase::class())`.
//!
//! **Divergences.**
//! * The test rules of `ruleChain` (`parentChainCallRule`,
//!   `threadAndTestNameRule`, `testFailureMarker`) are instance fields of
//!   `LuceneTestCase`; a declared class's instance is the subclass's own
//!   value, which this declaration cannot read, so the rules of the test
//!   being run are kept in a slot ([`LuceneTestCase::current_test_rules`])
//!   that reading `ruleChain` fills, once per test instance, as Java's
//!   instance initializer does. Tests of a suite run one at a time.
//! * `super.setUp()` and `super.tearDown()` of an overriding subclass are
//!   [`LuceneTestCase::super_set_up`] and [`LuceneTestCase::super_tear_down`].
//! * The class initializer also reloads the codec, postings, doc-values and
//!   KNN-vectors registries with this crate's service providers, which Java
//!   finds on the class path (`lucene-test-framework.jar`).

use std::sync::atomic::Ordering;
use std::sync::{Arc, LazyLock, Mutex, RwLock};

use randomizedtesting::annotations::{
    Action, ClassRef, Consequence, Group, Listeners, Scope, SeedDecoratorClass, SeedDecorators,
    TestMethodProviders, ThreadLeakAction, ThreadLeakFilters, ThreadLeakGroup, ThreadLeakLingering,
    ThreadLeakScope, ThreadLeakZombies, TimeoutSuite,
};
use randomizedtesting::jdk::reflect::{modifier, Class, Field, Method, Value};
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{
    After, AfterClass, Before, BeforeClass, ClassRule, Rule, RuleChain, RunListener, TestRule,
};
use randomizedtesting::rules::{
    NoClassHooksShadowingRule, NoInstanceHooksOverridesRule, TestRuleAdapterRule,
};
use randomizedtesting::{JUnit4MethodProvider, RandomizedTest, TestMethodProvider, ThreadFilter};
use rucene::search::{IndexSearcher, LRUQueryCache, QueryCache, QueryCachingPolicy};

use crate::util::abstract_before_after_rule::BeforeAfterRule;
use crate::util::failure_marker::FailureMarker;
use crate::util::lucene_junit3_method_provider::LuceneJUnit3MethodProvider;
use crate::util::lucene_test_case::{
    LuceneTestCase, CLASS_NAME_RULE, MAYBE_CACHE_POLICY, SUITE_FAILURE_MARKER,
    TEMP_FILES_CLEANUP_RULE,
};
use crate::util::quick_patch_threads_filter::QuickPatchThreadsFilter;
use crate::util::run_listener_print_reproduce_info::RunListenerPrintReproduceInfo;
use crate::util::test_rule_assertions_required::TestRuleAssertionsRequired;
use crate::util::test_rule_delegate::TestRuleDelegate;
use crate::util::test_rule_ignore_after_max_failures::TestRuleIgnoreAfterMaxFailures;
use crate::util::test_rule_ignore_test_suites::TestRuleIgnoreTestSuites;
use crate::util::test_rule_limit_sysouts::{Limit, TestRuleLimitSysouts};
use crate::util::test_rule_mark_failure::{TestRuleMarkFailure, TestRuleMarkFailureRule};
use crate::util::test_rule_restore_system_properties::TestRuleRestoreSystemProperties;
use crate::util::test_rule_setup_and_restore_class_env::TestRuleSetupAndRestoreClassEnv;
use crate::util::test_rule_setup_and_restore_instance_env::TestRuleSetupAndRestoreInstanceEnv;
use crate::util::test_rule_setup_teardown_chained::TestRuleSetupTeardownChained;
use crate::util::test_rule_thread_and_test_name::TestRuleThreadAndTestName;
use crate::util::time_units::TimeUnits;
use crate::util::verify_test_class_naming_convention::VerifyTestClassNamingConvention;
use crate::util::TestUtil;

/// `classEnvRule`: "class environment setup rule."
pub static CLASS_ENV_RULE: LazyLock<Arc<TestRuleSetupAndRestoreClassEnv>> =
    LazyLock::new(|| Arc::new(TestRuleSetupAndRestoreClassEnv::new()));

/// `ignoreAfterMaxFailuresDelegate`: "ignore tests after hitting a designated
/// number of initial failures. This is truly a "static" global singleton
/// since it needs to span the lifetime of all test classes running inside
/// this JVM (it cannot be part of a class rule)."
///
/// Its static initializer reads `tests.maxfailures` and `tests.failfast`.
pub static IGNORE_AFTER_MAX_FAILURES_DELEGATE: LazyLock<
    Arc<RwLock<Arc<TestRuleIgnoreAfterMaxFailures>>>,
> = LazyLock::new(|| {
    let mut max_failures =
        RandomizedTest::system_property_as_int(LuceneTestCase::SYSPROP_MAXFAILURES, i32::MAX);
    let fail_fast =
        RandomizedTest::system_property_as_boolean(LuceneTestCase::SYSPROP_FAILFAST, false);
    if fail_fast {
        if max_failures == i32::MAX {
            max_failures = 1;
        } else {
            system::err_println(&format!(
                "Property '{}'={max_failures}, 'failfast' is ignored.",
                LuceneTestCase::SYSPROP_MAXFAILURES
            ));
        }
    }
    Arc::new(RwLock::new(Arc::new(TestRuleIgnoreAfterMaxFailures::new(
        max_failures,
    ))))
});

/// `ignoreAfterMaxFailures`: `TestRuleDelegate.of(ignoreAfterMaxFailuresDelegate)`.
static IGNORE_AFTER_MAX_FAILURES: LazyLock<Arc<dyn TestRule>> = LazyLock::new(|| {
    Arc::new(TestRuleDelegate::of(Arc::clone(
        &IGNORE_AFTER_MAX_FAILURES_DELEGATE,
    )))
});

/// `classRules`: "this controls how suite-level rules are nested. It is
/// important that _all_ rules declared in `LuceneTestCase` are executed in
/// proper order if they depend on each other."
pub static CLASS_RULES: LazyLock<RuleChain> = LazyLock::new(|| {
    let r = RuleChain::outer_rule(Arc::new(TestRuleIgnoreTestSuites))
        .around(Arc::clone(&IGNORE_AFTER_MAX_FAILURES))
        .around(Arc::new(TestRuleMarkFailureRule(Arc::clone(
            &SUITE_FAILURE_MARKER,
        ))))
        .around(Arc::new(BeforeAfterRule(Arc::new(
            VerifyTestClassNamingConvention::lucene(),
        ))))
        .around(Arc::new(TestRuleAssertionsRequired))
        .around(Arc::new(TestRuleAdapterRule(Arc::new(
            TestRuleLimitSysouts::new(Arc::clone(&SUITE_FAILURE_MARKER)),
        ))))
        .around(Arc::new(TestRuleAdapterRule(Arc::clone(
            &TEMP_FILES_CLEANUP_RULE,
        ))));
    r.around(Arc::new(NoClassHooksShadowingRule::new()))
        .around(Arc::new(NoInstanceHooksOverridesRule::new().with_verify(
            |key: &Method| {
                let name = key.name();
                !(name == "setUp" || name == "tearDown")
            },
        )))
        .around(Arc::clone(&CLASS_NAME_RULE) as Arc<dyn TestRule>)
        .around(Arc::new(TestRuleAdapterRule(Arc::new(
            // Enlist all properties to which we have write access (security manager);
            // these should be restored to previous state, no matter what the outcome of the
            // test.

            // We reset the default locale and timezone; these properties change as a
            // side-effect
            TestRuleRestoreSystemProperties::new(&["user.language", "user.timezone"]),
        ))))
        .around(Arc::new(BeforeAfterRule(Arc::clone(&CLASS_ENV_RULE))))
});

/// The test-level rules of one `LuceneTestCase` instance (its instance
/// fields).
pub struct TestRules {
    /// `parentChainCallRule`: "enforces `setUp()` and `tearDown()` calls are
    /// chained."
    pub parent_chain_call_rule: Arc<TestRuleSetupTeardownChained>,
    /// `threadAndTestNameRule`: "save test thread and name."
    pub thread_and_test_name_rule: Arc<TestRuleThreadAndTestName>,
    /// `testFailureMarker`: "taint suite result with individual test
    /// failures."
    pub test_failure_marker: Arc<TestRuleMarkFailure>,
    /// `ruleChain`: "this controls how individual test rules are nested."
    pub rule_chain: RuleChain,
}

impl TestRules {
    /// The instance initializer of `LuceneTestCase`'s rule fields.
    fn new() -> TestRules {
        let parent_chain_call_rule = Arc::new(TestRuleSetupTeardownChained::default());
        let thread_and_test_name_rule = Arc::new(TestRuleThreadAndTestName::default());
        let test_failure_marker = Arc::new(TestRuleMarkFailure::new(vec![Arc::clone(
            &SUITE_FAILURE_MARKER,
        )]));
        let rule_chain = RuleChain::outer_rule(Arc::new(TestRuleMarkFailureRule(Arc::clone(
            &test_failure_marker,
        ))))
        .around(Arc::clone(&IGNORE_AFTER_MAX_FAILURES))
        .around(Arc::clone(&thread_and_test_name_rule) as Arc<dyn TestRule>)
        .around(Arc::new(BeforeAfterRule(Arc::new(
            TestRuleSetupAndRestoreInstanceEnv::default(),
        ))))
        .around(Arc::clone(&parent_chain_call_rule) as Arc<dyn TestRule>);
        TestRules {
            parent_chain_call_rule,
            thread_and_test_name_rule,
            test_failure_marker,
            rule_chain,
        }
    }
}

/// The rules of the test instance being run.
static CURRENT_TEST_RULES: Mutex<Option<Arc<TestRules>>> = Mutex::new(None);

/// The query cache and caching policy defaults.
type DefaultQueryCache = (
    Option<Arc<dyn QueryCache>>,
    Option<Arc<dyn QueryCachingPolicy>>,
);

/// `DEFAULT_QUERY_CACHE` and `DEFAULT_CACHING_POLICY`: the defaults when the
/// class is initialized.
static DEFAULT_QUERY_CACHE: LazyLock<DefaultQueryCache> = LazyLock::new(|| {
    (
        IndexSearcher::get_default_query_cache(),
        IndexSearcher::get_default_query_caching_policy(),
    )
});

impl LuceneTestCase {
    /// `replaceMaxFailureRule(newValue)`: "temporarily substitute the global
    /// `TestRuleIgnoreAfterMaxFailures`"; returns the previous one.
    ///
    /// As a static method of `LuceneTestCase`, its first call initializes the
    /// class (the stream capture of `TestRuleLimitSysouts` included).
    pub fn replace_max_failure_rule(
        new_value: Arc<TestRuleIgnoreAfterMaxFailures>,
    ) -> Arc<TestRuleIgnoreAfterMaxFailures> {
        if let Err(t) = Self::class().initialize() {
            t.throw();
        }
        std::mem::replace(
            &mut *IGNORE_AFTER_MAX_FAILURES_DELEGATE
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            new_value,
        )
    }

    /// The rules of the test instance being run (see the module's
    /// divergences); `None` outside a test.
    pub fn current_test_rules() -> Option<Arc<TestRules>> {
        CURRENT_TEST_RULES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// `super.setUp()`: `parentChainCallRule.setupCalled = true`.
    pub fn super_set_up() {
        if let Some(rules) = Self::current_test_rules() {
            rules
                .parent_chain_call_rule
                .setup_called
                .store(true, Ordering::SeqCst);
        }
    }

    /// `super.tearDown()`: `parentChainCallRule.teardownCalled = true`,
    /// `fieldToType.clear()` and `restoreIndexWriterMaxDocs()` ("test is
    /// supposed to call this itself, but we do this defensively in case it
    /// forgot").
    pub fn super_tear_down() {
        if let Some(rules) = Self::current_test_rules() {
            rules
                .parent_chain_call_rule
                .teardown_called
                .store(true, Ordering::SeqCst);
        }
        Self::clear_field_to_type();
        Self::restore_index_writer_max_docs();
    }

    /// `overrideDefaultQueryCache()` (`@BeforeClass`, and `@Before` through
    /// `overrideTestDefaultQueryCache()`): "we need to reset the query cache
    /// in an @BeforeClass so that tests that instantiate an IndexSearcher in
    /// an @BeforeClass method use a fresh new cache."
    pub fn override_default_query_cache() {
        LazyLock::force(&DEFAULT_QUERY_CACHE);
        let cache =
            LRUQueryCache::with_predicate(10000, 1 << 25, Arc::new(|_| true), f32::INFINITY)
                .expect("INVARIANT: an infinite skip-cache factor is valid");
        IndexSearcher::set_default_query_cache(Some(Arc::new(cache)));
        IndexSearcher::set_default_query_caching_policy(Some(Arc::clone(&MAYBE_CACHE_POLICY)));
    }

    /// `resetDefaultQueryCache()` (`@AfterClass`).
    pub fn reset_default_query_cache() {
        let (cache, policy) = &*DEFAULT_QUERY_CACHE;
        IndexSearcher::set_default_query_cache(cache.clone());
        IndexSearcher::set_default_query_caching_policy(policy.clone());
    }

    /// `setupCPUCoreCount()` (`@BeforeClass`): "randomize core count so CMS
    /// varies its dynamic defaults, and this also "fixes" core count from the
    /// master seed so it will always be the same on reproduce."
    pub fn setup_cpu_core_count() {
        let num_cores = TestUtil::next_int(&*Self::random(), 1, 4);
        system::set_property("lucene.cms.override_core_count", &num_cores.to_string());
    }

    /// `restoreCPUCoreCount()` (`@AfterClass`).
    pub fn restore_cpu_core_count() {
        system::clear_property("lucene.cms.override_core_count");
    }

    /// The declaration of `org.apache.lucene.tests.util.LuceneTestCase` for
    /// suites that extend it.
    pub fn class() -> Arc<Class> {
        static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
            Class::builder("org.apache.lucene.tests.util.LuceneTestCase")
                .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
                .annotate(TestMethodProviders {
                    value: vec![
                        ClassRef::new(LuceneJUnit3MethodProvider::CLASS_NAME, || {
                            Arc::new(LuceneJUnit3MethodProvider) as Arc<dyn TestMethodProvider>
                        }),
                        ClassRef::new(
                            "com.carrotsearch.randomizedtesting.JUnit4MethodProvider",
                            || Arc::new(JUnit4MethodProvider) as Arc<dyn TestMethodProvider>,
                        ),
                    ],
                })
                .annotate(Listeners {
                    value: vec![
                        ClassRef::new(RunListenerPrintReproduceInfo::CLASS_NAME, || {
                            Arc::new(RunListenerPrintReproduceInfo::new()) as Arc<dyn RunListener>
                        }),
                        ClassRef::new(FailureMarker::CLASS_NAME, || {
                            Arc::new(FailureMarker) as Arc<dyn RunListener>
                        }),
                    ],
                })
                // See LUCENE-3995 for rationale.
                .annotate(SeedDecorators {
                    value: vec![SeedDecoratorClass::mix_with_suite_name()],
                })
                .annotate(ThreadLeakScope {
                    value: Scope::Suite,
                })
                .annotate(ThreadLeakGroup { value: Group::Main })
                .annotate(ThreadLeakAction {
                    value: vec![Action::Warn, Action::Interrupt],
                })
                // Wait long for leaked threads to complete before failure. zk needs this.
                .annotate(ThreadLeakLingering { linger: 20000 })
                .annotate(ThreadLeakZombies {
                    value: Consequence::IgnoreRemainingTests,
                })
                .annotate(TimeoutSuite {
                    millis: 2 * TimeUnits::HOUR,
                })
                .annotate(ThreadLeakFilters {
                    default_filters: true,
                    filters: vec![ClassRef::new(QuickPatchThreadsFilter::CLASS_NAME, || {
                        Arc::new(QuickPatchThreadsFilter) as Arc<dyn ThreadFilter>
                    })],
                })
                .annotate(Limit {
                    bytes: TestRuleLimitSysouts::DEFAULT_LIMIT,
                    hard_limit: TestRuleLimitSysouts::DEFAULT_HARD_LIMIT,
                })
                .static_init(|| {
                    // The JVM gives every thread `lucene-core` starts (merge
                    // threads among them) its creator's thread group, where
                    // `RandomizedContext` finds the suite's context.
                    let _ = rucene::util::concurrent::set_thread_start_hook(
                        randomizedtesting::jdk::thread::Thread::start_context,
                    );
                    // The throwables of this runner are panics carrying a
                    // `Throwable`: `lucene-core` records the one that escapes
                    // a merge with its Java class and message.
                    let _ = rucene::util::concurrent::set_panic_describer(|payload| {
                        payload
                            .downcast_ref::<randomizedtesting::jdk::exception::Throwable>()
                            .map(ToString::to_string)
                    });
                    // `lucene-test-framework.jar` on the class path.
                    let loader = crate::class_loader();
                    let _ = rucene::codecs::reload_codecs(loader);
                    let _ = rucene::codecs::postings::reload_postings_formats(loader);
                    let _ = rucene::codecs::doc_values::reload_doc_values_formats(loader);
                    let _ = rucene::codecs::knn_vectors::reload_knn_vectors_format(loader);
                    LazyLock::force(&IGNORE_AFTER_MAX_FAILURES_DELEGATE);
                    // Try to capture streams early so that other classes don't have a chance
                    // to steal references to them (as is the case with ju.logging handlers).
                    TestRuleLimitSysouts::check_capture_streams();
                    LazyLock::force(&DEFAULT_QUERY_CACHE);
                })
                .field(
                    Field::new_static("classRules", "org.junit.rules.TestRule", || {
                        Value::TestRule(Arc::new(CLASS_RULES.clone()) as Arc<dyn TestRule>)
                    })
                    .modifiers(modifier::PUBLIC | modifier::FINAL)
                    .annotate(ClassRule),
                )
                .field(
                    Field::new_instance_any("ruleChain", "org.junit.rules.TestRule", || {
                        let rules = Arc::new(TestRules::new());
                        let chain = rules.rule_chain.clone();
                        *CURRENT_TEST_RULES
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(rules);
                        Value::TestRule(Arc::new(chain) as Arc<dyn TestRule>)
                    })
                    .modifiers(modifier::PUBLIC | modifier::FINAL)
                    .annotate(Rule),
                )
                .method(
                    Method::new("setUp")
                        .throws(&["java.lang.Exception"])
                        .annotate(Before)
                        .body_any(LuceneTestCase::super_set_up),
                )
                .method(
                    Method::new("tearDown")
                        .throws(&["java.lang.Exception"])
                        .annotate(After)
                        .body_any(LuceneTestCase::super_tear_down),
                )
                .method(
                    Method::new("overrideTestDefaultQueryCache")
                        .annotate(Before)
                        .body_any(LuceneTestCase::override_default_query_cache),
                )
                .method(
                    Method::new("overrideDefaultQueryCache")
                        .annotate(BeforeClass)
                        .static_body(LuceneTestCase::override_default_query_cache),
                )
                .method(
                    Method::new("resetDefaultQueryCache")
                        .annotate(AfterClass)
                        .static_body(LuceneTestCase::reset_default_query_cache),
                )
                .method(
                    Method::new("setupCPUCoreCount")
                        .annotate(BeforeClass)
                        .static_body(LuceneTestCase::setup_cpu_core_count),
                )
                .method(
                    Method::new("restoreCPUCoreCount")
                        .annotate(AfterClass)
                        .static_body(LuceneTestCase::restore_cpu_core_count),
                )
                .method(
                    Method::new("setUpExecutorService")
                        .annotate(BeforeClass)
                        .static_body(LuceneTestCase::set_up_executor_service),
                )
                .method(
                    Method::new("shutdownExecutorService")
                        .annotate(AfterClass)
                        .static_body(LuceneTestCase::shutdown_executor_service),
                )
                .build()
        });
        Arc::clone(&CLASS)
    }
}
