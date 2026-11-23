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

//! The types declared in `TrecContentSource.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use std::any::Any;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};
use rucene_analysis_common::jdk::charset::Charset;
use rucene_analysis_common::jdk::io::{BufferedReader, InputStreamReader};
use rucene_queryparser::jdk::date::Date;
use rucene_queryparser::jdk::format::ParsePosition;
use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::simple_date_format::SimpleDateFormat;

use crate::by_task::feeds::content_items_source::{ContentItemsSource, ContentItemsSourceBase};
use crate::by_task::feeds::content_source::ContentSource;
use crate::by_task::feeds::doc_data::DocData;
use crate::by_task::feeds::html_parser::HTMLParser;
use crate::by_task::feeds::no_more_data_exception::NoMoreDataException;
use crate::by_task::feeds::trec_doc_parser::{self, index_of, substring, ParsePathType, TrecDocParser};
use crate::by_task::feeds::trec_gov2_parser::TrecGov2Parser;
use crate::by_task::utils::config::Config;
use crate::by_task::utils::stream_utils::StreamUtils;
use crate::classes;
use crate::jdk::{self, ThreadLocal};

/// `DATE_FORMATS`.
const DATE_FORMATS: [&str; 9] = [
    "EEE, dd MMM yyyy kk:mm:ss z", // Tue, 09 Dec 2003 22:39:08 GMT
    "EEE MMM dd kk:mm:ss yyyy z",  // Tue Dec 09 16:45:08 2003 EST
    "EEE, dd-MMM-':'y kk:mm:ss z", // Tue, 09 Dec 2003 22:39:08 GMT
    "EEE, dd-MMM-yyy kk:mm:ss z",  // Tue, 09 Dec 2003 22:39:08 GMT
    "EEE MMM dd kk:mm:ss yyyy",    // Tue Dec 09 16:45:08 2003
    "dd MMM yyyy",                 // 1 March 1994
    "MMM dd, yyyy",                // February 3, 1994
    "yyMMdd",                      // 910513
    "hhmm z.z.z. MMM dd, yyyy",    // 0901 u.t.c. April 28, 1994
];

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.TrecContentSource.DateFormatInfo`.
pub(crate) struct DateFormatInfo {
    dfs: Vec<SimpleDateFormat>,
    pos: ParsePosition,
}

/// The reader `openNextFile` builds.
type TrecReader = BufferedReader<InputStreamReader<Box<dyn Read + Send>>>;

/// The fields guarded by `lock`.
#[derive(Default)]
struct Locked {
    input_files: Vec<PathBuf>,
    next_file: usize,
    reader: Option<TrecReader>,
    iteration: i32,
    curr_path_type: Option<ParsePathType>,
}

/// The fields `setConfig` sets.
struct Settings {
    data_dir: Option<PathBuf>,
    html_parser: Option<Arc<dyn HTMLParser>>,
    exclude_docname_iteration: bool,
    trec_doc_parser: Arc<dyn TrecDocParser>,
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.TrecContentSource`
/// (`public class TrecContentSource extends ContentSource`): implements a
/// [`ContentSource`] over the TREC collection.
///
/// Supports the following configuration parameters (on top of
/// `ContentSource`):
///
/// * **work.dir** - specifies the working directory. Required if "docs.dir"
///   denotes a relative path (**default=work**).
/// * **docs.dir** - specifies the directory where the TREC files reside. Can
///   be set to a relative path if "work.dir" is also specified
///   (**default=trec**).
/// * **trec.doc.parser** - specifies the [`TrecDocParser`] class to use for
///   parsing the TREC documents content (**default=TrecGov2Parser**).
/// * **html.parser** - specifies the [`HTMLParser`] class to use for parsing
///   the HTML parts of the TREC documents content
///   (**default=DemoHTMLParser**).
/// * **content.source.encoding** - if not specified, ISO-8859-1 is used.
/// * **content.source.excludeIteration** - if true, do not append iteration
///   number to docname
pub struct TrecContentSource {
    base: ContentItemsSourceBase,
    date_formats: ThreadLocal<DateFormatInfo>,
    trec_doc_buffer: ThreadLocal<Vec<u16>>,
    /// `lock`: use to synchronize threads on reading from the TREC
    /// documents.
    lock: Mutex<Locked>,
    settings: Mutex<Settings>,
}

impl Default for TrecContentSource {
    fn default() -> Self {
        Self::new()
    }
}

impl TrecContentSource {
    /// `DOCNO`.
    pub const DOCNO: &'static str = "<DOCNO>";
    /// `TERMINATING_DOCNO`.
    pub const TERMINATING_DOCNO: &'static str = "</DOCNO>";
    /// `DOC`.
    pub const DOC: &'static str = "<DOC>";
    /// `TERMINATING_DOC`.
    pub const TERMINATING_DOC: &'static str = "</DOC>";
    /// `NEW_LINE`: separator between lines in the byffer.
    pub const NEW_LINE: &'static str = jdk::NEW_LINE;

    /// `new TrecContentSource()`.
    pub fn new() -> Self {
        TrecContentSource {
            base: ContentItemsSourceBase::new(),
            date_formats: ThreadLocal::new(),
            trec_doc_buffer: ThreadLocal::new(),
            lock: Mutex::new(Locked::default()),
            settings: Mutex::new(Settings {
                data_dir: None,
                html_parser: None,
                exclude_docname_iteration: false,
                trec_doc_parser: Arc::new(TrecGov2Parser), // default
            }),
        }
    }

    /// `getDateFormatInfo()`.
    fn get_date_format_info(&self) -> Result<Arc<Mutex<DateFormatInfo>>> {
        if let Some(dfi) = self.date_formats.get() {
            return Ok(dfi);
        }
        let mut dfs = Vec::with_capacity(DATE_FORMATS.len());
        for pattern in DATE_FORMATS {
            let df = SimpleDateFormat::new(pattern, &Locale::english())?;
            df.set_lenient(true);
            dfs.push(df);
        }
        Ok(self.date_formats.set(Some(DateFormatInfo { dfs, pos: ParsePosition::new(0) })).expect("just set"))
    }

    /// `getHtmlParser()`.
    pub fn get_html_parser(&self) -> Option<Arc<dyn HTMLParser>> {
        jdk::lock(&self.settings).html_parser.clone()
    }

    /// `read(StringBuilder buf, String lineStart, boolean collectMatchLine,
    /// boolean collectAll)`: read until a line starting with the specified
    /// `lineStart`.
    fn read(
        &self,
        l: &mut Locked,
        buf: &mut Vec<u16>,
        line_start: Option<&str>,
        collect_match_line: bool,
        collect_all: bool,
    ) -> Result<()> {
        let mut sep: &str = "";
        loop {
            let reader = l.reader.as_mut().ok_or_else(LuceneError::null_pointer)?;
            let Some(line) = reader.read_line()? else {
                self.open_next_file(l)?;
                continue;
            };
            let line_str = String::from_utf16_lossy(&line);
            if let Some(line_start) = line_start {
                if line_str.starts_with(line_start) {
                    if collect_match_line {
                        buf.extend(sep.encode_utf16());
                        buf.extend_from_slice(&line);
                    }
                    return Ok(());
                }
            }
            if collect_all {
                buf.extend(sep.encode_utf16());
                buf.extend_from_slice(&line);
                sep = Self::NEW_LINE;
            }
        }
    }

    /// `close()` on the locked state.
    fn close_locked(&self, l: &mut Locked) {
        let Some(mut reader) = l.reader.take() else {
            return;
        };
        if let Err(e) = reader.close() {
            if self.base.verbose() {
                println!("failed to close reader !");
                println!("{e}");
            }
        }
    }

    /// `openNextFile()`.
    fn open_next_file(&self, l: &mut Locked) -> Result<()> {
        self.close_locked(l);
        l.curr_path_type = None;
        loop {
            if l.next_file >= l.input_files.len() {
                // exhausted files, start a new round, unless forever set to false.
                if !self.base.forever() {
                    return Err(NoMoreDataException::new());
                }
                l.next_file = 0;
                l.iteration += 1;
            }
            let f = l.input_files[l.next_file].clone();
            l.next_file += 1;
            if self.base.verbose() {
                println!("opening: {} length: {}", f.display(), std::fs::metadata(&f)?.len());
            }
            let opened = (|| -> Result<TrecReader> {
                // support either gzip, bzip2, or regular text file, by extension
                let input_stream = StreamUtils::input_stream(&f)?;
                let encoding = self.base.encoding().ok_or_else(LuceneError::null_pointer)?;
                let charset = Charset::for_name(&encoding)?;
                BufferedReader::with_size(InputStreamReader::with_charset(input_stream, charset), StreamUtils::BUFFER_SIZE as i32)
            })();
            match opened {
                Ok(reader) => {
                    l.reader = Some(reader);
                    l.curr_path_type = Some(trec_doc_parser::path_type(&f));
                    return Ok(());
                }
                Err(e) => {
                    if self.base.verbose() {
                        let absolute = std::path::absolute(&f).unwrap_or_else(|_| f.clone());
                        println!(
                            "Skipping 'bad' file {} due to {}",
                            absolute.display(),
                            jdk::get_message(&e).unwrap_or_else(|| "null".to_string())
                        );
                        continue;
                    }
                    return Err(NoMoreDataException::new());
                }
            }
        }
    }

    /// `parseDate(String dateStr)`: the first of [`DATE_FORMATS`] that
    /// parses, or `None` (the caller assigns 'now').
    ///
    /// # Errors
    ///
    /// The exception of building the date formats.
    pub fn parse_date(&self, date_str: &str) -> Result<Option<Date>> {
        let date_str = jdk::java_trim(date_str);
        let dfi = self.get_date_format_info()?;
        let mut dfi = jdk::lock(&dfi);
        let DateFormatInfo { dfs, pos } = &mut *dfi;
        for df in dfs.iter() {
            pos.index = 0;
            pos.error_index = -1;
            if let Some(d) = df.parse_position(date_str, pos)? {
                // Parse succeeded.
                return Ok(Some(d));
            }
        }
        // do not fail test just because a date could not be parsed
        if self.base.verbose() {
            println!("failed to parse date (assigning 'now') for: {date_str}");
        }
        Ok(None)
    }
}

impl ContentItemsSource for TrecContentSource {
    fn items_base(&self) -> &ContentItemsSourceBase {
        &self.base
    }

    fn java_simple_name(&self) -> &'static str {
        "TrecContentSource"
    }

    fn close(&self) -> Result<()> {
        let mut l = jdk::lock(&self.lock);
        self.close_locked(&mut l);
        Ok(())
    }

    fn reset_inputs(&self) -> Result<()> {
        let mut l = jdk::lock(&self.lock);
        self.base.reset_inputs();
        self.close_locked(&mut l);
        l.next_file = 0;
        l.iteration = 0;
        Ok(())
    }

    fn set_config(&self, config: Arc<Config>) -> Result<()> {
        self.base.set_config(Arc::clone(&config))?;
        // dirs
        let work_dir = PathBuf::from(config.get_str("work.dir", Some("work"))?.unwrap_or_default());
        let d = config.get_str("docs.dir", Some("trec"))?.unwrap_or_default();
        let mut data_dir = PathBuf::from(&d);
        if !data_dir.is_absolute() {
            data_dir = work_dir.join(&d);
        }
        // files
        {
            let mut l = jdk::lock(&self.lock);
            ContentItemsSourceBase::collect_files(&data_dir, &mut l.input_files).map_err(jdk::runtime_exception_from)?;
            if l.input_files.is_empty() {
                return Err(LuceneError::illegal_argument(format!("No files in dataDir: {}", data_dir.display())));
            }
        }
        // trec doc parser
        let trec_doc_parser_class_name = config
            .get_str("trec.doc.parser", Some("org.apache.lucene.benchmark.byTask.feeds.TrecGov2Parser"))?
            .unwrap_or_default();
        let trec_doc_parser =
            classes::trec_doc_parser_for_name(&trec_doc_parser_class_name).map_err(jdk::runtime_exception_from)?;
        // html parser
        let html_parser_class_name = config
            .get_str("html.parser", Some("org.apache.lucene.benchmark.byTask.feeds.DemoHTMLParser"))?
            .unwrap_or_default();
        let html_parser = classes::html_parser_for_name(&html_parser_class_name).map_err(jdk::runtime_exception_from)?;
        // encoding
        if self.base.encoding().is_none() {
            self.base.set_encoding(Some("ISO-8859-1".to_string()));
        }
        // iteration exclusion in doc name
        let exclude = config.get_bool("content.source.excludeIteration", false)?;
        let mut s = jdk::lock(&self.settings);
        s.data_dir = Some(data_dir);
        s.trec_doc_parser = trec_doc_parser;
        s.html_parser = Some(html_parser);
        s.exclude_docname_iteration = exclude;
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ContentSource for TrecContentSource {
    fn get_next_doc_data(&self, doc_data: &mut DocData) -> Result<()> {
        let doc_buf_cell = self.trec_doc_buffer.get_or_init(Vec::new);
        let mut doc_buf = jdk::lock(&doc_buf_cell);
        let exclude_docname_iteration = jdk::lock(&self.settings).exclude_docname_iteration;
        let (name, parsed_path_type) = {
            // protect reading from the TREC files by multiple threads. The rest of the
            // method, i.e., parsing the content and returning the DocData can run unprotected.
            let mut l = jdk::lock(&self.lock);
            if l.reader.is_none() {
                self.open_next_file(&mut l)?;
            }

            // 1. skip until doc start - required for all TREC formats
            doc_buf.clear();
            self.read(&mut l, &mut doc_buf, Some(Self::DOC), false, false)?;

            // save parsedFile for passing trecDataParser after the sync block, in
            // case another thread will open another file in between.
            let parsed_path_type = l.curr_path_type;

            // 2. name - required for all TREC formats
            doc_buf.clear();
            self.read(&mut l, &mut doc_buf, Some(Self::DOCNO), true, false)?;
            let docno_len = jdk::utf16_len(Self::DOCNO) as i64;
            let end = index_of(&doc_buf, Self::TERMINATING_DOCNO, docno_len);
            let mut name = jdk::java_trim(&substring(&doc_buf, docno_len, end)?).to_string();
            if !exclude_docname_iteration {
                name = format!("{name}_{}", l.iteration);
            }

            // 3. read all until end of doc
            doc_buf.clear();
            self.read(&mut l, &mut doc_buf, Some(Self::TERMINATING_DOC), false, true)?;
            (name, parsed_path_type)
        };

        // count char length of text to be parsed (may be larger than the resulted plain doc body text).
        self.base.add_bytes(doc_buf.len() as i64);

        // This code segment relies on HtmlParser being thread safe. When we get
        // here, everything else is already private to that thread, so we're safe.
        let trec_doc_parser = Arc::clone(&jdk::lock(&self.settings).trec_doc_parser);
        trec_doc_parser.parse(doc_data, &name, self, &doc_buf, parsed_path_type)?;
        self.base.add_item();
        Ok(())
    }
}
