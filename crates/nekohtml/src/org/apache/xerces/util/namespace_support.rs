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

//! Port of `NamespaceSupport.java`, of the Java package `org.apache.xerces.util` (Xerces-J 2.12.2).

use std::cell::RefCell;

use crate::org::apache::xerces::util::xml_symbols;
use crate::org::apache::xerces::xni::namespace_context::{NamespaceContext, XMLNS_URI, XML_URI};

/// Namespace support for XML document handlers. This class doesn't perform
/// any error checking and assumes that all strings are interned.
///
/// Equivalent to `org.apache.xerces.util.NamespaceSupport`.
#[derive(Debug)]
pub struct NamespaceSupport {
    state: RefCell<NamespaceState>,
}

#[derive(Debug)]
struct NamespaceState {
    /// Namespace binding information: `fNamespace`, pairs of prefix and URI.
    namespace: Vec<Option<String>>,
    /// Top of namespace information array: `fNamespaceSize`.
    namespace_size: i32,
    /// Context indexes: `fContext`.
    context: Vec<i32>,
    /// The current context: `fCurrentContext`.
    current_context: i32,
    /// `fPrefixes`.
    prefixes: Vec<Option<String>>,
}

impl Default for NamespaceSupport {
    fn default() -> Self {
        Self::new()
    }
}

impl NamespaceSupport {
    /// Default constructor.
    ///
    /// Equivalent to `new NamespaceSupport()`.
    pub fn new() -> Self {
        Self {
            state: RefCell::new(NamespaceState {
                namespace: vec![None; 16 * 2],
                namespace_size: 0,
                context: vec![0; 8],
                current_context: 0,
                prefixes: vec![None; 16],
            }),
        }
    }

    /// Constructs a namespace context object and initializes it with the
    /// prefixes declared in the specified context.
    ///
    /// Equivalent to `new NamespaceSupport(NamespaceContext)`.
    pub fn from_context(context: &dyn NamespaceContext) -> Self {
        let s = Self::new();
        s.push_context();
        for prefix in context.get_all_prefixes() {
            let uri = context.get_uri(&prefix);
            s.declare_prefix(&prefix, uri.as_deref());
        }
        s
    }

    /// Checks whether a binding or unbinding for the given prefix exists in
    /// the context.
    ///
    /// Equivalent to `containsPrefix(String)`.
    pub fn contains_prefix(&self, prefix: &str) -> bool {
        let s = self.state.borrow();
        let mut i = s.namespace_size;
        while i > 0 {
            if s.namespace[(i - 2) as usize].as_deref() == Some(prefix) {
                return true;
            }
            i -= 2;
        }
        false
    }
}

impl NamespaceContext for NamespaceSupport {
    fn reset(&self) {
        let mut s = self.state.borrow_mut();
        s.namespace_size = 0;
        s.current_context = 0;
        let cc = s.current_context as usize;
        s.context[cc] = s.namespace_size;
        for v in [
            xml_symbols::PREFIX_XML,
            XML_URI,
            xml_symbols::PREFIX_XMLNS,
            XMLNS_URI,
        ] {
            let i = s.namespace_size as usize;
            s.namespace[i] = Some(v.to_string());
            s.namespace_size += 1;
        }
        s.current_context += 1;
    }

    fn push_context(&self) {
        let mut s = self.state.borrow_mut();
        if (s.current_context + 1) as usize == s.context.len() {
            let len = s.context.len();
            s.context.resize(len * 2, 0);
        }
        s.current_context += 1;
        let cc = s.current_context as usize;
        s.context[cc] = s.namespace_size;
    }

    fn pop_context(&self) {
        let mut s = self.state.borrow_mut();
        let cc = s.current_context as usize;
        s.namespace_size = s.context[cc];
        s.current_context -= 1;
    }

    fn declare_prefix(&self, prefix: &str, uri: Option<&str>) -> bool {
        if prefix == xml_symbols::PREFIX_XML || prefix == xml_symbols::PREFIX_XMLNS {
            return false;
        }
        let mut s = self.state.borrow_mut();
        let start = s.context[s.current_context as usize];
        let mut i = s.namespace_size;
        while i > start {
            if s.namespace[(i - 2) as usize].as_deref() == Some(prefix) {
                s.namespace[(i - 1) as usize] = uri.map(str::to_string);
                return true;
            }
            i -= 2;
        }
        if s.namespace_size as usize == s.namespace.len() {
            let size = s.namespace_size as usize;
            s.namespace.resize(size * 2, None);
        }
        let i = s.namespace_size as usize;
        s.namespace[i] = Some(prefix.to_string());
        s.namespace[i + 1] = uri.map(str::to_string);
        s.namespace_size += 2;
        true
    }

    fn get_uri(&self, prefix: &str) -> Option<String> {
        let s = self.state.borrow();
        let mut i = s.namespace_size;
        while i > 0 {
            if s.namespace[(i - 2) as usize].as_deref() == Some(prefix) {
                return s.namespace[(i - 1) as usize].clone();
            }
            i -= 2;
        }
        None
    }

    fn get_prefix(&self, uri: Option<&str>) -> Option<String> {
        let candidates: Vec<Option<String>> = {
            let s = self.state.borrow();
            let mut v = Vec::new();
            let mut i = s.namespace_size;
            while i > 0 {
                if s.namespace[(i - 1) as usize].as_deref() == uri {
                    v.push(s.namespace[(i - 2) as usize].clone());
                }
                i -= 2;
            }
            v
        };
        for prefix in candidates {
            let p = prefix.as_deref().unwrap_or("");
            if self.get_uri(p).as_deref() == uri {
                return prefix;
            }
        }
        None
    }

    fn get_declared_prefix_count(&self) -> i32 {
        let s = self.state.borrow();
        (s.namespace_size - s.context[s.current_context as usize]) / 2
    }

    fn get_declared_prefix_at(&self, index: i32) -> String {
        let s = self.state.borrow();
        s.namespace[(s.context[s.current_context as usize] + index * 2) as usize]
            .clone()
            .unwrap_or_default()
    }

    fn get_all_prefixes(&self) -> Vec<String> {
        let mut s = self.state.borrow_mut();
        let mut count = 0usize;
        if s.prefixes.len() < s.namespace.len() / 2 {
            let size = s.namespace_size as usize;
            s.prefixes = vec![None; size];
        }
        let mut i = 2;
        while i < s.namespace_size - 2 {
            let prefix = s.namespace[(i + 2) as usize].clone();
            let unique = !s.prefixes[..count].contains(&prefix);
            if unique {
                s.prefixes[count] = prefix;
                count += 1;
            }
            i += 2;
        }
        s.prefixes[..count]
            .iter()
            .map(|p| p.clone().unwrap_or_default())
            .collect()
    }
}
