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

//! Port of `org.apache.lucene.codecs.uniformsplit.sharedterms.UnionFieldMetadataBuilder`.

use rucene::error::{LuceneError, Result};
use rucene::util::BytesRef;

use crate::uniformsplit::field_metadata::FieldMetadata;

/// Builds a [`FieldMetadata`] that is the union of multiple
/// [`FieldMetadata`].
///
/// Port of `org.apache.lucene.codecs.uniformsplit.sharedterms.UnionFieldMetadataBuilder`
/// (`@lucene.experimental`).
#[derive(Debug, Clone)]
pub struct UnionFieldMetadataBuilder {
    dictionary_start_fp: i64,
    min_start_block_fp: i64,
    max_end_block_fp: i64,
    max_last_term: Option<BytesRef>,
}

impl Default for UnionFieldMetadataBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl UnionFieldMetadataBuilder {
    /// Port of `UnionFieldMetadataBuilder()`.
    pub fn new() -> Self {
        Self {
            dictionary_start_fp: -1,
            min_start_block_fp: i64::MAX,
            max_end_block_fp: i64::MIN,
            max_last_term: None,
        }
    }

    /// Port of `UnionFieldMetadataBuilder.addFieldMetadata(FieldMetadata)`.
    pub fn add_field_metadata(&mut self, field_metadata: &FieldMetadata) -> &mut Self {
        debug_assert!(
            self.dictionary_start_fp == -1
                || self.dictionary_start_fp == field_metadata.get_dictionary_start_fp()
        );
        self.dictionary_start_fp = field_metadata.get_dictionary_start_fp();
        self.min_start_block_fp = self
            .min_start_block_fp
            .min(field_metadata.get_first_block_start_fp());
        self.max_end_block_fp = self
            .max_end_block_fp
            .max(field_metadata.get_last_block_start_fp());
        let last_term = field_metadata.get_last_term();
        let replace = match (&self.max_last_term, last_term) {
            (None, _) => true,
            (Some(max), Some(last)) => max < last,
            // Java's `compareTo(null)` raises a NullPointerException; the
            // reader's metadata always has a last term.
            (Some(_), None) => false,
        };
        if replace {
            self.max_last_term = last_term.cloned();
        }
        self
    }

    /// Port of `UnionFieldMetadataBuilder.build()`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` when no field metadata was provided.
    pub fn build(&self) -> Result<FieldMetadata> {
        let Some(max_last_term) = &self.max_last_term else {
            return Err(LuceneError::illegal_state("no field metadata was provided"));
        };
        Ok(FieldMetadata::virtual_field(
            self.dictionary_start_fp,
            self.min_start_block_fp,
            self.max_end_block_fp,
            max_last_term.clone(),
        ))
    }
}
