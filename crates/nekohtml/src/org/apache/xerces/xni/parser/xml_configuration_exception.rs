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

//! Port of `XMLConfigurationException.java`, of the Java package `org.apache.xerces.xni.parser` (Xerces-J 2.12.2).

use crate::java::Exception;
use crate::org::apache::xerces::xni::xni_exception::{XNIException, XNIExceptionClass};

/// An XNI parser configuration exception.
///
/// Equivalent to `org.apache.xerces.xni.parser.XMLConfigurationException`:
/// its constructors build an [`XNIException`] of that class.
#[derive(Debug)]
pub struct XMLConfigurationException;

/// Exception type: identifier not recognized.
pub const NOT_RECOGNIZED: i16 = 0;
/// Exception type: identifier not supported.
pub const NOT_SUPPORTED: i16 = 1;

impl XMLConfigurationException {
    /// Equivalent to `new XMLConfigurationException(short, String)`.
    pub fn new(type_: i16, identifier: &str) -> XNIException {
        XNIException::of(
            XNIExceptionClass::XMLConfigurationException {
                type_,
                identifier: Some(identifier.to_string()),
            },
            Some(identifier.to_string()),
            None,
        )
    }

    /// Equivalent to `new XMLConfigurationException(short, String,
    /// String)`.
    pub fn with_message(type_: i16, identifier: &str, message: Option<&str>) -> XNIException {
        XNIException::of(
            XNIExceptionClass::XMLConfigurationException {
                type_,
                identifier: Some(identifier.to_string()),
            },
            message.map(str::to_string),
            None,
        )
    }

    /// `e instanceof XMLConfigurationException ? (type, identifier) :
    /// null`: `getType()` and `getIdentifier()` of a configuration
    /// exception.
    pub fn of(e: &Exception) -> Option<(i16, Option<&str>)> {
        match e {
            Exception::Xni(x) => match x.class() {
                XNIExceptionClass::XMLConfigurationException { type_, identifier } => {
                    Some((*type_, identifier.as_deref()))
                }
                _ => None,
            },
            _ => None,
        }
    }
}
