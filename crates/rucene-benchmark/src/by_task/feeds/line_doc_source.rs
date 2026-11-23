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

//! The types declared in `LineDocSource.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use std::any::Any;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};
use rucene_analysis_common::jdk::charset::Charset;
use rucene_analysis_common::jdk::io::{BufferedReader, InputStreamReader};

use crate::by_task::feeds::content_items_source::{ContentItemsSource, ContentItemsSourceBase};
use crate::by_task::feeds::content_source::ContentSource;
use crate::by_task::feeds::doc_data::DocData;
use crate::by_task::feeds::doc_maker::DocMaker;
use crate::by_task::feeds::no_more_data_exception::NoMoreDataException;
use crate::by_task::tasks::write_line_doc_task::WriteLineDocTask;
use crate::by_task::utils::config::Config;
use crate::by_task::utils::stream_utils::StreamUtils;
use crate::classes;
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.LineDocSource.LineParser`
/// (`public abstract static class LineParser`): reader of a single input line
/// into [`DocData`].
pub trait LineParser: Send + Sync {
    /// `header`: header line found in the input file, or `None` if none.
    fn header(&self) -> Option<&[String]>;

    /// `parseLine(DocData docData, String line)`: parse an input line and
    /// fill doc data appropriately.
    ///
    /// # Errors
    ///
    /// A `RuntimeException` for a line in an invalid format.
    fn parse_line(&self, doc_data: &mut DocData, line: &str) -> Result<()>;
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.LineDocSource.SimpleLineParser`
/// (`public static class SimpleLineParser extends LineParser`):
/// [`LineParser`] which ignores the header passed to its constructor and
/// assumes simply that field names and their order are the same as in
/// [`WriteLineDocTask::DEFAULT_FIELDS`].
pub struct SimpleLineParser {
    header: Option<Vec<String>>,
}

impl SimpleLineParser {
    /// `SimpleLineParser(String[] header)`.
    pub fn new(header: Option<Vec<String>>) -> Self {
        SimpleLineParser { header }
    }
}

impl LineParser for SimpleLineParser {
    fn header(&self) -> Option<&[String]> {
        self.header.as_deref()
    }

    fn parse_line(&self, doc_data: &mut DocData, line: &str) -> Result<()> {
        let sep = WriteLineDocTask::SEP;
        let mut k1 = 0;
        let Some(k2) = line[k1..].find(sep).map(|p| p + k1) else {
            return Err(jdk::runtime_exception(format!(
                "line: [{line}] is in an invalid format (missing: separator title::date)!"
            )));
        };
        doc_data.set_title(Some(line[k1..k2].to_string()));
        k1 = k2 + 1;
        let Some(k2) = line[k1..].find(sep).map(|p| p + k1) else {
            return Err(jdk::runtime_exception(format!(
                "line: [{line}] is in an invalid format (missing: separator date::body)!"
            )));
        };
        doc_data.set_date(Some(line[k1..k2].to_string()));
        k1 = k2 + 1;
        if line[k1..].contains(sep) {
            return Err(jdk::runtime_exception(format!(
                "line: [{line}] is in an invalid format (too many separators)!"
            )));
        }
        // last one
        doc_data.set_body(Some(line[k1..].to_string()));
        Ok(())
    }
}

/// Equivalent to
/// `org.apache.lucene.benchmark.byTask.feeds.LineDocSource.HeaderLineParser.FieldName`
/// (`private enum FieldName`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FieldName {
    /// Java constant `NAME`.
    Name,
    /// Java constant `TITLE`.
    Title,
    /// Java constant `DATE`.
    Date,
    /// Java constant `BODY`.
    Body,
    /// Java constant `PROP`.
    Prop,
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.LineDocSource.HeaderLineParser`
/// (`public static class HeaderLineParser extends LineParser`): [`LineParser`]
/// which sets field names and order by the header - any header - of the
/// lines file. It is less efficient than [`SimpleLineParser`] but more
/// powerful.
pub struct HeaderLineParser {
    header: Vec<String>,
    pos_to_f: Vec<FieldName>,
}

impl HeaderLineParser {
    /// `HeaderLineParser(String[] header)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for a `null` header.
    pub fn new(header: Option<Vec<String>>) -> Result<Self> {
        let header = header.ok_or_else(LuceneError::null_pointer)?;
        let pos_to_f = header
            .iter()
            .map(|f| match f.as_str() {
                DocMaker::NAME_FIELD => FieldName::Name,
                DocMaker::TITLE_FIELD => FieldName::Title,
                DocMaker::DATE_FIELD => FieldName::Date,
                DocMaker::BODY_FIELD => FieldName::Body,
                _ => FieldName::Prop,
            })
            .collect();
        Ok(HeaderLineParser { header, pos_to_f })
    }

    /// `setDocDataField(DocData docData, int position, String text)`.
    fn set_doc_data_field(&self, doc_data: &mut DocData, position: usize, text: &str) {
        match self.pos_to_f[position] {
            FieldName::Name => doc_data.set_name(Some(text.to_string())),
            FieldName::Title => doc_data.set_title(Some(text.to_string())),
            FieldName::Date => doc_data.set_date(Some(text.to_string())),
            FieldName::Body => doc_data.set_body(Some(text.to_string())),
            FieldName::Prop => {
                let mut p = doc_data.get_props().cloned().unwrap_or_default();
                p.set_property(self.header[position].clone(), text);
                doc_data.set_props(Some(p));
            }
        }
    }
}

impl LineParser for HeaderLineParser {
    fn header(&self) -> Option<&[String]> {
        Some(&self.header)
    }

    fn parse_line(&self, doc_data: &mut DocData, line: &str) -> Result<()> {
        let invalid = |n: usize| {
            jdk::runtime_exception(format!(
                "input line has invalid format: {} fields instead of {} :: [{line}]",
                n + 1,
                self.header.len()
            ))
        };
        let mut n = 0;
        let mut k1 = 0;
        while let Some(k2) = line[k1..].find(WriteLineDocTask::SEP).map(|p| p + k1) {
            if n >= self.header.len() {
                return Err(invalid(n));
            }
            self.set_doc_data_field(doc_data, n, &line[k1..k2]);
            n += 1;
            k1 = k2 + 1;
        }
        if n as i64 != self.header.len() as i64 - 1 {
            return Err(invalid(n));
        }
        // last one
        self.set_doc_data_field(doc_data, n, &line[k1..]);
        Ok(())
    }
}

/// The line reader `openFile` builds.
type LineReader = BufferedReader<InputStreamReader<Box<dyn Read + Send>>>;

#[derive(Default)]
struct State {
    file: Option<PathBuf>,
    reader: Option<LineReader>,
    read_count: i32,
    doc_data_line_reader: Option<Arc<dyn LineParser>>,
    skip_header_line: bool,
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.LineDocSource`
/// (`public class LineDocSource extends ContentSource`): a
/// [`ContentSource`] reading one line at a time as a `Document` from a single
/// file. This saves IO cost (over DirContentSource) of recursing through a
/// directory and opening a new file for every document.
///
/// The expected format of each line is (arguments are separated by
/// <TAB>): *title, date, body*. If a line is read in a different format, a
/// `RuntimeException` will be thrown. In general, you should use this
/// content source for files that were created with [`WriteLineDocTask`].
///
/// Config properties:
///
/// * docs.file=<path to the file>
/// * content.source.encoding - default to UTF-8.
/// * line.parser - default to [`HeaderLineParser`] if a header line exists
///   which differs from [`WriteLineDocTask::DEFAULT_FIELDS`] and to
///   [`SimpleLineParser`] otherwise.
#[derive(Default)]
pub struct LineDocSource {
    base: ContentItemsSourceBase,
    state: Mutex<State>,
}

impl LineDocSource {
    /// `new LineDocSource()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `openFile()`.
    fn open_file(&self, s: &mut State) -> Result<()> {
        let result = (|| -> Result<()> {
            if let Some(mut reader) = s.reader.take() {
                reader.close()?;
            }
            let file = s.file.clone().ok_or_else(LuceneError::null_pointer)?;
            let is = StreamUtils::input_stream(&file)?;
            let encoding = self.base.encoding().unwrap_or_else(|| "UTF-8".to_string());
            let charset = Charset::for_name(&encoding)?;
            let mut reader = BufferedReader::with_size(
                InputStreamReader::with_charset(is, charset),
                StreamUtils::BUFFER_SIZE as i32,
            )?;
            if s.skip_header_line {
                reader.read_line()?; // skip one line - the header line - already handled that info
            }
            s.reader = Some(reader);
            Ok(())
        })();
        result.map_err(|e| {
            if e.is_io_exception() {
                jdk::runtime_exception_from(e)
            } else {
                e
            }
        })
    }

    /// `createDocDataLineReader(String line)`.
    fn create_doc_data_line_reader(
        &self,
        s: &mut State,
        line: &str,
    ) -> Result<Arc<dyn LineParser>> {
        let head_indicator = format!(
            "{}{}",
            WriteLineDocTask::FIELDS_HEADER_INDICATOR,
            WriteLineDocTask::SEP
        );
        let header: Vec<String> = if let Some(rest) = line.strip_prefix(&head_indicator) {
            s.skip_header_line = true; // mark to skip the header line when input file is reopened
            jdk::split(rest, &WriteLineDocTask::SEP.to_string())
        } else {
            WriteLineDocTask::DEFAULT_FIELDS
                .iter()
                .map(|s| s.to_string())
                .collect()
        };

        // if a specific DocDataLineReader was configured, must respect it
        let config = self
            .base
            .get_config()
            .ok_or_else(LuceneError::null_pointer)?;
        if let Some(class_name) = config.get_str("line.parser", None)? {
            return classes::line_parser_for_name(&class_name, Some(header)).map_err(|e| {
                jdk::runtime_exception_with_cause(format!("Failed to instantiate {class_name}"), e)
            });
        }

        // if this the simple case,
        if header
            .iter()
            .map(String::as_str)
            .eq(WriteLineDocTask::DEFAULT_FIELDS)
        {
            return Ok(Arc::new(SimpleLineParser::new(Some(header))));
        }
        Ok(Arc::new(HeaderLineParser::new(Some(header))?))
    }
}

impl ContentItemsSource for LineDocSource {
    fn items_base(&self) -> &ContentItemsSourceBase {
        &self.base
    }

    fn java_simple_name(&self) -> &'static str {
        "LineDocSource"
    }

    fn close(&self) -> Result<()> {
        let mut s = jdk::lock(&self.state);
        if let Some(mut reader) = s.reader.take() {
            reader.close()?;
        }
        Ok(())
    }

    fn reset_inputs(&self) -> Result<()> {
        self.base.reset_inputs();
        let mut s = jdk::lock(&self.state);
        self.open_file(&mut s)
    }

    fn set_config(&self, config: Arc<Config>) -> Result<()> {
        self.base.set_config(Arc::clone(&config))?;
        let Some(file_name) = config.get_str("docs.file", None)? else {
            return Err(LuceneError::illegal_argument("docs.file must be set"));
        };
        jdk::lock(&self.state).file =
            Some(std::path::absolute(&file_name).unwrap_or_else(|_| PathBuf::from(&file_name)));
        if self.base.encoding().is_none() {
            self.base.set_encoding(Some("UTF-8".to_string()));
        }
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ContentSource for LineDocSource {
    fn get_next_doc_data(&self, doc_data: &mut DocData) -> Result<()> {
        let (line, my_id, parser) = {
            let mut s = jdk::lock(&self.state);
            loop {
                let reader = s.reader.as_mut().ok_or_else(LuceneError::null_pointer)?;
                let line = match reader.read_line()? {
                    Some(line) => String::from_utf16_lossy(&line),
                    None => {
                        if !self.base.forever() {
                            return Err(NoMoreDataException::new());
                        }
                        // Reset the file
                        self.open_file(&mut s)?;
                        continue;
                    }
                };
                if s.doc_data_line_reader.is_none() {
                    // first line ever, one time initialization,
                    let parser = self.create_doc_data_line_reader(&mut s, &line)?;
                    s.doc_data_line_reader = Some(parser);
                    if s.skip_header_line {
                        continue;
                    }
                }
                // increment IDS only once...
                let my_id = s.read_count;
                s.read_count += 1;
                break (
                    line,
                    my_id,
                    Arc::clone(s.doc_data_line_reader.as_ref().expect("set above")),
                );
            }
        };

        // The date String was written in the format of DateTools.dateToString.
        doc_data.clear();
        doc_data.set_id(my_id);
        parser.parse_line(doc_data, &line)
    }
}

/// A constructor of a `LineParser`: `getConstructor(String[].class)`.
pub type LineParserConstructor =
    Arc<dyn Fn(Option<Vec<String>>) -> Result<Arc<dyn LineParser>> + Send + Sync>;
