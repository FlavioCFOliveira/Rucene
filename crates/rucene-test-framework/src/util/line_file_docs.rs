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

//! Port of `org.apache.lucene.tests.util.LineFileDocs`: a minimal port of
//! benchmark's `LineDocSource` + `DocMaker`, so tests can enumerate documents
//! from a line file created by benchmark's `WriteLineDoc` task.

use std::io::{self, Read};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::document::{
    Document, Field, FieldType, IntField, IntPoint, KeywordField, NumericDocValuesField, Store,
    StringField, TextField,
};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::{IndexOptions, IndexableField};
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::CloseableThreadLocal;

use randomizedtesting::jdk::{exception, system};
use rucene_analysis_common::jdk::charset::Charset;
use rucene_analysis_common::jdk::io::{BufferedReader, CodingErrorAction, InputStreamReader};

use crate::jdk::gzip::GzipInputStream;
use crate::util::lucene_test_case::{TEST_LINE_DOCS_FILE, VERBOSE};

/// The class-path resources of `org/apache/lucene/tests/util/` that
/// `getClass().getResourceAsStream(path)` finds, embedded in the crate.
const RESOURCES: [(&str, &[u8]); 2] = [
    (
        "europarl.lines.txt.gz",
        include_bytes!("resources/europarl.lines.txt.gz"),
    ),
    (
        "europarl.lines.txt.seek",
        include_bytes!("resources/europarl.lines.txt.seek"),
    ),
];

/// `getClass().getResourceAsStream(path)`: the embedded resource `path`
/// names, relative to the class's package.
fn resource(path: &str) -> Option<&'static [u8]> {
    RESOURCES
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(_, bytes)| *bytes)
}

/// `BUFFER_SIZE`: 64K.
const BUFFER_SIZE: i32 = 1 << 16;

/// `SEP`.
const SEP: char = '\t';

/// The bytes a line file is read from.
enum Source {
    /// A gzip stream started at a seek point (an embedded resource, or a
    /// file read from disk).
    Gzip(GzipInputStream<'static>),
    /// An uncompressed file from a byte offset.
    Plain(io::Cursor<Vec<u8>>),
}

impl Read for Source {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Source::Gzip(g) => g.read(buf),
            Source::Plain(c) => c.read(buf),
        }
    }
}

/// `new BufferedReader(new InputStreamReader(is, decoder), BUFFER_SIZE)`
/// with a UTF-8 decoder that `REPORT`s malformed input
/// (`LineFileDocs.java:216-221`): the JDK reader emulation, so a
/// `MalformedInputException` carries the JDK's length and surfaces at the
/// `readLine` whose read-ahead decodes it, as in Java.
type LineReader = BufferedReader<InputStreamReader<Source>>;

/// `reader.readLine()` as a `String` (the decoders produce no lone
/// surrogate, so the UTF-16 units convert losslessly).
fn read_line(reader: &mut BufferedReader<InputStreamReader<Source>>) -> Result<Option<String>> {
    Ok(reader
        .read_line()?
        .map(|units| String::from_utf16_lossy(&units)))
}

/// The `IOException` a [`LuceneError`] stands for, for the `io::Result`
/// methods.
fn into_io(e: LuceneError) -> io::Error {
    match e.into_kind() {
        LuceneErrorKind::Io(io) => io,
        other => io::Error::other(LuceneError::from(other)),
    }
}

/// The document [`LineFileDocs::next_doc`] returns: the calling thread's one
/// `DocState.doc`, shared with the caller as Java returns the same object on
/// every call from a thread (fields the caller adds stay in it).
pub type SharedDocument = Arc<AtomicRefCell<Document>>;

/// Port of `org.apache.lucene.tests.util.LineFileDocs`.
///
/// Java reuses one `Document` per thread (`CloseableThreadLocal<DocState>`)
/// and resets its fields' values on each `nextDoc`, so the returned instance
/// changes under the caller; so does this port ([`SharedDocument`]).
pub struct LineFileDocs {
    reader: Mutex<Option<LineReader>>,
    id: AtomicI32,
    path: String,
    random: JdkRandom,
    /// `threadDocs`.
    thread_docs: CloseableThreadLocal<Option<Arc<DocState>>>,
}

impl LineFileDocs {
    /// `new LineFileDocs(random, path)`: `this.random = new
    /// Random(random.nextLong())`, then `open()`.
    ///
    /// # Errors
    ///
    /// The errors of opening `path`.
    pub fn new<R: Random + ?Sized>(random: &R, path: &str) -> io::Result<Self> {
        let docs = LineFileDocs {
            reader: Mutex::new(None),
            id: AtomicI32::new(0),
            path: path.to_string(),
            random: JdkRandom::new(random.next_long()),
            thread_docs: CloseableThreadLocal::new(|| None),
        };
        let reader = docs.open()?;
        *docs.lock() = Some(reader);
        Ok(docs)
    }

    /// `new LineFileDocs(random)`: over `LuceneTestCase.TEST_LINE_DOCS_FILE`.
    ///
    /// # Errors
    ///
    /// The errors of opening the file.
    pub fn with_default_file<R: Random + ?Sized>(random: &R) -> io::Result<Self> {
        Self::new(random, &TEST_LINE_DOCS_FILE)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<LineReader>> {
        self.reader.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `close()`: `IOUtils.close(reader, threadDocs)`.
    pub fn close(&self) {
        *self.lock() = None;
        self.thread_docs.close();
    }

    /// `randomSeekPos(random, size)`.
    fn random_seek_pos<R: Random + ?Sized>(random: Option<&R>, size: i64) -> i64 {
        match random {
            Some(random) if size > 3 => (random.next_long() & i64::MAX) % (size / 3),
            _ => 0,
        }
    }

    /// `open()`.
    fn open(&self) -> io::Result<LineReader> {
        let path = self.path.as_str();
        let source = match resource(path) {
            // if the file comes from Classpath:
            Some(bytes) => {
                let seek_to = self.seek_point(path)?;
                // dev-tools/scripts/create_line_file_docs.py ensures this is
                // a "safe" skip point, and we can begin gunziping from here:
                Source::Gzip(GzipInputStream::with_offset(bytes, seek_to)?)
            }
            // if it's not in classpath, we load it as absolute filesystem
            // path (e.g. Jenkins' home dir)
            None => {
                let bytes = std::fs::read(path)?;
                if path.ends_with(".gz") {
                    // if it is a gzip file, we need to use InputStream and
                    // seek to one of the pre-computed skip points:
                    let seek_to = self.seek_point(path)?;
                    Source::Gzip(GzipInputStream::with_offset(bytes, seek_to)?)
                } else {
                    // file is not compressed: optimized seek
                    let size = bytes.len() as i64;
                    let seek_to = Self::random_seek_pos(Some(&self.random), size);
                    if *VERBOSE {
                        system::out_println(&format!(
                            "TEST: LineFileDocs: file seek to fp={seek_to} on open"
                        ));
                    }
                    let mut cursor = io::Cursor::new(bytes);
                    cursor.set_position(seek_to as u64);
                    // read until newline char, otherwise we may hit
                    // "java.nio.charset.MalformedInputException: Input length
                    // = 1" exception in readline() below, because we seeked
                    // part way through a multi-byte (in UTF-8) encoded
                    // unicode character:
                    if seek_to > 0 {
                        let mut b = [0u8; 1];
                        while cursor.read(&mut b)? == 1 && b[0] != 13 && b[0] != 10 {}
                    }
                    Source::Plain(cursor)
                }
            }
        };
        let decoder = Charset::utf_8()
            .new_decoder()
            .map_err(into_io)?
            .on_malformed_input(CodingErrorAction::Report)
            .on_unmappable_character(CodingErrorAction::Report);
        BufferedReader::with_size(
            InputStreamReader::with_decoder(source, decoder),
            BUFFER_SIZE,
        )
        .map_err(into_io)
    }

    /// The `needSkip` branch of `open()`: a skip point of the `.seek` file
    /// next to the gzip `path` (LUCENE-9191), drawn with `random.nextInt`.
    fn seek_point(&self, path: &str) -> io::Result<usize> {
        let index = path.rfind('.').ok_or_else(|| {
            io::Error::other(format!("could not determine extension for path \"{path}\""))
        })?;
        // e.g. foo.txt --> foo.seek, foo.txt.gz --> foo.txt.seek
        let seek_file_path = format!("{}.seek", &path[..index]);
        let seek_bytes = match resource(&seek_file_path) {
            Some(bytes) => bytes.to_vec(),
            None => std::fs::read(&seek_file_path)?,
        };
        // explicitly insert implicit 0 as the first skip point:
        let mut skip_points = vec![0i64];
        // `new BufferedReader(new InputStreamReader(seekIS, UTF_8))`: malformed
        // input is replaced, not reported.
        let mut reader = BufferedReader::new(InputStreamReader::new_utf8(Source::Plain(
            io::Cursor::new(seek_bytes),
        )));
        while let Some(line) = read_line(&mut reader).map_err(into_io)? {
            // `Long.parseLong(line.trim())`: an unchecked `NumberFormatException`.
            let trimmed = randomizedtesting::jdk::lang::trim(&line);
            let point = randomizedtesting::jdk::lang::parse_long(trimmed).unwrap_or_else(|| {
                exception::throw(
                    "java.lang.NumberFormatException",
                    format!("For input string: \"{trimmed}\""),
                )
            });
            skip_points.push(point);
        }
        let seek_to = skip_points[self.random.next_int_bound(skip_points.len() as i32) as usize];
        if *VERBOSE {
            system::out_println(&format!(
                "TEST: LineFileDocs: stream skip to fp={seek_to} on open"
            ));
        }
        Ok(seek_to as usize)
    }

    /// `reset()`.
    ///
    /// # Errors
    ///
    /// The errors of reopening the file.
    pub fn reset(&self) -> io::Result<()> {
        let mut reader = self.lock();
        // `reader.close()` on the `null` reader of a closed instance.
        if reader.is_none() {
            exception::throw_without_message(exception::NULL_POINTER_EXCEPTION)
        }
        *reader = None;
        *reader = Some(self.open()?);
        self.id.store(0, Ordering::SeqCst);
        Ok(())
    }

    /// `nextDoc()`: the document of the next line, rewinding at the end of
    /// the file.
    ///
    /// # Errors
    ///
    /// The file's errors; `RuntimeException` (`line: [...] is in an invalid
    /// format !`) for a line without two tabs.
    ///
    /// # Panics
    ///
    /// `NullPointerException`, as Java, after [`close`](Self::close) and
    /// for a file without lines.
    pub fn next_doc(&self) -> Result<SharedDocument> {
        let line = {
            let mut guard = self.lock();
            // `reader.readLine()` on the `null` reader of a closed instance.
            let Some(reader) = guard.as_mut() else {
                exception::throw_without_message(exception::NULL_POINTER_EXCEPTION)
            };
            match read_line(reader)? {
                Some(line) => line,
                None => {
                    // Always rewind at end:
                    if *VERBOSE {
                        system::out_println("TEST: LineFileDocs: now rewind file...");
                    }
                    *guard = None;
                    let mut reader = self.open()?;
                    let line = read_line(&mut reader);
                    *guard = Some(reader);
                    // A file without lines: `line.indexOf(SEP)` on `null`.
                    line?.unwrap_or_else(|| {
                        exception::throw_without_message(exception::NULL_POINTER_EXCEPTION)
                    })
                }
            }
        };

        let doc_state = {
            let slot = self.thread_docs.get();
            let mut slot = slot.borrow_mut();
            match slot.as_ref() {
                Some(doc_state) => Arc::clone(doc_state),
                None => {
                    let doc_state = Arc::new(DocState::new()?);
                    *slot = Some(Arc::clone(&doc_state));
                    doc_state
                }
            }
        };

        let invalid = || {
            LuceneError::from(LuceneErrorKind::Unchecked {
                class: "java.lang.RuntimeException",
                message: Some(format!("line: [{line}] is in an invalid format !")),
                cause: None,
            })
        };
        let spot = line.find(SEP).ok_or_else(invalid)?;
        let spot2 = line[spot + 1..]
            .find(SEP)
            .map(|i| i + spot + 1)
            .ok_or_else(invalid)?;

        doc_state
            .body
            .set_string_value(line[spot2 + 1..].to_string())?;
        let title = &line[..spot];
        doc_state.title.set_string_value(title.to_string())?;
        doc_state
            .title_tokenized
            .set_string_value(title.to_string())?;
        doc_state
            .date
            .set_string_value(line[spot + 1..spot2].to_string())?;
        let i = self.id.fetch_add(1, Ordering::SeqCst);
        doc_state.id.set_string_value(i.to_string())?;
        doc_state.id_num.set_value(i);
        doc_state
            .page_views
            .set_value(i64::from(self.random.next_int_bound(10_000)));

        if self.random.next_int_bound(5) == 4 {
            // Make some sparse fields
            let mut doc = Document::new();
            for field in doc_state.doc.borrow().get_fields() {
                doc.add_shared(Arc::clone(field));
            }

            // String.length(), in UTF-16 code units.
            let line_length = line.encode_utf16().count() as i32;
            let title_length = title.encode_utf16().count() as i32;
            if self.random.next_int_bound(3) == 1 {
                let x = self.random.next_int_bound(4);
                doc.add(Box::new(IntPoint::new(
                    &format!("docLength{x}"),
                    &[line_length],
                )?));
            }

            if self.random.next_int_bound(3) == 1 {
                let x = self.random.next_int_bound(4);
                doc.add(Box::new(IntPoint::new(
                    &format!("docTitleLength{x}"),
                    &[title_length],
                )?));
            }

            if self.random.next_int_bound(3) == 1 {
                let x = self.random.next_int_bound(4);
                doc.add(Box::new(NumericDocValuesField::new(
                    &format!("docLength{x}"),
                    i64::from(line_length),
                )));
            }

            // TODO: more random sparse fields here too
        }

        Ok(Arc::clone(&doc_state.doc))
    }
}

/// `DocState`: the per-thread document and the fields `nextDoc` changes in
/// place.
struct DocState {
    doc: SharedDocument,
    title_tokenized: Arc<Field>,
    title: Arc<KeywordField>,
    body: Arc<Field>,
    id: Arc<StringField>,
    id_num: Arc<IntField>,
    date: Arc<StringField>,
    page_views: Arc<NumericDocValuesField>,
}

impl DocState {
    /// `new DocState()`.
    fn new() -> Result<Self> {
        let mut doc = Document::new();

        let title = Arc::new(KeywordField::new("title", String::new(), Store::NO)?);
        doc.add_shared(Arc::clone(&title) as Arc<dyn IndexableField>);

        let ft = Arc::new(FieldType::new_from(TextField::type_stored()));
        ft.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)?;
        ft.set_store_term_vectors(true)?;
        ft.set_store_term_vector_offsets(true)?;
        ft.set_store_term_vector_positions(true)?;

        let title_tokenized = Arc::new(Field::new(
            "titleTokenized",
            String::new(),
            Arc::clone(&ft),
        )?);
        doc.add_shared(Arc::clone(&title_tokenized) as Arc<dyn IndexableField>);

        let body = Arc::new(Field::new("body", String::new(), ft)?);
        doc.add_shared(Arc::clone(&body) as Arc<dyn IndexableField>);

        let id = Arc::new(StringField::new("docid", String::new(), Store::YES)?);
        doc.add_shared(Arc::clone(&id) as Arc<dyn IndexableField>);

        let id_num = Arc::new(IntField::new("docid_int", 0, Store::NO));
        doc.add_shared(Arc::clone(&id_num) as Arc<dyn IndexableField>);

        let date = Arc::new(StringField::new("date", String::new(), Store::YES)?);
        doc.add_shared(Arc::clone(&date) as Arc<dyn IndexableField>);

        // A numeric DV field that can be used for DV updates
        let page_views = Arc::new(NumericDocValuesField::new("page_views", 0));
        doc.add_shared(Arc::clone(&page_views) as Arc<dyn IndexableField>);

        Ok(Self {
            doc: Arc::new(AtomicRefCell::new(doc)),
            title_tokenized,
            title,
            body,
            id,
            id_num,
            date,
            page_views,
        })
    }
}

/// `java.time.LocalDateTime`, the values
/// [`date_field_value_to_local_date_time`] returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LocalDateTime {
    /// The proleptic year.
    pub year: i32,
    /// The month, 1 to 12.
    pub month: u32,
    /// The day of the month.
    pub day: u32,
    /// The hour.
    pub hour: u32,
    /// The minute.
    pub minute: u32,
    /// The second.
    pub second: u32,
    /// The nanosecond.
    pub nano: u32,
}

impl LocalDateTime {
    /// `LocalDateTime.of(year, month, dayOfMonth, hour, minute, second)`.
    pub fn of(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> Self {
        LocalDateTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
            nano: 0,
        }
    }
}

/// `DATE_FIELD_VALUE_TO_LOCALDATETIME`: converts the date formats of europarl
/// (`2023-02-23`, `uuuu-MM-dd`) and enwiki (`12-JAN-2010 12:32:45.000`,
/// `dd-MMM-uuuu HH:mm:ss['.'SSS]`, case-insensitive, `Locale.ROOT` month
/// abbreviations) into a [`LocalDateTime`], with the strict resolver.
///
/// # Errors
///
/// Java's `DateTimeParseException` message head, `Text '…' could not be
/// parsed…`. **Divergence.** The text after it is the port's.
pub fn date_field_value_to_local_date_time(s: &str) -> std::result::Result<LocalDateTime, String> {
    let fail = |detail: &str| format!("Text '{s}' could not be parsed{detail}");
    let b = s.as_bytes();
    let digits = |from: usize, n: usize| -> Option<u32> {
        let part = b.get(from..from + n)?;
        part.iter()
            .all(u8::is_ascii_digit)
            .then(|| part.iter().fold(0u32, |v, d| v * 10 + u32::from(d - b'0')))
    };
    let (year, month, day, hour, minute, second, nano) = if b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && digits(0, 4).is_some()
        && digits(5, 2).is_some()
        && digits(8, 2).is_some()
    {
        let (y, m, d) = (
            digits(0, 4).unwrap(),
            digits(5, 2).unwrap(),
            digits(8, 2).unwrap(),
        );
        (y as i32, m, d, 0, 0, 0, 0)
    } else {
        const MONTHS: [&str; 12] = [
            "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
        ];
        let at = |i: usize| fail(&format!(" at index {i}"));
        let day = digits(0, 2).ok_or_else(|| at(0))?;
        if b.get(2) != Some(&b'-') {
            return Err(at(2));
        }
        let month_text = s.get(3..6).ok_or_else(|| at(3))?.to_ascii_lowercase();
        let month = MONTHS
            .iter()
            .position(|m| *m == month_text)
            .ok_or_else(|| at(3))? as u32
            + 1;
        if b.get(6) != Some(&b'-') {
            return Err(at(6));
        }
        // `uuuu`: `appendValue(YEAR, 4, 19, SignStyle.EXCEEDS_PAD)`, parsed
        // strictly: an optional sign, 4 to 19 digits, `+` only (and then
        // required) beyond 4 digits, no `-0`.
        let mut pos = 7;
        let sign = match b.get(pos) {
            Some(b'+') => Some(true),
            Some(b'-') => Some(false),
            _ => None,
        };
        if sign.is_some() {
            pos += 1;
        }
        let digits_start = pos;
        while pos < b.len() && pos - digits_start < 19 && b[pos].is_ascii_digit() {
            pos += 1;
        }
        let width = pos - digits_start;
        if width < 4 {
            return Err(at(if sign.is_some() { 7 } else { digits_start }));
        }
        match sign {
            Some(true) if width <= 4 => return Err(at(7)),
            None if width > 4 => return Err(at(7)),
            _ => {}
        }
        let magnitude: i128 = s[digits_start..pos].parse().map_err(|_| at(7))?;
        if sign == Some(false) && magnitude == 0 {
            return Err(at(7));
        }
        let year = if sign == Some(false) {
            -magnitude
        } else {
            magnitude
        };
        // `ChronoField.YEAR` is `-999999999..999999999`.
        if !(-999_999_999..=999_999_999).contains(&year) {
            return Err(fail(&format!(
                ": Invalid value for Year (valid values -999999999 - 999999999): {year}"
            )));
        }
        let year = year as i32;
        let o = pos - 11;
        let checks = [(11 + o, b' '), (14 + o, b':'), (17 + o, b':')];
        for (i, c) in checks {
            if b.get(i) != Some(&c) {
                return Err(at(i));
            }
        }
        let hour = digits(12 + o, 2).ok_or_else(|| at(12 + o))?;
        let minute = digits(15 + o, 2).ok_or_else(|| at(15 + o))?;
        let second = digits(18 + o, 2).ok_or_else(|| at(18 + o))?;
        let nano = match b.len() - o {
            20 => 0,
            24 if b[20 + o] == b'.' => digits(21 + o, 3).ok_or_else(|| at(21 + o))? * 1_000_000,
            _ => return Err(fail(&format!(", unparsed text found at index {}", 20 + o))),
        };
        (year, month, day, hour, minute, second, nano)
    };
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let month_length = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return Err(fail(&format!(": Invalid value for MonthOfYear: {month}"))),
    };
    if day == 0 || day > month_length || hour > 23 || minute > 59 || second > 59 {
        return Err(fail(": Invalid date"));
    }
    Ok(LocalDateTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
        nano,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `TestLineFileDocs.testDateFieldNormalization`.
    #[test]
    fn date_field_normalization() {
        // europarl corpus uses this data format.
        assert_eq!(
            date_field_value_to_local_date_time("2023-02-23"),
            Ok(LocalDateTime::of(2023, 2, 23, 0, 0, 0))
        );
        // enwiki uses this data format.
        assert_eq!(
            date_field_value_to_local_date_time("12-JAN-2010 12:32:45.000"),
            Ok(LocalDateTime::of(2010, 1, 12, 12, 32, 45))
        );
        assert!(date_field_value_to_local_date_time("2023-02-30").is_err());
        assert!(date_field_value_to_local_date_time("12-Foo-2010 12:32:45").is_err());
    }

    /// `uuuu` is `appendValue(YEAR, 4, 19, SignStyle.EXCEEDS_PAD)`: strictly
    /// parsed, a year beyond four digits needs (and four digits refuse) `+`,
    /// and a negative year is allowed.
    #[test]
    fn enwiki_year_follows_exceeds_pad_strict_parsing() {
        assert_eq!(
            date_field_value_to_local_date_time("12-JAN-+20100 12:32:45"),
            Ok(LocalDateTime::of(20100, 1, 12, 12, 32, 45))
        );
        assert_eq!(
            date_field_value_to_local_date_time("12-JAN--2010 12:32:45.500"),
            Ok(LocalDateTime {
                nano: 500_000_000,
                ..LocalDateTime::of(-2010, 1, 12, 12, 32, 45)
            })
        );
        assert!(date_field_value_to_local_date_time("12-JAN-20100 12:32:45").is_err());
        assert!(date_field_value_to_local_date_time("12-JAN-+2010 12:32:45").is_err());
        assert!(date_field_value_to_local_date_time("12-JAN--0000 12:32:45").is_err());
    }

    fn scratch_file(name: &str, bytes: &[u8]) -> String {
        let p = std::env::temp_dir().join(format!(
            "rucene-line-file-docs-{name}-{}.txt",
            std::process::id()
        ));
        std::fs::write(&p, bytes).unwrap();
        p.to_str().unwrap().to_string()
    }

    /// `LineFileDocs.java:282`: after `close()` the reader is `null`, and
    /// `nextDoc` throws `NullPointerException`.
    #[test]
    fn next_doc_after_close_is_a_null_pointer_exception() {
        let path = scratch_file("closed", b"t\td\tb\n");
        let docs = LineFileDocs::new(&JdkRandom::new(1), &path).unwrap();
        docs.close();
        let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| docs.next_doc()))
            .expect_err("NPE");
        let t = randomizedtesting::jdk::exception::Throwable::from_panic(payload);
        assert_eq!(t.class_name(), "java.lang.NullPointerException");
        std::fs::remove_file(&path).ok();
    }

    /// `LineFileDocs.java:216-221`: the 64K-unit `BufferedReader` over a
    /// reporting decoder decodes ahead, so malformed UTF-8 later in a small
    /// file fails the first `readLine` with the JDK's
    /// `MalformedInputException("Input length = 1")` (`C3 28`).
    #[test]
    fn malformed_utf8_surfaces_at_the_first_read_ahead() {
        let mut bytes = Vec::new();
        for _ in 0..64 {
            bytes.extend_from_slice(b"title\tdate\tbody\n");
        }
        bytes.extend_from_slice(b"\xC3(\tdate\tbody\n");
        let path = scratch_file("malformed", &bytes);
        let docs = LineFileDocs::new(&JdkRandom::new(7), &path).unwrap();
        let e = docs
            .next_doc()
            .expect_err("the read-ahead meets the malformed bytes");
        let LuceneErrorKind::Io(io) = e.kind() else {
            panic!("expected an IOException, got {e:?}");
        };
        assert_eq!(
            rucene::store::java_nio_file::java_class_of(io),
            "java.nio.charset.MalformedInputException"
        );
        assert_eq!(
            rucene::store::java_nio_file::java_message_of(io).as_deref(),
            Some("Input length = 1")
        );
        std::fs::remove_file(&path).ok();
    }
}
