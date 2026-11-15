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

//! Port of `MessageImpl.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.messages`.

use rucene::error::{LuceneError, Result};

use super::message::Message;
use super::nls::NLS;
use crate::jdk::locale::Locale;
use crate::jdk::object::JavaObject;

/// Default implementation of Message interface. For Native Language Support
/// (NLS), system of software internationalization.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.messages.MessageImpl`.
#[derive(Debug, Clone)]
pub struct MessageImpl {
    key: Option<String>,
    arguments: Option<Vec<JavaObject>>,
}

impl MessageImpl {
    /// Equivalent to `new MessageImpl(String key)`: no arguments (an empty
    /// array).
    pub fn new(key: impl Into<Option<String>>) -> Self {
        Self {
            key: key.into(),
            arguments: Some(Vec::new()),
        }
    }

    /// Equivalent to `new MessageImpl(String key, Object... args)`; `None`
    /// is a `null` array.
    pub fn with_args(
        key: impl Into<Option<String>>,
        args: impl Into<Option<Vec<JavaObject>>>,
    ) -> Self {
        Self {
            key: key.into(),
            arguments: args.into(),
        }
    }
}

impl Message for MessageImpl {
    fn get_key(&self) -> Option<&str> {
        self.key.as_deref()
    }

    fn get_arguments(&self) -> Option<&[JavaObject]> {
        self.arguments.as_deref()
    }

    fn get_localized_message(&self) -> Result<String> {
        self.get_localized_message_with_locale(&Locale::get_default())
    }

    fn get_localized_message_with_locale(&self, locale: &Locale) -> Result<String> {
        NLS::get_localized_message_with_locale_and_args(
            self.get_key(),
            locale,
            self.get_arguments(),
        )
    }

    /// `toString()`: the key, then the arguments, the first after a space
    /// and the others after `", "`, each as `String.valueOf` renders it.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for a `null` key (`new StringBuilder(null)`).
    fn to_java_string(&self) -> Result<String> {
        let args = self.get_arguments();
        let mut sb = self
            .get_key()
            .ok_or_else(LuceneError::null_pointer)?
            .to_string();
        if let Some(args) = args {
            for (i, arg) in args.iter().enumerate() {
                sb.push_str(if i == 0 { " " } else { ", " });
                sb.push_str(&arg.to_java_string());
            }
        }
        Ok(sb)
    }
}
