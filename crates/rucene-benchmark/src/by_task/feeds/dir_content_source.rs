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

//! The types declared in `DirContentSource.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use std::any::Any;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};
use rucene_queryparser::jdk::date::Date;
use rucene_queryparser::jdk::format::ParsePosition;
use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::simple_date_format::SimpleDateFormat;

use crate::by_task::feeds::content_items_source::{ContentItemsSource, ContentItemsSourceBase};
use crate::by_task::feeds::content_source::ContentSource;
use crate::by_task::feeds::doc_data::DocData;
use crate::by_task::feeds::no_more_data_exception::NoMoreDataException;
use crate::by_task::utils::config::Config;
use crate::jdk::{self, ThreadLocal};

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.DirContentSource.DateFormatInfo`.
pub(crate) struct DateFormatInfo {
    pub(crate) df: SimpleDateFormat,
    pub(crate) pos: ParsePosition,
}

impl DateFormatInfo {
    /// `new SimpleDateFormat("dd-MMM-yyyy kk:mm:ss.SSS", Locale.ENGLISH)`,
    /// lenient, with a `ParsePosition(0)`: date format: 30-MAR-1987
    /// 14:22:36.87.
    pub(crate) fn new() -> Result<Self> {
        let df = SimpleDateFormat::new("dd-MMM-yyyy kk:mm:ss.SSS", &Locale::english())?;
        df.set_lenient(true);
        Ok(DateFormatInfo { df, pos: ParsePosition::new(0) })
    }

    /// `parseDate(String dateStr)`: `df.parse(dateStr.trim(), pos)` after
    /// resetting the position.
    pub(crate) fn parse_date(&mut self, date_str: &str) -> Result<Option<Date>> {
        self.pos.index = 0;
        self.pos.error_index = -1;
        self.df.parse_position(jdk::java_trim(date_str), &mut self.pos)
    }
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.DirContentSource.Iterator.Comparator`
/// (`static class Comparator implements java.util.Comparator<Path>`).
pub struct Comparator;

impl Comparator {
    /// `compare(Path _a, Path _b)`: the paths' strings, the shorter one
    /// left-padded with `'0'`, compared reversed (because we're going to
    /// push, which reverses again).
    pub fn compare(&self, a: &Path, b: &Path) -> Ordering {
        let mut a: Vec<u16> = a.to_string_lossy().encode_utf16().collect();
        let mut b: Vec<u16> = b.to_string_lossy().encode_utf16().collect();
        let diff = a.len() as i64 - b.len() as i64;
        if diff > 0 {
            let mut padded = vec![u16::from(b'0'); diff as usize];
            padded.extend_from_slice(&b);
            b = padded;
        } else if diff < 0 {
            let mut padded = vec![u16::from(b'0'); (-diff) as usize];
            padded.extend_from_slice(&a);
            a = padded;
        }
        b.cmp(&a)
    }
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.DirContentSource.Iterator`
/// (`public static class Iterator implements java.util.Iterator<Path>`):
/// iterator over the files in the directory.
pub struct Iterator {
    count: i32,
    stack: Vec<PathBuf>,
    c: Comparator,
}

impl Iterator {
    /// `Iterator(Path f)`.
    ///
    /// # Errors
    ///
    /// The `IOException` of listing the directories.
    pub fn new(f: &Path) -> Result<Self> {
        let mut it = Iterator { count: 0, stack: Vec::new(), c: Comparator };
        it.push(f)?;
        Ok(it)
    }

    /// `find()`.
    fn find(&mut self) -> Result<()> {
        let Some(top) = self.stack.last() else {
            return Ok(());
        };
        if !top.is_dir() {
            return Ok(());
        }
        let f = self.stack.pop().expect("checked above");
        self.push(&f)
    }

    /// `push(Path f)`.
    fn push(&mut self, f: &Path) -> Result<()> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(f)? {
            let p = entry?.path();
            if p.is_dir() {
                found.push(p);
            }
        }
        self.push_all(found);

        // `Files.newDirectoryStream(f, "*.txt")`
        let mut found = Vec::new();
        for entry in std::fs::read_dir(f)? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().ends_with(".txt") {
                found.push(entry.path());
            }
        }
        self.push_all(found);

        self.find()
    }

    /// `push(Path[] files)`.
    fn push_all(&mut self, mut files: Vec<PathBuf>) {
        files.sort_by(|a, b| self.c.compare(a, b));
        for f in files {
            self.stack.push(f);
        }
    }

    /// `getCount()`.
    pub fn get_count(&self) -> i32 {
        self.count
    }

    /// `hasNext()`.
    pub fn has_next(&self) -> bool {
        !self.stack.is_empty()
    }

    /// `next()`.
    ///
    /// # Errors
    ///
    /// A `RuntimeException` wrapping the `IOException` of listing; Java's
    /// `ArrayDeque.pop()` `NoSuchElementException` when empty.
    pub fn next(&mut self) -> Result<PathBuf> {
        self.count += 1;
        let object = self
            .stack
            .pop()
            .ok_or_else(|| LuceneError::unchecked_with_message("java.util.NoSuchElementException", None))?;
        self.find().map_err(jdk::runtime_exception_from)?;
        Ok(object)
    }

    /// `remove()`.
    ///
    /// # Errors
    ///
    /// Always `RuntimeException("cannot")`.
    pub fn remove(&mut self) -> Result<()> {
        Err(jdk::runtime_exception("cannot"))
    }
}

#[derive(Default)]
struct State {
    data_dir: Option<PathBuf>,
    iteration: i32,
    input_files: Option<Iterator>,
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.DirContentSource`
/// (`public class DirContentSource extends ContentSource`): a
/// [`ContentSource`] using the Dir collection for its input. Supports the
/// following configuration parameters (on top of `ContentSource`):
///
/// * **work.dir** - specifies the working directory. Required if "docs.dir"
///   denotes a relative path (**default=work**).
/// * **docs.dir** - specifies the directory the Dir collection. Can be set to
///   a relative path if "work.dir" is also specified (**default=dir-out**).
#[derive(Default)]
pub struct DirContentSource {
    base: ContentItemsSourceBase,
    date_format: ThreadLocal<DateFormatInfo>,
    state: Mutex<State>,
}

impl DirContentSource {
    /// `new DirContentSource()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `getDateFormatInfo()`: get/initiate a thread-local simple date format
    /// (must do so because SimpleDateFormat is not thread-safe).
    fn get_date_format_info(&self) -> Result<Arc<Mutex<DateFormatInfo>>> {
        match self.date_format.get() {
            Some(dfi) => Ok(dfi),
            None => Ok(self.date_format.set(Some(DateFormatInfo::new()?)).expect("just set")),
        }
    }
}

/// Reads a Reuters/Dir collection file: first line is the date, 3rd is the
/// title, rest is body (each line followed by a space).
pub(crate) fn read_collection_file(f: &Path) -> Result<(Option<String>, Option<String>, String)> {
    let mut reader = jdk::new_buffered_reader_utf8(f)?;
    let result = (|| -> Result<(Option<String>, Option<String>, String)> {
        let date_str = jdk::read_line(&mut reader)?;
        jdk::read_line(&mut reader)?; // skip an empty line
        let title = jdk::read_line(&mut reader)?;
        jdk::read_line(&mut reader)?; // skip an empty line
        let mut body_buf = String::with_capacity(1024);
        while let Some(line) = jdk::read_line(&mut reader)? {
            body_buf.push_str(&line);
            body_buf.push(' ');
        }
        Ok((date_str, title, body_buf))
    })();
    let closed = reader.close();
    let value = result?;
    closed?;
    Ok(value)
}

impl ContentItemsSource for DirContentSource {
    fn items_base(&self) -> &ContentItemsSourceBase {
        &self.base
    }

    fn java_simple_name(&self) -> &'static str {
        "DirContentSource"
    }

    fn close(&self) -> Result<()> {
        jdk::lock(&self.state).input_files = None;
        Ok(())
    }

    fn reset_inputs(&self) -> Result<()> {
        let mut s = jdk::lock(&self.state);
        self.base.reset_inputs();
        let data_dir = s.data_dir.clone().ok_or_else(LuceneError::null_pointer)?;
        s.input_files = Some(Iterator::new(&data_dir)?);
        s.iteration = 0;
        Ok(())
    }

    fn set_config(&self, config: Arc<Config>) -> Result<()> {
        self.base.set_config(Arc::clone(&config))?;
        let work_dir = PathBuf::from(config.get_str("work.dir", Some("work"))?.unwrap_or_default());
        let d = config.get_str("docs.dir", Some("dir-out"))?.unwrap_or_default();
        let mut data_dir = PathBuf::from(&d);
        if !data_dir.is_absolute() {
            data_dir = work_dir.join(&d);
        }
        let input_files = Iterator::new(&data_dir).map_err(jdk::runtime_exception_from)?;
        let mut s = jdk::lock(&self.state);
        s.data_dir = Some(data_dir);
        s.input_files = Some(input_files);
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ContentSource for DirContentSource {
    fn get_next_doc_data(&self, doc_data: &mut DocData) -> Result<()> {
        let (f, name) = {
            let mut s = jdk::lock(&self.state);
            let has_next = s.input_files.as_ref().ok_or_else(LuceneError::null_pointer)?.has_next();
            if !has_next {
                // exhausted files, start a new round, unless forever set to false.
                if !self.base.forever() {
                    return Err(NoMoreDataException::new());
                }
                let data_dir = s.data_dir.clone().ok_or_else(LuceneError::null_pointer)?;
                s.input_files = Some(Iterator::new(&data_dir)?);
                s.iteration += 1;
            }
            let f = s.input_files.as_mut().expect("set above").next()?;
            // System.err.println(f);
            let name = format!("{}_{}", jdk::real_path_string(&f)?, s.iteration);
            (f, name)
        };

        let (date_str, title, body_buf) = read_collection_file(&f)?;
        self.base.add_bytes(std::fs::metadata(&f)?.len() as i64);

        let date_str = date_str.ok_or_else(LuceneError::null_pointer)?;
        let dfi = self.get_date_format_info()?;
        let date = jdk::lock(&dfi).parse_date(&date_str)?;
        doc_data.clear();
        doc_data.set_name(Some(name));
        doc_data.set_body(Some(body_buf));
        doc_data.set_title(title);
        doc_data.set_date_from(date.as_ref());
        Ok(())
    }
}
