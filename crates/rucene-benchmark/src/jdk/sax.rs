//! The streaming SAX parser `EnwikiContentSource` parses the Wikipedia dump
//! with.
//!
//! Java builds it with `SAXParserFactory.newDefaultInstance()` and
//! `setFeature(XMLConstants.FEATURE_SECURE_PROCESSING, true)`
//! (`EnwikiContentSource.java:52-62`), then calls `SAXParser.parse(new
//! InputSource(reader), handler)` with a UTF-8 decoding `Reader`
//! (`EnwikiContentSource.java:193-198`). That factory is not namespace
//! aware, so the JDK's (Xerces-derived) parser reports every element with an
//! empty namespace URI and local name and its raw name as the qualified
//! name, and reports `xmlns` attributes as ordinary attributes. The parser
//! reads its input as it goes and calls the handler as each event is
//! scanned, so a multi-gigabyte dump is never held in memory.
//!
//! [`SaxParser`] does the same over the `quick-xml` pull parser, the XML
//! parser of the workspace (rmp task #461): a [`quick_xml::Reader`] over a
//! [`BufRead`] that encodes the `Reader`'s UTF-16 units to UTF-8 one buffer
//! at a time, applying the XML line-end normalization (XML 1.0 section
//! 2.11), and the handler calls the JDK parser makes for the events it
//! scans: `startElement("", "", qName, attributes)` and `endElement("", "",
//! qName)` for every element (an empty element `<a/>` gives both),
//! `characters` for the text and `CDATA` content of elements, with the
//! predefined entities, character references and the general entities of
//! the internal subset expanded, and attribute values normalized as XML 1.0
//! section 3.3.3 says.
//!
//! What differs from the JDK parser is what differs for the `quick-xml`
//! SAX reader of `rucene_analysis_common::jdk::sax`, whose module
//! documentation declares it: the chunking of `characters` (one call per
//! run of text between markup and references), the external DTD subset
//! (not read; an undeclared reference is skipped when the `DOCTYPE` names
//! one and the document is not `standalone="yes"`, as the JDK skips it),
//! external general entities and replacement text holding markup (fatal
//! errors), unvalidated character ranges and names, and the messages of the
//! well-formedness errors that are not listed at [`SaxParser::parse`]
//! (`quick-xml`'s). The line and column of a [`SaxParseException`] are those
//! of the input the parser had consumed when it reported the error.

use std::collections::HashMap;
use std::io::{self, BufRead, Read};
use std::sync::Arc;

use quick_xml::errors::{Error as XmlError, IllFormedError};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use rucene::analysis::CharReader;
use rucene::error::{JavaThrowable, LuceneError, LuceneErrorKind, Result};

/// The attributes of a start tag, in document order.
///
/// Equivalent to `org.xml.sax.Attributes` as a parser that is not
/// namespace aware fills it: the qualified name and the normalized value of
/// each attribute, `xmlns` attributes included.
#[derive(Debug, Clone, Default)]
pub struct Attributes {
    list: Vec<(String, String)>,
}

// The `Attributes` read methods; `EnwikiContentSource`'s handler reads no
// attribute.
#[allow(dead_code)]
impl Attributes {
    /// `getLength()`.
    pub fn get_length(&self) -> usize {
        self.list.len()
    }

    /// `getQName(int index)`, `None` (Java's `null`) for an index out of
    /// range.
    pub fn get_qname(&self, index: usize) -> Option<&str> {
        self.list.get(index).map(|(name, _)| name.as_str())
    }

    /// `getValue(int index)`, `None` (Java's `null`) for an index out of
    /// range.
    pub fn get_value_at(&self, index: usize) -> Option<&str> {
        self.list.get(index).map(|(_, value)| value.as_str())
    }

    /// `getValue(String qName)`, `None` (Java's `null`) when the element has
    /// no such attribute.
    pub fn get_value(&self, qname: &str) -> Option<&str> {
        self.list
            .iter()
            .find(|(name, _)| name == qname)
            .map(|(_, value)| value.as_str())
    }
}

/// The `org.xml.sax.helpers.DefaultHandler` callbacks a content handler of
/// the benchmark overrides; the defaults do nothing, as `DefaultHandler`'s.
pub trait DefaultHandler {
    /// `startElement(String uri, String localName, String qName, Attributes
    /// attributes)`.
    ///
    /// # Errors
    ///
    /// What the handler throws; it ends the parse unchanged.
    fn start_element(
        &mut self,
        _uri: &str,
        _local_name: &str,
        _qname: &str,
        _attributes: &Attributes,
    ) -> Result<()> {
        Ok(())
    }

    /// `endElement(String uri, String localName, String qName)`.
    ///
    /// # Errors
    ///
    /// What the handler throws; it ends the parse unchanged.
    fn end_element(&mut self, _uri: &str, _local_name: &str, _qname: &str) -> Result<()> {
        Ok(())
    }

    /// `characters(char[] ch, int start, int length)`: the UTF-16 units
    /// `ch[start..start + length]`.
    ///
    /// # Errors
    ///
    /// What the handler throws; it ends the parse unchanged.
    fn characters(&mut self, _ch: &[u16], _start: usize, _length: usize) -> Result<()> {
        Ok(())
    }
}

/// Equivalent to `org.xml.sax.SAXParseException` as the parser throws it
/// for a document read from a `Reader` (no public or system id): the
/// message, line and column of a fatal error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaxParseException {
    message: String,
    line_number: i32,
    column_number: i32,
}

impl JavaThrowable for SaxParseException {
    fn java_class(&self) -> &'static str {
        "org.xml.sax.SAXParseException"
    }

    fn get_message(&self) -> Option<&str> {
        Some(&self.message)
    }

    /// `SAXParseException.toString()`: the class, then `"; lineNumber: "`
    /// and `"; columnNumber: "`, then `"; " + message`.
    fn to_java_string(&self) -> Option<String> {
        Some(format!(
            "{}; lineNumber: {}; columnNumber: {}; {}",
            self.java_class(),
            self.line_number,
            self.column_number,
            self.message
        ))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Equivalent to the `SAXParser` of `SAXParserFactory.newDefaultInstance()`
/// with `FEATURE_SECURE_PROCESSING` set (not namespace aware, not
/// validating).
#[derive(Debug, Default, Clone, Copy)]
pub struct SaxParser;

impl SaxParser {
    /// `SAX_PARSER_FACTORY.newSAXParser()`.
    pub fn new() -> Self {
        SaxParser
    }

    /// `parse(new InputSource(reader), handler)`: parses the document the
    /// character stream `reader` holds, reading it incrementally and calling
    /// `handler` as each event is scanned.
    ///
    /// # Errors
    ///
    /// What `reader` throws (an `IOException`), unchanged; what the handler
    /// throws, unchanged; a [`SaxParseException`] for a fatal error, with
    /// the JDK's message for `Content is not allowed in prolog.`, `Content
    /// is not allowed in trailing section.`, `Premature end of file.`, `XML
    /// document structures must start and end within the same entity.`,
    /// `The markup in the document following the root element must be
    /// well-formed.`, `The element type "a" must be terminated by the
    /// matching end-tag "</a>".`, `Attribute "a" was already specified for
    /// element "e".`, `The entity "x" was referenced, but not declared.` and
    /// `Recursive entity reference "x". (Reference path: x -> y -> x)`.
    pub fn parse(
        &self,
        reader: &mut dyn CharReader,
        handler: &mut dyn DefaultHandler,
    ) -> Result<()> {
        let mut parse = Parse {
            handler,
            reader: Reader::from_reader(Utf8Stream::new(reader)),
            entities: HashMap::new(),
            external_subset: false,
            standalone: false,
        };
        parse.run()
    }
}

/// A [`BufRead`] over a character stream: its UTF-16 units encoded as
/// UTF-8, with every `\r\n` and lone `\r` turned into `\n`, one buffer at a
/// time. It counts the lines and columns of the bytes consumed, for the
/// error locations.
struct Utf8Stream<'a> {
    reader: &'a mut dyn CharReader,
    units: Vec<u16>,
    /// A high surrogate read at the end of the last units buffer.
    pending_high: Option<u16>,
    /// The last character was a `\r`: a `\n` right after it is dropped.
    after_cr: bool,
    eof: bool,
    bytes: Vec<u8>,
    pos: usize,
    line: i32,
    column: i32,
}

impl<'a> Utf8Stream<'a> {
    fn new(reader: &'a mut dyn CharReader) -> Self {
        Utf8Stream {
            reader,
            units: vec![0; 8192],
            pending_high: None,
            after_cr: false,
            eof: false,
            bytes: Vec::with_capacity(8192 * 3),
            pos: 0,
            line: 1,
            column: 1,
        }
    }

    fn push_char(&mut self, c: char) {
        let c = match c {
            '\r' => {
                self.after_cr = true;
                '\n'
            }
            '\n' if self.after_cr => {
                self.after_cr = false;
                return;
            }
            c => {
                self.after_cr = false;
                c
            }
        };
        let mut utf8 = [0u8; 4];
        self.bytes
            .extend_from_slice(c.encode_utf8(&mut utf8).as_bytes());
    }

    /// Reads and encodes the next buffer of units; `false` at the end of the
    /// stream.
    fn refill(&mut self) -> Result<bool> {
        self.bytes.clear();
        self.pos = 0;
        while self.bytes.is_empty() && !self.eof {
            let n = self.reader.read(&mut self.units)?;
            if n == 0 {
                self.eof = true;
                if self.pending_high.take().is_some() {
                    // A lone high surrogate (character ranges are not
                    // validated).
                    self.push_char(char::REPLACEMENT_CHARACTER);
                }
                break;
            }
            let mut units: Vec<u16> = Vec::with_capacity(n + 1);
            units.extend(self.pending_high.take());
            units.extend_from_slice(&self.units[..n]);
            if units.last().is_some_and(|u| (0xD800..0xDC00).contains(u)) {
                self.pending_high = units.pop();
            }
            for c in char::decode_utf16(units) {
                self.push_char(c.unwrap_or(char::REPLACEMENT_CHARACTER));
            }
        }
        Ok(!self.bytes.is_empty())
    }
}

impl Read for Utf8Stream<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let n = available.len().min(buf.len());
        buf[..n].copy_from_slice(&available[..n]);
        self.consume(n);
        Ok(n)
    }
}

impl BufRead for Utf8Stream<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.pos == self.bytes.len() {
            // The reader's exception travels through `quick-xml` inside the
            // `io::Error`, and is taken back out by `Parse::xml_error`.
            self.refill().map_err(io::Error::other)?;
        }
        Ok(&self.bytes[self.pos..])
    }

    fn consume(&mut self, amt: usize) {
        for &b in &self.bytes[self.pos..self.pos + amt] {
            if b == b'\n' {
                self.line += 1;
                self.column = 1;
            } else if b & 0xC0 != 0x80 {
                self.column += 1;
            }
        }
        self.pos += amt;
    }
}

/// An `<!ENTITY>` declaration of the internal subset.
#[derive(Debug, Clone)]
enum EntityDecl {
    Internal(String),
    External,
}

/// The state of one parse.
struct Parse<'h, 'r> {
    handler: &'h mut dyn DefaultHandler,
    reader: Reader<Utf8Stream<'r>>,
    entities: HashMap<String, EntityDecl>,
    external_subset: bool,
    standalone: bool,
}

impl Parse<'_, '_> {
    /// A `SAXParseException` at the input consumed so far.
    fn fatal(&self, message: impl Into<String>) -> LuceneError {
        let stream = self.reader.get_ref();
        LuceneError::from(LuceneErrorKind::Throwable(Arc::new(SaxParseException {
            message: message.into(),
            line_number: stream.line,
            column_number: stream.column,
        })))
    }

    fn run(&mut self) -> Result<()> {
        {
            let config = self.reader.config_mut();
            config.expand_empty_elements = true;
            config.trim_text(false);
            config.check_end_names = true;
        }
        let mut buf = Vec::new();
        let mut seen_root = false;
        // The qualified names of the open elements, to report `endElement`.
        let mut open: Vec<String> = Vec::new();
        loop {
            buf.clear();
            let event = match self.reader.read_event_into(&mut buf) {
                Ok(event) => event.into_owned(),
                Err(e) => return Err(self.xml_error(e, &open)),
            };
            match event {
                Event::Decl(decl) => {
                    if let Some(Ok(standalone)) = decl.standalone() {
                        self.standalone = &*standalone == "yes";
                    }
                }
                Event::Comment(_) | Event::PI(_) => {}
                Event::DocType(doctype) => {
                    if seen_root {
                        return Err(self.fatal("The markup in the document following the root element must be well-formed."));
                    }
                    self.doctype(&doctype)?;
                }
                Event::Start(start) => {
                    if open.is_empty() && seen_root {
                        return Err(self.fatal("The markup in the document following the root element must be well-formed."));
                    }
                    let qname = start.name().as_ref().to_string();
                    let mut attributes = Attributes::default();
                    // Duplicates are checked here, to name the attribute.
                    for attr in start.attributes().with_checks(false) {
                        let attr = attr.map_err(|e| self.fatal(e.to_string()))?;
                        let name = attr.key.as_ref().to_string();
                        if attributes.list.iter().any(|(n, _)| *n == name) {
                            return Err(self.fatal(format!(
                                "Attribute \"{name}\" was already specified for element \"{qname}\"."
                            )));
                        }
                        let value = self.attribute_value(&attr.value)?;
                        attributes.list.push((name, value));
                    }
                    seen_root = true;
                    self.handler.start_element("", "", &qname, &attributes)?;
                    open.push(qname);
                }
                Event::End(_) => {
                    let qname = open
                        .pop()
                        .expect("INVARIANT: quick-xml checks end tags against open elements");
                    self.handler.end_element("", "", &qname)?;
                }
                Event::Empty(_) => {
                    unreachable!("INVARIANT: expand_empty_elements reports Start and End")
                }
                Event::Text(text) => {
                    let content: &str = &text;
                    if open.is_empty() {
                        if !content.chars().all(|c| matches!(c, ' ' | '\t' | '\n')) {
                            return Err(self.fatal(if seen_root {
                                "Content is not allowed in trailing section."
                            } else {
                                "Content is not allowed in prolog."
                            }));
                        }
                    } else {
                        self.characters(content)?;
                    }
                }
                Event::CData(cdata) => {
                    let content: &str = &cdata;
                    self.characters(content)?;
                }
                Event::GeneralRef(reference) => {
                    if open.is_empty() {
                        return Err(self.fatal(if seen_root {
                            "Content is not allowed in trailing section."
                        } else {
                            "Content is not allowed in prolog."
                        }));
                    }
                    let name: &str = &reference;
                    let expanded = self.expand_reference(name, false, &mut Vec::new())?;
                    self.characters(&expanded)?;
                }
                Event::Eof => {
                    if !open.is_empty() {
                        return Err(self.fatal(
                            "XML document structures must start and end within the same entity.",
                        ));
                    }
                    if !seen_root {
                        return Err(self.fatal("Premature end of file."));
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

    /// A `quick-xml` error: the reader's own exception when it carries one,
    /// otherwise the JDK's fatal error.
    fn xml_error(&self, error: XmlError, open: &[String]) -> LuceneError {
        match error {
            XmlError::Io(io) => match Arc::try_unwrap(io) {
                Ok(io) if io.get_ref().is_some_and(|e| e.is::<LuceneError>()) => *io
                    .into_inner()
                    .and_then(|e| e.downcast::<LuceneError>().ok())
                    .expect("INVARIANT: checked to carry a LuceneError above"),
                Ok(io) => LuceneError::from(io),
                Err(io) => LuceneError::from(io::Error::new(io.kind(), io.to_string())),
            },
            XmlError::IllFormed(IllFormedError::MismatchedEndTag { expected, .. }) => self.fatal(format!(
                "The element type \"{expected}\" must be terminated by the matching end-tag \"</{expected}>\"."
            )),
            XmlError::IllFormed(IllFormedError::MissingEndTag(_)) if !open.is_empty() => {
                self.fatal("XML document structures must start and end within the same entity.")
            }
            XmlError::IllFormed(IllFormedError::UnmatchedEndTag(_)) if open.is_empty() => {
                self.fatal("The markup in the document following the root element must be well-formed.")
            }
            other => self.fatal(other.to_string()),
        }
    }

    /// The value of a `<!DOCTYPE ...>`: the internal subset's general
    /// entities, and whether an external subset is named.
    fn doctype(&mut self, doctype: &str) -> Result<()> {
        let mut rest = doctype.trim_start();
        // The root element name.
        let name_end = rest
            .find(|c: char| c.is_whitespace() || c == '[')
            .unwrap_or(rest.len());
        rest = rest[name_end..].trim_start();
        if let Some(after) = rest.strip_prefix("SYSTEM") {
            let (_, after) = quoted(after.trim_start()).ok_or_else(|| {
                self.fatal("The system identifier must begin with either a single or double quote character.")
            })?;
            self.external_subset = true;
            rest = after.trim_start();
        } else if let Some(after) = rest.strip_prefix("PUBLIC") {
            let (_, after) = quoted(after.trim_start()).ok_or_else(|| {
                self.fatal("The public identifier must begin with either a single or double quote character.")
            })?;
            let (_, after) = quoted(after.trim_start()).ok_or_else(|| {
                self.fatal("White spaces are required between publicId and systemId.")
            })?;
            self.external_subset = true;
            rest = after.trim_start();
        }
        if let Some(subset) = rest.strip_prefix('[') {
            let end = subset.rfind(']').unwrap_or(subset.len());
            self.internal_subset(&subset[..end])?;
        }
        Ok(())
    }

    /// Records the `<!ENTITY name "value">` declarations of the internal
    /// subset; the first declaration of a name wins (XML 1.0 section 4.2).
    fn internal_subset(&mut self, subset: &str) -> Result<()> {
        let mut rest = subset;
        while let Some(start) = rest.find("<!ENTITY") {
            rest = rest[start + "<!ENTITY".len()..].trim_start();
            if rest.starts_with('%') {
                // A parameter entity: not expanded by this parser.
                continue;
            }
            let name_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let name = rest[..name_end].to_string();
            rest = rest[name_end..].trim_start();
            let decl = if rest.starts_with("SYSTEM") || rest.starts_with("PUBLIC") {
                EntityDecl::External
            } else {
                let (literal, _) = quoted(rest).ok_or_else(|| {
                    self.fatal(format!("The entity \"{name}\" must have a quoted value."))
                })?;
                EntityDecl::Internal(self.expand_char_refs(literal)?)
            };
            self.entities.entry(name).or_insert(decl);
        }
        Ok(())
    }

    /// Expands the character references of an entity value literal.
    fn expand_char_refs(&self, literal: &str) -> Result<String> {
        let mut out = String::with_capacity(literal.len());
        let mut rest = literal;
        while let Some(amp) = rest.find("&#") {
            out.push_str(&rest[..amp]);
            let after = &rest[amp + 1..];
            let semi = after.find(';').ok_or_else(|| {
                self.fatal("The character reference must end with the ';' delimiter.")
            })?;
            out.push(self.char_ref(&after[..semi])?);
            rest = &after[semi + 1..];
        }
        out.push_str(rest);
        Ok(out)
    }

    /// The character of a reference body `#n` or `#xh`.
    fn char_ref(&self, body: &str) -> Result<char> {
        let digits = body.strip_prefix('#').unwrap_or(body);
        let value = match digits.strip_prefix('x') {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => digits.parse::<u32>().ok(),
        };
        value
            .filter(|v| *v != 0)
            .and_then(char::from_u32)
            .ok_or_else(|| {
                self.fatal(format!(
                    "Character reference \"&{body}\" is an invalid XML character."
                ))
            })
    }

    /// The text a reference `&name;` stands for in content, or, with
    /// `in_attribute`, in an attribute value (where its whitespace is
    /// normalized). `path` holds the entities being expanded.
    fn expand_reference(
        &self,
        name: &str,
        in_attribute: bool,
        path: &mut Vec<String>,
    ) -> Result<String> {
        if name.starts_with('#') {
            return Ok(self.char_ref(name)?.to_string());
        }
        if let Some(c) = predefined_entity(name) {
            return Ok(c.to_string());
        }
        if path.iter().any(|entity| entity == name) {
            return Err(self.fatal(format!(
                "Recursive entity reference \"{name}\". (Reference path: {} -> {name})",
                path.join(" -> ")
            )));
        }
        match self.entities.get(name) {
            // With an external subset the declaration may be there (a
            // validity constraint, not a well-formedness one): the JDK skips
            // the reference (`skippedEntity`) without an error.
            None if self.external_subset && !self.standalone => Ok(String::new()),
            None => Err(self.fatal(format!(
                "The entity \"{name}\" was referenced, but not declared."
            ))),
            Some(EntityDecl::External) => Err(self.fatal(format!(
                "The external entity \"{name}\" is not supported by this parser."
            ))),
            Some(EntityDecl::Internal(text)) => {
                if text.contains('<') {
                    return Err(self.fatal(format!(
                        "The markup in the replacement text of entity \"{name}\" is not supported by this parser."
                    )));
                }
                path.push(name.to_string());
                let expanded = self.expand_text(text, in_attribute, path);
                path.pop();
                expanded
            }
        }
    }

    /// Expands the entity references of a replacement text or a raw
    /// attribute value; with `in_attribute`, each literal tab or line end
    /// becomes a space.
    fn expand_text(
        &self,
        text: &str,
        in_attribute: bool,
        path: &mut Vec<String>,
    ) -> Result<String> {
        let normalize = |out: &mut String, literal: &str| {
            if in_attribute {
                out.extend(
                    literal
                        .chars()
                        .map(|c| if matches!(c, '\t' | '\n') { ' ' } else { c }),
                );
            } else {
                out.push_str(literal);
            }
        };
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(amp) = rest.find('&') {
            normalize(&mut out, &rest[..amp]);
            let after = &rest[amp + 1..];
            let semi = after.find(';').ok_or_else(|| {
                self.fatal("The reference to entity must end with the ';' delimiter.")
            })?;
            let name = &after[..semi];
            if in_attribute && name.starts_with('#') {
                // A character reference is kept as is, a tab or line end too.
                out.push(self.char_ref(name)?);
            } else {
                out.push_str(&self.expand_reference(name, in_attribute, path)?);
            }
            rest = &after[semi + 1..];
        }
        normalize(&mut out, rest);
        Ok(out)
    }

    /// A normalized attribute value (XML 1.0 section 3.3.3).
    fn attribute_value(&self, raw: &str) -> Result<String> {
        if raw.contains('<') {
            return Err(self.fatal("The value of attribute must not contain the '<' character."));
        }
        self.expand_text(raw, true, &mut Vec::new())
    }
}

/// The five predefined entities (XML 1.0 section 4.6).
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

/// A quoted literal at the start of `s` and the text after it.
fn quoted(s: &str) -> Option<(&str, &str)> {
    let quote = s.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let body = &s[1..];
    let end = body.find(quote)?;
    Some((&body[..end], &body[end + 1..]))
}
