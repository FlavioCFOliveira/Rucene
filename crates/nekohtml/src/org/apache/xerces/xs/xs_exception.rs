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

//! Port of `XSException.java`, of the Java package `org.apache.xerces.xs` (Xerces-J 2.12.2).

use rucene::error::LuceneError;

use crate::java::Exception;

/// The XML Schema API operations only raise exceptions in "exceptional"
/// circumstances.
///
/// Equivalent to `org.apache.xerces.xs.XSException` (a
/// `RuntimeException`; [`Exception::from`] gives it as one, with its
/// message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XSException {
    /// `code`.
    pub code: i16,
    message: Option<String>,
}

impl XSException {
    /// Equivalent to `new XSException(short, String)`.
    pub fn new(code: i16, message: Option<&str>) -> Self {
        Self {
            code,
            message: message.map(str::to_string),
        }
    }

    /// `getMessage()`.
    pub fn get_message(&self) -> Option<&str> {
        self.message.as_deref()
    }
}

impl From<XSException> for Exception {
    fn from(e: XSException) -> Self {
        Exception::runtime(LuceneError::unchecked(
            "org.apache.xerces.xs.XSException",
            e.message.unwrap_or_default(),
        ))
    }
}

/// If the implementation does not support the requested type of object or
/// operation.
pub const NOT_SUPPORTED_ERR: i16 = 1;
/// If index or size is negative, or greater than the allowed value.
pub const INDEX_SIZE_ERR: i16 = 2;
