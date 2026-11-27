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

//! Port of `XSAttributeDeclaration.java`, of the Java package `org.apache.xerces.xs` (Xerces-J 2.12.2).

use std::rc::Rc;

use crate::java::JResult;
use crate::java::Object;
use crate::org::apache::xerces::xs::short_list::ShortList;
use crate::org::apache::xerces::xs::xs_annotation::XSAnnotation;
use crate::org::apache::xerces::xs::xs_complex_type_definition::XSComplexTypeDefinition;
use crate::org::apache::xerces::xs::xs_object::XSObject;
use crate::org::apache::xerces::xs::xs_object_list::XSObjectList;
use crate::org::apache::xerces::xs::xs_simple_type_definition::XSSimpleTypeDefinition;
use crate::org::apache::xerces::xs::xs_value::XSValue;

/// Equivalent to `org.apache.xerces.xs.XSAttributeDeclaration`.
pub trait XSAttributeDeclaration: XSObject {
    /// Equivalent to `getTypeDefinition()`.
    fn get_type_definition(&self) -> Option<Rc<dyn XSSimpleTypeDefinition>>;

    /// Equivalent to `getScope()`.
    fn get_scope(&self) -> i16;

    /// Equivalent to `getEnclosingCTDefinition()`.
    fn get_enclosing_ct_definition(&self) -> Option<Rc<dyn XSComplexTypeDefinition>>;

    /// Equivalent to `getConstraintType()`.
    fn get_constraint_type(&self) -> i16;

    /// Equivalent to `getConstraintValue()`.
    fn get_constraint_value(&self) -> Option<String>;

    /// Equivalent to `getActualVC()`.
    fn get_actual_vc(&self) -> JResult<Option<Object>>;

    /// Equivalent to `getActualVCType()`.
    fn get_actual_vctype(&self) -> JResult<i16>;

    /// Equivalent to `getItemValueTypes()`.
    fn get_item_value_types(&self) -> JResult<Option<Rc<dyn ShortList>>>;

    /// Equivalent to `getValueConstraintValue()`.
    fn get_value_constraint_value(&self) -> Option<Rc<dyn XSValue>>;

    /// Equivalent to `getAnnotation()`.
    fn get_annotation(&self) -> Option<Rc<dyn XSAnnotation>>;

    /// Equivalent to `getAnnotations()`.
    fn get_annotations(&self) -> Option<Rc<dyn XSObjectList>>;
}
