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

//! Port of `SAXParser.java`, of the Java package `org.cyberneko.html.parsers` (NekoHTML 1.9.17).

use std::ops::Deref;
use std::rc::Rc;

use crate::org::apache::xerces::parsers::abstract_sax_parser::AbstractSAXParser;
use crate::org::cyberneko::html::html_configuration::HTMLConfiguration;

/// A SAX parser for HTML documents.
///
/// Equivalent to `org.cyberneko.html.parsers.SAXParser` (an
/// `AbstractSAXParser` over an `HTMLConfiguration`, which it dereferences
/// to).
#[derive(Debug, Clone)]
pub struct SAXParser {
    parser: Rc<AbstractSAXParser>,
}

impl SAXParser {
    /// Default constructor.
    ///
    /// Equivalent to `new SAXParser()`.
    pub fn new() -> Self {
        Self {
            parser: AbstractSAXParser::new(HTMLConfiguration::new()),
        }
    }

    /// The parser as the shared `AbstractSAXParser` it is.
    pub fn as_abstract_sax_parser(&self) -> &Rc<AbstractSAXParser> {
        &self.parser
    }
}

impl Default for SAXParser {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for SAXParser {
    type Target = AbstractSAXParser;

    fn deref(&self) -> &AbstractSAXParser {
        &self.parser
    }
}
