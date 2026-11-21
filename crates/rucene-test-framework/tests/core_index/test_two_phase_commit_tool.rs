//! Port of `lucene/core/src/test/org/apache/lucene/index/TestTwoPhaseCommitTool.java`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::reflect::Method;
use randomizedtesting::junit::Assert;
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::{execute_two_phase_commit, TwoPhaseCommit};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assertion_error, class, random, test};

/// `TestTwoPhaseCommitTool`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestTwoPhaseCommitTool",
        vec![
            Method::new("setUp")
                .throws(&["java.lang.Exception"])
                .body(|_: &mut ()| set_up()),
            test("testPrepareThenCommit", test_prepare_then_commit),
            test("testRollback", test_rollback),
            test("testNullTPCs", test_null_tp_cs),
        ],
    )
}

/// `TwoPhaseCommitImpl.commitCalled`, a static of the nested class.
static COMMIT_CALLED: AtomicBool = AtomicBool::new(false);

/// `private static class TwoPhaseCommitImpl implements TwoPhaseCommit`.
#[derive(Debug)]
struct TwoPhaseCommitImpl {
    fail_on_prepare: bool,
    fail_on_commit: bool,
    fail_on_rollback: bool,
    rollback_called: AtomicBool,
}

impl TwoPhaseCommitImpl {
    fn new(fail_on_prepare: bool, fail_on_commit: bool, fail_on_rollback: bool) -> Self {
        Self {
            fail_on_prepare,
            fail_on_commit,
            fail_on_rollback,
            rollback_called: AtomicBool::new(false),
        }
    }

    /// `prepareCommit(Map<String, String> commitData)`.
    fn prepare_commit_with(&self, _commit_data: Option<()>) -> Result<i64> {
        // assertFalse(...): the AssertionError is a Throwable that
        // `TwoPhaseCommitTool.execute` catches, as any other failure.
        if COMMIT_CALLED.load(Ordering::SeqCst) {
            return Err(assertion_error(
                "commit should not have been called before all prepareCommit were",
            ));
        }
        if self.fail_on_prepare {
            return Err(LuceneError::from(LuceneErrorKind::Io(
                std::io::Error::other("failOnPrepare"),
            )));
        }
        Ok(1)
    }

    /// `commit(Map<String, String> commitData)`.
    fn commit_with(&self, _commit_data: Option<()>) -> Result<i64> {
        COMMIT_CALLED.store(true, Ordering::SeqCst);
        if self.fail_on_commit {
            return Err(LuceneError::unchecked(
                "java.lang.RuntimeException",
                "failOnCommit",
            ));
        }
        Ok(1)
    }
}

impl TwoPhaseCommit for TwoPhaseCommitImpl {
    fn prepare_commit(&self) -> Result<i64> {
        self.prepare_commit_with(None)
    }

    fn commit(&self) -> Result<i64> {
        self.commit_with(None)
    }

    fn rollback(&self) -> Result<()> {
        self.rollback_called.store(true, Ordering::SeqCst);
        if self.fail_on_rollback {
            return Err(LuceneError::unchecked("java.lang.Error", "failOnRollback"));
        }
        Ok(())
    }
}

/// `setUp()`.
fn set_up() {
    LuceneTestCase::super_set_up();
    COMMIT_CALLED.store(false, Ordering::SeqCst); // reset count before every test
}

/// `TwoPhaseCommitTool.execute(objects)`.
fn execute(objects: &[Option<&dyn TwoPhaseCommit>]) -> Result<()> {
    execute_two_phase_commit(objects).map_err(LuceneError::from)
}

/// Port of `TestTwoPhaseCommitTool.testPrepareThenCommit`.
fn test_prepare_then_commit() -> Result<()> {
    // tests that prepareCommit() is called on all objects before commit()
    let objects: Vec<TwoPhaseCommitImpl> = (0..2)
        .map(|_| TwoPhaseCommitImpl::new(false, false, false))
        .collect();

    // following call will fail if commit() is called before all prepare() were
    let tpcs: Vec<Option<&dyn TwoPhaseCommit>> = objects
        .iter()
        .map(|o| Some(o as &dyn TwoPhaseCommit))
        .collect();
    execute(&tpcs)
}

/// Port of `TestTwoPhaseCommitTool.testRollback`.
fn test_rollback() -> Result<()> {
    // tests that rollback is called if failure occurs at any stage
    let num_objects = random().next_int_bound(8) + 3; // between [3, 10]
    let mut objects = Vec::new();
    for _ in 0..num_objects {
        let fail_on_prepare = random().next_boolean();
        // we should not hit failures on commit usually
        let fail_on_commit = random().next_double() < 0.05;
        let rail_on_rollback = random().next_boolean();
        objects.push(TwoPhaseCommitImpl::new(
            fail_on_prepare,
            fail_on_commit,
            rail_on_rollback,
        ));
    }

    let tpcs: Vec<Option<&dyn TwoPhaseCommit>> = objects
        .iter()
        .map(|o| Some(o as &dyn TwoPhaseCommit))
        .collect();
    let any_failure = execute(&tpcs).is_err();

    if any_failure {
        // if any failure happened, ensure that rollback was called on all.
        for tpc in &objects {
            Assert::assert_true_msg(
                "rollback was not called while a failure occurred during the 2-phase commit",
                tpc.rollback_called.load(Ordering::SeqCst),
            );
        }
    }
    Ok(())
}

/// Port of `TestTwoPhaseCommitTool.testNullTPCs`.
fn test_null_tp_cs() -> Result<()> {
    let num_objects = random().next_int_bound(4) + 3; // between [3, 6]
    let mut tpcs: Vec<Option<TwoPhaseCommitImpl>> = Vec::new();
    let mut set_null = false;
    for _ in 0..num_objects {
        let is_null = random().next_double() < 0.3;
        if is_null {
            set_null = true;
            tpcs.push(None);
        } else {
            tpcs.push(Some(TwoPhaseCommitImpl::new(false, false, false)));
        }
    }

    if !set_null {
        // none of the TPCs were picked to be null, pick one at random
        let idx = random().next_int_bound(num_objects);
        tpcs[idx as usize] = None;
    }

    // following call would fail if TPCTool won't handle null TPCs properly
    let refs: Vec<Option<&dyn TwoPhaseCommit>> = tpcs
        .iter()
        .map(|o| o.as_ref().map(|o| o as &dyn TwoPhaseCommit))
        .collect();
    execute(&refs)
}
