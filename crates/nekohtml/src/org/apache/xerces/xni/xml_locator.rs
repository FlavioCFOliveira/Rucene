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

//! Port of `XMLLocator.java`, of the Java package `org.apache.xerces.xni` (Xerces-J 2.12.2).

/// Location information.
///
/// Equivalent to `org.apache.xerces.xni.XMLLocator`.
pub trait XMLLocator {
    /// Equivalent to `getPublicId()`.
    fn get_public_id(&self) -> Option<String>;
    /// Equivalent to `getLiteralSystemId()`.
    fn get_literal_system_id(&self) -> Option<String>;
    /// Equivalent to `getBaseSystemId()`.
    fn get_base_system_id(&self) -> Option<String>;
    /// Equivalent to `getExpandedSystemId()`.
    fn get_expanded_system_id(&self) -> Option<String>;
    /// Equivalent to `getLineNumber()`.
    fn get_line_number(&self) -> i32;
    /// Equivalent to `getColumnNumber()`.
    fn get_column_number(&self) -> i32;
    /// Equivalent to `getCharacterOffset()`.
    fn get_character_offset(&self) -> i32;
    /// Equivalent to `getEncoding()`.
    fn get_encoding(&self) -> Option<String>;
    /// Equivalent to `getXMLVersion()`.
    fn get_xml_version(&self) -> Option<String>;
}
