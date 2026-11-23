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

//! Port of `XSValue.java`, of the Java package `org.apache.xerces.xs` (Xerces-J 2.12.2).

use std::rc::Rc;

use crate::java::Object;
use crate::org::apache::xerces::xs::short_list::ShortList;
use crate::org::apache::xerces::xs::xs_object_list::XSObjectList;
use crate::org::apache::xerces::xs::xs_simple_type_definition::XSSimpleTypeDefinition;

/// Equivalent to `org.apache.xerces.xs.XSValue`.
pub trait XSValue {
    /// Equivalent to `getNormalizedValue()`.
    fn get_normalized_value(&self) -> Option<String>;

    /// Equivalent to `getActualValue()`.
    fn get_actual_value(&self) -> Option<Object>;

    /// Equivalent to `getTypeDefinition()`.
    fn get_type_definition(&self) -> Option<Rc<dyn XSSimpleTypeDefinition>>;

    /// Equivalent to `getMemberTypeDefinition()`.
    fn get_member_type_definition(&self) -> Option<Rc<dyn XSSimpleTypeDefinition>>;

    /// Equivalent to `getMemberTypeDefinitions()`.
    fn get_member_type_definitions(&self) -> Option<Rc<dyn XSObjectList>>;

    /// Equivalent to `getActualValueType()`.
    fn get_actual_value_type(&self) -> i16;

    /// Equivalent to `getListValueTypes()`.
    fn get_list_value_types(&self) -> Option<Rc<dyn ShortList>>;
}
