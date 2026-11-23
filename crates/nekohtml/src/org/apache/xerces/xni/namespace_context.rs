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

//! Port of `NamespaceContext.java`, of the Java package `org.apache.xerces.xni` (Xerces-J 2.12.2).

/// Represents an interface to query namespace information.
///
/// Equivalent to `org.apache.xerces.xni.NamespaceContext`.
pub trait NamespaceContext {
    /// Equivalent to `pushContext()`.
    fn push_context(&self);
    /// Equivalent to `popContext()`.
    fn pop_context(&self);
    /// Equivalent to `declarePrefix(String, String)`.
    fn declare_prefix(&self, prefix: &str, uri: Option<&str>) -> bool;
    /// Equivalent to `getURI(String)`.
    fn get_uri(&self, prefix: &str) -> Option<String>;
    /// Equivalent to `getPrefix(String)`.
    fn get_prefix(&self, uri: Option<&str>) -> Option<String>;
    /// Equivalent to `getDeclaredPrefixCount()`.
    fn get_declared_prefix_count(&self) -> i32;
    /// Equivalent to `getDeclaredPrefixAt(int)`.
    fn get_declared_prefix_at(&self, index: i32) -> String;
    /// Equivalent to `getAllPrefixes()`.
    fn get_all_prefixes(&self) -> Vec<String>;
    /// Equivalent to `reset()`.
    fn reset(&self);
}

/// The XML Namespace ("http://www.w3.org/XML/1998/namespace").
///
/// Equivalent to `NamespaceContext.XML_URI`.
pub const XML_URI: &str = "http://www.w3.org/XML/1998/namespace";

/// XML Information Set REC: all namespace attributes (including those named
/// xmlns, whose [prefix] property has no value) have a namespace URI of
/// http://www.w3.org/2000/xmlns/.
///
/// Equivalent to `NamespaceContext.XMLNS_URI`.
pub const XMLNS_URI: &str = "http://www.w3.org/2000/xmlns/";
