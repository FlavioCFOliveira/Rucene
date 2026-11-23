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

//! Port of `ShortList.java`, of the Java package `org.apache.xerces.xs` (Xerces-J 2.12.2).

use crate::java::JResult;

/// Equivalent to `org.apache.xerces.xs.ShortList`. Java's `ShortList` also extends `java.util.List`; the port
/// keeps the interface's own methods only (**Divergence**: the
/// `java.util` collection view is not part of the port, which never uses it).
pub trait ShortList {
    /// Equivalent to `getLength()`.
    fn get_length(&self) -> i32;

    /// Equivalent to `contains(short)`.
    fn contains(&self, item: i16) -> bool;

    /// Equivalent to `item(int)`.
    fn item(&self, index: i32) -> JResult<i16>;
}
