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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextKnnVectorsFormat`.

use rucene::codecs::state::OwnedSegmentWriteState;
use rucene::codecs::{
    BufferingKnnVectorsWriter, KnnVectorsFormat, KnnVectorsReader, KnnVectorsWriter,
    SegmentReadState,
};
use rucene::error::Result;

use super::knn_vectors_reader::SimpleTextKnnVectorsReader;
use super::knn_vectors_writer::SimpleTextKnnVectorsWriter;

/// Extension of vectors data file.
pub(crate) const VECTOR_EXTENSION: &str = "vec";

/// Extension of vectors index file.
pub(crate) const META_EXTENSION: &str = "gri";

/// For debugging, curiosity, transparency only!! Do not use this codec in
/// production.
///
/// This codec stores all data in a single human-readable text file
/// (`_N.vec`). You can view this in any text editor, and even edit it to
/// alter your index.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextKnnVectorsFormat`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SimpleTextKnnVectorsFormat;

impl SimpleTextKnnVectorsFormat {
    /// Creates the format, named `SimpleTextKnnVectorsFormat`.
    pub fn new() -> Self {
        Self
    }
}

impl KnnVectorsFormat for SimpleTextKnnVectorsFormat {
    fn name(&self) -> &str {
        "SimpleTextKnnVectorsFormat"
    }

    fn fields_writer(&self, state: &OwnedSegmentWriteState) -> Result<Box<dyn KnnVectorsWriter>> {
        Ok(Box::new(BufferingKnnVectorsWriter::new(
            SimpleTextKnnVectorsWriter::new(state)?,
        )))
    }

    fn fields_reader<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn KnnVectorsReader>> {
        Ok(Box::new(SimpleTextKnnVectorsReader::new(state)?))
    }

    /// `KnnVectorsFormat.DEFAULT_MAX_DIMENSIONS`.
    fn get_max_dimensions(&self, _field_name: &str) -> i32 {
        1024
    }
}
