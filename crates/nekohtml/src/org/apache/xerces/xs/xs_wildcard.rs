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

//! Port of `XSWildcard.java`, of the Java package `org.apache.xerces.xs` (Xerces-J 2.12.2).

use std::rc::Rc;

use crate::org::apache::xerces::xs::string_list::StringList;
use crate::org::apache::xerces::xs::xs_annotation::XSAnnotation;
use crate::org::apache::xerces::xs::xs_object_list::XSObjectList;
use crate::org::apache::xerces::xs::xs_term::XSTerm;

/// Equivalent to `org.apache.xerces.xs.XSWildcard`.
pub trait XSWildcard: XSTerm {
    /// Equivalent to `getConstraintType()`.
    fn get_constraint_type(&self) -> i16;

    /// Equivalent to `getNsConstraintList()`.
    fn get_ns_constraint_list(&self) -> Option<Rc<dyn StringList>>;

    /// Equivalent to `getProcessContents()`.
    fn get_process_contents(&self) -> i16;

    /// Equivalent to `getAnnotation()`.
    fn get_annotation(&self) -> Option<Rc<dyn XSAnnotation>>;

    /// Equivalent to `getAnnotations()`.
    fn get_annotations(&self) -> Option<Rc<dyn XSObjectList>>;
}

/// Equivalent to `XSWildcard.NSCONSTRAINT_ANY`.
pub const NSCONSTRAINT_ANY: i16 = 1;

/// Equivalent to `XSWildcard.NSCONSTRAINT_NOT`.
pub const NSCONSTRAINT_NOT: i16 = 2;

/// Equivalent to `XSWildcard.NSCONSTRAINT_LIST`.
pub const NSCONSTRAINT_LIST: i16 = 3;

/// Equivalent to `XSWildcard.PC_STRICT`.
pub const PC_STRICT: i16 = 1;

/// Equivalent to `XSWildcard.PC_SKIP`.
pub const PC_SKIP: i16 = 2;

/// Equivalent to `XSWildcard.PC_LAX`.
pub const PC_LAX: i16 = 3;
