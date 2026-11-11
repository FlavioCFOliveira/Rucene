//! `RandomizedContext`: resource disposal per `LifecycleScope`, context and
//! disposal errors, `runWithPrivateRandomness` and `cloneFor`
//! (`context.tsv`), driven the way `RandomizedRunner` drives the context.

use std::io;
use std::sync::{Arc, Mutex};

use super::{catch, java_error, mask_after, rows, Rec};
use crate::closeable_resource_info::CloseableResourceInfo;
use crate::group_evaluator::GroupEvaluator;
use crate::jdk::io::Closeable;
use crate::jdk::lang::string_hash_code;
use crate::jdk::random::Random;
use crate::jdk::thread::{Thread, ThreadGroup};
use crate::murmur_hash3::MurmurHash3;
use crate::random_supplier::default_supplier;
use crate::{LifecycleScope, RandomizedContext, Randomness, SeedUtils};

type Log = Arc<Mutex<Vec<String>>>;

struct Named {
    name: &'static str,
    fail: bool,
    log: Log,
}

impl Closeable for Named {
    fn close(&self) -> io::Result<()> {
        self.log
            .lock()
            .unwrap()
            .push(format!("close:{}", self.name));
        if self.fail {
            return Err(io::Error::other(format!("boom {}", self.name)));
        }
        Ok(())
    }
}

fn named(name: &'static str, fail: bool, log: &Log) -> Arc<Named> {
    Arc::new(Named {
        name,
        fail,
        log: log.clone(),
    })
}

/// The oracle's rows of `kind`, the value fields joined by tabs.
fn values(kind: &str) -> Vec<String> {
    rows("context.tsv")
        .into_iter()
        .filter(|r| r[0] == kind)
        .map(|r| r[1..].join("\t"))
        .collect()
}

/// The runner's disposer (`RandomizedRunner.withCloseContextResources`,
/// `RandomizedRunner.java:785-806`): closes each resource and records the
/// `ResourceDisposalError` it would add for a failure.
fn close_scope(ctx: &RandomizedContext, scope: LifecycleScope, errors: &Mutex<Vec<String>>) {
    ctx.close_resources(
        |info: CloseableResourceInfo| {
            if let Err(e) = info.get_resource().close() {
                errors.lock().unwrap().push(format!(
                    "com.carrotsearch.randomizedtesting.ResourceDisposalError\tResource in scope {} \
                     failed to close. Resource was registered from thread {}, registration stack \
                     trace below.\tjava.io.IOException\t{e}",
                    info.get_scope().name(),
                    info.get_thread_name()
                ));
            }
        },
        scope,
    );
}

/// Runs `body` as a suite: a thread in its own group, with a context whose
/// master seed is `DEADBEEF`, the class randomness pushed.
fn in_suite(suite: &str, body: impl FnOnce(Arc<RandomizedContext>) + Send + 'static) {
    let suite_class = format!("com.carrotsearch.randomizedtesting.RandomizedTestingOracle${suite}");
    let group = ThreadGroup::new(
        ThreadGroup::main(),
        format!("TGRP-RandomizedTestingOracle${suite}"),
    );
    let t = Thread::with_group(&group, "SUITE", move || {
        let runner = Randomness::new(0xDEAD_BEEF, default_supplier(), vec![]);
        let me = Thread::current();
        let ctx = RandomizedContext::create(
            &me.thread_group().unwrap(),
            &suite_class,
            runner.clone(),
            GroupEvaluator::new(),
        );
        ctx.push(runner.clone_for(&me));
        body(ctx.clone());
        ctx.pop_and_destroy();
        ctx.dispose();
    });
    t.start();
    if let Err(p) = t.join() {
        std::panic::resume_unwind(p);
    }
}

fn method_randomness(ctx: &RandomizedContext, name: &str) -> Randomness {
    let seed = ctx.get_runner_seed() ^ MurmurHash3::hash_long(i64::from(string_hash_code(name)));
    Randomness::new(seed, default_supplier(), vec![])
}

#[test]
fn outside_any_context_fails_as_randomized_runner() {
    let got: Arc<Mutex<String>> = Arc::default();
    let g = got.clone();
    let t = Thread::new("main", move || {
        *g.lock().unwrap() = match catch(RandomizedContext::current) {
            Ok(_) => "OK".into(),
            Err(p) => java_error(&p),
        };
    });
    t.start();
    t.join().unwrap();
    let got = got.lock().unwrap().clone();
    assert_eq!(
        mask_after(&got, "id="),
        mask_after(&values("outside")[0], "id=")
    );
}

#[test]
fn resources_close_at_end_of_their_scope_as_randomized_runner() {
    let log: Log = Arc::default();
    let saved: Arc<Mutex<Option<Arc<RandomizedContext>>>> = Arc::default();
    let (l, s) = (log.clone(), saved.clone());
    let errors: Arc<Mutex<Vec<String>>> = Arc::default();
    let errs = errors.clone();
    in_suite("ResourceSuite", move |ctx| {
        let push = |e: &str| l.lock().unwrap().push(e.to_string());
        push("beforeClass");
        ctx.close_at_end(named("suite-1", false, &l), LifecycleScope::Suite);
        ctx.close_at_end(
            named("test-from-beforeClass", false, &l),
            LifecycleScope::Test,
        );
        *s.lock().unwrap() = Some(ctx.clone());

        ctx.push(method_randomness(&ctx, "testOne"));
        ctx.set_target_method(Some("testOne"));
        push("before");
        push("testOne");
        let a = ctx.close_at_end(named("test-a", false, &l), LifecycleScope::Test);
        ctx.close_at_end(named("suite-b", false, &l), LifecycleScope::Suite);
        ctx.close_at_end(named("test-c", false, &l), LifecycleScope::Test);
        push(&format!("returned-same:{}", a.name == "test-a"));
        let saved_test_random = ctx.get_random();
        push("after");
        close_scope(&ctx, LifecycleScope::Test, &errs);
        ctx.set_target_method(None);
        ctx.pop_and_destroy();

        push("afterClass");
        match catch(|| saved_test_random.next_int()) {
            Ok(_) => push("saved random usable"),
            Err(p) => push(&format!("saved random: {}", java_error(&p))),
        }
        close_scope(&ctx, LifecycleScope::Suite, &errs);
    });
    let events: Vec<String> = log.lock().unwrap().clone();
    assert_eq!(events, values("resource-event"));
    assert!(errors.lock().unwrap().is_empty());
    assert!(values("resource-failure").is_empty());

    // The context is disposed with its suite.
    let ctx = saved.lock().unwrap().clone().unwrap();
    let disposed = |f: &dyn Fn()| {
        let got = java_error(&catch(f).unwrap_err());
        mask_after(&mask_after(&got, "RandomizedContext@"), "#")
    };
    let expect = |k: &str| mask_after(&mask_after(&values(k)[0], "RandomizedContext@"), "#");
    let main_like = Thread::new("main", move || {
        assert_eq!(
            disposed(&|| {
                ctx.get_target_class();
            }),
            expect("disposed")
        );
        assert_eq!(
            disposed(&|| {
                ctx.get_randomness();
            }),
            expect("disposed-getRandomness")
        );
    });
    main_like.start();
    main_like.join().unwrap();
}

#[test]
fn failing_resource_reports_and_the_rest_still_close() {
    let log: Log = Arc::default();
    let errors: Arc<Mutex<Vec<String>>> = Arc::default();
    let (l, errs) = (log.clone(), errors.clone());
    in_suite("FailingResourceSuite", move |ctx| {
        ctx.push(method_randomness(&ctx, "testFailing"));
        let me = Thread::current();
        let name = format!(
            "TEST-RandomizedTestingOracle$FailingResourceSuite.testFailing-seed#{}",
            SeedUtils::format_seed_chain(&[&ctx.get_randomnesses()[0]])
        );
        me.set_name(name);
        ctx.close_at_end(named("fails", true, &l), LifecycleScope::Test);
        ctx.close_at_end(named("after-fails", false, &l), LifecycleScope::Test);
        close_scope(&ctx, LifecycleScope::Test, &errs);
        ctx.pop_and_destroy();
    });
    assert_eq!(*log.lock().unwrap(), values("failing-event"));
    let java: Vec<String> = values("failing-failure")
        .iter()
        .zip(values("failing-cause"))
        .map(|(f, c)| format!("{f}\t{c}"))
        .collect();
    let got: Vec<String> = errors.lock().unwrap().clone();
    let mask = |v: &Vec<String>| v.iter().map(|s| mask_after(s, "id=")).collect::<Vec<_>>();
    assert_eq!(mask(&got), mask(&java));
}

#[test]
fn private_randomness_equals_randomized_runner() {
    let out: Arc<Mutex<String>> = Arc::default();
    let o = out.clone();
    in_suite("PrivateRandomnessSuite", move |ctx| {
        ctx.push(method_randomness(&ctx, "testPrivate"));
        let before = ctx.get_randomness().unwrap().to_string();
        let c2 = ctx.clone();
        let inside = ctx.run_with_private_randomness_seed(0xCAFE, move || {
            let mut rec = Rec::default();
            for _ in 0..100 {
                rec.l(RandomizedContext::current().get_random().next_long());
            }
            let chain = c2.get_randomnesses();
            let refs: Vec<&Randomness> = chain.iter().collect();
            format!(
                "{}\t{}\t{}",
                RandomizedContext::current().get_randomness().unwrap(),
                SeedUtils::format_seed_chain(&refs),
                rec.line()
            )
        });
        *o.lock().unwrap() = format!("{before}\t{inside}\t{}", ctx.get_randomness().unwrap());
        // The private randomness is gone even when the callable panics.
        let depth = ctx.get_randomnesses().len();
        assert!(catch(|| ctx.run_with_private_randomness_seed(1, || panic!("x"))).is_err());
        assert_eq!(ctx.get_randomnesses().len(), depth);
        ctx.pop_and_destroy();
    });
    assert_eq!(*out.lock().unwrap(), values("private")[0]);
}

#[test]
fn threads_of_the_group_get_the_runner_randomness_and_clone_for_copies_the_stack() {
    in_suite("CloneSuite", |ctx| {
        ctx.push(method_randomness(&ctx, "testClone"));
        let parent: Vec<String> = ctx
            .get_randomnesses()
            .iter()
            .map(|r| r.to_string())
            .collect();

        // A thread of the group, unknown to the context: the runner's seed.
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let s = seen.clone();
        let t = Thread::new("child", move || {
            let c = RandomizedContext::current();
            *s.lock().unwrap() = c.get_randomnesses().iter().map(|r| r.to_string()).collect();
        });
        t.start();
        t.join().unwrap();
        assert_eq!(
            *seen.lock().unwrap(),
            vec!["[Randomness, seed=[DEADBEEF]]".to_string()]
        );

        // cloneFor: a NEW thread of the same group gets a copy of the stack.
        let s = seen.clone();
        let t = Thread::new("cloned", move || {
            let c = RandomizedContext::current();
            *s.lock().unwrap() = c.get_randomnesses().iter().map(|r| r.to_string()).collect();
        });
        RandomizedContext::clone_for(&t);
        let again = catch(|| RandomizedContext::clone_for(&t)).unwrap_err();
        assert!(again.starts_with(&format!(
            "java.lang.IllegalStateException: Context already initialized for thread: {t}"
        )));
        t.start();
        t.join().unwrap();
        assert_eq!(*seen.lock().unwrap(), parent);
        let started = catch(|| RandomizedContext::clone_for(&t)).unwrap_err();
        assert!(started.starts_with(
            "java.lang.IllegalStateException: The thread to share context with is not in NEW state: "
        ));
        let other = Thread::with_group(ThreadGroup::main(), "other", || {});
        let wrong = catch(|| RandomizedContext::clone_for(&other)).unwrap_err();
        assert_eq!(
            wrong,
            "java.lang.IllegalArgumentException: Both threads must share the thread group."
        );
        ctx.pop_and_destroy();
    });
}
