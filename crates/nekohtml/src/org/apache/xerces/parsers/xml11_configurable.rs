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

//! Port of `XML11Configurable.java`, of the Java package `org.apache.xerces.parsers` (Xerces-J 2.12.2).

/// This interface is used to differentiate between parser configurations
/// that support XML 1.1 and those which do not.
///
/// Equivalent to `org.apache.xerces.parsers.XML11Configurable` (a marker
/// interface); a configuration states it through
/// `XMLParserConfiguration::is_xml11_configurable`.
pub trait XML11Configurable {}
