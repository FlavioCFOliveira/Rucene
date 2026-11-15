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

//! Port of `TestNLS.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene_queryparser::flexible::messages::message::Message;
use rucene_queryparser::flexible::messages::message_impl::MessageImpl;
use rucene_queryparser::flexible::messages::nls::NLS;
use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::object::JavaObject;

use super::messages_test_bundle::MessagesTestBundle;
use crate::qp_support::core_support::{class, test};
use crate::qp_support::{assert_str_eq, jdk_default_hooks};

/// `TestNLS`.
pub fn class_def() -> Arc<Class> {
    let mut methods = jdk_default_hooks();
    methods.extend([
        test("testMessageLoading", test_message_loading),
        test("testMessageLoading_ja", test_message_loading_ja),
        test("testNLSLoading", test_nls_loading),
        test("testNLSLoading_ja", test_nls_loading_ja),
        test("testNLSLoading_xx_XX", test_nls_loading_xx_xx),
        test("testMissingMessage", test_missing_message),
    ]);
    class(
        "org.apache.lucene.queryparser.flexible.messages.TestNLS",
        methods,
    )
}

/// `Locale.JAPANESE`.
fn japanese() -> Locale {
    Locale::of("ja", "", "")
}

/// `!Locale.getDefault().getLanguage().equals("ja")`: if the default locale
/// is ja, you get ja as a fallback (see `ResourceBundle.getBundle(String,
/// Locale, ClassLoader)`).
fn default_is_not_ja() -> bool {
    Locale::get_default().get_language() != "ja"
}

fn xxx() -> Vec<JavaObject> {
    vec![JavaObject::String("XXX".to_string())]
}

fn test_message_loading() -> Result<()> {
    let invalid_syntax =
        MessageImpl::with_args(MessagesTestBundle::Q0001E_INVALID_SYNTAX.get(), xxx());
    if default_is_not_ja() {
        assert_str_eq(
            "Syntax Error: XXX",
            &invalid_syntax.get_localized_message_with_locale(&Locale::english())?,
        );
    }
    Ok(())
}

fn test_message_loading_ja() -> Result<()> {
    let invalid_syntax =
        MessageImpl::with_args(MessagesTestBundle::Q0001E_INVALID_SYNTAX.get(), xxx());
    assert_str_eq(
        "構文エラー: XXX",
        &invalid_syntax.get_localized_message_with_locale(&japanese())?,
    );
    Ok(())
}

fn test_nls_loading() -> Result<()> {
    let message = NLS::get_localized_message_with_locale(
        MessagesTestBundle::Q0004E_INVALID_SYNTAX_ESCAPE_UNICODE_TRUNCATION
            .get()
            .as_deref(),
        &Locale::english(),
    )?;
    if default_is_not_ja() {
        assert_str_eq("Truncated unicode escape sequence.", &message);
    }

    let message = NLS::get_localized_message_with_locale_and_args(
        MessagesTestBundle::Q0001E_INVALID_SYNTAX.get().as_deref(),
        &Locale::english(),
        Some(&xxx()),
    )?;
    if default_is_not_ja() {
        assert_str_eq("Syntax Error: XXX", &message);
    }
    Ok(())
}

fn test_nls_loading_ja() -> Result<()> {
    let message = NLS::get_localized_message_with_locale(
        MessagesTestBundle::Q0004E_INVALID_SYNTAX_ESCAPE_UNICODE_TRUNCATION
            .get()
            .as_deref(),
        &japanese(),
    )?;
    assert_str_eq(
        "切り捨てられたユニコード・エスケープ・シーケンス。",
        &message,
    );

    let message = NLS::get_localized_message_with_locale_and_args(
        MessagesTestBundle::Q0001E_INVALID_SYNTAX.get().as_deref(),
        &japanese(),
        Some(&xxx()),
    )?;
    assert_str_eq("構文エラー: XXX", &message);
    Ok(())
}

fn test_nls_loading_xx_xx() -> Result<()> {
    // `new Locale.Builder().setLanguageTag("xx-XX").build()`
    let locale = Locale::builder_from_language_tag("xx-XX")?;
    let message = NLS::get_localized_message_with_locale(
        MessagesTestBundle::Q0004E_INVALID_SYNTAX_ESCAPE_UNICODE_TRUNCATION
            .get()
            .as_deref(),
        &locale,
    )?;
    if default_is_not_ja() {
        assert_str_eq("Truncated unicode escape sequence.", &message);
    }

    let message = NLS::get_localized_message_with_locale_and_args(
        MessagesTestBundle::Q0001E_INVALID_SYNTAX.get().as_deref(),
        &locale,
        Some(&xxx()),
    )?;
    if default_is_not_ja() {
        assert_str_eq("Syntax Error: XXX", &message);
    }
    Ok(())
}

fn test_missing_message() -> Result<()> {
    let locale = Locale::english();
    let message = NLS::get_localized_message_with_locale(
        MessagesTestBundle::Q0005E_MESSAGE_NOT_IN_BUNDLE
            .get()
            .as_deref(),
        &locale,
    )?;
    assert_str_eq(
        &format!(
            "Message with key:Q0005E_MESSAGE_NOT_IN_BUNDLE and locale: {} not found.",
            locale.to_language_tag()
        ),
        &message,
    );
    Ok(())
}
