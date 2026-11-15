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

//! Port of `QueryParserMessages.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.messages`, with the
//! resource `QueryParserMessages.properties` it loads.

use crate::flexible::messages::nls::{NlsClass, NlsField, FINAL, NLS, PRIVATE, PUBLIC, STATIC};

/// `private static final String BUNDLE_NAME = QueryParserMessages.class.getName()`.
pub const BUNDLE_NAME: &str =
    "org.apache.lucene.queryparser.flexible.core.messages.QueryParserMessages";

/// The resource `org/apache/lucene/queryparser/flexible/core/messages/QueryParserMessages.properties`
/// of `lucene/queryparser/src/resources`, byte for byte.
pub const PROPERTIES: &[u8] = include_bytes!("QueryParserMessages.properties");

/// The class `QueryParserMessages` as `NLS` reflects on it.
static CLASS: NlsClass = NlsClass::new(
    BUNDLE_NAME,
    &[
        &F_BUNDLE_NAME,
        &F_INVALID_SYNTAX,
        &F_INVALID_SYNTAX_CANNOT_PARSE,
        &F_INVALID_SYNTAX_FUZZY_LIMITS,
        &F_INVALID_SYNTAX_FUZZY_EDITS,
        &F_INVALID_SYNTAX_ESCAPE_UNICODE_TRUNCATION,
        &F_INVALID_SYNTAX_ESCAPE_CHARACTER,
        &F_INVALID_SYNTAX_ESCAPE_NONE_HEX_UNICODE,
        &F_NODE_ACTION_NOT_SUPPORTED,
        &F_PARAMETER_VALUE_NOT_SUPPORTED,
        &F_LUCENE_QUERY_CONVERSION_ERROR,
        &F_EMPTY_MESSAGE,
        &F_WILDCARD_NOT_SUPPORTED,
        &F_TOO_MANY_BOOLEAN_CLAUSES,
        &F_LEADING_WILDCARD_NOT_ALLOWED,
        &F_COULD_NOT_PARSE_NUMBER,
        &F_NUMBER_CLASS_NOT_SUPPORTED_BY_NUMERIC_RANGE_QUERY,
        &F_UNSUPPORTED_NUMERIC_DATA_TYPE,
        &F_NUMERIC_CANNOT_BE_EMPTY,
        &F_ANALYZER_REQUIRED,
    ],
    static_initializer,
);

/// The `static { ... }` block: register all string ids with NLS class and
/// initialize static string values.
fn static_initializer() {
    NLS::initialize_messages(BUNDLE_NAME, &CLASS);
}

static F_BUNDLE_NAME: NlsField = NlsField::new(
    &CLASS,
    "BUNDLE_NAME",
    PRIVATE | STATIC | FINAL,
    Some(BUNDLE_NAME),
);
static F_INVALID_SYNTAX: NlsField = NlsField::new(&CLASS, "INVALID_SYNTAX", PUBLIC | STATIC, None);
static F_INVALID_SYNTAX_CANNOT_PARSE: NlsField =
    NlsField::new(&CLASS, "INVALID_SYNTAX_CANNOT_PARSE", PUBLIC | STATIC, None);
static F_INVALID_SYNTAX_FUZZY_LIMITS: NlsField =
    NlsField::new(&CLASS, "INVALID_SYNTAX_FUZZY_LIMITS", PUBLIC | STATIC, None);
static F_INVALID_SYNTAX_FUZZY_EDITS: NlsField =
    NlsField::new(&CLASS, "INVALID_SYNTAX_FUZZY_EDITS", PUBLIC | STATIC, None);
static F_INVALID_SYNTAX_ESCAPE_UNICODE_TRUNCATION: NlsField = NlsField::new(
    &CLASS,
    "INVALID_SYNTAX_ESCAPE_UNICODE_TRUNCATION",
    PUBLIC | STATIC,
    None,
);
static F_INVALID_SYNTAX_ESCAPE_CHARACTER: NlsField = NlsField::new(
    &CLASS,
    "INVALID_SYNTAX_ESCAPE_CHARACTER",
    PUBLIC | STATIC,
    None,
);
static F_INVALID_SYNTAX_ESCAPE_NONE_HEX_UNICODE: NlsField = NlsField::new(
    &CLASS,
    "INVALID_SYNTAX_ESCAPE_NONE_HEX_UNICODE",
    PUBLIC | STATIC,
    None,
);
static F_NODE_ACTION_NOT_SUPPORTED: NlsField =
    NlsField::new(&CLASS, "NODE_ACTION_NOT_SUPPORTED", PUBLIC | STATIC, None);
static F_PARAMETER_VALUE_NOT_SUPPORTED: NlsField = NlsField::new(
    &CLASS,
    "PARAMETER_VALUE_NOT_SUPPORTED",
    PUBLIC | STATIC,
    None,
);
static F_LUCENE_QUERY_CONVERSION_ERROR: NlsField = NlsField::new(
    &CLASS,
    "LUCENE_QUERY_CONVERSION_ERROR",
    PUBLIC | STATIC,
    None,
);
static F_EMPTY_MESSAGE: NlsField = NlsField::new(&CLASS, "EMPTY_MESSAGE", PUBLIC | STATIC, None);
static F_WILDCARD_NOT_SUPPORTED: NlsField =
    NlsField::new(&CLASS, "WILDCARD_NOT_SUPPORTED", PUBLIC | STATIC, None);
static F_TOO_MANY_BOOLEAN_CLAUSES: NlsField =
    NlsField::new(&CLASS, "TOO_MANY_BOOLEAN_CLAUSES", PUBLIC | STATIC, None);
static F_LEADING_WILDCARD_NOT_ALLOWED: NlsField = NlsField::new(
    &CLASS,
    "LEADING_WILDCARD_NOT_ALLOWED",
    PUBLIC | STATIC,
    None,
);
static F_COULD_NOT_PARSE_NUMBER: NlsField =
    NlsField::new(&CLASS, "COULD_NOT_PARSE_NUMBER", PUBLIC | STATIC, None);
static F_NUMBER_CLASS_NOT_SUPPORTED_BY_NUMERIC_RANGE_QUERY: NlsField = NlsField::new(
    &CLASS,
    "NUMBER_CLASS_NOT_SUPPORTED_BY_NUMERIC_RANGE_QUERY",
    PUBLIC | STATIC,
    None,
);
static F_UNSUPPORTED_NUMERIC_DATA_TYPE: NlsField = NlsField::new(
    &CLASS,
    "UNSUPPORTED_NUMERIC_DATA_TYPE",
    PUBLIC | STATIC,
    None,
);
static F_NUMERIC_CANNOT_BE_EMPTY: NlsField =
    NlsField::new(&CLASS, "NUMERIC_CANNOT_BE_EMPTY", PUBLIC | STATIC, None);
static F_ANALYZER_REQUIRED: NlsField =
    NlsField::new(&CLASS, "ANALYZER_REQUIRED", PUBLIC | STATIC, None);

/// Flexible Query Parser message bundle class.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.core.messages.QueryParserMessages`,
/// which extends `NLS`. Its `public static String` message keys are the
/// associated constants; [`NlsField::get`] reads one (initializing the class
/// first, which registers the bundle with [`NLS`] and sets each key to its
/// own name, as `NLS.initializeMessages` does by reflection).
#[derive(Debug)]
pub struct QueryParserMessages {
    _do_not_instantiate: (),
}

impl QueryParserMessages {
    /// `public static String INVALID_SYNTAX`.
    pub const INVALID_SYNTAX: &'static NlsField = &F_INVALID_SYNTAX;
    /// `public static String INVALID_SYNTAX_CANNOT_PARSE`.
    pub const INVALID_SYNTAX_CANNOT_PARSE: &'static NlsField = &F_INVALID_SYNTAX_CANNOT_PARSE;
    /// `public static String INVALID_SYNTAX_FUZZY_LIMITS`.
    pub const INVALID_SYNTAX_FUZZY_LIMITS: &'static NlsField = &F_INVALID_SYNTAX_FUZZY_LIMITS;
    /// `public static String INVALID_SYNTAX_FUZZY_EDITS`.
    pub const INVALID_SYNTAX_FUZZY_EDITS: &'static NlsField = &F_INVALID_SYNTAX_FUZZY_EDITS;
    /// `public static String INVALID_SYNTAX_ESCAPE_UNICODE_TRUNCATION`.
    pub const INVALID_SYNTAX_ESCAPE_UNICODE_TRUNCATION: &'static NlsField =
        &F_INVALID_SYNTAX_ESCAPE_UNICODE_TRUNCATION;
    /// `public static String INVALID_SYNTAX_ESCAPE_CHARACTER`.
    pub const INVALID_SYNTAX_ESCAPE_CHARACTER: &'static NlsField =
        &F_INVALID_SYNTAX_ESCAPE_CHARACTER;
    /// `public static String INVALID_SYNTAX_ESCAPE_NONE_HEX_UNICODE`.
    pub const INVALID_SYNTAX_ESCAPE_NONE_HEX_UNICODE: &'static NlsField =
        &F_INVALID_SYNTAX_ESCAPE_NONE_HEX_UNICODE;
    /// `public static String NODE_ACTION_NOT_SUPPORTED`.
    pub const NODE_ACTION_NOT_SUPPORTED: &'static NlsField = &F_NODE_ACTION_NOT_SUPPORTED;
    /// `public static String PARAMETER_VALUE_NOT_SUPPORTED`.
    pub const PARAMETER_VALUE_NOT_SUPPORTED: &'static NlsField = &F_PARAMETER_VALUE_NOT_SUPPORTED;
    /// `public static String LUCENE_QUERY_CONVERSION_ERROR`.
    pub const LUCENE_QUERY_CONVERSION_ERROR: &'static NlsField = &F_LUCENE_QUERY_CONVERSION_ERROR;
    /// `public static String EMPTY_MESSAGE`.
    pub const EMPTY_MESSAGE: &'static NlsField = &F_EMPTY_MESSAGE;
    /// `public static String WILDCARD_NOT_SUPPORTED`.
    pub const WILDCARD_NOT_SUPPORTED: &'static NlsField = &F_WILDCARD_NOT_SUPPORTED;
    /// `public static String TOO_MANY_BOOLEAN_CLAUSES`.
    pub const TOO_MANY_BOOLEAN_CLAUSES: &'static NlsField = &F_TOO_MANY_BOOLEAN_CLAUSES;
    /// `public static String LEADING_WILDCARD_NOT_ALLOWED`.
    pub const LEADING_WILDCARD_NOT_ALLOWED: &'static NlsField = &F_LEADING_WILDCARD_NOT_ALLOWED;
    /// `public static String COULD_NOT_PARSE_NUMBER`.
    pub const COULD_NOT_PARSE_NUMBER: &'static NlsField = &F_COULD_NOT_PARSE_NUMBER;
    /// `public static String NUMBER_CLASS_NOT_SUPPORTED_BY_NUMERIC_RANGE_QUERY`.
    pub const NUMBER_CLASS_NOT_SUPPORTED_BY_NUMERIC_RANGE_QUERY: &'static NlsField =
        &F_NUMBER_CLASS_NOT_SUPPORTED_BY_NUMERIC_RANGE_QUERY;
    /// `public static String UNSUPPORTED_NUMERIC_DATA_TYPE`.
    pub const UNSUPPORTED_NUMERIC_DATA_TYPE: &'static NlsField = &F_UNSUPPORTED_NUMERIC_DATA_TYPE;
    /// `public static String NUMERIC_CANNOT_BE_EMPTY`.
    pub const NUMERIC_CANNOT_BE_EMPTY: &'static NlsField = &F_NUMERIC_CANNOT_BE_EMPTY;
    /// `public static String ANALYZER_REQUIRED`.
    pub const ANALYZER_REQUIRED: &'static NlsField = &F_ANALYZER_REQUIRED;
}
