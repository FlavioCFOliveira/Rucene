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

//! Oracle parity of `LuceneTestCase.Nightly`, `LuceneTestCase.SuppressSysoutChecks`
//! and `StringMockResourceLoader` (rmp #445) against Apache Lucene 10.5.0 on
//! JDK 25.
//!
//! The Java oracle (`org.apache.lucene.tests.util.TestFrameworkTypesOracle`,
//! in the workspace harness `crates/randomizedtesting/tests/fixtures/oracle/`)
//! writes `test_framework_types.tsv`: each annotation type's binary and simple
//! name, `@Inherited`, `@Documented`, retention, `@Target`, member count and
//! `@TestGroup` (with `TestGroup.Utilities`' group name and system property),
//! the `toString()` of annotation values (including a `bugUrl` with characters
//! JDK 25 escapes), inheritance by a subclass, and the bytes
//! `StringMockResourceLoader.openResource` returns for several texts and
//! resource names. `English` has its own oracle test
//! (`test_util_oracle.rs::english_is_lucenes`).
//!
//! Lucene has no test class of its own for any of these types.

mod oracle_support;

use std::collections::BTreeMap;
use std::io::Read;

use oracle_support::labelled;
use randomizedtesting::annotations::{AnnotationType, TestGroupUtilities};
use randomizedtesting::jdk::reflect::{Class, JavaAnnotation};
use rucene::util::resource_loader::ResourceLoader;
use rucene_test_framework::util::lucene_test_case::{
    Nightly, SuppressSysoutChecks, NIGHTLY, SUPPRESS_SYSOUT_CHECKS,
};
use rucene_test_framework::util::StringMockResourceLoader;

/// A fixture string: hex UTF-16 code units joined by `.`, `-` when empty.
fn string(s: &str) -> String {
    if s == "-" {
        return String::new();
    }
    let units: Vec<u16> = s
        .split('.')
        .map(|u| u16::from_str_radix(u, 16).expect("a hex code unit"))
        .collect();
    String::from_utf16(&units).expect("the oracle writes valid UTF-16")
}

/// A fixture byte array: hex bytes joined by `.`, `-` when empty.
fn bytes(s: &str) -> Vec<u8> {
    if s == "-" {
        return Vec::new();
    }
    s.split('.')
        .map(|b| u8::from_str_radix(b, 16).expect("a hex byte"))
        .collect()
}

/// `string`'s inverse, the spelling of a fixture key.
fn units(s: &str) -> String {
    if s.is_empty() {
        return "-".to_string();
    }
    s.encode_utf16()
        .map(|u| format!("{u:04x}"))
        .collect::<Vec<_>>()
        .join(".")
}

fn oracle() -> BTreeMap<String, String> {
    labelled("test_framework_types.tsv")
}

fn get<'a>(o: &'a BTreeMap<String, String>, key: &str) -> &'a str {
    o.get(key)
        .unwrap_or_else(|| panic!("the oracle has no row {key}"))
}

/// The annotation-type facts the oracle records under `key` (`type(...)`),
/// against the port's [`AnnotationType`]. Retention and `@Documented` have no
/// Rust counterpart; the oracle pins them so that a change in Lucene shows.
fn assert_type(o: &BTreeMap<String, String>, key: &str, ty: &AnnotationType, members: usize) {
    assert_eq!(
        string(get(o, &format!("{key}.name"))),
        ty.name,
        "{key}.name"
    );
    assert_eq!(
        string(get(o, &format!("{key}.simpleName"))),
        ty.simple_name,
        "{key}.simpleName"
    );
    assert_eq!(
        get(o, &format!("{key}.inherited")),
        ty.inherited.to_string(),
        "{key}.inherited"
    );
    assert_eq!(get(o, &format!("{key}.documented")), "true");
    assert_eq!(get(o, &format!("{key}.retention")), "RUNTIME");
    assert_eq!(
        get(o, &format!("{key}.members")),
        members.to_string(),
        "{key}.members"
    );
    assert_eq!(
        get(o, &format!("{key}.testGroup")),
        ty.test_group.is_some().to_string(),
        "{key}.testGroup"
    );
    if let Some(group) = ty.test_group {
        assert_eq!(string(get(o, &format!("{key}.testGroup.name"))), group.name);
        assert_eq!(
            string(get(o, &format!("{key}.testGroup.sysProperty"))),
            group.sys_property
        );
        assert_eq!(
            get(o, &format!("{key}.testGroup.enabled")),
            group.enabled.to_string()
        );
        assert_eq!(
            string(get(o, &format!("{key}.groupName"))),
            TestGroupUtilities::get_group_name(ty)
        );
        assert_eq!(
            string(get(o, &format!("{key}.groupSysProperty"))),
            TestGroupUtilities::get_sys_property(ty)
        );
    }
}

/// `LuceneTestCase.Nightly`: a member-less, inherited test group disabled by
/// default and enabled by `tests.nightly`.
#[test]
fn nightly_is_lucenes() {
    let o = oracle();
    assert_type(&o, "nightly", &NIGHTLY, 0);
    assert_eq!(get(&o, "nightly.target"), "null");
    assert_eq!(Nightly.annotation_type().name, NIGHTLY.name);
    assert_eq!(
        Nightly.to_java_string(),
        string(get(&o, "nightly.toString"))
    );

    // `@Inherited`: a subclass of an annotated suite carries it.
    let base = Class::builder("org.apache.lucene.tests.util.TestFrameworkTypesOracle$NightlyBase")
        .annotate(Nightly)
        .build();
    let child =
        Class::builder("org.apache.lucene.tests.util.TestFrameworkTypesOracle$NightlyChild")
            .extends(&base)
            .build();
    assert_eq!(
        get(&o, "nightly.inheritedBySubclass"),
        child.is_annotation_present(&NIGHTLY).to_string()
    );
}

/// `LuceneTestCase.SuppressSysoutChecks`: an inherited annotation of suite
/// classes (`@Target(TYPE)`) with one member, `bugUrl`.
#[test]
fn suppress_sysout_checks_is_lucenes() {
    let o = oracle();
    assert_type(&o, "sysout", &SUPPRESS_SYSOUT_CHECKS, 1);
    assert_eq!(get(&o, "sysout.target"), "[TYPE]");
    for i in 0..2 {
        let bug_url = string(get(&o, &format!("sysout.bugUrl.{i}")));
        let value = SuppressSysoutChecks {
            bug_url: bug_url.clone(),
        };
        assert_eq!(value.annotation_type().name, SUPPRESS_SYSOUT_CHECKS.name);
        assert_eq!(
            value.to_java_string(),
            string(get(&o, &format!("sysout.toString.{i}"))),
            "toString of bugUrl {bug_url:?}"
        );
    }

    let suite = Class::builder("org.apache.lucene.tests.util.TestFrameworkTypesOracle$SysoutSuite")
        .annotate(SuppressSysoutChecks {
            bug_url: "https://issues.apache.org/jira/browse/LUCENE-1".to_string(),
        })
        .build();
    let child = Class::builder("org.apache.lucene.tests.util.TestFrameworkTypesOracle$SysoutChild")
        .extends(&suite)
        .build();
    assert_eq!(
        get(&o, "sysout.inheritedBySubclass"),
        child
            .is_annotation_present(&SUPPRESS_SYSOUT_CHECKS)
            .to_string()
    );
}

/// `StringMockResourceLoader.openResource`: the UTF-8 bytes of the text,
/// whatever the resource's name, a fresh stream per call.
#[test]
fn string_mock_resource_loader_is_lucenes() {
    let o = oracle();
    let mut texts = 0;
    for i in 0.. {
        let Some(text) = o.get(&format!("smrl.text.{i}")) else {
            break;
        };
        texts += 1;
        let loader = StringMockResourceLoader::new(string(text));
        for name in ["stopwords.txt", "", "/absolute/path.txt"] {
            let mut read = Vec::new();
            loader
                .open_resource(name)
                .unwrap_or_else(|e| panic!("open {name:?}: {e}"))
                .read_to_end(&mut read)
                .unwrap();
            let key = format!("smrl.open.{i}.{}", units(name));
            assert_eq!(read, bytes(get(&o, &key)), "{key}");
        }
    }
    assert_eq!(texts, 5, "the oracle records five texts");
}
