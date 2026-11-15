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

//! Port of `MessagesTestBundle.java`, a message bundle class of the tests
//! (`extends NLS`), with its resources `MessagesTestBundle.properties` and
//! `MessagesTestBundle_ja.properties` (byte-identical copies under
//! `tests/resources/org/apache/lucene/queryparser/flexible/messages/`, SHA-1
//! checked by `query_oracle_flexible.rs`).
//!
//! The class is an [`NlsClass`] descriptor, as `QueryParserMessages` is in
//! the crate: its static initializer runs on the first read of one of its
//! fields. Putting the two resources on Java's class path is
//! [`ResourceBundle::register_properties`], done by the same initializer
//! before `NLS.initializeMessages` reads them.

use rucene_queryparser::flexible::messages::nls::{
    NlsClass, NlsField, FINAL, NLS, PRIVATE, PUBLIC, STATIC,
};
use rucene_queryparser::jdk::resource_bundle::ResourceBundle;

/// `MessagesTestBundle.class.getName()`.
pub const BUNDLE_NAME: &str = "org.apache.lucene.queryparser.flexible.messages.MessagesTestBundle";

/// The resource `MessagesTestBundle.properties`, byte for byte.
pub const PROPERTIES: &[u8] = include_bytes!(
    "../../resources/org/apache/lucene/queryparser/flexible/messages/MessagesTestBundle.properties"
);

/// The resource `MessagesTestBundle_ja.properties`, byte for byte.
pub const PROPERTIES_JA: &[u8] = include_bytes!(
    "../../resources/org/apache/lucene/queryparser/flexible/messages/MessagesTestBundle_ja.properties"
);

/// The class `MessagesTestBundle` as `NLS` reflects on it: its declared
/// fields in declaration order.
static CLASS: NlsClass = NlsClass::new(
    BUNDLE_NAME,
    &[
        &F_BUNDLE_NAME,
        &F_Q0001E_INVALID_SYNTAX,
        &F_Q0004E_INVALID_SYNTAX_ESCAPE_UNICODE_TRUNCATION,
        &F_Q0005E_MESSAGE_NOT_IN_BUNDLE,
    ],
    static_initializer,
);

/// The `static { ... }` block: register all string ids with NLS class and
/// initialize static string values. The registration of the resources
/// stands for the test class path, which exists before the class loads.
fn static_initializer() {
    ResourceBundle::register_properties(BUNDLE_NAME, PROPERTIES);
    ResourceBundle::register_properties(&format!("{BUNDLE_NAME}_ja"), PROPERTIES_JA);
    NLS::initialize_messages(BUNDLE_NAME, &CLASS);
}

/// `private static final String BUNDLE_NAME = MessagesTestBundle.class.getName()`.
static F_BUNDLE_NAME: NlsField = NlsField::new(
    &CLASS,
    "BUNDLE_NAME",
    PRIVATE | STATIC | FINAL,
    Some(BUNDLE_NAME),
);
static F_Q0001E_INVALID_SYNTAX: NlsField =
    NlsField::new(&CLASS, "Q0001E_INVALID_SYNTAX", PUBLIC | STATIC, None);
static F_Q0004E_INVALID_SYNTAX_ESCAPE_UNICODE_TRUNCATION: NlsField = NlsField::new(
    &CLASS,
    "Q0004E_INVALID_SYNTAX_ESCAPE_UNICODE_TRUNCATION",
    PUBLIC | STATIC,
    None,
);
static F_Q0005E_MESSAGE_NOT_IN_BUNDLE: NlsField = NlsField::new(
    &CLASS,
    "Q0005E_MESSAGE_NOT_IN_BUNDLE",
    PUBLIC | STATIC,
    None,
);

/// `public class MessagesTestBundle extends NLS`: its `public static
/// String` fields, which must match the strings in the property files.
pub struct MessagesTestBundle;

impl MessagesTestBundle {
    /// `public static String Q0001E_INVALID_SYNTAX`.
    pub const Q0001E_INVALID_SYNTAX: &'static NlsField = &F_Q0001E_INVALID_SYNTAX;
    /// `public static String Q0004E_INVALID_SYNTAX_ESCAPE_UNICODE_TRUNCATION`.
    pub const Q0004E_INVALID_SYNTAX_ESCAPE_UNICODE_TRUNCATION: &'static NlsField =
        &F_Q0004E_INVALID_SYNTAX_ESCAPE_UNICODE_TRUNCATION;
    /// `public static String Q0005E_MESSAGE_NOT_IN_BUNDLE`: this message is
    /// missing from the properties file.
    pub const Q0005E_MESSAGE_NOT_IN_BUNDLE: &'static NlsField = &F_Q0005E_MESSAGE_NOT_IN_BUNDLE;
}
