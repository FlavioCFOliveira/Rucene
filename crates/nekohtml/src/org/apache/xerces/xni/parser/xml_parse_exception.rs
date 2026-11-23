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

//! Port of `XMLParseException.java`, of the Java package `org.apache.xerces.xni.parser` (Xerces-J 2.12.2).

use std::ops::Deref;

use crate::java::Exception;
use crate::org::apache::xerces::xni::xml_locator::XMLLocator;
use crate::org::apache::xerces::xni::xni_exception::{XMLParseLocation, XNIException, XNIExceptionClass};

/// A parsing exception. This exception is different from the standard XNI
/// exception in that it stores the location in the document (or its
/// entities) where the exception occurred.
///
/// Equivalent to `org.apache.xerces.xni.parser.XMLParseException`: an
/// [`XNIException`] of that class, which it dereferences to.
#[derive(Debug, Clone)]
pub struct XMLParseException(XNIException);

impl XMLParseException {
    fn location(locator: Option<&dyn XMLLocator>) -> XMLParseLocation {
        match locator {
            Some(l) => XMLParseLocation {
                public_id: l.get_public_id(),
                literal_system_id: l.get_literal_system_id(),
                expanded_system_id: l.get_expanded_system_id(),
                base_system_id: l.get_base_system_id(),
                line_number: l.get_line_number(),
                column_number: l.get_column_number(),
                character_offset: l.get_character_offset(),
            },
            None => XMLParseLocation {
                public_id: None,
                literal_system_id: None,
                expanded_system_id: None,
                base_system_id: None,
                line_number: -1,
                column_number: -1,
                character_offset: -1,
            },
        }
    }

    /// Equivalent to `new XMLParseException(XMLLocator, String)`.
    pub fn new(locator: Option<&dyn XMLLocator>, message: Option<&str>) -> Self {
        Self(XNIException::of(
            XNIExceptionClass::XMLParseException(Self::location(locator)),
            message.map(str::to_string),
            None,
        ))
    }

    /// Equivalent to `new XMLParseException(XMLLocator, String,
    /// Exception)`.
    pub fn with_exception(locator: Option<&dyn XMLLocator>, message: Option<&str>, exception: Option<Exception>) -> Self {
        Self(XNIException::of(
            XNIExceptionClass::XMLParseException(Self::location(locator)),
            message.map(str::to_string),
            exception,
        ))
    }

    /// `(XMLParseException) e` of an [`XNIException`] of that class.
    pub fn from_xni_exception(e: &XNIException) -> Option<Self> {
        match e.class() {
            XNIExceptionClass::XMLParseException(_) => Some(Self(e.clone())),
            _ => None,
        }
    }

    fn loc(&self) -> &XMLParseLocation {
        match self.0.class() {
            XNIExceptionClass::XMLParseException(l) => l,
            _ => unreachable!("INVARIANT: an XMLParseException has a location"),
        }
    }

    /// Equivalent to `getPublicId()`.
    pub fn get_public_id(&self) -> Option<&str> {
        self.loc().public_id.as_deref()
    }

    /// Equivalent to `getExpandedSystemId()`.
    pub fn get_expanded_system_id(&self) -> Option<&str> {
        self.loc().expanded_system_id.as_deref()
    }

    /// Equivalent to `getLiteralSystemId()`.
    pub fn get_literal_system_id(&self) -> Option<&str> {
        self.loc().literal_system_id.as_deref()
    }

    /// Equivalent to `getBaseSystemId()`.
    pub fn get_base_system_id(&self) -> Option<&str> {
        self.loc().base_system_id.as_deref()
    }

    /// Equivalent to `getLineNumber()`.
    pub fn get_line_number(&self) -> i32 {
        self.loc().line_number
    }

    /// Equivalent to `getColumnNumber()`.
    pub fn get_column_number(&self) -> i32 {
        self.loc().column_number
    }

    /// Equivalent to `getCharacterOffset()`.
    pub fn get_character_offset(&self) -> i32 {
        self.loc().character_offset
    }

    /// The exception as its base class.
    pub fn into_xni_exception(self) -> XNIException {
        self.0
    }
}

impl Deref for XMLParseException {
    type Target = XNIException;

    fn deref(&self) -> &XNIException {
        &self.0
    }
}

impl From<XMLParseException> for Exception {
    fn from(e: XMLParseException) -> Self {
        Exception::Xni(e.0)
    }
}
