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

//! Port of `XMLStringBuffer.java`, of the Java package `org.apache.xerces.util` (Xerces-J 2.12.2).

use std::ops::{Deref, DerefMut};

use crate::org::apache::xerces::xni::xml_string::XMLString;

/// XMLString is a structure used to pass character arrays. However, XMLStringBuffer
/// is a buffer in which characters can be appended and extends XMLString
/// so that it can be passed to methods expecting an XMLString object.
///
/// Equivalent to `org.apache.xerces.util.XMLStringBuffer`; it dereferences
/// to the [`XMLString`] it extends, whose `ch` always holds the buffer array.
#[derive(Debug, Clone)]
pub struct XMLStringBuffer {
    base: XMLString,
}

/// Default buffer size (32).
pub const DEFAULT_SIZE: i32 = 32;

impl Default for XMLStringBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl XMLStringBuffer {
    /// Equivalent to `new XMLStringBuffer()`.
    pub fn new() -> Self {
        Self::with_size(DEFAULT_SIZE)
    }

    /// Equivalent to `new XMLStringBuffer(int)`.
    pub fn with_size(size: i32) -> Self {
        Self {
            base: XMLString {
                ch: Some(vec![0; size.max(0) as usize]),
                offset: 0,
                length: 0,
            },
        }
    }

    /// Equivalent to `new XMLStringBuffer(char)`.
    pub fn with_char(c: u16) -> Self {
        let mut s = Self::with_size(1);
        s.append_char(c);
        s
    }

    /// Equivalent to `new XMLStringBuffer(String)`.
    pub fn with_string(s: &str) -> Self {
        let units: Vec<u16> = s.encode_utf16().collect();
        let mut b = Self::with_size(units.len() as i32);
        b.append_chars(&units, 0, units.len() as i32);
        b
    }

    /// Equivalent to `new XMLStringBuffer(char[], int, int)`.
    pub fn with_chars(ch: &[u16], offset: i32, length: i32) -> Self {
        let mut b = Self::with_size(length);
        b.append_chars(ch, offset, length);
        b
    }

    /// Equivalent to `new XMLStringBuffer(XMLString)`.
    pub fn with_xml_string(s: &XMLString) -> Self {
        let mut b = Self::with_size(s.length);
        b.append_xml_string(s);
        b
    }

    fn array(&mut self) -> &mut Vec<u16> {
        self.base.ch.get_or_insert_with(Vec::new)
    }

    /// Clears the string buffer.
    ///
    /// Equivalent to `clear()`.
    pub fn clear(&mut self) {
        self.base.offset = 0;
        self.base.length = 0;
    }

    fn ensure(&mut self, extra: i32, append_one: bool) {
        let length = self.base.length;
        let cap = self.array().len() as i32;
        if length + extra > cap {
            let mut new_length = cap * 2;
            if append_one {
                if new_length < cap + DEFAULT_SIZE {
                    new_length = cap + DEFAULT_SIZE;
                }
            } else if new_length < length + extra + DEFAULT_SIZE {
                new_length = cap + extra + DEFAULT_SIZE;
            }
            let array = self.array();
            let mut newch = vec![0u16; new_length as usize];
            newch[..length as usize].copy_from_slice(&array[..length as usize]);
            *array = newch;
        }
    }

    /// Equivalent to `append(char)`.
    pub fn append_char(&mut self, c: u16) {
        self.ensure(1, true);
        let length = self.base.length as usize;
        self.array()[length] = c;
        self.base.length += 1;
    }

    /// Equivalent to `append(String)`.
    pub fn append_str(&mut self, s: &str) {
        let units: Vec<u16> = s.encode_utf16().collect();
        let length = units.len() as i32;
        self.ensure(length, false);
        let start = self.base.length as usize;
        self.array()[start..start + units.len()].copy_from_slice(&units);
        self.base.length += length;
    }

    /// Equivalent to `append(char[], int, int)`.
    pub fn append_chars(&mut self, ch: &[u16], offset: i32, length: i32) {
        self.ensure(length, false);
        let start = self.base.length as usize;
        self.array()[start..start + length as usize]
            .copy_from_slice(&ch[offset as usize..(offset + length) as usize]);
        self.base.length += length;
    }

    /// Equivalent to `append(XMLString)`.
    pub fn append_xml_string(&mut self, s: &XMLString) {
        let chars = if s.length > 0 { s.chars().to_vec() } else { Vec::new() };
        self.append_chars(&chars, 0, s.length.max(0));
    }
}

impl Deref for XMLStringBuffer {
    type Target = XMLString;

    fn deref(&self) -> &XMLString {
        &self.base
    }
}

impl DerefMut for XMLStringBuffer {
    fn deref_mut(&mut self) -> &mut XMLString {
        &mut self.base
    }
}
