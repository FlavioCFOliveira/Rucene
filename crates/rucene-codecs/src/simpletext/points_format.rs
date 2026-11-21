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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextPointsFormat`.

use rucene::codecs::{
    PointsFormat, PointsReader, PointsWriter, SegmentReadState, SegmentWriteState,
};
use rucene::error::Result;

use super::points_reader::SimpleTextPointsReader;
use super::points_writer::SimpleTextPointsWriter;

/// Extension of points data file.
pub(crate) const POINT_EXTENSION: &str = "dim";

/// Extension of points index file.
pub(crate) const POINT_INDEX_EXTENSION: &str = "dii";

/// For debugging, curiosity, transparency only!! Do not use this codec in
/// production.
///
/// This codec stores all dimensional data in a single human-readable text
/// file (`_N.dim`). You can view this in any text editor, and even edit it to
/// alter your index.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextPointsFormat`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SimpleTextPointsFormat;

impl SimpleTextPointsFormat {
    /// Creates the format.
    pub fn new() -> Self {
        Self
    }
}

impl PointsFormat for SimpleTextPointsFormat {
    fn name(&self) -> &str {
        "SimpleTextPointsFormat"
    }

    fn fields_writer(&self, state: &SegmentWriteState) -> Result<Box<dyn PointsWriter>> {
        Ok(Box::new(SimpleTextPointsWriter::new(state)?))
    }

    fn fields_reader(&self, state: &SegmentReadState) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(SimpleTextPointsReader::new(state)?))
    }
}
