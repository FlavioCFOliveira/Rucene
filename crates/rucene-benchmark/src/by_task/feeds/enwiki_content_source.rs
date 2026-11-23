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

//! The types declared in `EnwikiContentSource.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use std::any::Any;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};

use rucene::error::{LuceneError, Result};
use rucene_analysis_common::jdk::io::{CodingErrorAction, InputStreamReader};

use crate::by_task::feeds::content_items_source::{ContentItemsSource, ContentItemsSourceBase};
use crate::by_task::feeds::content_source::ContentSource;
use crate::by_task::feeds::doc_data::DocData;
use crate::by_task::feeds::no_more_data_exception::NoMoreDataException;
use crate::by_task::utils::config::Config;
use crate::by_task::utils::stream_utils::StreamUtils;
use crate::jdk;
use crate::jdk::sax::{Attributes, DefaultHandler, SaxParser};

const TITLE: usize = 0;
const DATE: usize = TITLE + 1;
const BODY: usize = DATE + 1;
const ID: usize = BODY + 1;
const LENGTH: usize = ID + 1;
// LENGTH is used as the size of the tuple, so whatever constants we need that
// should not be part of the tuple, we should define them after LENGTH.
const PAGE: usize = LENGTH + 1;

/// `months`.
const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

/// `getElementType(String elem)`: the type of the element if defined,
/// otherwise -1.
fn get_element_type(elem: &str) -> i64 {
    match elem {
        "page" => PAGE as i64,
        "text" => BODY as i64,
        "timestamp" => DATE as i64,
        "title" => TITLE as i64,
        "id" => ID as i64,
        _ => -1,
    }
}

/// The parser's hand-off state (`synchronized (this)` of `Parser`).
#[derive(Default)]
struct HandOff {
    started: bool,
    thread_done: bool,
    stopped: bool,
    tuple: Option<Vec<String>>,
    nmde: bool,
}

/// An input stream of the field `is`, shared by the source and the parser
/// thread as Java shares the object: `close()` closes it while the parser
/// thread reads it, whose next read then throws `IOException("Stream
/// closed")`, as `BufferedInputStream.read` does after `close()`. The field
/// compares the streams by identity (`localFileIS != is`), here
/// `Arc::ptr_eq`.
struct ClosableInputStream {
    stream: Mutex<Option<Box<dyn Read + Send>>>,
}

impl ClosableInputStream {
    fn new(stream: Box<dyn Read + Send>) -> Arc<Self> {
        Arc::new(ClosableInputStream {
            stream: Mutex::new(Some(stream)),
        })
    }

    /// `close()`.
    fn close(&self) {
        jdk::lock(&self.stream).take();
    }
}

/// A reading handle on a [`ClosableInputStream`].
struct SharedInputStream(Arc<ClosableInputStream>);

impl Read for SharedInputStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match jdk::lock(&self.0.stream).as_mut() {
            Some(stream) => stream.read(buf),
            None => Err(std::io::Error::other("Stream closed")),
        }
    }
}

/// `localFileIS == is`: the same stream object, or both `null`.
fn same_stream(a: &Option<Arc<ClosableInputStream>>, b: &Option<Arc<ClosableInputStream>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

/// The state the parser thread and the source share.
struct Shared {
    hand_off: Mutex<HandOff>,
    cond: Condvar,
    is: Mutex<Option<Arc<ClosableInputStream>>>,
    file: Mutex<Option<PathBuf>>,
    keep_images: Mutex<bool>,
    forever: Mutex<bool>,
    /// The `openInputStream()` of a subclass, if any.
    open_input_stream_override: Option<OpenInputStreamOverride>,
}

/// An override of the protected `openInputStream()` (a Java subclass
/// method), see [`EnwikiContentSource::with_open_input_stream`].
pub type OpenInputStreamOverride = Arc<dyn Fn() -> Result<Box<dyn Read + Send>> + Send + Sync>;

impl Shared {
    /// `openInputStream()`: open the input stream.
    fn open_input_stream(&self) -> Result<Arc<ClosableInputStream>> {
        if let Some(open_input_stream) = &self.open_input_stream_override {
            return Ok(ClosableInputStream::new(open_input_stream()?));
        }
        let file = jdk::lock(&self.file)
            .clone()
            .ok_or_else(LuceneError::null_pointer)?;
        Ok(ClosableInputStream::new(StreamUtils::input_stream(&file)?))
    }
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.EnwikiContentSource.Parser`
/// (`private class Parser extends DefaultHandler implements Runnable`): the
/// SAX handler, run on its own thread, that hands pages to `next()` one at
/// a time.
struct Parser {
    shared: Arc<Shared>,
    contents: Vec<u16>,
    title: Option<String>,
    body: Option<String>,
    time: Option<String>,
    id: Option<String>,
}

impl Parser {
    /// `time(String original)`.
    fn time(original: &str) -> Result<String> {
        let units: Vec<u16> = original.encode_utf16().collect();
        let sub = |a: usize, b: usize| -> Result<String> {
            if b > units.len() || a > b {
                return Err(LuceneError::unchecked(
                    "java.lang.StringIndexOutOfBoundsException",
                    format!("begin {a}, end {b}, length {}", units.len()),
                ));
            }
            Ok(String::from_utf16_lossy(&units[a..b]))
        };
        let mut buffer = String::new();
        buffer.push_str(&sub(8, 10)?);
        buffer.push('-');
        let month = jdk::parse_int(&sub(5, 7)?)?;
        let name = usize::try_from(month - 1)
            .ok()
            .and_then(|m| MONTHS.get(m))
            .ok_or_else(|| {
                LuceneError::unchecked(
                    "java.lang.ArrayIndexOutOfBoundsException",
                    format!("Index {} out of bounds for length 12", month - 1),
                )
            })?;
        buffer.push_str(name);
        buffer.push('-');
        buffer.push_str(&sub(0, 4)?);
        buffer.push(' ');
        buffer.push_str(&sub(11, 19)?);
        buffer.push_str(".000");
        Ok(buffer)
    }

    /// `run()`.
    fn run(&mut self) -> Result<()> {
        let reader = SaxParser::new();
        loop {
            if jdk::lock(&self.shared.hand_off).stopped {
                return Ok(());
            }
            let local_file_is = jdk::lock(&self.shared.is).clone();
            if let Some(local) = &local_file_is {
                // null means fileIS was closed on us
                // To work around a bug in XERCES (XERCESJ-1257), we assume the XML is always UTF8, so
                // we simply provide reader.
                let char_reader = InputStreamReader::with_action(
                    Box::new(SharedInputStream(Arc::clone(local))) as Box<dyn Read + Send>,
                    CodingErrorAction::Report,
                );
                let mut char_reader = ReadCharsAdapter(Mutex::new(char_reader));
                if let Err(e) = reader.parse(&mut char_reader, self) {
                    if e.is_io_exception() {
                        let is = jdk::lock(&self.shared.is);
                        if !same_stream(&local_file_is, &is) {
                            // fileIS was closed on us, so, just fall through
                        } else {
                            // Exception is real
                            return Err(e);
                        }
                    } else {
                        return Err(e);
                    }
                }
            }
            let mut hand_off = jdk::lock(&self.shared.hand_off);
            if hand_off.stopped || !*jdk::lock(&self.shared.forever) {
                hand_off.nmde = true;
                self.shared.cond.notify_all();
                return Ok(());
            }
            let mut is = jdk::lock(&self.shared.is);
            if same_stream(&local_file_is, &is) {
                // If file is not already re-opened then re-open it now
                *is = Some(self.shared.open_input_stream()?);
            }
        }
    }
}

/// An `InputStreamReader` as the `Reader` the SAX input source reads.
struct ReadCharsAdapter(Mutex<InputStreamReader<Box<dyn Read + Send>>>);

impl std::fmt::Debug for ReadCharsAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InputStreamReader")
    }
}

impl rucene::analysis::CharReader for ReadCharsAdapter {
    fn read(&mut self, cbuf: &mut [u16]) -> Result<usize> {
        jdk::lock(&self.0).read_units(cbuf)
    }

    fn close(&mut self) -> Result<()> {
        jdk::lock(&self.0).close_stream()
    }
}

impl DefaultHandler for Parser {
    fn characters(&mut self, ch: &[u16], start: usize, length: usize) -> Result<()> {
        self.contents.extend_from_slice(&ch[start..start + length]);
        Ok(())
    }

    fn end_element(&mut self, _namespace: &str, _simple: &str, qualified: &str) -> Result<()> {
        let elem_type = get_element_type(qualified);
        let contents = String::from_utf16_lossy(&self.contents);
        match elem_type {
            t if t == PAGE as i64 => {
                // the body must be null and we either are keeping image docs or the
                // title does not start with Image:
                let keep_images = *jdk::lock(&self.shared.keep_images);
                if let Some(body) = &self.body {
                    let title = self
                        .title
                        .as_deref()
                        .ok_or_else(LuceneError::null_pointer)?;
                    if keep_images || !title.starts_with("Image:") {
                        let mut tmp_tuple = vec![String::new(); LENGTH];
                        tmp_tuple[TITLE] = title.replace('\t', " ");
                        tmp_tuple[DATE] = self
                            .time
                            .as_deref()
                            .ok_or_else(LuceneError::null_pointer)?
                            .replace('\t', " ");
                        tmp_tuple[BODY] = body.replace(['\t', '\n'], " ");
                        tmp_tuple[ID] = self.id.clone().unwrap_or_else(|| "null".to_string());
                        let mut hand_off = jdk::lock(&self.shared.hand_off);
                        while hand_off.tuple.is_some() && !hand_off.stopped {
                            hand_off = self
                                .shared
                                .cond
                                .wait(hand_off)
                                .unwrap_or_else(|e| e.into_inner());
                        }
                        hand_off.tuple = Some(tmp_tuple);
                        self.shared.cond.notify_all();
                    }
                }
            }
            t if t == BODY as i64 => {
                // workaround that startswith doesn't have an ignore case option, get at least 20 chars.
                let n = 10.min(self.contents.len());
                let starts_with = String::from_utf16_lossy(&self.contents[..n]).to_lowercase();
                self.body = if starts_with.starts_with("#redirect") {
                    None
                } else {
                    Some(contents)
                };
            }
            t if t == DATE as i64 => {
                self.time = Some(Parser::time(&contents)?);
            }
            t if t == TITLE as i64 => {
                self.title = Some(contents);
            }
            t if t == ID as i64 => {
                // the doc id is the first one in the page.  All other ids after that one can be ignored
                // according to the schema
                if self.id.is_none() {
                    self.id = Some(contents);
                }
            }
            _ => {
                // this element should be discarded.
            }
        }
        Ok(())
    }

    fn start_element(
        &mut self,
        _namespace: &str,
        _simple: &str,
        qualified: &str,
        _attributes: &Attributes,
    ) -> Result<()> {
        let elem_type = get_element_type(qualified);
        match elem_type {
            t if t == PAGE as i64 => {
                self.title = None;
                self.body = None;
                self.time = None;
                self.id = None;
            }
            // intentional fall-through.
            t if t == BODY as i64 || t == DATE as i64 || t == TITLE as i64 || t == ID as i64 => {
                self.contents.clear();
            }
            _ => {
                // this element should be discarded.
            }
        }
        Ok(())
    }
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.EnwikiContentSource`
/// (`public class EnwikiContentSource extends ContentSource`): a
/// [`ContentSource`] which reads the English Wikipedia dump. You can read the
/// .bz2 file directly (it will be decompressed on the fly). Config
/// properties:
///
/// * keep.image.only.docs=false|true (default **true**).
/// * docs.file=<path to the file>
///
/// The XML is parsed by [`SaxParser`](crate::jdk::sax::SaxParser), which
/// streams the dump as the JDK's SAX parser does; its differences from the
/// JDK's parser are declared there.
pub struct EnwikiContentSource {
    base: ContentItemsSourceBase,
    shared: Arc<Shared>,
}

impl Default for EnwikiContentSource {
    fn default() -> Self {
        Self::new()
    }
}

impl EnwikiContentSource {
    /// `new EnwikiContentSource()`.
    pub fn new() -> Self {
        EnwikiContentSource {
            base: ContentItemsSourceBase::new(),
            shared: Arc::new(Shared {
                hand_off: Mutex::new(HandOff::default()),
                cond: Condvar::new(),
                is: Mutex::new(None),
                file: Mutex::new(None),
                keep_images: Mutex::new(true),
                forever: Mutex::new(true),
                open_input_stream_override: None,
            }),
        }
    }

    /// An `EnwikiContentSource` subclass that overrides the protected
    /// `openInputStream()` with `open_input_stream` (Rust has no
    /// subclassing; the override replaces the method body, as in Java).
    pub fn with_open_input_stream(open_input_stream: OpenInputStreamOverride) -> Self {
        let mut source = Self::new();
        Arc::get_mut(&mut source.shared)
            .expect("not shared yet")
            .open_input_stream_override = Some(open_input_stream);
        source
    }

    /// `Parser.next()`.
    fn next(&self) -> Result<Vec<String>> {
        let shared = Arc::clone(&self.shared);
        let mut hand_off = jdk::lock(&shared.hand_off);
        if !hand_off.started {
            hand_off.thread_done = false;
            hand_off.started = true;
            let thread_shared = Arc::clone(&shared);
            let spawned = std::thread::Builder::new()
                .name(jdk::next_thread_name())
                .spawn(move || {
                    let mut parser = Parser {
                        shared: Arc::clone(&thread_shared),
                        contents: Vec::new(),
                        title: None,
                        body: None,
                        time: None,
                        id: None,
                    };
                    if let Err(e) = parser.run() {
                        eprintln!(
                            "Exception in thread \"{}\" {}",
                            jdk::current_thread_name(),
                            jdk::runtime_exception_from(e)
                        );
                    }
                    let mut hand_off = jdk::lock(&thread_shared.hand_off);
                    hand_off.thread_done = true;
                    thread_shared.cond.notify_all();
                });
            spawned.map_err(LuceneError::from)?;
        }
        while hand_off.tuple.is_none()
            && !hand_off.nmde
            && !hand_off.thread_done
            && !hand_off.stopped
        {
            hand_off = shared
                .cond
                .wait(hand_off)
                .unwrap_or_else(|e| e.into_inner());
        }
        if let Some(result) = hand_off.tuple.take() {
            shared.cond.notify_all();
            return Ok(result);
        }
        if hand_off.nmde {
            // Set to null so we will re-start thread in case
            // we are re-used:
            hand_off.started = false;
            hand_off.nmde = false;
            return Err(NoMoreDataException::new());
        }
        // The thread has exited yet did not hit end of
        // data, so this means it hit an exception.  We
        // throw NoMorDataException here to force
        // benchmark to stop the current alg:
        Err(NoMoreDataException::new())
    }

    /// `Parser.stop()`.
    fn stop(&self) {
        let mut hand_off = jdk::lock(&self.shared.hand_off);
        hand_off.stopped = true;
        if hand_off.tuple.is_some() {
            hand_off.tuple = None;
        }
        self.shared.cond.notify_all();
    }
}

impl ContentItemsSource for EnwikiContentSource {
    fn items_base(&self) -> &ContentItemsSourceBase {
        &self.base
    }

    fn java_simple_name(&self) -> &'static str {
        "EnwikiContentSource"
    }

    fn close(&self) -> Result<()> {
        self.stop();
        let mut is = jdk::lock(&self.shared.is);
        if let Some(stream) = is.take() {
            stream.close();
        }
        Ok(())
    }

    fn reset_inputs(&self) -> Result<()> {
        self.base.reset_inputs();
        let stream = self.shared.open_input_stream()?;
        *jdk::lock(&self.shared.is) = Some(stream);
        Ok(())
    }

    fn set_config(&self, config: Arc<Config>) -> Result<()> {
        self.base.set_config(Arc::clone(&config))?;
        *jdk::lock(&self.shared.forever) = self.base.forever();
        *jdk::lock(&self.shared.keep_images) = config.get_bool("keep.image.only.docs", true)?;
        if let Some(file_name) = config.get_str("docs.file", None)? {
            *jdk::lock(&self.shared.file) =
                Some(std::path::absolute(&file_name).unwrap_or_else(|_| PathBuf::from(&file_name)));
        }
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ContentSource for EnwikiContentSource {
    fn get_next_doc_data(&self, doc_data: &mut DocData) -> Result<()> {
        let tuple = self.next()?;
        doc_data.clear();
        doc_data.set_name(Some(tuple[ID].clone()));
        doc_data.set_body(Some(tuple[BODY].clone()));
        doc_data.set_date(Some(tuple[DATE].clone()));
        doc_data.set_title(Some(tuple[TITLE].clone()));
        Ok(())
    }
}
