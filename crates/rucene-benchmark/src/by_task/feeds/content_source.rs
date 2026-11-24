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

//! The types declared in `ContentSource.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use rucene::error::Result;

use crate::by_task::feeds::content_items_source::ContentItemsSource;
use crate::by_task::feeds::doc_data::DocData;

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.ContentSource`
/// (`public abstract class ContentSource extends ContentItemsSource`):
/// represents content from a specified source, such as TREC, Reuters etc. A
/// `ContentSource` is responsible for creating [`DocData`] objects for its
/// documents to be consumed by
/// [`DocMaker`](crate::by_task::feeds::doc_maker::DocMaker). It also keeps
/// track of various statistics, such as how many documents were generated,
/// size in bytes etc.
///
/// For supported configuration parameters see [`ContentItemsSource`].
pub trait ContentSource: ContentItemsSource {
    /// `getNextDocData(DocData docData)`: fills `doc_data` with the next
    /// document of the content source (Java returns the filled object).
    /// Implementations must account for multi-threading, as multiple threads
    /// can call this method simultaneously.
    ///
    /// # Errors
    ///
    /// [`NoMoreDataException`](crate::by_task::feeds::no_more_data_exception::NoMoreDataException)
    /// when the source is exhausted; the `IOException` of reading.
    fn get_next_doc_data(&self, doc_data: &mut DocData) -> Result<()>;
}
