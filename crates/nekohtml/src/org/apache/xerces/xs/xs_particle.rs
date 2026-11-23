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

//! Port of `XSParticle.java`, of the Java package `org.apache.xerces.xs` (Xerces-J 2.12.2).

use std::rc::Rc;

use crate::org::apache::xerces::xs::xs_object::XSObject;
use crate::org::apache::xerces::xs::xs_object_list::XSObjectList;
use crate::org::apache::xerces::xs::xs_term::XSTerm;

/// Equivalent to `org.apache.xerces.xs.XSParticle`.
pub trait XSParticle: XSObject {
    /// Equivalent to `getMinOccurs()`.
    fn get_min_occurs(&self) -> i32;

    /// Equivalent to `getMaxOccurs()`.
    fn get_max_occurs(&self) -> i32;

    /// Equivalent to `getMaxOccursUnbounded()`.
    fn get_max_occurs_unbounded(&self) -> bool;

    /// Equivalent to `getTerm()`.
    fn get_term(&self) -> Option<Rc<dyn XSTerm>>;

    /// Equivalent to `getAnnotations()`.
    fn get_annotations(&self) -> Option<Rc<dyn XSObjectList>>;
}
