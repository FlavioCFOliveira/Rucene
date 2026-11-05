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

//! Port of the static environment and randomness API of
//! `org.apache.lucene.tests.util.LuceneTestCase` (task #338): the
//! `tests.*`-derived constants every framework class reads, `random()` and
//! the static helpers built on them; and the searcher factory `newSearcher`
//! with what it calls (task #345, under the sprint's cycle rule). The rules,
//! the index factories and the remaining members are ported by later tasks.

use std::collections::HashSet;
use std::fmt::Display;
use std::hash::Hash;
use std::io::{self, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::{AnnotationType, TestGroup};
use randomizedtesting::generators::{RandomNumbers, RandomPicks};
use randomizedtesting::jdk::exception::{
    self, java_assert, register_class, Throwable, ASSERTION_ERROR,
};
use randomizedtesting::jdk::lang::{double_to_string, float_to_string};
use randomizedtesting::jdk::locale::available_locales_sorted;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{quote_member, Class, JavaAnnotation};
use randomizedtesting::jdk::system;
use randomizedtesting::junit::Assert;
use randomizedtesting::{RandomizedContext, RandomizedTest};
use rucene::index::{
    CompositeReader, DirectoryReader, IndexReader, LeafReader, LeafReaderContext, MultiReader,
    ParallelCompositeReader, ParallelLeafReader, QueryTimeout,
};
use rucene::search::similarities::Similarity;
use rucene::search::{
    Collector, IndexSearcher, IndexSearcherOverrides, LeafReaderContextPartition, LeafSlice, Query,
    QueryCachingPolicy, ScoreMode, Weight,
};
use rucene::store::java_nio_file::{Files, IntoNioPath, Path};
use rucene::util::BytesRef;

use crate::hamcrest::{Matcher, MatcherAssert};
use crate::index::{
    AssertingDirectoryReader, AssertingLeafReader, FieldFilterLeafReader, MergingCodecReader,
    MergingDirectoryReaderWrapper, MismatchedDirectoryReader, MismatchedLeafReader,
};
use crate::jdk::locale::Locale;
use crate::jdk::{time_zone_available_ids, RUNTIME_VERSION_FEATURE};
use crate::search::AssertingIndexSearcher;
use crate::store::Throttling;
use crate::util::test_rule_mark_failure::TestRuleMarkFailure;
use crate::util::test_rule_store_class_name::TestRuleStoreClassName;
use crate::util::test_rule_temporary_files_cleanup::TestRuleTemporaryFilesCleanup;
use crate::util::test_util::TestUtil;

/// `junit.framework.AssertionFailedError`, which `expectThrows` throws.
pub const ASSERTION_FAILED_ERROR: &str = "junit.framework.AssertionFailedError";

/// `org.junit.internal.AssumptionViolatedException`.
const ASSUMPTION_VIOLATED_EXCEPTION: &str = "org.junit.internal.AssumptionViolatedException";

/// Port of the annotation `LuceneTestCase.Nightly`: tests that should only be
/// run during nightly builds (`@TestGroup(enabled = false, sysProperty =
/// SYSPROP_NIGHTLY)`). Ported under the cycle rule: `TEST_NIGHTLY` reads its
/// `TestGroup`.
pub const NIGHTLY: AnnotationType = AnnotationType {
    name: "org.apache.lucene.tests.util.LuceneTestCase$Nightly",
    simple_name: "Nightly",
    test_group: Some(TestGroup {
        name: "",
        sys_property: LuceneTestCase::SYSPROP_NIGHTLY,
        enabled: false,
    }),
    inherited: true,
};

/// Port of the annotation `LuceneTestCase.Weekly`: tests that should only be
/// run during weekly builds (`@TestGroup(enabled = false, sysProperty =
/// SYSPROP_WEEKLY)`).
pub const WEEKLY: AnnotationType = AnnotationType {
    name: "org.apache.lucene.tests.util.LuceneTestCase$Weekly",
    simple_name: "Weekly",
    test_group: Some(TestGroup {
        name: "",
        sys_property: LuceneTestCase::SYSPROP_WEEKLY,
        enabled: false,
    }),
    inherited: true,
};

/// Port of the annotation `LuceneTestCase.Monster`: monster tests that
/// require special setup (`@TestGroup(enabled = false, sysProperty =
/// SYSPROP_MONSTER)`).
pub const MONSTER: AnnotationType = AnnotationType {
    name: "org.apache.lucene.tests.util.LuceneTestCase$Monster",
    simple_name: "Monster",
    test_group: Some(TestGroup {
        name: "",
        sys_property: LuceneTestCase::SYSPROP_MONSTER,
        enabled: false,
    }),
    inherited: true,
};

/// Port of the annotation `LuceneTestCase.AwaitsFix`: tests which exhibit a
/// known issue and are temporarily disabled (`@TestGroup(enabled = false,
/// sysProperty = SYSPROP_AWAITSFIX)`).
pub const AWAITS_FIX: AnnotationType = AnnotationType {
    name: "org.apache.lucene.tests.util.LuceneTestCase$AwaitsFix",
    simple_name: "AwaitsFix",
    test_group: Some(TestGroup {
        name: "",
        sys_property: LuceneTestCase::SYSPROP_AWAITSFIX,
        enabled: false,
    }),
    inherited: true,
};

/// `@SuppressFileSystems` (`LuceneTestCase.SuppressFileSystems`, `@Inherited`):
/// the mock filesystems a suite does not want, by simple name, or `*` for all.
/// Ported under the cycle rule: `TestRuleTemporaryFilesCleanup` reads it.
pub static SUPPRESS_FILE_SYSTEMS: AnnotationType = AnnotationType::plain(
    "org.apache.lucene.tests.util.LuceneTestCase$SuppressFileSystems",
    "SuppressFileSystems",
    true,
);

/// An `@SuppressFileSystems(value)` annotation value.
#[derive(Debug, Clone, Default)]
pub struct SuppressFileSystems {
    /// `value()`.
    pub value: Vec<String>,
}

impl JavaAnnotation for SuppressFileSystems {
    fn annotation_type(&self) -> &'static AnnotationType {
        &SUPPRESS_FILE_SYSTEMS
    }
    fn to_java_string(&self) -> String {
        let items: Vec<String> = self.value.iter().map(|v| quote_member(v)).collect();
        format!(
            "@org.apache.lucene.tests.util.LuceneTestCase.SuppressFileSystems({{{}}})",
            items.join(", ")
        )
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// `@SuppressAssertingFormats` (`LuceneTestCase.SuppressAssertingFormats`,
/// `@Inherited`): suppresses specific asserting formats of the
/// `AssertingCodec` while keeping the other asserting formats active. Ported
/// under the cycle rule: `AssertingCodec`'s constructor reads it.
pub static SUPPRESS_ASSERTING_FORMATS: AnnotationType = AnnotationType::plain(
    "org.apache.lucene.tests.util.LuceneTestCase$SuppressAssertingFormats",
    "SuppressAssertingFormats",
    true,
);

/// An `@SuppressAssertingFormats(value)` annotation value.
#[derive(Debug, Clone, Default)]
pub struct SuppressAssertingFormats {
    /// `value()`.
    pub value: Vec<crate::codecs::asserting::Format>,
}

impl JavaAnnotation for SuppressAssertingFormats {
    fn annotation_type(&self) -> &'static AnnotationType {
        &SUPPRESS_ASSERTING_FORMATS
    }
    /// JDK 25's `Annotation.toString()`, enum constants by name: checked
    /// against `java` on `@SuppressAssertingFormats({STORED_FIELDS, NORMS})`.
    fn to_java_string(&self) -> String {
        let items: Vec<String> = self.value.iter().map(|v| v.to_string()).collect();
        format!(
            "@org.apache.lucene.tests.util.LuceneTestCase.SuppressAssertingFormats({{{}}})",
            items.join(", ")
        )
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// `@SuppressFsync` (`LuceneTestCase.SuppressFsync`, `@Inherited`): never
/// omit actual fsync calls. Ported under the cycle rule.
pub static SUPPRESS_FSYNC: AnnotationType = AnnotationType::plain(
    "org.apache.lucene.tests.util.LuceneTestCase$SuppressFsync",
    "SuppressFsync",
    true,
);

/// `@SuppressTempFileChecks` (`LuceneTestCase.SuppressTempFileChecks`,
/// `@Inherited`): leftover temporary files only warn. Ported under the cycle
/// rule.
pub static SUPPRESS_TEMP_FILE_CHECKS: AnnotationType = AnnotationType::plain(
    "org.apache.lucene.tests.util.LuceneTestCase$SuppressTempFileChecks",
    "SuppressTempFileChecks",
    true,
);

/// An `@SuppressTempFileChecks(bugUrl)` annotation value.
#[derive(Debug, Clone)]
pub struct SuppressTempFileChecks {
    /// `bugUrl()`, `"None"` by default.
    pub bug_url: String,
}

impl Default for SuppressTempFileChecks {
    fn default() -> Self {
        SuppressTempFileChecks {
            bug_url: "None".to_string(),
        }
    }
}

impl JavaAnnotation for SuppressTempFileChecks {
    fn annotation_type(&self) -> &'static AnnotationType {
        &SUPPRESS_TEMP_FILE_CHECKS
    }
    fn to_java_string(&self) -> String {
        format!(
            "@org.apache.lucene.tests.util.LuceneTestCase.SuppressTempFileChecks(bugUrl={})",
            quote_member(&self.bug_url)
        )
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// `suiteFailureMarker`: the suite's failure marker, in `LuceneTestCase`'s
/// class rules (task #348).
pub static SUITE_FAILURE_MARKER: LazyLock<Arc<TestRuleMarkFailure>> =
    LazyLock::new(|| Arc::new(TestRuleMarkFailure::new(Vec::new())));

/// `tempFilesCleanupRule`: the rule `createTempDir` and `createTempFile`
/// delegate to, in `LuceneTestCase`'s class rules (task #348).
pub static TEMP_FILES_CLEANUP_RULE: LazyLock<Arc<TestRuleTemporaryFilesCleanup>> =
    LazyLock::new(|| {
        Arc::new(TestRuleTemporaryFilesCleanup::new(Arc::clone(
            &SUITE_FAILURE_MARKER,
        )))
    });

fn test_group_enabled(annotation: &AnnotationType) -> bool {
    annotation
        .test_group
        .map(|group| group.enabled)
        .expect("INVARIANT: the annotation is a test group")
}

/// The `tests.*`-derived constants of `LuceneTestCase`, as its static
/// initializer computes them from the system properties.
///
/// Java computes them once, when the class is initialized; the statics of
/// this module ([`VERBOSE`], [`TEST_NIGHTLY`], …) are computed once too, from
/// [`StaticEnvironment::from_system_properties`], on first use. The struct
/// lets a property configuration be evaluated again, as a new JVM would.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticEnvironment {
    /// `VERBOSE`.
    pub verbose: bool,
    /// `INFOSTREAM`.
    pub infostream: bool,
    /// `TEST_ASSERTS_ENABLED`.
    pub test_asserts_enabled: bool,
    /// `TEST_CODEC`.
    pub test_codec: String,
    /// `TEST_POSTINGSFORMAT`.
    pub test_postingsformat: String,
    /// `TEST_DOCVALUESFORMAT`.
    pub test_docvaluesformat: String,
    /// `TEST_DIRECTORY`.
    pub test_directory: String,
    /// `TEST_LINE_DOCS_FILE`.
    pub test_line_docs_file: String,
    /// `TEST_NIGHTLY`.
    pub test_nightly: bool,
    /// `TEST_WEEKLY`.
    pub test_weekly: bool,
    /// `TEST_MONSTER`.
    pub test_monster: bool,
    /// `TEST_AWAITSFIX`.
    pub test_awaitsfix: bool,
    /// `TEST_THROTTLING`.
    pub test_throttling: Throttling,
    /// `RANDOM_MULTIPLIER`.
    pub random_multiplier: i32,
    /// `LEAVE_TEMPORARY`.
    pub leave_temporary: bool,
}

impl StaticEnvironment {
    /// The static initializer of `LuceneTestCase`, in declaration order.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` from `systemPropertyAsBoolean` or
    /// `systemPropertyAsInt` for a property that does not parse (Java's
    /// `ExceptionInInitializerError` carries it).
    pub fn from_system_properties() -> StaticEnvironment {
        let as_boolean = RandomizedTest::system_property_as_boolean;
        let property = |key: &str, default: &str| {
            system::get_property(key).unwrap_or_else(|| default.to_string())
        };
        let verbose = as_boolean("tests.verbose", false);
        let infostream = as_boolean("tests.infostream", verbose);
        let test_asserts_enabled = as_boolean("tests.asserts", true);
        let test_codec = property("tests.codec", "random");
        let test_postingsformat = property("tests.postingsformat", "random");
        let test_docvaluesformat = property("tests.docvaluesformat", "random");
        let test_directory = property("tests.directory", "random");
        let test_line_docs_file =
            property("tests.linedocsfile", LuceneTestCase::DEFAULT_LINE_DOCS_FILE);
        let test_nightly = as_boolean(
            LuceneTestCase::SYSPROP_NIGHTLY,
            test_group_enabled(&NIGHTLY),
        );
        let test_weekly = as_boolean(LuceneTestCase::SYSPROP_WEEKLY, test_group_enabled(&WEEKLY));
        let test_monster = as_boolean(
            LuceneTestCase::SYSPROP_MONSTER,
            test_group_enabled(&MONSTER),
        );
        let test_awaitsfix = as_boolean(
            LuceneTestCase::SYSPROP_AWAITSFIX,
            test_group_enabled(&AWAITS_FIX),
        );
        let test_throttling = if test_nightly {
            Throttling::Sometimes
        } else {
            Throttling::Never
        };
        let random_multiplier = RandomizedTest::system_property_as_int(
            "tests.multiplier",
            default_random_multiplier_of(test_nightly),
        );
        let mut leave_temporary = false;
        for property in [
            "tests.leaveTemporary", /* ANT tasks' (junit4) flag. */
            "tests.leavetemporary", /* lowercase */
            "tests.leavetmpdir",    /* default */
        ] {
            leave_temporary |= as_boolean(property, false);
        }
        StaticEnvironment {
            verbose,
            infostream,
            test_asserts_enabled,
            test_codec,
            test_postingsformat,
            test_docvaluesformat,
            test_directory,
            test_line_docs_file,
            test_nightly,
            test_weekly,
            test_monster,
            test_awaitsfix,
            test_throttling,
            random_multiplier,
            leave_temporary,
        }
    }
}

/// `defaultRandomMultiplier()` of a configuration: 2 when nightly, 1
/// otherwise.
fn default_random_multiplier_of(test_nightly: bool) -> i32 {
    if test_nightly {
        2
    } else {
        1
    }
}

static ENVIRONMENT: LazyLock<StaticEnvironment> =
    LazyLock::new(StaticEnvironment::from_system_properties);

/// `VERBOSE`: true if and only if tests are run in verbose mode
/// (`tests.verbose`). If false, tests are not expected to print any messages.
pub static VERBOSE: LazyLock<bool> = LazyLock::new(|| ENVIRONMENT.verbose);

/// `INFOSTREAM`: enables or disables dumping of `InfoStream` messages
/// (`tests.infostream`, `VERBOSE` by default).
pub static INFOSTREAM: LazyLock<bool> = LazyLock::new(|| ENVIRONMENT.infostream);

/// `TEST_ASSERTS_ENABLED` (`tests.asserts`, `true` by default).
pub static TEST_ASSERTS_ENABLED: LazyLock<bool> =
    LazyLock::new(|| ENVIRONMENT.test_asserts_enabled);

/// `TEST_CODEC`: the codec to run tests with (`tests.codec`, `random`).
pub static TEST_CODEC: LazyLock<String> = LazyLock::new(|| ENVIRONMENT.test_codec.clone());

/// `TEST_POSTINGSFORMAT`: the postings format to run tests with
/// (`tests.postingsformat`, `random`).
pub static TEST_POSTINGSFORMAT: LazyLock<String> =
    LazyLock::new(|| ENVIRONMENT.test_postingsformat.clone());

/// `TEST_DOCVALUESFORMAT`: the doc values format to run tests with
/// (`tests.docvaluesformat`, `random`).
pub static TEST_DOCVALUESFORMAT: LazyLock<String> =
    LazyLock::new(|| ENVIRONMENT.test_docvaluesformat.clone());

/// `TEST_DIRECTORY`: the directory to run tests with (`tests.directory`,
/// `random`).
pub static TEST_DIRECTORY: LazyLock<String> = LazyLock::new(|| ENVIRONMENT.test_directory.clone());

/// `TEST_LINE_DOCS_FILE`: the line file used in tests (`tests.linedocsfile`,
/// [`LuceneTestCase::DEFAULT_LINE_DOCS_FILE`]).
pub static TEST_LINE_DOCS_FILE: LazyLock<String> =
    LazyLock::new(|| ENVIRONMENT.test_line_docs_file.clone());

/// `TEST_NIGHTLY`: whether `Nightly` tests should run.
pub static TEST_NIGHTLY: LazyLock<bool> = LazyLock::new(|| ENVIRONMENT.test_nightly);

/// `TEST_WEEKLY`: whether `Weekly` tests should run.
pub static TEST_WEEKLY: LazyLock<bool> = LazyLock::new(|| ENVIRONMENT.test_weekly);

/// `TEST_MONSTER`: whether `Monster` tests should run.
pub static TEST_MONSTER: LazyLock<bool> = LazyLock::new(|| ENVIRONMENT.test_monster);

/// `TEST_AWAITSFIX`: whether `AwaitsFix` tests should run.
pub static TEST_AWAITSFIX: LazyLock<bool> = LazyLock::new(|| ENVIRONMENT.test_awaitsfix);

/// `TEST_THROTTLING`: `SOMETIMES` when nightly, `NEVER` otherwise.
pub static TEST_THROTTLING: LazyLock<Throttling> = LazyLock::new(|| ENVIRONMENT.test_throttling);

/// `RANDOM_MULTIPLIER`: a random multiplier which you should use when writing
/// random tests: multiply it by the number of iterations to scale your tests
/// (for nightly builds); `tests.multiplier`, 2 when nightly and 1 otherwise.
pub static RANDOM_MULTIPLIER: LazyLock<i32> = LazyLock::new(|| ENVIRONMENT.random_multiplier);

/// `LEAVE_TEMPORARY`: leave temporary files on disk, even on successful runs
/// (`tests.leaveTemporary`, `tests.leavetemporary` or `tests.leavetmpdir`).
pub static LEAVE_TEMPORARY: LazyLock<bool> = LazyLock::new(|| ENVIRONMENT.leave_temporary);

/// The anonymous `QueryCachingPolicy` of `MAYBE_CACHE_POLICY`: randomly
/// caches.
#[derive(Debug)]
pub struct MaybeCachePolicy;

impl QueryCachingPolicy for MaybeCachePolicy {
    fn on_use(&self, _query: &dyn Query) {}

    /// `shouldCache(query)`: `random().nextBoolean()`.
    fn should_cache(&self, _query: &dyn Query) -> rucene::error::Result<bool> {
        Ok(LuceneTestCase::random().next_boolean())
    }
}

/// `MAYBE_CACHE_POLICY`: a `QueryCachingPolicy` that randomly caches.
pub static MAYBE_CACHE_POLICY: LazyLock<Arc<dyn QueryCachingPolicy>> =
    LazyLock::new(|| Arc::new(MaybeCachePolicy));

/// `classNameRule`: the rule that stores the class under test, which
/// `getTestClass()` reads; installed by `LuceneTestCase`'s class rules.
pub static CLASS_NAME_RULE: LazyLock<Arc<TestRuleStoreClassName>> =
    LazyLock::new(|| Arc::new(TestRuleStoreClassName::new()));

/// `DEPRECATED_TIME_ZONE_IDS_JDK25`: time zone IDs that cause a deprecation
/// warning in JDK 25.
const DEPRECATED_TIME_ZONE_IDS_JDK25: [&str; 28] = [
    "ACT", "AET", "AGT", "ART", "AST", "BET", "BST", "CAT", "CNT", "CST", "CTT", "EAT", "ECT",
    "EST", "HST", "IET", "IST", "JST", "MIT", "MST", "NET", "NST", "PLT", "PNT", "PRT", "PST",
    "SST", "VST",
];

/// A `java.util.TimeZone` of JDK 25's available IDs, as
/// `TimeZone.getTimeZone(id)` returns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimeZone {
    id: &'static str,
}

impl TimeZone {
    /// `getID()`.
    pub fn get_id(&self) -> &'static str {
        self.id
    }
}

/// `availableLanguageTags`: the language tags of the available locales,
/// sorted and distinct.
fn available_language_tags() -> &'static [&'static str] {
    static TAGS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
        let mut tags: Vec<&'static str> = available_locales_sorted()
            .iter()
            .map(|l| l.to_language_tag())
            .collect();
        tags.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        tags.dedup();
        tags
    });
    &TAGS
}

/// `Class.getSimpleName()` of a class name.
fn simple_name(class_name: &str) -> &str {
    class_name.rsplit(['.', '$']).next().unwrap_or(class_name)
}

/// `List.toString()` of class simple names.
fn simple_names(class_names: &[&str]) -> String {
    let names: Vec<&str> = class_names.iter().map(|c| simple_name(c)).collect();
    format!("[{}]", names.join(", "))
}

fn register_assertion_failed_error() {
    register_class(ASSERTION_FAILED_ERROR, ASSERTION_ERROR);
}

/// Port of the static environment and randomness API of
/// `org.apache.lucene.tests.util.LuceneTestCase`.
pub struct LuceneTestCase;

impl LuceneTestCase {
    /// `SYSPROP_NIGHTLY`.
    pub const SYSPROP_NIGHTLY: &'static str = "tests.nightly";
    /// `SYSPROP_WEEKLY`.
    pub const SYSPROP_WEEKLY: &'static str = "tests.weekly";
    /// `SYSPROP_MONSTER`.
    pub const SYSPROP_MONSTER: &'static str = "tests.monster";
    /// `SYSPROP_AWAITSFIX`.
    pub const SYSPROP_AWAITSFIX: &'static str = "tests.awaitsfix";
    /// `SYSPROP_MAXFAILURES`.
    pub const SYSPROP_MAXFAILURES: &'static str = "tests.maxfailures";
    /// `SYSPROP_FAILFAST`.
    pub const SYSPROP_FAILFAST: &'static str = "tests.failfast";
    /// `DEFAULT_LINE_DOCS_FILE`: the default (embedded resource) lines file.
    pub const DEFAULT_LINE_DOCS_FILE: &'static str = "europarl.lines.txt.gz";
    /// `JENKINS_LARGE_LINE_DOCS_FILE`: a random sample from enwiki used in
    /// tests.
    pub const JENKINS_LARGE_LINE_DOCS_FILE: &'static str = "enwiki.random.lines.txt";

    /// `defaultRandomMultiplier()`: the default of the random multiplier,
    /// based on `TEST_NIGHTLY`.
    pub fn default_random_multiplier() -> i32 {
        default_random_multiplier_of(*TEST_NIGHTLY)
    }

    /// `random()`: the random of the current context,
    /// `RandomizedContext.current().getRandom()`.
    ///
    /// # Panics
    ///
    /// As `RandomizedContext.current()` outside a randomized context.
    pub fn random() -> Arc<dyn Random> {
        RandomizedContext::current().get_random()
    }

    /// `getTestClass()`: the class currently being tested,
    /// `classNameRule.getTestClass()`.
    ///
    /// # Panics
    ///
    /// `RuntimeException` ("The rule is not currently executing.") outside the
    /// class rule.
    pub fn get_test_class() -> Arc<Class> {
        CLASS_NAME_RULE.get_test_class()
    }

    /// `atLeast(random, i)`: a number of at least `i`, influenced by
    /// `RANDOM_MULTIPLIER` and some random fudge: `TestUtil.nextInt(random,
    /// min, min + min / 2)` with `min = i * RANDOM_MULTIPLIER`.
    pub fn at_least_with(random: &dyn Random, i: i32) -> i32 {
        let min = i.wrapping_mul(*RANDOM_MULTIPLIER);
        let max = min.wrapping_add(min / 2);
        TestUtil::next_int(random, min, max)
    }

    /// `newIOContext(random)`: `newIOContext(random, IOContext.DEFAULT)`.
    pub fn new_io_context(random: &dyn Random) -> Box<dyn rucene::store::IOContext> {
        Self::new_io_context_from(random, &*rucene::store::DEFAULT_IO_CONTEXT)
    }

    /// `newIOContext(random, oldContext)`: `oldContext` itself when it holds
    /// the `ReadOnceHint`; else a random context that keeps at least the
    /// estimated size of `oldContext`'s flush or merge information, or, for
    /// neither, a random one of `DEFAULT`, a merge context and a flush
    /// context.
    ///
    /// **Divergence.** Java returns `oldContext` itself; a Rust trait object
    /// cannot be returned by value from a reference, so the port returns a
    /// context with the same hints, merge and flush information.
    pub fn new_io_context_from(
        random: &dyn Random,
        old_context: &dyn rucene::store::IOContext,
    ) -> Box<dyn rucene::store::IOContext> {
        use rucene::store::{
            flush_io_context, hints_of, merge_io_context, DefaultIOContext, FlushInfo, MergeInfo,
            ReadOnceHint,
        };
        if !hints_of::<ReadOnceHint>(old_context).is_empty() {
            // just return as-is
            return match (old_context.merge_info(), old_context.flush_info()) {
                (Some(m), _) => merge_io_context(m),
                (_, Some(f)) => flush_io_context(f),
                _ => Box::new(
                    DefaultIOContext::with_hints_slice(old_context.hints())
                        .expect("INVARIANT: the hints of a valid context are valid"),
                ),
            };
        }
        let random_num_docs = random.next_int_bound(4192);
        let size = random.next_int_bound(512).wrapping_mul(random_num_docs);
        if let Some(flush) = old_context.flush_info() {
            // Always return at least the estimatedSegmentSize of
            // the incoming IOContext:
            flush_io_context(FlushInfo::new(
                random_num_docs,
                flush.estimated_segment_size.max(i64::from(size)),
            ))
        } else if let Some(merge) = old_context.merge_info() {
            // Always return at least the estimatedMergeBytes of
            // the incoming IOContext:
            let estimated = merge.estimated_merge_bytes.max(i64::from(size));
            let is_external = random.next_boolean();
            merge_io_context(MergeInfo::new(
                random_num_docs,
                estimated,
                is_external,
                TestUtil::next_int(random, 1, 100),
            ))
        } else {
            // Make a totally random IOContext
            match random.next_int_bound(3) {
                1 => merge_io_context(MergeInfo::new(random_num_docs, i64::from(size), true, -1)),
                2 => flush_io_context(FlushInfo::new(random_num_docs, i64::from(size))),
                _ => Box::new(
                    DefaultIOContext::with_hints_slice(&[])
                        .expect("INVARIANT: no hints are valid hints"),
                ),
            }
        }
    }

    /// `atLeast(i)`: `atLeast(random(), i)`.
    pub fn at_least(i: i32) -> i32 {
        Self::at_least_with(&*Self::random(), i)
    }

    /// `rarely(random)`: whether something should happen rarely; the chance
    /// grows with `TEST_NIGHTLY` and `RANDOM_MULTIPLIER` but is never more
    /// than 20%.
    pub fn rarely_with(random: &dyn Random) -> bool {
        let mut p: i32 = if *TEST_NIGHTLY { 5 } else { 1 };
        // `p += (p * Math.log(RANDOM_MULTIPLIER))`: the compound assignment
        // narrows the double sum to an int.
        p = (f64::from(p) + f64::from(p) * f64::from(*RANDOM_MULTIPLIER).ln()) as i32;
        let min = 100i32.wrapping_sub(p.min(20)); // never more than 20
        random.next_int_bound(100) >= min
    }

    /// `rarely()`: `rarely(random())`.
    pub fn rarely() -> bool {
        Self::rarely_with(&*Self::random())
    }

    /// `usually(random)`: `!rarely(random)`.
    pub fn usually_with(random: &dyn Random) -> bool {
        !Self::rarely_with(random)
    }

    /// `usually()`: `usually(random())`.
    pub fn usually() -> bool {
        Self::usually_with(&*Self::random())
    }

    /// `assumeTrue(msg, condition)`: `RandomizedTest.assumeTrue`.
    ///
    /// # Panics
    ///
    /// `AssumptionViolatedException` (`msg`) when `condition` is false.
    pub fn assume_true(msg: &str, condition: bool) {
        RandomizedTest::assume_true_msg(msg, condition);
    }

    /// `assumeFalse(msg, condition)`: `RandomizedTest.assumeFalse`.
    ///
    /// # Panics
    ///
    /// `AssumptionViolatedException` (`msg`) when `condition` is true.
    pub fn assume_false(msg: &str, condition: bool) {
        RandomizedTest::assume_false_msg(msg, condition);
    }

    /// `assumeNoException(msg, e)`: `RandomizedTest.assumeNoException`.
    ///
    /// # Panics
    ///
    /// `AssumptionViolatedException` (`msg`, caused by `e`) when `e` is
    /// present.
    pub fn assume_no_exception(msg: &str, e: Option<&Throwable>) {
        RandomizedTest::assume_no_exception_msg(msg, e);
    }

    /// `assertFloatUlpEquals(x, y, maxUlps)`.
    ///
    /// # Panics
    ///
    /// `AssertionError` ("x and y are not within maxUlps ULPs of each other").
    pub fn assert_float_ulp_equals(x: f32, y: f32, max_ulps: i16) {
        Assert::assert_true_msg(
            &format!(
                "{} and {} are not within {max_ulps} ULPs of each other",
                float_to_string(x),
                float_to_string(y)
            ),
            TestUtil::float_ulp_equals(x, y, max_ulps),
        );
    }

    /// `assertDoubleUlpEquals(x, y, maxUlps)`.
    ///
    /// # Panics
    ///
    /// `AssertionError` ("x and y are not within maxUlps ULPs of each other").
    pub fn assert_double_ulp_equals(x: f64, y: f64, max_ulps: i32) {
        Assert::assert_true_msg(
            &format!(
                "{} and {} are not within {max_ulps} ULPs of each other",
                double_to_string(x),
                double_to_string(y)
            ),
            TestUtil::double_ulp_equals(x, y, max_ulps),
        );
    }

    /// `asSet(args...)`: the arguments as a set; the order of the elements is
    /// not preserved in iterators.
    pub fn as_set<T: Eq + Hash>(args: impl IntoIterator<Item = T>) -> HashSet<T> {
        args.into_iter().collect()
    }

    /// `dumpIterator(label, iter, stream)`: logs `*** BEGIN label ***`, each
    /// item's `toString()` on its own line (or ` ... NULL ...` for a null
    /// iterator, `None`), then `*** END label ***`.
    ///
    /// # Errors
    ///
    /// The failure of the stream. (Java's `PrintStream` swallows it.)
    pub fn dump_iterator<I>(label: &str, iter: Option<I>, stream: &mut dyn Write) -> io::Result<()>
    where
        I: Iterator,
        I::Item: Display,
    {
        writeln!(stream, "*** BEGIN {label} ***")?;
        match iter {
            None => writeln!(stream, " ... NULL ...")?,
            Some(iter) => {
                for item in iter {
                    writeln!(stream, "{item}")?;
                }
            }
        }
        writeln!(stream, "*** END {label} ***")
    }

    /// `dumpArray(label, objs, stream)`: wraps the array in an iterator and
    /// delegates to [`dump_iterator`](Self::dump_iterator).
    ///
    /// # Errors
    ///
    /// The failure of the stream.
    pub fn dump_array<T: Display>(
        label: &str,
        objs: Option<&[T]>,
        stream: &mut dyn Write,
    ) -> io::Result<()> {
        Self::dump_iterator(label, objs.map(|o| o.iter()), stream)
    }

    /// `callStackContains(Class, methodName)`: whether a method `method_name`
    /// of the type `T` called us (this utility method excluded).
    ///
    /// **Divergence.** See [`crate::jdk::stack_walker`]: the class is the Rust
    /// path of `T`, and the frames are read from the build's symbols.
    pub fn call_stack_contains_method<T: ?Sized>(method_name: &str) -> bool {
        let class_name = std::any::type_name::<T>();
        crate::jdk::stack_walker::frames()
            .iter()
            .skip(1)
            .any(|f| f.class_name == class_name && f.method_name == method_name)
    }

    /// `callStackContainsAnyOf(methodNames...)`: whether a method of one of
    /// the given names (no class restriction) called us.
    pub fn call_stack_contains_any_of(method_names: &[&str]) -> bool {
        crate::jdk::stack_walker::frames()
            .iter()
            .skip(1)
            .any(|f| method_names.contains(&f.method_name.as_str()))
    }

    /// `callStackContains(Class)`: whether the type `T` called us.
    pub fn call_stack_contains<T: ?Sized>() -> bool {
        let class_name = std::any::type_name::<T>();
        crate::jdk::stack_walker::frames()
            .iter()
            .skip(1)
            .any(|f| f.class_name == class_name)
    }

    /// `_expectThrows(expectedTypes, runnable)`: an `AssertionError` or
    /// `AssumptionViolatedException` propagates unless an expected type is
    /// its class or a superclass; any other throwable is returned; `None`
    /// when the runnable completes.
    fn expect_throws_internal(
        expected_types: &[&str],
        runnable: impl FnOnce() -> Result<(), Throwable>,
    ) -> Option<Throwable> {
        let thrown = match catch_unwind(AssertUnwindSafe(runnable)) {
            Ok(Ok(())) => return None,
            Ok(Err(t)) => t,
            Err(payload) => Throwable::from_panic(payload),
        };
        if thrown.is_instance(ASSERTION_ERROR) || thrown.is_instance(ASSUMPTION_VIOLATED_EXCEPTION)
        {
            if expected_types.iter().any(|t| thrown.is_instance(t)) {
                return Some(thrown);
            }
            thrown.throw()
        }
        Some(thrown)
    }

    /// `expectThrows(expectedType, runnable)`: checks a specific exception
    /// class is thrown by the runnable, and returns it. The runnable throws by
    /// panicking with a [`Throwable`] or by returning one.
    ///
    /// # Panics
    ///
    /// `AssertionFailedError` ("Expected exception X but no exception was
    /// thrown", or "Unexpected exception type, expected X but got …").
    pub fn expect_throws(
        expected_type: &str,
        runnable: impl FnOnce() -> Result<(), Throwable>,
    ) -> Throwable {
        Self::expect_throws_msg(
            expected_type,
            &format!(
                "Expected exception {} but no exception was thrown",
                simple_name(expected_type)
            ),
            runnable,
        )
    }

    /// `expectThrows(expectedType, noExceptionMessage, runnable)`.
    ///
    /// # Panics
    ///
    /// `AssertionFailedError` (`noExceptionMessage`, or "Unexpected exception
    /// type, expected X but got …" caused by the throwable).
    pub fn expect_throws_msg(
        expected_type: &str,
        no_exception_message: &str,
        runnable: impl FnOnce() -> Result<(), Throwable>,
    ) -> Throwable {
        register_assertion_failed_error();
        let thrown = Self::expect_throws_internal(&[expected_type], runnable);
        match thrown {
            Some(t) if t.is_instance(expected_type) => t,
            None => exception::throw(ASSERTION_FAILED_ERROR, no_exception_message),
            Some(t) => Throwable::with_cause(
                ASSERTION_FAILED_ERROR,
                Some(format!(
                    "Unexpected exception type, expected {} but got {t}",
                    simple_name(expected_type)
                )),
                t,
            )
            .throw(),
        }
    }

    /// `expectThrowsAnyOf(expectedTypes, runnable)`.
    ///
    /// # Panics
    ///
    /// `AssertionError` ("At least one expected exception type is
    /// required?") for an empty list, and `AssertionFailedError` ("Unexpected
    /// exception type, expected any of [..] but got: …" or "Expected any of
    /// the following exception types: [..] but no exception was thrown.").
    pub fn expect_throws_any_of(
        expected_types: &[&str],
        runnable: impl FnOnce() -> Result<(), Throwable>,
    ) -> Throwable {
        register_assertion_failed_error();
        if expected_types.is_empty() {
            exception::throw(
                ASSERTION_ERROR,
                "At least one expected exception type is required?",
            )
        }
        let thrown = Self::expect_throws_internal(expected_types, runnable);
        if let Some(t) = &thrown {
            if expected_types.iter().any(|e| t.is_instance(e)) {
                return t.clone();
            }
        }
        let exception_types = simple_names(expected_types);
        match thrown {
            Some(t) => Throwable::with_cause(
                ASSERTION_FAILED_ERROR,
                Some(format!(
                    "Unexpected exception type, expected any of {exception_types} but got: {t}"
                )),
                t,
            )
            .throw(),
            None => exception::throw(
                ASSERTION_FAILED_ERROR,
                format!(
                    "Expected any of the following exception types: {exception_types} but no exception was thrown."
                ),
            ),
        }
    }

    /// `expectThrows(expectedOuterType, expectedWrappedType, runnable)`:
    /// checks that the outer and wrapped exception classes are thrown, and
    /// returns the wrapped exception.
    ///
    /// # Panics
    ///
    /// `AssertionFailedError`, as Java words it for each case.
    pub fn expect_throws_wrapped(
        expected_outer_type: &str,
        expected_wrapped_type: &str,
        runnable: impl FnOnce() -> Result<(), Throwable>,
    ) -> Throwable {
        register_assertion_failed_error();
        let Some(thrown) = Self::expect_throws_internal(&[expected_outer_type], runnable) else {
            exception::throw(
                ASSERTION_FAILED_ERROR,
                format!(
                    "Expected outer exception {} but no exception was thrown.",
                    simple_name(expected_outer_type)
                ),
            )
        };
        if thrown.is_instance(expected_outer_type) {
            let cause = thrown.cause().cloned();
            if let Some(cause) = cause.filter(|c| c.is_instance(expected_wrapped_type)) {
                return cause;
            }
            let cause = thrown
                .cause()
                .map_or_else(|| "null".to_string(), ToString::to_string);
            Throwable::with_cause(
                ASSERTION_FAILED_ERROR,
                Some(format!(
                    "Unexpected wrapped exception type, expected {} but got: {cause}",
                    simple_name(expected_wrapped_type)
                )),
                thrown,
            )
            .throw()
        }
        Throwable::with_cause(
            ASSERTION_FAILED_ERROR,
            Some(format!(
                "Unexpected outer exception type, expected {} but got: {thrown}",
                simple_name(expected_outer_type)
            )),
            thrown,
        )
        .throw()
    }

    /// `expectThrowsAnyOf(expectedOuterToWrappedTypes, runnable)`: checks
    /// that one of the outer classes, with one of its wrapped classes, is
    /// thrown, and returns the outer exception; an empty list of wrapped types
    /// accepts an outer exception without inspecting its cause and returns
    /// `None` (Java's `null`).
    ///
    /// # Panics
    ///
    /// `AssertionFailedError`, as Java words it for each case.
    pub fn expect_throws_any_of_wrapped(
        expected_outer_to_wrapped_types: &[(&str, &[&str])],
        runnable: impl FnOnce() -> Result<(), Throwable>,
    ) -> Option<Throwable> {
        register_assertion_failed_error();
        let outer_classes: Vec<&str> = expected_outer_to_wrapped_types
            .iter()
            .map(|(o, _)| *o)
            .collect();
        let Some(thrown) = Self::expect_throws_internal(&outer_classes, runnable) else {
            exception::throw(
                ASSERTION_FAILED_ERROR,
                format!(
                    "Expected any of the following outer exception types: {} but no exception was thrown.",
                    simple_names(&outer_classes)
                ),
            )
        };
        for (expected_outer_type, expected_wrapped_types) in expected_outer_to_wrapped_types {
            let cause = thrown.cause();
            if thrown.is_instance(expected_outer_type) {
                if expected_wrapped_types.is_empty() {
                    return None; // no wrapped exception
                }
                if cause.is_some_and(|c| expected_wrapped_types.iter().any(|w| c.is_instance(w))) {
                    return Some(thrown);
                }
                let cause = cause.map_or_else(|| "null".to_string(), ToString::to_string);
                Throwable::with_cause(
                    ASSERTION_FAILED_ERROR,
                    Some(format!(
                        "Unexpected wrapped exception type, expected one of {} but got: {cause}",
                        simple_names(expected_wrapped_types)
                    )),
                    thrown,
                )
                .throw()
            }
        }
        Throwable::with_cause(
            ASSERTION_FAILED_ERROR,
            Some(format!(
                "Unexpected outer exception type, expected one of {} but got: {thrown}",
                simple_names(&outer_classes)
            )),
            thrown,
        )
        .throw()
    }

    /// `assertThat(actual, matcher)`: `MatcherAssert.assertThat`, hiding the
    /// deprecated `Assert.assertThat`.
    ///
    /// # Panics
    ///
    /// `AssertionError` when `actual` does not match.
    pub fn assert_that<T: ?Sized>(actual: &T, matcher: &dyn Matcher<T>) {
        MatcherAssert::assert_that(actual, matcher);
    }

    /// `assertThat(reason, actual, matcher)`.
    ///
    /// # Panics
    ///
    /// `AssertionError` when `actual` does not match.
    pub fn assert_that_reason<T: ?Sized>(reason: &str, actual: &T, matcher: &dyn Matcher<T>) {
        MatcherAssert::assert_that_reason(reason, actual, matcher);
    }

    /// `localeForLanguageTag(languageTag)`: the locale equivalent to its
    /// programmatic name, `new Locale.Builder().setLanguageTag(languageTag).build()`.
    ///
    /// # Panics
    ///
    /// `IllformedLocaleException` for a tag that is not well-formed.
    pub fn locale_for_language_tag(language_tag: &str) -> Locale {
        Locale::builder_from_language_tag(language_tag)
    }

    /// `randomLocale(random)`: a random locale from the available locales on
    /// the system (LUCENE-4020).
    pub fn random_locale(random: &dyn Random) -> Locale {
        let tags = available_language_tags();
        Self::locale_for_language_tag(tags[random.next_int_bound(tags.len() as i32) as usize])
    }

    /// `randomTimeZone(random)`: a random time zone from the available time
    /// zones on the system, without the IDs that cause a deprecation warning
    /// in JDK 25 ("these can break certain tests that expect exact output").
    pub fn random_time_zone(random: &dyn Random) -> TimeZone {
        let mut tz_ids: Vec<&'static str> = time_zone_available_ids().to_vec();
        if RUNTIME_VERSION_FEATURE >= 25 {
            tz_ids.retain(|id| !DEPRECATED_TIME_ZONE_IDS_JDK25.contains(id));
        }
        TimeZone {
            id: RandomPicks::random_from_list(random, &tz_ids),
        }
    }

    /// `newBytesRef(String)`: a `BytesRef` holding the UTF-8 bytes of `s`, that
    /// sometimes uses a non-zero offset and non-zero end padding, to tickle
    /// latent bugs that fail to look at `BytesRef.offset`.
    pub fn new_bytes_ref_str(s: &str) -> BytesRef {
        Self::new_bytes_ref_bytes(s.as_bytes())
    }

    /// `newBytesRef(BytesRef)`: a copy of `b` with a random offset and
    /// padding.
    ///
    /// # Panics
    ///
    /// `AssertionError` when `b` is not valid.
    pub fn new_bytes_ref_from(b: &BytesRef) -> BytesRef {
        java_assert(b.is_valid().is_ok(), String::new);
        Self::new_bytes_ref_slice(&b.bytes, b.offset, b.length)
    }

    /// `newBytesRef(byte[])`.
    pub fn new_bytes_ref_bytes(b: &[u8]) -> BytesRef {
        Self::new_bytes_ref_slice(b, 0, b.len())
    }

    /// `newBytesRef()`: a random empty `BytesRef`.
    pub fn new_bytes_ref() -> BytesRef {
        Self::new_bytes_ref_slice(&[], 0, 0)
    }

    /// `newBytesRef(byteLength)`: a random empty `BytesRef` with at least
    /// `byte_length` bytes free.
    pub fn new_bytes_ref_len(byte_length: usize) -> BytesRef {
        Self::new_bytes_ref_slice(&vec![0u8; byte_length], 0, byte_length)
    }

    /// `newBytesRef(bytesIn, offset, length)`: a copy of the slice with a
    /// random start offset (half the time, in `[1, 20]`) and end padding (half
    /// the time, in `[1, 20]`), copied once more one time in 17 ("try to
    /// ferret out bugs in this method too!").
    ///
    /// # Panics
    ///
    /// `AssertionError` when the slice is out of range.
    pub fn new_bytes_ref_slice(bytes_in: &[u8], offset: usize, length: usize) -> BytesRef {
        java_assert(bytes_in.len() >= offset + length, || {
            format!(
                "got offset={offset} length={length} bytesIn.length={}",
                bytes_in.len()
            )
        });
        let random = Self::random();
        // randomly set a non-zero offset
        let start_offset = if random.next_boolean() {
            RandomNumbers::random_int_between(&*random, 1, 20) as usize
        } else {
            0
        };
        // also randomly set an end padding:
        let end_padding = if random.next_boolean() {
            RandomNumbers::random_int_between(&*random, 1, 20) as usize
        } else {
            0
        };
        let mut bytes = vec![0u8; start_offset + length + end_padding];
        bytes[start_offset..start_offset + length]
            .copy_from_slice(&bytes_in[offset..offset + length]);
        let it = BytesRef {
            bytes,
            offset: start_offset,
            length,
        };
        java_assert(it.is_valid().is_ok(), String::new);
        if RandomNumbers::random_int_between(&*random, 1, 17) == 7 {
            // try to ferret out bugs in this method too!
            return Self::new_bytes_ref_slice(&it.bytes, it.offset, it.length);
        }
        it
    }

    /// `slowFileExists(dir, fileName)`: whether `fileName` exists, found by
    /// opening it with `IOContext.READONCE` (and closing it): `false` on a
    /// `NoSuchFileException` or `FileNotFoundException`.
    ///
    /// Ported ahead of rmp #348 under the sprint's cycle rule:
    /// `MockDirectoryWrapper.openInput` (rmp #343) calls it.
    ///
    /// # Errors
    ///
    /// Any other failure opening or closing the file.
    pub fn slow_file_exists(
        dir: &dyn rucene::store::Directory,
        file_name: &str,
    ) -> rucene::error::Result<bool> {
        use rucene::error::LuceneError;
        use rucene::store::java_nio_file::{java_class_of, NO_SUCH_FILE_EXCEPTION};
        match dir.open_input(file_name, &*rucene::store::READONCE_IO_CONTEXT) {
            Ok(mut input) => {
                input.close()?;
                Ok(true)
            }
            // An `io::Error` of kind `NotFound` that carries no Java class
            // stands for one of the two (an OS error the port did not name).
            Err(LuceneError::Io(e))
                if matches!(
                    java_class_of(&e),
                    NO_SUCH_FILE_EXCEPTION | "java.io.FileNotFoundException"
                ) || (e.kind() == std::io::ErrorKind::NotFound
                    && rucene::store::java_nio_file::java_exception_of(&e).is_none()) =>
            {
                Ok(false)
            }
            // `IndexNotFoundException extends FileNotFoundException`.
            Err(LuceneError::IndexNotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// `createTempDir()`: `createTempDir("tempDir")`.
    ///
    /// # Panics
    ///
    /// As [`create_temp_dir_prefix`](Self::create_temp_dir_prefix).
    pub fn create_temp_dir() -> Path {
        Self::create_temp_dir_prefix("tempDir")
    }

    /// `createTempDir(prefix)`: a new temporary directory under the suite's
    /// temporary folder, removed after the suite (unless `LEAVE_TEMPORARY`),
    /// `tempFilesCleanupRule.createTempDir(prefix)`.
    ///
    /// # Panics
    ///
    /// When the rule is not running, as in Java.
    pub fn create_temp_dir_prefix(prefix: &str) -> Path {
        TEMP_FILES_CLEANUP_RULE.create_temp_dir(prefix)
    }

    /// `createTempFile(prefix, suffix)`:
    /// `tempFilesCleanupRule.createTempFile(prefix, suffix)`.
    ///
    /// # Errors
    ///
    /// As `TestRuleTemporaryFilesCleanup.createTempFile`.
    pub fn create_temp_file_with(prefix: &str, suffix: &str) -> io::Result<Path> {
        TEMP_FILES_CLEANUP_RULE.create_temp_file(prefix, suffix)
    }

    /// `createTempFile()`: `createTempFile("tempFile", ".tmp")`.
    ///
    /// # Errors
    ///
    /// As [`create_temp_file_with`](Self::create_temp_file_with).
    pub fn create_temp_file() -> io::Result<Path> {
        Self::create_temp_file_with("tempFile", ".tmp")
    }

    /// `getJvmForkArguments()`: the arguments to fork a JVM with the same
    /// class or module path, read from the file `tests.jvmForkArgsFile`
    /// names, one per line.
    ///
    /// # Errors
    ///
    /// The failure reading the file.
    ///
    /// # Panics
    ///
    /// `AssertionError` ("JVM fork arguments are not present.") when the
    /// property is unset or does not name a regular file.
    pub fn get_jvm_fork_arguments() -> io::Result<Vec<String>> {
        let fork_args_file = system::get_property("tests.jvmForkArgsFile");
        let fork_args_path = fork_args_file.map(|f| f.into_nio_path());
        match fork_args_path {
            Some(path) if Files::is_regular_file(&path, &[]) => Files::read_all_lines(&path),
            _ => exception::throw(ASSERTION_ERROR, "JVM fork arguments are not present."),
        }
    }
}

// ---------------------------------------------------------------------------
// The searcher factory (`newSearcher`) and what it calls (`maybeWrapReader`,
// `wrapReader`, `Concurrency`, the shared executor, the `slices` override),
// ported in full by task #345 under the sprint's cycle rule.
// ---------------------------------------------------------------------------

/// Java's `IndexReader` argument of `wrapReader`/`maybeWrapReader`/
/// `newSearcher`, typed by the run-time classes those methods test with
/// `instanceof`.
///
/// **Divergence.** Java passes an `IndexReader` and tests `instanceof
/// LeafReader`, `CompositeReader`, `DirectoryReader` and `CodecReader`. An
/// `Arc<dyn IndexReader>` cannot be narrowed to the other reader traits, so
/// the caller states the class: a leaf (a `CodecReader` is a leaf whose
/// [`LeafReader::as_codec_reader`] answers), a `DirectoryReader`, or another
/// `CompositeReader`. The conversions from each reader trait object build it.
#[derive(Clone)]
pub enum TestReader {
    /// `r instanceof LeafReader`.
    Leaf(Arc<dyn LeafReader>),
    /// `r instanceof DirectoryReader` (also a `CompositeReader`).
    Directory(Arc<dyn DirectoryReader>),
    /// `r instanceof CompositeReader`, not a `DirectoryReader`.
    Composite(Arc<dyn CompositeReader>),
}

impl std::fmt::Debug for TestReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&*self.index_reader(), f)
    }
}

impl TestReader {
    /// The reader as an `IndexReader`.
    pub fn index_reader(&self) -> Arc<dyn IndexReader> {
        match self {
            TestReader::Leaf(r) => Arc::clone(r).as_index_reader(),
            TestReader::Directory(r) => Arc::clone(r) as Arc<dyn IndexReader>,
            TestReader::Composite(r) => Arc::clone(r) as Arc<dyn IndexReader>,
        }
    }

    /// The reader as a `CompositeReader` (`(CompositeReader) r`), when it is
    /// one.
    fn composite(&self) -> Option<Arc<dyn CompositeReader>> {
        match self {
            TestReader::Leaf(_) => None,
            TestReader::Directory(r) => Some(Arc::clone(r) as Arc<dyn CompositeReader>),
            TestReader::Composite(r) => Some(Arc::clone(r)),
        }
    }
}

impl From<Arc<dyn LeafReader>> for TestReader {
    fn from(r: Arc<dyn LeafReader>) -> Self {
        TestReader::Leaf(r)
    }
}

impl From<Arc<dyn DirectoryReader>> for TestReader {
    fn from(r: Arc<dyn DirectoryReader>) -> Self {
        TestReader::Directory(r)
    }
}

impl From<Arc<dyn CompositeReader>> for TestReader {
    fn from(r: Arc<dyn CompositeReader>) -> Self {
        TestReader::Composite(r)
    }
}

impl From<Arc<MultiReader>> for TestReader {
    fn from(r: Arc<MultiReader>) -> Self {
        TestReader::Composite(r)
    }
}

/// `LuceneTestCase.Concurrency`: what level of concurrency is supported by the
/// searcher being created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Concurrency {
    /// No concurrency, meaning an executor won't be provided to the searcher.
    None,
    /// Inter-segment concurrency, meaning an executor will be provided to the
    /// searcher and slices will be randomly created to concurrently search
    /// entire segments.
    InterSegment,
    /// Intra-segment concurrency, meaning an executor will be provided to the
    /// searcher and slices will be randomly created to concurrently search
    /// segment partitions.
    IntraSegment,
}

/// The `ThreadPoolExecutor` of `setUpExecutorService`: `threads` core and
/// maximum threads, no keep-alive, an unbounded `LinkedBlockingQueue` and a
/// `NamedThreadFactory("LuceneTestCase")`.
///
/// As `ThreadPoolExecutor.execute`, a submitted task starts a new core thread
/// while fewer than `threads` are running, and is queued otherwise. The
/// threads are randomizedtesting [`Thread`](randomizedtesting::jdk::thread::Thread)s,
/// started from the creating thread's group, so that tasks run inside the
/// test's randomized context as Java's do.
///
/// **Divergence.** A task that panics does not end its worker: Java's pool
/// lets the worker die, reports the exception to the thread's uncaught
/// exception handler and replaces the worker. The tasks submitted by the
/// port's `TaskExecutor` (wrapped as Java's `FutureTask`s) catch their own
/// failures, so the difference is never reached through `newSearcher`.
pub struct LuceneTestCaseExecutor {
    shared: Arc<ExecutorShared>,
    threads: usize,
    thread_factory: rucene::util::NamedThreadFactory,
}

struct ExecutorShared {
    state: std::sync::Mutex<ExecutorState>,
    /// Signalled when a task is queued or the pool shuts down.
    work_available: std::sync::Condvar,
    /// Signalled when a worker exits.
    terminated: std::sync::Condvar,
}

struct ExecutorState {
    queue: std::collections::VecDeque<rucene::search::Command>,
    shutdown: bool,
    started: usize,
    live: usize,
}

impl std::fmt::Debug for LuceneTestCaseExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LuceneTestCaseExecutor")
            .field("threads", &self.threads)
            .finish_non_exhaustive()
    }
}

impl LuceneTestCaseExecutor {
    /// `new ThreadPoolExecutor(threads, threads, 0L, MILLISECONDS, new
    /// LinkedBlockingQueue<>(), new NamedThreadFactory("LuceneTestCase"))`.
    pub fn new(threads: usize) -> LuceneTestCaseExecutor {
        LuceneTestCaseExecutor {
            shared: Arc::new(ExecutorShared {
                state: std::sync::Mutex::new(ExecutorState {
                    queue: std::collections::VecDeque::new(),
                    shutdown: false,
                    started: 0,
                    live: 0,
                }),
                work_available: std::sync::Condvar::new(),
                terminated: std::sync::Condvar::new(),
            }),
            threads,
            thread_factory: rucene::util::NamedThreadFactory::new("LuceneTestCase"),
        }
    }

    /// The number of core (and maximum) threads.
    pub fn threads(&self) -> usize {
        self.threads
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ExecutorState> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The worker loop: the first task, then queued tasks until the pool is
    /// shut down and the queue is empty.
    fn run_worker(shared: Arc<ExecutorShared>, first: rucene::search::Command) {
        let mut task = Some(first);
        loop {
            if let Some(command) = task.take() {
                // A panicking task does not end the worker: ThreadPoolExecutor
                // replaces a worker whose task throws, which is equivalent here.
                let _ = catch_unwind(AssertUnwindSafe(command));
            }
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            loop {
                if let Some(next) = state.queue.pop_front() {
                    task = Some(next);
                    break;
                }
                if state.shutdown {
                    state.live -= 1;
                    shared.terminated.notify_all();
                    return;
                }
                state = shared
                    .work_available
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        }
    }
}

impl rucene::search::Executor for LuceneTestCaseExecutor {
    fn execute(
        &self,
        command: rucene::search::Command,
    ) -> std::result::Result<(), rucene::search::RejectedExecutionException> {
        let mut state = self.lock();
        if state.shutdown {
            return Err(rucene::search::RejectedExecutionException {
                command,
                // `"Task " + r + " rejected from " + executor`: neither has a
                // string form here.
                message: "Task <task> rejected from <LuceneTestCase executor>".to_string(),
            });
        }
        if state.started < self.threads {
            state.started += 1;
            state.live += 1;
            drop(state);
            let shared = Arc::clone(&self.shared);
            let name = self.thread_factory.next_thread_name();
            randomizedtesting::jdk::thread::Thread::new(name, move || {
                Self::run_worker(shared, command)
            })
            .start();
        } else {
            state.queue.push_back(command);
            drop(state);
            self.shared.work_available.notify_one();
        }
        Ok(())
    }
}

impl crate::jdk::executor_service::ExecutorService for LuceneTestCaseExecutor {
    fn shutdown(&self) {
        self.lock().shutdown = true;
        self.shared.work_available.notify_all();
    }

    fn await_termination(
        &self,
        timeout: std::time::Duration,
    ) -> std::result::Result<bool, rucene::util::InterruptedException> {
        let deadline = std::time::Instant::now() + timeout;
        let mut state = self.lock();
        while state.live != 0 {
            let now = std::time::Instant::now();
            if now >= deadline {
                return Ok(false);
            }
            state = self
                .shared
                .terminated
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        Ok(true)
    }
}

/// `LuceneTestCase.executor`, set by `setUpExecutorService` and cleared by
/// `shutdownExecutorService`.
static EXECUTOR: std::sync::Mutex<Option<Arc<LuceneTestCaseExecutor>>> =
    std::sync::Mutex::new(None);

/// `classEnvRule.similarity`: the similarity `TestRuleSetupAndRestoreClassEnv`
/// draws for the class (`new AssertingSimilarity(new
/// RandomSimilarity(random()))`), which `newSearcher` installs.
///
/// **Divergence (pending task #348).** The rule's `before()`, which draws it
/// after the codec, locale and time zone of the class, is ported by task
/// #348; until then the class sets it with
/// [`LuceneTestCase::set_class_env_similarity`].
static CLASS_ENV_SIMILARITY: std::sync::Mutex<Option<Arc<dyn Similarity>>> =
    std::sync::Mutex::new(None);

/// The anonymous subclasses of `IndexSearcher` and `AssertingIndexSearcher`
/// that `newSearcher` builds with an executor: `slices(leaves)` returns
/// `LuceneTestCase.slices(leaves, maxDocPerSlice, maxSegmentsPerSlice,
/// concurrency)`; every other method is the superclass's.
#[derive(Debug)]
pub struct NewSearcherSlices {
    /// The superclass's overrides: `AssertingIndexSearcher`'s, or none for an
    /// `IndexSearcher`.
    parent: Option<Arc<dyn IndexSearcherOverrides>>,
    max_doc_per_slice: i32,
    max_segments_per_slice: i32,
    concurrency: Concurrency,
}

impl IndexSearcherOverrides for NewSearcherSlices {
    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        query: Arc<dyn Query>,
        score_mode: ScoreMode,
        boost: f32,
    ) -> rucene::error::Result<Arc<dyn Weight>> {
        match &self.parent {
            Some(parent) => parent.create_weight(searcher, query, score_mode, boost),
            None => searcher.super_create_weight(query, score_mode, boost),
        }
    }

    fn rewrite(
        &self,
        searcher: &IndexSearcher,
        original: Arc<dyn Query>,
    ) -> rucene::error::Result<Arc<dyn Query>> {
        match &self.parent {
            Some(parent) => parent.rewrite(searcher, original),
            None => searcher.super_rewrite(original),
        }
    }

    fn search_partitions(
        &self,
        searcher: &IndexSearcher,
        partitions: &[LeafReaderContextPartition],
        weight: &Arc<dyn Weight>,
        collector: &mut dyn Collector,
    ) -> rucene::error::Result<()> {
        match &self.parent {
            Some(parent) => parent.search_partitions(searcher, partitions, weight, collector),
            None => searcher.super_search_partitions(partitions, weight, collector),
        }
    }

    fn search_leaf(
        &self,
        searcher: &IndexSearcher,
        ctx: &Arc<LeafReaderContext>,
        min_doc_id: i32,
        max_doc_id: i32,
        weight: &Arc<dyn Weight>,
        collector: &mut dyn Collector,
    ) -> rucene::error::Result<()> {
        match &self.parent {
            Some(parent) => {
                parent.search_leaf(searcher, ctx, min_doc_id, max_doc_id, weight, collector)
            }
            None => searcher.super_search_leaf(ctx, min_doc_id, max_doc_id, weight, collector),
        }
    }

    fn slices(
        &self,
        _searcher: &IndexSearcher,
        leaves: &[Arc<LeafReaderContext>],
    ) -> rucene::error::Result<Vec<LeafSlice>> {
        LuceneTestCase::slices(
            leaves,
            self.max_doc_per_slice,
            self.max_segments_per_slice,
            self.concurrency,
        )
    }

    fn fmt(&self, searcher: &IndexSearcher, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.parent {
            Some(parent) => IndexSearcherOverrides::fmt(&**parent, searcher, f),
            None => searcher.super_fmt(f),
        }
    }
}

impl NewSearcherSlices {
    /// The `concurrency` argument of `newSearcher`.
    pub fn concurrency(&self) -> Concurrency {
        self.concurrency
    }

    /// `maxDocPerSlice`.
    pub fn max_doc_per_slice(&self) -> i32 {
        self.max_doc_per_slice
    }

    /// `maxSegmentsPerSlice`.
    pub fn max_segments_per_slice(&self) -> i32 {
        self.max_segments_per_slice
    }
}

/// The `QueryTimeout` `() -> false` that `newSearcher` sometimes installs.
#[derive(Debug)]
pub struct NeverTimeout;

impl QueryTimeout for NeverTimeout {
    fn should_exit(&self) -> bool {
        false
    }
}

impl LuceneTestCase {
    /// `setUpExecutorService()` (`@BeforeClass`): creates the shared executor
    /// with `TestUtil.nextInt(random(), 1, 2)` threads.
    pub fn set_up_executor_service() {
        let threads = TestUtil::next_int(&*Self::random(), 1, 2);
        let executor = Arc::new(LuceneTestCaseExecutor::new(threads as usize));
        if *VERBOSE {
            println!("NOTE: Created shared ExecutorService with {threads} threads");
        }
        *EXECUTOR
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(executor);
    }

    /// `shutdownExecutorService()` (`@AfterClass`).
    pub fn shutdown_executor_service() {
        let executor = EXECUTOR
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        TestUtil::shutdown_executor_service(
            executor
                .as_deref()
                .map(|e| e as &dyn crate::jdk::executor_service::ExecutorService),
        );
    }

    /// The shared executor, when `setUpExecutorService` created it.
    pub fn executor() -> Option<Arc<LuceneTestCaseExecutor>> {
        EXECUTOR
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// `classEnvRule.similarity`: the similarity `TestRuleSetupAndRestoreClassEnv`
    /// draws for the class, which `newSearcher` installs.
    ///
    /// **Divergence (pending task #348).** The rule's `before()` is ported by
    /// task #348; until then the class sets it with
    /// [`set_class_env_similarity`](Self::set_class_env_similarity).
    ///
    /// # Panics
    ///
    /// When the class environment has not set it.
    pub fn class_env_similarity() -> Arc<dyn Similarity> {
        CLASS_ENV_SIMILARITY
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .expect(
                "classEnvRule.similarity is set by TestRuleSetupAndRestoreClassEnv.before() \
                 (task #348); set it with LuceneTestCase::set_class_env_similarity",
            )
    }

    /// Sets `classEnvRule.similarity` (see
    /// [`class_env_similarity`](Self::class_env_similarity)).
    pub fn set_class_env_similarity(similarity: Option<Arc<dyn Similarity>>) {
        *CLASS_ENV_SIMILARITY
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = similarity;
    }

    /// `wrapReader(r)`: wraps the reader one to six times, each time with a
    /// random choice of `ParallelLeafReader`/`ParallelCompositeReader`, a
    /// `ParallelLeafReader` over two complementary `FieldFilterLeafReader`s,
    /// `AssertingLeafReader`/`AssertingDirectoryReader`,
    /// `MismatchedLeafReader`/`MismatchedDirectoryReader`, or
    /// `MergingCodecReader`/`MergingDirectoryReaderWrapper`.
    ///
    /// # Errors
    ///
    /// Propagates the wrappers' construction errors.
    pub fn wrap_reader(r: TestReader) -> rucene::error::Result<TestReader> {
        let random = Self::random();
        let mut r = r;

        let c = random.next_int_bound(6) + 1;
        for _ in 0..c {
            match random.next_int_bound(5) {
                0 => {
                    // will create no FC insanity in atomic case, as ParallelLeafReader has own
                    // cache key:
                    if *VERBOSE {
                        println!(
                            "NOTE: LuceneTestCase.wrapReader: wrapping previous reader={r:?} \
                             with ParallelLeaf/CompositeReader"
                        );
                    }
                    r =
                        match r.composite() {
                            None => {
                                let TestReader::Leaf(leaf) = r else {
                                    unreachable!("a reader that is not composite is a leaf")
                                };
                                TestReader::Leaf(Arc::new(ParallelLeafReader::new(vec![leaf])?))
                            }
                            Some(composite) => TestReader::Composite(Arc::new(
                                ParallelCompositeReader::new(vec![composite])?,
                            )),
                        };
                }
                1 => {
                    if let TestReader::Leaf(ar) = &r {
                        let mut all_fields: Vec<String> = ar
                            .get_field_infos()
                            .iter()
                            .map(|fi| fi.name.clone())
                            .collect();
                        randomizedtesting::jdk::util::shuffle(&mut all_fields, &*random);
                        let end = if all_fields.is_empty() {
                            0
                        } else {
                            random.next_int_bound(all_fields.len() as i32) as usize
                        };
                        let fields: HashSet<String> = all_fields[..end].iter().cloned().collect();
                        // will create no FC insanity as ParallelLeafReader has own cache key:
                        if *VERBOSE {
                            println!(
                                "NOTE: LuceneTestCase.wrapReader: wrapping previous reader={r:?} \
                                 with ParallelLeafReader"
                            );
                        }
                        let kept: Arc<dyn LeafReader> = Arc::new(FieldFilterLeafReader::new(
                            Arc::clone(ar),
                            fields.iter().cloned(),
                            false,
                        )?);
                        let negated: Arc<dyn LeafReader> = Arc::new(FieldFilterLeafReader::new(
                            Arc::clone(ar),
                            fields.iter().cloned(),
                            true,
                        )?);
                        r = TestReader::Leaf(Arc::new(ParallelLeafReader::new(vec![
                            kept, negated,
                        ])?));
                    }
                }
                2 => {
                    // Häckidy-Hick-Hack: a standard Reader will cause FC insanity, so we use
                    // QueryUtils' reader with a fake cache key, so insanity checker cannot walk
                    // along our reader:
                    if *VERBOSE {
                        println!(
                            "NOTE: LuceneTestCase.wrapReader: wrapping previous reader={r:?} \
                             with AssertingLeaf/DirectoryReader"
                        );
                    }
                    match &r {
                        TestReader::Leaf(leaf) => {
                            r = TestReader::Leaf(Arc::new(AssertingLeafReader::new(Arc::clone(
                                leaf,
                            ))?));
                        }
                        TestReader::Directory(dr) => {
                            r = TestReader::Directory(Arc::new(AssertingDirectoryReader::new(
                                Arc::clone(dr),
                            )?));
                        }
                        TestReader::Composite(_) => {}
                    }
                }
                3 => {
                    if *VERBOSE {
                        println!(
                            "NOTE: LuceneTestCase.wrapReader: wrapping previous reader={r:?} \
                             with MismatchedLeaf/Directory/CodecReader"
                        );
                    }
                    // `r instanceof CodecReader` is never reached: a CodecReader is a
                    // LeafReader, which the first branch takes.
                    match &r {
                        TestReader::Leaf(leaf) => {
                            r = TestReader::Leaf(Arc::new(MismatchedLeafReader::new(
                                Arc::clone(leaf),
                                &*random,
                            )?));
                        }
                        TestReader::Directory(dr) => {
                            r = TestReader::Directory(Arc::new(MismatchedDirectoryReader::new(
                                Arc::clone(dr),
                                Arc::clone(&random),
                            )?));
                        }
                        TestReader::Composite(_) => {}
                    }
                }
                4 => {
                    if *VERBOSE {
                        println!(
                            "NOTE: LuceneTestCase.wrapReader: wrapping previous reader={r:?} \
                             with MergingCodecReader"
                        );
                    }
                    match &r {
                        TestReader::Leaf(leaf) => {
                            if let Some(codec) = Arc::clone(leaf).as_codec_reader() {
                                r = TestReader::Leaf(Arc::new(MergingCodecReader::new(codec)));
                            }
                        }
                        TestReader::Directory(dr) => {
                            let mut all_leaves_are_codec_readers = true;
                            for ctx in r.index_reader().leaves()? {
                                if ctx.leaf_reader().as_codec_reader().is_none() {
                                    all_leaves_are_codec_readers = false;
                                    break;
                                }
                            }
                            if all_leaves_are_codec_readers {
                                r = TestReader::Directory(Arc::new(
                                    MergingDirectoryReaderWrapper::new(Arc::clone(dr))?,
                                ));
                            }
                        }
                        TestReader::Composite(_) => {}
                    }
                }
                _ => Assert::fail("should not get here"),
            }
        }

        if *VERBOSE {
            println!("wrapReader wrapped: {r:?}");
        }

        Ok(r)
    }

    /// `maybeWrapReader(r)`: sometimes (`rarely()`) wraps the reader with
    /// [`wrap_reader`](Self::wrap_reader).
    ///
    /// # Errors
    ///
    /// As [`wrap_reader`](Self::wrap_reader).
    pub fn maybe_wrap_reader(r: TestReader) -> rucene::error::Result<TestReader> {
        if Self::rarely() {
            return Self::wrap_reader(r);
        }
        Ok(r)
    }

    /// `newSearcher(r)`: `newSearcher(r, true)`.
    pub fn new_searcher(r: impl Into<TestReader>) -> IndexSearcher {
        Self::new_searcher_wrap(r, true)
    }

    /// `newSearcher(r, maybeWrap)`: `newSearcher(r, maybeWrap, true)`.
    pub fn new_searcher_wrap(r: impl Into<TestReader>, maybe_wrap: bool) -> IndexSearcher {
        Self::new_searcher_assert(r, maybe_wrap, true)
    }

    /// `newSearcher(r, maybeWrap, wrapWithAssertions)`: with threads when
    /// `randomBoolean()`.
    pub fn new_searcher_assert(
        r: impl Into<TestReader>,
        maybe_wrap: bool,
        wrap_with_assertions: bool,
    ) -> IndexSearcher {
        Self::new_searcher_threads(
            r,
            maybe_wrap,
            wrap_with_assertions,
            RandomizedTest::random_boolean(),
        )
    }

    /// `newSearcher(r, maybeWrap, wrapWithAssertions, useThreads)`:
    /// `Concurrency.INTRA_SEGMENT` with threads, `Concurrency.NONE` without.
    pub fn new_searcher_threads(
        r: impl Into<TestReader>,
        maybe_wrap: bool,
        wrap_with_assertions: bool,
        use_threads: bool,
    ) -> IndexSearcher {
        let concurrency = if use_threads {
            Concurrency::IntraSegment
        } else {
            Concurrency::None
        };
        Self::new_searcher_concurrency(r, maybe_wrap, wrap_with_assertions, concurrency)
    }

    /// `newSearcher(r, maybeWrap, wrapWithAssertions, concurrency)`: a
    /// searcher, an `AssertingIndexSearcher` when `wrapWithAssertions` and a
    /// coin says so, over the reader (maybe wrapped, without concurrency),
    /// with the class similarity; with concurrency, maybe the shared
    /// executor, random slices, `MAYBE_CACHE_POLICY` and maybe a timeout that
    /// never expires.
    ///
    /// # Panics
    ///
    /// As Java (`Rethrow.rethrow`), the I/O errors of wrapping, checking or
    /// opening the reader.
    pub fn new_searcher_concurrency(
        r: impl Into<TestReader>,
        maybe_wrap: bool,
        wrap_with_assertions: bool,
        concurrency: Concurrency,
    ) -> IndexSearcher {
        let random = Self::random();
        let mut r: TestReader = r.into();
        let rethrow = |e: rucene::error::LuceneError| -> ! {
            panic!("{e}");
        };
        if concurrency == Concurrency::None {
            if maybe_wrap {
                r = Self::maybe_wrap_reader(r).unwrap_or_else(|e| rethrow(e));
            }
            // TODO: this whole check is a coverage hack, we should move it to tests for various
            // filterreaders.
            // ultimately whatever you do will be checkIndex'd at the end anyway.
            if random.next_int_bound(500) == 0 && matches!(r, TestReader::Leaf(_)) {
                // TODO: not useful to check DirectoryReader (redundant with checkindex)
                // but maybe sometimes run this on the other crazy readers maybeWrapReader creates?
                TestUtil::check_reader(&r.index_reader()).unwrap_or_else(|e| rethrow(e));
            }
            let reader = r.index_reader();
            let mut ret = if wrap_with_assertions {
                if random.next_boolean() {
                    AssertingIndexSearcher::new(&*random, reader)
                } else {
                    Arc::clone(&reader)
                        .get_context()
                        .and_then(|context| AssertingIndexSearcher::from_context(&*random, context))
                }
            } else if random.next_boolean() {
                IndexSearcher::new(reader)
            } else {
                Arc::clone(&reader)
                    .get_context()
                    .and_then(|context| IndexSearcher::from_context(context, None))
            }
            .unwrap_or_else(|e| rethrow(e));
            ret.set_similarity(Self::class_env_similarity());
            ret
        } else {
            let ex: Option<Arc<dyn rucene::search::Executor>> = if random.next_boolean() {
                None
            } else {
                if *VERBOSE {
                    println!("NOTE: newSearcher using shared ExecutorService");
                }
                Self::executor().map(|e| e as Arc<dyn rucene::search::Executor>)
            };
            let max_doc_per_slice = if random.next_boolean() {
                1
            } else {
                1 + random.next_int_bound(1000)
            };
            let max_segments_per_slice = if random.next_boolean() {
                1
            } else {
                1 + random.next_int_bound(10)
            };
            let reader = r.index_reader();
            let mut ret = if wrap_with_assertions {
                if random.next_boolean() {
                    AssertingIndexSearcher::with_executor(&*random, reader, ex)
                } else {
                    Arc::clone(&reader).get_context().and_then(|context| {
                        AssertingIndexSearcher::from_context_with_executor(&*random, context, ex)
                    })
                }
            } else {
                IndexSearcher::with_executor(reader, ex)
            }
            .unwrap_or_else(|e| rethrow(e));
            let parent = ret.get_overrides().cloned();
            ret.set_overrides(Arc::new(NewSearcherSlices {
                parent,
                max_doc_per_slice,
                max_segments_per_slice,
                concurrency,
            }));
            ret.set_similarity(Self::class_env_similarity());
            ret.set_query_caching_policy(Arc::clone(&MAYBE_CACHE_POLICY));
            if Self::random().next_boolean() {
                ret.set_timeout(Some(Arc::new(NeverTimeout)));
            }
            ret
        }
    }

    /// The private `slices(leaves, maxDocsPerSlice, maxSegmentsPerSlice,
    /// concurrency)`: leaf slices according to the concurrency argument, that
    /// optionally leverage intra-segment concurrency by splitting segments
    /// into multiple partitions according to `maxDocsPerSlice`.
    ///
    /// # Errors
    ///
    /// As [`IndexSearcher::compute_slices`].
    fn slices(
        leaves: &[Arc<LeafReaderContext>],
        max_docs_per_slice: i32,
        max_segments_per_slice: i32,
        concurrency: Concurrency,
    ) -> rucene::error::Result<Vec<LeafSlice>> {
        exception::java_assert_no_message(concurrency != Concurrency::None);
        // Rarely test slices without partitions even though intra-segment concurrency is supported
        IndexSearcher::compute_slices(
            leaves,
            max_docs_per_slice,
            max_segments_per_slice as usize,
            concurrency == Concurrency::IntraSegment && RandomizedTest::frequently(),
        )
    }
}

#[cfg(test)]
mod executor_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use rucene::search::Executor;

    use super::LuceneTestCaseExecutor;
    use crate::jdk::executor_service::ExecutorService;

    /// The pool runs every submitted task on at most `threads` workers,
    /// terminates after `shutdown`, and rejects (handing the command back)
    /// once shut down.
    #[test]
    fn runs_tasks_then_terminates_and_rejects() {
        let executor = LuceneTestCaseExecutor::new(2);
        let done = Arc::new(AtomicUsize::new(0));
        for _ in 0..50 {
            let done = Arc::clone(&done);
            executor
                .execute(Box::new(move || {
                    done.fetch_add(1, Ordering::SeqCst);
                }))
                .unwrap();
        }
        assert!(executor.lock().started <= 2);
        ExecutorService::shutdown(&executor);
        assert!(executor.await_termination(Duration::from_secs(30)).unwrap());
        assert_eq!(done.load(Ordering::SeqCst), 50);
        let rejected = executor.execute(Box::new(|| {})).unwrap_err();
        assert!(rejected.message.starts_with("Task "));
        (rejected.command)();
    }
}
