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

//! Port of `XMLString.java`, of the Java package `org.apache.xerces.xni` (Xerces-J 2.12.2).

use crate::java::from_chars;

/// This class is used as a structure to pass text contained in the
/// underlying character buffer of the scanner.
///
/// Equivalent to `org.apache.xerces.xni.XMLString`. **Divergence.** Java's
/// `ch` is a reference to a `char[]` that `setValues` aliases (often the
/// scanner's own buffer); here `ch` owns its units and `set_values` copies
/// the given slice, so an `XMLString` cannot see later writes to the array it
/// was set from. Every reader of an `XMLString` in the parser reads it during
/// the call that receives it, so the values seen are the same.
#[derive(Debug, Clone)]
pub struct XMLString {
    /// The character array; `None` is Java's `null`.
    pub ch: Option<Vec<u16>>,
    /// The offset into the character array.
    pub offset: i32,
    /// The length of characters from the offset.
    pub length: i32,
}

impl Default for XMLString {
    /// Equivalent to `new XMLString()`.
    fn default() -> Self {
        Self {
            ch: None,
            offset: 0,
            length: 0,
        }
    }
}

impl XMLString {
    /// Equivalent to `new XMLString()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Equivalent to `new XMLString(char[], int, int)`.
    pub fn with_values(ch: &[u16], offset: i32, length: i32) -> Self {
        let mut s = Self::new();
        s.set_values(ch, offset, length);
        s
    }

    /// Equivalent to `new XMLString(XMLString)`.
    pub fn from_string(string: &XMLString) -> Self {
        string.clone()
    }

    /// Equivalent to `setValues(char[], int, int)` (see the type's
    /// divergence).
    pub fn set_values(&mut self, ch: &[u16], offset: i32, length: i32) {
        self.ch = Some(ch.to_vec());
        self.offset = offset;
        self.length = length;
    }

    /// Equivalent to `setValues(XMLString)`.
    pub fn set_values_from(&mut self, s: &XMLString) {
        self.ch.clone_from(&s.ch);
        self.offset = s.offset;
        self.length = s.length;
    }

    /// Equivalent to `clear()`.
    pub fn clear(&mut self) {
        self.ch = None;
        self.offset = 0;
        self.length = -1;
    }

    /// The `length` characters from `offset`.
    ///
    /// # Panics
    ///
    /// When `ch` is `None` or the range is out of bounds, as Java's array
    /// access throws.
    pub fn chars(&self) -> &[u16] {
        let ch = self.ch.as_deref().expect("java.lang.NullPointerException: XMLString.ch");
        &ch[self.offset as usize..(self.offset + self.length.max(0)) as usize]
    }

    /// Equivalent to `equals(char[], int, int)`.
    pub fn equals_chars(&self, ch: Option<&[u16]>, offset: i32, length: i32) -> bool {
        let Some(ch) = ch else {
            return false;
        };
        if self.length != length {
            return false;
        }
        let own = self.ch.as_deref().unwrap_or(&[]);
        for i in 0..length {
            if own[(self.offset + i) as usize] != ch[(offset + i) as usize] {
                return false;
            }
        }
        true
    }

    /// Equivalent to `equals(String)`.
    pub fn equals_str(&self, s: Option<&str>) -> bool {
        let Some(s) = s else {
            return false;
        };
        let s: Vec<u16> = s.encode_utf16().collect();
        if self.length != s.len() as i32 {
            return false;
        }
        let own = self.ch.as_deref().unwrap_or(&[]);
        for i in 0..self.length {
            if own[(self.offset + i) as usize] != s[i as usize] {
                return false;
            }
        }
        true
    }

    /// Equivalent to `toString()`.
    pub fn to_java_string(&self) -> String {
        if self.length > 0 {
            from_chars(self.chars())
        } else {
            String::new()
        }
    }
}

impl std::fmt::Display for XMLString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_java_string())
    }
}
