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
//! with what it calls (task #345, under the sprint's cycle rule). The class
//! declaration with its class and test rule chains (task #348) is in
//! [`crate::util::lucene_test_case_rules`]; task #349 ports the remaining
//! members (`collate` over [`crate::jdk::collator`]).

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

/// The annotation type of [`Nightly`] (`LuceneTestCase.Nightly.class`): tests
/// that should only be run during nightly builds (`@Documented @Inherited
/// @Retention(RUNTIME) @TestGroup(enabled = false, sysProperty =
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

/// An `@LuceneTestCase.Nightly` annotation value (the annotation has no
/// members), for the tests and suites Lucene marks with it.
///
/// Equivalent to the annotation `LuceneTestCase.Nightly`: "Annotation for
/// tests that should only be run during nightly builds." Its type, with the
/// `@TestGroup` and `@Inherited` meta-annotations, is [`NIGHTLY`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Nightly;

impl JavaAnnotation for Nightly {
    fn annotation_type(&self) -> &'static AnnotationType {
        &NIGHTLY
    }
    fn to_java_string(&self) -> String {
        "@org.apache.lucene.tests.util.LuceneTestCase.Nightly()".to_string()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

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

/// An `@LuceneTestCase.Monster(value)` annotation value, for the tests
/// Lucene marks with it.
#[derive(Debug, Clone, Default)]
pub struct Monster {
    /// `value()`: the reason the test is a monster.
    pub value: String,
}

impl JavaAnnotation for Monster {
    fn annotation_type(&self) -> &'static AnnotationType {
        &MONSTER
    }
    fn to_java_string(&self) -> String {
        format!(
            "@org.apache.lucene.tests.util.LuceneTestCase.Monster({})",
            randomizedtesting::jdk::reflect::quote_member(&self.value)
        )
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

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

/// An `@AwaitsFix(bugUrl)` annotation value.
#[derive(Debug, Clone, Default)]
pub struct AwaitsFix {
    /// `bugUrl()`: point to JIRA entry.
    pub bug_url: String,
}

impl JavaAnnotation for AwaitsFix {
    fn annotation_type(&self) -> &'static AnnotationType {
        &AWAITS_FIX
    }
    fn to_java_string(&self) -> String {
        format!(
            "@org.apache.lucene.tests.util.LuceneTestCase.AwaitsFix(bugUrl={})",
            quote_member(&self.bug_url)
        )
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

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

/// `@SuppressCodecs` (`LuceneTestCase.SuppressCodecs`, `@Inherited`):
/// "annotation for test classes that should avoid certain codec types
/// (because they are expensive, for example)."
pub static SUPPRESS_CODECS: AnnotationType = AnnotationType::plain(
    "org.apache.lucene.tests.util.LuceneTestCase$SuppressCodecs",
    "SuppressCodecs",
    true,
);

/// An `@SuppressCodecs(value)` annotation value.
#[derive(Debug, Clone, Default)]
pub struct SuppressCodecs {
    /// `value()`.
    pub value: Vec<String>,
}

impl JavaAnnotation for SuppressCodecs {
    fn annotation_type(&self) -> &'static AnnotationType {
        &SUPPRESS_CODECS
    }
    fn to_java_string(&self) -> String {
        let items: Vec<String> = self.value.iter().map(|v| quote_member(v)).collect();
        format!(
            "@org.apache.lucene.tests.util.LuceneTestCase.SuppressCodecs({{{}}})",
            items.join(", ")
        )
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// `LuceneTestCase.SuppressSysoutChecks.class.getName()`.
pub const SUPPRESS_SYSOUT_CHECKS_NAME: &str =
    "org.apache.lucene.tests.util.LuceneTestCase$SuppressSysoutChecks";

/// `@SuppressSysoutChecks` (`LuceneTestCase.SuppressSysoutChecks`,
/// `@Inherited`): "ignore `TestRuleLimitSysouts` for any suite which is known
/// to print over the default limit of bytes to `System.out` or `System.err`."
pub static SUPPRESS_SYSOUT_CHECKS: AnnotationType =
    AnnotationType::plain(SUPPRESS_SYSOUT_CHECKS_NAME, "SuppressSysoutChecks", true);

/// An `@SuppressSysoutChecks(bugUrl)` annotation value.
///
/// Equivalent to the annotation `LuceneTestCase.SuppressSysoutChecks`
/// (`@Documented @Inherited @Retention(RUNTIME) @Target(TYPE)`). Its type is
/// [`SUPPRESS_SYSOUT_CHECKS`]; `@Target(TYPE)` has no Rust counterpart, the
/// one reader (`TestRuleLimitSysouts`) looks it up on the suite class only, as
/// Java does.
#[derive(Debug, Clone, Default)]
pub struct SuppressSysoutChecks {
    /// `bugUrl()`: "point to JIRA entry."
    pub bug_url: String,
}

impl JavaAnnotation for SuppressSysoutChecks {
    fn annotation_type(&self) -> &'static AnnotationType {
        &SUPPRESS_SYSOUT_CHECKS
    }
    fn to_java_string(&self) -> String {
        format!(
            "@org.apache.lucene.tests.util.LuceneTestCase.SuppressSysoutChecks(bugUrl={})",
            quote_member(&self.bug_url)
        )
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// `@SuppressReproduceLine` (`LuceneTestCase.SuppressReproduceLine`,
/// `@Inherited`): "suppress the default `reproduce with: ant test...`."
pub static SUPPRESS_REPRODUCE_LINE: AnnotationType = AnnotationType::plain(
    "org.apache.lucene.tests.util.LuceneTestCase$SuppressReproduceLine",
    "SuppressReproduceLine",
    true,
);

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
        enable_error_backtraces();
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

/// Turns on the stack capture of every `LuceneError`, so that
/// `Throwable.getStackTrace()` has frames as it has in Java: sets
/// `RUST_LIB_BACKTRACE=1` unless the variable is already set. `Backtrace`
/// reads the variable once, at the first capture, so this runs as early as
/// the test framework can: the static initializer of `LuceneTestCase`.
pub fn enable_error_backtraces() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if std::env::var_os("RUST_LIB_BACKTRACE").is_none() {
            std::env::set_var("RUST_LIB_BACKTRACE", "1");
        }
    });
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

/// A `java.util.TimeZone` as `TimeZone.getTimeZone(id)` returns it: a zone of
/// JDK 25's available IDs, or a custom `GMT±hh:mm` zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimeZone {
    /// `getID()`.
    id: &'static str,
    /// Where the zone's rules come from.
    rules: ZoneRules,
}

/// The rules of a [`TimeZone`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ZoneRules {
    /// The `ZoneInfo` of an available ID (`GMT`'s for `GMT+00:00` and
    /// `GMT-00:00`, which Java builds by renaming a copy of it).
    Available(&'static str),
    /// `ZoneInfoFile.getCustomTimeZone(id, gmtOffset)`: a fixed offset, in
    /// milliseconds.
    Custom(i32),
}

/// The length of `"GMT"`, `TimeZone.GMT_ID_LENGTH`.
const GMT_ID_LENGTH: usize = 3;

/// Returns a `&'static str` equal to `id`, one allocation per distinct
/// custom zone ID for the life of the process (a [`TimeZone`] is `Copy`).
fn intern_custom_id(id: String) -> &'static str {
    static IDS: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
    let mut ids = IDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(interned) = ids.iter().find(|i| **i == id) {
        return interned;
    }
    let interned: &'static str = Box::leak(id.into_boxed_str());
    ids.push(interned);
    interned
}

impl TimeZone {
    /// `TimeZone.getTimeZone(id)`: "the specified TimeZone, or the GMT zone
    /// if the given ID cannot be understood" — the zone of an available ID,
    /// else the custom zone `parseCustomTimeZone(id)` parses, else `GMT`.
    pub fn get_time_zone(id: &str) -> TimeZone {
        if let Some(available) = time_zone_available_ids().iter().find(|z| **z == id) {
            return TimeZone {
                id: available,
                rules: ZoneRules::Available(available),
            };
        }
        Self::parse_custom_time_zone(id).unwrap_or(TimeZone {
            id: "GMT",
            rules: ZoneRules::Available("GMT"),
        })
    }

    /// `TimeZone.parseCustomTimeZone(id)` (private): a custom ID of the form
    /// `GMT` followed by a sign and either `h`, `hh`, `hmm`, `hhmm` or
    /// `h[h]:mm`, with at most 23 hours and 59 minutes, as a zone of that
    /// offset whose ID is the normalized `GMT±hh:mm`
    /// (`ZoneInfoFile.toCustomID`); a zero offset is `GMT`'s rules under the
    /// ID `GMT+00:00` or `GMT-00:00`. `None` when `id` is not such an ID.
    ///
    /// The JDK 25 source is not available here; the grammar was determined
    /// empirically against the JDK 25 oracle, whose `ltc_members.tsv` grid
    /// pins it: a `:` needs one or two digits before it, and a body without
    /// one has at most four digits.
    fn parse_custom_time_zone(id: &str) -> Option<TimeZone> {
        let chars: Vec<u16> = id.encode_utf16().collect();
        let length = chars.len();
        // Error if the length of id isn't long enough or id doesn't
        // start with "GMT".
        if length < GMT_ID_LENGTH + 2 || !id.starts_with("GMT") {
            return None;
        }
        // `ZoneInfoFile.getZoneInfo(id)`: an available ID was handled by the
        // caller.
        let mut index = GMT_ID_LENGTH;
        let mut c = chars[index];
        index += 1;
        let negative = if c == u16::from(b'-') {
            true
        } else if c == u16::from(b'+') {
            false
        } else {
            return None;
        };

        let mut hours: i32 = 0;
        let mut num: i32 = 0;
        let mut count_delim = 0;
        let mut len = 0;
        while index < length {
            c = chars[index];
            index += 1;
            if c == u16::from(b':') {
                if count_delim > 0 {
                    return None;
                }
                if len == 0 || len > 2 {
                    return None;
                }
                hours = num;
                count_delim += 1;
                num = 0;
                len = 0;
                continue;
            }
            if !(u16::from(b'0')..=u16::from(b'9')).contains(&c) {
                return None;
            }
            num = num
                .wrapping_mul(10)
                .wrapping_add(i32::from(c - u16::from(b'0')));
            len += 1;
        }
        if count_delim == 0 {
            if len <= 2 {
                hours = num;
                num = 0;
            } else if len <= 4 {
                hours = num / 100;
                num %= 100;
            } else {
                return None;
            }
        } else if len != 2 {
            return None;
        }
        if hours > 23 || num > 59 {
            return None;
        }
        let gmt_offset = hours
            .wrapping_mul(60)
            .wrapping_add(num)
            .wrapping_mul(60)
            .wrapping_mul(1000);
        if gmt_offset == 0 {
            let id = if negative { "GMT-00:00" } else { "GMT+00:00" };
            return Some(TimeZone {
                id,
                rules: ZoneRules::Available("GMT"),
            });
        }
        let offset = if negative {
            gmt_offset.wrapping_neg()
        } else {
            gmt_offset
        };
        let custom_id = Self::to_custom_id(offset);
        Some(TimeZone {
            id: intern_custom_id(custom_id),
            rules: ZoneRules::Custom(offset),
        })
    }

    /// `ZoneInfoFile.toCustomID(gmtOffset)`: `GMT`, the sign, two hour
    /// digits, `:` and two minute digits, each digit built by adding to `'0'`
    /// as Java's `char` arithmetic does.
    fn to_custom_id(gmt_offset: i32) -> String {
        let mut offset = gmt_offset / 60_000;
        let sign = if offset >= 0 {
            u16::from(b'+')
        } else {
            offset = offset.wrapping_neg();
            u16::from(b'-')
        };
        let hh = offset / 60;
        let mm = offset % 60;
        let zero = u16::from(b'0');
        let mut buf: [u16; 9] = [
            u16::from(b'G'),
            u16::from(b'M'),
            u16::from(b'T'),
            sign,
            zero,
            zero,
            u16::from(b':'),
            zero,
            zero,
        ];
        if hh >= 10 {
            buf[4] = buf[4].wrapping_add((hh / 10) as u16);
        }
        buf[5] = buf[5].wrapping_add((hh % 10) as u16);
        if mm != 0 {
            buf[7] = buf[7].wrapping_add((mm / 10) as u16);
            buf[8] = buf[8].wrapping_add((mm % 10) as u16);
        }
        String::from_utf16_lossy(&buf)
    }

    /// `getID()`.
    pub fn get_id(&self) -> &'static str {
        self.id
    }

    /// The `sun.util.calendar.ZoneInfo` `TimeZone.getTimeZone(id)` returns,
    /// whose rules give the offsets.
    pub fn zone_info(&self) -> &'static crate::jdk::time_zone::ZoneInfo {
        match self.rules {
            ZoneRules::Available(id) => crate::jdk::time_zone::zone_info(id)
                .expect("INVARIANT: the rules of an ID of TimeZone.getAvailableIDs()"),
            ZoneRules::Custom(offset) => crate::jdk::time_zone::custom_zone_info(offset),
        }
    }

    /// `getOffset(long date)`: the offset from UTC, in milliseconds, at the
    /// UTC instant `date`, daylight saving time included.
    pub fn get_offset(&self, date: i64) -> i32 {
        self.zone_info().get_offset(date)
    }

    /// `getRawOffset()`: the offset of standard time, in milliseconds.
    pub fn get_raw_offset(&self) -> i32 {
        match self.rules {
            ZoneRules::Available(_) => self.zone_info().raw_offset(),
            ZoneRules::Custom(offset) => offset,
        }
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

    /// `assertsAreEnabled`: "true if assertions (-ea) are enabled (at least
    /// for this class)"; `debug_assertions`, the Rust counterpart of `-ea`.
    pub const ASSERTS_ARE_ENABLED: bool = cfg!(debug_assertions);

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
        TimeZone::get_time_zone(RandomPicks::random_from_list(random, &tz_ids))
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
        use rucene::error::{LuceneError, LuceneErrorKind};
        use rucene::store::java_nio_file::{java_class_of, NO_SUCH_FILE_EXCEPTION};
        match dir.open_input(file_name, &*rucene::store::READONCE_IO_CONTEXT) {
            Ok(mut input) => {
                input.close()?;
                Ok(true)
            }
            // An `io::Error` of kind `NotFound` that carries no Java class
            // stands for one of the two (an OS error the port did not name).
            Err(LuceneError {
                kind: LuceneErrorKind::Io(e),
                ..
            }) if matches!(
                java_class_of(&e),
                NO_SUCH_FILE_EXCEPTION | "java.io.FileNotFoundException"
            ) || (e.kind() == std::io::ErrorKind::NotFound
                && rucene::store::java_nio_file::java_exception_of(&e).is_none()) =>
            {
                Ok(false)
            }
            // `IndexNotFoundException extends FileNotFoundException`.
            Err(LuceneError {
                kind: LuceneErrorKind::IndexNotFound(_),
                ..
            }) => Ok(false),
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

    /// Replaces the `executor` field and returns the previous value: what the
    /// oracles do reflectively (`Field.set(null, executor)`).
    pub fn replace_executor(
        executor: Option<Arc<LuceneTestCaseExecutor>>,
    ) -> Option<Arc<LuceneTestCaseExecutor>> {
        std::mem::replace(
            &mut *EXECUTOR
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            executor,
        )
    }

    /// The shared executor, when `setUpExecutorService` created it.
    pub fn executor() -> Option<Arc<LuceneTestCaseExecutor>> {
        EXECUTOR
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// `classEnvRule.similarity`: the similarity `TestRuleSetupAndRestoreClassEnv`
    /// draws for the class (`new AssertingSimilarity(new
    /// RandomSimilarity(random()))`), which `newSearcher` installs.
    ///
    /// # Panics
    ///
    /// `NullPointerException` before the rule drew it, where Java's
    /// `setSimilarity(null)` would leave the searcher unusable.
    pub fn class_env_similarity() -> Arc<dyn Similarity> {
        match crate::util::lucene_test_case_rules::CLASS_ENV_RULE.similarity() {
            Some(similarity) => similarity,
            None => exception::throw_without_message(exception::NULL_POINTER_EXCEPTION),
        }
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
mod ensure_sane_iwc_tests {
    use std::sync::Arc;

    use rucene::index::index_writer_config::IndexWriterConfig;
    use rucene::index::merge_policy::{LogMergePolicy, MergePolicy, TieredMergePolicy};

    use super::LuceneTestCase;

    /// `ensureSaneIWCOnNightly`'s body changes the merge policy the
    /// configuration already holds — the same instance, not a copy — as
    /// Java's casts and setters do. Expected strings measured with
    /// `lucene-core-10.5.0.jar` (JDK 25) running the same body on
    /// `new LogByteSizeMergePolicy()`, `new LogDocMergePolicy()` and
    /// `new TieredMergePolicy()` after `setNoCFSRatio(0.1)`: the policy held is
    /// `==` the one installed, `getUseCompoundFile()` is `true`, and
    /// `toString()` is the string below (Lucene 10.5.0's
    /// `TieredMergePolicy.toString` has no closing `]`).
    #[test]
    fn mutates_the_installed_policy_as_lucene_does() {
        let policies: [(Arc<dyn MergePolicy>, &str); 3] = [
            (
                Arc::new(LogMergePolicy::by_byte_size()),
                "[LogByteSizeMergePolicy: minMergeSize=16777216, mergeFactor=10, \
                 maxMergeSize=1048576000, maxMergeSizeForForcedMerge=9223372036854775807, \
                 calibrateSizeByDeletes=true, maxMergeDocs=2147483647, \
                 maxCFSSegmentSizeMB=8.796093022208E12, noCFSRatio=0.25]",
            ),
            (
                Arc::new(LogMergePolicy::by_doc_count()),
                "[LogDocMergePolicy: minMergeSize=1000, mergeFactor=10, \
                 maxMergeSize=9223372036854775807, maxMergeSizeForForcedMerge=9223372036854775807, \
                 calibrateSizeByDeletes=true, maxMergeDocs=100000, \
                 maxCFSSegmentSizeMB=8.796093022208E12, noCFSRatio=0.25]",
            ),
            (
                Arc::new(TieredMergePolicy::new()),
                "[TieredMergePolicy: maxMergeAtOnce=10, maxMergedSegmentMB=5000.0, \
                 floorSegmentMB=16.0, forceMergeDeletesPctAllowed=10.0, segmentsPerTier=8.0, \
                 maxCFSSegmentSizeMB=8.796093022208E12, noCFSRatio=0.25, deletesPctAllowed=20.0, \
                 targetSearchConcurrency=1",
            ),
        ];
        for (mp, expected) in policies {
            let mut conf = IndexWriterConfig::new();
            conf.set_merge_policy(Some(Arc::clone(&mp))).unwrap();
            mp.set_no_cfs_ratio(0.1).unwrap();
            LuceneTestCase::ensure_sane_iwc(&mut conf);
            assert!(Arc::ptr_eq(&conf.merge_policy(), &mp));
            assert!(conf.use_compound_file());
            assert_eq!(mp.to_string(), expected);
        }
    }
}

#[cfg(test)]
mod vector_format_support_tests {
    use rucene::codecs::knn_vectors::{knn_vectors_for_name, reload_knn_vectors_format};
    use rucene::index::VectorEncoding;

    use super::LuceneTestCase;

    /// `supportsVectorEncoding(format, BYTE)`, `(format, FLOAT32)` and
    /// `supportsVectorSearch(format)` answer as Java's for every KNN-vectors
    /// format on the oracle's class path (`vfmt` rows of `ltc_members.tsv`,
    /// recorded by reflection on `LuceneTestCase` under `lucene-core`,
    /// `lucene-codecs` and `lucene-test-framework` 10.5.0).
    #[test]
    fn support_predicates_match_lucene() {
        reload_knn_vectors_format(crate::class_loader()).unwrap();
        reload_knn_vectors_format(rucene_codecs::class_loader()).unwrap();
        let fixture = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/ltc_members.tsv"
        ))
        .unwrap();
        let mut rows = 0;
        for line in fixture.lines().filter(|l| l.starts_with("vfmt\t")) {
            let r: Vec<&str> = line.split('\t').collect();
            let format = knn_vectors_for_name(r[1]).unwrap_or_else(|| panic!("{} missing", r[1]));
            let actual = (
                format!(
                    "{},{}",
                    LuceneTestCase::supports_vector_encoding(format.as_ref(), VectorEncoding::BYTE),
                    LuceneTestCase::supports_vector_encoding(
                        format.as_ref(),
                        VectorEncoding::FLOAT32
                    )
                ),
                LuceneTestCase::supports_vector_search(format.as_ref()).to_string(),
            );
            assert_eq!(
                (actual.0.as_str(), actual.1.as_str()),
                (r[2], r[3]),
                "{}",
                r[1]
            );
            rows += 1;
        }
        assert_eq!(rows, 5);
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

// -----------------------------------------------------------------------------
// The writer and directory members `BaseTokenStreamTestCase.checkRandomData`'s
// indexing branch and `TestMockAnalyzer.testChangeGaps` call, ported in full
// under the sprint's cycle rule.
// -----------------------------------------------------------------------------

/// Port of the nested enum `LuceneTestCase.LiveIWCFlushMode`: "Flush mode for
/// live IW config", what `maybeChangeLiveIndexWriterConfig` changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LiveIWCFlushMode {
    /// `BY_RAM`.
    ByRam,
    /// `BY_DOCS`.
    ByDocs,
    /// `EITHER`.
    Either,
}

/// `fieldToType`: the field types `newField` chose per field name, which
/// `tearDown()` clears.
static FIELD_TO_TYPE: LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, rucene::document::FieldType>>,
> = LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// `liveIWCFlushMode`, which `TestRuleSetupAndRestoreClassEnv` sets.
static LIVE_IWC_FLUSH_MODE: std::sync::Mutex<Option<LiveIWCFlushMode>> =
    std::sync::Mutex::new(None);

/// `TimeZone.getDefault()`, which `TestRuleSetupAndRestoreClassEnv` sets
/// (`TimeZone.setDefault(timeZone)`); `None` until it is set (see
/// [`LuceneTestCase::get_default_time_zone`]).
static DEFAULT_TIME_ZONE: std::sync::Mutex<Option<TimeZone>> = std::sync::Mutex::new(None);

/// `FS_DIRECTORIES`: "Filesystem-based Directory implementations."
const FS_DIRECTORIES: [&str; 2] = ["NIOFSDirectory", "MMapDirectory"];

/// `CORE_DIRECTORIES`: "All Directory implementations", `FS_DIRECTORIES` and
/// `ByteBuffersDirectory`.
const CORE_DIRECTORIES: [&str; 3] = ["NIOFSDirectory", "MMapDirectory", "ByteBuffersDirectory"];

/// `ByteBuffersDirectory.class.getName()`.
const BYTE_BUFFERS_DIRECTORY: &str = "org.apache.lucene.store.ByteBuffersDirectory";

/// The `BaseDirectoryWrapper` `newDirectory`, `newFSDirectory` and
/// `wrapDirectory` return: a [`MockDirectoryWrapper`](crate::store::MockDirectoryWrapper)
/// or, for a bare directory, a [`RawDirectoryWrapper`](crate::store::RawDirectoryWrapper).
///
/// Java returns the abstract class `BaseDirectoryWrapper`; Rust has no class
/// hierarchy, so the two subclasses are the variants. A writer or a reader
/// takes [`as_directory`](Self::as_directory); [`close`](Self::close) is
/// `Directory.close()`.
pub enum TestDirectory {
    /// A `MockDirectoryWrapper`.
    Mock(crate::store::MockDirectoryWrapper),
    /// A `RawDirectoryWrapper` (a bare directory).
    Raw(Arc<crate::store::RawDirectoryWrapper>),
}

impl TestDirectory {
    /// The wrapper as the `Arc<dyn Directory>` a writer or a reader takes.
    pub fn as_directory(&self) -> Arc<dyn rucene::store::Directory> {
        match self {
            TestDirectory::Mock(mock) => mock.as_directory(),
            TestDirectory::Raw(raw) => Arc::clone(raw) as Arc<dyn rucene::store::Directory>,
        }
    }

    /// `isOpen()`.
    pub fn is_open(&self) -> bool {
        match self {
            TestDirectory::Mock(mock) => mock.is_open(),
            TestDirectory::Raw(raw) => raw.base().is_open(),
        }
    }

    /// The `MockDirectoryWrapper`, when the wrapper is one.
    pub fn as_mock(&self) -> Option<&crate::store::MockDirectoryWrapper> {
        match self {
            TestDirectory::Mock(mock) => Some(mock),
            TestDirectory::Raw(_) => None,
        }
    }

    /// `close()`.
    ///
    /// # Errors
    ///
    /// What the wrapper's `close()` throws.
    pub fn close(&self) -> rucene::error::Result<()> {
        use rucene::store::Directory;
        match self {
            TestDirectory::Mock(mock) => mock.close(),
            TestDirectory::Raw(raw) => raw.close(),
        }
    }

    /// `closeAfterSuite(new CloseableDirectory(this, suiteFailureMarker))`.
    fn close_after_suite(&self) {
        let probe = match self {
            TestDirectory::Mock(mock) => mock.open_probe(),
            TestDirectory::Raw(raw) => crate::util::closeable_directory::DirectoryOpenProbe::new(
                raw.base().open_flag(),
                raw.to_string(),
            ),
        };
        RandomizedTest::close_after_suite(Arc::new(crate::util::CloseableDirectory::new(
            Arc::new(probe),
            Arc::clone(&SUITE_FAILURE_MARKER),
        )));
    }
}

impl std::fmt::Display for TestDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TestDirectory::Mock(mock) => mock.fmt(f),
            TestDirectory::Raw(raw) => raw.fmt(f),
        }
    }
}

/// `Rethrow.rethrow(e)` of a checked exception a reflective constructor
/// threw: the error as the Java throwable.
fn rethrow_lucene_error(e: &rucene::error::LuceneError) -> ! {
    let (class, message) = crate::analysis::base_token_stream_test_case::java_class_and_message(e);
    Throwable::new(class, message).throw()
}

impl LuceneTestCase {
    /// `setLiveIWCFlushMode(flushMode)` (package-private), which
    /// `TestRuleSetupAndRestoreClassEnv.before()` calls. Package-private in
    /// Java; public for the oracles, which set the field reflectively.
    pub fn set_live_iwc_flush_mode(flush_mode: Option<LiveIWCFlushMode>) {
        *LIVE_IWC_FLUSH_MODE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = flush_mode;
    }

    /// `liveIWCFlushMode`.
    ///
    /// # Panics
    ///
    /// When the class environment has not set it, where Java's `switch` over
    /// the `null` field throws `NullPointerException`.
    pub fn live_iwc_flush_mode() -> LiveIWCFlushMode {
        LIVE_IWC_FLUSH_MODE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .unwrap_or_else(|| exception::throw_without_message(exception::NULL_POINTER_EXCEPTION))
    }

    /// `fieldToType.clear()`, as `tearDown()` calls it.
    pub(crate) fn clear_field_to_type() {
        FIELD_TO_TYPE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    /// `setIndexWriterMaxDocs(limit)`: "tells `IndexWriter` to enforce the
    /// specified limit as the maximum number of documents in one index; call
    /// `restoreIndexWriterMaxDocs` once your test is done."
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` for a limit above `IndexWriter.MAX_DOCS`.
    pub fn set_index_writer_max_docs(limit: i32) {
        if let Err(e) = rucene::index::index_writer::set_max_docs(limit) {
            exception::illegal_argument(e)
        }
    }

    /// `restoreIndexWriterMaxDocs()`: "returns to the default
    /// `IndexWriter.MAX_DOCS` limit."
    pub fn restore_index_writer_max_docs() {
        Self::set_index_writer_max_docs(rucene::index::documents_writer::MAX_DOCS);
    }

    /// `TimeZone.getDefault()`: the time zone `TestRuleSetupAndRestoreClassEnv`
    /// set for the class.
    ///
    /// **Divergence.** A Rust process has no JVM default time zone: before
    /// the rule set one, this is `None` where Java returns the platform's.
    pub fn get_default_time_zone() -> Option<TimeZone> {
        *DEFAULT_TIME_ZONE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `TimeZone.setDefault(timeZone)` (see
    /// [`get_default_time_zone`](Self::get_default_time_zone)).
    pub(crate) fn set_default_time_zone(time_zone: Option<TimeZone>) {
        *DEFAULT_TIME_ZONE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = time_zone;
    }

    /// `getOnlyLeafReader(reader)`: "Some tests expect the directory to contain
    /// a single segment, and want to do tests on that segment's reader. This is
    /// an utility method to help them."
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` (`… has N segments instead of exactly one`),
    /// and what listing the leaves throws.
    pub fn get_only_leaf_reader(
        reader: Arc<dyn IndexReader>,
    ) -> rucene::error::Result<Arc<dyn rucene::index::LeafReader>> {
        let sub_readers = Arc::clone(&reader).leaves()?;
        if sub_readers.len() != 1 {
            return Err(rucene::error::LuceneError::illegal_argument(format!(
                "{} has {} segments instead of exactly one",
                reader,
                sub_readers.len()
            )));
        }
        Ok(sub_readers[0].leaf_reader())
    }

    /// `newIndexWriterConfig()`: "create a new index writer config with random
    /// defaults", over a `new MockAnalyzer(random())`.
    pub fn new_index_writer_config() -> rucene::index::index_writer_config::IndexWriterConfig {
        let random = Self::random();
        Self::new_index_writer_config_analyzer(Some(Arc::new(crate::analysis::MockAnalyzer::new(
            &*random,
        ))))
    }

    /// `newIndexWriterConfig(a)`: "create a new index writer config with random
    /// defaults". `None` is Java's `null` analyzer.
    pub fn new_index_writer_config_analyzer(
        a: Option<Arc<dyn rucene::analysis::Analyzer>>,
    ) -> rucene::index::index_writer_config::IndexWriterConfig {
        let random = Self::random();
        Self::new_index_writer_config_with(&*random, a)
    }

    /// `newIndexWriterConfig(r, a)`: "create a new index writer config with
    /// random defaults using the specified random".
    ///
    /// # Panics
    ///
    /// When the class environment has not set `classEnvRule.similarity` (or,
    /// when the merge policy drawn is an `AlcoholicMergePolicy`,
    /// `classEnvRule.timeZone`), and on the failures Java's setters throw for
    /// the values drawn, which never happen.
    pub fn new_index_writer_config_with(
        r: &dyn Random,
        a: Option<Arc<dyn rucene::analysis::Analyzer>>,
    ) -> rucene::index::index_writer_config::IndexWriterConfig {
        use rucene::index::merge_scheduler::{ConcurrentMergeScheduler, SerialMergeScheduler};
        const INVARIANT: &str = "INVARIANT: the drawn value is in the setter's range";

        let c = rucene::index::index_writer_config::IndexWriterConfig::with_analyzer(a);
        c.set_similarity(Some(Self::class_env_similarity()))
            .expect(INVARIANT);
        if *VERBOSE {
            // Even though TestRuleSetupAndRestoreClassEnv calls
            // InfoStream.setDefault, we do it again here so that
            // the PrintStreamInfoStream.messageID increments so
            // that when there are separate instances of
            // IndexWriter created we see "IW 0", "IW 1", "IW 2",
            // ... instead of just always "IW 0":
            c.set_info_stream(Arc::new(
                crate::util::test_rule_setup_and_restore_class_env::ThreadNameFixingPrintStreamInfoStream::new_system_out(),
            ))
            .expect(INVARIANT);
        }

        if Self::rarely_with(r) {
            c.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
                .expect(INVARIANT);
        } else if Self::rarely_with(r) {
            let cms = if r.next_boolean() {
                Self::new_test_concurrent_merge_scheduler()
            } else {
                let cms = Self::new_test_concurrent_merge_scheduler();
                cms.set_maybe_stall_override(Some(Arc::new(|_merge_source| Ok(true))));
                cms
            };
            let max_thread_count = TestUtil::next_int(r, 1, 4);
            let max_merge_count = TestUtil::next_int(r, max_thread_count, max_thread_count + 4);
            cms.set_max_merges_and_threads(max_merge_count, max_thread_count)
                .expect(INVARIANT);
            if Self::random().next_boolean() {
                cms.disable_auto_io_throttle().expect(INVARIANT);
                Assert::assert_false(cms.get_auto_io_throttle());
            }
            cms.set_force_merge_mb_per_sec(10.0 + 10.0 * Self::random().next_double())
                .expect(INVARIANT);
            c.set_merge_scheduler(Some(Arc::new(cms))).expect(INVARIANT);
        } else {
            // Always use consistent settings, else CMS's dynamic (SSD or not)
            // defaults can change, hurting reproducibility:
            let cms = if RandomizedTest::random_boolean() {
                Self::new_test_concurrent_merge_scheduler()
            } else {
                ConcurrentMergeScheduler::new()
            };

            // Only 1 thread can run at once (should maybe help reproducibility),
            // with up to 3 pending merges before segment-producing threads are
            // stalled:
            cms.set_max_merges_and_threads(3, 1).expect(INVARIANT);
            c.set_merge_scheduler(Some(Arc::new(cms))).expect(INVARIANT);
        }

        if r.next_boolean() {
            if Self::rarely_with(r) {
                // crazy value
                c.set_max_buffered_docs(TestUtil::next_int(r, 2, 15))
                    .expect(INVARIANT);
            } else {
                // reasonable value
                c.set_max_buffered_docs(TestUtil::next_int(r, 16, 1000))
                    .expect(INVARIANT);
            }
        }

        c.set_merge_policy(Some(Self::new_merge_policy_with(r)))
            .expect(INVARIANT);

        if Self::rarely_with(r) {
            let info_stream = c.info_stream();
            c.set_merged_segment_warmer(Some(Arc::new(
                rucene::index::sorting_codec_reader::SimpleMergedSegmentWarmer::new(info_stream),
            )))
            .expect(INVARIANT);
        }
        c.set_use_compound_file(r.next_boolean()).expect(INVARIANT);
        c.set_reader_pooling(r.next_boolean());
        if Self::rarely_with(r) {
            c.set_check_pending_flush_update(false).expect(INVARIANT);
        }

        if Self::rarely_with(r) {
            c.set_index_writer_event_listener(Arc::new(
                crate::index::MockIndexWriterEventListener::new(),
            ));
        }
        match r.next_int_bound(3) {
            0 => {
                // Disable merge on refresh
                c.set_max_full_flush_merge_wait_millis(0);
            }
            1 => {
                // Very low timeout, merges will likely not be able to run in time
                c.set_max_full_flush_merge_wait_millis(1);
            }
            _ => {
                // Very long timeout, merges will almost always be able to run in time
                c.set_max_full_flush_merge_wait_millis(1000);
            }
        }

        let wait = if Self::rarely() {
            Self::at_least_with(r, 1000)
        } else {
            Self::at_least_with(r, 200)
        };
        c.set_max_full_flush_merge_wait_millis(i64::from(wait));
        c
    }

    /// `new TestConcurrentMergeScheduler()`: the package-private
    /// `LuceneTestCase.TestConcurrentMergeScheduler extends
    /// ConcurrentMergeScheduler`, whose `getIntraMergeExecutor` "Always do[es]
    /// the intra merge executor to ensure we test it". The override is the
    /// scheduler's hook, as every `ConcurrentMergeScheduler` subclass is in
    /// this port.
    pub fn new_test_concurrent_merge_scheduler(
    ) -> rucene::index::merge_scheduler::ConcurrentMergeScheduler {
        let cms = rucene::index::merge_scheduler::ConcurrentMergeScheduler::new();
        cms.set_intra_merge_executor_override(Some(Arc::new(|cms, _merge| {
            let executor = cms.intra_merge_executor();
            randomizedtesting::jdk::exception::java_assert(executor.is_some(), || {
                "scaledExecutor is not initialized".to_string()
            });
            // Always do the intra merge executor to ensure we test it
            Ok(executor)
        })));
        cms
    }

    /// `newMergePolicy(r)`: `newMergePolicy(r, true)`.
    pub fn new_merge_policy_with(
        r: &dyn Random,
    ) -> Arc<dyn rucene::index::merge_policy::MergePolicy> {
        Self::new_merge_policy_include_mock(r, true)
    }

    /// `newMergePolicy(r, includeMockMP)`.
    pub fn new_merge_policy_include_mock(
        r: &dyn Random,
        include_mock_mp: bool,
    ) -> Arc<dyn rucene::index::merge_policy::MergePolicy> {
        if include_mock_mp && Self::rarely_with(r) {
            Arc::new(crate::index::MockRandomMergePolicy::new(r))
        } else if r.next_boolean() {
            Arc::new(Self::new_tiered_merge_policy_with(r))
        } else if Self::rarely_with(r) {
            Arc::new(Self::new_alcoholic_merge_policy_with(
                r,
                Self::get_default_time_zone().unwrap_or_else(|| {
                    exception::throw_without_message(exception::NULL_POINTER_EXCEPTION)
                }),
            ))
        } else {
            Arc::new(Self::new_log_merge_policy_with(r))
        }
    }

    /// `newMergePolicy()`: `newMergePolicy(random())`.
    pub fn new_merge_policy() -> Arc<dyn rucene::index::merge_policy::MergePolicy> {
        Self::new_merge_policy_with(&*Self::random())
    }

    /// `newAlcoholicMergePolicy(r, tz)`: `new AlcoholicMergePolicy(tz, new
    /// Random(r.nextLong()))`.
    pub fn new_alcoholic_merge_policy_with(
        r: &dyn Random,
        tz: TimeZone,
    ) -> rucene::index::merge_policy::LogMergePolicy {
        crate::index::AlcoholicMergePolicy::new(
            tz,
            randomizedtesting::jdk::random::JdkRandom::new(r.next_long()),
        )
    }

    /// `newLogMergePolicy(r)`.
    pub fn new_log_merge_policy_with(
        r: &dyn Random,
    ) -> rucene::index::merge_policy::LogMergePolicy {
        use rucene::index::merge_policy::LogMergePolicy;
        const INVARIANT: &str = "INVARIANT: the drawn value is in the setter's range";
        let mut logmp = if r.next_boolean() {
            LogMergePolicy::by_doc_count()
        } else {
            LogMergePolicy::by_byte_size()
        };
        logmp.set_calibrate_size_by_deletes(r.next_boolean());
        logmp
            .set_target_search_concurrency(TestUtil::next_int(&*Self::random(), 1, 16))
            .expect(INVARIANT);
        if Self::rarely_with(r) {
            logmp
                .set_merge_factor(TestUtil::next_int(r, 2, 9))
                .expect(INVARIANT);
        } else {
            logmp
                .set_merge_factor(TestUtil::next_int(r, 10, 50))
                .expect(INVARIANT);
        }
        Self::configure_random(r, &logmp);
        logmp
    }

    /// `configureRandom(r, mergePolicy)` (private).
    fn configure_random(
        r: &dyn Random,
        merge_policy: &dyn rucene::index::merge_policy::MergePolicy,
    ) {
        const INVARIANT: &str = "INVARIANT: the drawn value is in the setter's range";
        if r.next_boolean() {
            merge_policy
                .set_no_cfs_ratio(0.1 + r.next_double() * 0.8)
                .expect(INVARIANT);
        } else {
            merge_policy
                .set_no_cfs_ratio(if r.next_boolean() { 1.0 } else { 0.0 })
                .expect(INVARIANT);
        }

        if Self::rarely_with(r) {
            merge_policy
                .set_max_cfs_segment_size_mb(0.2 + r.next_double() * 2.0)
                .expect(INVARIANT);
        } else {
            merge_policy
                .set_max_cfs_segment_size_mb(f64::INFINITY)
                .expect(INVARIANT);
        }
    }

    /// `newTieredMergePolicy(r)`.
    pub fn new_tiered_merge_policy_with(
        r: &dyn Random,
    ) -> rucene::index::merge_policy::TieredMergePolicy {
        const INVARIANT: &str = "INVARIANT: the drawn value is in the setter's range";
        let mut tmp = rucene::index::merge_policy::TieredMergePolicy::new();
        if Self::rarely_with(r) {
            tmp.set_max_merge_at_once(TestUtil::next_int(r, 2, 9))
                .expect(INVARIANT);
        } else {
            tmp.set_max_merge_at_once(TestUtil::next_int(r, 10, 50))
                .expect(INVARIANT);
        }
        if Self::rarely_with(r) {
            tmp.set_max_merged_segment_mb(0.2 + r.next_double() * 2.0)
                .expect(INVARIANT);
        } else {
            tmp.set_max_merged_segment_mb(10.0 + r.next_double() * 100.0)
                .expect(INVARIANT);
        }
        tmp.set_floor_segment_mb(0.2 + r.next_double() * 2.0)
            .expect(INVARIANT);
        tmp.set_force_merge_deletes_pct_allowed(0.0 + r.next_double() * 30.0)
            .expect(INVARIANT);
        if Self::rarely_with(r) {
            tmp.set_segments_per_tier(f64::from(TestUtil::next_int(r, 2, 20)))
                .expect(INVARIANT);
        } else {
            tmp.set_segments_per_tier(f64::from(TestUtil::next_int(r, 10, 50)))
                .expect(INVARIANT);
        }
        if Self::rarely_with(r) {
            tmp.set_target_search_concurrency(TestUtil::next_int(r, 10, 50))
                .expect(INVARIANT);
        } else {
            tmp.set_target_search_concurrency(TestUtil::next_int(r, 2, 20))
                .expect(INVARIANT);
        }

        Self::configure_random(r, &tmp);
        tmp.set_deletes_pct_allowed(20.0 + Self::random().next_double() * 30.0)
            .expect(INVARIANT);
        tmp
    }

    /// `maybeChangeLiveIndexWriterConfig(r, c)`: "if you want it in
    /// LiveIndexWriterConfig: it must and will be tested here."
    ///
    /// # Panics
    ///
    /// When the class environment has not set `liveIWCFlushMode` and the
    /// flush parameters are drawn to change.
    pub fn maybe_change_live_index_writer_config(
        r: &dyn Random,
        c: &rucene::index::index_writer_config::LiveIndexWriterConfig,
    ) {
        use rucene::index::index_writer_config::LiveIndexWriterConfig;
        use rucene::index::merge_policy::{LogMergePolicy, TieredMergePolicy};
        use rucene::index::merge_scheduler::ConcurrentMergeScheduler;
        const INVARIANT: &str = "INVARIANT: the drawn value is in the setter's range";

        let mut did_change = false;

        let previous = c.to_string();

        if Self::rarely_with(r) {
            // change flush parameters:
            // this is complicated because the api requires you "invoke setters in a magical order!"
            // LUCENE-5661: workaround for race conditions in the API
            c.synchronized_flush_settings(|settings| {
                let flush_by_ram = match Self::live_iwc_flush_mode() {
                    LiveIWCFlushMode::ByRam => true,
                    LiveIWCFlushMode::ByDocs => false,
                    LiveIWCFlushMode::Either => r.next_boolean(),
                };
                if flush_by_ram {
                    settings
                        .set_ram_buffer_size_mb(f64::from(TestUtil::next_int(r, 1, 10)))
                        .expect(INVARIANT);
                    settings
                        .set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)
                        .expect(INVARIANT);
                } else {
                    if Self::rarely_with(r) {
                        // crazy value
                        settings
                            .set_max_buffered_docs(TestUtil::next_int(r, 2, 15))
                            .expect(INVARIANT);
                    } else {
                        // reasonable value
                        settings
                            .set_max_buffered_docs(TestUtil::next_int(r, 16, 1000))
                            .expect(INVARIANT);
                    }
                    settings
                        .set_ram_buffer_size_mb(f64::from(
                            LiveIndexWriterConfig::DISABLE_AUTO_FLUSH,
                        ))
                        .expect(INVARIANT);
                }
            });
            did_change = true;
        }

        if Self::rarely_with(r) {
            let cur_warmer = c.merged_segment_warmer();
            if cur_warmer.as_ref().is_none_or(|warmer| {
                rucene::index::index_writer_config::MergedSegmentWarmer::as_any(&**warmer)
                    .is_some_and(|any| {
                        any.is::<rucene::index::sorting_codec_reader::SimpleMergedSegmentWarmer>()
                    })
            }) {
                // change warmer parameters
                if r.next_boolean() {
                    c.set_merged_segment_warmer(Some(Arc::new(
                        rucene::index::sorting_codec_reader::SimpleMergedSegmentWarmer::new(
                            c.info_stream(),
                        ),
                    )));
                } else {
                    c.set_merged_segment_warmer(None);
                }
            }
            did_change = true;
        }

        if Self::rarely_with(r) {
            // change CFS flush parameters
            c.set_use_compound_file(r.next_boolean());
            did_change = true;
        }

        if Self::rarely_with(r) {
            // change CMS merge parameters
            let ms = c.merge_scheduler();
            if let Some(cms) = ms
                .as_any()
                .and_then(|any| any.downcast_ref::<ConcurrentMergeScheduler>())
            {
                let max_thread_count = TestUtil::next_int(r, 1, 4);
                let max_merge_count = TestUtil::next_int(r, max_thread_count, max_thread_count + 4);
                let enable_auto_io_throttle = Self::random().next_boolean();
                if enable_auto_io_throttle {
                    cms.enable_auto_io_throttle().expect(INVARIANT);
                } else {
                    cms.disable_auto_io_throttle().expect(INVARIANT);
                }
                cms.set_max_merges_and_threads(max_merge_count, max_thread_count)
                    .expect(INVARIANT);
                did_change = true;
            }
        }

        if Self::rarely_with(r) {
            let mp = c.merge_policy();
            Self::configure_random(r, &*mp);
            let any = mp.as_any();
            if let Some(logmp) = any.and_then(|any| any.downcast_ref::<LogMergePolicy>()) {
                logmp.set_calibrate_size_by_deletes(r.next_boolean());
                if Self::rarely_with(r) {
                    logmp
                        .set_merge_factor(TestUtil::next_int(r, 2, 9))
                        .expect(INVARIANT);
                } else {
                    logmp
                        .set_merge_factor(TestUtil::next_int(r, 10, 50))
                        .expect(INVARIANT);
                }
            } else if let Some(tmp) = any.and_then(|any| any.downcast_ref::<TieredMergePolicy>()) {
                if Self::rarely_with(r) {
                    tmp.set_max_merge_at_once(TestUtil::next_int(r, 2, 9))
                        .expect(INVARIANT);
                } else {
                    tmp.set_max_merge_at_once(TestUtil::next_int(r, 10, 50))
                        .expect(INVARIANT);
                }
                if Self::rarely_with(r) {
                    tmp.set_max_merged_segment_mb(0.2 + r.next_double() * 2.0)
                        .expect(INVARIANT);
                } else {
                    tmp.set_max_merged_segment_mb(r.next_double() * 100.0)
                        .expect(INVARIANT);
                }
                tmp.set_floor_segment_mb(0.2 + r.next_double() * 2.0)
                    .expect(INVARIANT);
                tmp.set_force_merge_deletes_pct_allowed(0.0 + r.next_double() * 30.0)
                    .expect(INVARIANT);
                if Self::rarely_with(r) {
                    tmp.set_segments_per_tier(f64::from(TestUtil::next_int(r, 2, 20)))
                        .expect(INVARIANT);
                } else {
                    tmp.set_segments_per_tier(f64::from(TestUtil::next_int(r, 10, 50)))
                        .expect(INVARIANT);
                }
                Self::configure_random(r, tmp);
                tmp.set_deletes_pct_allowed(20.0 + Self::random().next_double() * 30.0)
                    .expect(INVARIANT);
            }
            did_change = true;
        }
        if *VERBOSE && did_change {
            let current = c.to_string();
            let previous_lines: Vec<&str> = previous.split('\n').collect();
            let current_lines: Vec<&str> = current.split('\n').collect();
            let mut diff = String::new();

            // this should always be the case, diff each line
            if previous_lines.len() == current_lines.len() {
                for (previous_line, current_line) in previous_lines.iter().zip(&current_lines) {
                    if previous_line != current_line {
                        diff.push_str(&format!("- {previous_line}\n"));
                        diff.push_str(&format!("+ {current_line}\n"));
                    }
                }
            } else {
                // but just in case of something ridiculous...
                diff.push_str(&current);
            }

            // its possible to be empty, if we "change" a value to what it had before.
            if !diff.is_empty() {
                println!("NOTE: LuceneTestCase: randomly changed IWC's live settings:");
                println!("{diff}");
            }
        }
    }

    /// `newDirectory()`: "Returns a new Directory instance. Use this when the
    /// test does not care about the specific Directory implementation (most
    /// tests)."
    pub fn new_directory() -> TestDirectory {
        Self::new_directory_with(&*Self::random())
    }

    /// `newDirectory(r)`: "Returns a new Directory instance, using the
    /// specified random."
    pub fn new_directory_with(r: &dyn Random) -> TestDirectory {
        let directory = Self::new_directory_impl(r, &TEST_DIRECTORY, &LockFactoryArg::Default);
        let bare = Self::rarely_with(r);
        Self::wrap_directory(r, directory, bare, false)
    }

    /// `newFSDirectory(f)`: "Returns a new FSDirectory instance over the given
    /// file, which must be a folder.", with `FSLockFactory.getDefault()`.
    pub fn new_fs_directory(f: &Path) -> TestDirectory {
        let bare = Self::rarely();
        Self::new_fs_directory_bare(f, &LockFactoryArg::Default, bare)
    }

    /// The private `newFSDirectory(f, lf, bare)`.
    fn new_fs_directory_bare(f: &Path, lf: &LockFactoryArg, bare: bool) -> TestDirectory {
        use rucene::util::command_line_util::CommandLineUtil;
        let random = Self::random();
        let mut fsdir_class = TEST_DIRECTORY.clone();
        if fsdir_class == "random" {
            fsdir_class = RandomPicks::random_from(&*random, &FS_DIRECTORIES).to_string();
        }

        let clazz = match CommandLineUtil::load_fs_directory_class(&fsdir_class) {
            Ok(clazz) => clazz,
            // TEST_DIRECTORY is not a sub-class of FSDirectory, so draw one at random
            // (`CommandLineUtil.loadFSDirectoryClass`'s `ClassCastException`
            // carries `Class.toString()`, `class <name>` or `interface <name>`).
            Err(rucene::error::LuceneError {
                kind: rucene::error::LuceneErrorKind::Other(message),
                ..
            }) if message.starts_with("class ") || message.starts_with("interface ") => {
                fsdir_class = RandomPicks::random_from(&*random, &FS_DIRECTORIES).to_string();
                CommandLineUtil::load_fs_directory_class(&fsdir_class)
                    .unwrap_or_else(|e| rethrow_lucene_error(&e))
            }
            Err(e) => rethrow_lucene_error(&e),
        };

        let fsdir = Self::new_fs_directory_impl(clazz, f, lf);
        Self::wrap_directory(&*random, fsdir, bare, true)
    }

    /// `newFSDirectoryImpl(clazz, path, lf)` (private).
    fn new_fs_directory_impl(
        clazz: rucene::util::command_line_util::FSDirectoryKind,
        path: &Path,
        lf: &LockFactoryArg,
    ) -> Box<dyn rucene::store::Directory> {
        use rucene::util::command_line_util::CommandLineUtil;
        match CommandLineUtil::new_fs_directory_of_kind(clazz, path, lf.instance()) {
            Ok(d) => d,
            Err(e) => rethrow_lucene_error(&e),
        }
    }

    /// `newDirectoryImpl(random, clazzName)` and `newDirectoryImpl(random,
    /// clazzName, lf)` (package-private); [`LockFactoryArg::Default`] is the
    /// first overload's `FSLockFactory.getDefault()`.
    fn new_directory_impl(
        random: &dyn Random,
        clazz_name: &str,
        lf: &LockFactoryArg,
    ) -> Box<dyn rucene::store::Directory> {
        use rucene::util::command_line_util::CommandLineUtil;
        let mut clazz_name = clazz_name.to_string();
        if clazz_name == "random" {
            if Self::rarely_with(random) {
                clazz_name = RandomPicks::random_from(random, &CORE_DIRECTORIES).to_string();
            } else if Self::rarely_with(random) {
                let clazz_name1 = if Self::rarely_with(random) {
                    RandomPicks::random_from(random, &CORE_DIRECTORIES).to_string()
                } else {
                    BYTE_BUFFERS_DIRECTORY.to_string()
                };
                let clazz_name2 = if Self::rarely_with(random) {
                    RandomPicks::random_from(random, &CORE_DIRECTORIES).to_string()
                } else {
                    BYTE_BUFFERS_DIRECTORY.to_string()
                };
                let dir1 = Self::new_directory_impl(random, &clazz_name1, lf);
                let dir2 = Self::new_directory_impl(random, &clazz_name2, lf);
                return Self::new_file_switch_directory(random, dir1, dir2);
            } else {
                clazz_name = BYTE_BUFFERS_DIRECTORY.to_string();
            }
        }

        let clazz = CommandLineUtil::load_directory_class(&clazz_name)
            .unwrap_or_else(|e| rethrow_lucene_error(&e));
        // If it is a FSDirectory type, try its ctor(Path)
        if let Ok(kind) = CommandLineUtil::load_fs_directory_class(&clazz) {
            let dir = Self::create_temp_dir_prefix(&format!("index-{clazz_name}"));
            return Self::new_fs_directory_impl(kind, &dir, lf);
        }

        // See if it has a Path/LockFactory ctor even though it's not an
        // FSDir subclass: no other directory of the store package has one.
        // the remaining dirs are no longer filesystem based, so we must check that the
        // passedLockFactory is not file based:
        if clazz == BYTE_BUFFERS_DIRECTORY {
            if !lf.is_fs_lock_factory() {
                // try ctor with only LockFactory
                return Box::new(rucene::store::ByteBuffersDirectory::with_lock_factory(
                    lf.instance(),
                ));
            }
            // try empty ctor
            return Box::new(rucene::store::ByteBuffersDirectory::new());
        }
        rethrow_lucene_error(&rucene::error::LuceneError::unchecked(
            "java.lang.NoSuchMethodException",
            format!("{clazz}.<init>()"),
        ))
    }

    /// `newFileSwitchDirectory(random, dir1, dir2)` (private).
    fn new_file_switch_directory(
        random: &dyn Random,
        dir1: Box<dyn rucene::store::Directory>,
        dir2: Box<dyn rucene::store::Directory>,
    ) -> Box<dyn rucene::store::Directory> {
        let mut file_extensions = vec![
            "fdt", "fdx", "tim", "tip", "si", "fnm", "pos", "dii", "dim", "nvm", "nvd", "dvm",
            "dvd",
        ];
        randomizedtesting::jdk::util::shuffle(&mut file_extensions, random);
        let len = 1 + random.next_int_bound(file_extensions.len() as i32) as usize;
        file_extensions.truncate(len);
        let extensions: HashSet<String> = file_extensions.iter().map(|e| e.to_string()).collect();
        match rucene::store::FileSwitchDirectory::new(extensions, dir1, dir2, true) {
            Ok(d) => Box::new(d),
            Err(e) => rethrow_lucene_error(&e),
        }
    }

    /// `wrapDirectory(random, directory, bare, filesystem)` (private).
    fn wrap_directory(
        random: &dyn Random,
        directory: Box<dyn rucene::store::Directory>,
        bare: bool,
        filesystem: bool,
    ) -> TestDirectory {
        let mut directory = directory;
        // IOContext randomization might make NRTCachingDirectory make bad decisions, so avoid
        // using it if the user requested a filesystem directory.
        if Self::rarely_with(random) && !bare && !filesystem {
            let max_merge_size_mb = random.next_double();
            let max_cached_mb = random.next_double();
            directory = Box::new(rucene::store::NRTCachingDirectory::new(
                directory,
                max_merge_size_mb,
                max_cached_mb,
            ));
        }

        let directory: Arc<dyn rucene::store::Directory> = Arc::from(directory);
        let wrapped = if bare {
            TestDirectory::Raw(Arc::new(crate::store::RawDirectoryWrapper::new(directory)))
        } else {
            let mock = crate::store::MockDirectoryWrapper::new(random, directory);
            mock.set_throttling(*TEST_THROTTLING);
            TestDirectory::Mock(mock)
        };
        wrapped.close_after_suite();
        wrapped
    }
}

// ---------------------------------------------------------------------------
// Task #349: the remaining members of `LuceneTestCase`.
// ---------------------------------------------------------------------------

/// The `LockFactory lf` argument of the directory factories: Java's
/// `FSLockFactory.getDefault()` for the overloads without one, or the factory
/// the caller passed.
#[derive(Clone)]
pub enum LockFactoryArg {
    /// `FSLockFactory.getDefault()`: `NativeFSLockFactory.INSTANCE`.
    Default,
    /// The caller's factory, shared by every directory built with it as
    /// Java shares the one instance.
    Given(Arc<dyn rucene::store::LockFactory>),
}

impl LockFactoryArg {
    /// The factory as the `Box` a directory constructor takes.
    fn instance(&self) -> Box<dyn rucene::store::LockFactory> {
        match self {
            LockFactoryArg::Default => Box::new(rucene::store::NativeFSLockFactory),
            LockFactoryArg::Given(lf) => Box::new(SharedLockFactory(Arc::clone(lf))),
        }
    }

    /// `lf instanceof FSLockFactory`.
    ///
    /// **Divergence from Lucene 10.5.0.** A `dyn LockFactory` cannot be asked
    /// whether it implements [`rucene::store::FSLockFactory`]; the port
    /// recognises the two `FSLockFactory` subclasses Lucene ships,
    /// `NativeFSLockFactory` and `SimpleFSLockFactory`, by their type name.
    fn is_fs_lock_factory(&self) -> bool {
        match self {
            LockFactoryArg::Default => true,
            LockFactoryArg::Given(lf) => {
                let name = lf.directory_type_name();
                name.ends_with("::NativeFSLockFactory") || name.ends_with("::SimpleFSLockFactory")
            }
        }
    }
}

/// A [`rucene::store::LockFactory`] that forwards to a shared one, so that
/// several directories hold the same factory instance.
struct SharedLockFactory(Arc<dyn rucene::store::LockFactory>);

impl rucene::store::LockFactory for SharedLockFactory {
    fn obtain_lock(
        &self,
        dir: &dyn rucene::store::Directory,
        lock_name: &str,
    ) -> rucene::error::Result<Box<dyn rucene::store::Lock>> {
        self.0.obtain_lock(dir, lock_name)
    }

    fn directory_type_name(&self) -> &'static str {
        self.0.directory_type_name()
    }

    fn lock_factory_to_string(&self) -> String {
        self.0.lock_factory_to_string()
    }
}

/// The `Object value` of `newField(random, name, value, type)`: a `String`
/// or a `BytesRef` (Java's `createField` rejects anything else with
/// `IllegalArgumentException("value must be String or BytesRef")`, which the
/// type makes unrepresentable).
#[derive(Clone, Debug)]
pub enum NewFieldValue {
    /// A `String` value.
    String(String),
    /// A `BytesRef` value.
    BytesRef(BytesRef),
}

/// The order in which a `java.util.HashMap` (or `HashSet`) whose table has
/// `table_length` buckets iterates entries with the given hash codes, inserted
/// in the given order: by bucket `(h ^ (h >>> 16)) & (n - 1)`, then by
/// insertion order (a resize keeps each chain's relative order). A bucket
/// grown to a tree is not reproduced.
fn java_hash_order<T>(entries: Vec<(i32, T)>, table_length: usize) -> Vec<T> {
    let mut keyed: Vec<(usize, T)> = entries
        .into_iter()
        .map(|(h, t)| {
            let spread = h ^ ((h as u32) >> 16) as i32;
            ((spread as u32 as usize) & (table_length - 1), t)
        })
        .collect();
    keyed.sort_by_key(|(bucket, _)| *bucket);
    keyed.into_iter().map(|(_, t)| t).collect()
}

/// The table length of a `java.util.HashMap` built with `new HashMap<>()` (or
/// `new HashSet<>()`) after `size` insertions: 16, doubled whenever the size
/// exceeds three quarters of it.
fn java_default_table_length(size: usize) -> usize {
    let mut n = 16usize;
    while size > n * 3 / 4 {
        n *= 2;
    }
    n
}

/// The table length of `new HashSet<>(collection)` over `size` elements:
/// `tableSizeFor(max((int) (size / .75f) + 1, 16))`.
fn java_copy_table_length(size: usize) -> usize {
    let cap = std::cmp::max((size as f32 / 0.75f32) as usize + 1, 16);
    cap.next_power_of_two()
}

/// `AbstractCollection.toString()` of a `HashSet<String>` built as
/// `new HashSet<>(collection)`: `[a, b]` in Java's iteration order.
fn java_copied_string_set_to_string(set: &HashSet<String>) -> String {
    let mut names: Vec<&String> = set.iter().collect();
    names.sort();
    let entries = names
        .into_iter()
        .map(|n| {
            (
                randomizedtesting::jdk::lang::string_hash_code(n),
                n.as_str(),
            )
        })
        .collect();
    let ordered = java_hash_order(entries, java_copy_table_length(set.len()));
    format!("[{}]", ordered.join(", "))
}

/// JUnit's `assertEquals(message, Object, Object)` for two `HashSet<String>`
/// that `assertTermsEquals`, `assertNormsEquals` and `assertDocValuesEquals`
/// compare.
fn assert_string_sets_equal(info: &str, left: &HashSet<String>, right: &HashSet<String>) {
    if left != right {
        Assert::fail(&format!(
            "{} expected:<{}> but was:<{}>",
            info,
            java_copied_string_set_to_string(left),
            java_copied_string_set_to_string(right)
        ));
    }
}

/// `Assert.assertEquals(message, Object, Object)` for nullable values
/// (`None` is `null`).
fn assert_option_equals<T: PartialEq + Display>(
    message: Option<&str>,
    expected: Option<&T>,
    actual: Option<&T>,
) {
    if expected == actual {
        return;
    }
    let show = |v: Option<&T>| v.map_or_else(|| "null".to_string(), |v| v.to_string());
    let prefix = match message {
        Some(m) if !m.is_empty() => format!("{m} "),
        _ => String::new(),
    };
    Assert::fail(&format!(
        "{prefix}expected:<{}> but was:<{}>",
        show(expected),
        show(actual)
    ))
}

/// `Assert.assertNull(message, object)`: `message expected null, but
/// was:<object>`.
fn assert_null<T: Display>(message: Option<&str>, object: Option<&T>) {
    if let Some(object) = object {
        let prefix = match message {
            Some(m) => format!("{m} "),
            None => String::new(),
        };
        Assert::fail(&format!("{prefix}expected null, but was:<{object}>"))
    }
}

/// `Assert.assertNotNull(message, object)`: `assertTrue(message, object !=
/// null)`, an `AssertionError` carrying `message` (or none).
fn assert_not_null(message: Option<&str>, present: bool) {
    if !present {
        match message {
            Some(m) => Assert::fail(m),
            None => Assert::fail_without_message(),
        }
    }
}

/// Unwraps a `Result` the way a Java method declared `throws IOException`
/// lets the exception propagate: the error is thrown.
fn io<T>(result: rucene::error::Result<T>) -> T {
    result.unwrap_or_else(|e| rethrow_lucene_error(&e))
}

/// The string `PostingsEnum.getPayload()` renders as in a failure message:
/// `BytesRef.toString()`, or `null`.
fn payload_of(docs: &dyn rucene::index::postings_enum::PostingsEnum) -> Option<BytesRef> {
    io(docs.get_payload()).map(|p| BytesRef::new(p.to_vec()))
}

/// `SeekStatus.toString()`: the constant's name.
struct SeekStatusName(rucene::index::terms::SeekStatus);

impl PartialEq for SeekStatusName {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Display for SeekStatusName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use rucene::index::terms::SeekStatus;
        f.write_str(match self.0 {
            SeekStatus::FOUND => "FOUND",
            SeekStatus::NOT_FOUND => "NOT_FOUND",
            SeekStatus::END => "END",
        })
    }
}

impl LuceneTestCase {
    /// `newLogMergePolicy()`: `newLogMergePolicy(random())`.
    pub fn new_log_merge_policy() -> rucene::index::merge_policy::LogMergePolicy {
        Self::new_log_merge_policy_with(&*Self::random())
    }

    /// `newTieredMergePolicy()`: `newTieredMergePolicy(random())`.
    pub fn new_tiered_merge_policy() -> rucene::index::merge_policy::TieredMergePolicy {
        Self::new_tiered_merge_policy_with(&*Self::random())
    }

    /// `newAlcoholicMergePolicy()`: `newAlcoholicMergePolicy(random(),
    /// classEnvRule.timeZone)`.
    pub fn new_alcoholic_merge_policy() -> rucene::index::merge_policy::LogMergePolicy {
        let tz = Self::get_default_time_zone()
            .unwrap_or_else(|| exception::throw_without_message(exception::NULL_POINTER_EXCEPTION));
        Self::new_alcoholic_merge_policy_with(&*Self::random(), tz)
    }

    /// `newLogMergePolicy(useCFS)`: `newLogMergePolicy()` with
    /// `setNoCFSRatio(useCFS ? 1.0 : 0.0)`.
    pub fn new_log_merge_policy_use_cfs(
        use_cfs: bool,
    ) -> Arc<dyn rucene::index::merge_policy::MergePolicy> {
        use rucene::index::merge_policy::MergePolicy;
        let logmp = Self::new_log_merge_policy();
        logmp
            .set_no_cfs_ratio(if use_cfs { 1.0 } else { 0.0 })
            .expect("INVARIANT: 0.0 and 1.0 are valid no-CFS ratios");
        Arc::new(logmp)
    }

    /// `newLogMergePolicy(useCFS, mergeFactor)`.
    ///
    /// # Errors
    ///
    /// `setMergeFactor`'s `IllegalArgumentException` for a `mergeFactor`
    /// below 2.
    pub fn new_log_merge_policy_use_cfs_merge_factor(
        use_cfs: bool,
        merge_factor: i32,
    ) -> rucene::error::Result<rucene::index::merge_policy::LogMergePolicy> {
        use rucene::index::merge_policy::MergePolicy;
        let logmp = Self::new_log_merge_policy();
        logmp.set_no_cfs_ratio(if use_cfs { 1.0 } else { 0.0 })?;
        logmp.set_merge_factor(merge_factor)?;
        Ok(logmp)
    }

    /// `newLogMergePolicy(mergeFactor)`.
    ///
    /// # Errors
    ///
    /// `setMergeFactor`'s `IllegalArgumentException` for a `mergeFactor`
    /// below 2.
    pub fn new_log_merge_policy_merge_factor(
        merge_factor: i32,
    ) -> rucene::error::Result<rucene::index::merge_policy::LogMergePolicy> {
        let logmp = Self::new_log_merge_policy();
        logmp.set_merge_factor(merge_factor)?;
        Ok(logmp)
    }

    /// `newSnapshotIndexWriterConfig(analyzer)`: "create a new index writer
    /// config with a snapshot deletion policy" over
    /// `NoDeletionPolicy.INSTANCE`.
    pub fn new_snapshot_index_writer_config(
        analyzer: Option<Arc<dyn rucene::analysis::Analyzer>>,
    ) -> rucene::index::index_writer_config::IndexWriterConfig {
        use rucene::index::index_deletion_policy::{NoDeletionPolicy, SnapshotDeletionPolicy};
        let c = Self::new_index_writer_config_analyzer(analyzer);
        c.set_index_deletion_policy(Some(Arc::new(SnapshotDeletionPolicy::new(Some(
            NoDeletionPolicy::instance(),
        )))))
        .expect("INVARIANT: a deletion policy is never null");
        c
    }

    /// `collate(collator, s1, s2)`: "compares two strings with a collator,
    /// also looking to see if the strings are impacted by jdk bugs"
    /// (JDK-8071862): `collator.compare(s1, s2)`, after assuming that the
    /// collation keys of `s1` and `s2` compare with the same sign.
    ///
    /// The collator is the port of `java.text.RuleBasedCollator`
    /// ([`crate::jdk::collator`]), Java's only `Collator` implementation.
    ///
    /// # Panics
    ///
    /// `AssumptionViolatedException` (`hit JDK collator bug`) when the signs
    /// differ.
    pub fn collate(collator: &crate::jdk::collator::RuleBasedCollator, s1: &str, s2: &str) -> i32 {
        let v1 = collator.compare(s1, s2);
        let v2 = collator
            .get_collation_key(s1)
            .compare_to(&collator.get_collation_key(s2));
        // if collation keys don't really respect collation order, things are screwed.
        Self::assume_true("hit JDK collator bug", v1.signum() == v2.signum());
        v1
    }

    /// `ensureSaneIWCOnNightly(conf)` (protected): under `tests.nightly`,
    /// caps the merged segment size of the configuration's own merge policy,
    /// turns compound files on for flushes and keeps the no-CFS ratio at 0.25
    /// or more.
    pub fn ensure_sane_iwc_on_nightly(
        conf: &mut rucene::index::index_writer_config::IndexWriterConfig,
    ) -> &mut rucene::index::index_writer_config::IndexWriterConfig {
        if *TEST_NIGHTLY {
            Self::ensure_sane_iwc(conf);
        }
        conf
    }

    /// The body of [`ensure_sane_iwc_on_nightly`](Self::ensure_sane_iwc_on_nightly)
    /// under `if (LuceneTestCase.TEST_NIGHTLY)`, so that a test can run it
    /// whatever `tests.nightly` is.
    fn ensure_sane_iwc(conf: &mut rucene::index::index_writer_config::IndexWriterConfig) {
        use rucene::index::merge_policy::{LogMergePolicy, TieredMergePolicy};
        const INVARIANT: &str = "INVARIANT: the value is in the setter's range";
        // newIWConfig makes smallish max seg size, which
        // results in tons and tons of segments for this test
        // when run nightly:
        let mp = conf.merge_policy();
        let any = mp.as_any();
        if let Some(tmp) = any.and_then(|any| any.downcast_ref::<TieredMergePolicy>()) {
            tmp.set_max_merged_segment_mb(5000.0).expect(INVARIANT);
        } else if let Some(logmp) = any.and_then(|any| any.downcast_ref::<LogMergePolicy>()) {
            // `instanceof LogByteSizeMergePolicy`: its `toString()` starts
            // with that simple name.
            if logmp.to_string().starts_with("[LogByteSizeMergePolicy:") {
                logmp.set_max_merge_mb(1000.0);
            } else {
                logmp.set_max_merge_docs(100000);
            }
        }
        // when running nightly, merging can still have crazy parameters,
        // and might use many per-field codecs. turn on CFS for IW flushes
        // and ensure CFS ratio is reasonable to keep it contained.
        conf.set_use_compound_file(true).expect(INVARIANT);
        mp.set_no_cfs_ratio(f64::max(0.25, mp.get_no_cfs_ratio()))
            .expect(INVARIANT);
    }

    /// `closeAfterTest(resource)`: "Registers a `Closeable` resource that
    /// should be closed after the test completes." Returns `resource`.
    pub fn close_after_test<T: randomizedtesting::jdk::io::Closeable + 'static>(
        resource: Arc<T>,
    ) -> Arc<T> {
        RandomizedContext::current().close_at_end(resource, randomizedtesting::LifecycleScope::Test)
    }

    /// `closeAfterSuite(resource)`: "Registers a `Closeable` resource that
    /// should be closed after the suite completes." Returns `resource`.
    pub fn close_after_suite<T: randomizedtesting::jdk::io::Closeable + 'static>(
        resource: Arc<T>,
    ) -> Arc<T> {
        RandomizedContext::current()
            .close_at_end(resource, randomizedtesting::LifecycleScope::Suite)
    }

    /// `overrideTestDefaultQueryCache()` (`@Before`): "Make sure each test
    /// method has its own cache" — `overrideDefaultQueryCache()`.
    pub fn override_test_default_query_cache() {
        Self::override_default_query_cache();
    }

    /// `getTestName()`: "Return the name of the currently executing test
    /// case" (`threadAndTestNameRule.testMethodName`); `None` outside a test.
    pub fn get_test_name() -> Option<String> {
        Self::current_test_rules()
            .and_then(|rules| rules.thread_and_test_name_rule.test_method_name())
    }

    /// `isTestThread()` (protected): "Returns true if and only if the
    /// calling thread is the primary thread executing the test case."
    ///
    /// # Panics
    ///
    /// `AssertionError("Test case thread not set?")` outside a test.
    pub fn is_test_thread() -> bool {
        let thread = Self::current_test_rules()
            .and_then(|rules| rules.thread_and_test_name_rule.test_case_thread());
        assert_not_null(Some("Test case thread not set?"), thread.is_some());
        thread == Some(randomizedtesting::jdk::thread::Thread::current())
    }

    /// `newMaybeVirusCheckingDirectory()`: "Like `newDirectory` except
    /// randomly the `VirusCheckingFS` may be installed".
    pub fn new_maybe_virus_checking_directory() -> TestDirectory {
        if Self::random().next_int_bound(5) == 4 {
            let path = Self::add_virus_checker(&Self::create_temp_dir());
            Self::new_fs_directory(&path)
        } else {
            Self::new_directory_with(&*Self::random())
        }
    }

    /// `newDirectory(r, lf)`.
    pub fn new_directory_lock_factory(
        r: &dyn Random,
        lf: Arc<dyn rucene::store::LockFactory>,
    ) -> TestDirectory {
        let directory = Self::new_directory_impl(r, &TEST_DIRECTORY, &LockFactoryArg::Given(lf));
        let bare = Self::rarely_with(r);
        Self::wrap_directory(r, directory, bare, false)
    }

    /// `newMockDirectory()`.
    pub fn new_mock_directory() -> crate::store::MockDirectoryWrapper {
        Self::new_mock_directory_with(&*Self::random())
    }

    /// `newMockDirectory(r)`.
    pub fn new_mock_directory_with(r: &dyn Random) -> crate::store::MockDirectoryWrapper {
        let directory = Self::new_directory_impl(r, &TEST_DIRECTORY, &LockFactoryArg::Default);
        Self::into_mock(Self::wrap_directory(r, directory, false, false))
    }

    /// `newMockDirectory(r, lf)`.
    pub fn new_mock_directory_lock_factory(
        r: &dyn Random,
        lf: Arc<dyn rucene::store::LockFactory>,
    ) -> crate::store::MockDirectoryWrapper {
        let directory = Self::new_directory_impl(r, &TEST_DIRECTORY, &LockFactoryArg::Given(lf));
        Self::into_mock(Self::wrap_directory(r, directory, false, false))
    }

    /// `newMockFSDirectory(f)`: `newFSDirectory(f, FSLockFactory.getDefault(),
    /// false)`.
    pub fn new_mock_fs_directory(f: &Path) -> crate::store::MockDirectoryWrapper {
        Self::into_mock(Self::new_fs_directory_bare(
            f,
            &LockFactoryArg::Default,
            false,
        ))
    }

    /// `newMockFSDirectory(f, lf)`.
    pub fn new_mock_fs_directory_lock_factory(
        f: &Path,
        lf: Arc<dyn rucene::store::LockFactory>,
    ) -> crate::store::MockDirectoryWrapper {
        Self::into_mock(Self::new_fs_directory_bare(
            f,
            &LockFactoryArg::Given(lf),
            false,
        ))
    }

    /// The `(MockDirectoryWrapper)` cast of a wrapper built with `bare ==
    /// false`, which is always a mock.
    fn into_mock(directory: TestDirectory) -> crate::store::MockDirectoryWrapper {
        match directory {
            TestDirectory::Mock(mock) => mock,
            TestDirectory::Raw(_) => unreachable!("a wrapper built with bare == false is a mock"),
        }
    }

    /// `addVirusChecker(path)`: wraps `path` in a `VirusCheckingFS` salted
    /// with `random().nextLong()`, unless its filesystem already has one.
    pub fn add_virus_checker(path: &Path) -> Path {
        use crate::mockfile::FilterFileSystemProvider;
        if !TestUtil::has_virus_checker_path(path) {
            let fs = crate::mockfile::VirusCheckingFS::new(
                Arc::clone(path.get_file_system()),
                Self::random().next_long(),
            );
            return fs.wrap_path(path);
        }
        path.clone()
    }

    /// `newDirectory(d)`: "Returns a new Directory instance, with contents
    /// copied from the provided directory."
    ///
    /// # Errors
    ///
    /// What listing or copying `d`'s files raises.
    pub fn new_directory_copy(
        d: &dyn rucene::store::Directory,
    ) -> rucene::error::Result<TestDirectory> {
        Self::new_directory_copy_with(&*Self::random(), d)
    }

    /// `newMaybeVirusCheckingFSDirectory(f)`: "Like `newFSDirectory(Path)`,
    /// but randomly insert `VirusCheckingFS`".
    pub fn new_maybe_virus_checking_fs_directory(f: &Path) -> TestDirectory {
        let mut f = f.clone();
        if Self::random().next_int_bound(5) == 4 {
            f = Self::add_virus_checker(&f);
        }
        Self::new_fs_directory_lock_factory_arg(&f, &LockFactoryArg::Default)
    }

    /// `newFSDirectory(f, lf)`: `newFSDirectory(f, lf, rarely())`.
    pub fn new_fs_directory_lock_factory(
        f: &Path,
        lf: Arc<dyn rucene::store::LockFactory>,
    ) -> TestDirectory {
        Self::new_fs_directory_lock_factory_arg(f, &LockFactoryArg::Given(lf))
    }

    /// `newFSDirectory(f, lf)` over either kind of lock factory argument.
    fn new_fs_directory_lock_factory_arg(f: &Path, lf: &LockFactoryArg) -> TestDirectory {
        let bare = Self::rarely();
        Self::new_fs_directory_bare(f, lf, bare)
    }

    /// `newDirectory(r, d)`: "Returns a new Directory instance, using the
    /// specified random with contents copied from the provided directory":
    /// the `segments*` files and every codec file.
    ///
    /// # Errors
    ///
    /// What listing or copying `d`'s files raises.
    pub fn new_directory_copy_with(
        r: &dyn Random,
        d: &dyn rucene::store::Directory,
    ) -> rucene::error::Result<TestDirectory> {
        use rucene::index::index_file_names;
        let imp = Self::new_directory_impl(r, &TEST_DIRECTORY, &LockFactoryArg::Default);
        for file in d.list_all()? {
            if file.starts_with(index_file_names::SEGMENTS)
                || index_file_names::is_codec_file(&file)
            {
                imp.copy_from(d, &file, &file, &*Self::new_io_context(r))?;
            }
        }
        let bare = Self::rarely_with(r);
        Ok(Self::wrap_directory(r, imp, bare, false))
    }

    /// `newStringField(name, value, stored)`.
    pub fn new_string_field(
        name: &str,
        value: &str,
        stored: rucene::document::Store,
    ) -> rucene::document::Field {
        Self::new_string_field_with(&*Self::random(), name, value, stored)
    }

    /// `newStringField(name, BytesRef value, stored)`.
    pub fn new_string_field_bytes(
        name: &str,
        value: BytesRef,
        stored: rucene::document::Store,
    ) -> rucene::document::Field {
        Self::new_string_field_bytes_with(&*Self::random(), name, value, stored)
    }

    /// `newTextField(name, value, stored)`.
    pub fn new_text_field(
        name: &str,
        value: &str,
        stored: rucene::document::Store,
    ) -> rucene::document::Field {
        Self::new_text_field_with(&*Self::random(), name, value, stored)
    }

    /// `newStringField(random, name, value, stored)`.
    pub fn new_string_field_with(
        random: &dyn Random,
        name: &str,
        value: &str,
        stored: rucene::document::Store,
    ) -> rucene::document::Field {
        Self::new_field_with(
            random,
            name,
            NewFieldValue::String(value.to_string()),
            Self::string_field_type(stored),
        )
    }

    /// `newStringField(random, name, BytesRef value, stored)`.
    pub fn new_string_field_bytes_with(
        random: &dyn Random,
        name: &str,
        value: BytesRef,
        stored: rucene::document::Store,
    ) -> rucene::document::Field {
        Self::new_field_with(
            random,
            name,
            NewFieldValue::BytesRef(value),
            Self::string_field_type(stored),
        )
    }

    /// `newTextField(random, name, value, stored)`.
    pub fn new_text_field_with(
        random: &dyn Random,
        name: &str,
        value: &str,
        stored: rucene::document::Store,
    ) -> rucene::document::Field {
        use rucene::document::{Store, TextField};
        let field_type = if stored == Store::YES {
            TextField::type_stored()
        } else {
            TextField::type_not_stored()
        };
        Self::new_field_with(
            random,
            name,
            NewFieldValue::String(value.to_string()),
            field_type,
        )
    }

    /// `stored == Store.YES ? StringField.TYPE_STORED :
    /// StringField.TYPE_NOT_STORED`.
    fn string_field_type(stored: rucene::document::Store) -> &'static rucene::document::FieldType {
        use rucene::document::{Store, StringField};
        if stored == Store::YES {
            StringField::type_stored()
        } else {
            StringField::type_not_stored()
        }
    }

    /// `newField(name, value, type)`: `newField(random(), name, value,
    /// type)`.
    pub fn new_field(
        name: &str,
        value: &str,
        field_type: &rucene::document::FieldType,
    ) -> rucene::document::Field {
        Self::new_field_with(
            &*Self::random(),
            name,
            NewFieldValue::String(value.to_string()),
            field_type,
        )
    }

    /// `newField(random, name, value, type)` (`synchronized`): the field
    /// with the type chosen for `name` the first time it was asked for — a
    /// copy of `type` randomly stored, and, when indexed, randomly given term
    /// vectors with random positions, payloads and (for a `String` value)
    /// offsets. `tearDown()` forgets the choices.
    ///
    /// # Panics
    ///
    /// What `new Field(name, value, type)` throws for an invalid
    /// combination.
    pub fn new_field_with(
        random: &dyn Random,
        name: &str,
        value: NewFieldValue,
        field_type: &rucene::document::FieldType,
    ) -> rucene::document::Field {
        use rucene::index::{IndexOptions, IndexableFieldType};
        const INVARIANT: &str = "INVARIANT: the copied type is not frozen";
        // The lock is `synchronized`'s class monitor.
        let mut field_to_type = FIELD_TO_TYPE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        if let Some(prev_type) = field_to_type.get(name) {
            // always use the same fieldType for the same field name
            return Self::create_field(name, value, prev_type.clone());
        }

        let new_type = rucene::document::FieldType::new_from(field_type);
        if !new_type.stored() && random.next_boolean() {
            new_type.set_stored(true).expect(INVARIANT); // randomly store it
        }
        if new_type.index_options() != IndexOptions::NONE
            && new_type.index_options() != IndexOptions::DOCS_AND_CUSTOM_FREQS
            && !new_type.store_term_vectors()
            && random.next_boolean()
        {
            new_type.set_store_term_vectors(true).expect(INVARIANT);
            if !new_type.store_term_vector_positions() {
                new_type
                    .set_store_term_vector_positions(random.next_boolean())
                    .expect(INVARIANT);
                if new_type.store_term_vector_positions() && !new_type.store_term_vector_payloads()
                {
                    new_type
                        .set_store_term_vector_payloads(random.next_boolean())
                        .expect(INVARIANT);
                }
            }
            // Check for strings as offsets are disallowed on binary fields
            if matches!(value, NewFieldValue::String(_)) && !new_type.store_term_vector_offsets() {
                new_type
                    .set_store_term_vector_offsets(random.next_boolean())
                    .expect(INVARIANT);
            }
            if *VERBOSE {
                println!("NOTE: LuceneTestCase: upgrade name={name} type={new_type}");
            }
        }
        new_type.freeze();
        field_to_type.insert(name.to_string(), new_type.clone());

        Self::create_field(name, value, new_type)
    }

    /// `createField(name, value, fieldType)` (private).
    fn create_field(
        name: &str,
        value: NewFieldValue,
        field_type: rucene::document::FieldType,
    ) -> rucene::document::Field {
        let field = match value {
            NewFieldValue::String(value) => rucene::document::Field::new(name, value, field_type),
            NewFieldValue::BytesRef(value) => {
                rucene::document::Field::new_with_bytes(name, value, field_type)
            }
        };
        io(field)
    }

    /// `getDataPath(name)` (protected): "Gets a resource from the test's
    /// classpath as `Path`."
    ///
    /// **Divergence from Lucene 10.5.0.** Java resolves `name` with
    /// `getClass().getResource(name)`, relative to the test class's package
    /// on the classpath. Rust has no classpath: `class_resources` is the
    /// directory that stands for that package (a test passes the directory
    /// its resources live in, such as one under `CARGO_MANIFEST_DIR`), and a
    /// leading `/` makes `name` relative to the classpath root, which the
    /// port takes as `class_resources` too.
    ///
    /// # Errors
    ///
    /// `IOUtils.requireResourceNonNull`'s `FileNotFoundException("The
    /// resource '" + name + "' was not found.")` when there is no such
    /// resource.
    pub fn get_data_path(
        class_resources: &std::path::Path,
        name: &str,
    ) -> rucene::error::Result<std::path::PathBuf> {
        let path = class_resources.join(name.trim_start_matches('/'));
        rucene::util::IOUtils::require_resource_non_null(path.exists().then_some(path), name)
    }

    /// `getDataInputStream(name)` (protected): "Gets a resource from the
    /// test's classpath as `InputStream`", resolved as
    /// [`get_data_path`](Self::get_data_path) does.
    ///
    /// # Errors
    ///
    /// The `FileNotFoundException` of [`get_data_path`](Self::get_data_path),
    /// and what opening the file raises.
    pub fn get_data_input_stream(
        class_resources: &std::path::Path,
        name: &str,
    ) -> rucene::error::Result<std::fs::File> {
        let path = Self::get_data_path(class_resources, name)?;
        std::fs::File::open(path).map_err(rucene::error::LuceneError::from)
    }

    /// `runWithRestrictedPermissions(action, permissions...)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java first assumes
    /// `System.getSecurityManager() != null` and runs the action in an
    /// `AccessControlContext` restricted to `permissions`. Rust has no
    /// security manager, so the assumption always fails, as it does in Java
    /// on a JVM without one: the test is skipped with
    /// `runWithRestrictedPermissions requires a SecurityManager enabled`,
    /// and the action never runs.
    pub fn run_with_restricted_permissions<T>(_action: impl FnOnce() -> T) -> T {
        Self::assume_true(
            "runWithRestrictedPermissions requires a SecurityManager enabled",
            false,
        );
        unreachable!("assumeTrue(false) throws AssumptionViolatedException")
    }

    /// `supportsVectorEncoding(format, vectorEncoding)` (private):
    /// `HnswBitVectorsFormat` "only supports BYTE".
    ///
    /// **Divergence from Lucene 10.5.0.** Java tests `format instanceof
    /// HnswBitVectorsFormat`; a `dyn KnnVectorsFormat` has no class, so the
    /// port compares the SPI name that class (`final`) registers.
    fn supports_vector_encoding(
        format: &dyn rucene::codecs::knn_vectors::KnnVectorsFormat,
        vector_encoding: rucene::index::VectorEncoding,
    ) -> bool {
        if format.name() == "HnswBitVectorsFormat" {
            // special case, this only supports BYTE
            return vector_encoding == rucene::index::VectorEncoding::BYTE;
        }
        true
    }

    /// `supportsVectorSearch(format)` (private): `format instanceof
    /// FlatVectorsFormat == false`.
    ///
    /// **Divergence from Lucene 10.5.0.** A `dyn KnnVectorsFormat` has no
    /// class hierarchy; the port recognises the `FlatVectorsFormat`
    /// subclasses of Lucene's core and codecs modules,
    /// `Lucene99FlatVectorsFormat` and `Lucene104ScalarQuantizedVectorsFormat`,
    /// by the SPI names they register.
    fn supports_vector_search(format: &dyn rucene::codecs::knn_vectors::KnnVectorsFormat) -> bool {
        !matches!(
            format.name(),
            "Lucene99FlatVectorsFormat" | "Lucene104ScalarQuantizedVectorsFormat"
        )
    }

    /// `randomVectorFormat(vectorEncoding)` (protected): a random one of the
    /// registered KNN-vectors formats that supports `vector_encoding` and
    /// vector search, in the order of `availableKnnVectorsFormats()`.
    pub fn random_vector_format(
        vector_encoding: rucene::index::VectorEncoding,
    ) -> Arc<dyn rucene::codecs::knn_vectors::KnnVectorsFormat> {
        use rucene::codecs::knn_vectors::{available_knn_vectors_formats, knn_vectors_for_name};
        let available_formats: Vec<Arc<dyn rucene::codecs::knn_vectors::KnnVectorsFormat>> =
            available_knn_vectors_formats()
                .iter()
                .map(|name| {
                    knn_vectors_for_name(name).unwrap_or_else(|| {
                        rethrow_lucene_error(&rucene::error::LuceneError::illegal_argument(
                            format!("An SPI class of type org.apache.lucene.codecs.KnnVectorsFormat with name '{name}' does not exist."),
                        ))
                    })
                })
                .filter(|format| Self::supports_vector_encoding(format.as_ref(), vector_encoding))
                .filter(|format| Self::supports_vector_search(format.as_ref()))
                .collect();
        Arc::clone(RandomPicks::random_from(
            &*Self::random(),
            &available_formats,
        ))
    }
}

/// The UTF-16 order of `String.compareTo`.
fn java_string_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// The class name and the `toString()` of a stored field's `numericValue()`.
fn numeric_value_parts(value: &rucene::document::NumericValue) -> (&'static str, String) {
    use rucene::document::NumericValue;
    let class = match value {
        NumericValue::Int(_) => "java.lang.Integer",
        NumericValue::Long(_) => "java.lang.Long",
        NumericValue::Float(_) => "java.lang.Float",
        NumericValue::Double(_) => "java.lang.Double",
    };
    (class, value.to_string())
}

/// `Assert.assertEquals(message, Object, Object)` for two boxed numbers:
/// `Number.equals` (same class and value), and `Assert.format`'s message,
/// which adds the class names when both values print the same.
fn assert_numeric_values_equal(
    message: &str,
    expected: Option<&rucene::document::NumericValue>,
    actual: Option<&rucene::document::NumericValue>,
) {
    let parts_e = expected.map(numeric_value_parts);
    let parts_a = actual.map(numeric_value_parts);
    if parts_e == parts_a {
        return;
    }
    let prefix = if message.is_empty() {
        String::new()
    } else {
        format!("{message} ")
    };
    let show = |p: &Option<(&str, String)>| p.as_ref().map_or("null".to_string(), |p| p.1.clone());
    let (e, a) = (show(&parts_e), show(&parts_a));
    if e == a {
        let class = |p: &Option<(&'static str, String)>| p.as_ref().map_or("null", |p| p.0);
        Assert::fail(&format!(
            "{prefix}expected: {}<{e}> but was: {}<{a}>",
            class(&parts_e),
            class(&parts_a)
        ))
    }
    Assert::fail(&format!("{prefix}expected:<{e}> but was:<{a}>"))
}

/// `Assert.assertEquals(message, long, long)`.
fn assert_long_equals(message: Option<&str>, expected: i64, actual: i64) {
    Assert::assert_equals_msg(message, expected, actual)
}

/// `Assert.assertEquals(message, Object, Object)` for two `Boolean`s.
fn assert_bool_equals(message: &str, expected: bool, actual: bool) {
    Assert::assert_equals_msg(Some(message), expected, actual)
}

/// The packed values of each document of a point field, as the private
/// `uninvert` collects them: a `HashMap<Integer, Set<BytesRef>>`, kept here in
/// insertion order so that the failure message can be rendered in Java's
/// iteration order.
type UninvertedPoints = Vec<(i32, Vec<BytesRef>)>;

/// The visitor of `uninvert`: "We pretend our query shape is so hairy that it
/// crosses every single cell".
struct UninvertVisitor<'a> {
    doc_base: i32,
    doc_values: &'a mut UninvertedPoints,
}

impl rucene::index::point_values::IntersectVisitor for UninvertVisitor<'_> {
    fn visit(&mut self, _doc_id: i32) -> rucene::error::Result<()> {
        Err(rucene::error::LuceneError::unsupported_operation(
            String::new(),
        ))
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> rucene::error::Result<()> {
        let top_doc_id = self.doc_base + doc_id;
        let value = BytesRef::new(packed_value.to_vec());
        match self.doc_values.iter_mut().find(|(d, _)| *d == top_doc_id) {
            Some((_, set)) => {
                if !set.contains(&value) {
                    set.push(value);
                }
            }
            None => self.doc_values.push((top_doc_id, vec![value])),
        }
        Ok(())
    }

    fn compare(
        &self,
        _min_packed_value: &[u8],
        _max_packed_value: &[u8],
    ) -> rucene::error::Result<rucene::index::point_values::Relation> {
        Ok(rucene::index::point_values::Relation::CellCrossesQuery)
    }
}

/// `AbstractMap.toString()` of `uninvert`'s `HashMap<Integer,
/// Set<BytesRef>>` in Java's iteration order: `{doc=[bytes, ...], ...}`, each
/// set a `new HashSet<>()` ordered by `BytesRef.hashCode()`.
fn uninverted_to_string(points: &UninvertedPoints) -> String {
    let entries: Vec<(i32, &(i32, Vec<BytesRef>))> = points.iter().map(|e| (e.0, e)).collect();
    let ordered = java_hash_order(entries, java_default_table_length(points.len()));
    let rendered: Vec<String> = ordered
        .into_iter()
        .map(|(doc, set)| {
            let values: Vec<(i32, String)> =
                set.iter().map(|b| (b.hash_code(), b.to_string())).collect();
            let values = java_hash_order(values, java_default_table_length(set.len()));
            format!("{doc}=[{}]", values.join(", "))
        })
        .collect();
    format!("{{{}}}", rendered.join(", "))
}

/// `Map.equals` of two `uninvert` results.
fn uninverted_equal(left: &UninvertedPoints, right: &UninvertedPoints) -> bool {
    left.len() == right.len()
        && left.iter().all(|(doc, set)| {
            right
                .iter()
                .any(|(d, s)| d == doc && s.len() == set.len() && set.iter().all(|v| s.contains(v)))
        })
}

impl LuceneTestCase {
    /// `assertReaderEquals(info, leftReader, rightReader)`: every check
    /// below, in Java's order.
    pub fn assert_reader_equals(
        info: &str,
        left_reader: &Arc<dyn IndexReader>,
        right_reader: &Arc<dyn IndexReader>,
    ) {
        Self::assert_reader_statistics_equals(info, left_reader, right_reader);
        Self::assert_terms_equals(info, left_reader, right_reader, true);
        Self::assert_norms_equals(info, left_reader, right_reader);
        Self::assert_stored_fields_equals(info, left_reader, right_reader);
        Self::assert_term_vectors_equals(info, left_reader, right_reader);
        Self::assert_doc_values_equals(info, left_reader, right_reader);
        Self::assert_deleted_docs_equals(info, left_reader, right_reader);
        Self::assert_field_infos_equals(info, left_reader, right_reader);
        Self::assert_points_equals(info, left_reader, right_reader);
    }

    /// `assertReaderStatisticsEquals`: "checks that reader-level statistics
    /// are the same".
    pub fn assert_reader_statistics_equals(
        info: &str,
        left_reader: &Arc<dyn IndexReader>,
        right_reader: &Arc<dyn IndexReader>,
    ) {
        // Somewhat redundant: we never delete docs
        let info = Some(info);
        assert_long_equals(
            info,
            left_reader.max_doc().into(),
            right_reader.max_doc().into(),
        );
        assert_long_equals(
            info,
            left_reader.num_docs().into(),
            right_reader.num_docs().into(),
        );
        assert_long_equals(
            info,
            left_reader.num_deleted_docs().into(),
            right_reader.num_deleted_docs().into(),
        );
        Assert::assert_equals_msg(
            info,
            left_reader.has_deletions(),
            right_reader.has_deletions(),
        );
    }

    /// `assertTermsEquals(info, leftReader, rightReader, deep)`: "Fields api
    /// equivalency".
    pub fn assert_terms_equals(
        info: &str,
        left_reader: &Arc<dyn IndexReader>,
        right_reader: &Arc<dyn IndexReader>,
        deep: bool,
    ) {
        use rucene::index::multi_fields::MultiTerms;
        let left_fields = io(rucene::index::FieldInfos::get_indexed_fields(Arc::clone(
            left_reader,
        )));
        let right_fields = io(rucene::index::FieldInfos::get_indexed_fields(Arc::clone(
            right_reader,
        )));
        assert_string_sets_equal(info, &left_fields, &right_fields);

        for field in Self::java_copied_set_order(&left_fields) {
            Self::assert_terms_equals_terms(
                info,
                left_reader,
                io(MultiTerms::get_terms(left_reader, &field)).as_deref(),
                io(MultiTerms::get_terms(right_reader, &field)).as_deref(),
                deep,
            );
        }
    }

    /// The iteration order of `new HashSet<>(fields)`.
    fn java_copied_set_order(set: &HashSet<String>) -> Vec<String> {
        let mut names: Vec<&String> = set.iter().collect();
        names.sort();
        let entries = names
            .into_iter()
            .map(|n| (randomizedtesting::jdk::lang::string_hash_code(n), n.clone()))
            .collect();
        java_hash_order(entries, java_copy_table_length(set.len()))
    }

    /// `assertTermsEquals(info, leftReader, leftTerms, rightTerms, deep)`:
    /// "Terms api equivalency".
    pub fn assert_terms_equals_terms(
        info: &str,
        left_reader: &Arc<dyn IndexReader>,
        left_terms: Option<&dyn rucene::index::terms::Terms>,
        right_terms: Option<&dyn rucene::index::terms::Terms>,
        deep: bool,
    ) {
        use rucene::util::automaton::compiled_automaton::{AutomatonType, CompiledAutomaton};
        use rucene::util::automaton::operations::{Operations, DEFAULT_DETERMINIZE_WORK_LIMIT};
        use rucene::util::automaton::reg_exp::RegExp;
        let (Some(left_terms), Some(right_terms)) = (left_terms, right_terms) else {
            assert_null(Some(info), left_terms.map(|_| "Terms").as_ref());
            assert_null(Some(info), right_terms.map(|_| "Terms").as_ref());
            return;
        };
        Self::assert_terms_statistics_equals(info, left_terms, right_terms);
        assert_bool_equals(
            "hasOffsets",
            left_terms.has_offsets(),
            right_terms.has_offsets(),
        );
        assert_bool_equals(
            "hasPositions",
            left_terms.has_positions(),
            right_terms.has_positions(),
        );
        assert_bool_equals(
            "hasPayloads",
            left_terms.has_payloads(),
            right_terms.has_payloads(),
        );

        let mut left_terms_enum = io(left_terms.iterator());
        let mut right_terms_enum = io(right_terms.iterator());
        Self::assert_terms_enum_equals(
            info,
            left_reader,
            left_terms_enum.as_mut(),
            right_terms_enum.as_mut(),
            true,
        );

        Self::assert_terms_seeking_equals(info, left_terms, right_terms);

        if deep {
            let num_intersections = Self::at_least(3);
            for _ in 0..num_intersections {
                let re = crate::util::automaton::AutomatonTestUtil::random_regexp(&*Self::random());
                let a = io(io(RegExp::with_flags(&re, RegExp::NONE)).to_automaton());
                let a = Operations::determinize(&a, DEFAULT_DETERMINIZE_WORK_LIMIT)
                    .unwrap_or_else(|e| rethrow_lucene_error(&e.into()));
                let automaton = io(CompiledAutomaton::new(a, false, true, false));
                if automaton.automaton_type == AutomatonType::Normal {
                    // TODO: test start term too
                    let mut left_intersection = io(left_terms.intersect(&automaton, None));
                    let mut right_intersection = io(right_terms.intersect(&automaton, None));
                    let deep = Self::rarely();
                    Self::assert_terms_enum_equals(
                        info,
                        left_reader,
                        left_intersection.as_mut(),
                        right_intersection.as_mut(),
                        deep,
                    );
                }
            }
        }
    }

    /// `assertTermsStatisticsEquals`: "checks collection-level statistics on
    /// Terms".
    pub fn assert_terms_statistics_equals(
        info: &str,
        left_terms: &dyn rucene::index::terms::Terms,
        right_terms: &dyn rucene::index::terms::Terms,
    ) {
        let info = Some(info);
        assert_long_equals(
            info,
            io(left_terms.doc_count()).into(),
            io(right_terms.doc_count()).into(),
        );
        assert_long_equals(
            info,
            io(left_terms.sum_doc_freq()),
            io(right_terms.sum_doc_freq()),
        );
        assert_long_equals(
            info,
            io(left_terms.sum_total_term_freq()),
            io(right_terms.sum_total_term_freq()),
        );
        let (left_size, right_size) = (io(left_terms.size()), io(right_terms.size()));
        if left_size != -1 && right_size != -1 {
            assert_long_equals(info, left_size, right_size);
        }
    }

    /// `assertTermsEnumEquals`: "checks the terms enum sequentially if deep is
    /// false, it does a 'shallow' test that doesnt go down to the docsenums".
    pub fn assert_terms_enum_equals(
        info: &str,
        left_reader: &Arc<dyn IndexReader>,
        left_terms_enum: &mut dyn rucene::index::terms::TermsEnum,
        right_terms_enum: &mut dyn rucene::index::terms::TermsEnum,
        deep: bool,
    ) {
        use rucene::index::postings_enum::{
            POSTINGS_ENUM_ALL, POSTINGS_ENUM_FREQS, POSTINGS_ENUM_NONE,
        };
        let mut left_positions = None;
        let mut right_positions = None;
        let mut left_docs = None;
        let mut right_docs = None;

        while let Some(term) = io(left_terms_enum.next()) {
            let right_term = io(right_terms_enum.next());
            assert_option_equals(Some(info), Some(&term), right_term.as_ref());
            Self::assert_term_stats_equals(info, left_terms_enum, right_terms_enum);
            if deep {
                let mut lp = io(left_terms_enum.postings(left_positions.take(), POSTINGS_ENUM_ALL));
                let mut rp =
                    io(right_terms_enum.postings(right_positions.take(), POSTINGS_ENUM_ALL));
                Self::assert_docs_and_positions_enum_equals(
                    info,
                    lp.as_mut()
                        .map(|p| &mut **p as &mut dyn rucene::index::postings_enum::PostingsEnum),
                    rp.as_mut()
                        .map(|p| &mut **p as &mut dyn rucene::index::postings_enum::PostingsEnum),
                );

                let doc_freq = io(left_terms_enum.doc_freq());
                let mut lp = io(left_terms_enum.postings(lp, POSTINGS_ENUM_ALL));
                let mut rp = io(right_terms_enum.postings(rp, POSTINGS_ENUM_ALL));
                Self::assert_positions_skipping_equals(
                    info,
                    left_reader,
                    doc_freq,
                    lp.as_mut()
                        .map(|p| &mut **p as &mut dyn rucene::index::postings_enum::PostingsEnum),
                    rp.as_mut()
                        .map(|p| &mut **p as &mut dyn rucene::index::postings_enum::PostingsEnum),
                );
                left_positions = lp;
                right_positions = rp;

                // with freqs:
                let mut ld = io(left_terms_enum.postings(left_docs.take(), POSTINGS_ENUM_FREQS));
                let mut rd = io(right_terms_enum.postings(right_docs.take(), POSTINGS_ENUM_FREQS));
                Self::assert_docs_enum_equals(
                    info,
                    ld.as_mut()
                        .map(|p| &mut **p as &mut dyn rucene::index::postings_enum::PostingsEnum),
                    rd.as_mut()
                        .map(|p| &mut **p as &mut dyn rucene::index::postings_enum::PostingsEnum),
                    true,
                );

                // w/o freqs:
                let mut ld = io(left_terms_enum.postings(ld, POSTINGS_ENUM_NONE));
                let mut rd = io(right_terms_enum.postings(rd, POSTINGS_ENUM_NONE));
                Self::assert_docs_enum_equals(
                    info,
                    ld.as_mut()
                        .map(|p| &mut **p as &mut dyn rucene::index::postings_enum::PostingsEnum),
                    rd.as_mut()
                        .map(|p| &mut **p as &mut dyn rucene::index::postings_enum::PostingsEnum),
                    false,
                );

                // with freqs:
                let doc_freq = io(left_terms_enum.doc_freq());
                let mut ld = io(left_terms_enum.postings(ld, POSTINGS_ENUM_FREQS));
                let mut rd = io(right_terms_enum.postings(rd, POSTINGS_ENUM_FREQS));
                Self::assert_docs_skipping_equals(
                    info,
                    left_reader,
                    doc_freq,
                    ld.as_mut()
                        .map(|p| &mut **p as &mut dyn rucene::index::postings_enum::PostingsEnum),
                    rd.as_mut()
                        .map(|p| &mut **p as &mut dyn rucene::index::postings_enum::PostingsEnum),
                    true,
                );

                // w/o freqs:
                let doc_freq = io(left_terms_enum.doc_freq());
                let mut ld = io(left_terms_enum.postings(ld, POSTINGS_ENUM_NONE));
                let mut rd = io(right_terms_enum.postings(rd, POSTINGS_ENUM_NONE));
                Self::assert_docs_skipping_equals(
                    info,
                    left_reader,
                    doc_freq,
                    ld.as_mut()
                        .map(|p| &mut **p as &mut dyn rucene::index::postings_enum::PostingsEnum),
                    rd.as_mut()
                        .map(|p| &mut **p as &mut dyn rucene::index::postings_enum::PostingsEnum),
                    false,
                );
                left_docs = ld;
                right_docs = rd;
            }
        }
        assert_null(Some(info), io(right_terms_enum.next()).as_ref());
    }

    /// `assertDocsAndPositionsEnumEquals`: "checks docs + freqs + positions
    /// + payloads, sequentially".
    pub fn assert_docs_and_positions_enum_equals(
        info: &str,
        left_docs: Option<&mut dyn rucene::index::postings_enum::PostingsEnum>,
        right_docs: Option<&mut dyn rucene::index::postings_enum::PostingsEnum>,
    ) {
        use rucene::search::NO_MORE_DOCS;
        assert_not_null(None, left_docs.is_some());
        assert_not_null(None, right_docs.is_some());
        let (Some(left_docs), Some(right_docs)) = (left_docs, right_docs) else {
            unreachable!("both are present")
        };
        let i = Some(info);
        assert_long_equals(i, -1, left_docs.doc_id().into());
        assert_long_equals(i, -1, right_docs.doc_id().into());
        loop {
            let docid = io(left_docs.next_doc());
            if docid == NO_MORE_DOCS {
                break;
            }
            assert_long_equals(i, docid.into(), io(right_docs.next_doc()).into());
            let freq = io(left_docs.freq());
            assert_long_equals(i, freq.into(), io(right_docs.freq()).into());
            for _ in 0..freq {
                assert_long_equals(
                    i,
                    io(left_docs.next_position()).into(),
                    io(right_docs.next_position()).into(),
                );
                assert_option_equals(
                    i,
                    payload_of(left_docs).as_ref(),
                    payload_of(right_docs).as_ref(),
                );
                assert_long_equals(
                    i,
                    left_docs.start_offset().into(),
                    right_docs.start_offset().into(),
                );
                assert_long_equals(
                    i,
                    left_docs.end_offset().into(),
                    right_docs.end_offset().into(),
                );
            }
        }
        assert_long_equals(i, NO_MORE_DOCS.into(), io(right_docs.next_doc()).into());
    }

    /// `assertDocsEnumEquals`: "checks docs + freqs, sequentially".
    pub fn assert_docs_enum_equals(
        info: &str,
        left_docs: Option<&mut dyn rucene::index::postings_enum::PostingsEnum>,
        right_docs: Option<&mut dyn rucene::index::postings_enum::PostingsEnum>,
        has_freqs: bool,
    ) {
        use rucene::search::NO_MORE_DOCS;
        let Some(left_docs) = left_docs else {
            assert_null(None, right_docs.map(|d| d.to_string()).as_ref());
            return;
        };
        let right_docs = right_docs
            .unwrap_or_else(|| exception::throw_without_message(exception::NULL_POINTER_EXCEPTION));
        let i = Some(info);
        assert_long_equals(i, -1, left_docs.doc_id().into());
        assert_long_equals(i, -1, right_docs.doc_id().into());
        loop {
            let docid = io(left_docs.next_doc());
            if docid == NO_MORE_DOCS {
                break;
            }
            assert_long_equals(i, docid.into(), io(right_docs.next_doc()).into());
            if has_freqs {
                assert_long_equals(i, io(left_docs.freq()).into(), io(right_docs.freq()).into());
            }
        }
        assert_long_equals(i, NO_MORE_DOCS.into(), io(right_docs.next_doc()).into());
    }

    /// The `advance` target of `assertDocsSkippingEquals` and
    /// `assertPositionsSkippingEquals`: `docid + (int)
    /// Math.ceil(Math.abs(skipInterval + random().nextGaussian() *
    /// averageGap))`.
    fn skip_target(docid: i32, average_gap: i32) -> i32 {
        let skip_interval = 16;
        let gap = (f64::from(skip_interval)
            + Self::random().next_gaussian() * f64::from(average_gap))
        .abs()
        .ceil() as i32;
        docid.wrapping_add(gap)
    }

    /// `assertDocsSkippingEquals`: "checks advancing docs".
    pub fn assert_docs_skipping_equals(
        info: &str,
        left_reader: &Arc<dyn IndexReader>,
        doc_freq: i32,
        left_docs: Option<&mut dyn rucene::index::postings_enum::PostingsEnum>,
        right_docs: Option<&mut dyn rucene::index::postings_enum::PostingsEnum>,
        has_freqs: bool,
    ) {
        use rucene::search::NO_MORE_DOCS;
        let Some(left_docs) = left_docs else {
            assert_null(None, right_docs.map(|d| d.to_string()).as_ref());
            return;
        };
        let right_docs = right_docs
            .unwrap_or_else(|| exception::throw_without_message(exception::NULL_POINTER_EXCEPTION));
        let i = Some(info);
        let mut docid;
        let average_gap = left_reader.max_doc() / (1 + doc_freq);
        docid = -1;
        loop {
            if Self::random().next_boolean() {
                // nextDoc()
                docid = io(left_docs.next_doc());
                assert_long_equals(i, docid.into(), io(right_docs.next_doc()).into());
            } else {
                // advance()
                let skip = Self::skip_target(docid, average_gap);
                docid = io(left_docs.advance(skip));
                assert_long_equals(i, docid.into(), io(right_docs.advance(skip)).into());
            }

            if docid == NO_MORE_DOCS {
                return;
            }
            if has_freqs {
                assert_long_equals(i, io(left_docs.freq()).into(), io(right_docs.freq()).into());
            }
        }
    }

    /// `assertPositionsSkippingEquals`: "checks advancing docs + positions".
    pub fn assert_positions_skipping_equals(
        info: &str,
        left_reader: &Arc<dyn IndexReader>,
        doc_freq: i32,
        left_docs: Option<&mut dyn rucene::index::postings_enum::PostingsEnum>,
        right_docs: Option<&mut dyn rucene::index::postings_enum::PostingsEnum>,
    ) {
        use rucene::search::NO_MORE_DOCS;
        let (Some(left_docs), Some(right_docs)) = (left_docs, right_docs) else {
            // `assertNull(leftDocs); assertNull(rightDocs);`: one side is
            // null, so at most one of them fails.
            Assert::fail("expected null, but was:<PostingsEnum>")
        };
        let i = Some(info);
        let mut docid = -1;
        let average_gap = left_reader.max_doc() / (1 + doc_freq);
        loop {
            if Self::random().next_boolean() {
                // nextDoc()
                docid = io(left_docs.next_doc());
                assert_long_equals(i, docid.into(), io(right_docs.next_doc()).into());
            } else {
                // advance()
                let skip = Self::skip_target(docid, average_gap);
                docid = io(left_docs.advance(skip));
                assert_long_equals(i, docid.into(), io(right_docs.advance(skip)).into());
            }

            if docid == NO_MORE_DOCS {
                return;
            }
            let freq = io(left_docs.freq());
            assert_long_equals(i, freq.into(), io(right_docs.freq()).into());
            for _ in 0..freq {
                assert_long_equals(
                    i,
                    io(left_docs.next_position()).into(),
                    io(right_docs.next_position()).into(),
                );
                assert_option_equals(
                    i,
                    payload_of(left_docs).as_ref(),
                    payload_of(right_docs).as_ref(),
                );
            }
        }
    }

    /// `assertTermsSeekingEquals` (private): seeks both sides to terms drawn
    /// from the left side, truncations of them, offset copies, and terms
    /// before the first, past the last and random.
    fn assert_terms_seeking_equals(
        info: &str,
        left_terms: &dyn rucene::index::terms::Terms,
        right_terms: &dyn rucene::index::terms::Terms,
    ) {
        use rucene::index::terms::SeekStatus;
        // just an upper bound
        let num_tests = Self::at_least(20);
        let random = Self::random();

        let mut left_enum = None;

        // collect this number of terms from the left side; a `HashSet`,
        // kept in insertion order and re-ordered as Java iterates it below.
        let mut tests: Vec<BytesRef> = Vec::new();
        let add = |tests: &mut Vec<BytesRef>, b: BytesRef| {
            if !tests.contains(&b) {
                tests.push(b);
            }
        };
        let mut num_passes = 0;
        while num_passes < 10 && (tests.len() as i32) < num_tests {
            let mut e = io(left_terms.iterator());
            while let Some(term) = io(e.next()) {
                let code = random.next_int_bound(10);
                if code == 0 {
                    // the term
                    add(&mut tests, BytesRef::deep_copy_of(&term));
                } else if code == 1 {
                    // truncated subsequence of term
                    let mut term = BytesRef::deep_copy_of(&term);
                    if term.length > 0 {
                        // truncate it
                        term.length = random.next_int_bound(term.length as i32) as usize;
                    }
                } else if code == 2 {
                    // term, but ensure a non-zero offset
                    let mut newbytes = vec![0u8; term.length + 5];
                    newbytes[5..5 + term.length]
                        .copy_from_slice(&term.bytes[term.offset..term.offset + term.length]);
                    let mut b = BytesRef::new(newbytes);
                    b.offset = 5;
                    b.length = term.length;
                    add(&mut tests, b);
                } else if code == 3 {
                    match Self::random().next_int_bound(3) {
                        0 => add(&mut tests, BytesRef::new(Vec::new())), // before the first term
                        1 => add(&mut tests, BytesRef::new(vec![0xFF, 0xFF])), // past the last term
                        2 => add(
                            &mut tests,
                            BytesRef::from_char_sequence(&TestUtil::random_simple_string(
                                &*Self::random(),
                            )),
                        ), // random term
                        _ => exception::throw_without_message(exception::ASSERTION_ERROR),
                    }
                }
            }
            left_enum = Some(e);
            num_passes += 1;
        }

        let mut right_enum = io(right_terms.iterator());

        // `new ArrayList<>(tests)`: the `HashSet`'s iteration order.
        let entries = tests
            .into_iter()
            .map(|b| (b.hash_code(), b))
            .collect::<Vec<_>>();
        let len = entries.len();
        let mut shuffled_tests = java_hash_order(entries, java_default_table_length(len));
        randomizedtesting::jdk::util::shuffle(&mut shuffled_tests, &*random);

        let mut left_enum = left_enum
            .unwrap_or_else(|| exception::throw_without_message(exception::NULL_POINTER_EXCEPTION));
        let i = Some(info);
        for b in &shuffled_tests {
            if Self::rarely() {
                // make new enums
                left_enum = io(left_terms.iterator());
                right_enum = io(right_terms.iterator());
            }

            let seek_exact = Self::random().next_boolean();

            if seek_exact {
                Assert::assert_equals_msg(
                    i,
                    io(left_enum.seek_exact(b)),
                    io(right_enum.seek_exact(b)),
                );
            } else {
                let left_status = io(left_enum.seek_ceil(b));
                let right_status = io(right_enum.seek_ceil(b));
                Assert::assert_equals_msg(
                    i,
                    SeekStatusName(left_status),
                    SeekStatusName(right_status),
                );
                if left_status != SeekStatus::END {
                    assert_option_equals(
                        i,
                        io(left_enum.term()).as_ref(),
                        io(right_enum.term()).as_ref(),
                    );
                    Self::assert_term_stats_equals(info, left_enum.as_mut(), right_enum.as_mut());
                }
            }
        }
    }

    /// `assertTermStatsEquals`: "checks term-level statistics".
    pub fn assert_term_stats_equals(
        info: &str,
        left_terms_enum: &mut dyn rucene::index::terms::TermsEnum,
        right_terms_enum: &mut dyn rucene::index::terms::TermsEnum,
    ) {
        let i = Some(info);
        assert_long_equals(
            i,
            io(left_terms_enum.doc_freq()).into(),
            io(right_terms_enum.doc_freq()).into(),
        );
        assert_long_equals(
            i,
            io(left_terms_enum.total_term_freq()),
            io(right_terms_enum.total_term_freq()),
        );
    }

    /// `assertNormsEquals`: "checks that norms are the same across all
    /// fields".
    pub fn assert_norms_equals(
        info: &str,
        left_reader: &Arc<dyn IndexReader>,
        right_reader: &Arc<dyn IndexReader>,
    ) {
        use rucene::index::multi_doc_values::MultiDocValues;
        let left_fields = io(rucene::index::FieldInfos::get_indexed_fields(Arc::clone(
            left_reader,
        )));
        let right_fields = io(rucene::index::FieldInfos::get_indexed_fields(Arc::clone(
            right_reader,
        )));
        assert_string_sets_equal(info, &left_fields, &right_fields);

        for field in Self::java_copied_set_order(&left_fields) {
            let left_norms = io(MultiDocValues::get_norm_values(left_reader, &field));
            let right_norms = io(MultiDocValues::get_norm_values(right_reader, &field));
            match (left_norms, right_norms) {
                (Some(mut l), Some(mut r)) => Self::assert_numeric_doc_values_equals(
                    info,
                    left_reader.max_doc(),
                    Some(l.as_mut()),
                    Some(r.as_mut()),
                ),
                (l, r) => {
                    assert_null(Some(info), l.map(|_| "NumericDocValues").as_ref());
                    assert_null(Some(info), r.map(|_| "NumericDocValues").as_ref());
                }
            }
        }
    }

    /// `assertStoredFieldsEquals`: "checks that stored fields of all documents
    /// are the same", each document's fields sorted (stably) by name.
    pub fn assert_stored_fields_equals(
        info: &str,
        left_reader: &Arc<dyn IndexReader>,
        right_reader: &Arc<dyn IndexReader>,
    ) {
        exception::java_assert_no_message(left_reader.max_doc() == right_reader.max_doc());
        let left_stored_fields = io(left_reader.stored_fields());
        let right_stored_fields = io(right_reader.stored_fields());
        for i in 0..left_reader.max_doc() {
            let left_doc = io(left_stored_fields.document(i));
            let right_doc = io(right_stored_fields.document(i));

            // TODO: I think this is bogus because we don't document what the order should be
            // from these iterators, etc. I think the codec/IndexReader should be free to order this stuff
            // in whatever way it wants (e.g. maybe it packs related fields together or something)
            // To fix this, we sort the fields in both documents by name, but
            // we still assume that all instances with same name are in order:
            let mut left_fields: Vec<&dyn rucene::index::IndexableField> =
                left_doc.get_fields().iter().map(|f| f.as_ref()).collect();
            let mut right_fields: Vec<&dyn rucene::index::IndexableField> =
                right_doc.get_fields().iter().map(|f| f.as_ref()).collect();
            left_fields.sort_by(|a, b| java_string_cmp(a.name(), b.name()));
            right_fields.sort_by(|a, b| java_string_cmp(a.name(), b.name()));

            let mut right_iterator = right_fields.into_iter();
            for left in left_fields {
                let right = right_iterator.next();
                Assert::assert_true_msg(info, right.is_some());
                Self::assert_stored_field_equals(info, left, right.expect("checked above"));
            }
            Assert::assert_false_msg(info, right_iterator.next().is_some());
        }
    }

    /// `assertStoredFieldEquals`: "checks that two stored fields are
    /// equivalent" — name, binary, string and numeric values.
    pub fn assert_stored_field_equals(
        info: &str,
        left_field: &dyn rucene::index::IndexableField,
        right_field: &dyn rucene::index::IndexableField,
    ) {
        Assert::assert_equals_str(
            Some(info),
            Some(left_field.name()),
            Some(right_field.name()),
        );
        assert_option_equals(
            Some(info),
            left_field.binary_value().as_ref(),
            right_field.binary_value().as_ref(),
        );
        Assert::assert_equals_str(
            Some(info),
            left_field.string_value().as_deref(),
            right_field.string_value().as_deref(),
        );
        assert_numeric_values_equal(
            info,
            left_field.numeric_value().as_ref(),
            right_field.numeric_value().as_ref(),
        );
        // TODO: should we check the FT at all?
    }

    /// `assertTermVectorsEquals`: "checks that term vectors across all fields
    /// are equivalent". As in Java, the check stops at the first document
    /// whose vectors are null on both sides.
    pub fn assert_term_vectors_equals(
        info: &str,
        left_reader: &Arc<dyn IndexReader>,
        right_reader: &Arc<dyn IndexReader>,
    ) {
        exception::java_assert_no_message(left_reader.max_doc() == right_reader.max_doc());
        let left_vectors = io(left_reader.term_vectors());
        let right_vectors = io(right_reader.term_vectors());
        for i in 0..left_reader.max_doc() {
            let left_fields = io(left_vectors.get(i));
            let right_fields = io(right_vectors.get(i));

            // Fields could be null if there are no postings,
            // but then it must be null for both
            let (Some(left_fields), Some(right_fields)) = (&left_fields, &right_fields) else {
                assert_null(Some(info), left_fields.as_ref().map(|_| "Fields").as_ref());
                assert_null(Some(info), right_fields.as_ref().map(|_| "Fields").as_ref());
                return;
            };
            if left_fields.size() != -1 && right_fields.size() != -1 {
                assert_long_equals(
                    Some(info),
                    left_fields.size().into(),
                    right_fields.size().into(),
                );
            }

            let mut right_enum = right_fields.iterator();
            for field in left_fields.iterator() {
                Assert::assert_equals_str(Some(info), Some(&field), right_enum.next().as_deref());
                let left_terms = io(left_fields.terms(&field));
                let right_terms = io(right_fields.terms(&field));
                let deep = Self::rarely();
                Self::assert_terms_equals_terms(
                    info,
                    left_reader,
                    left_terms.as_deref(),
                    right_terms.as_deref(),
                    deep,
                );
            }
            Assert::assert_false(right_enum.next().is_some());
        }
    }

    /// `getDVFields(reader)` (private): the fields with doc values, as a
    /// `new HashSet<>()` filled in field-info order.
    fn get_dv_fields(reader: &Arc<dyn IndexReader>) -> HashSet<String> {
        let mut fields = HashSet::new();
        for fi in io(rucene::index::FieldInfos::get_merged_field_infos(
            Arc::clone(reader),
        ))
        .iter()
        {
            if fi.get_doc_values_type() != rucene::index::DocValuesType::NONE {
                fields.insert(fi.name.clone());
            }
        }
        fields
    }

    /// `assertDocValuesEquals(info, leftReader, rightReader)`: "checks that
    /// docvalues across all fields are equivalent".
    pub fn assert_doc_values_equals(
        info: &str,
        left_reader: &Arc<dyn IndexReader>,
        right_reader: &Arc<dyn IndexReader>,
    ) {
        use rucene::index::multi_doc_values::MultiDocValues;
        use rucene::search::NO_MORE_DOCS;
        let left_fields = Self::get_dv_fields(left_reader);
        let right_fields = Self::get_dv_fields(right_reader);
        assert_string_sets_equal(info, &left_fields, &right_fields);
        let i = Some(info);

        let entries: Vec<(i32, String)> = {
            // `getDVFields` fills a `new HashSet<>()` element by element.
            let mut names: Vec<&String> = left_fields.iter().collect();
            names.sort();
            names
                .into_iter()
                .map(|n| (randomizedtesting::jdk::lang::string_hash_code(n), n.clone()))
                .collect()
        };
        let len = entries.len();
        for field in java_hash_order(entries, java_default_table_length(len)) {
            // TODO: clean this up... very messy
            {
                let left_values = io(MultiDocValues::get_numeric_values(left_reader, &field));
                let right_values = io(MultiDocValues::get_numeric_values(right_reader, &field));
                match (left_values, right_values) {
                    (Some(mut l), Some(mut r)) => Self::assert_numeric_doc_values_equals(
                        info,
                        left_reader.max_doc(),
                        Some(l.as_mut()),
                        Some(r.as_mut()),
                    ),
                    (l, r) => {
                        Assert::assert_true_msg(
                            &format!("{info}: left numeric doc values for field=\"{field}\" are not null"),
                            l.is_none_or(|mut l| io(l.next_doc()) == NO_MORE_DOCS),
                        );
                        Assert::assert_true_msg(
                            &format!("{info}: right numeric doc values for field=\"{field}\" are not null"),
                            r.is_none_or(|mut r| io(r.next_doc()) == NO_MORE_DOCS),
                        );
                    }
                }
            }

            {
                let left_values = io(MultiDocValues::get_binary_values(left_reader, &field));
                let right_values = io(MultiDocValues::get_binary_values(right_reader, &field));
                match (left_values, right_values) {
                    (Some(mut l), Some(mut r)) => loop {
                        let doc_id = io(l.next_doc());
                        assert_long_equals(None, doc_id.into(), io(r.next_doc()).into());
                        if doc_id == NO_MORE_DOCS {
                            break;
                        }
                        Assert::assert_equals(io(l.binary_value()), io(r.binary_value()));
                    },
                    (l, r) => {
                        Assert::assert_true_msg(
                            info,
                            l.is_none_or(|mut l| io(l.next_doc()) == NO_MORE_DOCS),
                        );
                        Assert::assert_true_msg(
                            info,
                            r.is_none_or(|mut r| io(r.next_doc()) == NO_MORE_DOCS),
                        );
                    }
                }
            }

            {
                let left_values = io(MultiDocValues::get_sorted_values(left_reader, &field));
                let right_values = io(MultiDocValues::get_sorted_values(right_reader, &field));
                match (left_values, right_values) {
                    (Some(mut l), Some(mut r)) => {
                        // numOrds
                        let value_count = io(l.get_value_count());
                        assert_long_equals(i, value_count.into(), io(r.get_value_count()).into());
                        // ords
                        for ord in 0..value_count {
                            let left = BytesRef::deep_copy_of(&io(l.lookup_ord(ord)));
                            let right = io(r.lookup_ord(ord));
                            Assert::assert_equals_msg(i, left, right);
                        }
                        // bytes
                        loop {
                            let doc_id = io(l.next_doc());
                            assert_long_equals(None, doc_id.into(), io(r.next_doc()).into());
                            if doc_id == NO_MORE_DOCS {
                                break;
                            }
                            let left = BytesRef::deep_copy_of(&io(l.lookup_ord(io(l.ord_value()))));
                            let right = io(r.lookup_ord(io(r.ord_value())));
                            Assert::assert_equals_msg(i, left, right);
                        }
                    }
                    (l, r) => {
                        assert_null(i, l.map(|_| "SortedDocValues").as_ref());
                        assert_null(i, r.map(|_| "SortedDocValues").as_ref());
                    }
                }
            }

            {
                let left_values = io(MultiDocValues::get_sorted_set_values(left_reader, &field));
                let right_values = io(MultiDocValues::get_sorted_set_values(right_reader, &field));
                match (left_values, right_values) {
                    (Some(mut l), Some(mut r)) => {
                        // numOrds
                        let value_count = io(l.get_value_count());
                        assert_long_equals(i, value_count, io(r.get_value_count()));
                        // ords
                        for ord in 0..value_count {
                            let left = BytesRef::deep_copy_of(&io(l.lookup_ord(ord)));
                            let right = io(r.lookup_ord(ord));
                            Assert::assert_equals_msg(i, left, right);
                        }
                        // ord lists
                        loop {
                            let doc_id = io(l.next_doc());
                            assert_long_equals(None, doc_id.into(), io(r.next_doc()).into());
                            if doc_id == NO_MORE_DOCS {
                                break;
                            }
                            let count = io(l.doc_value_count());
                            assert_long_equals(i, count.into(), io(r.doc_value_count()).into());
                            for _ in 0..count {
                                assert_long_equals(i, io(l.next_ord()), io(r.next_ord()));
                            }
                        }
                    }
                    (l, r) => {
                        assert_null(i, l.map(|_| "SortedSetDocValues").as_ref());
                        assert_null(i, r.map(|_| "SortedSetDocValues").as_ref());
                    }
                }
            }

            {
                let left_values = io(MultiDocValues::get_sorted_numeric_values(
                    left_reader,
                    &field,
                ));
                let right_values = io(MultiDocValues::get_sorted_numeric_values(
                    right_reader,
                    &field,
                ));
                match (left_values, right_values) {
                    (Some(mut l), Some(mut r)) => loop {
                        let doc_id = io(l.next_doc());
                        assert_long_equals(None, doc_id.into(), io(r.next_doc()).into());
                        if doc_id == NO_MORE_DOCS {
                            break;
                        }
                        let count = io(l.doc_value_count());
                        assert_long_equals(i, count.into(), io(r.doc_value_count()).into());
                        for _ in 0..count {
                            assert_long_equals(i, io(l.next_value()), io(r.next_value()));
                        }
                    },
                    (l, r) => {
                        assert_null(i, l.map(|_| "SortedNumericDocValues").as_ref());
                        assert_null(i, r.map(|_| "SortedNumericDocValues").as_ref());
                    }
                }
            }
        }
    }

    /// `assertDocValuesEquals(info, num, leftDocValues, rightDocValues)`.
    pub fn assert_numeric_doc_values_equals(
        info: &str,
        _num: i32,
        left_doc_values: Option<&mut dyn rucene::index::doc_values::NumericDocValues>,
        right_doc_values: Option<&mut dyn rucene::index::doc_values::NumericDocValues>,
    ) {
        use rucene::search::NO_MORE_DOCS;
        assert_not_null(Some(info), left_doc_values.is_some());
        assert_not_null(Some(info), right_doc_values.is_some());
        let (Some(left), Some(right)) = (left_doc_values, right_doc_values) else {
            unreachable!("both are present")
        };
        loop {
            let left_doc_id = io(left.next_doc());
            let right_doc_id = io(right.next_doc());
            assert_long_equals(None, left_doc_id.into(), right_doc_id.into());
            if left_doc_id == NO_MORE_DOCS {
                return;
            }
            assert_long_equals(None, io(left.long_value()), io(right.long_value()));
        }
    }

    /// `assertDeletedDocsEquals`: the live docs of both readers, bit by bit.
    pub fn assert_deleted_docs_equals(
        info: &str,
        left_reader: &Arc<dyn IndexReader>,
        right_reader: &Arc<dyn IndexReader>,
    ) {
        use rucene::index::multi_bits::MultiBits;
        exception::java_assert_no_message(
            left_reader.num_deleted_docs() == right_reader.num_deleted_docs(),
        );
        let left_bits = io(MultiBits::get_live_docs(left_reader));
        let right_bits = io(MultiBits::get_live_docs(right_reader));

        let (Some(left_bits), Some(right_bits)) = (&left_bits, &right_bits) else {
            assert_null(
                Some(info),
                left_bits.as_ref().map(|b| format!("{b:?}")).as_ref(),
            );
            assert_null(
                Some(info),
                right_bits.as_ref().map(|b| format!("{b:?}")).as_ref(),
            );
            return;
        };

        exception::java_assert_no_message(left_reader.max_doc() == right_reader.max_doc());
        assert_long_equals(
            Some(info),
            left_bits.length() as i64,
            right_bits.length() as i64,
        );
        for i in 0..left_reader.max_doc() as usize {
            Assert::assert_equals_msg(Some(info), left_bits.get(i), right_bits.get(i));
        }
    }

    /// `assertFieldInfosEquals`: the names of the merged field infos, as two
    /// `TreeSet`s.
    pub fn assert_field_infos_equals(
        info: &str,
        left_reader: &Arc<dyn IndexReader>,
        right_reader: &Arc<dyn IndexReader>,
    ) {
        let left_infos = io(rucene::index::FieldInfos::get_merged_field_infos(
            Arc::clone(left_reader),
        ));
        let right_infos = io(rucene::index::FieldInfos::get_merged_field_infos(
            Arc::clone(right_reader),
        ));

        // TODO: would be great to verify more than just the names of the fields!
        let tree_set = |infos: &rucene::index::FieldInfos| {
            let mut names: Vec<String> = infos.iter().map(|fi| fi.name.clone()).collect();
            names.sort_by(|a, b| java_string_cmp(a, b));
            names.dedup();
            names
        };
        let left = tree_set(&left_infos);
        let right = tree_set(&right_infos);
        if left != right {
            Assert::fail(&format!(
                "{info} expected:<[{}]> but was:<[{}]>",
                left.join(", "),
                right.join(", ")
            ));
        }
    }

    /// `uninvert(fieldName, reader)` (private): "naive silly memory heavy
    /// uninversion!! maps docID -> packed values".
    fn uninvert(field_name: &str, reader: &Arc<dyn IndexReader>) -> UninvertedPoints {
        let mut doc_values = UninvertedPoints::new();
        for ctx in io(Arc::clone(reader).leaves()) {
            let Some(points) = io(ctx.leaf_reader().get_point_values(field_name)) else {
                continue;
            };
            io(points.intersect(&mut UninvertVisitor {
                doc_base: ctx.doc_base(),
                doc_values: &mut doc_values,
            }));
        }
        doc_values
    }

    /// `assertPointsEquals`: the point dimensions of every point field, and
    /// the uninverted values of each field of the left reader.
    ///
    /// As in Java, the first loop compares the right reader's dimensions with
    /// themselves.
    pub fn assert_points_equals(
        info: &str,
        left_reader: &Arc<dyn IndexReader>,
        right_reader: &Arc<dyn IndexReader>,
    ) {
        let field_infos1 = io(rucene::index::FieldInfos::get_merged_field_infos(
            Arc::clone(left_reader),
        ));
        let field_infos2 = io(rucene::index::FieldInfos::get_merged_field_infos(
            Arc::clone(right_reader),
        ));
        let i = Some(info);
        let npe = || -> &rucene::index::FieldInfo {
            exception::throw_without_message(exception::NULL_POINTER_EXCEPTION)
        };
        for field_info1 in field_infos1.iter() {
            if field_info1.get_point_dimension_count() != 0 {
                let field_info2 = field_infos2
                    .field_info(&field_info1.name)
                    .unwrap_or_else(npe);
                // same data dimension count?
                assert_long_equals(
                    i,
                    field_info2.get_point_dimension_count().into(),
                    field_info2.get_point_dimension_count().into(),
                );
                // same index dimension count?
                assert_long_equals(
                    i,
                    field_info2.get_point_index_dimension_count().into(),
                    field_info2.get_point_index_dimension_count().into(),
                );
                // same bytes per dimension?
                assert_long_equals(
                    i,
                    field_info2.get_point_num_bytes().into(),
                    field_info2.get_point_num_bytes().into(),
                );

                let left = Self::uninvert(&field_info1.name, left_reader);
                let right = Self::uninvert(&field_info1.name, right_reader);
                if !uninverted_equal(&left, &right) {
                    Assert::fail(&format!(
                        "{info} field={} expected:<{}> but was:<{}>",
                        field_info1.name,
                        uninverted_to_string(&left),
                        uninverted_to_string(&right)
                    ));
                }
            }
        }

        // make sure FieldInfos2 doesn't have any point fields that FieldInfo1 didn't have
        for field_info2 in field_infos2.iter() {
            if field_info2.get_point_dimension_count() != 0 {
                let field_info1 = field_infos1
                    .field_info(&field_info2.name)
                    .unwrap_or_else(npe);
                // same data dimension count?
                assert_long_equals(
                    i,
                    field_info2.get_point_dimension_count().into(),
                    field_info1.get_point_dimension_count().into(),
                );
                // same index dimension count?
                assert_long_equals(
                    i,
                    field_info2.get_point_index_dimension_count().into(),
                    field_info1.get_point_index_dimension_count().into(),
                );
                // same bytes per dimension?
                assert_long_equals(
                    i,
                    field_info2.get_point_num_bytes().into(),
                    field_info1.get_point_num_bytes().into(),
                );

                // we don't need to uninvert and compare here ... we did that in the first loop above
            }
        }
    }
}
