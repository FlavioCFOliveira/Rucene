// SAX input source.
// http://www.saxproject.org
// No warranty; no copyright -- use this as you will.
// $Id: InputSource.java 226184 2005-04-08 10:53:24Z neeraj $

//! Port of `InputSource.java`, of the Java package `org.xml.sax` (SAX 2.0.2, xml-apis 1.4.01).

use crate::java::{InputStream, Reader};

/// A single input source for an XML entity.
///
/// Equivalent to `org.xml.sax.InputSource`; the streams are shared
/// references, as Java's.
#[derive(Clone, Default)]
pub struct InputSource {
    public_id: Option<String>,
    system_id: Option<String>,
    byte_stream: Option<InputStream>,
    encoding: Option<String>,
    character_stream: Option<Reader>,
}

impl std::fmt::Debug for InputSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputSource")
            .field("public_id", &self.public_id)
            .field("system_id", &self.system_id)
            .field("byte_stream", &self.byte_stream.is_some())
            .field("encoding", &self.encoding)
            .field("character_stream", &self.character_stream.is_some())
            .finish()
    }
}

impl InputSource {
    /// Equivalent to `new InputSource()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Equivalent to `new InputSource(String)`.
    pub fn with_system_id(system_id: Option<&str>) -> Self {
        let mut s = Self::default();
        s.set_system_id(system_id);
        s
    }

    /// Equivalent to `new InputSource(InputStream)`.
    pub fn with_byte_stream(byte_stream: Option<InputStream>) -> Self {
        let mut s = Self::default();
        s.set_byte_stream(byte_stream);
        s
    }

    /// Equivalent to `new InputSource(Reader)`.
    pub fn with_character_stream(character_stream: Option<Reader>) -> Self {
        let mut s = Self::default();
        s.set_character_stream(character_stream);
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

    /// Equivalent to `setByteStream(InputStream)`.
    pub fn set_byte_stream(&mut self, byte_stream: Option<InputStream>) {
        self.byte_stream = byte_stream;
    }

    /// Equivalent to `getByteStream()`.
    pub fn get_byte_stream(&self) -> Option<InputStream> {
        self.byte_stream.clone()
    }

    /// Equivalent to `setEncoding(String)`.
    pub fn set_encoding(&mut self, encoding: Option<&str>) {
        self.encoding = encoding.map(str::to_string);
    }

    /// Equivalent to `getEncoding()`.
    pub fn get_encoding(&self) -> Option<&str> {
        self.encoding.as_deref()
    }

    /// Equivalent to `setCharacterStream(Reader)`.
    pub fn set_character_stream(&mut self, character_stream: Option<Reader>) {
        self.character_stream = character_stream;
    }

    /// Equivalent to `getCharacterStream()`.
    pub fn get_character_stream(&self) -> Option<Reader> {
        self.character_stream.clone()
    }
}
