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

//! Port of `Message.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.messages`.

use std::fmt;

use rucene::error::Result;

use crate::jdk::locale::Locale;
use crate::jdk::object::JavaObject;

/// Message Interface for a lazy loading. For Native Language Support (NLS),
/// system of software internationalization.
///
/// Equivalent to the interface
/// `org.apache.lucene.queryparser.flexible.messages.Message`; Java's
/// `toString()`, which every object has, is
/// [`to_java_string`](Message::to_java_string).
pub trait Message: fmt::Debug + Send + Sync {
    /// Equivalent to `getKey()`; `None` for Java's `null`.
    fn get_key(&self) -> Option<&str>;

    /// Equivalent to `getArguments()`; `None` for Java's `null` array.
    fn get_arguments(&self) -> Option<&[JavaObject]>;

    /// Equivalent to `getLocalizedMessage()`.
    ///
    /// # Errors
    ///
    /// The exceptions of the message lookup.
    fn get_localized_message(&self) -> Result<String>;

    /// Equivalent to `getLocalizedMessage(Locale locale)`.
    ///
    /// # Errors
    ///
    /// The exceptions of the message lookup.
    fn get_localized_message_with_locale(&self, locale: &Locale) -> Result<String>;

    /// Equivalent to `toString()`.
    ///
    /// # Errors
    ///
    /// The unchecked exceptions of the class's `toString()`.
    fn to_java_string(&self) -> Result<String>;
}
