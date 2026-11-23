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

//! Port of `XSTypeDefinition.java`, of the Java package `org.apache.xerces.xs` (Xerces-J 2.12.2).

use std::rc::Rc;

use crate::org::apache::xerces::xs::xs_object::XSObject;

/// Equivalent to `org.apache.xerces.xs.XSTypeDefinition`.
pub trait XSTypeDefinition: XSObject {
    /// Equivalent to `getTypeCategory()`.
    fn get_type_category(&self) -> i16;

    /// Equivalent to `getBaseType()`.
    fn get_base_type(&self) -> Option<Rc<dyn XSTypeDefinition>>;

    /// Equivalent to `isFinal(short)`.
    fn is_final(&self, restriction: i16) -> bool;

    /// Equivalent to `getFinal()`.
    fn get_final(&self) -> i16;

    /// Equivalent to `getAnonymous()`.
    fn get_anonymous(&self) -> bool;

    /// Equivalent to `derivedFromType(XSTypeDefinition, short)`.
    fn derived_from_type(&self, ancestor_type: Option<&dyn XSTypeDefinition>, derivation_method: i16) -> bool;

    /// Equivalent to `derivedFrom(String, String, short)`.
    fn derived_from(&self, namespace: Option<&str>, name: Option<&str>, derivation_method: i16) -> bool;
}

/// Equivalent to `XSTypeDefinition.COMPLEX_TYPE`.
pub const COMPLEX_TYPE: i16 = 15;

/// Equivalent to `XSTypeDefinition.SIMPLE_TYPE`.
pub const SIMPLE_TYPE: i16 = 16;
