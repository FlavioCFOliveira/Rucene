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

//! Port of `XSAnnotation.java`, of the Java package `org.apache.xerces.xs` (Xerces-J 2.12.2).

use crate::java::Object;
use crate::org::apache::xerces::xs::xs_object::XSObject;

/// Equivalent to `org.apache.xerces.xs.XSAnnotation`.
pub trait XSAnnotation: XSObject {
    /// Equivalent to `writeAnnotation(Object, short)`.
    fn write_annotation(&self, target: Option<Object>, target_type: i16) -> bool;

    /// Equivalent to `getAnnotationString()`.
    fn get_annotation_string(&self) -> Option<String>;
}

/// Equivalent to `XSAnnotation.W3C_DOM_ELEMENT`.
pub const W3C_DOM_ELEMENT: i16 = 1;

/// Equivalent to `XSAnnotation.SAX_CONTENTHANDLER`.
pub const SAX_CONTENTHANDLER: i16 = 2;

/// Equivalent to `XSAnnotation.W3C_DOM_DOCUMENT`.
pub const W3C_DOM_DOCUMENT: i16 = 3;
