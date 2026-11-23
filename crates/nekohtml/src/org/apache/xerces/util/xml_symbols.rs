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

//! Port of `XMLSymbols.java`, of the Java package `org.apache.xerces.util` (Xerces-J 2.12.2).

/// All internalized xml symbols. They can be compared using "==".
///
/// Equivalent to `org.apache.xerces.util.XMLSymbols` (its constants).
#[derive(Debug, Default)]
pub struct XMLSymbols;

/// The empty string.
pub const EMPTY_STRING: &str = "";
/// The internalized "xml" prefix.
pub const PREFIX_XML: &str = "xml";
/// The internalized "xmlns" prefix.
pub const PREFIX_XMLNS: &str = "xmlns";
/// Symbol: "ANY".
pub const F_ANY_SYMBOL: &str = "ANY";
/// Symbol: "CDATA".
pub const F_CDATA_SYMBOL: &str = "CDATA";
/// Symbol: "ID".
pub const F_ID_SYMBOL: &str = "ID";
/// Symbol: "IDREF".
pub const F_IDREF_SYMBOL: &str = "IDREF";
/// Symbol: "IDREFS".
pub const F_IDREFS_SYMBOL: &str = "IDREFS";
/// Symbol: "ENTITY".
pub const F_ENTITY_SYMBOL: &str = "ENTITY";
/// Symbol: "ENTITIES".
pub const F_ENTITIES_SYMBOL: &str = "ENTITIES";
/// Symbol: "NMTOKEN".
pub const F_NMTOKEN_SYMBOL: &str = "NMTOKEN";
/// Symbol: "NMTOKENS".
pub const F_NMTOKENS_SYMBOL: &str = "NMTOKENS";
/// Symbol: "NOTATION".
pub const F_NOTATION_SYMBOL: &str = "NOTATION";
/// Symbol: "ENUMERATION".
pub const F_ENUMERATION_SYMBOL: &str = "ENUMERATION";
/// Symbol: "#IMPLIED".
pub const F_IMPLIED_SYMBOL: &str = "#IMPLIED";
/// Symbol: "#REQUIRED".
pub const F_REQUIRED_SYMBOL: &str = "#REQUIRED";
/// Symbol: "#FIXED".
pub const F_FIXED_SYMBOL: &str = "#FIXED";
