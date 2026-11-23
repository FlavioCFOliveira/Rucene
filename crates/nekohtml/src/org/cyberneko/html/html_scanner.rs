/* 
 * Copyright 2002-2009 Andy Clark, Marc Guillemot
 * 
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `HTMLScanner.java`, of the Java package `org.cyberneko.html` (NekoHTML 1.9.17).

use std::cell::{Cell, RefCell, RefMut};
use std::io::Read;
use std::ops::{Deref, DerefMut};
use std::rc::{Rc, Weak};

use rucene::error::LuceneError;
use rucene_analysis_common::jdk::charset::Charset;
use rucene_analysis_common::jdk::io::InputStreamReader;
use rucene_queryparser::jdk::object::JavaObject;

use crate::java::{
    char_upper, equals_ignore_case, from_chars, interface_object, io_exception, is_java_whitespace,
    is_letter_or_digit, java_len, object_as_interface, object_as_str, string_object, to_chars, to_lower_case,
    to_upper_case, trim, Exception, InputStream, JResult, Object, Reader,
};
use crate::org::apache::xerces::util::encoding_map::EncodingMap;
use crate::org::apache::xerces::util::namespace_support::NamespaceSupport;
use crate::org::apache::xerces::util::uri::URI;
use crate::org::apache::xerces::util::xml_attributes_impl::XMLAttributesImpl;
use crate::org::apache::xerces::util::xml_resource_identifier_impl::XMLResourceIdentifierImpl;
use crate::org::apache::xerces::util::xml_string_buffer::XMLStringBuffer;
use crate::org::apache::xerces::xni::augmentations::Augmentations;
use crate::org::apache::xerces::xni::namespace_context::NamespaceContext;
use crate::org::apache::xerces::xni::parser::xml_component::XMLComponent;
use crate::org::apache::xerces::xni::parser::xml_component_manager::XMLComponentManager;
use crate::org::apache::xerces::xni::parser::xml_document_scanner::XMLDocumentScanner;
use crate::org::apache::xerces::xni::parser::xml_document_source::XMLDocumentSource;
use crate::org::apache::xerces::xni::parser::xml_input_source::XMLInputSource;
use crate::org::apache::xerces::xni::qname::QName;
use crate::org::apache::xerces::xni::xml_attributes::XMLAttributes;
use crate::org::apache::xerces::xni::xml_document_handler::XMLDocumentHandler;
use crate::org::apache::xerces::xni::xml_locator::XMLLocator;
use crate::org::apache::xerces::xni::xml_string::XMLString;
use crate::org::cyberneko::html::html_augmentations::HTMLAugmentations;
use crate::org::cyberneko::html::html_component::HTMLComponent;
use crate::org::cyberneko::html::html_elements;
use crate::org::cyberneko::html::html_entities;
use crate::org::cyberneko::html::html_error_reporter::HTMLErrorReporter;
use crate::org::cyberneko::html::html_event_info::{HTMLEventInfo, SynthesizedItem};
use crate::org::cyberneko::html::xercesbridge::xerces_bridge;

// doctype info: HTML 4.01 strict

/// HTML 4.01 strict public identifier ("-//W3C//DTD HTML 4.01//EN").
pub const HTML_4_01_STRICT_PUBID: &str = "-//W3C//DTD HTML 4.01//EN";
/// HTML 4.01 strict system identifier ("http://www.w3.org/TR/html4/strict.dtd").
pub const HTML_4_01_STRICT_SYSID: &str = "http://www.w3.org/TR/html4/strict.dtd";
/// HTML 4.01 transitional public identifier ("-//W3C//DTD HTML 4.01 Transitional//EN").
pub const HTML_4_01_TRANSITIONAL_PUBID: &str = "-//W3C//DTD HTML 4.01 Transitional//EN";
/// HTML 4.01 transitional system identifier ("http://www.w3.org/TR/html4/loose.dtd").
pub const HTML_4_01_TRANSITIONAL_SYSID: &str = "http://www.w3.org/TR/html4/loose.dtd";
/// HTML 4.01 frameset public identifier ("-//W3C//DTD HTML 4.01 Frameset//EN").
pub const HTML_4_01_FRAMESET_PUBID: &str = "-//W3C//DTD HTML 4.01 Frameset//EN";
/// HTML 4.01 frameset system identifier ("http://www.w3.org/TR/html4/frameset.dtd").
pub const HTML_4_01_FRAMESET_SYSID: &str = "http://www.w3.org/TR/html4/frameset.dtd";

// features

/// Include infoset augmentations.
pub const AUGMENTATIONS: &str = "http://cyberneko.org/html/features/augmentations";
/// Report errors.
pub const REPORT_ERRORS: &str = "http://cyberneko.org/html/features/report-errors";
/// Notify character entity references (e.g. &amp;#32;, &amp;#x20;, etc).
pub const NOTIFY_CHAR_REFS: &str = "http://apache.org/xml/features/scanner/notify-char-refs";
/// Notify handler of built-in entity references (e.g. &amp;amp;, &amp;lt;,
/// etc).
pub const NOTIFY_XML_BUILTIN_REFS: &str = "http://apache.org/xml/features/scanner/notify-builtin-refs";
/// Notify handler of built-in entity references (e.g. &amp;nobr;,
/// &amp;copy;, etc).
pub const NOTIFY_HTML_BUILTIN_REFS: &str = "http://cyberneko.org/html/features/scanner/notify-builtin-refs";
/// Fix Microsoft Windows&reg; character entity references.
pub const FIX_MSWINDOWS_REFS: &str = "http://cyberneko.org/html/features/scanner/fix-mswindows-refs";
/// Strip HTML comment delimiters ("&lt;!&minus;&minus;" and
/// "&minus;&minus;&gt;") from SCRIPT tag contents.
pub const SCRIPT_STRIP_COMMENT_DELIMS: &str =
    "http://cyberneko.org/html/features/scanner/script/strip-comment-delims";
/// Strip XHTML CDATA delimiters ("&lt;![CDATA[" and "]]&gt;") from SCRIPT
/// tag contents.
pub const SCRIPT_STRIP_CDATA_DELIMS: &str = "http://cyberneko.org/html/features/scanner/script/strip-cdata-delims";
/// Strip HTML comment delimiters from STYLE tag contents.
pub const STYLE_STRIP_COMMENT_DELIMS: &str = "http://cyberneko.org/html/features/scanner/style/strip-comment-delims";
/// Strip XHTML CDATA delimiters from STYLE tag contents.
pub const STYLE_STRIP_CDATA_DELIMS: &str = "http://cyberneko.org/html/features/scanner/style/strip-cdata-delims";
/// Ignore specified charset found in the &lt;meta equiv='Content-Type'
/// content='text/html;charset=&hellip;'&gt; tag or in the &lt;?xml
/// &hellip; encoding='&hellip;'&gt; processing instruction.
pub const IGNORE_SPECIFIED_CHARSET: &str = "http://cyberneko.org/html/features/scanner/ignore-specified-charset";
/// Scan CDATA sections.
pub const CDATA_SECTIONS: &str = "http://cyberneko.org/html/features/scanner/cdata-sections";
/// Override doctype declaration public and system identifiers.
pub const OVERRIDE_DOCTYPE: &str = "http://cyberneko.org/html/features/override-doctype";
/// Insert document type declaration.
pub const INSERT_DOCTYPE: &str = "http://cyberneko.org/html/features/insert-doctype";
/// Parse &lt;noscript&gt;...&lt;/noscript&gt; content.
pub const PARSE_NOSCRIPT_CONTENT: &str = "http://cyberneko.org/html/features/parse-noscript-content";
/// Allows self closing &lt;iframe/&gt; tag.
pub const ALLOW_SELFCLOSING_IFRAME: &str = "http://cyberneko.org/html/features/scanner/allow-selfclosing-iframe";
/// Allows self closing tags e.g. &lt;div/&gt; (XHTML).
pub const ALLOW_SELFCLOSING_TAGS: &str = "http://cyberneko.org/html/features/scanner/allow-selfclosing-tags";
/// Normalize attribute values.
pub const NORMALIZE_ATTRIBUTES: &str = "http://cyberneko.org/html/features/scanner/normalize-attrs";

/// Recognized features.
const RECOGNIZED_FEATURES: [&str; 18] = [
    AUGMENTATIONS,
    REPORT_ERRORS,
    NOTIFY_CHAR_REFS,
    NOTIFY_XML_BUILTIN_REFS,
    NOTIFY_HTML_BUILTIN_REFS,
    FIX_MSWINDOWS_REFS,
    SCRIPT_STRIP_CDATA_DELIMS,
    SCRIPT_STRIP_COMMENT_DELIMS,
    STYLE_STRIP_CDATA_DELIMS,
    STYLE_STRIP_COMMENT_DELIMS,
    IGNORE_SPECIFIED_CHARSET,
    CDATA_SECTIONS,
    OVERRIDE_DOCTYPE,
    INSERT_DOCTYPE,
    NORMALIZE_ATTRIBUTES,
    PARSE_NOSCRIPT_CONTENT,
    ALLOW_SELFCLOSING_IFRAME,
    ALLOW_SELFCLOSING_TAGS,
];

/// Recognized features defaults.
const RECOGNIZED_FEATURES_DEFAULTS: [Option<bool>; 18] = [
    None,
    None,
    Some(false),
    Some(false),
    Some(false),
    Some(false),
    Some(false),
    Some(false),
    Some(false),
    Some(false),
    Some(false),
    Some(false),
    Some(false),
    Some(false),
    Some(false),
    Some(true),
    Some(false),
    Some(false),
];

// properties

/// Modify HTML element names: { "upper", "lower", "default" }.
pub const NAMES_ELEMS: &str = "http://cyberneko.org/html/properties/names/elems";
/// Modify HTML attribute names: { "upper", "lower", "default" }.
pub const NAMES_ATTRS: &str = "http://cyberneko.org/html/properties/names/attrs";
/// Default encoding.
pub const DEFAULT_ENCODING: &str = "http://cyberneko.org/html/properties/default-encoding";
/// Error reporter.
pub const ERROR_REPORTER: &str = "http://cyberneko.org/html/properties/error-reporter";
/// Doctype declaration public identifier.
pub const DOCTYPE_PUBID: &str = "http://cyberneko.org/html/properties/doctype/pubid";
/// Doctype declaration system identifier.
pub const DOCTYPE_SYSID: &str = "http://cyberneko.org/html/properties/doctype/sysid";

/// Recognized properties.
const RECOGNIZED_PROPERTIES: [&str; 6] = [
    NAMES_ELEMS,
    NAMES_ATTRS,
    DEFAULT_ENCODING,
    ERROR_REPORTER,
    DOCTYPE_PUBID,
    DOCTYPE_SYSID,
];

/// Recognized properties defaults.
fn recognized_properties_default(i: usize) -> Option<Object> {
    match i {
        2 => Some(string_object("Windows-1252")),
        4 => Some(string_object(HTML_4_01_TRANSITIONAL_PUBID)),
        5 => Some(string_object(HTML_4_01_TRANSITIONAL_SYSID)),
        _ => None,
    }
}

// states

/// State: content.
pub const STATE_CONTENT: i16 = 0;
/// State: markup bracket.
pub const STATE_MARKUP_BRACKET: i16 = 1;
/// State: start document.
pub const STATE_START_DOCUMENT: i16 = 10;
/// State: end document.
pub const STATE_END_DOCUMENT: i16 = 11;

// modify HTML names

/// Don't modify HTML names.
pub const NAMES_NO_CHANGE: i16 = 0;
/// Uppercase HTML names.
pub const NAMES_UPPERCASE: i16 = 1;
/// Lowercase HTML names.
pub const NAMES_LOWERCASE: i16 = 2;

// defaults

/// Default buffer size.
pub const DEFAULT_BUFFER_SIZE: i32 = 2048;

// debugging

/// Set to true to debug changes in the scanner.
const DEBUG_SCANNER: bool = false;
/// Set to true to debug changes in the scanner state.
const DEBUG_SCANNER_STATE: bool = false;
/// Set to true to debug the buffer.
const DEBUG_BUFFER: bool = false;
/// Set to true to debug character encoding handling.
const DEBUG_CHARSET: bool = false;
/// Set to true to debug callbacks.
const DEBUG_CALLBACKS: bool = false;

/// Synthesized event info item.
///
/// Equivalent to `HTMLScanner.SYNTHESIZED_ITEM`.
pub const SYNTHESIZED_ITEM: SynthesizedItem = SynthesizedItem;

/// `ENTITY_CHARS`: the characters of an entity reference name.
fn entity_char(c: i32) -> bool {
    const STR: &str = "-.0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ_abcdefghijklmnopqrstuvwxyz";
    c >= 0 && c < 128 && STR.as_bytes().contains(&(c as u8))
}

/// `new EOFException()`.
fn eof() -> Exception {
    io_exception("java.io.EOFException", None)
}

/// `e instanceof EOFException`: the Java class the I/O error carries
/// (`LuceneError::java_simple_name` of every I/O error is `IOException`).
fn is_eof(e: &Exception) -> bool {
    matches!(
        e,
        Exception::Io(io) if matches!(
            io.kind(),
            rucene::error::LuceneErrorKind::Io(err)
                if rucene::store::java_nio_file::java_class_of(err) == "java.io.EOFException"
        )
    )
}

fn npe() -> Exception {
    Exception::runtime(LuceneError::null_pointer())
}

/// `String.valueOf(Object)` of a property value: the string, or `"null"`.
fn value_of(o: &Option<Object>) -> String {
    o.as_ref()
        .and_then(object_as_str)
        .map(str::to_string)
        .unwrap_or_else(|| "null".to_string())
}

/// A field Java reads and writes in place while it passes it on to a
/// handler (a buffer or the attribute list): the port takes it out for the
/// call and puts it back when done, so that a handler that re-enters the
/// scanner (`evaluateInputSource`) works on a fresh object instead of one
/// borrowed by the caller.
#[derive(Debug, Default)]
struct Slot<T: Default>(RefCell<Option<T>>);

/// The value of a [`Slot`] while it is out; put back on drop.
struct SlotGuard<'a, T: Default> {
    slot: &'a Slot<T>,
    value: Option<T>,
}

impl<T: Default> Slot<T> {
    fn new(value: T) -> Self {
        Self(RefCell::new(Some(value)))
    }

    fn take(&self) -> SlotGuard<'_, T> {
        let value = self.0.borrow_mut().take().unwrap_or_default();
        SlotGuard {
            slot: self,
            value: Some(value),
        }
    }
}

impl<T: Default> Deref for SlotGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.value.as_ref().expect("INVARIANT: present until dropped")
    }
}

impl<T: Default> DerefMut for SlotGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.value.as_mut().expect("INVARIANT: present until dropped")
    }
}

impl<T: Default> Drop for SlotGuard<'_, T> {
    fn drop(&mut self) {
        *self.slot.0.borrow_mut() = self.value.take();
    }
}

/// The scanner `fScanner` designates: one of the two scanners of the
/// enclosing `HTMLScanner`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScannerKind {
    /// `fContentScanner`.
    Content,
    /// `fSpecialScanner`.
    Special,
}

/// Basic scanner interface.
///
/// Equivalent to `HTMLScanner.Scanner`; a scanner of this port is one of
/// the two inner scanners ([`ScannerKind`]).
pub trait Scanner {
    /// Scans part of the document. This interface allows scanning to be
    /// performed in a pulling manner.
    ///
    /// Equivalent to `scan(boolean)`.
    ///
    /// # Errors
    ///
    /// I/O and XNI exceptions.
    fn scan(&self, complete: bool) -> JResult<bool>;
}

/// Location infoset item.
///
/// Equivalent to the nested class `HTMLScanner.LocationItem`; its fields
/// are cells because the scanner reuses one instance it has already handed
/// out in an `Augmentations`, as Java does.
#[derive(Debug, Default)]
pub struct LocationItem {
    begin_line_number: Cell<i32>,
    begin_column_number: Cell<i32>,
    begin_character_offset: Cell<i32>,
    end_line_number: Cell<i32>,
    end_column_number: Cell<i32>,
    end_character_offset: Cell<i32>,
}

impl LocationItem {
    /// Equivalent to `new LocationItem()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Equivalent to the package-private copy constructor
    /// `LocationItem(LocationItem)`.
    pub fn copy_of(other: &LocationItem) -> Self {
        let s = Self::new();
        s.set_values(
            other.begin_line_number.get(),
            other.begin_column_number.get(),
            other.begin_character_offset.get(),
            other.end_line_number.get(),
            other.end_column_number.get(),
            other.end_character_offset.get(),
        );
        s
    }

    /// Sets the values of this item.
    ///
    /// Equivalent to `setValues(int, int, int, int, int, int)`.
    pub fn set_values(&self, begin_line: i32, begin_column: i32, begin_offset: i32, end_line: i32, end_column: i32, end_offset: i32) {
        self.begin_line_number.set(begin_line);
        self.begin_column_number.set(begin_column);
        self.begin_character_offset.set(begin_offset);
        self.end_line_number.set(end_line);
        self.end_column_number.set(end_column);
        self.end_character_offset.set(end_offset);
    }

    /// Returns a string representation of this object.
    ///
    /// Equivalent to `toString()`.
    pub fn to_java_string(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}:{}",
            self.begin_line_number.get(),
            self.begin_column_number.get(),
            self.begin_character_offset.get(),
            self.end_line_number.get(),
            self.end_column_number.get(),
            self.end_character_offset.get()
        )
    }
}

impl HTMLEventInfo for LocationItem {
    fn get_begin_line_number(&self) -> i32 {
        self.begin_line_number.get()
    }

    fn get_begin_column_number(&self) -> i32 {
        self.begin_column_number.get()
    }

    fn get_begin_character_offset(&self) -> i32 {
        self.begin_character_offset.get()
    }

    fn get_end_line_number(&self) -> i32 {
        self.end_line_number.get()
    }

    fn get_end_column_number(&self) -> i32 {
        self.end_column_number.get()
    }

    fn get_end_character_offset(&self) -> i32 {
        self.end_character_offset.get()
    }

    fn is_synthesized(&self) -> bool {
        false
    }
}

/// `InputStreamReader.getEncoding()`: the historical name of the charset
/// (`HistoricallyNamedCharset.historicalName()`) when it has one, else its
/// canonical name.
///
/// **Divergence.** The JDK charsets carry their historical names; the
/// charset emulation the port uses does not, so the names of the charset
/// families NekoHTML deals with (`US-ASCII`, `UTF-8`, `UTF-16*`,
/// `ISO-8859-*`, `windows-125*`, `IBM*`, `Shift_JIS`, `EUC-*`) are mapped
/// as the JDK names them, and any other charset reports its canonical name.
fn historical_name(charset: &Charset) -> String {
    let name = charset.name();
    match name {
        "US-ASCII" => "ASCII".to_string(),
        "UTF-8" => "UTF8".to_string(),
        "UTF-16BE" => "UnicodeBigUnmarked".to_string(),
        "UTF-16LE" => "UnicodeLittleUnmarked".to_string(),
        "UTF-16" => "UTF-16".to_string(),
        "Shift_JIS" => "SJIS".to_string(),
        "EUC-JP" => "EUC_JP".to_string(),
        "EUC-KR" => "EUC_KR".to_string(),
        "KOI8-R" => "KOI8_R".to_string(),
        "GB2312" => "EUC_CN".to_string(),
        _ => {
            if let Some(n) = name.strip_prefix("ISO-8859-") {
                format!("ISO8859_{n}")
            } else if let Some(n) = name.strip_prefix("windows-") {
                format!("Cp{n}")
            } else if let Some(n) = name.strip_prefix("x-IBM") {
                format!("Cp{n}")
            } else if let Some(n) = name.strip_prefix("IBM") {
                format!("Cp{}", n.trim_start_matches('0'))
            } else {
                name.to_string()
            }
        }
    }
}

/// `Charset.forName` as `new InputStreamReader(InputStream, String)` uses
/// it: an unknown name is `UnsupportedEncodingException(charsetName)`, an
/// illegal one the `IllegalCharsetNameException` `Charset.isSupported`
/// throws.
fn charset_for_reader(name: &str) -> JResult<Charset> {
    match Charset::for_name(name) {
        Ok(c) => Ok(c),
        Err(e) if e.java_simple_name() == "IllegalCharsetNameException" => Err(Exception::from(e)),
        Err(_) => Err(io_exception(
            "java.io.UnsupportedEncodingException",
            Some(name.to_string()),
        )),
    }
}

/// The `java.io.InputStream` view of the shared playback stream, for an
/// `InputStreamReader` that owns its input.
struct PlaybackRead(Rc<RefCell<PlaybackInputStream>>);

impl Read for PlaybackRead {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let len = buf.len() as i32;
        match self.0.borrow_mut().read_into(buf, 0, len) {
            Ok(-1) => Ok(0),
            Ok(n) => Ok(n as usize),
            Err(e) => Err(std::io::Error::other(e.to_java_string())),
        }
    }
}

/// The `java.io.InputStream` view of a shared stream.
struct SharedRead(InputStream);

impl Read for SharedRead {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().read(buf)
    }
}

/// `new InputStreamReader(in, charsetName)` as a [`Reader`], with the name
/// `getEncoding()` reports.
fn input_stream_reader<R: Read + 'static>(input: R, charset_name: &str) -> JResult<(Reader, String)> {
    let charset = charset_for_reader(charset_name)?;
    let reader: Reader = Rc::new(RefCell::new(InputStreamReader::with_charset(input, charset)));
    Ok((reader, historical_name(&charset)))
}

/// Current entity.
///
/// Equivalent to the nested class `HTMLScanner.CurrentEntity`.
pub struct CurrentEntity {
    /// Character stream.
    stream: Reader,
    /// Encoding.
    encoding: Option<String>,
    /// Public identifier.
    pub public_id: Option<String>,
    /// Base system identifier.
    pub base_system_id: Option<String>,
    /// Literal system identifier.
    pub literal_system_id: Option<String>,
    /// Expanded system identifier.
    pub expanded_system_id: Option<String>,
    /// XML version.
    pub version: String,
    /// Line number.
    line_number: i32,
    /// Column number.
    column_number: i32,
    /// Character offset in the file.
    pub character_offset: i32,
    /// Character buffer.
    pub buffer: Vec<u16>,
    /// Offset into character buffer.
    pub offset: i32,
    /// Length of characters read into character buffer.
    pub length: i32,
    end_reached: bool,
}

impl std::fmt::Debug for CurrentEntity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CurrentEntity")
            .field("encoding", &self.encoding)
            .field("line_number", &self.line_number)
            .field("column_number", &self.column_number)
            .field("offset", &self.offset)
            .field("length", &self.length)
            .finish_non_exhaustive()
    }
}

impl CurrentEntity {
    /// Constructs an entity from the specified stream.
    ///
    /// Equivalent to `new CurrentEntity(Reader, String, String, String,
    /// String, String)`.
    pub fn new(
        stream: Reader,
        encoding: Option<&str>,
        public_id: Option<&str>,
        base_system_id: Option<&str>,
        literal_system_id: Option<&str>,
        expanded_system_id: Option<&str>,
    ) -> Self {
        Self {
            stream,
            encoding: encoding.map(str::to_string),
            public_id: public_id.map(str::to_string),
            base_system_id: base_system_id.map(str::to_string),
            literal_system_id: literal_system_id.map(str::to_string),
            expanded_system_id: expanded_system_id.map(str::to_string),
            version: "1.0".to_string(),
            line_number: 1,
            column_number: 1,
            character_offset: 0,
            buffer: vec![0; DEFAULT_BUFFER_SIZE as usize],
            offset: 0,
            length: 0,
            end_reached: false,
        }
    }

    fn get_current_char(&self) -> u16 {
        self.buffer[self.offset as usize]
    }

    /// Returns the next character and moves the current offset.
    fn get_next_char(&mut self) -> u16 {
        self.character_offset += 1;
        self.column_number += 1;
        let c = self.buffer[self.offset as usize];
        self.offset += 1;
        c
    }

    fn close_quietly(&mut self) {
        let _ = self.stream.borrow_mut().close_chars();
    }

    /// Indicates if there are characters left.
    fn has_next(&self) -> bool {
        self.offset < self.length
    }

    /// Loads a new chunk of data into the buffer and returns the number of
    /// characters loaded or -1 if no additional characters were loaded.
    ///
    /// Equivalent to `load(int)`.
    ///
    /// # Errors
    ///
    /// What the stream raises.
    pub fn load(&mut self, offset: i32) -> JResult<i32> {
        if DEBUG_BUFFER {
            self.debug_buffer_if_needed("(load: ", "");
        }
        // resize buffer, if needed
        if offset as usize == self.buffer.len() {
            let adjust = self.buffer.len() / 4;
            self.buffer.resize(self.buffer.len() + adjust, 0);
        }
        // read a block of characters
        let read = {
            let mut stream = self.stream.borrow_mut();
            stream.read_chars(&mut self.buffer[offset as usize..]).map_err(Exception::from)?
        };
        let count = if read == 0 { -1 } else { read as i32 };
        if count == -1 {
            self.end_reached = true;
        }
        self.length = if count != -1 { count + offset } else { offset };
        self.offset = offset;
        if DEBUG_BUFFER {
            self.debug_buffer_if_needed(")load: ", &format!(" -> {count}"));
        }
        Ok(count)
    }

    /// Reads a single character.
    ///
    /// Equivalent to `read()`.
    ///
    /// # Errors
    ///
    /// What the stream raises.
    pub fn read(&mut self) -> JResult<i32> {
        if DEBUG_BUFFER {
            self.debug_buffer_if_needed("(read: ", "");
        }
        if self.offset == self.length {
            if self.end_reached {
                return Ok(-1);
            }
            if self.load(0)? == -1 {
                if DEBUG_BUFFER {
                    println!(")read: -> -1");
                }
                return Ok(-1);
            }
        }
        let c = self.buffer[self.offset as usize];
        self.offset += 1;
        self.character_offset += 1;
        self.column_number += 1;
        if DEBUG_BUFFER {
            self.debug_buffer_if_needed(")read: ", &format!(" -> {}", from_chars(&[c])));
        }
        Ok(c as i32)
    }

    /// Prints the contents of the character buffer to standard out.
    fn debug_buffer_if_needed(&self, prefix: &str, suffix: &str) {
        if DEBUG_BUFFER {
            let mut out = String::new();
            out.push_str(prefix);
            out.push('[');
            out.push_str(&self.length.to_string());
            out.push(' ');
            out.push_str(&self.offset.to_string());
            if self.length > 0 {
                out.push_str(" \"");
                for i in 0..self.length {
                    if i == self.offset {
                        out.push('^');
                    }
                    match self.buffer[i as usize] {
                        0x0D => out.push_str("\\r"),
                        0x0A => out.push_str("\\n"),
                        0x09 => out.push_str("\\t"),
                        0x22 => out.push_str("\\\""),
                        c => out.push_str(&from_chars(&[c])),
                    }
                }
                if self.offset == self.length {
                    out.push('^');
                }
                out.push('"');
            }
            out.push(']');
            out.push_str(suffix);
            println!("{out}");
        }
    }

    fn set_stream(&mut self, stream: Reader, encoding: String) {
        self.stream = stream;
        self.offset = 0;
        self.length = 0;
        self.character_offset = 0;
        self.line_number = 1;
        self.column_number = 1;
        self.encoding = Some(encoding);
    }

    /// Goes back, cancelling the effect of the previous read() call.
    fn rewind(&mut self) {
        self.offset -= 1;
        self.character_offset -= 1;
        self.column_number -= 1;
    }

    fn rewind_by(&mut self, i: i32) {
        self.offset -= i;
        self.character_offset -= i;
        self.column_number -= i;
    }

    fn inc_line(&mut self) {
        self.line_number += 1;
        self.column_number = 1;
    }

    fn inc_lines(&mut self, nb_lines: i32) {
        self.line_number += nb_lines;
        self.column_number = 1;
    }

    /// Equivalent to `getLineNumber()`.
    pub fn get_line_number(&self) -> i32 {
        self.line_number
    }

    /// Equivalent to `resetBuffer(XMLStringBuffer, int, int, int)`; Java
    /// makes the entity read the string buffer's array, the port copies it.
    fn reset_buffer(&mut self, buffer: &XMLStringBuffer, line_number: i32, column_number: i32, character_offset: i32) {
        self.line_number = line_number;
        self.column_number = column_number;
        self.character_offset = character_offset;
        self.buffer = buffer.ch.clone().unwrap_or_default();
        self.offset = buffer.offset;
        self.length = buffer.length;
    }

    fn get_column_number(&self) -> i32 {
        self.column_number
    }

    fn restore_position(&mut self, original_offset: i32, original_column_number: i32, original_character_offset: i32) {
        self.offset = original_offset;
        self.column_number = original_column_number;
        self.character_offset = original_character_offset;
    }

    fn get_character_offset(&self) -> i32 {
        self.character_offset
    }
}

/// A playback input stream. This class has the ability to save the bytes
/// read from the underlying input stream and play the bytes back later.
/// This class is used by the HTML scanner to switch encodings when a
/// &lt;meta&gt; tag is detected that specifies a different encoding.
///
/// Equivalent to the nested class `HTMLScanner.PlaybackInputStream` (a
/// `FilterInputStream` over `in`).
pub struct PlaybackInputStream {
    /// The underlying input stream (`FilterInputStream.in`).
    input: InputStream,
    /// Playback mode.
    playback: bool,
    /// Buffer cleared.
    cleared: bool,
    /// Encoding detected.
    detected: bool,
    /// Byte buffer.
    byte_buffer: Option<Vec<u8>>,
    /// Offset into byte buffer during playback.
    byte_offset: i32,
    /// Length of bytes read into byte buffer.
    byte_length: i32,
    /// Pushback offset.
    pub pushback_offset: i32,
    /// Pushback length.
    pub pushback_length: i32,
}

/// Set to true to debug playback.
const DEBUG_PLAYBACK: bool = false;

impl PlaybackInputStream {
    /// Constructor.
    ///
    /// Equivalent to `new PlaybackInputStream(InputStream)`.
    pub fn new(input: InputStream) -> Self {
        Self {
            input,
            playback: false,
            cleared: false,
            detected: false,
            byte_buffer: Some(vec![0; 1024]),
            byte_offset: 0,
            byte_length: 0,
            pushback_offset: 0,
            pushback_length: 0,
        }
    }

    fn in_read(&mut self) -> JResult<i32> {
        let mut b = [0u8; 1];
        loop {
            match self.input.borrow_mut().read(&mut b) {
                Ok(0) => return Ok(-1),
                Ok(_) => return Ok(b[0] as i32),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(Exception::io(LuceneError::from(e))),
            }
        }
    }

    fn in_read_into(&mut self, array: &mut [u8], offset: i32, length: i32) -> JResult<i32> {
        if length == 0 {
            return Ok(0);
        }
        loop {
            match self.input.borrow_mut().read(&mut array[offset as usize..(offset + length) as usize]) {
                Ok(0) => return Ok(-1),
                Ok(n) => return Ok(n as i32),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(Exception::io(LuceneError::from(e))),
            }
        }
    }

    /// Detect encoding.
    ///
    /// Equivalent to `detectEncoding(String[])`.
    ///
    /// # Errors
    ///
    /// `IOException("Should not detect encoding twice.")`, and the stream's.
    pub fn detect_encoding(&mut self, encodings: &mut [Option<String>; 2]) -> JResult<()> {
        if self.detected {
            return Err(io_exception(
                "java.io.IOException",
                Some("Should not detect encoding twice.".to_string()),
            ));
        }
        self.detected = true;
        let b1 = self.read()?;
        if b1 == -1 {
            return Ok(());
        }
        let b2 = self.read()?;
        if b2 == -1 {
            self.pushback_length = 1;
            return Ok(());
        }
        // UTF-8 BOM: 0xEFBBBF
        if b1 == 0xEF && b2 == 0xBB {
            let b3 = self.read()?;
            if b3 == 0xBF {
                self.pushback_offset = 3;
                encodings[0] = Some("UTF-8".to_string());
                encodings[1] = Some("UTF8".to_string());
                return Ok(());
            }
            self.pushback_length = 3;
        }
        // UTF-16 LE BOM: 0xFFFE
        if b1 == 0xFF && b2 == 0xFE {
            encodings[0] = Some("UTF-16".to_string());
            encodings[1] = Some("UnicodeLittleUnmarked".to_string());
            return Ok(());
        }
        // UTF-16 BE BOM: 0xFEFF
        else if b1 == 0xFE && b2 == 0xFF {
            encodings[0] = Some("UTF-16".to_string());
            encodings[1] = Some("UnicodeBigUnmarked".to_string());
            return Ok(());
        }
        // unknown
        self.pushback_length = 2;
        Ok(())
    }

    /// Playback buffer contents.
    ///
    /// Equivalent to `playback()`.
    pub fn playback(&mut self) {
        self.playback = true;
    }

    /// Clears the buffer.
    ///
    /// Equivalent to `clear()`.
    pub fn clear(&mut self) {
        if !self.playback {
            self.cleared = true;
            self.byte_buffer = None;
        }
    }

    fn buffer(&self) -> &Vec<u8> {
        self.byte_buffer
            .as_ref()
            .expect("java.lang.NullPointerException: fByteBuffer")
    }

    /// Read a byte.
    ///
    /// Equivalent to `read()` (a pushed-back byte is returned sign-extended,
    /// as Java's `byte` to `int` conversion does).
    ///
    /// # Errors
    ///
    /// What the stream raises.
    pub fn read(&mut self) -> JResult<i32> {
        if DEBUG_PLAYBACK {
            println!("(read");
        }
        if self.pushback_offset < self.pushback_length {
            let b = self.buffer()[self.pushback_offset as usize];
            self.pushback_offset += 1;
            return Ok(b as i8 as i32);
        }
        if self.cleared {
            return self.in_read();
        }
        if self.playback {
            let c = self.buffer()[self.byte_offset as usize] as i8 as i32;
            self.byte_offset += 1;
            if self.byte_offset == self.byte_length {
                self.cleared = true;
                self.byte_buffer = None;
            }
            if DEBUG_PLAYBACK {
                println!(")read -> {c}");
            }
            return Ok(c);
        }
        let c = self.in_read()?;
        if c != -1 {
            let len = self.byte_length as usize;
            let buffer = self
                .byte_buffer
                .as_mut()
                .expect("java.lang.NullPointerException: fByteBuffer");
            if len == buffer.len() {
                buffer.resize(len + 1024, 0);
            }
            buffer[len] = c as u8;
            self.byte_length += 1;
        }
        if DEBUG_PLAYBACK {
            println!(")read -> {c}");
        }
        Ok(c)
    }

    /// Read an array of bytes.
    ///
    /// Equivalent to `read(byte[], int, int)`: -1 at the end of the stream.
    ///
    /// # Errors
    ///
    /// What the stream raises.
    pub fn read_into(&mut self, array: &mut [u8], offset: i32, mut length: i32) -> JResult<i32> {
        if DEBUG_PLAYBACK {
            println!(")read({offset},{length})");
        }
        if self.pushback_offset < self.pushback_length {
            let mut count = self.pushback_length - self.pushback_offset;
            if count > length {
                count = length;
            }
            let from = self.pushback_offset as usize;
            array[offset as usize..(offset + count) as usize]
                .copy_from_slice(&self.buffer()[from..from + count as usize]);
            self.pushback_offset += count;
            return Ok(count);
        }
        if self.cleared {
            return self.in_read_into(array, offset, length);
        }
        if self.playback {
            if self.byte_offset + length > self.byte_length {
                length = self.byte_length - self.byte_offset;
            }
            let from = self.byte_offset as usize;
            array[offset as usize..(offset + length) as usize]
                .copy_from_slice(&self.buffer()[from..from + length as usize]);
            self.byte_offset += length;
            if self.byte_offset == self.byte_length {
                self.cleared = true;
                self.byte_buffer = None;
            }
            return Ok(length);
        }
        let count = self.in_read_into(array, offset, length)?;
        if count != -1 {
            let len = self.byte_length as usize;
            let buffer = self
                .byte_buffer
                .as_mut()
                .expect("java.lang.NullPointerException: fByteBuffer");
            if len + count as usize > buffer.len() {
                buffer.resize(len + count as usize + 512, 0);
            }
            buffer[len..len + count as usize].copy_from_slice(&array[offset as usize..(offset + count) as usize]);
            self.byte_length += count;
        }
        if DEBUG_PLAYBACK {
            println!(")read({offset},{length}) -> {count}");
        }
        Ok(count)
    }
}

/// A simple HTML scanner. This scanner makes no attempt to balance tags or
/// fix other problems in the source document &mdash; it just scans what it
/// can and generates XNI document "events", ignoring errors of all kinds.
///
/// Equivalent to `org.cyberneko.html.HTMLScanner`. The scanner is shared as
/// an `Rc` (it is the `XMLLocator` it hands to the pipeline); its state is
/// in cells, and its two inner scanners (`ContentScanner`,
/// `SpecialScanner`) are its methods over their own fields, selected by
/// [`ScannerKind`].
pub struct HTMLScanner {
    self_ref: Weak<HTMLScanner>,

    // features
    /// Augmentations.
    augmentations: Cell<bool>,
    /// Report errors.
    report_errors: Cell<bool>,
    /// Notify character entity references.
    notify_char_refs: Cell<bool>,
    /// Notify XML built-in general entity references.
    notify_xml_builtin_refs: Cell<bool>,
    /// Notify HTML built-in general entity references.
    notify_html_builtin_refs: Cell<bool>,
    /// Fix Microsoft Windows&reg; character entity references.
    fix_windows_char_refs: Cell<bool>,
    /// Strip CDATA delimiters from SCRIPT tags.
    script_strip_cdata_delims: Cell<bool>,
    /// Strip comment delimiters from SCRIPT tags.
    script_strip_comment_delims: Cell<bool>,
    /// Strip CDATA delimiters from STYLE tags.
    style_strip_cdata_delims: Cell<bool>,
    /// Strip comment delimiters from STYLE tags.
    style_strip_comment_delims: Cell<bool>,
    /// Ignore specified character set.
    ignore_specified_charset: Cell<bool>,
    /// CDATA sections.
    cdata_sections: Cell<bool>,
    /// Override doctype declaration public and system identifiers.
    override_doctype: Cell<bool>,
    /// Insert document type declaration.
    insert_doctype: Cell<bool>,
    /// Normalize attribute values.
    normalize_attributes: Cell<bool>,
    /// Parse noscript content.
    parse_no_script_content: Cell<bool>,
    /// Parse noframes content.
    parse_no_frames_content: Cell<bool>,
    /// Allows self closing iframe tags.
    allow_selfclosing_iframe: Cell<bool>,
    /// Allows self closing tags.
    allow_selfclosing_tags: Cell<bool>,

    // properties
    /// Modify HTML element names.
    names_elems: Cell<i16>,
    /// Modify HTML attribute names.
    names_attrs: Cell<i16>,
    /// Default encoding.
    default_iana_encoding: RefCell<String>,
    /// Error reporter.
    error_reporter: RefCell<Option<Rc<dyn HTMLErrorReporter>>>,
    /// Doctype declaration public identifier.
    doctype_pubid: RefCell<String>,
    /// Doctype declaration system identifier.
    doctype_sysid: RefCell<String>,

    // boundary locator information
    /// Beginning line number.
    begin_line_number: Cell<i32>,
    /// Beginning column number.
    begin_column_number: Cell<i32>,
    /// Beginning character offset in the file.
    begin_character_offset: Cell<i32>,
    /// Ending line number.
    end_line_number: Cell<i32>,
    /// Ending column number.
    end_column_number: Cell<i32>,
    /// Ending character offset in the file.
    end_character_offset: Cell<i32>,

    // state
    /// The playback byte stream.
    byte_stream: RefCell<Option<Rc<RefCell<PlaybackInputStream>>>>,
    /// Current entity.
    current_entity: RefCell<Option<CurrentEntity>>,
    /// The current entity stack.
    current_entity_stack: RefCell<Vec<Option<CurrentEntity>>>,
    /// The current scanner.
    scanner: Cell<ScannerKind>,
    /// The current scanner state.
    scanner_state: Cell<i16>,
    /// The document handler.
    document_handler: RefCell<Option<Rc<dyn XMLDocumentHandler>>>,
    /// Auto-detected IANA encoding.
    iana_encoding: RefCell<Option<String>>,
    /// Auto-detected Java encoding.
    java_encoding: RefCell<Option<String>>,
    /// True if the encoding matches "ISO-8859-*".
    iso8859_encoding: Cell<bool>,
    /// Element count.
    element_count: Cell<i32>,
    /// Element depth.
    element_depth: Cell<i32>,

    // ContentScanner fields
    /// `ContentScanner.fQName`.
    content_qname: Slot<QName>,
    /// `ContentScanner.fAttributes`.
    content_attributes: Slot<XMLAttributesImpl>,

    // SpecialScanner fields
    /// `SpecialScanner.fElementName`.
    special_element_name: RefCell<String>,
    /// `SpecialScanner.fStyle`.
    special_style: Cell<bool>,
    /// `SpecialScanner.fTextarea`.
    special_textarea: Cell<bool>,
    /// `SpecialScanner.fTitle`.
    special_title: Cell<bool>,
    /// `SpecialScanner.fQName`.
    special_qname: Slot<QName>,
    /// `SpecialScanner.fStringBuffer`.
    special_string_buffer: Slot<XMLStringBuffer>,

    // temp vars
    /// String buffer.
    string_buffer: Slot<XMLStringBuffer>,
    /// String buffer used when resolving entity refs.
    string_buffer2: Slot<XMLStringBuffer>,
    /// Non-normalized attribute string buffer.
    non_norm_attr: Slot<XMLStringBuffer>,
    /// Augmentations.
    infoset_augs: HTMLAugmentations,
    /// Location infoset item.
    location_item: Rc<LocationItem>,
    /// Single boolean array.
    single_boolean: Cell<bool>,
    /// Resource identifier.
    resource_id: RefCell<XMLResourceIdentifierImpl>,
}

impl std::fmt::Debug for HTMLScanner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HTMLScanner")
            .field("scanner", &self.scanner.get())
            .field("scanner_state", &self.scanner_state.get())
            .finish_non_exhaustive()
    }
}

/// Modifies the given name based on the specified mode.
///
/// Equivalent to the protected static `modifyName(String, short)`.
pub fn modify_name(name: &str, mode: i16) -> String {
    match mode {
        NAMES_UPPERCASE => to_upper_case(name),
        NAMES_LOWERCASE => to_lower_case(name),
        _ => name.to_string(),
    }
}

/// Converts HTML names string value to constant value.
///
/// Equivalent to the protected static `getNamesValue(String)`.
pub fn get_names_value(value: &str) -> i16 {
    if value == "lower" {
        return NAMES_LOWERCASE;
    }
    if value == "upper" {
        return NAMES_UPPERCASE;
    }
    NAMES_NO_CHANGE
}

/// Returns the value of the specified attribute, ignoring case.
///
/// Equivalent to the protected static `getValue(XMLAttributes, String)`.
pub fn get_value(attrs: Option<&dyn XMLAttributes>, aname: &str) -> Option<String> {
    let attrs = attrs?;
    for i in 0..attrs.get_length() {
        if equals_ignore_case(&attrs.get_qname(i).unwrap_or_default(), aname) {
            return attrs.get_value(i);
        }
    }
    None
}

/// Expands a system id and returns the system id as a URI, if it can be
/// expanded. A return value of null means that the identifier is already
/// expanded. An exception thrown indicates a failure to expand the id.
///
/// Equivalent to the static `expandSystemId(String, String)`.
pub fn expand_system_id(system_id: Option<&str>, base_system_id: Option<&str>) -> Option<String> {
    // check for bad parameters id
    let system_id = match system_id {
        None | Some("") => return system_id.map(str::to_string),
        Some(s) => s,
    };
    // if id already expanded, return
    if URI::parse(Some(system_id)).is_ok() {
        return Some(system_id.to_string());
    }
    // normalize id
    let id = fix_uri(system_id);

    // normalize base
    let user_dir = || {
        std::env::current_dir()
            .map(|d| fix_uri(&d.to_string_lossy()))
            .unwrap_or_default()
    };
    let uri = (|| -> Result<URI, crate::org::apache::xerces::util::uri::MalformedURIException> {
        let base = match base_system_id {
            None | Some("") => {
                let mut dir = user_dir();
                if !dir.ends_with('/') {
                    dir.push('/');
                }
                URI::with_host_parts(Some("file"), Some(""), Some(&dir), None, None)?
            }
            Some(b) if b == system_id => {
                let mut dir = user_dir();
                if !dir.ends_with('/') {
                    dir.push('/');
                }
                URI::with_host_parts(Some("file"), Some(""), Some(&dir), None, None)?
            }
            Some(base_system_id) => match URI::parse(Some(&fix_uri(base_system_id))) {
                Ok(base) => base,
                Err(_) => {
                    let mut dir = user_dir();
                    if base_system_id.contains(':') {
                        // for xml schemas we might have baseURI with
                        // a specified drive
                        URI::with_host_parts(Some("file"), Some(""), Some(&fix_uri(base_system_id)), None, None)?
                    } else {
                        if !dir.ends_with('/') {
                            dir.push('/');
                        }
                        dir.push_str(&fix_uri(base_system_id));
                        URI::with_host_parts(Some("file"), Some(""), Some(&dir), None, None)?
                    }
                }
            },
        };
        // expand id
        URI::with_base(Some(&base), Some(&id))
    })();
    match uri {
        // let it go through
        Err(_) => Some(system_id.to_string()),
        Ok(uri) => Some(uri.to_java_string()),
    }
}

/// Fixes Windows file paths.
///
/// Equivalent to the protected static `fixURI(String)`.
pub fn fix_uri(str: &str) -> String {
    // handle platform dependent strings
    let mut str = str.replace(std::path::MAIN_SEPARATOR, "/");

    // Windows fix
    let units = to_chars(&str);
    if units.len() >= 2 {
        let ch1 = units[1];
        // change "C:blah" to "/C:blah"
        if ch1 == b':' as u16 {
            let ch0 = char_upper(units[0]);
            if (b'A' as u16..=b'Z' as u16).contains(&ch0) {
                str = format!("/{str}");
            }
        }
        // change "//blah" to "file://blah"
        else if ch1 == b'/' as u16 && units[0] == b'/' as u16 {
            str = format!("file:{str}");
        }
    }

    // done
    str
}

/// Returns true if the name is a built-in XML general entity reference.
///
/// Equivalent to the protected static `builtinXmlRef(String)`.
pub fn builtin_xml_ref(name: &str) -> bool {
    matches!(name, "amp" | "lt" | "gt" | "quot" | "apos")
}

/// `java.net.URL(spec).openStream()`.
///
/// **Divergence.** Java opens any URL protocol the platform supports
/// (`file`, `http`, `https`, `jar`, ...). The port opens `file:` URLs (the
/// path, percent-decoded as `URLDecoder` does for `File` URLs) and throws
/// `java.net.MalformedURLException` ("unknown protocol: <p>" or "no
/// protocol: <spec>") for any other, as a JVM without that protocol
/// handler does.
fn open_url_stream(spec: Option<&str>) -> JResult<InputStream> {
    let spec = spec.ok_or_else(npe)?;
    let Some(colon) = spec.find(':') else {
        return Err(io_exception(
            "java.net.MalformedURLException",
            Some(format!("no protocol: {spec}")),
        ));
    };
    let protocol = spec[..colon].to_lowercase();
    if protocol != "file" {
        return Err(io_exception(
            "java.net.MalformedURLException",
            Some(format!("unknown protocol: {protocol}")),
        ));
    }
    let mut path = &spec[colon + 1..];
    if let Some(rest) = path.strip_prefix("//") {
        // authority: only the local host is a file
        path = match rest.find('/') {
            Some(i) => &rest[i..],
            None => "",
        };
    }
    let path = percent_decode(path.split(['?', '#']).next().unwrap_or(""));
    match std::fs::File::open(&path) {
        Ok(f) => Ok(Rc::new(RefCell::new(f)) as InputStream),
        Err(e) => Err(io_exception(
            "java.io.FileNotFoundException",
            Some(format!("{path} ({})", e)),
        )),
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() + 0 && i + 2 <= bytes.len() - 1 {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `Integer.parseInt(s, radix)`; `None` where Java throws
/// `NumberFormatException`.
fn parse_int_radix(s: &str, radix: u32) -> Option<i32> {
    if radix == 10 {
        return crate::java::parse_int(s);
    }
    let units: Vec<u16> = s.encode_utf16().collect();
    if units.is_empty() {
        return None;
    }
    let (negative, start) = match units[0] {
        0x2D => (true, 1),
        0x2B => (false, 1),
        _ => (false, 0),
    };
    if start == units.len() {
        return None;
    }
    let mut result: i64 = 0;
    for &u in &units[start..] {
        let d = char::from_u32(u as u32).and_then(|c| c.to_digit(radix))? as i64;
        result = result * radix as i64 + d;
        if result > i32::MAX as i64 + 1 {
            return None;
        }
    }
    let result = if negative { -result } else { result };
    i32::try_from(result).ok()
}

/// Encodes the ASCII `reference` in the charset `name`:
/// `String.getBytes(String)` of an ASCII string.
///
/// **Divergence.** The JDK charset emulation the port uses decodes every
/// JDK charset but encodes only the standard ones. The bytes of each ASCII
/// character are found from the charset's decoder: the shortest sequence
/// (one, two, then four bytes, low values first) the decoder maps to
/// exactly that character; charsets whose encoder writes a byte order mark
/// first (`UTF-16`, `UTF-32`) get it, big-endian, as the JDK encoders do.
fn get_bytes_ascii(reference: &str, name: &str) -> JResult<Vec<u8>> {
    let charset = charset_for_reader(name)?;
    let decode = |bytes: &[u8]| -> Vec<u16> {
        charset
            .new_decoder()
            .map(|d| d.decode_replacing(bytes))
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    match charset.name() {
        "UTF-16" => out.extend_from_slice(&[0xFE, 0xFF]),
        "UTF-32" => {}
        _ => {}
    }
    'chars: for c in reference.encode_utf16() {
        for b in 0..=255u8 {
            if decode(&[b]) == [c] {
                out.push(b);
                continue 'chars;
            }
        }
        for seq in [[0u8, c as u8], [c as u8, 0u8]] {
            if decode(&seq) == [c] {
                out.extend_from_slice(&seq);
                continue 'chars;
            }
        }
        for seq in [[0u8, 0, 0, c as u8], [c as u8, 0, 0, 0]] {
            if decode(&seq) == [c] {
                out.extend_from_slice(&seq);
                continue 'chars;
            }
        }
        // unmappable: the encoder's replacement, '?'
        out.push(b'?');
    }
    Ok(out)
}

/// Ensures that the encoding switch is possible: the reference text encoded
/// with the first encoding reads the same with the second one.
///
/// Equivalent to the package-private `isEncodingCompatible(String,
/// String)`.
pub(crate) fn is_encoding_compatible(encoding1: &str, encoding2: Option<&str>) -> bool {
    let reference = "<html><head><meta http-equiv=\"Content-Type\" content=\"text/html;charset=";
    let Ok(bytes_encoding1) = get_bytes_ascii(reference, encoding1) else {
        return false;
    };
    let Some(encoding2) = encoding2 else {
        return false;
    };
    let Ok(charset2) = charset_for_reader(encoding2) else {
        return false;
    };
    let reference_with_encoding2 = charset2
        .new_decoder()
        .map(|d| d.decode_replacing(&bytes_encoding1))
        .unwrap_or_default();
    to_chars(reference) == reference_with_encoding2
}

/// Indicates if the end of the buffer's content equals `string`.
fn ends_with(buffer: &XMLStringBuffer, string: &str) -> bool {
    let s = to_chars(string);
    let l = s.len() as i32;
    if buffer.length < l {
        return false;
    }
    let ch = buffer.ch.as_deref().unwrap_or(&[]);
    ch[(buffer.length - l) as usize..buffer.length as usize] == s[..]
}

/// Reduces the buffer to the content between start and end marker when
/// only whitespaces are found before the startMarker as well as after the
/// end marker.
///
/// Equivalent to the package-private static `reduceToContent(
/// XMLStringBuffer, String, String)`.
pub(crate) fn reduce_to_content(buffer: &mut XMLStringBuffer, start_marker: &str, end_marker: &str) {
    let start = to_chars(start_marker);
    let end = to_chars(end_marker);
    let mut i = 0;
    let mut start_content = -1;
    let l1 = start.len() as i32;
    let l2 = end.len() as i32;
    {
        let ch = buffer.ch.as_deref().unwrap_or(&[]);
        while i < buffer.length - l1 - l2 {
            let c = ch[(buffer.offset + i) as usize];
            if is_java_whitespace(c) {
                i += 1;
            } else if c == start[0] && ch[(buffer.offset + i) as usize..(buffer.offset + i + l1) as usize] == start[..] {
                start_content = buffer.offset + i + l1;
                break;
            } else {
                return; // start marker not found
            }
        }
    }
    if start_content == -1 {
        // start marker not found
        return;
    }

    i = buffer.length - 1;
    while i > start_content + l2 {
        let c = buffer.ch.as_deref().unwrap_or(&[])[(buffer.offset + i) as usize];
        if is_java_whitespace(c) {
            i -= 1;
        } else if c == end[(l2 - 1) as usize]
            && buffer.ch.as_deref().unwrap_or(&[])
                [(buffer.offset + i - l2 + 1) as usize..(buffer.offset + i + 1) as usize]
                == end[..]
        {
            buffer.length = buffer.offset + i - start_content - 2;
            buffer.offset = start_content;
            return;
        } else {
            return; // start marker not found
        }
    }
}

impl Default for HTMLScanner {
    fn default() -> Self {
        Self::fields(Weak::new())
    }
}

impl HTMLScanner {
    /// Equivalent to `new HTMLScanner()`.
    pub fn new() -> Rc<Self> {
        Rc::new_cyclic(|weak| Self::fields(weak.clone()))
    }

    fn fields(self_ref: Weak<HTMLScanner>) -> Self {
        Self {
            self_ref,
            augmentations: Cell::new(false),
            report_errors: Cell::new(false),
            notify_char_refs: Cell::new(false),
            notify_xml_builtin_refs: Cell::new(false),
            notify_html_builtin_refs: Cell::new(false),
            fix_windows_char_refs: Cell::new(false),
            script_strip_cdata_delims: Cell::new(false),
            script_strip_comment_delims: Cell::new(false),
            style_strip_cdata_delims: Cell::new(false),
            style_strip_comment_delims: Cell::new(false),
            ignore_specified_charset: Cell::new(false),
            cdata_sections: Cell::new(false),
            override_doctype: Cell::new(false),
            insert_doctype: Cell::new(false),
            normalize_attributes: Cell::new(false),
            parse_no_script_content: Cell::new(false),
            parse_no_frames_content: Cell::new(false),
            allow_selfclosing_iframe: Cell::new(false),
            allow_selfclosing_tags: Cell::new(false),
            names_elems: Cell::new(NAMES_NO_CHANGE),
            names_attrs: Cell::new(NAMES_NO_CHANGE),
            default_iana_encoding: RefCell::new(String::new()),
            error_reporter: RefCell::new(None),
            doctype_pubid: RefCell::new(String::new()),
            doctype_sysid: RefCell::new(String::new()),
            begin_line_number: Cell::new(0),
            begin_column_number: Cell::new(0),
            begin_character_offset: Cell::new(0),
            end_line_number: Cell::new(0),
            end_column_number: Cell::new(0),
            end_character_offset: Cell::new(0),
            byte_stream: RefCell::new(None),
            current_entity: RefCell::new(None),
            current_entity_stack: RefCell::new(Vec::new()),
            scanner: Cell::new(ScannerKind::Content),
            scanner_state: Cell::new(STATE_CONTENT),
            document_handler: RefCell::new(None),
            iana_encoding: RefCell::new(None),
            java_encoding: RefCell::new(None),
            iso8859_encoding: Cell::new(false),
            element_count: Cell::new(0),
            element_depth: Cell::new(0),
            content_qname: Slot::new(QName::new()),
            content_attributes: Slot::new(XMLAttributesImpl::new()),
            special_element_name: RefCell::new(String::new()),
            special_style: Cell::new(false),
            special_textarea: Cell::new(false),
            special_title: Cell::new(false),
            special_qname: Slot::new(QName::new()),
            special_string_buffer: Slot::new(XMLStringBuffer::new()),
            string_buffer: Slot::new(XMLStringBuffer::with_size(1024)),
            string_buffer2: Slot::new(XMLStringBuffer::with_size(1024)),
            non_norm_attr: Slot::new(XMLStringBuffer::with_size(128)),
            infoset_augs: HTMLAugmentations::new(),
            location_item: Rc::new(LocationItem::new()),
            single_boolean: Cell::new(false),
            resource_id: RefCell::new(XMLResourceIdentifierImpl::new()),
        }
    }

    /// The current entity (`fCurrentEntity`), borrowed briefly.
    ///
    /// # Panics
    ///
    /// Without a current entity, where Java throws `NullPointerException`.
    fn ce(&self) -> RefMut<'_, CurrentEntity> {
        RefMut::map(self.current_entity.borrow_mut(), |e| {
            e.as_mut().expect("java.lang.NullPointerException: fCurrentEntity")
        })
    }

    fn handler(&self) -> Option<Rc<dyn XMLDocumentHandler>> {
        self.document_handler.borrow().clone()
    }

    /// `fElementCount >= fElementDepth`.
    fn emitting(&self) -> bool {
        self.element_count.get() >= self.element_depth.get()
    }

    /// Copies the current entity's position into the ending locator fields.
    fn set_end_location(&self) {
        let ce = self.ce();
        self.end_line_number.set(ce.get_line_number());
        self.end_column_number.set(ce.get_column_number());
        self.end_character_offset.set(ce.get_character_offset());
    }

    /// Copies the current entity's position into the beginning locator
    /// fields.
    fn set_begin_location(&self) {
        let ce = self.ce();
        self.begin_line_number.set(ce.get_line_number());
        self.begin_column_number.set(ce.get_column_number());
        self.begin_character_offset.set(ce.get_character_offset());
    }

    fn reporter(&self) -> JResult<Rc<dyn HTMLErrorReporter>> {
        self.error_reporter.borrow().clone().ok_or_else(npe)
    }

    fn report_error(&self, key: &str, args: Option<&[JavaObject]>) -> JResult<()> {
        self.reporter()?.report_error(key, args)
    }

    fn report_warning(&self, key: &str, args: Option<&[JavaObject]>) -> JResult<()> {
        self.reporter()?.report_warning(key, args)
    }

    /// This scanner as the `Rc` the pipeline holds.
    fn this(&self) -> Option<Rc<HTMLScanner>> {
        self.self_ref.upgrade()
    }

    //
    // Public methods
    //

    /// Pushes an input source onto the current entity stack. This enables
    /// the scanner to transparently scan new content (e.g. the output
    /// written by an embedded script). At the end of the current entity,
    /// the scanner returns where it left off at the time this entity
    /// source was pushed.
    ///
    /// Equivalent to `pushInputSource(XMLInputSource)`.
    ///
    /// # Errors
    ///
    /// The charset errors of a byte stream's reader.
    pub fn push_input_source(&self, input_source: &XMLInputSource) -> JResult<()> {
        let reader = self.get_reader(input_source)?;

        let previous = self.current_entity.borrow_mut().take();
        self.current_entity_stack.borrow_mut().push(previous);
        let encoding = input_source.get_encoding();
        let public_id = input_source.get_public_id();
        let base_system_id = input_source.get_base_system_id();
        let literal_system_id = input_source.get_system_id();
        let expanded_system_id = expand_system_id(literal_system_id, base_system_id);
        *self.current_entity.borrow_mut() = Some(CurrentEntity::new(
            reader.ok_or_else(npe)?,
            encoding,
            public_id,
            base_system_id,
            literal_system_id,
            expanded_system_id.as_deref(),
        ));
        Ok(())
    }

    fn get_reader(&self, input_source: &XMLInputSource) -> JResult<Option<Reader>> {
        let reader = input_source.get_character_stream();
        if reader.is_none() {
            let byte_stream = input_source.get_byte_stream().ok_or_else(npe)?;
            let java_encoding = self.java_encoding.borrow().clone().ok_or_else(npe)?;
            return match input_stream_reader(SharedRead(byte_stream), &java_encoding) {
                Ok((r, _)) => Ok(Some(r)),
                // should not happen as this encoding is already used to
                // parse the "main" source
                Err(e) if matches!(&e, Exception::Io(io) if io.java_simple_name() == "UnsupportedEncodingException") => {
                    Ok(None)
                }
                Err(e) => Err(e),
            };
        }
        Ok(reader)
    }

    /// Immediately evaluates an input source and add the new content (e.g.
    /// the output written by an embedded script).
    ///
    /// Equivalent to `evaluateInputSource(XMLInputSource)`.
    ///
    /// # Errors
    ///
    /// The XNI exceptions of the handlers (Java ignores the
    /// `IOException`s).
    pub fn evaluate_input_source(&self, input_source: &XMLInputSource) -> JResult<()> {
        let previous_scanner = self.scanner.get();
        let previous_scanner_state = self.scanner_state.get();
        let reader = self.get_reader(input_source)?;

        let encoding = input_source.get_encoding();
        let public_id = input_source.get_public_id();
        let base_system_id = input_source.get_base_system_id();
        let literal_system_id = input_source.get_system_id();
        let expanded_system_id = expand_system_id(literal_system_id, base_system_id);
        let previous_entity = self.current_entity.borrow_mut().replace(CurrentEntity::new(
            reader.ok_or_else(npe)?,
            encoding,
            public_id,
            base_system_id,
            literal_system_id,
            expanded_system_id.as_deref(),
        ));
        self.set_scanner(ScannerKind::Content);
        self.set_scanner_state(STATE_CONTENT);
        let result = (|| -> JResult<()> {
            loop {
                self.scan_with(self.scanner.get(), false)?;
                if self.scanner_state.get() == STATE_END_DOCUMENT {
                    break;
                }
            }
            Ok(())
        })();
        match result {
            Ok(()) => {}
            // ignore
            Err(Exception::Io(_)) => {}
            Err(e) => {
                *self.current_entity.borrow_mut() = previous_entity;
                return Err(e);
            }
        }
        self.set_scanner(previous_scanner);
        self.set_scanner_state(previous_scanner_state);
        *self.current_entity.borrow_mut() = previous_entity;
        Ok(())
    }

    /// Cleans up used resources. For example, if scanning is terminated
    /// early, then this method ensures all remaining open streams are
    /// closed.
    ///
    /// Equivalent to `cleanup(boolean)`.
    pub fn cleanup(&self, closeall: bool) {
        let size = self.current_entity_stack.borrow().len();
        if size > 0 {
            // current entity is not the original, so close it
            if let Some(ce) = self.current_entity.borrow_mut().as_mut() {
                ce.close_quietly();
            }
            // close remaining streams
            let from = if closeall { 0 } else { 1 };
            for _ in from..size {
                let popped = self.current_entity_stack.borrow_mut().pop().flatten();
                *self.current_entity.borrow_mut() = popped;
                if let Some(ce) = self.current_entity.borrow_mut().as_mut() {
                    ce.close_quietly();
                }
            }
        } else if closeall {
            if let Some(ce) = self.current_entity.borrow_mut().as_mut() {
                ce.close_quietly();
            }
        }
    }

    /// Sets the input source.
    ///
    /// Equivalent to `setInputSource(XMLInputSource)`.
    ///
    /// # Errors
    ///
    /// The I/O errors of opening and reading the source.
    pub fn set_input_source(&self, source: &XMLInputSource) -> JResult<()> {
        // reset state
        self.element_count.set(0);
        self.element_depth.set(-1);
        *self.byte_stream.borrow_mut() = None;
        self.current_entity_stack.borrow_mut().clear();

        self.begin_line_number.set(1);
        self.begin_column_number.set(1);
        self.begin_character_offset.set(0);
        self.end_line_number.set(self.begin_line_number.get());
        self.end_column_number.set(self.begin_column_number.get());
        self.end_character_offset.set(self.begin_character_offset.get());

        // reset encoding information
        let default_iana = self.default_iana_encoding.borrow().clone();
        *self.iana_encoding.borrow_mut() = Some(default_iana.clone());
        *self.java_encoding.borrow_mut() = Some(default_iana.clone());

        // get location information
        let mut encoding = source.get_encoding().map(str::to_string);
        let public_id = source.get_public_id();
        let base_system_id = source.get_base_system_id();
        let literal_system_id = source.get_system_id();
        let expanded_system_id = expand_system_id(literal_system_id, base_system_id);

        // open stream
        let mut reader = source.get_character_stream();
        if reader.is_none() {
            let input_stream = match source.get_byte_stream() {
                Some(s) => s,
                None => open_url_stream(expanded_system_id.as_deref())?,
            };
            let byte_stream = Rc::new(RefCell::new(PlaybackInputStream::new(input_stream)));
            *self.byte_stream.borrow_mut() = Some(Rc::clone(&byte_stream));
            let mut encodings: [Option<String>; 2] = [None, None];
            if encoding.is_none() {
                byte_stream.borrow_mut().detect_encoding(&mut encodings)?;
            } else {
                encodings[0] = encoding.clone();
            }
            if encodings[0].is_none() {
                encodings[0] = Some(default_iana.clone());
                if self.report_errors.get() {
                    self.report_warning("HTML1000", None)?;
                }
            }
            if encodings[1].is_none() {
                let iana = encodings[0].clone().unwrap_or_default();
                encodings[1] = EncodingMap::get_iana2java_mapping(&to_upper_case(&iana));
                if encodings[1].is_none() {
                    encodings[1] = Some(iana.clone());
                    if self.report_errors.get() {
                        self.report_warning("HTML1001", Some(&[JavaObject::String(iana)]))?;
                    }
                }
            }
            *self.iana_encoding.borrow_mut() = encodings[0].clone();
            *self.java_encoding.borrow_mut() = encodings[1].clone();
            /* PATCH: Asgeir Asgeirsson */
            let iana = encodings[0].clone().unwrap_or_default();
            self.iso8859_encoding.set(
                to_upper_case(&iana).starts_with("ISO-8859") || equals_ignore_case(&iana, &default_iana),
            );
            encoding = Some(iana);
            let java_encoding = encodings[1].clone().unwrap_or_default();
            let (r, _) = input_stream_reader(PlaybackRead(byte_stream), &java_encoding)?;
            reader = Some(r);
        }
        *self.current_entity.borrow_mut() = Some(CurrentEntity::new(
            reader.ok_or_else(npe)?,
            encoding.as_deref(),
            public_id,
            base_system_id,
            literal_system_id,
            expanded_system_id.as_deref(),
        ));

        // set scanner and state
        self.set_scanner(ScannerKind::Content);
        self.set_scanner_state(STATE_START_DOCUMENT);
        Ok(())
    }

    /// Scans the document.
    ///
    /// Equivalent to `scanDocument(boolean)`.
    ///
    /// # Errors
    ///
    /// The I/O and XNI exceptions of the scan.
    pub fn scan_document(&self, complete: bool) -> JResult<bool> {
        loop {
            if !self.scan_with(self.scanner.get(), complete)? {
                return Ok(false);
            }
            if !complete {
                break;
            }
        }
        Ok(true)
    }

    /// Sets the document handler.
    ///
    /// Equivalent to `setDocumentHandler(XMLDocumentHandler)`.
    pub fn set_document_handler(&self, handler: Option<Rc<dyn XMLDocumentHandler>>) {
        *self.document_handler.borrow_mut() = handler;
    }

    /// Returns the document handler.
    ///
    /// Equivalent to `getDocumentHandler()`.
    pub fn get_document_handler(&self) -> Option<Rc<dyn XMLDocumentHandler>> {
        self.document_handler.borrow().clone()
    }

    //
    // Protected methods
    //

    /// Fixes Microsoft Windows&reg; specific characters.
    ///
    /// Equivalent to the protected `fixWindowsCharacter(int)`.
    pub fn fix_windows_character(&self, orig_char: i32) -> i32 {
        /* PATCH: Asgeir Asgeirsson */
        match orig_char {
            130 => 8218,
            131 => 402,
            132 => 8222,
            133 => 8230,
            134 => 8224,
            135 => 8225,
            136 => 710,
            137 => 8240,
            138 => 352,
            139 => 8249,
            140 => 338,
            145 => 8216,
            146 => 8217,
            147 => 8220,
            148 => 8221,
            149 => 8226,
            150 => 8211,
            151 => 8212,
            152 => 732,
            153 => 8482,
            154 => 353,
            155 => 8250,
            156 => 339,
            159 => 376,
            _ => orig_char,
        }
    }

    /// Reads a single character, preserving the old buffer content.
    ///
    /// Equivalent to the protected `read()`.
    fn read(&self) -> JResult<i32> {
        self.ce().read()
    }

    /// Sets the scanner.
    ///
    /// Equivalent to the protected `setScanner(Scanner)`.
    fn set_scanner(&self, scanner: ScannerKind) {
        self.scanner.set(scanner);
        if DEBUG_SCANNER {
            println!(
                "$$$ setScanner({});",
                match scanner {
                    ScannerKind::Content => "org.cyberneko.html.HTMLScanner$ContentScanner",
                    ScannerKind::Special => "org.cyberneko.html.HTMLScanner$SpecialScanner",
                }
            );
        }
    }

    /// Sets the scanner state.
    ///
    /// Equivalent to the protected `setScannerState(short)`.
    fn set_scanner_state(&self, state: i16) {
        self.scanner_state.set(state);
        if DEBUG_SCANNER_STATE {
            let name = match state {
                STATE_CONTENT => "STATE_CONTENT",
                STATE_MARKUP_BRACKET => "STATE_MARKUP_BRACKET",
                STATE_START_DOCUMENT => "STATE_START_DOCUMENT",
                STATE_END_DOCUMENT => "STATE_END_DOCUMENT",
                _ => "",
            };
            println!("$$$ setScannerState({name});");
        }
    }

    /// Scans a DOCTYPE line.
    ///
    /// Equivalent to the protected `scanDoctype()`.
    fn scan_doctype(&self) -> JResult<()> {
        let mut root = None;
        let mut pubid = None;
        let mut sysid = None;

        if self.skip_spaces()? {
            root = self.scan_name()?;
            if root.is_none() {
                if self.report_errors.get() {
                    self.report_error("HTML1014", None)?;
                }
            } else {
                root = Some(modify_name(root.as_deref().unwrap_or(""), self.names_elems.get()));
            }
            if self.skip_spaces()? {
                if self.skip("PUBLIC", false)? {
                    self.skip_spaces()?;
                    pubid = self.scan_literal()?;
                    if self.skip_spaces()? {
                        sysid = self.scan_literal()?;
                    }
                } else if self.skip("SYSTEM", false)? {
                    self.skip_spaces()?;
                    sysid = self.scan_literal()?;
                }
            }
        }
        loop {
            let c = self.read()?;
            if c == -1 {
                break;
            }
            if c == b'<' as i32 {
                self.ce().rewind();
                break;
            }
            if c == b'>' as i32 {
                break;
            }
            if c == b'[' as i32 {
                self.skip_markup(true)?;
                break;
            }
        }

        if let Some(h) = self.handler() {
            if self.override_doctype.get() {
                pubid = Some(self.doctype_pubid.borrow().clone());
                sysid = Some(self.doctype_sysid.borrow().clone());
            }
            self.set_end_location();
            h.doctype_decl(root.as_deref(), pubid.as_deref(), sysid.as_deref(), self.location_augs())?;
        }
        Ok(())
    }

    /// Scans a quoted literal.
    ///
    /// Equivalent to the protected `scanLiteral()`.
    fn scan_literal(&self) -> JResult<Option<String>> {
        let quote = self.read()?;
        if quote == b'\'' as i32 || quote == b'"' as i32 {
            let mut str: Vec<u16> = Vec::new();
            let mut c;
            loop {
                c = self.read()?;
                if c == -1 {
                    break;
                }
                if c == quote {
                    break;
                }
                if c == b'\r' as i32 || c == b'\n' as i32 {
                    self.ce().rewind();
                    // NOTE: This collapses newlines to a single space.
                    //       Is this the right thing to do? -Ac
                    self.skip_newlines()?;
                    str.push(b' ' as u16);
                } else if c == b'<' as i32 {
                    self.ce().rewind();
                    break;
                } else {
                    str.push(c as u16);
                }
            }
            if c == -1 {
                if self.report_errors.get() {
                    self.report_error("HTML1007", None)?;
                }
                return Err(eof());
            }
            return Ok(Some(from_chars(&str)));
        }
        self.ce().rewind();
        Ok(None)
    }

    /// Scans a name.
    ///
    /// Equivalent to the protected `scanName()`.
    fn scan_name(&self) -> JResult<Option<String>> {
        let mut ce = self.ce();
        if DEBUG_BUFFER {
            ce.debug_buffer_if_needed("(scanName: ", "");
        }
        if ce.offset == ce.length && ce.load(0)? == -1 {
            if DEBUG_BUFFER {
                ce.debug_buffer_if_needed(")scanName: ", "");
            }
            return Ok(None);
        }
        let mut offset = ce.offset;
        loop {
            while ce.has_next() {
                let c = ce.get_next_char();
                if !is_letter_or_digit(c) && !(c == b'-' as u16 || c == b'.' as u16 || c == b':' as u16 || c == b'_' as u16) {
                    ce.rewind();
                    break;
                }
            }
            if ce.offset == ce.length {
                let length = ce.length - offset;
                ce.buffer.copy_within(offset as usize..(offset + length) as usize, 0);
                let count = ce.load(length)?;
                offset = 0;
                if count == -1 {
                    break;
                }
            } else {
                break;
            }
        }
        let length = ce.offset - offset;
        let name = if length > 0 {
            Some(from_chars(&ce.buffer[offset as usize..(offset + length) as usize]))
        } else {
            None
        };
        if DEBUG_BUFFER {
            ce.debug_buffer_if_needed(")scanName: ", &format!(" -> \"{name:?}\""));
        }
        Ok(name)
    }

    /// Scans an entity reference.
    ///
    /// Equivalent to the protected `scanEntityRef(XMLStringBuffer,
    /// boolean)`.
    fn scan_entity_ref(&self, str: &mut XMLStringBuffer, content: bool) -> JResult<i32> {
        str.clear();
        str.append_char(b'&' as u16);
        let mut ends_with_semicolon = false;
        loop {
            let c = self.read()?;
            if c == b';' as i32 {
                str.append_char(b';' as u16);
                ends_with_semicolon = true;
                break;
            } else if c == -1 {
                break;
            } else if !entity_char(c) && c != b'#' as i32 {
                self.ce().rewind();
                break;
            }
            str.append_char(c as u16);
        }

        if !ends_with_semicolon && self.report_errors.get() {
            self.report_warning("HTML1004", None)?;
        }
        if str.length == 1 {
            if content {
                if let Some(h) = self.handler() {
                    if self.emitting() {
                        self.set_end_location();
                        h.characters(str, self.location_augs())?;
                    }
                }
            }
            return Ok(-1);
        }

        let s = str.chars().to_vec();
        let name = if ends_with_semicolon {
            from_chars(&s[1..s.len() - 1])
        } else {
            from_chars(&s[1..])
        };
        if let Some(number) = name.strip_prefix('#') {
            let parsed = if name.starts_with("#x") || name.starts_with("#X") {
                parse_int_radix(&name[2..], 16)
            } else {
                parse_int_radix(number, 10)
            };
            let value = match parsed {
                Some(mut value) => {
                    /* PATCH: Asgeir Asgeirsson */
                    if self.fix_windows_char_refs.get() && self.iso8859_encoding.get() {
                        value = self.fix_windows_character(value);
                    }
                    if content {
                        if let Some(h) = self.handler() {
                            if self.emitting() {
                                self.set_end_location();
                                if self.notify_char_refs.get() {
                                    let id = self.resource_id();
                                    h.start_general_entity(&name, Some(&id), None, self.location_augs())?;
                                }
                                str.clear();
                                str.append_char(value as u16);
                                h.characters(str, self.location_augs())?;
                                if self.notify_char_refs.get() {
                                    h.end_general_entity(&name, self.location_augs())?;
                                }
                            }
                        }
                    }
                    value
                }
                None => {
                    if self.report_errors.get() {
                        self.report_error("HTML1005", Some(&[JavaObject::String(name.clone())]))?;
                    }
                    if content {
                        if let Some(h) = self.handler() {
                            if self.emitting() {
                                self.set_end_location();
                                h.characters(str, self.location_augs())?;
                            }
                        }
                    }
                    -1
                }
            };
            return Ok(value);
        }

        let c = html_entities::get(&name);
        // in attributes, some incomplete entities should be recognized,
        // not all
        // TODO: investigate to find which ones (there are some
        // differences between browsers)
        // in a first time, consider only those that behave the same in FF
        // and IE
        let invalid_entity_in_attribute = !content && !ends_with_semicolon && c > 256;
        if c == -1 || invalid_entity_in_attribute {
            if self.report_errors.get() {
                self.report_warning("HTML1006", Some(&[JavaObject::String(name.clone())]))?;
            }
            if content {
                if let Some(h) = self.handler() {
                    if self.emitting() {
                        self.set_end_location();
                        h.characters(str, self.location_augs())?;
                    }
                }
            }
            return Ok(-1);
        }

        if content {
            if let Some(h) = self.handler() {
                if self.emitting() {
                    self.set_end_location();
                    let notify = self.notify_html_builtin_refs.get()
                        || (self.notify_xml_builtin_refs.get() && builtin_xml_ref(&name));
                    if notify {
                        let id = self.resource_id();
                        h.start_general_entity(&name, Some(&id), None, self.location_augs())?;
                    }
                    str.clear();
                    str.append_char(c as u16);
                    h.characters(str, self.location_augs())?;
                    if notify {
                        h.end_general_entity(&name, self.location_augs())?;
                    }
                }
            }
        }

        Ok(c)
    }

    /// Returns true if the specified text is present and is skipped.
    ///
    /// Equivalent to the protected `skip(String, boolean)`.
    fn skip(&self, s: &str, case_sensitive: bool) -> JResult<bool> {
        let s = to_chars(s);
        let mut ce = self.ce();
        for i in 0..s.len() as i32 {
            if ce.offset == ce.length {
                let from = (ce.offset - i) as usize;
                ce.buffer.copy_within(from..from + i as usize, 0);
                if ce.load(i)? == -1 {
                    ce.offset = 0;
                    return Ok(false);
                }
            }
            let mut c0 = s[i as usize];
            let mut c1 = ce.get_next_char();
            if !case_sensitive {
                c0 = upper_first_unit(c0);
                c1 = upper_first_unit(c1);
            }
            if c0 != c1 {
                ce.rewind_by(i + 1);
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Skips markup.
    ///
    /// Equivalent to the protected `skipMarkup(boolean)`.
    fn skip_markup(&self, balance: bool) -> JResult<bool> {
        if DEBUG_BUFFER {
            self.ce().debug_buffer_if_needed("(skipMarkup: ", "");
        }
        let mut depth = 1;
        let mut slashgt = false;
        'outer: loop {
            {
                let mut ce = self.ce();
                if ce.offset == ce.length && ce.load(0)? == -1 {
                    break 'outer;
                }
            }
            loop {
                let mut ce = self.ce();
                if !ce.has_next() {
                    break;
                }
                let mut c = ce.get_next_char();
                if balance && c == b'<' as u16 {
                    depth += 1;
                } else if c == b'>' as u16 {
                    depth -= 1;
                    if depth == 0 {
                        break 'outer;
                    }
                } else if c == b'/' as u16 {
                    if ce.offset == ce.length && ce.load(0)? == -1 {
                        break 'outer;
                    }
                    c = ce.get_next_char();
                    if c == b'>' as u16 {
                        slashgt = true;
                        depth -= 1;
                        if depth == 0 {
                            break 'outer;
                        }
                    } else {
                        ce.rewind();
                    }
                } else if c == b'\r' as u16 || c == b'\n' as u16 {
                    ce.rewind();
                    drop(ce);
                    self.skip_newlines()?;
                }
            }
        }
        if DEBUG_BUFFER {
            self.ce().debug_buffer_if_needed(")skipMarkup: ", &format!(" -> {slashgt}"));
        }
        Ok(slashgt)
    }

    /// Skips whitespace.
    ///
    /// Equivalent to the protected `skipSpaces()`.
    fn skip_spaces(&self) -> JResult<bool> {
        if DEBUG_BUFFER {
            self.ce().debug_buffer_if_needed("(skipSpaces: ", "");
        }
        let mut spaces = false;
        loop {
            let mut ce = self.ce();
            if ce.offset == ce.length && ce.load(0)? == -1 {
                break;
            }
            let c = ce.get_next_char();
            if !is_java_whitespace(c) {
                ce.rewind();
                break;
            }
            spaces = true;
            if c == b'\r' as u16 || c == b'\n' as u16 {
                ce.rewind();
                drop(ce);
                self.skip_newlines()?;
                continue;
            }
        }
        if DEBUG_BUFFER {
            self.ce().debug_buffer_if_needed(")skipSpaces: ", &format!(" -> {spaces}"));
        }
        Ok(spaces)
    }

    /// Skips newlines and returns the number of newlines skipped.
    ///
    /// Equivalent to the protected `skipNewlines()`.
    fn skip_newlines(&self) -> JResult<i32> {
        let mut ce = self.ce();
        if DEBUG_BUFFER {
            ce.debug_buffer_if_needed("(skipNewlines: ", "");
        }

        if !ce.has_next() && ce.load(0)? == -1 {
            if DEBUG_BUFFER {
                ce.debug_buffer_if_needed(")skipNewlines: ", "");
            }
            return Ok(0);
        }
        let mut c = ce.get_current_char();
        let mut newlines = 0;
        if c == b'\n' as u16 || c == b'\r' as u16 {
            loop {
                c = ce.get_next_char();
                if c == b'\r' as u16 {
                    newlines += 1;
                    if ce.offset == ce.length {
                        ce.offset = newlines;
                        if ce.load(newlines)? == -1 {
                            break;
                        }
                    }
                    if ce.get_current_char() == b'\n' as u16 {
                        ce.offset += 1;
                        ce.character_offset += 1;
                    }
                } else if c == b'\n' as u16 {
                    newlines += 1;
                    if ce.offset == ce.length {
                        ce.offset = newlines;
                        if ce.load(newlines)? == -1 {
                            break;
                        }
                    }
                } else {
                    ce.rewind();
                    break;
                }
                if ce.offset >= ce.length - 1 {
                    break;
                }
            }
            ce.inc_lines(newlines);
        }
        if DEBUG_BUFFER {
            ce.debug_buffer_if_needed(")skipNewlines: ", &format!(" -> {newlines}"));
        }
        Ok(newlines)
    }

    /// Returns an augmentations object with a location item added.
    ///
    /// Equivalent to the protected `locationAugs()`.
    fn location_augs(&self) -> Option<&dyn Augmentations> {
        if self.augmentations.get() {
            self.location_item.set_values(
                self.begin_line_number.get(),
                self.begin_column_number.get(),
                self.begin_character_offset.get(),
                self.end_line_number.get(),
                self.end_column_number.get(),
                self.end_character_offset.get(),
            );
            self.infoset_augs.remove_all_items();
            self.infoset_augs
                .put_item(AUGMENTATIONS, Some(Rc::clone(&self.location_item) as Object));
            Some(&self.infoset_augs)
        } else {
            None
        }
    }

    /// Returns an augmentations object with a synthesized item added.
    ///
    /// Equivalent to the protected `synthesizedAugs()`.
    fn synthesized_augs(&self) -> Option<&dyn Augmentations> {
        if self.augmentations.get() {
            self.infoset_augs.remove_all_items();
            self.infoset_augs
                .put_item(AUGMENTATIONS, Some(Rc::new(SYNTHESIZED_ITEM) as Object));
            Some(&self.infoset_augs)
        } else {
            None
        }
    }

    /// Returns an empty resource identifier.
    ///
    /// Equivalent to the protected `resourceId()`: Java clears and returns
    /// its one `fResourceId`; the port returns a cleared copy.
    fn resource_id(&self) -> XMLResourceIdentifierImpl {
        /***/
        self.resource_id.borrow_mut().clear();
        self.resource_id.borrow().clone()
    }

    /// Reads the next characters WITHOUT impacting the buffer content up to
    /// current offset.
    ///
    /// Equivalent to the protected `readPreservingBufferContent()`.
    fn read_preserving_buffer_content(&self) -> JResult<i32> {
        let mut ce = self.ce();
        if DEBUG_BUFFER {
            ce.debug_buffer_if_needed("(read: ", "");
        }
        if ce.offset == ce.length {
            let length = ce.length;
            if ce.load(length)? < 1 {
                if DEBUG_BUFFER {
                    println!(")read: -> -1");
                }
                return Ok(-1);
            }
        }
        let c = ce.get_next_char();
        if DEBUG_BUFFER {
            ce.debug_buffer_if_needed(")read: ", &format!(" -> {}", from_chars(&[c])));
        }
        Ok(c as i32)
    }

    /// Indicates if the end comment --&gt; is available, loading further
    /// data if needed, without to reset the buffer.
    fn end_comment_available(&self) -> JResult<bool> {
        let mut nb_caret = 0;
        let (original_offset, original_column_number, original_character_offset) = {
            let ce = self.ce();
            (ce.offset, ce.get_column_number(), ce.get_character_offset())
        };

        loop {
            let c = self.read_preserving_buffer_content()?;
            if c == -1 {
                self.ce()
                    .restore_position(original_offset, original_column_number, original_character_offset);
                return Ok(false);
            } else if c == b'>' as i32 && nb_caret >= 2 {
                self.ce()
                    .restore_position(original_offset, original_column_number, original_character_offset);
                return Ok(true);
            } else if c == b'-' as i32 {
                nb_caret += 1;
            } else {
                nb_caret = 0;
            }
        }
    }

    /// Dispatches `fScanner.scan(complete)`.
    fn scan_with(&self, kind: ScannerKind, complete: bool) -> JResult<bool> {
        match kind {
            ScannerKind::Content => self.content_scan(complete),
            ScannerKind::Special => self.special_scan(complete),
        }
    }
}

/// How a scanner's `switch` body leaves: Java's `next = true; break`,
/// `continue`, `return`, or the end of the switch.
enum Flow {
    Next,
    Continue,
    Return(bool),
    Fallthrough,
}

/// `String.valueOf(c).toUpperCase(Locale.ENGLISH).charAt(0)`.
fn upper_first_unit(c: u16) -> u16 {
    let s = from_chars(&[c]);
    to_chars(&to_upper_case(&s)).first().copied().unwrap_or(c)
}

//
// ContentScanner: the scanner for regular HTML content
//

impl HTMLScanner {
    /// `ContentScanner.scan(boolean)`: scans part of the document.
    fn content_scan(&self, complete: bool) -> JResult<bool> {
        let mut next;
        loop {
            next = false;
            let result = (|| -> JResult<Flow> {
                match self.scanner_state.get() {
                    STATE_CONTENT => {
                        self.set_begin_location();
                        let c = self.read()?;
                        if c == b'<' as i32 {
                            self.set_scanner_state(STATE_MARKUP_BRACKET);
                            return Ok(Flow::Next);
                        } else if c == b'&' as i32 {
                            let mut buf = self.string_buffer.take();
                            self.scan_entity_ref(&mut buf, true)?;
                        } else if c == -1 {
                            return Err(eof());
                        } else {
                            self.ce().rewind();
                            self.scan_characters()?;
                        }
                    }
                    STATE_MARKUP_BRACKET => {
                        let c = self.read()?;
                        if c == b'!' as i32 {
                            if self.skip("--", false)? {
                                self.scan_comment()?;
                            } else if self.skip("[CDATA[", false)? {
                                self.scan_cdata()?;
                            } else if self.skip("DOCTYPE", false)? {
                                self.scan_doctype()?;
                            } else {
                                if self.report_errors.get() {
                                    self.report_error("HTML1002", None)?;
                                }
                                self.skip_markup(true)?;
                            }
                        } else if c == b'?' as i32 {
                            self.scan_pi()?;
                        } else if c == b'/' as i32 {
                            self.scan_end_element()?;
                        } else if c == -1 {
                            if self.report_errors.get() {
                                self.report_error("HTML1003", None)?;
                            }
                            if let Some(h) = self.handler() {
                                if self.emitting() {
                                    let mut buf = self.string_buffer.take();
                                    buf.clear();
                                    buf.append_char(b'<' as u16);
                                    h.characters(&buf, None)?;
                                }
                            }
                            return Err(eof());
                        } else {
                            self.ce().rewind();
                            self.element_count.set(self.element_count.get() + 1);
                            self.single_boolean.set(false);
                            let mut empty = false;
                            let ename = self.scan_start_element(&mut empty)?;
                            self.single_boolean.set(empty);
                            let ename_lc = ename.as_deref().map(to_lower_case);
                            self.set_begin_location();
                            if ename_lc.as_deref() == Some("script") {
                                self.scan_script_content()?;
                            } else if !self.allow_selfclosing_tags.get()
                                && !self.allow_selfclosing_iframe.get()
                                && ename_lc.as_deref() == Some("iframe")
                            {
                                self.scan_until_end_tag("iframe")?;
                            } else if !self.parse_no_script_content.get() && ename_lc.as_deref() == Some("noscript") {
                                self.scan_until_end_tag("noscript")?;
                            } else if !self.parse_no_frames_content.get() && ename_lc.as_deref() == Some("noframes") {
                                self.scan_until_end_tag("noframes")?;
                            } else if let Some(ename) = &ename {
                                let lc = ename_lc.as_deref().unwrap_or("");
                                if !self.single_boolean.get()
                                    && html_elements::get_element(lc).is_special()
                                    && (!equals_ignore_case(ename, "TITLE") || self.is_ended(lc))
                                {
                                    self.special_set_element_name(ename);
                                    self.set_scanner(ScannerKind::Special);
                                    self.set_scanner_state(STATE_CONTENT);
                                    return Ok(Flow::Return(true));
                                }
                            }
                        }
                        self.set_scanner_state(STATE_CONTENT);
                    }
                    STATE_START_DOCUMENT => {
                        if let Some(h) = self.handler() {
                            if self.emitting() {
                                if DEBUG_CALLBACKS {
                                    println!("startDocument()");
                                }
                                let locator: Option<Rc<dyn XMLLocator>> =
                                    self.this().map(|s| s as Rc<dyn XMLLocator>);
                                let encoding = self.iana_encoding.borrow().clone();
                                let augs = self.location_augs();
                                let nscontext: Rc<dyn NamespaceContext> = Rc::new(NamespaceSupport::new());
                                xerces_bridge::get_instance().xml_document_handler_start_document(
                                    &*h,
                                    locator,
                                    encoding.as_deref(),
                                    Some(nscontext),
                                    augs,
                                )?;
                            }
                        }
                        if self.insert_doctype.get() {
                            if let Some(h) = self.handler() {
                                let root = html_elements::get_element_by_code(html_elements::HTML).name.clone();
                                let root = modify_name(&root, self.names_elems.get());
                                let pubid = self.doctype_pubid.borrow().clone();
                                let sysid = self.doctype_sysid.borrow().clone();
                                h.doctype_decl(Some(&root), Some(&pubid), Some(&sysid), self.synthesized_augs())?;
                            }
                        }
                        self.set_scanner_state(STATE_CONTENT);
                    }
                    STATE_END_DOCUMENT => {
                        if let Some(h) = self.handler() {
                            if self.emitting() && complete {
                                if DEBUG_CALLBACKS {
                                    println!("endDocument()");
                                }
                                self.set_end_location();
                                h.end_document(self.location_augs())?;
                            }
                        }
                        return Ok(Flow::Return(false));
                    }
                    state => {
                        return Err(Exception::runtime(LuceneError::unchecked(
                            "java.lang.RuntimeException",
                            format!("unknown scanner state: {state}"),
                        )));
                    }
                }
                Ok(Flow::Fallthrough)
            })();
            match result {
                Ok(Flow::Next) => next = true,
                Ok(Flow::Return(r)) => return Ok(r),
                Ok(Flow::Continue) | Ok(Flow::Fallthrough) => {}
                Err(e) if is_eof(&e) => {
                    let popped = {
                        let mut stack = self.current_entity_stack.borrow_mut();
                        if stack.is_empty() {
                            None
                        } else {
                            Some(stack.pop().flatten())
                        }
                    };
                    match popped {
                        None => self.set_scanner_state(STATE_END_DOCUMENT),
                        Some(entity) => *self.current_entity.borrow_mut() = entity,
                    }
                    next = true;
                }
                Err(e) => return Err(e),
            }
            if !(next || complete) {
                break;
            }
        }
        Ok(true)
    }

    /// Scans the content of an element (iframe, noscript, noframes) until
    /// its end tag.
    fn scan_until_end_tag(&self, tag_name: &str) -> JResult<()> {
        let mut buffer = XMLStringBuffer::new();
        let end = format!("/{tag_name}");
        let length_to_scan = java_len(tag_name) + 2;

        loop {
            let c = self.read()?;
            if c == -1 {
                break;
            }
            if c == b'<' as i32 {
                let next = to_chars(&(self.next_content(length_to_scan)? + " "));
                let end_units = to_chars(&end);
                if next.len() as i32 >= length_to_scan
                    && equals_ignore_case(&end, &from_chars(&next[..end_units.len()]))
                    && (b'>' as u16 == next[(length_to_scan - 1) as usize]
                        || is_java_whitespace(next[(length_to_scan - 1) as usize]))
                {
                    self.ce().rewind();
                    break;
                }
            }
            if c == b'\r' as i32 || c == b'\n' as i32 {
                self.ce().rewind();
                let newlines = self.skip_newlines()?;
                for _ in 0..newlines {
                    buffer.append_char(b'\n' as u16);
                }
            } else {
                buffer.append_char(c as u16);
            }
        }
        if buffer.length > 0 {
            if let Some(h) = self.handler() {
                self.set_end_location();
                h.characters(&buffer, self.location_augs())?;
            }
        }
        Ok(())
    }

    fn scan_script_content(&self) -> JResult<()> {
        let mut buffer = XMLStringBuffer::new();
        let mut wait_for_end_comment = false;
        loop {
            let c = self.read()?;
            if c == -1 {
                break;
            } else if c == b'-' as i32 && ends_with(&buffer, "<!-") {
                wait_for_end_comment = self.end_comment_available()?;
            } else if !wait_for_end_comment && c == b'<' as i32 {
                let next = to_chars(&(self.next_content(8)? + " "));
                if next.len() >= 8
                    && equals_ignore_case("/script", &from_chars(&next[..7]))
                    && (b'>' as u16 == next[7] || is_java_whitespace(next[7]))
                {
                    self.ce().rewind();
                    break;
                }
            } else if c == b'>' as i32 && ends_with(&buffer, "--") {
                wait_for_end_comment = false;
            }

            if c == b'\r' as i32 || c == b'\n' as i32 {
                self.ce().rewind();
                let newlines = self.skip_newlines()?;
                for _ in 0..newlines {
                    buffer.append_char(b'\n' as u16);
                }
            } else {
                buffer.append_char(c as u16);
            }
        }

        if self.script_strip_comment_delims.get() {
            reduce_to_content(&mut buffer, "<!--", "-->");
        }
        if self.script_strip_cdata_delims.get() {
            reduce_to_content(&mut buffer, "<![CDATA[", "]]>");
        }

        if buffer.length > 0 {
            if let Some(h) = self.handler() {
                if self.emitting() {
                    if DEBUG_CALLBACKS {
                        println!("characters({})", buffer.to_java_string());
                    }
                    self.set_end_location();
                    h.characters(&buffer, self.location_augs())?;
                }
            }
        }
        Ok(())
    }

    /// Reads the next characters WITHOUT impacting the buffer content up to
    /// current offset.
    ///
    /// Equivalent to the protected `ContentScanner.nextContent(int)`.
    fn next_content(&self, len: i32) -> JResult<String> {
        let mut ce = self.ce();
        let original_offset = ce.offset;
        let original_column_number = ce.get_column_number();
        let original_character_offset = ce.get_character_offset();

        let mut buff: Vec<u16> = Vec::with_capacity(len.max(0) as usize);
        for _ in 0..len {
            if ce.offset == ce.length {
                if ce.length as usize == ce.buffer.len() {
                    let l = ce.buffer.len() as i32;
                    ce.load(l)?;
                } else {
                    // everything was already loaded
                    break;
                }
            }

            let c = ce.read()?;
            if c == -1 {
                break;
            }
            buff.push(c as u16);
        }
        ce.restore_position(original_offset, original_column_number, original_character_offset);
        Ok(from_chars(&buff))
    }

    /// Scans characters.
    ///
    /// Equivalent to the protected `ContentScanner.scanCharacters()`.
    fn scan_characters(&self) -> JResult<()> {
        if DEBUG_BUFFER {
            self.ce().debug_buffer_if_needed("(scanCharacters: ", "");
        }
        let mut buf = self.string_buffer.take();
        buf.clear();
        loop {
            let newlines = self.skip_newlines()?;
            let mut ce = self.ce();
            if newlines == 0 && ce.offset == ce.length {
                if DEBUG_BUFFER {
                    ce.debug_buffer_if_needed(")scanCharacters: ", "");
                }
                break;
            }
            let offset = ce.offset - newlines;
            for i in offset..ce.offset {
                ce.buffer[i as usize] = b'\n' as u16;
            }
            while ce.has_next() {
                let c = ce.get_next_char();
                if c == b'<' as u16 || c == b'&' as u16 || c == b'\n' as u16 || c == b'\r' as u16 {
                    ce.rewind();
                    break;
                }
            }
            if ce.offset > offset && self.document_handler.borrow().is_some() && self.emitting() {
                if DEBUG_CALLBACKS {
                    let xml_string = XMLString::with_values(&ce.buffer, offset, ce.offset - offset);
                    println!("characters({})", xml_string.to_java_string());
                }
                self.end_line_number.set(ce.get_line_number());
                self.end_column_number.set(ce.get_column_number());
                self.end_character_offset.set(ce.get_character_offset());
                let len = ce.offset - offset;
                buf.append_chars(&ce.buffer, offset, len);
            }
            if DEBUG_BUFFER {
                ce.debug_buffer_if_needed(")scanCharacters: ", "");
            }

            let has_next = (ce.offset as usize) < ce.buffer.len();
            let next = if has_next { ce.get_current_char() as i32 } else { -1 };

            if next == b'&' as i32 || next == b'<' as i32 || next == -1 {
                break;
            }
        }
        if buf.length != 0 {
            let h = self.handler().ok_or_else(npe)?;
            h.characters(&buf, self.location_augs())?;
        }
        Ok(())
    }

    /// Scans a CDATA section.
    ///
    /// Equivalent to the protected `ContentScanner.scanCDATA()`.
    fn scan_cdata(&self) -> JResult<()> {
        if DEBUG_BUFFER {
            self.ce().debug_buffer_if_needed("(scanCDATA: ", "");
        }
        let mut buf = self.string_buffer.take();
        buf.clear();
        if self.cdata_sections.get() {
            if let Some(h) = self.handler() {
                if self.emitting() {
                    self.set_end_location();
                    if DEBUG_CALLBACKS {
                        println!("startCDATA()");
                    }
                    h.start_cdata(self.location_augs())?;
                }
            }
        } else {
            buf.append_str("[CDATA[");
        }
        let eof_reached = self.scan_markup_content(&mut buf, b']' as u16)?;
        if !self.cdata_sections.get() {
            buf.append_str("]]");
        }
        if let Some(h) = self.handler() {
            if self.emitting() {
                self.set_end_location();
                if self.cdata_sections.get() {
                    if DEBUG_CALLBACKS {
                        println!("characters({})", buf.to_java_string());
                    }
                    h.characters(&buf, self.location_augs())?;
                    if DEBUG_CALLBACKS {
                        println!("endCDATA()");
                    }
                    h.end_cdata(self.location_augs())?;
                } else {
                    if DEBUG_CALLBACKS {
                        println!("comment({})", buf.to_java_string());
                    }
                    h.comment(&buf, self.location_augs())?;
                }
            }
        }
        if DEBUG_BUFFER {
            self.ce().debug_buffer_if_needed(")scanCDATA: ", "");
        }
        if eof_reached {
            return Err(eof());
        }
        Ok(())
    }

    /// Scans a comment.
    ///
    /// Equivalent to the protected `ContentScanner.scanComment()`.
    fn scan_comment(&self) -> JResult<()> {
        if DEBUG_BUFFER {
            self.ce().debug_buffer_if_needed("(scanComment: ", "");
        }
        self.set_end_location();
        let mut buffer = XMLStringBuffer::new();
        let mut eof_reached = self.scan_markup_content(&mut buffer, b'-' as u16)?;
        // no --> found, comment with end only with >
        if eof_reached {
            self.ce().reset_buffer(
                &buffer,
                self.end_line_number.get(),
                self.end_column_number.get(),
                self.end_character_offset.get(),
            );
            buffer = XMLStringBuffer::new(); // take a new one to avoid interactions
            loop {
                let c = self.read()?;
                if c == -1 {
                    if self.report_errors.get() {
                        self.report_error("HTML1007", None)?;
                    }
                    eof_reached = true;
                    break;
                } else if c != b'>' as i32 {
                    buffer.append_char(c as u16);
                    continue;
                } else if c == b'\n' as i32 || c == b'\r' as i32 {
                    self.ce().rewind();
                    let newlines = self.skip_newlines()?;
                    for _ in 0..newlines {
                        buffer.append_char(b'\n' as u16);
                    }
                    continue;
                }
                eof_reached = false;
                break;
            }
        }
        if let Some(h) = self.handler() {
            if self.emitting() {
                if DEBUG_CALLBACKS {
                    println!("comment({})", buffer.to_java_string());
                }
                self.set_end_location();
                h.comment(&buffer, self.location_augs())?;
            }
        }
        if DEBUG_BUFFER {
            self.ce().debug_buffer_if_needed(")scanComment: ", "");
        }
        if eof_reached {
            return Err(eof());
        }
        Ok(())
    }

    /// Scans markup content.
    ///
    /// Equivalent to the protected `ContentScanner.scanMarkupContent(
    /// XMLStringBuffer, char)`.
    fn scan_markup_content(&self, buffer: &mut XMLStringBuffer, cend: u16) -> JResult<bool> {
        let mut c;
        loop {
            c = self.read()?;
            if c == cend as i32 {
                let mut count = 1;
                loop {
                    c = self.read()?;
                    if c == cend as i32 {
                        count += 1;
                        continue;
                    }
                    break;
                }
                if c == -1 {
                    if self.report_errors.get() {
                        self.report_error("HTML1007", None)?;
                    }
                    break;
                }
                if count < 2 {
                    buffer.append_char(cend);
                    //if (c != -1) {
                    self.ce().rewind();
                    //}
                    continue;
                }
                if c != b'>' as i32 {
                    for _ in 0..count {
                        buffer.append_char(cend);
                    }
                    self.ce().rewind();
                    continue;
                }
                for _ in 0..count - 2 {
                    buffer.append_char(cend);
                }
                break;
            } else if c == b'\n' as i32 || c == b'\r' as i32 {
                self.ce().rewind();
                let newlines = self.skip_newlines()?;
                for _ in 0..newlines {
                    buffer.append_char(b'\n' as u16);
                }
                continue;
            } else if c == -1 {
                if self.report_errors.get() {
                    self.report_error("HTML1007", None)?;
                }
                break;
            }
            buffer.append_char(c as u16);
        }
        Ok(c == -1)
    }

    /// Scans a processing instruction.
    ///
    /// Equivalent to the protected `ContentScanner.scanPI()`.
    fn scan_pi(&self) -> JResult<()> {
        if DEBUG_BUFFER {
            self.ce().debug_buffer_if_needed("(scanPI: ", "");
        }
        if self.report_errors.get() {
            self.report_warning("HTML1008", None)?;
        }

        // scan processing instruction
        let target = self.scan_name()?;
        if let Some(target) = target.as_deref().filter(|t| !equals_ignore_case(t, "xml")) {
            // skip spaces
            loop {
                let mut c = self.read()?;
                if c == b'\r' as i32 || c == b'\n' as i32 {
                    if c == b'\r' as i32 {
                        c = self.read()?;
                        if c != b'\n' as i32 {
                            let mut ce = self.ce();
                            ce.offset -= 1;
                            ce.character_offset -= 1;
                        }
                    }
                    self.ce().inc_line();
                    continue;
                }
                if c == -1 {
                    break;
                }
                if c != b' ' as i32 && c != b'\t' as i32 {
                    self.ce().rewind();
                    break;
                }
            }

            // scan data
            let mut buf = self.string_buffer.take();
            buf.clear();
            loop {
                let mut c = self.read()?;
                if c == b'?' as i32 || c == b'/' as i32 {
                    let c0 = c as u16;
                    c = self.read()?;
                    if c == b'>' as i32 {
                        break;
                    }
                    buf.append_char(c0);
                    self.ce().rewind();
                    continue;
                } else if c == b'\r' as i32 || c == b'\n' as i32 {
                    buf.append_char(b'\n' as u16);
                    if c == b'\r' as i32 {
                        c = self.read()?;
                        if c != b'\n' as i32 {
                            let mut ce = self.ce();
                            ce.offset -= 1;
                            ce.character_offset -= 1;
                        }
                    }
                    self.ce().inc_line();
                    continue;
                } else if c == -1 {
                    break;
                } else {
                    buf.append_char(c as u16);
                }
            }
            if let Some(h) = self.handler() {
                self.set_end_location();
                h.processing_instruction(target, Some(&buf), self.location_augs())?;
            }
        }
        // scan xml/text declaration
        else {
            let begin_line_number = self.begin_line_number.get();
            let begin_column_number = self.begin_column_number.get();
            let begin_character_offset = self.begin_character_offset.get();
            let mut attributes = self.content_attributes.take();
            attributes.remove_all_attributes();
            let mut aindex = 0;
            while self.scan_pseudo_attribute(&mut attributes)? {
                // if we've scanned a value for an attribute
                if attributes.get_value(aindex).unwrap_or_default().is_empty() {
                    attributes.remove_attribute_at(aindex);
                } else {
                    let mut q = self.content_qname.take();
                    attributes.get_name(aindex, &mut q);
                    q.rawname = q.rawname.as_deref().map(to_lower_case);
                    attributes.set_name(aindex, &q);
                    aindex += 1;
                }
            }
            if let Some(h) = self.handler() {
                let version = attributes.get_value_by_qname("version");
                let encoding = attributes.get_value_by_qname("encoding");
                let standalone = attributes.get_value_by_qname("standalone");

                // if the encoding is successfully changed, the stream will
                // be processed again with the right encoding an we will come
                // here again but without need to change the encoding
                let xml_decl_now = self.ignore_specified_charset.get() || !self.change_encoding(encoding.as_deref())?;
                if xml_decl_now {
                    self.begin_line_number.set(begin_line_number);
                    self.begin_column_number.set(begin_column_number);
                    self.begin_character_offset.set(begin_character_offset);
                    self.set_end_location();
                    h.xml_decl(version.as_deref(), encoding.as_deref(), standalone.as_deref(), self.location_augs())?;
                }
            }
        }

        if DEBUG_BUFFER {
            self.ce().debug_buffer_if_needed(")scanPI: ", "");
        }
        Ok(())
    }

    /// Scans a start element.
    ///
    /// Equivalent to the protected `ContentScanner.scanStartElement(
    /// boolean[])`.
    fn scan_start_element(&self, empty: &mut bool) -> JResult<Option<String>> {
        let ename = self.scan_name()?;
        let units = ename.as_deref().map(to_chars).unwrap_or_default();
        let length = units.len();
        let c = if length > 0 { units[0] as i32 } else { -1 };
        if length == 0 || !((b'a' as i32..=b'z' as i32).contains(&c) || (b'A' as i32..=b'Z' as i32).contains(&c)) {
            if self.report_errors.get() {
                self.report_error("HTML1009", None)?;
            }
            if let Some(h) = self.handler() {
                if self.emitting() {
                    let mut buf = self.string_buffer.take();
                    buf.clear();
                    buf.append_char(b'<' as u16);
                    if length > 0 {
                        buf.append_str(ename.as_deref().unwrap_or(""));
                    }
                    h.characters(&buf, None)?;
                }
            }
            return Ok(None);
        }
        let ename = modify_name(ename.as_deref().unwrap_or(""), self.names_elems.get());
        let mut attributes = self.content_attributes.take();
        attributes.remove_all_attributes();
        let begin_line_number = self.begin_line_number.get();
        let begin_column_number = self.begin_column_number.get();
        let begin_character_offset = self.begin_character_offset.get();
        while self.scan_attribute(&mut attributes, empty)? {
            // do nothing
        }
        self.begin_line_number.set(begin_line_number);
        self.begin_column_number.set(begin_column_number);
        self.begin_character_offset.set(begin_character_offset);
        let has_byte_stream = self.byte_stream.borrow().is_some();
        if has_byte_stream && self.element_depth.get() == -1 {
            if equals_ignore_case(&ename, "META") && !self.ignore_specified_charset.get() {
                if DEBUG_CHARSET {
                    println!("+++ <META>");
                }
                let http_equiv = get_value(Some(&*attributes), "http-equiv");
                if let Some(http_equiv) = http_equiv.filter(|h| equals_ignore_case(h, "content-type")) {
                    if DEBUG_CHARSET {
                        println!("+++ @content-type: \"{http_equiv}\"");
                    }
                    let content = get_value(Some(&*attributes), "content");
                    if let Some(content) = content {
                        let content = remove_spaces(&content);
                        let lower = to_chars(&to_lower_case(&content));
                        let cu = to_chars(&content);
                        let needle = to_chars("charset=");
                        let index1 = (0..lower.len().saturating_sub(needle.len() - 1))
                            .find(|&i| lower[i..].starts_with(&needle));
                        if let Some(index1) = index1 {
                            let index2 = (index1..cu.len()).find(|&i| cu[i] == b';' as u16);
                            let charset = match index2 {
                                Some(index2) => from_chars(&cu[index1 + 8..index2]),
                                None => from_chars(&cu[index1 + 8..]),
                            };
                            self.change_encoding(Some(&charset))?;
                        }
                    }
                } else {
                    let meta_charset = get_value(Some(&*attributes), "charset");
                    if let Some(meta_charset) = meta_charset {
                        self.change_encoding(Some(&meta_charset))?;
                    }
                }
            } else if equals_ignore_case(&ename, "BODY") {
                if let Some(bs) = self.byte_stream.borrow_mut().take() {
                    bs.borrow_mut().clear();
                }
            } else {
                let element = html_elements::get_element(&ename);
                if let Some(parent) = element.parent() {
                    if !parent.is_empty() && parent[0].code == html_elements::BODY {
                        if let Some(bs) = self.byte_stream.borrow_mut().take() {
                            bs.borrow_mut().clear();
                        }
                    }
                }
            }
        }
        if let Some(h) = self.handler() {
            if self.emitting() {
                let mut q = self.content_qname.take();
                q.set_values(None, Some(&ename), Some(&ename), None);
                if DEBUG_CALLBACKS {
                    println!("startElement({},{:?})", q.to_java_string(), &*attributes);
                }
                self.set_end_location();
                if *empty {
                    h.empty_element(&mut q, &mut *attributes, self.location_augs())?;
                } else {
                    h.start_element(&mut q, &mut *attributes, self.location_augs())?;
                }
            }
        }
        Ok(Some(ename))
    }

    /// Tries to change the encoding used to read the input stream to the
    /// specified one.
    fn change_encoding(&self, charset: Option<&str>) -> JResult<bool> {
        let Some(charset) = charset else {
            return Ok(false);
        };
        let Some(byte_stream) = self.byte_stream.borrow().clone() else {
            return Ok(false);
        };
        let charset = trim(charset);
        let mut encoding_changed = false;
        let iana_encoding = charset.clone();
        let mut java_encoding = EncodingMap::get_iana2java_mapping(&to_upper_case(&iana_encoding));
        if DEBUG_CHARSET {
            println!("+++ ianaEncoding: {iana_encoding}");
            println!("+++ javaEncoding: {java_encoding:?}");
        }
        if java_encoding.is_none() {
            java_encoding = Some(iana_encoding.clone());
            if self.report_errors.get() {
                self.report_error("HTML1001", Some(&[JavaObject::String(iana_encoding.clone())]))?;
            }
        }
        let java_encoding = java_encoding.unwrap_or_default();
        // TODO: compare encoding names after normalization (see
        //       https://sourceforge.net/tracker/?func=detail&atid=952178&aid=1765734&group_id=195122)
        let current = self.java_encoding.borrow().clone();
        if Some(&java_encoding) != current.as_ref() {
            if !is_encoding_compatible(&java_encoding, current.as_deref()) {
                if self.report_errors.get() {
                    self.report_error(
                        "HTML1015",
                        Some(&[
                            JavaObject::String(java_encoding.clone()),
                            current.map(JavaObject::String).unwrap_or(JavaObject::Null),
                        ]),
                    )?;
                }
            }
            // change the charset
            else {
                match input_stream_reader(PlaybackRead(Rc::clone(&byte_stream)), &java_encoding) {
                    Ok((reader, encoding)) => {
                        let default_iana = self.default_iana_encoding.borrow().clone();
                        self.iso8859_encoding.set(
                            to_upper_case(&iana_encoding).starts_with("ISO-8859")
                                || equals_ignore_case(&iana_encoding, &default_iana),
                        );
                        *self.java_encoding.borrow_mut() = Some(java_encoding.clone());
                        self.ce().set_stream(reader, encoding);
                        byte_stream.borrow_mut().playback();
                        self.element_depth.set(self.element_count.get());
                        self.element_count.set(0);
                        encoding_changed = true;
                    }
                    Err(e) if matches!(&e, Exception::Io(io) if io.java_simple_name() == "UnsupportedEncodingException") => {
                        if self.report_errors.get() {
                            self.report_error("HTML1010", Some(&[JavaObject::String(charset.clone())]))?;
                        }
                        // NOTE: If the encoding change doesn't work,
                        //       then there's no point in continuing to
                        //       buffer the input stream.
                        byte_stream.borrow_mut().clear();
                        *self.byte_stream.borrow_mut() = None;
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(encoding_changed)
    }

    /// Scans a real attribute.
    ///
    /// Equivalent to the protected `ContentScanner.scanAttribute(
    /// XMLAttributesImpl, boolean[])`.
    fn scan_attribute(&self, attributes: &mut XMLAttributesImpl, empty: &mut bool) -> JResult<bool> {
        self.scan_attribute_with(attributes, empty, b'/' as u16)
    }

    /// Scans a pseudo attribute.
    ///
    /// Equivalent to the protected `ContentScanner.scanPseudoAttribute(
    /// XMLAttributesImpl)`.
    fn scan_pseudo_attribute(&self, attributes: &mut XMLAttributesImpl) -> JResult<bool> {
        let mut single = self.single_boolean.get();
        let r = self.scan_attribute_with(attributes, &mut single, b'?' as u16);
        self.single_boolean.set(single);
        r
    }

    /// Scans an attribute, pseudo or real.
    ///
    /// Equivalent to the protected `ContentScanner.scanAttribute(
    /// XMLAttributesImpl, boolean[], char)`.
    fn scan_attribute_with(&self, attributes: &mut XMLAttributesImpl, empty: &mut bool, _endc: u16) -> JResult<bool> {
        let skipped_spaces = self.skip_spaces()?;
        self.set_begin_location();
        let mut c = self.read()?;
        if c == -1 {
            if self.report_errors.get() {
                self.report_error("HTML1007", None)?;
            }
            return Ok(false);
        } else if c == b'>' as i32 {
            return Ok(false);
        } else if c == b'<' as i32 {
            self.ce().rewind();
            return Ok(false);
        }
        self.ce().rewind();
        let aname = self.scan_name()?;
        let Some(aname) = aname else {
            if self.report_errors.get() {
                self.report_error("HTML1011", None)?;
            }
            *empty = self.skip_markup(false)?;
            return Ok(false);
        };
        if !skipped_spaces && self.report_errors.get() {
            self.report_error("HTML1013", Some(&[JavaObject::String(aname.clone())]))?;
        }
        let aname = modify_name(&aname, self.names_attrs.get());
        self.skip_spaces()?;
        c = self.read()?;
        if c == -1 {
            if self.report_errors.get() {
                self.report_error("HTML1007", None)?;
            }
            return Err(eof());
        }
        let mut q = self.content_qname.take();
        if c == b'/' as i32 || c == b'>' as i32 {
            q.set_values(None, Some(&aname), Some(&aname), None);
            attributes.add_attribute(&q, "CDATA", Some(""));
            let last = attributes.get_length() - 1;
            attributes.set_specified(last, true);
            if self.augmentations.get() {
                self.add_location_item(attributes, attributes.get_length() - 1);
            }
            if c == b'/' as i32 {
                self.ce().rewind();
                *empty = self.skip_markup(false)?;
            }
            return Ok(false);
        }
        /***/
        //enum = 'value' | "value"
        if c == b'=' as i32 {
            self.skip_spaces()?;
            c = self.read()?;
            if c == -1 {
                if self.report_errors.get() {
                    self.report_error("HTML1007", None)?;
                }
                return Err(eof());
            }
            // Xiaowei/Ac: Fix for <a href=/cgi-bin/myscript>...</a>
            if c == b'>' as i32 {
                q.set_values(None, Some(&aname), Some(&aname), None);
                attributes.add_attribute(&q, "CDATA", Some(""));
                let last = attributes.get_length() - 1;
                attributes.set_specified(last, true);
                if self.augmentations.get() {
                    self.add_location_item(attributes, attributes.get_length() - 1);
                }
                return Ok(false);
            }
            let mut buf = self.string_buffer.take();
            let mut non_norm = self.non_norm_attr.take();
            buf.clear();
            non_norm.clear();
            if c != b'\'' as i32 && c != b'"' as i32 {
                self.ce().rewind();
                loop {
                    c = self.read()?;
                    // Xiaowei/Ac: Fix for <a href=/broken/>...</a>
                    if is_java_whitespace(c as u16) || c == b'>' as i32 {
                        //fCharOffset--;
                        self.ce().rewind();
                        break;
                    }
                    if c == -1 {
                        if self.report_errors.get() {
                            self.report_error("HTML1007", None)?;
                        }
                        return Err(eof());
                    }
                    if c == b'&' as i32 {
                        let mut buf2 = self.string_buffer2.take();
                        let ce = self.scan_entity_ref(&mut buf2, false)?;
                        if ce != -1 {
                            buf.append_char(ce as u16);
                        } else {
                            buf.append_xml_string(&buf2);
                        }
                        non_norm.append_xml_string(&buf2);
                    } else {
                        buf.append_char(c as u16);
                        non_norm.append_char(c as u16);
                    }
                }
                q.set_values(None, Some(&aname), Some(&aname), None);
                let avalue = buf.to_java_string();
                attributes.add_attribute(&q, "CDATA", Some(&avalue));

                let lastattr = attributes.get_length() - 1;
                attributes.set_specified(lastattr, true);
                attributes.set_non_normalized_value(lastattr, Some(&non_norm.to_java_string()));
                if self.augmentations.get() {
                    self.add_location_item(attributes, attributes.get_length() - 1);
                }
                return Ok(true);
            }
            let quote = c;
            let mut is_start = true;
            let mut prev_space = false;
            loop {
                let accept_space = !self.normalize_attributes.get() || (!is_start && !prev_space);
                c = self.read()?;
                if c == -1 {
                    if self.report_errors.get() {
                        self.report_error("HTML1007", None)?;
                    }
                    break;
                    // throw new EOFException();
                }
                if c == b'&' as i32 {
                    is_start = false;
                    let mut buf2 = self.string_buffer2.take();
                    let ce = self.scan_entity_ref(&mut buf2, false)?;
                    if ce != -1 {
                        buf.append_char(ce as u16);
                    } else {
                        buf.append_xml_string(&buf2);
                    }
                    non_norm.append_xml_string(&buf2);
                } else if c == b' ' as i32 || c == b'\t' as i32 {
                    if accept_space {
                        buf.append_char(if self.normalize_attributes.get() { b' ' as u16 } else { c as u16 });
                    }
                    non_norm.append_char(c as u16);
                } else if c == b'\r' as i32 || c == b'\n' as i32 {
                    if c == b'\r' as i32 {
                        let c2 = self.read()?;
                        if c2 != b'\n' as i32 {
                            self.ce().rewind();
                        } else {
                            non_norm.append_char(b'\r' as u16);
                            c = c2;
                        }
                    }
                    if accept_space {
                        buf.append_char(if self.normalize_attributes.get() { b' ' as u16 } else { b'\n' as u16 });
                    }
                    self.ce().inc_line();
                    non_norm.append_char(c as u16);
                } else if c != quote {
                    is_start = false;
                    buf.append_char(c as u16);
                    non_norm.append_char(c as u16);
                }
                prev_space = c == b' ' as i32 || c == b'\t' as i32 || c == b'\r' as i32 || c == b'\n' as i32;
                is_start = is_start && prev_space;
                if c == quote {
                    break;
                }
            }

            if self.normalize_attributes.get() && buf.length > 0 {
                // trailing whitespace already normalized to single space
                let last = (buf.length - 1) as usize;
                if buf.ch.as_deref().unwrap_or(&[])[last] == b' ' as u16 {
                    buf.length -= 1;
                }
            }

            q.set_values(None, Some(&aname), Some(&aname), None);
            let avalue = buf.to_java_string();
            attributes.add_attribute(&q, "CDATA", Some(&avalue));

            let lastattr = attributes.get_length() - 1;
            attributes.set_specified(lastattr, true);
            attributes.set_non_normalized_value(lastattr, Some(&non_norm.to_java_string()));
            if self.augmentations.get() {
                self.add_location_item(attributes, attributes.get_length() - 1);
            }
        } else {
            q.set_values(None, Some(&aname), Some(&aname), None);
            attributes.add_attribute(&q, "CDATA", Some(""));
            let last = attributes.get_length() - 1;
            attributes.set_specified(last, true);
            self.ce().rewind();
            if self.augmentations.get() {
                self.add_location_item(attributes, attributes.get_length() - 1);
            }
        }
        Ok(true)
    }

    /// Adds location augmentations to the specified attribute.
    ///
    /// Equivalent to the protected `ContentScanner.addLocationItem(
    /// XMLAttributes, int)`.
    fn add_location_item(&self, attributes: &mut XMLAttributesImpl, index: i32) {
        self.set_end_location();
        let location_item = LocationItem::new();
        location_item.set_values(
            self.begin_line_number.get(),
            self.begin_column_number.get(),
            self.begin_character_offset.get(),
            self.end_line_number.get(),
            self.end_column_number.get(),
            self.end_character_offset.get(),
        );
        let augs = attributes
            .get_augmentations(index)
            .expect("java.lang.NullPointerException: attribute augmentations");
        augs.put_item(AUGMENTATIONS, Some(Rc::new(location_item) as Object));
    }

    /// Scans an end element.
    ///
    /// Equivalent to the protected `ContentScanner.scanEndElement()`.
    fn scan_end_element(&self) -> JResult<()> {
        let ename = self.scan_name()?;
        if self.report_errors.get() && ename.is_none() {
            self.report_error("HTML1012", None)?;
        }
        self.skip_markup(false)?;
        if let Some(ename) = ename {
            let ename = modify_name(&ename, self.names_elems.get());
            if let Some(h) = self.handler() {
                if self.emitting() {
                    let mut q = self.content_qname.take();
                    q.set_values(None, Some(&ename), Some(&ename), None);
                    if DEBUG_CALLBACKS {
                        println!("endElement({})", q.to_java_string());
                    }
                    self.set_end_location();
                    h.end_element(&mut q, self.location_augs())?;
                }
            }
        }
        Ok(())
    }

    /// Indicates if the end tag of the element is in the buffer.
    fn is_ended(&self, ename: &str) -> bool {
        let ce = self.ce();
        let content = from_chars(&ce.buffer[ce.offset as usize..ce.length as usize]);
        to_lower_case(&content).contains(&format!("</{}>", to_lower_case(ename)))
    }
}

/// Removes all whitespace from the content.
fn remove_spaces(content: &str) -> String {
    let units: Vec<u16> = to_chars(content);
    if !units.iter().any(|&c| is_java_whitespace(c)) {
        return content.to_string();
    }
    from_chars(&units.into_iter().filter(|&c| !is_java_whitespace(c)).collect::<Vec<_>>())
}

//
// SpecialScanner: special scanner used for elements whose content needs to
// be scanned as plain text, ignoring markup such as elements and entity
// references. For example: &lt;SCRIPT&gt; and &lt;COMMENT&gt;.
//

impl HTMLScanner {
    /// Sets the element name.
    ///
    /// Equivalent to `SpecialScanner.setElementName(String)`.
    fn special_set_element_name(&self, ename: &str) {
        *self.special_element_name.borrow_mut() = ename.to_string();
        self.special_style.set(equals_ignore_case(ename, "STYLE"));
        self.special_textarea.set(equals_ignore_case(ename, "TEXTAREA"));
        self.special_title.set(equals_ignore_case(ename, "TITLE"));
    }

    /// `SpecialScanner.scan(boolean)`: scans part of the document.
    fn special_scan(&self, complete: bool) -> JResult<bool> {
        loop {
            let result = (|| -> JResult<Flow> {
                match self.scanner_state.get() {
                    STATE_CONTENT => {
                        self.set_begin_location();
                        let c = self.read()?;
                        if c == b'<' as i32 {
                            self.set_scanner_state(STATE_MARKUP_BRACKET);
                            return Ok(Flow::Continue);
                        }
                        let mut buf = self.special_string_buffer.take();
                        if c == b'&' as i32 {
                            if self.special_textarea.get() || self.special_title.get() {
                                self.scan_entity_ref(&mut buf, true)?;
                                return Ok(Flow::Continue);
                            }
                            buf.clear();
                            buf.append_char(b'&' as u16);
                        } else if c == -1 {
                            if self.report_errors.get() {
                                self.report_error("HTML1007", None)?;
                            }
                            return Err(eof());
                        } else {
                            self.ce().rewind();
                            buf.clear();
                        }
                        self.special_scan_characters(&mut buf, -1)?;
                    }
                    STATE_MARKUP_BRACKET => {
                        let delimiter = -1;
                        let c = self.read()?;
                        let mut buf = self.special_string_buffer.take();
                        if c == b'/' as i32 {
                            let ename = self.scan_name()?;
                            if let Some(ename) = ename {
                                let element_name = self.special_element_name.borrow().clone();
                                if equals_ignore_case(&ename, &element_name) {
                                    if self.read()? == b'>' as i32 {
                                        let ename = modify_name(&ename, self.names_elems.get());
                                        if let Some(h) = self.handler() {
                                            if self.emitting() {
                                                let mut q = self.special_qname.take();
                                                q.set_values(None, Some(&ename), Some(&ename), None);
                                                if DEBUG_CALLBACKS {
                                                    println!("endElement({})", q.to_java_string());
                                                }
                                                self.set_end_location();
                                                h.end_element(&mut q, self.location_augs())?;
                                            }
                                        }
                                        self.set_scanner(ScannerKind::Content);
                                        self.set_scanner_state(STATE_CONTENT);
                                        return Ok(Flow::Return(true));
                                    }
                                    self.ce().rewind();
                                }
                                buf.clear();
                                buf.append_str("</");
                                buf.append_str(&ename);
                            } else {
                                buf.clear();
                                buf.append_str("</");
                            }
                        } else {
                            buf.clear();
                            buf.append_char(b'<' as u16);
                            buf.append_char(c as u16);
                        }
                        self.special_scan_characters(&mut buf, delimiter)?;
                        self.set_scanner_state(STATE_CONTENT);
                    }
                    _ => {}
                }
                Ok(Flow::Fallthrough)
            })();
            match result {
                // `continue` and the end of the body both evaluate the
                // do-while condition (`next` is false here)
                Ok(Flow::Continue) | Ok(Flow::Fallthrough) | Ok(Flow::Next) => {}
                Ok(Flow::Return(r)) => return Ok(r),
                Err(e) if is_eof(&e) => {
                    self.set_scanner(ScannerKind::Content);
                    let popped = {
                        let mut stack = self.current_entity_stack.borrow_mut();
                        if stack.is_empty() {
                            None
                        } else {
                            Some(stack.pop().flatten())
                        }
                    };
                    match popped {
                        None => self.set_scanner_state(STATE_END_DOCUMENT),
                        Some(entity) => {
                            *self.current_entity.borrow_mut() = entity;
                            self.set_scanner_state(STATE_CONTENT);
                        }
                    }
                    return Ok(true);
                }
                Err(e) => return Err(e),
            }
            if !complete {
                break;
            }
        }
        Ok(true)
    }

    /// Scan characters.
    ///
    /// Equivalent to the protected `SpecialScanner.scanCharacters(
    /// XMLStringBuffer, int)`.
    fn special_scan_characters(&self, buffer: &mut XMLStringBuffer, delimiter: i32) -> JResult<()> {
        if DEBUG_BUFFER {
            self.ce()
                .debug_buffer_if_needed(&format!("(scanCharacters, delimiter={delimiter}: "), "");
        }

        loop {
            let c = self.read()?;
            if c == -1 || (c == b'<' as i32 || c == b'&' as i32) {
                if c != -1 {
                    self.ce().rewind();
                }
                break;
            }
            // Patch supplied by Jonathan Baxter
            else if c == b'\r' as i32 || c == b'\n' as i32 {
                self.ce().rewind();
                let newlines = self.skip_newlines()?;
                for _ in 0..newlines {
                    buffer.append_char(b'\n' as u16);
                }
            } else {
                buffer.append_char(c as u16);
                if c == b'\n' as i32 {
                    self.ce().inc_line();
                }
            }
        }

        if self.special_style.get() {
            if self.style_strip_comment_delims.get() {
                reduce_to_content(buffer, "<!--", "-->");
            }
            if self.style_strip_cdata_delims.get() {
                reduce_to_content(buffer, "<![CDATA[", "]]>");
            }
        }

        if buffer.length > 0 {
            if let Some(h) = self.handler() {
                if self.emitting() {
                    if DEBUG_CALLBACKS {
                        println!("characters({})", buffer.to_java_string());
                    }
                    self.set_end_location();
                    h.characters(buffer, self.location_augs())?;
                }
            }
        }
        if DEBUG_BUFFER {
            self.ce().debug_buffer_if_needed(")scanCharacters: ", "");
        }
        Ok(())
    }
}

//
// XMLLocator methods
//

impl XMLLocator for HTMLScanner {
    fn get_encoding(&self) -> Option<String> {
        self.current_entity.borrow().as_ref().and_then(|e| e.encoding.clone())
    }

    fn get_public_id(&self) -> Option<String> {
        self.current_entity.borrow().as_ref().and_then(|e| e.public_id.clone())
    }

    fn get_base_system_id(&self) -> Option<String> {
        self.current_entity.borrow().as_ref().and_then(|e| e.base_system_id.clone())
    }

    fn get_literal_system_id(&self) -> Option<String> {
        self.current_entity.borrow().as_ref().and_then(|e| e.literal_system_id.clone())
    }

    fn get_expanded_system_id(&self) -> Option<String> {
        self.current_entity.borrow().as_ref().and_then(|e| e.expanded_system_id.clone())
    }

    fn get_line_number(&self) -> i32 {
        self.current_entity.borrow().as_ref().map_or(-1, |e| e.get_line_number())
    }

    fn get_column_number(&self) -> i32 {
        self.current_entity.borrow().as_ref().map_or(-1, |e| e.get_column_number())
    }

    fn get_xml_version(&self) -> Option<String> {
        self.current_entity.borrow().as_ref().map(|e| e.version.clone())
    }

    fn get_character_offset(&self) -> i32 {
        self.current_entity.borrow().as_ref().map_or(-1, |e| e.get_character_offset())
    }
}

//
// HTMLComponent methods
//

impl XMLComponent for HTMLScanner {
    fn get_feature_default(&self, feature_id: &str) -> Option<bool> {
        RECOGNIZED_FEATURES
            .iter()
            .position(|f| *f == feature_id)
            .and_then(|i| RECOGNIZED_FEATURES_DEFAULTS[i])
    }

    fn get_property_default(&self, property_id: &str) -> Option<Object> {
        RECOGNIZED_PROPERTIES
            .iter()
            .position(|p| *p == property_id)
            .and_then(recognized_properties_default)
    }

    fn get_recognized_features(&self) -> Option<Vec<String>> {
        Some(RECOGNIZED_FEATURES.iter().map(|s| s.to_string()).collect())
    }

    fn get_recognized_properties(&self) -> Option<Vec<String>> {
        Some(RECOGNIZED_PROPERTIES.iter().map(|s| s.to_string()).collect())
    }

    /// Resets the component.
    fn reset(&self, manager: &dyn XMLComponentManager) -> JResult<()> {
        // get features
        self.augmentations.set(manager.get_feature(AUGMENTATIONS)?);
        self.report_errors.set(manager.get_feature(REPORT_ERRORS)?);
        self.notify_char_refs.set(manager.get_feature(NOTIFY_CHAR_REFS)?);
        self.notify_xml_builtin_refs.set(manager.get_feature(NOTIFY_XML_BUILTIN_REFS)?);
        self.notify_html_builtin_refs.set(manager.get_feature(NOTIFY_HTML_BUILTIN_REFS)?);
        self.fix_windows_char_refs.set(manager.get_feature(FIX_MSWINDOWS_REFS)?);
        self.script_strip_cdata_delims.set(manager.get_feature(SCRIPT_STRIP_CDATA_DELIMS)?);
        self.script_strip_comment_delims.set(manager.get_feature(SCRIPT_STRIP_COMMENT_DELIMS)?);
        self.style_strip_cdata_delims.set(manager.get_feature(STYLE_STRIP_CDATA_DELIMS)?);
        self.style_strip_comment_delims.set(manager.get_feature(STYLE_STRIP_COMMENT_DELIMS)?);
        self.ignore_specified_charset.set(manager.get_feature(IGNORE_SPECIFIED_CHARSET)?);
        self.cdata_sections.set(manager.get_feature(CDATA_SECTIONS)?);
        self.override_doctype.set(manager.get_feature(OVERRIDE_DOCTYPE)?);
        self.insert_doctype.set(manager.get_feature(INSERT_DOCTYPE)?);
        self.normalize_attributes.set(manager.get_feature(NORMALIZE_ATTRIBUTES)?);
        self.parse_no_script_content.set(manager.get_feature(PARSE_NOSCRIPT_CONTENT)?);
        self.allow_selfclosing_iframe.set(manager.get_feature(ALLOW_SELFCLOSING_IFRAME)?);
        self.allow_selfclosing_tags.set(manager.get_feature(ALLOW_SELFCLOSING_TAGS)?);

        // get properties
        self.names_elems
            .set(get_names_value(&value_of(&manager.get_property(NAMES_ELEMS)?)));
        self.names_attrs
            .set(get_names_value(&value_of(&manager.get_property(NAMES_ATTRS)?)));
        *self.default_iana_encoding.borrow_mut() = value_of(&manager.get_property(DEFAULT_ENCODING)?);
        let reporter = manager.get_property(ERROR_REPORTER)?;
        *self.error_reporter.borrow_mut() = match &reporter {
            None => None,
            Some(o) => Some(object_as_interface::<dyn HTMLErrorReporter>(o).ok_or_else(|| {
                Exception::runtime(LuceneError::unchecked(
                    "java.lang.ClassCastException",
                    "org.cyberneko.html.HTMLErrorReporter",
                ))
            })?),
        };
        *self.doctype_pubid.borrow_mut() = value_of(&manager.get_property(DOCTYPE_PUBID)?);
        *self.doctype_sysid.borrow_mut() = value_of(&manager.get_property(DOCTYPE_SYSID)?);
        Ok(())
    }

    /// Sets a feature.
    fn set_feature(&self, feature_id: &str, state: bool) -> JResult<()> {
        match feature_id {
            AUGMENTATIONS => self.augmentations.set(state),
            IGNORE_SPECIFIED_CHARSET => self.ignore_specified_charset.set(state),
            NOTIFY_CHAR_REFS => self.notify_char_refs.set(state),
            NOTIFY_XML_BUILTIN_REFS => self.notify_xml_builtin_refs.set(state),
            NOTIFY_HTML_BUILTIN_REFS => self.notify_html_builtin_refs.set(state),
            FIX_MSWINDOWS_REFS => self.fix_windows_char_refs.set(state),
            SCRIPT_STRIP_CDATA_DELIMS => self.script_strip_cdata_delims.set(state),
            SCRIPT_STRIP_COMMENT_DELIMS => self.script_strip_comment_delims.set(state),
            STYLE_STRIP_CDATA_DELIMS => self.style_strip_cdata_delims.set(state),
            STYLE_STRIP_COMMENT_DELIMS => self.style_strip_comment_delims.set(state),
            PARSE_NOSCRIPT_CONTENT => self.parse_no_script_content.set(state),
            ALLOW_SELFCLOSING_IFRAME => self.allow_selfclosing_iframe.set(state),
            ALLOW_SELFCLOSING_TAGS => self.allow_selfclosing_tags.set(state),
            _ => {}
        }
        Ok(())
    }

    /// Sets a property.
    fn set_property(&self, property_id: &str, value: Option<Object>) -> JResult<()> {
        if property_id == NAMES_ELEMS {
            self.names_elems.set(get_names_value(&value_of(&value)));
            return Ok(());
        }
        if property_id == NAMES_ATTRS {
            self.names_attrs.set(get_names_value(&value_of(&value)));
            return Ok(());
        }
        if property_id == DEFAULT_ENCODING {
            *self.default_iana_encoding.borrow_mut() = value_of(&value);
            return Ok(());
        }
        Ok(())
    }
}

impl HTMLComponent for HTMLScanner {}

impl XMLDocumentSource for HTMLScanner {
    fn set_document_handler(&self, handler: Option<Rc<dyn XMLDocumentHandler>>) {
        HTMLScanner::set_document_handler(self, handler);
    }

    fn get_document_handler(&self) -> Option<Rc<dyn XMLDocumentHandler>> {
        HTMLScanner::get_document_handler(self)
    }
}

impl XMLDocumentScanner for HTMLScanner {
    fn set_input_source(&self, input_source: &XMLInputSource) -> JResult<()> {
        HTMLScanner::set_input_source(self, input_source)
    }

    fn scan_document(&self, complete: bool) -> JResult<bool> {
        HTMLScanner::scan_document(self, complete)
    }
}

impl Scanner for HTMLScanner {
    /// `fScanner.scan(complete)`.
    fn scan(&self, complete: bool) -> JResult<bool> {
        self.scan_with(self.scanner.get(), complete)
    }
}

/// `interface_object` of an error reporter, the value of the
/// `ERROR_REPORTER` property.
pub fn error_reporter_object(reporter: Rc<dyn HTMLErrorReporter>) -> Object {
    interface_object(reporter)
}
