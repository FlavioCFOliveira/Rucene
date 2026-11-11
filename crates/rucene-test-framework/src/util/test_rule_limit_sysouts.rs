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

//! Port of `org.apache.lucene.tests.util.TestRuleLimitSysouts`.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::AnnotationType;
use randomizedtesting::jdk::exception::{self, Throwable};
use randomizedtesting::jdk::reflect::{Class, JavaAnnotation};
use randomizedtesting::jdk::system::{self, PrintSink};
use randomizedtesting::rules::TestRuleAdapter;
use randomizedtesting::RandomizedTest;

use crate::util::lucene_test_case::{
    INFOSTREAM, MONSTER, SUPPRESS_SYSOUT_CHECKS, SUPPRESS_SYSOUT_CHECKS_NAME, VERBOSE,
};
use crate::util::test_rule_mark_failure::TestRuleMarkFailure;

const KB: i64 = 1024;
const MB: i64 = KB * 1024;
const GB: i64 = MB * 1024;

/// `TestRuleLimitSysouts.Limit` (`@Inherited`): "an annotation specifying
/// the limit of bytes per class."
pub static LIMIT: AnnotationType = AnnotationType::plain(
    "org.apache.lucene.tests.util.TestRuleLimitSysouts$Limit",
    "Limit",
    true,
);

/// An `@Limit(bytes, hardLimit)` annotation value.
#[derive(Debug, Clone, Copy)]
pub struct Limit {
    /// `bytes()`: "the maximum number of bytes written to stdout or stderr.
    /// If exceeded, a suite failure will be triggered."
    pub bytes: i64,
    /// `hardLimit()`: "maximum number of bytes passed to actual stdout or
    /// stderr. Any writes beyond this limit will be ignored", default
    /// [`TestRuleLimitSysouts::DEFAULT_HARD_LIMIT`].
    pub hard_limit: i64,
}

impl Limit {
    /// `@Limit(bytes = bytes)`.
    pub fn bytes(bytes: i64) -> Limit {
        Limit {
            bytes,
            hard_limit: TestRuleLimitSysouts::DEFAULT_HARD_LIMIT,
        }
    }
}

impl JavaAnnotation for Limit {
    fn annotation_type(&self) -> &'static AnnotationType {
        &LIMIT
    }
    fn to_java_string(&self) -> String {
        format!(
            "@org.apache.lucene.tests.util.TestRuleLimitSysouts.Limit(bytes={}L, hardLimit={}L)",
            self.bytes, self.hard_limit
        )
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The streams the static initializer captures: "we capture system output
/// and error streams as early as possible because certain components (like
/// the Java logging system) steal these references and never refresh them."
struct Captured {
    out: PrintSink,
    err: PrintSink,
}

/// `bytesWritten`.
static BYTES_WRITTEN: AtomicI64 = AtomicI64::new(0);
/// `hardLimit`.
static HARD_LIMIT: AtomicI64 = AtomicI64::new(i32::MAX as i64);

/// The static initializer: wraps `System.out` and `System.err` in delegate
/// streams counting into `bytesWritten` and installs them.
static CAPTURED: LazyLock<Captured> = LazyLock::new(|| {
    let sout = system::out();
    let serr = system::err();
    let write_err = {
        let serr = serr.clone();
        move |text: &str| match &serr {
            Some(s) => s(text),
            None => {
                use std::io::Write;
                let mut e = std::io::stderr().lock();
                let _ = e.write_all(text.as_bytes());
                let _ = e.flush();
            }
        }
    };
    // `limitCheck`: "Crossing the boundary. Write directly to stderr."; the
    // `IOException` it throws past the limit is "silently ignored by
    // PrintStreams", so the write is dropped.
    let limit_check: Arc<dyn Fn(i64, i64) -> bool + Send + Sync> =
        Arc::new(move |before, after| {
            let limit = HARD_LIMIT.load(Ordering::SeqCst);
            if after > limit {
                if before < limit {
                    write_err(
                        "\nNOTE: Hard limit on sysout exceeded, further output truncated.\n\n",
                    );
                }
                return false;
            }
            true
        });
    let delegate = |target: Option<PrintSink>, process_err: bool| -> PrintSink {
        let check = Arc::clone(&limit_check);
        Arc::new(move |text: &str| {
            // `DelegateStream.write`: `checkLimit(len)` (UTF-8 bytes), then
            // the delegate.
            let len = text.len() as i64;
            if len > 0 {
                let after = BYTES_WRITTEN.fetch_add(len, Ordering::SeqCst) + len;
                if !check(after - len, after) {
                    return;
                }
            }
            match &target {
                Some(s) => s(text),
                None => {
                    use std::io::Write;
                    if process_err {
                        let mut e = std::io::stderr().lock();
                        let _ = e.write_all(text.as_bytes());
                        let _ = e.flush();
                    } else {
                        let mut o = std::io::stdout().lock();
                        let _ = o.write_all(text.as_bytes());
                        let _ = o.flush();
                    }
                }
            }
        })
    };
    let out = delegate(sout, false);
    let err = delegate(serr, true);
    system::set_out(Some(Arc::clone(&out)));
    system::set_err(Some(Arc::clone(&err)));
    Captured { out, err }
});

/// Port of `TestRuleLimitSysouts`: "this test rule serves two purposes: it
/// fails the test if it prints too much to stdout and stderr (tests that
/// chatter too much are discouraged); the rule ensures an absolute hard
/// limit of stuff written to stdout and stderr to prevent accidental
/// infinite loops from filling all available disk space with persisted
/// output. The rule is not enforced for certain test types (see
/// `isEnforced()`)."
///
/// **Divergence.** `System.out` and `System.err` are the port's replaceable
/// sinks ([`randomizedtesting::jdk::system`]); the captured `PrintStream`s are
/// the counting sinks the static initializer installs, compared by identity.
/// A sink has no class, so `checkCaptureStreams` names the replacement
/// `java.io.PrintStream`, the class of every stream Lucene's tests install.
pub struct TestRuleLimitSysouts {
    /// "Test failures from any tests or rules before."
    failure_marker: Arc<TestRuleMarkFailure>,
}

impl TestRuleLimitSysouts {
    /// `DEFAULT_LIMIT`: "max limit of bytes printed to either `System.out`
    /// or `System.err`. This limit is enforced per-class (suite)."
    pub const DEFAULT_LIMIT: i64 = 8 * KB;
    /// `DEFAULT_HARD_LIMIT`: "max hard limit of sysout bytes."
    pub const DEFAULT_HARD_LIMIT: i64 = 2 * GB;
    /// `MAX_LIMIT`: "maximum limit allowed for `Limit.bytes()` before sysout
    /// check suppression is suggested."
    pub const MAX_LIMIT: i64 = MB;

    /// `new TestRuleLimitSysouts(failureMarker)`.
    pub fn new(failure_marker: Arc<TestRuleMarkFailure>) -> TestRuleLimitSysouts {
        LazyLock::force(&CAPTURED);
        TestRuleLimitSysouts { failure_marker }
    }

    /// `checkCaptureStreams()`: "ensures `System.out` and `System.err` point
    /// to delegate streams."
    ///
    /// # Panics
    ///
    /// `AssertionError` ("Something has changed System.out to: …").
    pub fn check_capture_streams() {
        let captured = &*CAPTURED;
        let same = |current: Option<PrintSink>, captured: &PrintSink| {
            current.is_some_and(|c| Arc::ptr_eq(&c, captured))
        };
        // Make sure we still hold the right references to wrapper streams.
        if !same(system::out(), &captured.out) {
            Throwable::new(
                exception::ASSERTION_ERROR,
                "Something has changed System.out to: java.io.PrintStream",
            )
            .throw();
        }
        if !same(system::err(), &captured.err) {
            Throwable::new(
                exception::ASSERTION_ERROR,
                "Something has changed System.err to: java.io.PrintStream",
            )
            .throw();
        }
    }

    /// The bytes written to the captured streams since the last reset.
    pub fn bytes_written() -> i64 {
        BYTES_WRITTEN.load(Ordering::SeqCst)
    }

    fn target_class() -> Arc<Class> {
        match RandomizedTest::get_context().get_target_class_object() {
            Some(c) => c,
            None => exception::throw_without_message(exception::NULL_POINTER_EXCEPTION),
        }
    }

    fn apply_class_annotations() {
        let target = Self::target_class();
        if let Some(limit_ann) = target.get_annotation::<Limit>() {
            let bytes = limit_ann.bytes;
            if !(0..=Self::MAX_LIMIT).contains(&bytes) {
                Throwable::new(
                    exception::ASSERTION_ERROR,
                    format!(
                        "This sysout limit is very high: {bytes}. Did you want to use \
                         @{SUPPRESS_SYSOUT_CHECKS_NAME} annotation to avoid sysout checks \
                         entirely (this is discouraged)?"
                    ),
                )
                .throw();
            }
            HARD_LIMIT.store(limit_ann.hard_limit, Ordering::SeqCst);
        }
    }

    /// `isEnforced()`.
    pub fn is_enforced() -> bool {
        let target = Self::target_class();
        if *VERBOSE
            || *INFOSTREAM
            || target.is_annotation_present(&MONSTER)
            || target.is_annotation_present(&SUPPRESS_SYSOUT_CHECKS)
        {
            return false;
        }
        target.is_annotation_present(&LIMIT)
    }

    fn reset_capture_state() {
        BYTES_WRITTEN.store(0, Ordering::SeqCst);
        HARD_LIMIT.store(i32::MAX as i64, Ordering::SeqCst);
    }
}

impl TestRuleAdapter for TestRuleLimitSysouts {
    /// `before()`.
    fn before(&self) {
        if Self::is_enforced() {
            Self::check_capture_streams();
        }
        Self::reset_capture_state();
        Self::apply_class_annotations();
    }

    /// `afterIfSuccessful()`: "we're only interested in failing the suite if
    /// it was successful (otherwise just propagate the original problem and
    /// don't bother doing anything else)."
    fn after_if_successful(&self) {
        if Self::is_enforced() {
            Self::check_capture_streams();
            // Check for offenders, but only if everything was successful so far.
            let ann = Self::target_class()
                .get_annotation::<Limit>()
                .expect("INVARIANT: isEnforced() requires @Limit");
            let limit = ann.bytes;
            let hard_limit = ann.hard_limit;
            let written = BYTES_WRITTEN.load(Ordering::SeqCst);
            if written >= limit && self.failure_marker.was_successful() {
                Throwable::new(
                    exception::ASSERTION_ERROR,
                    format!(
                        "The test or suite printed {written} bytes to stdout and stderr, even \
                         though the limit was set to {limit} bytes.{} Increase the limit with \
                         @Limit, ignore it completely with @SuppressSysoutChecks or run with \
                         -Dtests.verbose=true",
                        if written <= hard_limit {
                            ""
                        } else {
                            "Hard limit was enforced so output is truncated."
                        }
                    ),
                )
                .throw();
            }
        }
    }

    /// `afterAlways(errors)`.
    fn after_always(&self, _errors: &mut Vec<Throwable>) {
        Self::reset_capture_state();
    }
}
