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

//! Port of `ParserException.java`, of the Java package
//! `org.apache.lucene.queryparser.xml`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::{JavaThrowable, LuceneError, LuceneErrorKind};

use crate::flexible::core::query_node_exception::throwable_to_string;
use crate::jdk::dom::SaxException;

/// The binary name of `ParserException`.
const CLASS: &str = "org.apache.lucene.queryparser.xml.ParserException";

/// Thrown when the xml queryparser encounters invalid syntax/configuration.
///
/// Equivalent to `org.apache.lucene.queryparser.xml.ParserException`, a
/// checked `Exception`; it is thrown as a [`LuceneError`] of kind
/// [`LuceneErrorKind::Throwable`].
#[derive(Debug, Clone, Default)]
pub struct ParserException {
    message: Option<String>,
    cause: Option<Arc<LuceneError>>,
}

/// `Throwable.toString()` of an exception the XML parser wraps: a SAX
/// exception's own form, else the class and message.
pub(crate) fn exception_to_string(error: &LuceneError) -> String {
    match SaxException::of(error) {
        Some(sax) => sax.to_java_string(),
        None => throwable_to_string(error),
    }
}

impl ParserException {
    /// Equivalent to `new ParserException()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Equivalent to `new ParserException(String message)`.
    pub fn with_message(message: impl Into<String>) -> Self {
        Self {
            message: Some(message.into()),
            cause: None,
        }
    }

    /// Equivalent to `new ParserException(String message, Throwable cause)`.
    pub fn with_message_and_cause(message: impl Into<String>, cause: LuceneError) -> Self {
        Self {
            message: Some(message.into()),
            cause: Some(Arc::new(cause)),
        }
    }

    /// Equivalent to `new ParserException(Throwable cause)`: the message is
    /// the cause's `toString()`.
    pub fn with_cause(cause: LuceneError) -> Self {
        Self {
            message: Some(exception_to_string(&cause)),
            cause: Some(Arc::new(cause)),
        }
    }

    /// Equivalent to `getMessage()`.
    pub fn get_message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// This exception as the [`LuceneError`] it is thrown as.
    pub fn error(self) -> LuceneError {
        LuceneError::from(LuceneErrorKind::Throwable(Arc::new(self)))
    }

    /// The `ParserException` `error` stands for, when it is one.
    pub fn of(error: &LuceneError) -> Option<&ParserException> {
        match error.kind() {
            LuceneErrorKind::Throwable(t) => t.as_any().downcast_ref(),
            _ => None,
        }
    }
}

impl JavaThrowable for ParserException {
    fn java_class(&self) -> &'static str {
        CLASS
    }

    fn get_message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    fn get_cause(&self) -> Option<&Arc<LuceneError>> {
        self.cause.as_ref()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
