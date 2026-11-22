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

//! Port of `PostingsOffsetStrategy.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReader;

use crate::uhighlight::field_offset_strategy::FieldOffsetStrategy;
use crate::uhighlight::offsets_enum::OffsetsEnum;
use crate::uhighlight::uh_components::UHComponents;
use crate::uhighlight::unified_highlighter::OffsetSource;

/// Uses offsets in postings -- `IndexOptions.DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS`.
/// This does not support multi-term queries; the highlighter will fallback
/// to analysis for that.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.PostingsOffsetStrategy`.
#[derive(Debug)]
pub struct PostingsOffsetStrategy {
    components: UHComponents,
}

impl PostingsOffsetStrategy {
    /// Equivalent to `new PostingsOffsetStrategy(UHComponents)`.
    pub fn new(components: UHComponents) -> Self {
        Self { components }
    }
}

impl FieldOffsetStrategy for PostingsOffsetStrategy {
    fn components(&self) -> Option<&UHComponents> {
        Some(&self.components)
    }

    fn get_offsets_enum(
        &self,
        reader: Option<&Arc<dyn LeafReader>>,
        doc_id: i32,
        _content: &str,
    ) -> Result<Box<dyn OffsetsEnum>> {
        let reader = reader.ok_or_else(LuceneError::null_pointer)?;
        self.create_offsets_enum_from_reader(reader, doc_id)
    }

    fn get_offset_source(&self) -> OffsetSource {
        OffsetSource::POSTINGS
    }
}