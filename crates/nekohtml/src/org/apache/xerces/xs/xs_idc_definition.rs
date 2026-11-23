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

//! Port of `XSIDCDefinition.java`, of the Java package `org.apache.xerces.xs` (Xerces-J 2.12.2).

use std::rc::Rc;

use crate::org::apache::xerces::xs::string_list::StringList;
use crate::org::apache::xerces::xs::xs_object::XSObject;
use crate::org::apache::xerces::xs::xs_object_list::XSObjectList;

/// Equivalent to `org.apache.xerces.xs.XSIDCDefinition`.
pub trait XSIDCDefinition: XSObject {
    /// Equivalent to `getCategory()`.
    fn get_category(&self) -> i16;

    /// Equivalent to `getSelectorStr()`.
    fn get_selector_str(&self) -> Option<String>;

    /// Equivalent to `getFieldStrs()`.
    fn get_field_strs(&self) -> Option<Rc<dyn StringList>>;

    /// Equivalent to `getRefKey()`.
    fn get_ref_key(&self) -> Option<Rc<dyn XSIDCDefinition>>;

    /// Equivalent to `getAnnotations()`.
    fn get_annotations(&self) -> Option<Rc<dyn XSObjectList>>;
}

/// Equivalent to `XSIDCDefinition.IC_KEY`.
pub const IC_KEY: i16 = 1;

/// Equivalent to `XSIDCDefinition.IC_KEYREF`.
pub const IC_KEYREF: i16 = 2;

/// Equivalent to `XSIDCDefinition.IC_UNIQUE`.
pub const IC_UNIQUE: i16 = 3;
