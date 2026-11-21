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

//! Helpers shared by the test targets of `rucene-queryparser` that port the
//! tests of Lucene 10.5.0 `lucene/queryparser/src/test` (sprint 13, test
//! passes T3a and T3b).
//!
//! * [`core_support`] and [`search_support`], the suite plumbing of the
//!   sprint-9 core test ports, reused, not copied: declaring a Java test
//!   class whose methods are Rust functions, running it as a suite through
//!   the ported `randomizedtesting` runner, and the JUnit assertions the
//!   ported bodies use beyond `Assert`.
//! * [`query_parser_test_base`], the port of `org.apache.lucene.queryparser.util.QueryParserTestBase`,
//!   which the classic `TestQueryParser` (and through it
//!   `TestExtendableQueryParser`) and the flexible `TestStandardQP` extend.
//! * [`jdk_default_hooks`], the random `Locale` and `TimeZone` the test
//!   framework sets for a class, handed to the queryparser's own JDK
//!   emulation (by language tag and zone ID, the decision of rmp #476), so
//!   `Locale.getDefault()` and `TimeZone.getDefault()` answer what the
//!   Java test reads.
//! * [`catch_java`], Java's `try { … } catch (Throwable t)` over a Rucene
//!   call, whether it returns an error or throws.
//!
//! A target uses it as `mod qp_support;`.

#![allow(dead_code)]

#[path = "../../../rucene-test-framework/tests/core_support/mod.rs"]
pub mod core_support;

#[path = "../../../rucene-test-framework/tests/core_search/support.rs"]
pub mod search_support;

pub mod query_parser_test_base;

use std::any::Any;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Once};

use randomizedtesting::jdk::exception::{register_class, Throwable};
use randomizedtesting::jdk::reflect::Method;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::error::Result;
use rucene::search::Query;
use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::time_zone::TimeZone;
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::LuceneTestCase;

use search_support::static_hook;

/// `org.apache.lucene.queryparser.classic.ParseException`.
pub const CLASSIC_PARSE_EXCEPTION: &str = "org.apache.lucene.queryparser.classic.ParseException";
/// `org.apache.lucene.queryparser.flexible.standard.parser.ParseException`.
pub const STANDARD_PARSE_EXCEPTION: &str =
    "org.apache.lucene.queryparser.flexible.standard.parser.ParseException";
/// `org.apache.lucene.queryparser.surround.parser.ParseException`.
pub const SURROUND_PARSE_EXCEPTION: &str =
    "org.apache.lucene.queryparser.surround.parser.ParseException";
/// `org.apache.lucene.queryparser.xml.ParserException`.
pub const PARSER_EXCEPTION: &str = "org.apache.lucene.queryparser.xml.ParserException";

/// The queryparser's throwable classes and their superclasses (the
/// `extends` clauses of the Lucene 10.5.0 sources), so that
/// `Throwable::is_instance` answers Java's `instanceof`.
const HIERARCHY: &[(&str, &str)] = &[
    (CLASSIC_PARSE_EXCEPTION, "java.lang.Exception"),
    (
        "org.apache.lucene.queryparser.classic.TokenMgrError",
        "java.lang.Error",
    ),
    (SURROUND_PARSE_EXCEPTION, "java.lang.Exception"),
    (
        "org.apache.lucene.queryparser.surround.parser.TokenMgrError",
        "java.lang.Error",
    ),
    (
        "org.apache.lucene.queryparser.surround.query.TooManyBasicQueries",
        "java.io.IOException",
    ),
    (PARSER_EXCEPTION, "java.lang.Exception"),
    (
        "org.apache.lucene.queryparser.flexible.core.QueryNodeException",
        "java.lang.Exception",
    ),
    (
        "org.apache.lucene.queryparser.flexible.core.QueryNodeParseException",
        "org.apache.lucene.queryparser.flexible.core.QueryNodeException",
    ),
    (
        STANDARD_PARSE_EXCEPTION,
        "org.apache.lucene.queryparser.flexible.core.QueryNodeParseException",
    ),
    (
        "org.apache.lucene.queryparser.flexible.core.QueryNodeError",
        "java.lang.Error",
    ),
    (
        "org.apache.lucene.queryparser.flexible.standard.parser.TokenMgrError",
        "java.lang.Error",
    ),
    ("org.xml.sax.SAXException", "java.lang.Exception"),
    ("org.xml.sax.SAXParseException", "org.xml.sax.SAXException"),
    (
        "org.apache.lucene.util.automaton.TooComplexToDeterminizeException",
        "java.lang.RuntimeException",
    ),
    (
        "org.apache.lucene.search.IndexSearcher$TooManyClauses",
        "java.lang.RuntimeException",
    ),
    (
        "org.apache.lucene.search.IndexSearcher$TooManyNestedClauses",
        "org.apache.lucene.search.IndexSearcher$TooManyClauses",
    ),
    (
        "java.lang.NumberFormatException",
        "java.lang.IllegalArgumentException",
    ),
    (
        "java.util.ConcurrentModificationException",
        "java.lang.RuntimeException",
    ),
];

/// Registers [`HIERARCHY`] once per process.
pub fn register_queryparser_exceptions() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        for (class, superclass) in HIERARCHY {
            register_class(class, superclass);
        }
    });
}

/// `try { body } catch (Throwable t) { … }` over a Rucene call: its value,
/// or the throwable it stands for, whether it returned an error or threw
/// (panicked with) a Java exception.
pub fn catch_java<T>(body: impl FnOnce() -> Result<T>) -> std::result::Result<T, Throwable> {
    register_queryparser_exceptions();
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(java_throwable(&e)),
        Err(payload) => Err(Throwable::from_panic(payload)),
    }
}

/// `expectThrows(class, runnable)` over a Rucene call that returns an
/// error or throws.
pub fn expect_throws<T>(class: &str, body: impl FnOnce() -> Result<T>) -> Throwable {
    register_queryparser_exceptions();
    rucene_test_framework::util::java_exceptions::expect_throws(class, body)
}

/// The queryparser `Locale` standing for the test framework's: by language
/// tag, or by its fields when the tag does not round-trip through
/// `Locale.Builder` (a variant that is no well-formed BCP 47 subtag).
fn to_queryparser_locale(l: &rucene_test_framework::jdk::locale::Locale) -> Locale {
    let tag = l.to_language_tag();
    match Locale::builder_from_language_tag(&tag) {
        Ok(locale) if locale.to_language_tag() == tag => locale,
        _ => Locale::of_parts(
            l.get_language(),
            l.get_script(),
            l.get_country(),
            l.get_variant(),
        ),
    }
}

/// `@BeforeClass`: `Locale.setDefault`/`TimeZone.setDefault` of the
/// queryparser's JDK emulation to the random values
/// `TestRuleSetupAndRestoreClassEnv` chose for the class (that rule runs
/// before every `@BeforeClass`, as in Java).
fn sync_jdk_defaults() -> Result<()> {
    register_queryparser_exceptions();
    Locale::set_default(
        rucene_test_framework::jdk::locale::get_default().map(|l| to_queryparser_locale(&l)),
    );
    TimeZone::set_default(
        LuceneTestCase::get_default_time_zone().map(|z| TimeZone::get_time_zone(z.get_id())),
    );
    Ok(())
}

/// `@AfterClass`: restores the emulation's startup defaults.
fn restore_jdk_defaults() -> Result<()> {
    Locale::set_default(None);
    TimeZone::set_default(None);
    Ok(())
}

/// The two hooks of [`sync_jdk_defaults`] and [`restore_jdk_defaults`], added
/// to every ported class: Java's parsers read the JVM defaults the test
/// framework randomised.
pub fn jdk_default_hooks() -> Vec<Method> {
    vec![
        static_hook("rucenesyncJdkDefaults", BeforeClass, sync_jdk_defaults),
        static_hook("rucenerestoreJdkDefaults", AfterClass, restore_jdk_defaults),
    ]
}

/// `q instanceof T`.
pub fn is<T: Any>(q: &dyn Query) -> bool {
    q.as_any().is::<T>()
}

/// `(T) q`: a `ClassCastException` (with the JDK's message) when `q` is no
/// `T`.
pub fn cast<'a, T: Any>(q: &'a dyn Query, class: &str) -> &'a T {
    q.as_any().downcast_ref::<T>().unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw(
            "java.lang.ClassCastException",
            format!(
                "class {} cannot be cast to class {class}",
                q.java_class_name()
            ),
        )
    })
}

/// `assertEquals(expected, actual)` over two queries (`Query.equals`).
pub fn assert_query_eq(expected: &dyn Query, actual: &dyn Query) {
    search_support::assert_query_equals(expected, actual);
}

/// `assertEquals(message, expected, actual)` over two queries.
pub fn assert_query_eq_msg(message: &str, expected: &dyn Query, actual: &dyn Query) {
    if !expected.query_eq(actual) {
        Assert::fail(&format!(
            "{message} expected:<{}> but was:<{}>",
            rucene::search::query_to_string(expected),
            rucene::search::query_to_string(actual)
        ));
    }
}

/// `assertEquals(expected, actual)` over two strings.
pub fn assert_str_eq(expected: &str, actual: &str) {
    Assert::assert_equals_str(None, Some(expected), Some(actual));
}

/// `assertEquals(message, expected, actual)` over two strings.
pub fn assert_str_eq_msg(message: &str, expected: &str, actual: &str) {
    Assert::assert_equals_str(Some(message), Some(expected), Some(actual));
}

/// `q.toString()`.
pub fn to_s(q: &dyn Query) -> String {
    rucene::search::query_to_string(q)
}

/// The queryparser's `Locale.ENGLISH`.
pub fn locale_english() -> Locale {
    Locale::english()
}

/// The test-framework analyzer as the shared `Analyzer` handle the parsers
/// take.
pub fn analyzer<A: rucene::analysis::Analyzer + 'static>(
    a: A,
) -> Arc<dyn rucene::analysis::Analyzer> {
    Arc::new(a)
}
