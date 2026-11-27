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

//! Port of `Augmentations.java`, of the Java package `org.apache.xerces.xni` (Xerces-J 2.12.2).

use crate::java::Object;

/// The Augmentations interface defines a table of additional data that may
/// be passed along the document pipeline.
///
/// Equivalent to `org.apache.xerces.xni.Augmentations`; `keys()` returns the
/// keys in the enumeration order of the implementation.
pub trait Augmentations {
    /// Equivalent to `putItem(String, Object)`.
    fn put_item(&self, key: &str, item: Option<Object>) -> Option<Object>;
    /// Equivalent to `getItem(String)`.
    fn get_item(&self, key: &str) -> Option<Object>;
    /// Equivalent to `removeItem(String)`.
    fn remove_item(&self, key: &str) -> Option<Object>;
    /// Equivalent to `keys()`.
    fn keys(&self) -> Vec<String>;
    /// Equivalent to `removeAllItems()`.
    fn remove_all_items(&self);
}
