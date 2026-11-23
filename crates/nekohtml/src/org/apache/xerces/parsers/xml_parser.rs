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

//! Port of `XMLParser.java`, of the Java package `org.apache.xerces.parsers` (Xerces-J 2.12.2).

use std::rc::Rc;

use crate::java::JResult;
use crate::org::apache::xerces::r#impl::constants;
use crate::org::apache::xerces::xni::parser::xml_input_source::XMLInputSource;
use crate::org::apache::xerces::xni::parser::xml_parser_configuration::XMLParserConfiguration;

/// Property identifier: entity resolver.
pub const ENTITY_RESOLVER: &str = concatcp!(
    constants::XERCES_PROPERTY_PREFIX,
    constants::ENTITY_RESOLVER_PROPERTY
);

/// Property identifier: error handler.
pub const ERROR_HANDLER: &str = concatcp!(
    constants::XERCES_PROPERTY_PREFIX,
    constants::ERROR_HANDLER_PROPERTY
);

/// Recognized properties.
const RECOGNIZED_PROPERTIES: [&str; 2] = [ENTITY_RESOLVER, ERROR_HANDLER];

/// Base class of all XML-related parsers.
///
/// Equivalent to the abstract class `org.apache.xerces.parsers.XMLParser`:
/// its state, held by a subclass as its `base`. Its `parse(XMLInputSource)`
/// calls the subclass's `reset()`, so the subclass provides it.
pub struct XMLParser {
    /// The parser configuration.
    pub configuration: Rc<dyn XMLParserConfiguration>,
}

impl std::fmt::Debug for XMLParser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("XMLParser")
    }
}

impl XMLParser {
    /// Default Constructor.
    ///
    /// Equivalent to `XMLParser(XMLParserConfiguration)`.
    pub fn new(config: Rc<dyn XMLParserConfiguration>) -> Self {
        let props: Vec<String> = RECOGNIZED_PROPERTIES
            .iter()
            .map(|s| s.to_string())
            .collect();
        config.add_recognized_properties(Some(&props));
        Self {
            configuration: config,
        }
    }

    /// The body of `parse(XMLInputSource)` after `reset()`.
    ///
    /// # Errors
    ///
    /// What the configuration's parse throws.
    pub fn parse_after_reset(&self, input_source: &XMLInputSource) -> JResult<()> {
        self.configuration.parse(input_source)
    }

    /// Equivalent to the protected `reset()`: nothing at this level.
    pub fn reset(&self) {}
}
