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

//! Port of `PostingsWithTermVectorsOffsetStrategy.java`, of the Java
//! package `org.apache.lucene.search.uhighlight`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReader;

use crate::uhighlight::field_offset_strategy::FieldOffsetStrategy;
use crate::uhighlight::offsets_enum::{empty, OffsetsEnum};
use crate::uhighlight::term_vector_filtered_leaf_reader::TermVectorFilteredLeafReader;
use crate::uhighlight::uh_components::UHComponents;
use crate::uhighlight::unified_highlighter::OffsetSource;

/// Like `PostingsOffsetStrategy` but also uses term vectors (only terms
/// needed) for multi-term queries.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.PostingsWithTermVectorsOffsetStrategy`.
#[derive(Debug)]
pub struct PostingsWithTermVectorsOffsetStrategy {
    components: UHComponents,
}

impl PostingsWithTermVectorsOffsetStrategy {
    /// Equivalent to `new PostingsWithTermVectorsOffsetStrategy(UHComponents)`.
    pub fn new(components: UHComponents) -> Self {
        Self { components }
    }
}

impl FieldOffsetStrategy for PostingsWithTermVectorsOffsetStrategy {
    fn components(&self) -> Option<&UHComponents> {
        Some(&self.components)
    }

    fn get_offsets_enum(
        &self,
        leaf_reader: Option<&Arc<dyn LeafReader>>,
        doc_id: i32,
        _content: &str,
    ) -> Result<Box<dyn OffsetsEnum>> {
        let leaf_reader = leaf_reader.ok_or_else(LuceneError::null_pointer)?;
        let Some(doc_terms) = leaf_reader
            .term_vectors()?
            .get_field(doc_id, self.get_field()?)?
        else {
            return Ok(empty());
        };
        let leaf_reader: Arc<dyn LeafReader> = Arc::new(TermVectorFilteredLeafReader::new(
            Arc::clone(leaf_reader),
            doc_terms,
            self.get_field()?,
        )?);

        self.create_offsets_enum_from_reader(&leaf_reader, doc_id)
    }

    fn get_offset_source(&self) -> OffsetSource {
        OffsetSource::POSTINGS_WITH_TERM_VECTORS
    }
}
