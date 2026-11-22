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

//! Port of `MultiFieldsOffsetStrategy.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReader;

use crate::uhighlight::field_offset_strategy::FieldOffsetStrategy;
use crate::uhighlight::offsets_enum::{is_empty_offsets_enum, MultiOffsetsEnum, OffsetsEnum};
use crate::uhighlight::uh_components::UHComponents;
use crate::uhighlight::unified_highlighter::OffsetSource;

/// FieldOffsetStrategy that combines offsets from multiple fields. Used to
/// highlight a single field based on matches from multiple fields.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.MultiFieldsOffsetStrategy`.
#[derive(Debug)]
pub struct MultiFieldsOffsetStrategy {
    fields_offset_strategies: Vec<Box<dyn FieldOffsetStrategy>>,
}

impl MultiFieldsOffsetStrategy {
    /// Equivalent to `new MultiFieldsOffsetStrategy(List)`.
    pub fn new(fields_offset_strategies: Vec<Box<dyn FieldOffsetStrategy>>) -> Self {
        Self {
            fields_offset_strategies,
        }
    }
}

impl FieldOffsetStrategy for MultiFieldsOffsetStrategy {
    fn components(&self) -> Option<&UHComponents> {
        None
    }

    fn get_field(&self) -> Result<&str> {
        Err(LuceneError::illegal_state(
            "MultiFieldsOffsetStrategy does not have a single field.",
        ))
    }

    /// # Panics
    ///
    /// When there is no strategy, where Java throws
    /// `NoSuchElementException`.
    fn get_offset_source(&self) -> OffsetSource {
        // TODO: what should be returned here as offset source?
        self.fields_offset_strategies
            .first()
            .expect("NoSuchElementException")
            .get_offset_source()
    }

    fn get_offsets_enum(
        &self,
        reader: Option<&Arc<dyn LeafReader>>,
        doc_id: i32,
        content: &str,
    ) -> Result<Box<dyn OffsetsEnum>> {
        let mut fields_offsets_enums: Vec<Box<dyn OffsetsEnum>> =
            Vec::with_capacity(self.fields_offset_strategies.len());
        for field_offset_strategy in &self.fields_offset_strategies {
            let offsets_enum = field_offset_strategy.get_offsets_enum(reader, doc_id, content)?;
            if !is_empty_offsets_enum(&*offsets_enum) {
                fields_offsets_enums.push(offsets_enum);
            }
        }
        Ok(Box::new(MultiOffsetsEnum::new(fields_offsets_enums)?))
    }
}