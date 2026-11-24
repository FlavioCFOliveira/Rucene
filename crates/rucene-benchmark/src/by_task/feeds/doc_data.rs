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
    body: Option<Vec<u16>>,
    title: Option<Vec<u16>>,
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
        self.body.as_deref()
    }

    /// `getBody()` converted to UTF-8 by `UnicodeUtil.UTF16toUTF8` (an
    /// unpaired surrogate becomes U+FFFD), for consumers that take Rust text.
    pub fn get_body_string(&self) -> Option<String> {
        self.body.as_deref().map(jdk::utf16_to_string)
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
        self.title.as_deref()
    }

    /// `getTitle()` converted to UTF-8 by `UnicodeUtil.UTF16toUTF8` (an
    /// unpaired surrogate becomes U+FFFD), for consumers that take Rust text.
    pub fn get_title_string(&self) -> Option<String> {
        self.title.as_deref().map(jdk::utf16_to_string)
    }

    /// `setBody(String)` with Rust text.
    pub fn set_body(&mut self, body: Option<String>) {
        self.body = body.map(|b| b.encode_utf16().collect());
    }

    /// `setBody(String)` with a Java String held as UTF-16 code units.
    pub fn set_body_utf16(&mut self, body: Option<Vec<u16>>) {
        self.body = body;
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
        self.title = title.map(|t| t.encode_utf16().collect());
    }

    /// `setTitle(String)` with a Java String held as UTF-16 code units.
    pub fn set_title_utf16(&mut self, title: Option<Vec<u16>>) {
        self.title = title;
    }
}
