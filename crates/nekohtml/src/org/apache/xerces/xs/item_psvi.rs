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

//! Port of `ItemPSVI.java`, of the Java package `org.apache.xerces.xs` (Xerces-J 2.12.2).

use std::rc::Rc;

use crate::java::JResult;
use crate::java::Object;
use crate::org::apache::xerces::xs::short_list::ShortList;
use crate::org::apache::xerces::xs::string_list::StringList;
use crate::org::apache::xerces::xs::xs_simple_type_definition::XSSimpleTypeDefinition;
use crate::org::apache::xerces::xs::xs_type_definition::XSTypeDefinition;
use crate::org::apache::xerces::xs::xs_value::XSValue;

/// Equivalent to `org.apache.xerces.xs.ItemPSVI`.
pub trait ItemPSVI {
    /// Equivalent to `constant()`.
    fn constant(&self) -> Option<Rc<dyn ItemPSVI>>;

    /// Equivalent to `isConstant()`.
    fn is_constant(&self) -> bool;

    /// Equivalent to `getValidationContext()`.
    fn get_validation_context(&self) -> Option<String>;

    /// Equivalent to `getValidity()`.
    fn get_validity(&self) -> i16;

    /// Equivalent to `getValidationAttempted()`.
    fn get_validation_attempted(&self) -> i16;

    /// Equivalent to `getErrorCodes()`.
    fn get_error_codes(&self) -> Option<Rc<dyn StringList>>;

    /// Equivalent to `getErrorMessages()`.
    fn get_error_messages(&self) -> Option<Rc<dyn StringList>>;

    /// Equivalent to `getSchemaNormalizedValue()`.
    fn get_schema_normalized_value(&self) -> Option<String>;

    /// Equivalent to `getActualNormalizedValue()`.
    fn get_actual_normalized_value(&self) -> JResult<Option<Object>>;

    /// Equivalent to `getActualNormalizedValueType()`.
    fn get_actual_normalized_value_type(&self) -> JResult<i16>;

    /// Equivalent to `getItemValueTypes()`.
    fn get_item_value_types(&self) -> JResult<Option<Rc<dyn ShortList>>>;

    /// Equivalent to `getSchemaValue()`.
    fn get_schema_value(&self) -> Option<Rc<dyn XSValue>>;

    /// Equivalent to `getTypeDefinition()`.
    fn get_type_definition(&self) -> Option<Rc<dyn XSTypeDefinition>>;

    /// Equivalent to `getMemberTypeDefinition()`.
    fn get_member_type_definition(&self) -> Option<Rc<dyn XSSimpleTypeDefinition>>;

    /// Equivalent to `getSchemaDefault()`.
    fn get_schema_default(&self) -> Option<String>;

    /// Equivalent to `getIsSchemaSpecified()`.
    fn get_is_schema_specified(&self) -> bool;
}

/// Equivalent to `ItemPSVI.VALIDITY_NOTKNOWN`.
pub const VALIDITY_NOTKNOWN: i16 = 0;

/// Equivalent to `ItemPSVI.VALIDITY_INVALID`.
pub const VALIDITY_INVALID: i16 = 1;

/// Equivalent to `ItemPSVI.VALIDITY_VALID`.
pub const VALIDITY_VALID: i16 = 2;

/// Equivalent to `ItemPSVI.VALIDATION_NONE`.
pub const VALIDATION_NONE: i16 = 0;

/// Equivalent to `ItemPSVI.VALIDATION_PARTIAL`.
pub const VALIDATION_PARTIAL: i16 = 1;

/// Equivalent to `ItemPSVI.VALIDATION_FULL`.
pub const VALIDATION_FULL: i16 = 2;
