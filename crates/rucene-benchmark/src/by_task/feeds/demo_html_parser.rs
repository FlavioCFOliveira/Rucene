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

//! The types declared in `DemoHTMLParser.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nekohtml::java::{string_object, Exception, JResult, Reader};
use nekohtml::org::cyberneko::html::parsers::sax_parser::SAXParser;
use nekohtml::org::xml::sax::attributes::Attributes;
use nekohtml::org::xml::sax::content_handler::ContentHandler;
use nekohtml::org::xml::sax::entity_resolver::EntityResolver;
use nekohtml::org::xml::sax::error_handler::ErrorHandler;
use nekohtml::org::xml::sax::helpers::default_handler::DefaultHandler;
use nekohtml::org::xml::sax::input_source::InputSource;
use nekohtml::org::xml::sax::locator::Locator;
use nekohtml::org::xml::sax::sax_exception::SAXException;
use nekohtml::org::xml::sax::sax_parse_exception::SAXParseException;
use rucene::error::{LuceneError, Result};
use rucene_analysis_common::jdk::io::ReadChars;
use rucene_queryparser::jdk::date::Date;

use crate::by_task::feeds::doc_data::DocData;
use crate::by_task::feeds::html_parser::HTMLParser;
use crate::by_task::feeds::trec_content_source::TrecContentSource;
use crate::jdk;
use crate::jdk::properties::Properties;

/// `new StringReader(s)`: the UTF-16 units of `s`.
struct StringReader {
    units: Vec<u16>,
    pos: usize,
}

impl StringReader {
    fn new(s: &str) -> Self {
        StringReader {
            units: s.encode_utf16().collect(),
            pos: 0,
        }
    }
}

impl ReadChars for StringReader {
    fn read_chars(&mut self, cbuf: &mut [u16]) -> Result<usize> {
        let n = cbuf.len().min(self.units.len() - self.pos);
        cbuf[..n].copy_from_slice(&self.units[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }

    fn close_chars(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A `Reader` over `s`.
fn string_reader(s: &str) -> Reader {
    Rc::new(RefCell::new(StringReader::new(s)))
}

/// `ENDLINE_ELEMENTS`: HTML elements that cause a line break (they are
/// block-elements).
const ENDLINE_ELEMENTS: [&str; 27] = [
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "div",
    "ul",
    "ol",
    "dl",
    "pre",
    "hr",
    "blockquote",
    "address",
    "fieldset",
    "table",
    "form",
    "noscript",
    "li",
    "dt",
    "dd",
    "noframes",
    "br",
    "tr",
    "select",
    "option",
];

/// `SUPPRESS_ELEMENTS`: HTML elements with contents that are ignored.
const SUPPRESS_ELEMENTS: [&str; 2] = ["style", "script"];

/// The anonymous `DefaultHandler` of `Parser(InputSource)`.
#[derive(Default)]
struct Handler {
    in_body: Cell<i32>,
    in_head: Cell<i32>,
    in_title: Cell<i32>,
    suppressed: Cell<i32>,
    meta_tags: RefCell<Properties>,
    title: RefCell<Vec<u16>>,
    body: RefCell<Vec<u16>>,
}

impl ContentHandler for Handler {
    fn set_document_locator(&self, _locator: Rc<dyn Locator>) {}

    fn start_document(&self) -> JResult<()> {
        Ok(())
    }

    fn end_document(&self) -> JResult<()> {
        Ok(())
    }

    fn start_prefix_mapping(&self, _prefix: &str, _uri: &str) -> JResult<()> {
        Ok(())
    }

    fn end_prefix_mapping(&self, _prefix: &str) -> JResult<()> {
        Ok(())
    }

    fn start_element(
        &self,
        _uri: &str,
        local_name: &str,
        _qname: &str,
        atts: &dyn Attributes,
    ) -> JResult<()> {
        if self.in_head.get() > 0 {
            if local_name == "title" {
                self.in_title.set(self.in_title.get() + 1);
            } else if local_name == "meta" {
                let name = atts
                    .get_value_by_qname("name")
                    .or_else(|| atts.get_value_by_qname("http-equiv"));
                let val = atts.get_value_by_qname("content");
                if let (Some(name), Some(val)) = (name, val) {
                    self.meta_tags
                        .borrow_mut()
                        .set_property(name.to_lowercase(), val);
                }
            }
        } else if self.in_body.get() > 0 {
            if SUPPRESS_ELEMENTS.contains(&local_name) {
                self.suppressed.set(self.suppressed.get() + 1);
            } else if local_name == "img" {
                // the original javacc-based parser preserved <IMG alt="..."/>
                // attribute as body text in [] parenthesis:
                if let Some(alt) = atts.get_value_by_qname("alt") {
                    let mut body = self.body.borrow_mut();
                    body.push(u16::from(b'['));
                    body.extend(alt.encode_utf16());
                    body.push(u16::from(b']'));
                }
            }
        } else if local_name == "body" {
            self.in_body.set(self.in_body.get() + 1);
        } else if local_name == "head" {
            self.in_head.set(self.in_head.get() + 1);
        } else if local_name == "frameset" {
            return Err(Exception::Sax(SAXException::with_message(Some(
                "This parser does not support HTML framesets.",
            ))));
        }
        Ok(())
    }

    fn end_element(&self, _uri: &str, local_name: &str, _qname: &str) -> JResult<()> {
        if self.in_body.get() > 0 {
            if local_name == "body" {
                self.in_body.set(self.in_body.get() - 1);
            } else if ENDLINE_ELEMENTS.contains(&local_name) {
                self.body.borrow_mut().push(u16::from(b'\n'));
            } else if SUPPRESS_ELEMENTS.contains(&local_name) {
                self.suppressed.set(self.suppressed.get() - 1);
            }
        } else if self.in_head.get() > 0 {
            if local_name == "head" {
                self.in_head.set(self.in_head.get() - 1);
            } else if self.in_title.get() > 0 && local_name == "title" {
                self.in_title.set(self.in_title.get() - 1);
            }
        }
        Ok(())
    }

    fn characters(&self, ch: &[u16], start: i32, length: i32) -> JResult<()> {
        let chunk = &ch[start as usize..(start + length) as usize];
        if self.in_body.get() > 0 && self.suppressed.get() == 0 {
            self.body.borrow_mut().extend_from_slice(chunk);
        } else if self.in_title.get() > 0 {
            self.title.borrow_mut().extend_from_slice(chunk);
        }
        Ok(())
    }

    fn ignorable_whitespace(&self, _ch: &[u16], _start: i32, _length: i32) -> JResult<()> {
        Ok(())
    }

    fn processing_instruction(&self, _target: &str, _data: Option<&str>) -> JResult<()> {
        Ok(())
    }

    fn skipped_entity(&self, _name: &str) -> JResult<()> {
        Ok(())
    }
}

impl ErrorHandler for Handler {
    fn warning(&self, exception: &SAXParseException) -> JResult<()> {
        DefaultHandler::new().warning(exception)
    }

    fn error(&self, exception: &SAXParseException) -> JResult<()> {
        DefaultHandler::new().error(exception)
    }

    fn fatal_error(&self, exception: &SAXParseException) -> JResult<()> {
        DefaultHandler::new().fatal_error(exception)
    }
}

impl EntityResolver for Handler {
    /// `resolveEntity`: disable network access caused by DTDs.
    fn resolve_entity(
        &self,
        _public_id: Option<&str>,
        _system_id: Option<&str>,
    ) -> JResult<Option<InputSource>> {
        Ok(Some(InputSource::with_character_stream(Some(
            string_reader(""),
        ))))
    }
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.DemoHTMLParser.Parser`
/// (`public static final class Parser`): the actual parser to read HTML
/// documents.
pub struct Parser {
    /// `metaTags`.
    pub meta_tags: Properties,
    /// `title`.
    pub title: String,
    /// `body`.
    pub body: String,
}

// `JResult` carries nekohtml's Java exception by value (see `nekohtml`).
#[allow(clippy::result_large_err)]
impl Parser {
    /// `Parser(Reader reader)`.
    ///
    /// # Errors
    ///
    /// The `IOException` or `SAXException` of parsing.
    pub fn from_reader(reader: &str) -> JResult<Self> {
        Self::new(&InputSource::with_character_stream(Some(string_reader(
            reader,
        ))))
    }

    /// `Parser(InputSource source)`.
    ///
    /// # Errors
    ///
    /// The `IOException` or `SAXException` of parsing.
    pub fn new(source: &InputSource) -> JResult<Self> {
        let parser = SAXParser::new();
        parser.set_feature("http://xml.org/sax/features/namespaces", true)?;
        parser.set_feature("http://cyberneko.org/html/features/balance-tags", true)?;
        parser.set_feature("http://cyberneko.org/html/features/report-errors", false)?;
        parser.set_property(
            "http://cyberneko.org/html/properties/names/elems",
            Some(string_object("lower")),
        )?;
        parser.set_property(
            "http://cyberneko.org/html/properties/names/attrs",
            Some(string_object("lower")),
        )?;

        let handler = Rc::new(Handler::default());
        parser.set_content_handler(Some(Rc::clone(&handler) as Rc<dyn ContentHandler>));
        parser.set_error_handler(Some(Rc::clone(&handler) as Rc<dyn ErrorHandler>));
        parser.parse(source)?;

        // the javacc-based parser trimmed title (which should be done for HTML in all cases):
        let title = jdk::java_trim(&String::from_utf16_lossy(&handler.title.borrow())).to_string();
        // assign body text
        let body = String::from_utf16_lossy(&handler.body.borrow());
        let meta_tags = handler.meta_tags.borrow().clone();
        Ok(Parser {
            meta_tags,
            title,
            body,
        })
    }
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.DemoHTMLParser`
/// (`public class DemoHTMLParser implements HTMLParser`): simple HTML Parser
/// extracting title, meta tags, and body text that is based on
/// [NekoHTML](http://nekohtml.sourceforge.net/).
#[derive(Debug, Default)]
pub struct DemoHTMLParser;

impl DemoHTMLParser {
    /// `new DemoHTMLParser()`.
    pub fn new() -> Self {
        DemoHTMLParser
    }

    /// `parse(DocData docData, String name, Date date, InputSource source,
    /// TrecContentSource trecSrc)`.
    ///
    /// # Errors
    ///
    /// The `IOException` or `SAXException` of parsing.
    #[allow(clippy::result_large_err)]
    pub fn parse_source(
        &self,
        doc_data: &mut DocData,
        name: Option<&str>,
        date: Option<&Date>,
        source: &InputSource,
        trec_src: Option<&TrecContentSource>,
    ) -> JResult<()> {
        let p = Parser::new(source)?;

        // properties
        let props = p.meta_tags;
        let mut date = date.cloned();
        if let Some(date_str) = props.get_property("date") {
            let trec_src =
                trec_src.ok_or_else(|| Exception::runtime(LuceneError::null_pointer()))?;
            if let Some(new_date) = trec_src.parse_date(date_str).map_err(Exception::runtime)? {
                date = Some(new_date);
            }
        }

        doc_data.clear();
        doc_data.set_name(name.map(str::to_string));
        doc_data.set_body(Some(p.body));
        doc_data.set_title(Some(p.title));
        doc_data.set_props(Some(props));
        doc_data.set_date_from(date.as_ref());
        Ok(())
    }
}

impl HTMLParser for DemoHTMLParser {
    fn parse(
        &self,
        doc_data: &mut DocData,
        name: Option<&str>,
        date: Option<&Date>,
        reader: &str,
        trec_src: Option<&TrecContentSource>,
    ) -> Result<()> {
        let source = InputSource::with_character_stream(Some(string_reader(reader)));
        match self.parse_source(doc_data, name, date, &source, trec_src) {
            Ok(()) => Ok(()),
            Err(Exception::Sax(_)) => Err(jdk::io_exception(
                "SAX exception occurred while parsing HTML document.",
            )),
            Err(e) => Err(e.into_lucene_error()),
        }
    }
}
