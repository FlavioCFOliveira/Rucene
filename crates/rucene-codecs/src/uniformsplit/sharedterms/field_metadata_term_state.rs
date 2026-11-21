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

//! Port of `org.apache.lucene.codecs.uniformsplit.sharedterms.FieldMetadataTermState`.

use rucene::codecs::BlockTermState;

/// Pair of `FieldMetadata` and [`BlockTermState`] for a specific field.
///
/// Port of the record
/// `org.apache.lucene.codecs.uniformsplit.sharedterms.FieldMetadataTermState`
/// (`@lucene.experimental`). Java's record references the field's
/// `FieldMetadata`, which the terms writer also updates; the port's
/// `field_metadata` is the index of that metadata in the list the
/// [`STUniformSplitTermsWriter`](super::STUniformSplitTermsWriter) keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldMetadataTermState {
    /// The index of the field's `FieldMetadata` in the writer's list.
    pub field_metadata: usize,
    /// The term's state for the field.
    pub state: BlockTermState,
}

impl FieldMetadataTermState {
    /// Port of the record's canonical constructor.
    pub fn new(field_metadata: usize, state: BlockTermState) -> Self {
        Self {
            field_metadata,
            state,
        }
    }

    /// Port of the record accessor `fieldMetadata()`.
    pub fn field_metadata(&self) -> usize {
        self.field_metadata
    }

    /// Port of the record accessor `state()`.
    pub fn state(&self) -> &BlockTermState {
        &self.state
    }
}
