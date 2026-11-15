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

//! Port of `TestExtensions.java`: testcase for the `Extensions` class.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene_queryparser::ext::extensions::{Extensions, DEFAULT_EXTENSION_FIELD_DELIMITER};
use rucene_queryparser::ext::parser_extension::ParserExtension;

use super::extension_stub::ExtensionStub;
use crate::qp_support::search_support::{fixture_class, ftest, set_up};
use crate::qp_support::{assert_str_eq, expect_throws, jdk_default_hooks};

/// The instance of `TestExtensions`: `private Extensions ext`.
#[derive(Default)]
pub struct TestExtensions {
    ext: Option<Extensions>,
}

impl TestExtensions {
    fn ext(&self) -> &Extensions {
        self.ext.as_ref().expect("setUp ran")
    }
}

/// `TestExtensions`.
pub fn class_def() -> Arc<Class> {
    let mut methods = jdk_default_hooks();
    methods.extend([
        set_up(set_up_impl),
        ftest("testBuildExtensionField", test_build_extension_field),
        ftest("testSplitExtensionField", test_split_extension_field),
        ftest("testAddGetExtension", test_add_get_extension),
        ftest("testGetExtDelimiter", test_get_ext_delimiter),
        ftest("testEscapeExtension", test_escape_extension),
    ]);
    fixture_class::<TestExtensions>("org.apache.lucene.queryparser.ext.TestExtensions", methods)
}

fn set_up_impl(t: &mut TestExtensions) -> Result<()> {
    t.ext = Some(Extensions::new());
    Ok(())
}

fn test_build_extension_field(t: &mut TestExtensions) -> Result<()> {
    assert_str_eq(
        "field\\:key",
        &t.ext().build_extension_field_with_field("key", "field"),
    );
    assert_str_eq("\\:key", &t.ext().build_extension_field("key"));

    t.ext = Some(Extensions::with_delimiter('.'));
    assert_str_eq(
        "field.key",
        &t.ext().build_extension_field_with_field("key", "field"),
    );
    assert_str_eq(".key", &t.ext().build_extension_field("key"));
    Ok(())
}

fn test_split_extension_field(t: &mut TestExtensions) -> Result<()> {
    assert_str_eq(
        "field\\:key",
        &t.ext().build_extension_field_with_field("key", "field"),
    );
    assert_str_eq("\\:key", &t.ext().build_extension_field("key"));

    t.ext = Some(Extensions::with_delimiter('.'));
    assert_str_eq(
        "field.key",
        &t.ext().build_extension_field_with_field("key", "field"),
    );
    assert_str_eq(".key", &t.ext().build_extension_field("key"));
    Ok(())
}

fn test_add_get_extension(t: &mut TestExtensions) -> Result<()> {
    let extension: Arc<dyn ParserExtension> = Arc::new(ExtensionStub);
    Assert::assert_true(t.ext().get_extension(Some("foo")).is_none());
    t.ext().add(Some("foo"), Some(Arc::clone(&extension)));
    // `assertSame(extension, ext.getExtension("foo"))`
    Assert::assert_true(
        t.ext()
            .get_extension(Some("foo"))
            .is_some_and(|e| Arc::ptr_eq(&e, &extension)),
    );
    t.ext().add(Some("foo"), None);
    Assert::assert_true(t.ext().get_extension(Some("foo")).is_none());
    Ok(())
}

fn test_get_ext_delimiter(t: &mut TestExtensions) -> Result<()> {
    Assert::assert_equals(
        DEFAULT_EXTENSION_FIELD_DELIMITER.to_string(),
        t.ext().get_extension_field_delimiter().to_string(),
    );
    t.ext = Some(Extensions::with_delimiter('?'));
    Assert::assert_equals(
        "?".to_string(),
        t.ext().get_extension_field_delimiter().to_string(),
    );
    Ok(())
}

fn test_escape_extension(t: &mut TestExtensions) -> Result<()> {
    assert_str_eq(
        "abc\\:\\?\\{\\}\\[\\]\\\\\\(\\)\\+\\-\\!\\~",
        &t.ext().escape_extension_field(Some("abc:?{}[]\\()+-!~"))?,
    );
    // should throw NPE - escape string is null
    expect_throws("java.lang.NullPointerException", || {
        t.ext().escape_extension_field(None)
    });
    Ok(())
}
