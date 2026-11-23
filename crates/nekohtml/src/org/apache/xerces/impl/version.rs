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

//! Port of `Version.java`, of the Java package `org.apache.xerces.impl` (Xerces-J 2.12.2).

/// This class defines the version number of the parser.
///
/// Equivalent to `org.apache.xerces.impl.Version`. The source's
/// `@@VERSION@@` placeholder is the value the xercesImpl 2.12.2 build
/// substitutes (read from its `Version.class`).
#[derive(Debug, Default)]
pub struct Version;

/// Version string.
///
/// Equivalent to `Version.fVersion` (a mutable static in Java, never
/// reassigned by the parser).
pub const F_VERSION: &str = "Xerces-J 2.12.2";

/// Equivalent to `Version.fImmutableVersion`.
const F_IMMUTABLE_VERSION: &str = "Xerces-J 2.12.2";

impl Version {
    /// Returns the version.
    ///
    /// Equivalent to `getVersion()`.
    pub fn get_version() -> &'static str {
        F_IMMUTABLE_VERSION
    }

    /// Prints out the version number to System.out.
    ///
    /// Equivalent to `main(String[])`.
    pub fn main() {
        println!("{F_VERSION}");
    }
}
