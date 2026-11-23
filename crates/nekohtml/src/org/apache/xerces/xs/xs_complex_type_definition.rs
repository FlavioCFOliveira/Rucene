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

//! Port of `XSComplexTypeDefinition.java`, of the Java package `org.apache.xerces.xs` (Xerces-J 2.12.2).

use std::rc::Rc;

use crate::org::apache::xerces::xs::xs_object_list::XSObjectList;
use crate::org::apache::xerces::xs::xs_particle::XSParticle;
use crate::org::apache::xerces::xs::xs_simple_type_definition::XSSimpleTypeDefinition;
use crate::org::apache::xerces::xs::xs_type_definition::XSTypeDefinition;
use crate::org::apache::xerces::xs::xs_wildcard::XSWildcard;

/// Equivalent to `org.apache.xerces.xs.XSComplexTypeDefinition`.
pub trait XSComplexTypeDefinition: XSTypeDefinition {
    /// Equivalent to `getDerivationMethod()`.
    fn get_derivation_method(&self) -> i16;

    /// Equivalent to `getAbstract()`.
    fn get_abstract(&self) -> bool;

    /// Equivalent to `getAttributeUses()`.
    fn get_attribute_uses(&self) -> Option<Rc<dyn XSObjectList>>;

    /// Equivalent to `getAttributeWildcard()`.
    fn get_attribute_wildcard(&self) -> Option<Rc<dyn XSWildcard>>;

    /// Equivalent to `getContentType()`.
    fn get_content_type(&self) -> i16;

    /// Equivalent to `getSimpleType()`.
    fn get_simple_type(&self) -> Option<Rc<dyn XSSimpleTypeDefinition>>;

    /// Equivalent to `getParticle()`.
    fn get_particle(&self) -> Option<Rc<dyn XSParticle>>;

    /// Equivalent to `isProhibitedSubstitution(short)`.
    fn is_prohibited_substitution(&self, restriction: i16) -> bool;

    /// Equivalent to `getProhibitedSubstitutions()`.
    fn get_prohibited_substitutions(&self) -> i16;

    /// Equivalent to `getAnnotations()`.
    fn get_annotations(&self) -> Option<Rc<dyn XSObjectList>>;
}

/// Equivalent to `XSComplexTypeDefinition.CONTENTTYPE_EMPTY`.
pub const CONTENTTYPE_EMPTY: i16 = 0;

/// Equivalent to `XSComplexTypeDefinition.CONTENTTYPE_SIMPLE`.
pub const CONTENTTYPE_SIMPLE: i16 = 1;

/// Equivalent to `XSComplexTypeDefinition.CONTENTTYPE_ELEMENT`.
pub const CONTENTTYPE_ELEMENT: i16 = 2;

/// Equivalent to `XSComplexTypeDefinition.CONTENTTYPE_MIXED`.
pub const CONTENTTYPE_MIXED: i16 = 3;
