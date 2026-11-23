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

//! Port of `DefaultErrorHandler.java`, of the Java package `org.apache.xerces.util` (Xerces-J 2.12.2).

use std::any::Any;
use std::cell::RefCell;
use std::io::Write;

use crate::java::JResult;
use crate::org::apache::xerces::xni::parser::xml_error_handler::XMLErrorHandler;
use crate::org::apache::xerces::xni::parser::xml_parse_exception::XMLParseException;

/// Default error handler.
///
/// Equivalent to `org.apache.xerces.util.DefaultErrorHandler`; `fOut` (a
/// `PrintWriter`) is any writer, standard error by default. Write errors
/// are ignored, as `PrintWriter` swallows them.
pub struct DefaultErrorHandler {
    out: RefCell<Box<dyn Write>>,
}

impl std::fmt::Debug for DefaultErrorHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DefaultErrorHandler")
    }
}

impl Default for DefaultErrorHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultErrorHandler {
    /// Constructs an error handler that prints error messages to
    /// `System.err`.
    ///
    /// Equivalent to `new DefaultErrorHandler()`.
    pub fn new() -> Self {
        Self::with_writer(Box::new(std::io::stderr()))
    }

    /// Constructs an error handler that prints error messages to the
    /// specified `PrintWriter`.
    ///
    /// Equivalent to `new DefaultErrorHandler(PrintWriter)`.
    pub fn with_writer(out: Box<dyn Write>) -> Self {
        Self { out: RefCell::new(out) }
    }

    /// Prints the error message.
    fn print_error(&self, type_: &str, ex: &XMLParseException) {
        let mut text = String::new();
        text.push('[');
        text.push_str(type_);
        text.push_str("] ");
        if let Some(system_id) = ex.get_expanded_system_id() {
            let system_id = match system_id.rfind('/') {
                Some(index) => &system_id[index + 1..],
                None => system_id,
            };
            text.push_str(system_id);
        }
        text.push(':');
        text.push_str(&ex.get_line_number().to_string());
        text.push(':');
        text.push_str(&ex.get_column_number().to_string());
        text.push_str(": ");
        text.push_str(ex.get_message().unwrap_or("null"));
        text.push('\n');
        let mut out = self.out.borrow_mut();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }
}

impl XMLErrorHandler for DefaultErrorHandler {
    fn warning(&self, _domain: &str, _key: &str, ex: &XMLParseException) -> JResult<()> {
        self.print_error("Warning", ex);
        Ok(())
    }

    fn error(&self, _domain: &str, _key: &str, ex: &XMLParseException) -> JResult<()> {
        self.print_error("Error", ex);
        Ok(())
    }

    fn fatal_error(&self, _domain: &str, _key: &str, ex: &XMLParseException) -> JResult<()> {
        self.print_error("Fatal Error", ex);
        Err(ex.clone().into())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
