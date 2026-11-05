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

//! Port of `org.apache.lucene.tests.index.AllDeletedFilterReader`: filters
//! the incoming reader and makes all documents appear deleted.

use std::fmt;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::index_reader::{CacheHelper, IndexReaderCore};
use rucene::index::LeafReader;
use rucene::util::{Bits, MatchNoBits};

/// Port of `org.apache.lucene.tests.index.AllDeletedFilterReader`
/// (`extends FilterLeafReader`): every document of the wrapped reader is
/// deleted (`getLiveDocs()` is a `Bits.MatchNoBits` of `maxDoc` bits,
/// `numDocs()` is 0), and the reader has no reader cache helper, since its
/// live documents differ from the wrapped reader's.
pub struct AllDeletedFilterReader {
    core: IndexReaderCore,
    r#in: Arc<dyn LeafReader>,
    live_docs: MatchNoBits,
}

impl AllDeletedFilterReader {
    /// `new AllDeletedFilterReader(in)`.
    ///
    /// # Errors
    ///
    /// The `AssertionError` of `assert maxDoc() == 0 || hasDeletions()`.
    pub fn new(r#in: Arc<dyn LeafReader>) -> Result<Self> {
        let live_docs = MatchNoBits::new(r#in.max_doc() as usize);
        let reader = AllDeletedFilterReader {
            core: super::filter_leaf_core(&r#in),
            r#in,
            live_docs,
        };
        java_assert!(reader.max_doc() == 0 || reader.max_doc() - reader.num_docs() > 0);
        Ok(reader)
    }

    /// `getDelegate()`: the wrapped reader.
    pub fn get_delegate(&self) -> &Arc<dyn LeafReader> {
        &self.r#in
    }
}

impl fmt::Debug for AllDeletedFilterReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AllDeletedFilterReader")
            .finish_non_exhaustive()
    }
}

impl fmt::Display for AllDeletedFilterReader {
    /// The inherited `FilterLeafReader.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FilterLeafReader({})", self.r#in)
    }
}

impl LeafReader for AllDeletedFilterReader {
    forward_leaf_reader!(filter r#in;
        core,
        filter_leaf_reader_delegate,
        term_vectors,
        max_doc,
        stored_fields,
        do_close,
        terms,
        get_numeric_doc_values,
        get_binary_doc_values,
        get_sorted_doc_values,
        get_sorted_numeric_doc_values,
        get_sorted_set_doc_values,
        get_norm_values,
        get_doc_values_skipper,
        get_float_vector_values,
        get_byte_vector_values,
        search_nearest_vectors,
        search_nearest_vectors_byte,
        get_field_infos,
        get_point_values,
        check_integrity,
        get_meta_data,
    );

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        Some(Box::new(self.live_docs))
    }

    fn num_docs(&self) -> i32 {
        0
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.r#in.get_core_cache_helper()
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
}
