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

//! The types declared in `HTMLParser.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use rucene::error::Result;
use rucene_queryparser::jdk::date::Date;

use crate::by_task::feeds::doc_data::DocData;
use crate::by_task::feeds::trec_content_source::TrecContentSource;

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.HTMLParser`
/// (`public interface HTMLParser`): HTML parsing interface for test
/// purposes.
pub trait HTMLParser: Send + Sync {
    /// `parse(DocData docData, String name, Date date, Reader reader,
    /// TrecContentSource trecSrc)`: parse the input text and fill
    /// `doc_data`. The provided name, title, date are used for the result,
    /// unless when they're `None`, in which case an attempt is made to set
    /// them from the parsed data. `trec_src` is the [`TrecContentSource`]
    /// used to parse dates.
    ///
    /// # Errors
    ///
    /// If there is a low-level I/O error.
    fn parse(
        &self,
        doc_data: &mut DocData,
        name: Option<&str>,
        date: Option<&Date>,
        reader: &str,
        trec_src: Option<&TrecContentSource>,
    ) -> Result<()>;
}
