/* 
 * Copyright 2002-2009 Andy Clark, Marc Guillemot
 * 
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `HTMLErrorReporter.java`, of the Java package `org.cyberneko.html` (NekoHTML 1.9.17).

use rucene_queryparser::jdk::object::JavaObject;

use crate::java::JResult;

/// Defines an error reporter for reporting HTML errors.
///
/// Equivalent to `org.cyberneko.html.HTMLErrorReporter`; `Object[] args`
/// is `Option<&[JavaObject]>`.
pub trait HTMLErrorReporter {
    /// Format message without reporting error.
    ///
    /// Equivalent to `formatMessage(String, Object[])`.
    fn format_message(&self, key: &str, args: Option<&[JavaObject]>) -> String;

    /// Reports a warning.
    ///
    /// Equivalent to `reportWarning(String, Object[])`.
    fn report_warning(&self, key: &str, args: Option<&[JavaObject]>) -> JResult<()>;

    /// Reports an error.
    ///
    /// Equivalent to `reportError(String, Object[])`.
    fn report_error(&self, key: &str, args: Option<&[JavaObject]>) -> JResult<()>;
}
