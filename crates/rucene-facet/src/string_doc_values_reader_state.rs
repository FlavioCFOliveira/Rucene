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

//! Port of `StringDocValuesReaderState.java`, of the Java package
//! `org.apache.lucene.facet`.

use std::fmt;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{DocValues, IndexReader, OrdinalMap};
use rucene::util::packed::PackedInts;

/// Stores an `OrdinalMap` for a field/reader pair so it can be reused across
/// `StringValueFacetCounts` calls.
///
/// Equivalent to `org.apache.lucene.facet.StringDocValuesReaderState`.
pub struct StringDocValuesReaderState {
    pub(crate) reader: Arc<dyn IndexReader>,
    pub(crate) field: String,
    pub(crate) ordinal_map: Option<Arc<OrdinalMap>>,
}

impl StringDocValuesReaderState {
    /// Construct state specific to a reader + field.
    ///
    /// Equivalent to `StringDocValuesReaderState(IndexReader, String)`.
    pub fn new(reader: Arc<dyn IndexReader>, field: &str) -> Result<Self> {
        let ordinal_map = Self::build_ordinal_map(&reader, field)?;
        Ok(Self {
            reader,
            field: field.to_string(),
            ordinal_map: ordinal_map.map(Arc::new),
        })
    }

    /// Equivalent to `buildOrdinalMap(IndexReader, String)`.
    fn build_ordinal_map(reader: &Arc<dyn IndexReader>, field: &str) -> Result<Option<OrdinalMap>> {
        let leaves = Arc::clone(reader).leaves()?;
        let leaf_count = leaves.len();
        if leaf_count <= 1 {
            return Ok(None);
        }
        let mut doc_values = Vec::with_capacity(leaf_count);
        for context in &leaves {
            doc_values.push(DocValues::get_sorted_set(&*context.leaf_reader(), field)?);
        }
        let owner = reader.get_reader_cache_helper().map(|h| h.get_key().clone());
        Ok(Some(OrdinalMap::build_sorted_set(owner, doc_values, PackedInts::DEFAULT)?))
    }
}

impl fmt::Display for StringDocValuesReaderState {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "StringDocValuesReaderState(field={} reader={})",
            self.field, self.reader
        )
    }
}