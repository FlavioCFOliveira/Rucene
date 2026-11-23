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

//! Port of `EntityResolver2Wrapper.java`, of the Java package `org.apache.xerces.util` (Xerces-J 2.12.2).

use std::cell::RefCell;
use std::rc::Rc;

use crate::java::{Exception, JResult};
use crate::org::apache::xerces::r#impl::external_subset_resolver::ExternalSubsetResolver;
use crate::org::apache::xerces::xni::grammars::xml_dtd_description::XMLDTDDescription;
use crate::org::apache::xerces::xni::parser::xml_entity_resolver::XMLEntityResolver;
use crate::org::apache::xerces::xni::parser::xml_input_source::XMLInputSource;
use crate::org::apache::xerces::xni::xml_resource_identifier::XMLResourceIdentifier;
use crate::org::apache::xerces::xni::xni_exception::XNIException;
use crate::org::xml::sax::entity_resolver::EntityResolver;
use crate::org::xml::sax::input_source::InputSource;

/// This class wraps a SAX entity resolver (EntityResolver2) in an XNI
/// entity resolver.
///
/// Equivalent to `org.apache.xerces.util.EntityResolver2Wrapper`; the
/// wrapped resolver is an [`EntityResolver`] whose
/// [`as_entity_resolver2`](EntityResolver::as_entity_resolver2) answers.
#[derive(Default)]
pub struct EntityResolver2Wrapper {
    entity_resolver: RefCell<Option<Rc<dyn EntityResolver>>>,
}

impl std::fmt::Debug for EntityResolver2Wrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EntityResolver2Wrapper")
    }
}

impl EntityResolver2Wrapper {
    /// Default constructor.
    ///
    /// Equivalent to `new EntityResolver2Wrapper()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new instance wrapping the given SAX entity resolver.
    ///
    /// Equivalent to `new EntityResolver2Wrapper(EntityResolver2)`.
    pub fn with_resolver(entity_resolver: Option<Rc<dyn EntityResolver>>) -> Self {
        let s = Self::new();
        s.set_entity_resolver(entity_resolver);
        s
    }

    /// Sets the SAX entity resolver wrapped by this object.
    ///
    /// Equivalent to `setEntityResolver(EntityResolver2)`.
    pub fn set_entity_resolver(&self, entity_resolver: Option<Rc<dyn EntityResolver>>) {
        *self.entity_resolver.borrow_mut() = entity_resolver;
    }

    /// Returns the SAX entity resolver wrapped by this object.
    ///
    /// Equivalent to `getEntityResolver()`.
    pub fn get_entity_resolver(&self) -> Option<Rc<dyn EntityResolver>> {
        self.entity_resolver.borrow().clone()
    }

    /// Creates an XMLInputSource from a SAX InputSource.
    fn create_xml_input_source(source: &InputSource, base_uri: Option<&str>) -> XMLInputSource {
        let mut xml_input_source = XMLInputSource::new(source.get_public_id(), source.get_system_id(), base_uri);
        xml_input_source.set_byte_stream(source.get_byte_stream());
        xml_input_source.set_character_stream(source.get_character_stream());
        xml_input_source.set_encoding(source.get_encoding());
        xml_input_source
    }

    fn wrap(e: Exception) -> Exception {
        match e {
            Exception::Sax(e) => {
                let ex = e.get_exception().cloned().unwrap_or(Exception::Sax(e));
                XNIException::with_exception(ex).into()
            }
            other => other,
        }
    }
}

impl ExternalSubsetResolver for EntityResolver2Wrapper {
    fn get_external_subset(&self, grammar_description: &dyn XMLDTDDescription) -> JResult<Option<XMLInputSource>> {
        if let Some(resolver) = self.get_entity_resolver() {
            if let Some(resolver2) = resolver.as_entity_resolver2() {
                let name = grammar_description.get_root_name();
                let base_uri = grammar_description.get_base_system_id();
                let input_source = resolver2
                    .get_external_subset(name.as_deref().unwrap_or(""), base_uri.as_deref())
                    .map_err(Self::wrap)?;
                return Ok(input_source.map(|s| Self::create_xml_input_source(&s, base_uri.as_deref())));
            }
        }
        Ok(None)
    }
}

impl XMLEntityResolver for EntityResolver2Wrapper {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn resolve_entity(&self, resource_identifier: &dyn XMLResourceIdentifier) -> JResult<Option<XMLInputSource>> {
        if let Some(resolver) = self.get_entity_resolver() {
            if let Some(resolver2) = resolver.as_entity_resolver2() {
                let pub_id = resource_identifier.get_public_id();
                let sys_id = resource_identifier.get_literal_system_id();
                let base_uri = resource_identifier.get_base_system_id();
                let name = if resource_identifier.is_dtd_description() {
                    Some("[dtd]".to_string())
                } else {
                    resource_identifier.as_entity_description().and_then(|d| d.get_entity_name())
                };

                // When both pubId and sysId are null, the user's entity
                // resolver can do nothing about it. We'd better not bother
                // calling it.
                if pub_id.is_none() && sys_id.is_none() {
                    return Ok(None);
                }

                // Resolve using EntityResolver2
                let input_source = resolver2
                    .resolve_entity2(name.as_deref(), pub_id.as_deref(), base_uri.as_deref(), sys_id.as_deref())
                    .map_err(Self::wrap)?;
                return Ok(input_source.map(|s| Self::create_xml_input_source(&s, base_uri.as_deref())));
            }
        }
        Ok(None)
    }
}
