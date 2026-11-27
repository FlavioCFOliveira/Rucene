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

//! Port of `TermVectorOffsetStrategy.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReader;

use crate::highlight::term_vector_leaf_reader::TermVectorLeafReader;
use crate::uhighlight::field_offset_strategy::FieldOffsetStrategy;
use crate::uhighlight::offsets_enum::{empty, OffsetsEnum};
use crate::uhighlight::overlay_single_doc_terms_leaf_reader::OverlaySingleDocTermsLeafReader;
use crate::uhighlight::uh_components::UHComponents;
use crate::uhighlight::unified_highlighter::OffsetSource;

/// Uses term vectors that contain offsets.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.TermVectorOffsetStrategy`.
#[derive(Debug)]
pub struct TermVectorOffsetStrategy {
    components: UHComponents,
}

impl TermVectorOffsetStrategy {
    /// Equivalent to `new TermVectorOffsetStrategy(UHComponents)`.
    pub fn new(components: UHComponents) -> Self {
        Self { components }
    }
}

impl FieldOffsetStrategy for TermVectorOffsetStrategy {
    fn components(&self) -> Option<&UHComponents> {
        Some(&self.components)
    }

    fn get_offset_source(&self) -> OffsetSource {
        OffsetSource::TERM_VECTORS
    }

    fn get_offsets_enum(
        &self,
        reader: Option<&Arc<dyn LeafReader>>,
        doc_id: i32,
        _content: &str,
    ) -> Result<Box<dyn OffsetsEnum>> {
        let reader = reader.ok_or_else(LuceneError::null_pointer)?;
        let Some(tv_terms) = reader
            .term_vectors()?
            .get_field(doc_id, self.get_field()?)?
        else {
            return Ok(empty());
        };

        let single_doc_reader: Arc<dyn LeafReader> =
            Arc::new(TermVectorLeafReader::new(self.get_field()?, tv_terms));
        let overlay: Arc<dyn LeafReader> = Arc::new(OverlaySingleDocTermsLeafReader::new(
            Arc::clone(reader),
            single_doc_reader,
            self.get_field()?,
            doc_id,
        )?);
        self.create_offsets_enum_from_reader(&overlay, doc_id)
    }
}
