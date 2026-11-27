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

//! Port of `QName.java`, of the Java package `org.apache.xerces.xni` (Xerces-J 2.12.2).

use crate::java::string_hash_code;

/// A structure that holds the components of an XML Namespaces qualified
/// name.
///
/// Equivalent to `org.apache.xerces.xni.QName`; `clone()` is [`Clone`].
#[derive(Debug, Clone, Default)]
pub struct QName {
    /// The qname prefix. For example, the prefix for the qname "a:foo" is
    /// "a".
    pub prefix: Option<String>,
    /// The qname localpart. For example, the localpart for the qname "a:foo"
    /// is "foo".
    pub localpart: Option<String>,
    /// The qname rawname. For example, the rawname for the qname "a:foo" is
    /// "a:foo".
    pub rawname: Option<String>,
    /// The URI to which the qname prefix is bound.
    pub uri: Option<String>,
}

impl QName {
    /// Equivalent to `new QName()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Equivalent to `new QName(String, String, String, String)`.
    pub fn with_values(
        prefix: Option<&str>,
        localpart: Option<&str>,
        rawname: Option<&str>,
        uri: Option<&str>,
    ) -> Self {
        let mut q = Self::new();
        q.set_values(prefix, localpart, rawname, uri);
        q
    }

    /// Equivalent to `setValues(QName)`.
    pub fn set_values_from(&mut self, qname: &QName) {
        self.prefix.clone_from(&qname.prefix);
        self.localpart.clone_from(&qname.localpart);
        self.rawname.clone_from(&qname.rawname);
        self.uri.clone_from(&qname.uri);
    }

    /// Equivalent to `setValues(String, String, String, String)`.
    pub fn set_values(
        &mut self,
        prefix: Option<&str>,
        localpart: Option<&str>,
        rawname: Option<&str>,
        uri: Option<&str>,
    ) {
        self.prefix = prefix.map(str::to_string);
        self.localpart = localpart.map(str::to_string);
        self.rawname = rawname.map(str::to_string);
        self.uri = uri.map(str::to_string);
    }

    /// Equivalent to `clear()`.
    pub fn clear(&mut self) {
        self.prefix = None;
        self.localpart = None;
        self.rawname = None;
        self.uri = None;
    }

    /// Equivalent to `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        if let Some(uri) = &self.uri {
            return string_hash_code(uri)
                .wrapping_add(self.localpart.as_deref().map_or(0, string_hash_code));
        }
        self.rawname.as_deref().map_or(0, string_hash_code)
    }

    /// Equivalent to `equals(Object)` (identity of interned symbols is
    /// equality here; see the crate documentation).
    pub fn equals(&self, qname: &QName) -> bool {
        if qname.uri.is_some() {
            return self.uri == qname.uri && self.localpart == qname.localpart;
        } else if self.uri.is_none() {
            return self.rawname == qname.rawname;
        }
        false
    }

    /// Equivalent to `toString()`.
    pub fn to_java_string(&self) -> String {
        let mut str = String::new();
        let mut comma = false;
        if let Some(prefix) = &self.prefix {
            str.push_str("prefix=\"");
            str.push_str(prefix);
            str.push('"');
            comma = true;
        }
        if let Some(localpart) = &self.localpart {
            if comma {
                str.push(',');
            }
            str.push_str("localpart=\"");
            str.push_str(localpart);
            str.push('"');
            comma = true;
        }
        if let Some(rawname) = &self.rawname {
            if comma {
                str.push(',');
            }
            str.push_str("rawname=\"");
            str.push_str(rawname);
            str.push('"');
            comma = true;
        }
        if let Some(uri) = &self.uri {
            if comma {
                str.push(',');
            }
            str.push_str("uri=\"");
            str.push_str(uri);
            str.push('"');
        }
        str
    }
}

impl PartialEq for QName {
    fn eq(&self, other: &Self) -> bool {
        self.equals(other)
    }
}

impl std::fmt::Display for QName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_java_string())
    }
}
