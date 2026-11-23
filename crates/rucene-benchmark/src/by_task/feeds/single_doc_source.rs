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

//! The types declared in `SingleDocSource.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use std::any::Any;
use std::sync::Mutex;

use rucene::error::Result;

use crate::by_task::feeds::content_items_source::{ContentItemsSource, ContentItemsSourceBase};
use crate::by_task::feeds::content_source::ContentSource;
use crate::by_task::feeds::doc_data::DocData;
use crate::by_task::feeds::no_more_data_exception::NoMoreDataException;
use crate::jdk;

/// `DOC_TEXT`.
pub(crate) const DOC_TEXT: &str = concat!(
    "Well, this is just some plain text we use for creating the ",
    "test documents. It used to be a text from an online collection ",
    "devoted to first aid, but if there was there an (online) lawyers ",
    "first aid collection with legal advices, \"it\" might have quite ",
    "probably advised one not to include \"it\"'s text or the text of ",
    "any other online collection in one's code, unless one has money ",
    "that one don't need and one is happy to donate for lawyers ",
    "charity. Anyhow at some point, rechecking the usage of this text, ",
    "it became uncertain that this text is free to use, because ",
    "the web site in the disclaimer of he eBook containing that text ",
    "was not responding anymore, and at the same time, in projGut, ",
    "searching for first aid no longer found that eBook as well. ",
    "So here we are, with a perhaps much less interesting ",
    "text for the test, but oh much much safer. "
);

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.SingleDocSource`:
/// creates the same document each time
/// [`get_next_doc_data`](ContentSource::get_next_doc_data) is called.
#[derive(Debug, Default)]
pub struct SingleDocSource {
    base: ContentItemsSourceBase,
    doc_id: Mutex<i32>,
}

impl SingleDocSource {
    /// `new SingleDocSource()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `newdocid()`: return a new docid.
    fn newdocid(&self) -> Result<i32> {
        let mut doc_id = jdk::lock(&self.doc_id);
        if *doc_id > 0 && !self.base.forever() {
            return Err(NoMoreDataException::new());
        }
        let id = *doc_id;
        *doc_id += 1;
        Ok(id)
    }
}

impl ContentItemsSource for SingleDocSource {
    fn items_base(&self) -> &ContentItemsSourceBase {
        &self.base
    }

    fn java_simple_name(&self) -> &'static str {
        "SingleDocSource"
    }

    fn close(&self) -> Result<()> {
        Ok(())
    }

    fn reset_inputs(&self) -> Result<()> {
        let mut doc_id = jdk::lock(&self.doc_id);
        self.base.reset_inputs();
        *doc_id = 0;
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ContentSource for SingleDocSource {
    fn get_next_doc_data(&self, doc_data: &mut DocData) -> Result<()> {
        let id = self.newdocid()?;
        self.base.add_bytes(jdk::utf16_len(DOC_TEXT) as i64);
        doc_data.clear();
        doc_data.set_name(Some(format!("doc{id}")));
        doc_data.set_body(Some(DOC_TEXT.to_string()));
        Ok(())
    }
}
