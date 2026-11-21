//! Port of `lucene/core/src/test/org/apache/lucene/util/TestVersion.java`.
//!
//! Java reflects over `Version.class.getDeclaredFields()`; Rust has no
//! reflection, so [`VERSION_FIELDS`] declares the static `Version` fields of
//! `Version.java` (10.5.0), in declaration order, with their names and
//! whether each carries `@Deprecated`.
//!
//! Java's `Version.parse` and `parseLeniently` throw `java.text.ParseException`,
//! which Rucene raises as that exception kind (see [`Version::parse`]).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::system;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::Version;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_equals_msg, class, random, test};

/// `java.text.ParseException`.
const PARSE_EXCEPTION: &str = rucene::util::extra::PARSE_EXCEPTION;

/// The static `Version` fields of `Version.java`: `(name, value,
/// @Deprecated)`.
#[allow(deprecated)]
const VERSION_FIELDS: &[(&str, Version, bool)] = &[
    ("LUCENE_9_0_0", Version::LUCENE_9_0_0, true),
    ("LUCENE_9_1_0", Version::LUCENE_9_1_0, true),
    ("LUCENE_9_2_0", Version::LUCENE_9_2_0, true),
    ("LUCENE_9_3_0", Version::LUCENE_9_3_0, true),
    ("LUCENE_9_4_0", Version::LUCENE_9_4_0, true),
    ("LUCENE_9_4_1", Version::LUCENE_9_4_1, true),
    ("LUCENE_9_4_2", Version::LUCENE_9_4_2, true),
    ("LUCENE_9_5_0", Version::LUCENE_9_5_0, true),
    ("LUCENE_9_6_0", Version::LUCENE_9_6_0, true),
    ("LUCENE_9_7_0", Version::LUCENE_9_7_0, true),
    ("LUCENE_9_8_0", Version::LUCENE_9_8_0, true),
    ("LUCENE_9_9_0", Version::LUCENE_9_9_0, true),
    ("LUCENE_9_9_1", Version::LUCENE_9_9_1, true),
    ("LUCENE_9_9_2", Version::LUCENE_9_9_2, true),
    ("LUCENE_9_10_0", Version::LUCENE_9_10_0, true),
    ("LUCENE_9_11_0", Version::LUCENE_9_11_0, true),
    ("LUCENE_9_11_1", Version::LUCENE_9_11_1, true),
    ("LUCENE_9_12_0", Version::LUCENE_9_12_0, true),
    ("LUCENE_9_12_1", Version::LUCENE_9_12_1, true),
    ("LUCENE_9_12_2", Version::LUCENE_9_12_2, true),
    ("LUCENE_9_12_3", Version::LUCENE_9_12_3, true),
    ("LUCENE_9_12_4", Version::LUCENE_9_12_4, true),
    ("LUCENE_10_0_0", Version::LUCENE_10_0_0, true),
    ("LUCENE_10_1_0", Version::LUCENE_10_1_0, true),
    ("LUCENE_10_2_0", Version::LUCENE_10_2_0, true),
    ("LUCENE_10_2_1", Version::LUCENE_10_2_1, true),
    ("LUCENE_10_2_2", Version::LUCENE_10_2_2, true),
    ("LUCENE_10_3_0", Version::LUCENE_10_3_0, true),
    ("LUCENE_10_3_1", Version::LUCENE_10_3_1, true),
    ("LUCENE_10_3_2", Version::LUCENE_10_3_2, true),
    ("LUCENE_10_4_0", Version::LUCENE_10_4_0, true),
    ("LUCENE_10_5_0", Version::LUCENE_10_5_0, false),
    ("LATEST", Version::LATEST, false),
    ("LUCENE_CURRENT", Version::LUCENE_CURRENT, true),
];

/// `TestVersion`.
pub fn class_def() -> Arc<Class> {
    // `ParseException extends Exception`.
    randomizedtesting::jdk::exception::register_class(
        PARSE_EXCEPTION,
        randomizedtesting::jdk::exception::EXCEPTION,
    );
    class(
        "org.apache.lucene.util.TestVersion",
        vec![
            test("testOnOrAfter", test_on_or_after),
            test("testToString", test_to_string),
            test("testParseLeniently", test_parse_leniently),
            test(
                "testParseLenientlyExceptions",
                test_parse_leniently_exceptions,
            ),
            test(
                "testParseLenientlyOnAllConstants",
                test_parse_leniently_on_all_constants,
            ),
            test("testParse", test_parse),
            test("testForwardsCompatibility", test_forwards_compatibility),
            test("testParseExceptions", test_parse_exceptions),
            test("testDeprecations", test_deprecations),
            test(
                "testNonFloatingPointCompliantVersionNumbers",
                test_non_floating_point_compliant_version_numbers,
            ),
            test(
                "testLatestVersionCommonBuild",
                test_latest_version_common_build,
            ),
            test("testEqualsHashCode", test_equals_hash_code),
        ],
    )
}

fn test_on_or_after() -> Result<()> {
    for (_, v, _) in VERSION_FIELDS {
        Assert::assert_true_msg(
            &format!("LATEST must be always onOrAfter({v})"),
            Version::LATEST.on_or_after(v),
        );
    }
    Assert::assert_true(Version::LUCENE_10_0_0.on_or_after(&Version::LUCENE_9_0_0));
    Ok(())
}

fn test_to_string() -> Result<()> {
    assert_equals(&"9.0.0".to_string(), &Version::LUCENE_9_0_0.to_string());
    assert_equals(&"10.0.0".to_string(), &Version::LUCENE_10_0_0.to_string());
    Ok(())
}

fn test_parse_leniently() -> Result<()> {
    assert_equals(&Version::LUCENE_10_0_0, &Version::parse_leniently("10.0")?);
    assert_equals(
        &Version::LUCENE_10_0_0,
        &Version::parse_leniently("10.0.0")?,
    );
    assert_equals(
        &Version::LUCENE_10_0_0,
        &Version::parse_leniently("LUCENE_10_0")?,
    );
    assert_equals(
        &Version::LUCENE_10_0_0,
        &Version::parse_leniently("LUCENE_10_0_0")?,
    );
    assert_equals(&Version::LUCENE_9_0_0, &Version::parse_leniently("9.0")?);
    assert_equals(&Version::LUCENE_9_0_0, &Version::parse_leniently("9.0.0")?);
    assert_equals(
        &Version::LUCENE_9_0_0,
        &Version::parse_leniently("LUCENE_90")?,
    );
    assert_equals(
        &Version::LUCENE_9_0_0,
        &Version::parse_leniently("LUCENE_9_0")?,
    );
    assert_equals(
        &Version::LUCENE_9_0_0,
        &Version::parse_leniently("LUCENE_9_0_0")?,
    );

    assert_equals(&Version::LATEST, &Version::parse_leniently("LATEST")?);
    assert_equals(&Version::LATEST, &Version::parse_leniently("latest")?);
    assert_equals(
        &Version::LATEST,
        &Version::parse_leniently("LUCENE_CURRENT")?,
    );
    assert_equals(
        &Version::LATEST,
        &Version::parse_leniently("lucene_current")?,
    );
    Ok(())
}

/// `expectThrows(ParseException.class, () -> parse(input))` and
/// `assertTrue(expected.getMessage().contains(input))`.
fn assert_parse_exception(
    parse: fn(&str) -> std::result::Result<Version, rucene::error::LuceneError>,
    input: &str,
) {
    let expected = expect_throws(PARSE_EXCEPTION, || parse(input));
    Assert::assert_true(expected.message().unwrap_or_default().contains(input));
}

fn test_parse_leniently_exceptions() -> Result<()> {
    for input in ["LUCENE", "LUCENE_610", "LUCENE61", "LUCENE_7.0.0"] {
        assert_parse_exception(Version::parse_leniently, input);
    }
    Ok(())
}

fn test_parse_leniently_on_all_constants() -> Result<()> {
    let mut at_least_one = false;
    for (name, v, _) in VERSION_FIELDS {
        at_least_one = true;
        assert_equals(v, &Version::parse_leniently(&v.to_string())?);
        assert_equals(v, &Version::parse_leniently(name)?);
        assert_equals(v, &Version::parse_leniently(&name.to_lowercase())?);
    }
    Assert::assert_true(at_least_one);
    Ok(())
}

fn test_parse() -> Result<()> {
    assert_equals(&Version::LUCENE_10_0_0, &Version::parse("10.0.0")?);
    assert_equals(&Version::LUCENE_9_0_0, &Version::parse("9.0.0")?);

    // Version does not pass judgement on the major version:
    assert_equals(&1, &Version::parse("1.0")?.major);
    assert_equals(&7, &Version::parse("7.0.0")?.major);
    Ok(())
}

fn test_forwards_compatibility() -> Result<()> {
    Assert::assert_true(Version::parse("9.10.20")?.on_or_after(&Version::LUCENE_9_0_0));
    Ok(())
}

fn test_parse_exceptions() -> Result<()> {
    for input in [
        "LUCENE_7_0_0",
        "7.256",
        "7.-1",
        "7.1.256",
        "7.1.-1",
        "7.1.1.3",
        "7.1.1.-1",
        "7.1.1.1",
        "7.1.1.2",
        "7.0.0.0",
        "7.0.0.1.42",
        "7..0.1",
    ] {
        assert_parse_exception(Version::parse, input);
    }
    Ok(())
}

fn test_deprecations() -> Result<()> {
    // all but the latest version should be deprecated
    let mut at_least_one = false;
    for (name, v, dep) in VERSION_FIELDS {
        at_least_one = true;
        if *v == Version::LATEST && *name != "LUCENE_CURRENT" {
            Assert::assert_false_msg(&format!("{name} should not be deprecated"), *dep);
        } else {
            Assert::assert_true_msg(&format!("{name} should be deprecated"), *dep);
        }
    }
    Assert::assert_true(at_least_one);
    Ok(())
}

fn test_non_floating_point_compliant_version_numbers() -> Result<()> {
    let version800 = Version::parse("8.0.0")?;
    Assert::assert_true(Version::parse("8.10.0")?.on_or_after(&version800));
    Assert::assert_true(Version::parse("8.10.0")?.on_or_after(&Version::parse("8.9.255")?));
    Assert::assert_true(Version::parse("8.128.0")?.on_or_after(&version800));
    Assert::assert_true(Version::parse("8.255.0")?.on_or_after(&version800));

    let version400 = Version::parse("4.0.0")?;
    Assert::assert_true(version800.on_or_after(&version400));
    Assert::assert_true(Version::parse("8.128.0")?.on_or_after(&version400));
    Assert::assert_false(version400.on_or_after(&version800));
    Ok(())
}

fn test_latest_version_common_build() -> Result<()> {
    // common-build.xml sets 'tests.LUCENE_VERSION', if not, we skip this test!
    let common_build_version = system::get_property("tests.LUCENE_VERSION");
    LuceneTestCase::assume_true(
        "Null 'tests.LUCENE_VERSION' test property. You should run the tests with the official Lucene build file",
        common_build_version.is_some(),
    );
    assert_equals_msg(
        Some("Version.LATEST does not match the one given in tests.LUCENE_VERSION property"),
        &Some(Version::LATEST.to_string()),
        &common_build_version,
    );
    Ok(())
}

fn test_equals_hash_code() -> Result<()> {
    let random = random();
    let version = format!(
        "{}.{}.{}",
        4 + random.next_int_bound(1),
        random.next_int_bound(10),
        random.next_int_bound(10)
    );
    let v1 = Version::parse_leniently(&version)?;
    let v2 = Version::parse_leniently(&version)?;
    assert_equals(&v1.hash_code(), &v2.hash_code());
    assert_equals(&v1, &v2);
    let iters = 10 + random.next_int_bound(20);
    for _ in 0..iters {
        let v = format!(
            "{}.{}.{}",
            4 + random.next_int_bound(1),
            random.next_int_bound(10),
            random.next_int_bound(10)
        );
        if v == version {
            assert_equals(&Version::parse_leniently(&v)?.hash_code(), &v1.hash_code());
            assert_equals(&Version::parse_leniently(&v)?, &v1);
        } else {
            Assert::assert_false(Version::parse_leniently(&v)? == v1);
        }
    }
    Ok(())
}
