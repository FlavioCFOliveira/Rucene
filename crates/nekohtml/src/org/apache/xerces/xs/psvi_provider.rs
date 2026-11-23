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

//! Port of `PSVIProvider.java`, of the Java package `org.apache.xerces.xs` (Xerces-J 2.12.2).

use std::rc::Rc;

use crate::org::apache::xerces::xs::attribute_psvi::AttributePSVI;
use crate::org::apache::xerces::xs::element_psvi::ElementPSVI;

/// Equivalent to `org.apache.xerces.xs.PSVIProvider`.
pub trait PSVIProvider {
    /// Equivalent to `getElementPSVI()`.
    fn get_element_psvi(&self) -> Option<Rc<dyn ElementPSVI>>;

    /// Equivalent to `getAttributePSVI(int)`.
    fn get_attribute_psvi(&self, index: i32) -> Option<Rc<dyn AttributePSVI>>;

    /// Equivalent to `getAttributePSVIByName(String, String)`.
    fn get_attribute_psvi_by_name(&self, uri: Option<&str>, localname: Option<&str>) -> Option<Rc<dyn AttributePSVI>>;
}
