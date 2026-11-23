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

//! Port of `XSSimpleTypeDefinition.java`, of the Java package `org.apache.xerces.xs` (Xerces-J 2.12.2).

use std::rc::Rc;

use crate::org::apache::xerces::xs::string_list::StringList;
use crate::org::apache::xerces::xs::xs_object::XSObject;
use crate::org::apache::xerces::xs::xs_object_list::XSObjectList;
use crate::org::apache::xerces::xs::xs_type_definition::XSTypeDefinition;

/// Equivalent to `org.apache.xerces.xs.XSSimpleTypeDefinition`.
pub trait XSSimpleTypeDefinition: XSTypeDefinition {
    /// Equivalent to `getVariety()`.
    fn get_variety(&self) -> i16;

    /// Equivalent to `getPrimitiveType()`.
    fn get_primitive_type(&self) -> Option<Rc<dyn XSSimpleTypeDefinition>>;

    /// Equivalent to `getBuiltInKind()`.
    fn get_built_in_kind(&self) -> i16;

    /// Equivalent to `getItemType()`.
    fn get_item_type(&self) -> Option<Rc<dyn XSSimpleTypeDefinition>>;

    /// Equivalent to `getMemberTypes()`.
    fn get_member_types(&self) -> Option<Rc<dyn XSObjectList>>;

    /// Equivalent to `getDefinedFacets()`.
    fn get_defined_facets(&self) -> i16;

    /// Equivalent to `isDefinedFacet(short)`.
    fn is_defined_facet(&self, facet_name: i16) -> bool;

    /// Equivalent to `getFixedFacets()`.
    fn get_fixed_facets(&self) -> i16;

    /// Equivalent to `isFixedFacet(short)`.
    fn is_fixed_facet(&self, facet_name: i16) -> bool;

    /// Equivalent to `getLexicalFacetValue(short)`.
    fn get_lexical_facet_value(&self, facet_name: i16) -> Option<String>;

    /// Equivalent to `getLexicalEnumeration()`.
    fn get_lexical_enumeration(&self) -> Option<Rc<dyn StringList>>;

    /// Equivalent to `getLexicalPattern()`.
    fn get_lexical_pattern(&self) -> Option<Rc<dyn StringList>>;

    /// Equivalent to `getOrdered()`.
    fn get_ordered(&self) -> i16;

    /// Equivalent to `getFinite()`.
    fn get_finite(&self) -> bool;

    /// Equivalent to `getBounded()`.
    fn get_bounded(&self) -> bool;

    /// Equivalent to `getNumeric()`.
    fn get_numeric(&self) -> bool;

    /// Equivalent to `getFacets()`.
    fn get_facets(&self) -> Option<Rc<dyn XSObjectList>>;

    /// Equivalent to `getMultiValueFacets()`.
    fn get_multi_value_facets(&self) -> Option<Rc<dyn XSObjectList>>;

    /// Equivalent to `getFacet(int)`.
    fn get_facet(&self, facet_type: i32) -> Option<Rc<dyn XSObject>>;

    /// Equivalent to `getAnnotations()`.
    fn get_annotations(&self) -> Option<Rc<dyn XSObjectList>>;
}

/// Equivalent to `XSSimpleTypeDefinition.VARIETY_ABSENT`.
pub const VARIETY_ABSENT: i16 = 0;

/// Equivalent to `XSSimpleTypeDefinition.VARIETY_ATOMIC`.
pub const VARIETY_ATOMIC: i16 = 1;

/// Equivalent to `XSSimpleTypeDefinition.VARIETY_LIST`.
pub const VARIETY_LIST: i16 = 2;

/// Equivalent to `XSSimpleTypeDefinition.VARIETY_UNION`.
pub const VARIETY_UNION: i16 = 3;

/// Equivalent to `XSSimpleTypeDefinition.FACET_NONE`.
pub const FACET_NONE: i16 = 0;

/// Equivalent to `XSSimpleTypeDefinition.FACET_LENGTH`.
pub const FACET_LENGTH: i16 = 1;

/// Equivalent to `XSSimpleTypeDefinition.FACET_MINLENGTH`.
pub const FACET_MINLENGTH: i16 = 2;

/// Equivalent to `XSSimpleTypeDefinition.FACET_MAXLENGTH`.
pub const FACET_MAXLENGTH: i16 = 4;

/// Equivalent to `XSSimpleTypeDefinition.FACET_PATTERN`.
pub const FACET_PATTERN: i16 = 8;

/// Equivalent to `XSSimpleTypeDefinition.FACET_WHITESPACE`.
pub const FACET_WHITESPACE: i16 = 16;

/// Equivalent to `XSSimpleTypeDefinition.FACET_MAXINCLUSIVE`.
pub const FACET_MAXINCLUSIVE: i16 = 32;

/// Equivalent to `XSSimpleTypeDefinition.FACET_MAXEXCLUSIVE`.
pub const FACET_MAXEXCLUSIVE: i16 = 64;

/// Equivalent to `XSSimpleTypeDefinition.FACET_MINEXCLUSIVE`.
pub const FACET_MINEXCLUSIVE: i16 = 128;

/// Equivalent to `XSSimpleTypeDefinition.FACET_MININCLUSIVE`.
pub const FACET_MININCLUSIVE: i16 = 256;

/// Equivalent to `XSSimpleTypeDefinition.FACET_TOTALDIGITS`.
pub const FACET_TOTALDIGITS: i16 = 512;

/// Equivalent to `XSSimpleTypeDefinition.FACET_FRACTIONDIGITS`.
pub const FACET_FRACTIONDIGITS: i16 = 1024;

/// Equivalent to `XSSimpleTypeDefinition.FACET_ENUMERATION`.
pub const FACET_ENUMERATION: i16 = 2048;

/// Equivalent to `XSSimpleTypeDefinition.ORDERED_FALSE`.
pub const ORDERED_FALSE: i16 = 0;

/// Equivalent to `XSSimpleTypeDefinition.ORDERED_PARTIAL`.
pub const ORDERED_PARTIAL: i16 = 1;

/// Equivalent to `XSSimpleTypeDefinition.ORDERED_TOTAL`.
pub const ORDERED_TOTAL: i16 = 2;
