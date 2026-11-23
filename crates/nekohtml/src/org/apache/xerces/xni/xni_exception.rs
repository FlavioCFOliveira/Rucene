/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `XNIException.java`, of the Java package `org.apache.xerces.xni` (Xerces-J 2.12.2).

use crate::java::{throwable_to_string, Exception};

/// The class of an [`XNIException`]: the base class or one of its
/// subclasses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XNIExceptionClass {
    /// `org.apache.xerces.xni.XNIException`.
    XNIException,
    /// `org.apache.xerces.xni.parser.XMLParseException`, with its location.
    XMLParseException(XMLParseLocation),
    /// `org.apache.xerces.xni.parser.XMLConfigurationException`.
    XMLConfigurationException {
        /// `fType`: `NOT_RECOGNIZED` or `NOT_SUPPORTED`.
        type_: i16,
        /// `fIdentifier`.
        identifier: Option<String>,
    },
}

/// The location fields of an `XMLParseException`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XMLParseLocation {
    /// `fPublicId`.
    pub public_id: Option<String>,
    /// `fLiteralSystemId`.
    pub literal_system_id: Option<String>,
    /// `fExpandedSystemId`.
    pub expanded_system_id: Option<String>,
    /// `fBaseSystemId`.
    pub base_system_id: Option<String>,
    /// `fLineNumber`.
    pub line_number: i32,
    /// `fColumnNumber`.
    pub column_number: i32,
    /// `fCharacterOffset`.
    pub character_offset: i32,
}

/// This exception is the base exception of all XNI exceptions. It can be
/// constructed with an error message or used to wrap another exception
/// object.
///
/// Equivalent to `org.apache.xerces.xni.XNIException`;
/// [`class`](Self::class) tells which subclass an instance is.
#[derive(Debug, Clone)]
pub struct XNIException {
    class: XNIExceptionClass,
    message: Option<String>,
    exception: Option<Box<Exception>>,
}

impl XNIException {
    /// Equivalent to `new XNIException(String)`.
    pub fn with_message(message: Option<&str>) -> Self {
        Self::of(
            XNIExceptionClass::XNIException,
            message.map(str::to_string),
            None,
        )
    }

    /// Equivalent to `new XNIException(Exception)`.
    pub fn with_exception(exception: Exception) -> Self {
        let message = exception.get_message();
        Self::of(XNIExceptionClass::XNIException, message, Some(exception))
    }

    /// Equivalent to `new XNIException(String, Exception)`.
    pub fn with_message_and_exception(message: Option<&str>, exception: Option<Exception>) -> Self {
        Self::of(
            XNIExceptionClass::XNIException,
            message.map(str::to_string),
            exception,
        )
    }

    /// An XNI exception of class `class`.
    pub(crate) fn of(
        class: XNIExceptionClass,
        message: Option<String>,
        exception: Option<Exception>,
    ) -> Self {
        Self {
            class,
            message,
            exception: exception.map(Box::new),
        }
    }

    /// The class of this exception (`instanceof`).
    pub fn class(&self) -> &XNIExceptionClass {
        &self.class
    }

    /// `getClass().getName()`.
    pub fn class_name(&self) -> &'static str {
        match self.class {
            XNIExceptionClass::XNIException => "org.apache.xerces.xni.XNIException",
            XNIExceptionClass::XMLParseException(_) => {
                "org.apache.xerces.xni.parser.XMLParseException"
            }
            XNIExceptionClass::XMLConfigurationException { .. } => {
                "org.apache.xerces.xni.parser.XMLConfigurationException"
            }
        }
    }

    /// `Throwable.getMessage()`.
    pub fn get_message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Returns the wrapped exception.
    ///
    /// Equivalent to `getException()` (and `getCause()`).
    pub fn get_exception(&self) -> Option<&Exception> {
        self.exception.as_deref()
    }

    /// Equivalent to `initCause(Throwable)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` when a cause is already set.
    pub fn init_cause(&mut self, throwable: Exception) -> crate::java::JResult<()> {
        if self.exception.is_some() {
            return Err(Exception::runtime(
                rucene::error::LuceneError::illegal_state(""),
            ));
        }
        self.exception = Some(Box::new(throwable));
        Ok(())
    }

    /// `toString()`: `Throwable.toString()`, or `XMLParseException`'s
    /// override.
    pub fn to_java_string(&self) -> String {
        match &self.class {
            XNIExceptionClass::XMLParseException(l) => {
                let mut str = String::new();
                if let Some(v) = &l.public_id {
                    str.push_str(v);
                }
                str.push(':');
                if let Some(v) = &l.literal_system_id {
                    str.push_str(v);
                }
                str.push(':');
                if let Some(v) = &l.expanded_system_id {
                    str.push_str(v);
                }
                str.push(':');
                if let Some(v) = &l.base_system_id {
                    str.push_str(v);
                }
                str.push(':');
                str.push_str(&l.line_number.to_string());
                str.push(':');
                str.push_str(&l.column_number.to_string());
                str.push(':');
                str.push_str(&l.character_offset.to_string());
                str.push(':');
                let mut message = self.message.clone();
                if message.is_none() {
                    if let Some(exception) = self.get_exception() {
                        message = exception.get_message();
                    }
                }
                if let Some(m) = message {
                    str.push_str(&m);
                }
                str
            }
            _ => throwable_to_string(self.class_name(), self.message.as_deref()),
        }
    }
}

impl std::fmt::Display for XNIException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_java_string())
    }
}
