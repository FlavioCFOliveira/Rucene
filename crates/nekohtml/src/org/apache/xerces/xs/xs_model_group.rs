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

//! Port of `XSModelGroup.java`, of the Java package `org.apache.xerces.xs` (Xerces-J 2.12.2).

use std::rc::Rc;

use crate::org::apache::xerces::xs::xs_annotation::XSAnnotation;
use crate::org::apache::xerces::xs::xs_object_list::XSObjectList;
use crate::org::apache::xerces::xs::xs_term::XSTerm;

/// Equivalent to `org.apache.xerces.xs.XSModelGroup`.
pub trait XSModelGroup: XSTerm {
    /// Equivalent to `getCompositor()`.
    fn get_compositor(&self) -> i16;

    /// Equivalent to `getParticles()`.
    fn get_particles(&self) -> Option<Rc<dyn XSObjectList>>;

    /// Equivalent to `getAnnotation()`.
    fn get_annotation(&self) -> Option<Rc<dyn XSAnnotation>>;

    /// Equivalent to `getAnnotations()`.
    fn get_annotations(&self) -> Option<Rc<dyn XSObjectList>>;
}

/// Equivalent to `XSModelGroup.COMPOSITOR_SEQUENCE`.
pub const COMPOSITOR_SEQUENCE: i16 = 1;

/// Equivalent to `XSModelGroup.COMPOSITOR_CHOICE`.
pub const COMPOSITOR_CHOICE: i16 = 2;

/// Equivalent to `XSModelGroup.COMPOSITOR_ALL`.
pub const COMPOSITOR_ALL: i16 = 3;
