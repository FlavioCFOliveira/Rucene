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

//! The types declared in `DocData.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use rucene::document::{DateTools, Resolution};
use rucene_queryparser::jdk::date::Date;

use crate::jdk;
use crate::jdk::properties::Properties;

/// A Java String set on a [`DocData`]: Rust text, or UTF-16 code units.
///
/// PERF: a source that sets Rust text (`LongToEnglishContentSource`, every
/// parser of `String`s) keeps it as it is, and its UTF-16 units are made only
/// for a caller that reads them, once; the indexing path reads the text, so
/// it is never encoded to UTF-16 and back.
#[derive(Clone, Debug, Default)]
pub(crate) struct JavaText {
    /// The text as set, when it was set as Rust text.
    text: Option<String>,
    /// The UTF-16 code units: as set, or made from `text` on first read.
    units: std::sync::OnceLock<Vec<u16>>,
}

impl JavaText {
    fn from_text(text: String) -> Self {
        Self {
            text: Some(text),
            units: std::sync::OnceLock::new(),
        }
    }

    fn from_units(units: Vec<u16>) -> Self {
        Self {
            text: None,
            units: std::sync::OnceLock::from(units),
        }
    }

    /// The UTF-16 code units.
    fn units(&self) -> &[u16] {
        self.units.get_or_init(|| {
            self.text
                .as_deref()
                .unwrap_or_default()
                .encode_utf16()
                .collect()
        })
    }

    /// The text converted to UTF-8 as `UnicodeUtil.UTF16toUTF8` converts it.
    fn to_text(&self) -> String {
        match &self.text {
            Some(text) => text.clone(),
            None => jdk::utf16_to_string(self.units()),
        }
    }

    /// The UTF-16 code units, taken.
    fn into_units(mut self) -> Vec<u16> {
        match self.units.take() {
            Some(units) => units,
            None => self.text.unwrap_or_default().encode_utf16().collect(),
        }
    }

    /// The Rust text when the string was set as one, else `None`.
    pub(crate) fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// The text taken: as set when it was set as Rust text, else converted
    /// from the UTF-16 units as `UnicodeUtil.UTF16toUTF8` converts them.
    pub(crate) fn into_text(self) -> String {
        match self.text {
            Some(text) => text,
            None => jdk::utf16_to_string(self.units()),
        }
    }

    /// Whether the string is empty, `isEmpty()`.
    pub(crate) fn is_empty(&self) -> bool {
        match (&self.text, self.units.get()) {
            (_, Some(units)) => units.is_empty(),
            (Some(text), None) => text.is_empty(),
            (None, None) => true,
        }
    }

    /// Whether `String.length()`, in UTF-16 code units, is at most `n`.
    /// Text of at most `n` bytes is (a UTF-8 byte sequence is never shorter
    /// than its UTF-16 units); longer text is encoded, once, to count.
    pub(crate) fn utf16_len_at_most(&self, n: usize) -> bool {
        match (&self.text, self.units.get()) {
            (_, Some(units)) => units.len() <= n,
            (Some(text), None) if text.len() <= n => true,
            _ => self.units().len() <= n,
        }
    }

    /// The UTF-16 code units, taken.
    pub(crate) fn take_units(self) -> Vec<u16> {
        self.into_units()
    }
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.DocData`: output
/// of parsing (e.g. HTML parsing) of an input document.
///
/// `body` and `title` are Java Strings held as UTF-16 code units, so an
/// unpaired surrogate a source produces (e.g. `DemoHTMLParser` on `&#xD800;`)
/// is kept as Java keeps it; it becomes U+FFFD only where Java converts the
/// String to UTF-8 (`UnicodeUtil.UTF16toUTF8` when the field is indexed, see
/// [`jdk::utf16_to_string`]).
#[derive(Clone, Debug, Default)]
pub struct DocData {
    name: Option<String>,
    body: Option<JavaText>,
    title: Option<JavaText>,
    date: Option<String>,
    id: i32,
    props: Option<Properties>,
}

impl DocData {
    /// `new DocData()`: every field `null`, `id` 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// `clear()`.
    pub fn clear(&mut self) {
        self.name = None;
        self.body = None;
        self.title = None;
        self.date = None;
        self.props = None;
        self.id = -1;
    }

    /// `getBody()`: the Java String as UTF-16 code units.
    pub fn get_body(&self) -> Option<&[u16]> {
        self.body.as_ref().map(JavaText::units)
    }

    /// `getBody()` converted to UTF-8 by `UnicodeUtil.UTF16toUTF8` (an
    /// unpaired surrogate becomes U+FFFD), for consumers that take Rust text.
    pub fn get_body_string(&self) -> Option<String> {
        self.body.as_ref().map(JavaText::to_text)
    }

    /// `getDate()`: the date. If the setter with a `Date` was called, then
    /// the String returned is the output of `DateTools.dateToString(Date,
    /// Resolution.SECOND)`. Otherwise it's the String passed to the other
    /// setter.
    pub fn get_date(&self) -> Option<&str> {
        self.date.as_deref()
    }

    /// `getName()`.
    pub fn get_name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// `getID()`.
    pub fn get_id(&self) -> i32 {
        self.id
    }

    /// `getProps()`.
    pub fn get_props(&self) -> Option<&Properties> {
        self.props.as_ref()
    }

    /// `getTitle()`: the Java String as UTF-16 code units.
    pub fn get_title(&self) -> Option<&[u16]> {
        self.title.as_ref().map(JavaText::units)
    }

    /// `getTitle()` converted to UTF-8 by `UnicodeUtil.UTF16toUTF8` (an
    /// unpaired surrogate becomes U+FFFD), for consumers that take Rust text.
    pub fn get_title_string(&self) -> Option<String> {
        self.title.as_ref().map(JavaText::to_text)
    }

    /// `setBody(String)` with Rust text.
    pub fn set_body(&mut self, body: Option<String>) {
        self.body = body.map(JavaText::from_text);
    }

    /// `setBody(String)` with a Java String held as UTF-16 code units.
    pub fn set_body_utf16(&mut self, body: Option<Vec<u16>>) {
        self.body = body.map(JavaText::from_units);
    }

    /// Moves the body out, leaving `None`, for a caller that replaces it
    /// right after (`DocMaker.createDocument`'s `docData.setBody(...)`).
    pub(crate) fn take_body_utf16(&mut self) -> Option<Vec<u16>> {
        self.body.take().map(JavaText::into_units)
    }

    /// Moves the body out as it was set, leaving `None`.
    pub(crate) fn take_body(&mut self) -> Option<JavaText> {
        self.body.take()
    }

    /// The body as set, or `None` for a `null` body.
    pub(crate) fn body(&self) -> Option<&JavaText> {
        self.body.as_ref()
    }

    /// `setDate(Date)`.
    pub fn set_date_from(&mut self, date: Option<&Date>) {
        match date {
            Some(date) => self.set_date(Some(DateTools::time_to_string(
                date.get_time(),
                Resolution::SECOND,
            ))),
            None => self.date = None,
        }
    }

    /// `setDate(String)`.
    pub fn set_date(&mut self, date: Option<String>) {
        self.date = date;
    }

    /// `setName(String)`.
    pub fn set_name(&mut self, name: Option<String>) {
        self.name = name;
    }

    /// `setID(int)`.
    pub fn set_id(&mut self, id: i32) {
        self.id = id;
    }

    /// `setProps(Properties)`.
    pub fn set_props(&mut self, props: Option<Properties>) {
        self.props = props;
    }

    /// `setTitle(String)` with Rust text.
    pub fn set_title(&mut self, title: Option<String>) {
        self.title = title.map(JavaText::from_text);
    }

    /// `setTitle(String)` with a Java String held as UTF-16 code units.
    pub fn set_title_utf16(&mut self, title: Option<Vec<u16>>) {
        self.title = title.map(JavaText::from_units);
    }
}
