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

//! The type declared in `PatternParser.java`, of the Java package
//! `org.apache.lucene.analysis.compound.hyphenation`.

use std::io::Cursor;
use std::sync::LazyLock;

use rucene::analysis::java_regex::JavaPattern;
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::store::java_nio_file::io_exception_with_cause;

use super::hyphen::Hyphen;
use super::pattern_consumer::{HyphenatedWordPart, PatternConsumer};
use crate::jdk::character::{is_digit, is_whitespace};
use crate::jdk::sax::{Attributes, DefaultHandler, InputSource, SaxException, XmlReader};

const ELEM_CLASSES: i32 = 1;

const ELEM_EXCEPTIONS: i32 = 2;

const ELEM_PATTERNS: i32 = 3;

const ELEM_HYPHEN: i32 = 4;

/// The class-path name of the DTD `resolveEntity` supplies:
/// `PatternParser.class.getResource("hyphenation.dtd")`.
const HYPHENATION_DTD: &str = "org/apache/lucene/analysis/compound/hyphenation/hyphenation.dtd";

/// `systemId.matches("(?i).*\\bhyphenation.dtd\\b.*")`.
static HYPHENATION_DTD_PATTERN: LazyLock<JavaPattern> = LazyLock::new(|| {
    JavaPattern::compile("(?i).*\\bhyphenation.dtd\\b.*", 0)
        .expect("INVARIANT: the pattern is valid")
});

/// A SAX document handler to read and parse hyphenation patterns from a XML
/// file.
///
/// This class has been taken from the Apache FOP project
/// (<http://xmlgraphics.apache.org/fop/>). They have been slightly modified.
///
/// The parser is the SAX-shaped [`XmlReader`] (see the
/// [`sax`](crate::jdk::sax) module for how it differs from the JDK's), and
/// the consumer is borrowed for the parser's lifetime. Where Java throws
/// `NullPointerException` (a `null` consumer, a `hyphen` element before any
/// `exceptions`), the handler returns that exception, with the JDK's
/// helpful message.
///
/// Equivalent to `org.apache.lucene.analysis.compound.hyphenation.PatternParser`,
/// which extends `DefaultHandler`.
pub struct PatternParser<'a> {
    parser: XmlReader,
    curr_element: i32,
    consumer: Option<&'a mut dyn PatternConsumer>,
    token: Vec<u16>,
    exception: Option<Vec<HyphenatedWordPart>>,
    hyphen_char: u16,
}

impl std::fmt::Debug for PatternParser<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PatternParser")
            .field("curr_element", &self.curr_element)
            .field("has_consumer", &self.consumer.is_some())
            .field("token", &String::from_utf16_lossy(&self.token))
            .field("exception", &self.exception)
            .field("hyphen_char", &self.hyphen_char)
            .finish()
    }
}

impl Default for PatternParser<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// Java's `NullPointerException` with the JDK's helpful message.
fn null_pointer(message: &str) -> LuceneError {
    LuceneError::unchecked("java.lang.NullPointerException", message)
}

impl<'a> PatternParser<'a> {
    /// Equivalent to `PatternParser()`.
    pub fn new() -> Self {
        Self {
            token: Vec::new(),
            parser: Self::create_parser(),
            curr_element: 0,
            consumer: None,
            exception: None,
            hyphen_char: '-' as u16, // default
        }
    }

    /// Equivalent to `PatternParser(PatternConsumer consumer)`.
    pub fn with_consumer(consumer: &'a mut dyn PatternConsumer) -> Self {
        let mut parser = Self::new();
        parser.consumer = Some(consumer);
        parser
    }

    /// Equivalent to `PatternParser.setConsumer(PatternConsumer)`.
    pub fn set_consumer(&mut self, consumer: &'a mut dyn PatternConsumer) {
        self.consumer = Some(consumer);
    }

    /// Parses a hyphenation pattern file.
    ///
    /// Equivalent to `PatternParser.parse(String filename)`.
    ///
    /// # Errors
    ///
    /// As [`parse`](Self::parse).
    pub fn parse_file_name(&mut self, filename: &str) -> Result<()> {
        self.parse(InputSource::from_system_id(filename))
    }

    /// Parses a hyphenation pattern file.
    ///
    /// Equivalent to `PatternParser.parse(InputSource)`.
    ///
    /// # Errors
    ///
    /// An `IOException` in case of an exception while parsing: the parser's,
    /// or one whose cause is the `SAXException` it threw (its message is the
    /// cause's `toString()`, as `new IOException(e)` makes it); and the
    /// unchecked exceptions of the handler.
    pub fn parse(&mut self, source: InputSource) -> Result<()> {
        let parser = self.parser;
        match parser.parse(source, self) {
            Ok(()) => Ok(()),
            Err(e) => match SaxException::of(&e) {
                Some(sax) => {
                    let message = sax.to_java_string();
                    Err(LuceneError::from(LuceneErrorKind::Io(
                        io_exception_with_cause(message, Some(e)),
                    )))
                }
                None => Err(e),
            },
        }
    }

    /// Creates a SAX parser using JAXP
    ///
    /// Equivalent to the package-private `PatternParser.createParser()`: the
    /// JDK's namespace-aware parser cannot fail to be created, nor can this
    /// one.
    pub(crate) fn create_parser() -> XmlReader {
        XmlReader::new()
    }

    /// Equivalent to the protected `PatternParser.readToken(StringBuilder)`:
    /// the next whitespace-terminated word of `chars` (which it consumes),
    /// joined to the partial token of the previous calls, or `None`.
    pub(crate) fn read_token(&mut self, chars: &mut Vec<u16>) -> Option<Vec<u16>> {
        let mut space = false;
        let mut i = 0;
        while i < chars.len() {
            if is_whitespace(chars[i] as i32) {
                space = true;
            } else {
                break;
            }
            i += 1;
        }
        if space {
            // chars.delete(0,i);
            chars.drain(..i);
            if !self.token.is_empty() {
                return Some(std::mem::take(&mut self.token));
            }
        }
        space = false;
        i = 0;
        while i < chars.len() {
            if is_whitespace(chars[i] as i32) {
                space = true;
                break;
            }
            i += 1;
        }
        self.token.extend_from_slice(&chars[..i]);
        // chars.delete(0,i);
        chars.drain(..i);
        if space {
            return Some(std::mem::take(&mut self.token));
        }
        self.token.extend_from_slice(chars);
        None
    }

    /// Equivalent to the protected static `PatternParser.getPattern(String)`:
    /// `word` without its digits.
    pub(crate) fn get_pattern(word: &[u16]) -> Vec<u16> {
        word.iter()
            .copied()
            .filter(|&c| !is_digit(c as i32))
            .collect()
    }

    /// Equivalent to the protected `PatternParser.normalizeException(ArrayList<?>)`.
    pub(crate) fn normalize_exception(&self, ex: &[HyphenatedWordPart]) -> Vec<HyphenatedWordPart> {
        let mut res = Vec::new();
        for item in ex {
            match item {
                HyphenatedWordPart::String(str) => {
                    let mut buf = Vec::new();
                    for &c in str {
                        if c != self.hyphen_char {
                            buf.push(c);
                        } else {
                            res.push(HyphenatedWordPart::String(std::mem::take(&mut buf)));
                            // we use here hyphenChar which is not necessarily
                            // the one to be printed
                            res.push(HyphenatedWordPart::Hyphen(Hyphen::new(
                                Some(String::from_utf16_lossy(&[self.hyphen_char])),
                                None,
                                None,
                            )));
                        }
                    }
                    if !buf.is_empty() {
                        res.push(HyphenatedWordPart::String(buf));
                    }
                }
                HyphenatedWordPart::Hyphen(_) => res.push(item.clone()),
            }
        }
        res
    }

    /// Equivalent to the protected `PatternParser.getExceptionWord(ArrayList<?>)`.
    pub(crate) fn get_exception_word(&self, ex: &[HyphenatedWordPart]) -> Vec<u16> {
        let mut res = Vec::new();
        for item in ex {
            match item {
                HyphenatedWordPart::String(s) => res.extend_from_slice(s),
                HyphenatedWordPart::Hyphen(h) => {
                    if let Some(no_break) = &h.no_break {
                        res.extend(no_break.encode_utf16());
                    }
                }
            }
        }
        res
    }

    /// Equivalent to the protected static `PatternParser.getInterletterValues(String)`.
    pub(crate) fn get_interletter_values(pat: &[u16]) -> Vec<u16> {
        let mut il = Vec::new();
        let mut word = pat.to_vec();
        word.push('a' as u16); // add dummy letter to serve as sentinel
        let len = word.len();
        let mut i = 0;
        while i < len {
            let c = word[i];
            if is_digit(c as i32) {
                il.push(c);
                i += 1;
            } else {
                il.push('0' as u16);
            }
            i += 1;
        }
        il
    }

    fn consumer(&mut self, method: &str) -> Result<&mut (dyn PatternConsumer + 'a)> {
        match self.consumer.as_deref_mut() {
            Some(consumer) => Ok(consumer),
            None => Err(null_pointer(&format!(
                "Cannot invoke \"org.apache.lucene.analysis.compound.hyphenation.PatternConsumer.{method}\" because \"this.consumer\" is null"
            ))),
        }
    }

    fn exception_mut(&mut self, method: &str) -> Result<&mut Vec<HyphenatedWordPart>> {
        self.exception.as_mut().ok_or_else(|| {
            null_pointer(&format!(
                "Cannot invoke \"java.util.ArrayList.{method}\" because \"this.exception\" is null"
            ))
        })
    }

    /// `exception.add(word); exception = normalizeException(exception);
    /// consumer.addException(getExceptionWord(exception), (ArrayList) exception.clone());`
    fn add_exception_word(&mut self, word: Vec<u16>) -> Result<()> {
        self.exception_mut("add(Object)")?
            .push(HyphenatedWordPart::String(word));
        let normalized = {
            let exception = self.exception.as_deref().unwrap_or_default();
            self.normalize_exception(exception)
        };
        let exception_word = self.get_exception_word(&normalized);
        let copy = normalized.clone();
        self.exception = Some(normalized);
        self.consumer("addException(String, java.util.ArrayList)")?
            .add_exception(&exception_word, copy);
        Ok(())
    }
}

impl DefaultHandler for PatternParser<'_> {
    /// Equivalent to `PatternParser.resolveEntity(String, String)`: the
    /// module's `hyphenation.dtd` for a system id naming it or the public id
    /// `hyphenation-info`. Java answers the DTD's class-path URL
    /// (`getResource("hyphenation.dtd").toExternalForm()`, a `jar:` URL);
    /// Rust has no class path, so the source carries the embedded file as its
    /// byte stream and its resource name as its system id.
    ///
    /// # Errors
    ///
    /// `SAXException` (`External Entity resolving unsupported:  publicId="…"
    /// systemId="…"`, `null` printed for a missing id) for any other entity.
    fn resolve_entity(
        &mut self,
        public_id: Option<&str>,
        system_id: Option<&str>,
    ) -> Result<Option<InputSource>> {
        // supply the internal hyphenation.dtd if possible
        if system_id.is_some_and(|id| HYPHENATION_DTD_PATTERN.matches(id))
            || public_id == Some("hyphenation-info")
        {
            let dtd = crate::resources::get_resource(HYPHENATION_DTD)
                .expect("INVARIANT: the module embeds hyphenation.dtd");
            let mut source = InputSource::from_byte_stream(Box::new(Cursor::new(dtd)));
            source.set_system_id(Some(HYPHENATION_DTD.to_string()));
            return Ok(Some(source));
        }
        Err(LuceneError::from(SaxException::new(format!(
            "External Entity resolving unsupported:  publicId=\"{}\" systemId=\"{}\"",
            public_id.unwrap_or("null"),
            system_id.unwrap_or("null")
        ))))
    }

    /// Equivalent to `PatternParser.startElement(String, String, String, Attributes)`.
    fn start_element(
        &mut self,
        _uri: &str,
        local: &str,
        _raw: &str,
        attrs: &Attributes,
    ) -> Result<()> {
        if local == "hyphen-char" {
            if let Some(h) = attrs.get_value("value") {
                let units: Vec<u16> = h.encode_utf16().collect();
                if units.len() == 1 {
                    self.hyphen_char = units[0];
                }
            }
        } else if local == "classes" {
            self.curr_element = ELEM_CLASSES;
        } else if local == "patterns" {
            self.curr_element = ELEM_PATTERNS;
        } else if local == "exceptions" {
            self.curr_element = ELEM_EXCEPTIONS;
            self.exception = Some(Vec::new());
        } else if local == "hyphen" {
            if !self.token.is_empty() {
                let token = self.token.clone();
                self.exception_mut("add(Object)")?
                    .push(HyphenatedWordPart::String(token));
            }
            let hyphen = Hyphen::new(
                attrs.get_value("pre").map(str::to_string),
                attrs.get_value("no").map(str::to_string),
                attrs.get_value("post").map(str::to_string),
            );
            self.exception_mut("add(Object)")?
                .push(HyphenatedWordPart::Hyphen(hyphen));
            self.curr_element = ELEM_HYPHEN;
        }
        self.token.clear();
        Ok(())
    }

    /// Equivalent to `PatternParser.endElement(String, String, String)`.
    fn end_element(&mut self, _uri: &str, _local: &str, _raw: &str) -> Result<()> {
        if !self.token.is_empty() {
            let word = self.token.clone();
            match self.curr_element {
                ELEM_CLASSES => self.consumer("addClass(String)")?.add_class(&word),
                ELEM_EXCEPTIONS => self.add_exception_word(word)?,
                ELEM_PATTERNS => {
                    let pattern = Self::get_pattern(&word);
                    let values = Self::get_interletter_values(&word);
                    self.consumer("addPattern(String, String)")?
                        .add_pattern(&pattern, &values);
                }
                ELEM_HYPHEN => {
                    // nothing to do
                }
                _ => {}
            }
            if self.curr_element != ELEM_HYPHEN {
                self.token.clear();
            }
        }
        if self.curr_element == ELEM_HYPHEN {
            self.curr_element = ELEM_EXCEPTIONS;
        } else {
            self.curr_element = 0;
        }
        Ok(())
    }

    /// Equivalent to `PatternParser.characters(char[], int, int)`.
    fn characters(&mut self, ch: &[u16], start: usize, length: usize) -> Result<()> {
        let mut chars = ch[start..start + length].to_vec();
        let mut word = self.read_token(&mut chars);
        while let Some(w) = word {
            match self.curr_element {
                ELEM_CLASSES => self.consumer("addClass(String)")?.add_class(&w),
                ELEM_EXCEPTIONS => {
                    self.add_exception_word(w)?;
                    self.exception_mut("clear()")?.clear();
                }
                ELEM_PATTERNS => {
                    let pattern = Self::get_pattern(&w);
                    let values = Self::get_interletter_values(&w);
                    self.consumer("addPattern(String, String)")?
                        .add_pattern(&pattern, &values);
                }
                _ => {}
            }
            word = self.read_token(&mut chars);
        }
        Ok(())
    }
}
