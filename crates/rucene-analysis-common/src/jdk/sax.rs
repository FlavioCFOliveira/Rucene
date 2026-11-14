/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! The part of `org.xml.sax` (module `java.xml`) that `PatternParser`
//! (`analysis.compound.hyphenation`) uses: [`InputSource`], [`Attributes`],
//! the [`DefaultHandler`] callbacks it overrides, [`SaxException`], and the
//! `XMLReader` that `SAXParserFactory.newInstance()` with
//! `setNamespaceAware(true)` creates, as [`XmlReader`].
//!
//! # Divergence: a SAX-shaped reader over `quick-xml`
//!
//! Java parses with the JDK's own SAX parser (the Xerces-derived
//! `com.sun.org.apache.xerces.internal`). Rust has none; per the user's
//! decision on rmp task #461 the reader is a thin adapter over the
//! `quick-xml` pull parser that makes the handler calls the JDK parser makes,
//! observed on JDK 25.0.5 (`SaxProbe.java`, a handler that logs every call):
//!
//! * `resolveEntity(publicId, systemId)` for the external DTD subset named by
//!   the `DOCTYPE`, after the internal subset and before the root element,
//!   with the system id expanded against the document's own (or, without
//!   one, the working directory, `file:///<dir>/`); an exception it throws
//!   ends the parse unchanged, and the source it returns is read.
//! * `startElement(uri, localName, qName, attributes)` and `endElement` for
//!   every element (an empty element `<a/>` gives both), namespace-aware:
//!   `xmlns` attributes are not reported, a prefix with no binding is the
//!   JDK's fatal error.
//! * `characters` for the text and `CDATA` content of elements, after the
//!   XML line-end normalization, with the predefined entities, character
//!   references and the general entities of the internal subset expanded;
//!   attribute values are normalized as XML 1.0 section 3.3.3 says (each
//!   literal tab or line end becomes a space, references are expanded).
//!
//! What differs, and why:
//!
//! * **Chunking of `characters`.** The JDK splits a run of text at line ends
//!   and buffer boundaries (`"\naA\nbB"` then `"\ncC\ndD"`); the reader
//!   reports each run between markup and references in one call.
//!   `PatternParser`'s only `characters` logic, `readToken`, keeps a partial
//!   token across calls, so its consumer calls do not depend on the chunking.
//! * **The external DTD is read, not parsed.** Its declarations (attribute
//!   defaults, entities, element content models) are not applied: whitespace
//!   in element-only content is reported through `characters`, where the JDK
//!   reports it through `ignorableWhitespace`. `hyphenation.dtd`, the only
//!   DTD `PatternParser` resolves, declares no default and no entity, and
//!   `PatternParser` ignores text outside its elements, so its consumer calls
//!   do not change.
//! * **Entities.** Internal general entities (`<!ENTITY name "value">`) are
//!   expanded; a reference to an external general entity, and replacement
//!   text holding markup, are fatal errors here (the JDK resolves and parses
//!   them). Parameter entities are not expanded.
//! * **Errors.** A fatal error is a [`SaxException`] of class
//!   `org.xml.sax.SAXParseException` with the JDK's message for the errors
//!   listed in [`XmlReader::parse`]; any other well-formedness error carries
//!   `quick-xml`'s message, and its line and column are those of the
//!   position `quick-xml` reports. Character ranges (XML 1.0 section 2.2)
//!   and names are not validated.
//! * **Encodings.** The bytes are decoded with the JDK charset emulation of
//!   [`charset`](super::charset): the `InputSource` encoding when set,
//!   otherwise a byte-order mark, the UTF-16 forms of `<?`, or the `encoding`
//!   of the XML declaration (UTF-8 by default), as the JDK detects them.
//!   Malformed input is a fatal error with the JDK's message for UTF-8 and a
//!   generic one otherwise.
//! * **System ids** are opened as files: a `file:` URL or a path, relative
//!   ones against the working directory; other URL schemes are an
//!   `IOException`.

use std::collections::HashMap;
use std::fmt;
use std::io::Read;
use std::sync::Arc;

use quick_xml::errors::{Error as XmlError, IllFormedError};
use quick_xml::events::Event;
use quick_xml::name::{QName, ResolveResult};
use quick_xml::reader::NsReader;
use rucene::error::{JavaThrowable, LuceneError, LuceneErrorKind, Result};

use super::charset::Charset;
use super::io::io_exception;

/// A single input source for an XML entity.
///
/// Equivalent to `org.xml.sax.InputSource`: a public id, a system id, a byte
/// stream and an encoding (the character-stream form is not used by the
/// module). The reader reads the byte stream when there is one, otherwise it
/// opens the system id.
#[derive(Default)]
pub struct InputSource {
    public_id: Option<String>,
    system_id: Option<String>,
    byte_stream: Option<Box<dyn Read + Send>>,
    encoding: Option<String>,
}

impl fmt::Debug for InputSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InputSource")
            .field("public_id", &self.public_id)
            .field("system_id", &self.system_id)
            .field(
                "byte_stream",
                &self.byte_stream.as_ref().map(|_| "InputStream"),
            )
            .field("encoding", &self.encoding)
            .finish()
    }
}

impl InputSource {
    /// Zero-argument default constructor.
    ///
    /// Equivalent to `InputSource()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new input source with a system identifier.
    ///
    /// Equivalent to `InputSource(String systemId)`.
    pub fn from_system_id(system_id: impl Into<String>) -> Self {
        Self {
            system_id: Some(system_id.into()),
            ..Self::default()
        }
    }

    /// Create a new input source with a byte stream.
    ///
    /// Equivalent to `InputSource(InputStream byteStream)`.
    pub fn from_byte_stream(byte_stream: Box<dyn Read + Send>) -> Self {
        Self {
            byte_stream: Some(byte_stream),
            ..Self::default()
        }
    }

    /// Equivalent to `InputSource.setPublicId(String)`.
    pub fn set_public_id(&mut self, public_id: Option<String>) {
        self.public_id = public_id;
    }

    /// Equivalent to `InputSource.getPublicId()`.
    pub fn get_public_id(&self) -> Option<&str> {
        self.public_id.as_deref()
    }

    /// Equivalent to `InputSource.setSystemId(String)`.
    pub fn set_system_id(&mut self, system_id: Option<String>) {
        self.system_id = system_id;
    }

    /// Equivalent to `InputSource.getSystemId()`.
    pub fn get_system_id(&self) -> Option<&str> {
        self.system_id.as_deref()
    }

    /// Equivalent to `InputSource.setByteStream(InputStream)`.
    pub fn set_byte_stream(&mut self, byte_stream: Option<Box<dyn Read + Send>>) {
        self.byte_stream = byte_stream;
    }

    /// Equivalent to `InputSource.setEncoding(String)`.
    pub fn set_encoding(&mut self, encoding: Option<String>) {
        self.encoding = encoding;
    }

    /// Equivalent to `InputSource.getEncoding()`.
    pub fn get_encoding(&self) -> Option<&str> {
        self.encoding.as_deref()
    }
}

/// The attributes of a start tag, in document order.
///
/// Equivalent to `org.xml.sax.Attributes` as a namespace-aware JDK parser
/// fills it (no `xmlns` attributes): the qualified name and the normalized
/// value of each attribute.
#[derive(Debug, Clone, Default)]
pub struct Attributes {
    list: Vec<(String, String)>,
}

impl Attributes {
    /// Equivalent to `Attributes.getLength()`.
    pub fn get_length(&self) -> usize {
        self.list.len()
    }

    /// Equivalent to `Attributes.getQName(int)`, `None` for an index out of
    /// range (Java's `null`).
    pub fn get_qname(&self, index: usize) -> Option<&str> {
        self.list.get(index).map(|(name, _)| name.as_str())
    }

    /// Equivalent to `Attributes.getValue(int)`.
    pub fn get_value_at(&self, index: usize) -> Option<&str> {
        self.list.get(index).map(|(_, value)| value.as_str())
    }

    /// Look up an attribute's value by XML qualified name, `None` (Java's
    /// `null`) when the element has no such attribute.
    ///
    /// Equivalent to `Attributes.getValue(String qName)`.
    pub fn get_value(&self, qname: &str) -> Option<&str> {
        self.list
            .iter()
            .find(|(name, _)| name == qname)
            .map(|(_, value)| value.as_str())
    }
}

/// The SAX callbacks of `org.xml.sax.helpers.DefaultHandler` that the module
/// overrides; the defaults do what `DefaultHandler` does.
///
/// Equivalent to `DefaultHandler` as `EntityResolver` and `ContentHandler`
/// (its `ErrorHandler` methods keep their defaults: warnings and errors are
/// ignored, a fatal error is thrown).
pub trait DefaultHandler {
    /// Resolve an external entity; `None` lets the parser open the system id.
    ///
    /// Equivalent to `DefaultHandler.resolveEntity(String, String)`.
    ///
    /// # Errors
    ///
    /// A [`SaxException`] (Java's `SAXException`) or an `IOException`; it ends
    /// the parse unchanged.
    fn resolve_entity(
        &mut self,
        _public_id: Option<&str>,
        _system_id: Option<&str>,
    ) -> Result<Option<InputSource>> {
        Ok(None)
    }

    /// Receive notification of the start of an element.
    ///
    /// Equivalent to `DefaultHandler.startElement(String, String, String, Attributes)`.
    ///
    /// # Errors
    ///
    /// What the handler throws; it ends the parse unchanged.
    fn start_element(
        &mut self,
        _uri: &str,
        _local: &str,
        _raw: &str,
        _attrs: &Attributes,
    ) -> Result<()> {
        Ok(())
    }

    /// Receive notification of the end of an element.
    ///
    /// Equivalent to `DefaultHandler.endElement(String, String, String)`.
    ///
    /// # Errors
    ///
    /// What the handler throws; it ends the parse unchanged.
    fn end_element(&mut self, _uri: &str, _local: &str, _raw: &str) -> Result<()> {
        Ok(())
    }

    /// Receive notification of character data inside an element: the
    /// UTF-16 units `ch[start..start + length]`.
    ///
    /// Equivalent to `DefaultHandler.characters(char[], int, int)`.
    ///
    /// # Errors
    ///
    /// What the handler throws; it ends the parse unchanged.
    fn characters(&mut self, _ch: &[u16], _start: usize, _length: usize) -> Result<()> {
        Ok(())
    }
}

/// A `org.xml.sax.SAXException` or `org.xml.sax.SAXParseException`, carried
/// in [`LuceneErrorKind::Throwable`].
///
/// Equivalent to those classes: the message and, for a parse exception, the
/// system id, public id, line and column of the error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaxException {
    message: Option<String>,
    location: Option<Locator>,
}

/// The location of a `SAXParseException`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Locator {
    public_id: Option<String>,
    system_id: Option<String>,
    line_number: i32,
    column_number: i32,
}

impl SaxException {
    /// The binary name of `org.xml.sax.SAXException`.
    pub const SAX_EXCEPTION: &'static str = "org.xml.sax.SAXException";

    /// The binary name of `org.xml.sax.SAXParseException`.
    pub const SAX_PARSE_EXCEPTION: &'static str = "org.xml.sax.SAXParseException";

    /// Create a new SAXException.
    ///
    /// Equivalent to `new SAXException(String message)`; [`LuceneError::from`]
    /// throws it.
    pub fn new(message: impl Into<String>) -> Self {
        SaxException {
            message: Some(message.into()),
            location: None,
        }
    }

    /// The [`SaxException`] `error` carries, if it is one.
    pub fn of(error: &LuceneError) -> Option<&SaxException> {
        match error.kind() {
            LuceneErrorKind::Throwable(throwable) => throwable.as_any().downcast_ref(),
            _ => None,
        }
    }

    /// Equivalent to `SAXException.toString()` (`Throwable.toString()`, the
    /// class then `": " + message`) and `SAXParseException.toString()` (the
    /// class, then `"; publicId: "`, `"; systemId: "`, `"; lineNumber: "`,
    /// `"; columnNumber: "` for each one known, then `"; " + message`).
    pub fn to_java_string(&self) -> String {
        match &self.location {
            None => match &self.message {
                Some(message) => format!("{}: {message}", Self::SAX_EXCEPTION),
                None => Self::SAX_EXCEPTION.to_string(),
            },
            Some(location) => {
                let mut buf = String::from(Self::SAX_PARSE_EXCEPTION);
                if let Some(public_id) = &location.public_id {
                    buf.push_str("; publicId: ");
                    buf.push_str(public_id);
                }
                if let Some(system_id) = &location.system_id {
                    buf.push_str("; systemId: ");
                    buf.push_str(system_id);
                }
                if location.line_number != -1 {
                    buf.push_str(&format!("; lineNumber: {}", location.line_number));
                }
                if location.column_number != -1 {
                    buf.push_str(&format!("; columnNumber: {}", location.column_number));
                }
                if let Some(message) = &self.message {
                    buf.push_str("; ");
                    buf.push_str(message);
                }
                buf
            }
        }
    }
}

impl From<SaxException> for LuceneError {
    fn from(exception: SaxException) -> Self {
        LuceneError::from(LuceneErrorKind::Throwable(Arc::new(exception)))
    }
}

impl JavaThrowable for SaxException {
    fn java_class(&self) -> &'static str {
        if self.location.is_some() {
            Self::SAX_PARSE_EXCEPTION
        } else {
            Self::SAX_EXCEPTION
        }
    }

    fn get_message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The `XMLReader` of a namespace-aware `SAXParser` of the JDK.
///
/// Equivalent to `SAXParserFactory.newInstance()` with
/// `setNamespaceAware(true)`, `newSAXParser().getXMLReader()`; the handler
/// set with `setContentHandler`, `setErrorHandler` and `setEntityResolver`
/// is the one passed to [`parse`](Self::parse).
#[derive(Debug, Default, Clone, Copy)]
pub struct XmlReader;

/// The predefined general entities of XML 1.0 section 4.6.
fn predefined_entity(name: &str) -> Option<char> {
    match name {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "apos" => Some('\''),
        "quot" => Some('"'),
        _ => None,
    }
}

/// An internal general entity: its replacement text with character
/// references expanded and entity references kept.
#[derive(Debug, Clone)]
enum EntityDecl {
    Internal(String),
    External,
}

/// The state of one parse.
struct Parse<'h> {
    handler: &'h mut dyn DefaultHandler,
    text: String,
    system_id: Option<String>,
    public_id: Option<String>,
    entities: HashMap<String, EntityDecl>,
}

impl XmlReader {
    /// Creates the reader.
    ///
    /// Equivalent to `PatternParser.createParser()`'s
    /// `SAXParserFactory.newInstance()`, `setNamespaceAware(true)`,
    /// `newSAXParser().getXMLReader()`.
    pub fn new() -> Self {
        Self
    }

    /// Parse an XML document, calling `handler`.
    ///
    /// Equivalent to `XMLReader.parse(InputSource)`.
    ///
    /// # Errors
    ///
    /// An `IOException` for an unreadable source (`FileNotFoundException`
    /// with the JDK's `<path> (<reason>)` message for a missing file); what
    /// the handler throws, unchanged; and a [`SaxException`] of class
    /// `SAXParseException` for a fatal error, with the JDK's message for
    /// `Content is not allowed in prolog.`, `Content is not allowed in
    /// trailing section.`, `Premature end of file.`, `XML document structures
    /// must start and end within the same entity.`, `The markup in the
    /// document following the root element must be well-formed.`, `The
    /// element type "a" must be terminated by the matching end-tag "</a>".`,
    /// the unbound-prefix messages, `The entity "x" was referenced, but not
    /// declared.` and `Invalid encoding name "x".`.
    pub fn parse(&self, mut source: InputSource, handler: &mut dyn DefaultHandler) -> Result<()> {
        let system_id = source
            .system_id
            .as_deref()
            .map(|id| expand_system_id(id, None));
        let bytes = match source.byte_stream.take() {
            Some(mut stream) => {
                let mut bytes = Vec::new();
                stream.read_to_end(&mut bytes)?;
                bytes
            }
            None => match &system_id {
                Some(id) => open_system_id(id)?,
                None => {
                    return Err(io_exception(
                        "java.net.MalformedURLException",
                        Some("no protocol: null".to_string()),
                    ))
                }
            },
        };
        let mut parse = Parse {
            handler,
            text: String::new(),
            system_id,
            public_id: source.public_id.clone(),
            entities: HashMap::new(),
        };
        parse.text = parse.decode(&bytes, source.encoding.as_deref())?;
        parse.run()
    }
}

/// Expands a system id as the JDK's `XMLEntityManager.expandSystemId` does
/// for the ids `PatternParser` meets: an absolute URI (`scheme:`) as is, a
/// relative one resolved against `base`, or against the working directory
/// (`file:///<dir>/`) without one.
fn expand_system_id(system_id: &str, base: Option<&str>) -> String {
    let has_scheme = system_id.split_once(':').is_some_and(|(scheme, _)| {
        scheme.len() > 1
            && scheme
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    });
    if has_scheme {
        return system_id.to_string();
    }
    if system_id.starts_with('/') {
        return format!("file://{system_id}");
    }
    let base = match base {
        Some(base) => base.to_string(),
        None => {
            let dir = std::env::current_dir()
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_default();
            format!("file://{}/", dir.trim_end_matches('/'))
        }
    };
    match base.rfind('/') {
        Some(slash) => format!("{}{system_id}", &base[..=slash]),
        None => system_id.to_string(),
    }
}

/// Opens an expanded system id: a `file:` URL is read as a file.
fn open_system_id(system_id: &str) -> Result<Vec<u8>> {
    let path = match system_id.strip_prefix("file:") {
        Some(rest) => {
            let rest = rest.strip_prefix("//").unwrap_or(rest);
            rest.to_string()
        }
        None => {
            return Err(io_exception(
                "java.net.MalformedURLException",
                Some(format!("unknown protocol: {system_id}")),
            ))
        }
    };
    std::fs::read(&path).map_err(|e| {
        let text = e.to_string();
        let reason = match e.raw_os_error() {
            Some(code) => text
                .strip_suffix(&format!(" (os error {code})"))
                .unwrap_or(&text)
                .to_string(),
            None => text,
        };
        io_exception(
            "java.io.FileNotFoundException",
            Some(format!("{path} ({reason})")),
        )
    })
}

impl Parse<'_> {
    /// A `SAXParseException` at byte offset `pos` of the decoded text.
    fn fatal(&self, pos: usize, message: impl Into<String>) -> LuceneError {
        let pos = pos.min(self.text.len());
        let before = &self.text[..pos];
        let line = before.matches('\n').count() as i32 + 1;
        let column = before
            .rsplit('\n')
            .next()
            .map(|l| l.chars().count())
            .unwrap_or(0) as i32
            + 1;
        LuceneError::from(LuceneErrorKind::Throwable(Arc::new(SaxException {
            message: Some(message.into()),
            location: Some(Locator {
                public_id: self.public_id.clone(),
                system_id: self.system_id.clone(),
                line_number: line,
                column_number: column,
            }),
        })))
    }

    /// Decodes the document as the JDK detects its encoding, then applies the
    /// XML line-end normalization (XML 1.0 section 2.11).
    fn decode(&self, bytes: &[u8], encoding: Option<&str>) -> Result<String> {
        let (name, skip): (String, usize) = if let Some(encoding) = encoding {
            (encoding.to_string(), 0)
        } else if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            ("UTF-8".to_string(), 3)
        } else if bytes.starts_with(&[0xFE, 0xFF]) {
            ("UTF-16BE".to_string(), 2)
        } else if bytes.starts_with(&[0xFF, 0xFE]) {
            ("UTF-16LE".to_string(), 2)
        } else if bytes.starts_with(&[0x00, 0x3C, 0x00, 0x3F]) {
            ("UTF-16BE".to_string(), 0)
        } else if bytes.starts_with(&[0x3C, 0x00, 0x3F, 0x00]) {
            ("UTF-16LE".to_string(), 0)
        } else {
            (
                declared_encoding(bytes).unwrap_or_else(|| "UTF-8".to_string()),
                0,
            )
        };
        let charset = Charset::for_name(&name)
            .map_err(|_| self.fatal(0, format!("Invalid encoding name \"{name}\".")))?;
        let units =
            decode_reporting(&charset, &bytes[skip..]).map_err(|message| self.fatal(0, message))?;
        let text = String::from_utf16(&units)
            .map_err(|_| self.fatal(0, "Invalid surrogate pair in the decoded document."))?;
        Ok(text.replace("\r\n", "\n").replace('\r', "\n"))
    }

    /// Parses the decoded document; `self.text` stays in place for the
    /// error locations.
    fn run(&mut self) -> Result<()> {
        let text = self.text.clone();
        self.run_on(&text)
    }

    fn run_on(&mut self, text: &str) -> Result<()> {
        let mut reader = NsReader::from_str(text);
        {
            let config = reader.config_mut();
            config.expand_empty_elements = true;
            config.trim_text(false);
            config.check_end_names = true;
        }
        let mut depth = 0usize;
        let mut seen_root = false;
        // The element names, to report `endElement` with the start tag's
        // resolved names.
        let mut open: Vec<(String, String, String)> = Vec::new();
        loop {
            let pos = reader.buffer_position() as usize;
            let event = match reader.read_event() {
                Ok(event) => event,
                Err(e) => return Err(self.xml_error(&e, reader.error_position() as usize, &open)),
            };
            match event {
                Event::Decl(_) | Event::Comment(_) | Event::PI(_) => {}
                Event::DocType(doctype) => {
                    if seen_root {
                        return Err(self.fatal(
                            pos,
                            "The markup in the document following the root element must be well-formed.",
                        ));
                    }
                    self.doctype(&doctype, pos)?;
                }
                Event::Start(start) => {
                    if depth == 0 && seen_root {
                        return Err(self.fatal(
                            pos,
                            "The markup in the document following the root element must be well-formed.",
                        ));
                    }
                    let raw = start.name().as_ref().to_string();
                    let (uri, local) = match reader.resolver().resolve_element(start.name()) {
                        (ResolveResult::Unknown(prefix), _) => {
                            return Err(self.fatal(
                                reader.buffer_position() as usize,
                                format!(
                                    "The prefix \"{prefix}\" for element \"{raw}\" is not bound."
                                ),
                            ))
                        }
                        (ResolveResult::Bound(ns), local) => {
                            (ns.as_ref().to_string(), local.as_ref().to_string())
                        }
                        (ResolveResult::Unbound, local) => {
                            (String::new(), local.as_ref().to_string())
                        }
                    };
                    let mut attrs = Attributes::default();
                    for attr in start.attributes() {
                        let attr = match attr {
                            Ok(attr) => attr,
                            Err(e) => {
                                return Err(
                                    self.fatal(reader.buffer_position() as usize, e.to_string())
                                )
                            }
                        };
                        let qname = attr.key.as_ref().to_string();
                        if qname == "xmlns" || qname.starts_with("xmlns:") {
                            continue;
                        }
                        if let (ResolveResult::Unknown(prefix), _) = reader
                            .resolver()
                            .resolve_attribute(QName(attr.key.as_ref()))
                        {
                            return Err(self.fatal(
                                reader.buffer_position() as usize,
                                format!(
                                    "The prefix \"{prefix}\" for attribute \"{qname}\" associated with an element type \"{raw}\" is not bound."
                                ),
                            ));
                        }
                        let value =
                            self.attribute_value(&attr.value, reader.buffer_position() as usize)?;
                        attrs.list.push((qname, value));
                    }
                    depth += 1;
                    seen_root = true;
                    self.handler.start_element(&uri, &local, &raw, &attrs)?;
                    open.push((uri, local, raw));
                }
                Event::End(_) => {
                    let (uri, local, raw) = open
                        .pop()
                        .expect("INVARIANT: quick-xml checks end tags against open elements");
                    depth -= 1;
                    self.handler.end_element(&uri, &local, &raw)?;
                }
                Event::Empty(_) => {
                    unreachable!("INVARIANT: expand_empty_elements reports Start and End")
                }
                Event::Text(t) => {
                    let content: &str = &t;
                    if depth == 0 {
                        if !content.chars().all(|c| matches!(c, ' ' | '\t' | '\n')) {
                            let message = if seen_root {
                                "Content is not allowed in trailing section."
                            } else {
                                "Content is not allowed in prolog."
                            };
                            return Err(self.fatal(pos, message));
                        }
                    } else {
                        self.characters(content)?;
                    }
                }
                Event::CData(c) => {
                    let content: &str = &c;
                    self.characters(content)?;
                }
                Event::GeneralRef(r) => {
                    if depth == 0 {
                        let message = if seen_root {
                            "Content is not allowed in trailing section."
                        } else {
                            "Content is not allowed in prolog."
                        };
                        return Err(self.fatal(pos, message));
                    }
                    let name: &str = &r;
                    let expanded = self.expand_reference(name, pos, false, 0)?;
                    self.characters(&expanded)?;
                }
                Event::Eof => {
                    if !open.is_empty() {
                        return Err(self.fatal(
                            text.len(),
                            "XML document structures must start and end within the same entity.",
                        ));
                    }
                    if !seen_root {
                        return Err(self.fatal(text.len(), "Premature end of file."));
                    }
                    return Ok(());
                }
            }
        }
    }

    /// Reports `text` through `characters`, as UTF-16 units.
    fn characters(&mut self, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        let units: Vec<u16> = text.encode_utf16().collect();
        self.handler.characters(&units, 0, units.len())
    }

    /// A `quick-xml` error as the JDK's fatal error.
    fn xml_error(
        &self,
        error: &XmlError,
        pos: usize,
        open: &[(String, String, String)],
    ) -> LuceneError {
        match error {
            XmlError::IllFormed(IllFormedError::MismatchedEndTag { expected, .. }) => self.fatal(
                pos,
                format!("The element type \"{expected}\" must be terminated by the matching end-tag \"</{expected}>\"."),
            ),
            XmlError::IllFormed(IllFormedError::MissingEndTag(_)) if !open.is_empty() => self.fatal(
                self.text.len(),
                "XML document structures must start and end within the same entity.",
            ),
            XmlError::IllFormed(IllFormedError::UnmatchedEndTag(_)) if open.is_empty() => self.fatal(
                pos,
                "The markup in the document following the root element must be well-formed.",
            ),
            other => self.fatal(pos, other.to_string()),
        }
    }

    /// The value of a `<!DOCTYPE ...>`: the internal subset's general
    /// entities, then the external subset through `resolveEntity`.
    fn doctype(&mut self, doctype: &str, pos: usize) -> Result<()> {
        let mut rest = doctype.trim_start();
        // The root element name.
        let name_end = rest
            .find(|c: char| c.is_whitespace() || c == '[')
            .unwrap_or(rest.len());
        rest = rest[name_end..].trim_start();
        let mut public_id = None;
        let mut system_id = None;
        if let Some(after) = rest.strip_prefix("SYSTEM") {
            let (literal, after) = quoted(after.trim_start())
                .ok_or_else(|| self.fatal(pos, "The system identifier must begin with either a single or double quote character."))?;
            system_id = Some(literal.to_string());
            rest = after.trim_start();
        } else if let Some(after) = rest.strip_prefix("PUBLIC") {
            let (public, after) = quoted(after.trim_start())
                .ok_or_else(|| self.fatal(pos, "The public identifier must begin with either a single or double quote character."))?;
            let (literal, after) = quoted(after.trim_start()).ok_or_else(|| {
                self.fatal(
                    pos,
                    "White spaces are required between publicId and systemId.",
                )
            })?;
            public_id = Some(public.to_string());
            system_id = Some(literal.to_string());
            rest = after.trim_start();
        }
        if let Some(subset) = rest.strip_prefix('[') {
            let end = subset.rfind(']').unwrap_or(subset.len());
            self.internal_subset(&subset[..end], pos)?;
        }
        if let Some(system_id) = system_id {
            let expanded = expand_system_id(&system_id, self.system_id.as_deref());
            match self
                .handler
                .resolve_entity(public_id.as_deref(), Some(&expanded))?
            {
                Some(mut source) => match source.byte_stream.take() {
                    Some(mut stream) => {
                        let mut dtd = Vec::new();
                        stream.read_to_end(&mut dtd)?;
                    }
                    None => {
                        if let Some(id) = source.system_id.as_deref() {
                            if crate::resources::get_resource(id).is_none() {
                                open_system_id(&expand_system_id(id, Some(&expanded)))?;
                            }
                        }
                    }
                },
                None => {
                    open_system_id(&expanded)?;
                }
            }
        }
        Ok(())
    }

    /// Records the `<!ENTITY name "value">` declarations of the internal
    /// subset; the first declaration of a name wins (XML 1.0 section 4.2).
    fn internal_subset(&mut self, subset: &str, pos: usize) -> Result<()> {
        let mut rest = subset;
        while let Some(start) = rest.find("<!ENTITY") {
            rest = rest[start + "<!ENTITY".len()..].trim_start();
            if rest.starts_with('%') {
                // A parameter entity: not expanded by this reader.
                continue;
            }
            let name_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let name = rest[..name_end].to_string();
            rest = rest[name_end..].trim_start();
            let decl = if rest.starts_with("SYSTEM") || rest.starts_with("PUBLIC") {
                EntityDecl::External
            } else {
                let (literal, _) = quoted(rest).ok_or_else(|| {
                    self.fatal(
                        pos,
                        format!("The entity \"{name}\" must have a quoted value."),
                    )
                })?;
                EntityDecl::Internal(self.expand_char_refs(literal, pos)?)
            };
            self.entities.entry(name).or_insert(decl);
        }
        Ok(())
    }

    /// Expands the character references of an entity value literal.
    fn expand_char_refs(&self, literal: &str, pos: usize) -> Result<String> {
        let mut out = String::with_capacity(literal.len());
        let mut rest = literal;
        while let Some(amp) = rest.find("&#") {
            out.push_str(&rest[..amp]);
            let after = &rest[amp + 1..];
            let semi = after.find(';').ok_or_else(|| {
                self.fatal(
                    pos,
                    "The character reference must end with the ';' delimiter.",
                )
            })?;
            out.push(self.char_ref(&after[..semi], pos)?);
            rest = &after[semi + 1..];
        }
        out.push_str(rest);
        Ok(out)
    }

    /// The character of a reference body `#n` or `#xh`.
    fn char_ref(&self, body: &str, pos: usize) -> Result<char> {
        let digits = body.strip_prefix('#').unwrap_or(body);
        let value = match digits.strip_prefix('x') {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => digits.parse::<u32>().ok(),
        };
        value.and_then(char::from_u32).ok_or_else(|| {
            self.fatal(
                pos,
                format!("Character reference \"&{body}\" is an invalid XML character."),
            )
        })
    }

    /// The text a reference `&name;` stands for in content, or, with
    /// `in_attribute`, in an attribute value (where its whitespace is
    /// normalized).
    fn expand_reference(
        &self,
        name: &str,
        pos: usize,
        in_attribute: bool,
        level: usize,
    ) -> Result<String> {
        if name.starts_with('#') {
            return Ok(self.char_ref(name, pos)?.to_string());
        }
        if let Some(c) = predefined_entity(name) {
            return Ok(c.to_string());
        }
        if level > 64 {
            return Err(self.fatal(
                pos,
                format!(
                    "Recursive entity reference \"{name}\". (Reference path: {name} -> {name})"
                ),
            ));
        }
        match self.entities.get(name) {
            None => Err(self.fatal(
                pos,
                format!("The entity \"{name}\" was referenced, but not declared."),
            )),
            Some(EntityDecl::External) => Err(self.fatal(
                pos,
                format!("The external entity \"{name}\" is not supported by this reader."),
            )),
            Some(EntityDecl::Internal(text)) => {
                if text.contains('<') {
                    return Err(self.fatal(
                        pos,
                        format!("The markup in the replacement text of entity \"{name}\" is not supported by this reader."),
                    ));
                }
                self.expand_text(text, pos, in_attribute, level + 1)
            }
        }
    }

    /// Expands the entity references of a replacement text.
    fn expand_text(
        &self,
        text: &str,
        pos: usize,
        in_attribute: bool,
        level: usize,
    ) -> Result<String> {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(amp) = rest.find('&') {
            let literal = &rest[..amp];
            if in_attribute {
                out.extend(
                    literal
                        .chars()
                        .map(|c| if matches!(c, '\t' | '\n') { ' ' } else { c }),
                );
            } else {
                out.push_str(literal);
            }
            let after = &rest[amp + 1..];
            let semi = after.find(';').ok_or_else(|| {
                self.fatal(
                    pos,
                    "The reference to entity must end with the ';' delimiter.",
                )
            })?;
            out.push_str(&self.expand_reference(&after[..semi], pos, in_attribute, level)?);
            rest = &after[semi + 1..];
        }
        if in_attribute {
            out.extend(
                rest.chars()
                    .map(|c| if matches!(c, '\t' | '\n') { ' ' } else { c }),
            );
        } else {
            out.push_str(rest);
        }
        Ok(out)
    }

    /// A normalized attribute value (XML 1.0 section 3.3.3).
    fn attribute_value(&self, raw: &str, pos: usize) -> Result<String> {
        if raw.contains('<') {
            return Err(self.fatal(
                pos,
                "The value of attribute must not contain the '<' character.",
            ));
        }
        let mut out = String::with_capacity(raw.len());
        let mut rest = raw;
        while let Some(amp) = rest.find('&') {
            out.extend(
                rest[..amp]
                    .chars()
                    .map(|c| if matches!(c, '\t' | '\n') { ' ' } else { c }),
            );
            let after = &rest[amp + 1..];
            let semi = after.find(';').ok_or_else(|| {
                self.fatal(
                    pos,
                    "The reference to entity must end with the ';' delimiter.",
                )
            })?;
            let name = &after[..semi];
            if name.starts_with('#') {
                // A character reference is kept as is, a tab or line end too.
                out.push(self.char_ref(name, pos)?);
            } else {
                out.push_str(&self.expand_reference(name, pos, true, 0)?);
            }
            rest = &after[semi + 1..];
        }
        out.extend(
            rest.chars()
                .map(|c| if matches!(c, '\t' | '\n') { ' ' } else { c }),
        );
        Ok(out)
    }
}

/// A quoted literal at the start of `s` and the text after it.
fn quoted(s: &str) -> Option<(&str, &str)> {
    let quote = s.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let body = &s[1..];
    let end = body.find(quote)?;
    Some((&body[..end], &body[end + 1..]))
}

/// The `encoding` pseudo-attribute of an XML declaration at the start of an
/// ASCII-compatible document.
fn declared_encoding(bytes: &[u8]) -> Option<String> {
    if !bytes.starts_with(b"<?xml") {
        return None;
    }
    let end = bytes.windows(2).position(|w| w == b"?>")?;
    let decl = std::str::from_utf8(&bytes[..end]).ok()?;
    let at = decl.find("encoding")?;
    let after = decl[at + "encoding".len()..]
        .trim_start()
        .strip_prefix('=')?;
    quoted(after.trim_start()).map(|(name, _)| name.to_string())
}

/// Decodes `bytes` with `charset`, failing on malformed input or an
/// unmappable character as the JDK's readers do (`Invalid byte N of M-byte
/// UTF-8 sequence.` for UTF-8, a generic message otherwise).
fn decode_reporting(charset: &Charset, bytes: &[u8]) -> std::result::Result<Vec<u16>, String> {
    if charset.name() == "UTF-8" {
        return decode_utf8(bytes);
    }
    let mut decoder = charset.new_decoder().map_err(|e| e.to_string())?;
    let mut out = vec![0u16; bytes.len() * 2 + 16];
    let (mut sp, mut dp) = (0usize, 0usize);
    loop {
        match decoder.decode_loop(bytes, &mut sp, &mut out, &mut dp) {
            super::charset::CoderResult::Underflow => break,
            super::charset::CoderResult::Overflow => {
                let len = out.len();
                out.resize(len * 2, 0);
            }
            _ => {
                return Err(format!(
                    "Invalid byte sequence at byte {sp} for the encoding \"{}\".",
                    charset.name()
                ))
            }
        }
    }
    while let super::charset::CoderResult::Overflow = decoder.flush(&mut out, &mut dp) {
        let len = out.len();
        out.resize(len * 2, 0);
    }
    out.truncate(dp);
    Ok(out)
}

/// Decodes UTF-8 as the JDK's `UTF8Reader` (Xerces) does, failing with its
/// message on the first malformed sequence.
fn decode_utf8(bytes: &[u8]) -> std::result::Result<Vec<u16>, String> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.encode_utf16().collect()),
        Err(e) => {
            let at = e.valid_up_to();
            let first = bytes[at];
            let expected = if first & 0xE0 == 0xC0 {
                2
            } else if first & 0xF0 == 0xE0 {
                3
            } else if first & 0xF8 == 0xF0 {
                4
            } else {
                1
            };
            let index = match e.error_len() {
                Some(n) if expected > 1 => n + 1,
                _ => 1,
            };
            Err(format!(
                "Invalid byte {index} of {expected}-byte UTF-8 sequence."
            ))
        }
    }
}
