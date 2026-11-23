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

//! Port of `XMLInputSource.java`, of the Java package `org.apache.xerces.xni.parser` (Xerces-J 2.12.2).

use crate::java::{InputStream, Reader};
use crate::org::apache::xerces::xni::xml_resource_identifier::XMLResourceIdentifier;

/// This class represents an input source for an XML document.
///
/// Equivalent to `org.apache.xerces.xni.parser.XMLInputSource`; the streams
/// are shared references, as Java's.
#[derive(Clone, Default)]
pub struct XMLInputSource {
    public_id: Option<String>,
    system_id: Option<String>,
    base_system_id: Option<String>,
    byte_stream: Option<InputStream>,
    char_stream: Option<Reader>,
    encoding: Option<String>,
}

impl std::fmt::Debug for XMLInputSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XMLInputSource")
            .field("public_id", &self.public_id)
            .field("system_id", &self.system_id)
            .field("base_system_id", &self.base_system_id)
            .field("byte_stream", &self.byte_stream.is_some())
            .field("char_stream", &self.char_stream.is_some())
            .field("encoding", &self.encoding)
            .finish()
    }
}

impl XMLInputSource {
    /// Equivalent to `new XMLInputSource(String, String, String)`.
    pub fn new(
        public_id: Option<&str>,
        system_id: Option<&str>,
        base_system_id: Option<&str>,
    ) -> Self {
        Self {
            public_id: public_id.map(str::to_string),
            system_id: system_id.map(str::to_string),
            base_system_id: base_system_id.map(str::to_string),
            ..Self::default()
        }
    }

    /// Equivalent to `new XMLInputSource(XMLResourceIdentifier)`.
    pub fn from_resource_identifier(resource_identifier: &dyn XMLResourceIdentifier) -> Self {
        Self {
            public_id: resource_identifier.get_public_id(),
            system_id: resource_identifier.get_literal_system_id(),
            base_system_id: resource_identifier.get_base_system_id(),
            ..Self::default()
        }
    }

    /// Equivalent to `new XMLInputSource(String, String, String,
    /// InputStream, String)`.
    pub fn with_byte_stream(
        public_id: Option<&str>,
        system_id: Option<&str>,
        base_system_id: Option<&str>,
        byte_stream: Option<InputStream>,
        encoding: Option<&str>,
    ) -> Self {
        let mut s = Self::new(public_id, system_id, base_system_id);
        s.byte_stream = byte_stream;
        s.encoding = encoding.map(str::to_string);
        s
    }

    /// Equivalent to `new XMLInputSource(String, String, String, Reader,
    /// String)`.
    pub fn with_char_stream(
        public_id: Option<&str>,
        system_id: Option<&str>,
        base_system_id: Option<&str>,
        char_stream: Option<Reader>,
        encoding: Option<&str>,
    ) -> Self {
        let mut s = Self::new(public_id, system_id, base_system_id);
        s.char_stream = char_stream;
        s.encoding = encoding.map(str::to_string);
        s
    }

    /// Equivalent to `setPublicId(String)`.
    pub fn set_public_id(&mut self, public_id: Option<&str>) {
        self.public_id = public_id.map(str::to_string);
    }

    /// Equivalent to `getPublicId()`.
    pub fn get_public_id(&self) -> Option<&str> {
        self.public_id.as_deref()
    }

    /// Equivalent to `setSystemId(String)`.
    pub fn set_system_id(&mut self, system_id: Option<&str>) {
        self.system_id = system_id.map(str::to_string);
    }

    /// Equivalent to `getSystemId()`.
    pub fn get_system_id(&self) -> Option<&str> {
        self.system_id.as_deref()
    }

    /// Equivalent to `setBaseSystemId(String)`.
    pub fn set_base_system_id(&mut self, base_system_id: Option<&str>) {
        self.base_system_id = base_system_id.map(str::to_string);
    }

    /// Equivalent to `getBaseSystemId()`.
    pub fn get_base_system_id(&self) -> Option<&str> {
        self.base_system_id.as_deref()
    }

    /// Equivalent to `setByteStream(InputStream)`.
    pub fn set_byte_stream(&mut self, byte_stream: Option<InputStream>) {
        self.byte_stream = byte_stream;
    }

    /// Equivalent to `getByteStream()`.
    pub fn get_byte_stream(&self) -> Option<InputStream> {
        self.byte_stream.clone()
    }

    /// Equivalent to `setCharacterStream(Reader)`.
    pub fn set_character_stream(&mut self, char_stream: Option<Reader>) {
        self.char_stream = char_stream;
    }

    /// Equivalent to `getCharacterStream()`.
    pub fn get_character_stream(&self) -> Option<Reader> {
        self.char_stream.clone()
    }

    /// Equivalent to `setEncoding(String)`.
    pub fn set_encoding(&mut self, encoding: Option<&str>) {
        self.encoding = encoding.map(str::to_string);
    }

    /// Equivalent to `getEncoding()`.
    pub fn get_encoding(&self) -> Option<&str> {
        self.encoding.as_deref()
    }
}
