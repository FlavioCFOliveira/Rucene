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

//! Port of `XMLErrorHandler.java`, of the Java package `org.apache.xerces.xni.parser` (Xerces-J 2.12.2).

use std::any::Any;

use crate::java::JResult;
use crate::org::apache::xerces::xni::parser::xml_parse_exception::XMLParseException;

/// An interface for handling errors.
///
/// Equivalent to `org.apache.xerces.xni.parser.XMLErrorHandler`.
pub trait XMLErrorHandler: Any {
    /// Equivalent to `warning(String, String, XMLParseException)`.
    fn warning(&self, domain: &str, key: &str, exception: &XMLParseException) -> JResult<()>;
    /// Equivalent to `error(String, String, XMLParseException)`.
    fn error(&self, domain: &str, key: &str, exception: &XMLParseException) -> JResult<()>;
    /// Equivalent to `fatalError(String, String, XMLParseException)`.
    fn fatal_error(&self, domain: &str, key: &str, exception: &XMLParseException) -> JResult<()>;

    /// This handler as `Any`, for `instanceof` tests.
    fn as_any(&self) -> &dyn Any;
}
