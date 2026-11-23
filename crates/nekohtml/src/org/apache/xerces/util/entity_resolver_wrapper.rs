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

//! Port of `EntityResolverWrapper.java`, of the Java package `org.apache.xerces.util` (Xerces-J 2.12.2).

use std::cell::RefCell;
use std::rc::Rc;

use crate::java::{Exception, JResult};
use crate::org::apache::xerces::xni::parser::xml_entity_resolver::XMLEntityResolver;
use crate::org::apache::xerces::xni::parser::xml_input_source::XMLInputSource;
use crate::org::apache::xerces::xni::xml_resource_identifier::XMLResourceIdentifier;
use crate::org::apache::xerces::xni::xni_exception::XNIException;
use crate::org::xml::sax::entity_resolver::EntityResolver;

/// This class wraps a SAX entity resolver in an XNI entity resolver.
///
/// Equivalent to `org.apache.xerces.util.EntityResolverWrapper`.
#[derive(Default)]
pub struct EntityResolverWrapper {
    entity_resolver: RefCell<Option<Rc<dyn EntityResolver>>>,
}

impl std::fmt::Debug for EntityResolverWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EntityResolverWrapper")
    }
}

impl EntityResolverWrapper {
    /// Default constructor.
    ///
    /// Equivalent to `new EntityResolverWrapper()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Wraps the specified SAX entity resolver.
    ///
    /// Equivalent to `new EntityResolverWrapper(EntityResolver)`.
    pub fn with_resolver(entity_resolver: Option<Rc<dyn EntityResolver>>) -> Self {
        let s = Self::new();
        s.set_entity_resolver(entity_resolver);
        s
    }

    /// Sets the SAX entity resolver.
    ///
    /// Equivalent to `setEntityResolver(EntityResolver)`.
    pub fn set_entity_resolver(&self, entity_resolver: Option<Rc<dyn EntityResolver>>) {
        *self.entity_resolver.borrow_mut() = entity_resolver;
    }

    /// Returns the SAX entity resolver.
    ///
    /// Equivalent to `getEntityResolver()`.
    pub fn get_entity_resolver(&self) -> Option<Rc<dyn EntityResolver>> {
        self.entity_resolver.borrow().clone()
    }
}

impl XMLEntityResolver for EntityResolverWrapper {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn resolve_entity(&self, resource_identifier: &dyn XMLResourceIdentifier) -> JResult<Option<XMLInputSource>> {
        // When both pubId and sysId are null, the user's entity resolver
        // can do nothing about it. We'd better not bother calling it.
        let pub_id = resource_identifier.get_public_id();
        let sys_id = resource_identifier.get_expanded_system_id();
        if pub_id.is_none() && sys_id.is_none() {
            return Ok(None);
        }

        // resolve entity using SAX entity resolver
        if let Some(entity_resolver) = self.get_entity_resolver() {
            match entity_resolver.resolve_entity(pub_id.as_deref(), sys_id.as_deref()) {
                Ok(Some(input_source)) => {
                    let base_system_id = resource_identifier.get_base_system_id();
                    let mut xml_input_source = XMLInputSource::new(
                        input_source.get_public_id(),
                        input_source.get_system_id(),
                        base_system_id.as_deref(),
                    );
                    xml_input_source.set_byte_stream(input_source.get_byte_stream());
                    xml_input_source.set_character_stream(input_source.get_character_stream());
                    xml_input_source.set_encoding(input_source.get_encoding());
                    return Ok(Some(xml_input_source));
                }
                Ok(None) => {}
                // error resolving entity
                Err(Exception::Sax(e)) => {
                    let ex = e.get_exception().cloned().unwrap_or(Exception::Sax(e));
                    return Err(XNIException::with_exception(ex).into());
                }
                Err(e) => return Err(e),
            }
        }

        // use default resolution
        Ok(None)
    }
}
