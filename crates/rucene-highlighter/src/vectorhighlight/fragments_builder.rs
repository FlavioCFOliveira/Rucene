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

//! Port of `FragmentsBuilder.java`, of the Java package
//! `org.apache.lucene.search.vectorhighlight`.

use std::fmt::Debug;

use rucene::error::Result;
use rucene::index::IndexReader;

use crate::highlight::encoder::Encoder;
use crate::vectorhighlight::field_frag_list::FieldFragList;

/// `FragmentsBuilder` is an interface for fragments (snippets) builder
/// classes. A `FragmentsBuilder` class can be plugged in to
/// `FastVectorHighlighter`.
///
/// Equivalent to `org.apache.lucene.search.vectorhighlight.FragmentsBuilder`.
/// The frag list is lent mutably, as Java's builders sort and edit the list
/// they are given.
pub trait FragmentsBuilder: Debug + Send + Sync {
    /// create a fragment; `None` (Java's `null`) when the document has no
    /// value for the field.
    ///
    /// Equivalent to `createFragment(IndexReader, int, String,
    /// FieldFragList)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    fn create_fragment(
        &self,
        reader: &dyn IndexReader,
        doc_id: i32,
        field_name: &str,
        field_frag_list: &mut dyn FieldFragList,
    ) -> Result<Option<String>>;

    /// create multiple fragments.
    ///
    /// Equivalent to `createFragments(IndexReader, int, String,
    /// FieldFragList, int)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    fn create_fragments(
        &self,
        reader: &dyn IndexReader,
        doc_id: i32,
        field_name: &str,
        field_frag_list: &mut dyn FieldFragList,
        max_num_fragments: i32,
    ) -> Result<Option<Vec<String>>>;

    /// create a fragment.
    ///
    /// Equivalent to `createFragment(IndexReader, int, String,
    /// FieldFragList, String[], String[], Encoder)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    #[allow(clippy::too_many_arguments)]
    fn create_fragment_with_tags(
        &self,
        reader: &dyn IndexReader,
        doc_id: i32,
        field_name: &str,
        field_frag_list: &mut dyn FieldFragList,
        pre_tags: &[String],
        post_tags: &[String],
        encoder: &dyn Encoder,
    ) -> Result<Option<String>>;

    /// create multiple fragments.
    ///
    /// Equivalent to `createFragments(IndexReader, int, String,
    /// FieldFragList, int, String[], String[], Encoder)`.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    #[allow(clippy::too_many_arguments)]
    fn create_fragments_with_tags(
        &self,
        reader: &dyn IndexReader,
        doc_id: i32,
        field_name: &str,
        field_frag_list: &mut dyn FieldFragList,
        max_num_fragments: i32,
        pre_tags: &[String],
        post_tags: &[String],
        encoder: &dyn Encoder,
    ) -> Result<Option<Vec<String>>>;
}
