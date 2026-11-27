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

//! The types declared in `LongToEnglishContentSource.java`, of the Java
//! package `org.apache.lucene.benchmark.byTask.feeds`.

use std::any::Any;
use std::sync::Mutex;

use rucene::error::Result;
use rucene_queryparser::jdk::date::Date;

use crate::by_task::feeds::content_items_source::{ContentItemsSource, ContentItemsSourceBase};
use crate::by_task::feeds::content_source::ContentSource;
use crate::by_task::feeds::doc_data::DocData;
use crate::icu::RuleBasedNumberFormat;
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.LongToEnglishContentSource`
/// (`public class LongToEnglishContentSource extends ContentSource`): creates
/// documents whose content is a `long` number starting from
/// `Long.MIN_VALUE + 10`.
///
/// The text is icu4j's root `SPELLOUT` of the number (see
/// [`RuleBasedNumberFormat`]).
#[derive(Debug, Default)]
pub struct LongToEnglishContentSource {
    base: ContentItemsSourceBase,
    counter: Mutex<i64>,
    // TODO: we could take param to specify locale...
    rnbf: RuleBasedNumberFormat,
}

impl LongToEnglishContentSource {
    /// `new LongToEnglishContentSource()`: `counter = 0`.
    pub fn new() -> Self {
        LongToEnglishContentSource {
            base: ContentItemsSourceBase::new(),
            counter: Mutex::new(0),
            rnbf: RuleBasedNumberFormat::root_spellout(),
        }
    }
}

impl ContentItemsSource for LongToEnglishContentSource {
    fn items_base(&self) -> &ContentItemsSourceBase {
        &self.base
    }

    fn java_simple_name(&self) -> &'static str {
        "LongToEnglishContentSource"
    }

    fn close(&self) -> Result<()> {
        Ok(())
    }

    fn reset_inputs(&self) -> Result<()> {
        *jdk::lock(&self.counter) = i64::MIN + 10;
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl ContentSource for LongToEnglishContentSource {
    fn get_next_doc_data(&self, doc_data: &mut DocData) -> Result<()> {
        doc_data.clear();
        // store the current counter to avoid synchronization later on
        let cur_counter = {
            let mut counter = jdk::lock(&self.counter);
            let cur = *counter;
            if *counter == i64::MAX {
                *counter = i64::MIN; // loop around
            } else {
                *counter += 1;
            }
            cur
        };

        doc_data.set_body(Some(self.rnbf.format(cur_counter)));
        doc_data.set_name(Some(format!("doc_{cur_counter}")));
        doc_data.set_title(Some(format!("title_{cur_counter}")));
        doc_data.set_date_from(Some(&Date::now()));
        Ok(())
    }
}
