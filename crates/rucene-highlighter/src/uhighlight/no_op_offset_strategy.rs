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

//! Port of `NoOpOffsetStrategy.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use std::collections::HashSet;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReader;
use rucene::search::match_no_docs_query::MatchNoDocsQuery;

use crate::uhighlight::field_offset_strategy::FieldOffsetStrategy;
use crate::uhighlight::offsets_enum::{empty, OffsetsEnum};
use crate::uhighlight::phrase_helper::PhraseHelper;
use crate::uhighlight::uh_components::UHComponents;
use crate::uhighlight::unified_highlighter::OffsetSource;

/// Never returns offsets. Used when the query would highlight nothing.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.NoOpOffsetStrategy`.
#[derive(Debug)]
pub struct NoOpOffsetStrategy {
    components: UHComponents,
}

impl NoOpOffsetStrategy {
    /// Equivalent to `NoOpOffsetStrategy.INSTANCE`.
    pub fn instance() -> Self {
        Self {
            components: UHComponents::new(
                "_ignored_",
                Arc::new(|_| false),
                Arc::new(MatchNoDocsQuery::instance()),
                Some(Vec::new()),
                PhraseHelper::none(),
                Some(Vec::new()),
                false,
                HashSet::new(),
            ),
        }
    }
}

impl FieldOffsetStrategy for NoOpOffsetStrategy {
    fn components(&self) -> Option<&UHComponents> {
        Some(&self.components)
    }

    fn get_offset_source(&self) -> OffsetSource {
        OffsetSource::NONE_NEEDED
    }

    fn get_offsets_enum(
        &self,
        _reader: Option<&Arc<dyn LeafReader>>,
        _doc_id: i32,
        _content: &str,
    ) -> Result<Box<dyn OffsetsEnum>> {
        Ok(empty())
    }
}