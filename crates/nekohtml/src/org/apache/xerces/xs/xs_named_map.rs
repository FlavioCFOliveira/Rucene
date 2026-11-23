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

//! Port of `XSNamedMap.java`, of the Java package `org.apache.xerces.xs` (Xerces-J 2.12.2).

use std::rc::Rc;

use crate::org::apache::xerces::xs::xs_object::XSObject;

/// Equivalent to `org.apache.xerces.xs.XSNamedMap`. Java's `XSNamedMap` also extends `java.util.Map`; the port
/// keeps the interface's own methods only (**Divergence**: the
/// `java.util` collection view is not part of the port, which never uses it).
pub trait XSNamedMap {
    /// Equivalent to `getLength()`.
    fn get_length(&self) -> i32;

    /// Equivalent to `item(int)`.
    fn item(&self, index: i32) -> Option<Rc<dyn XSObject>>;

    /// Equivalent to `itemByName(String, String)`.
    fn item_by_name(&self, namespace: Option<&str>, local_name: Option<&str>) -> Option<Rc<dyn XSObject>>;
}
