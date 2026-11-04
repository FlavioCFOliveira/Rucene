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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextTermVectorsFormat`.

use std::sync::Arc;

use rucene::codecs::{TermVectorsFormat, TermVectorsReader, TermVectorsWriter};
use rucene::error::Result;
use rucene::index::{FieldInfos, SegmentInfo};
use rucene::store::{Directory, IOContext};

use super::term_vectors_reader::SimpleTextTermVectorsReader;
use super::term_vectors_writer::SimpleTextTermVectorsWriter;

/// Plain text term vectors format.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextTermVectorsFormat`.
///
/// **FOR RECREATIONAL USE ONLY**
#[derive(Debug, Default, Clone, Copy)]
pub struct SimpleTextTermVectorsFormat;

impl SimpleTextTermVectorsFormat {
    /// Creates the format.
    pub fn new() -> Self {
        Self
    }
}

impl TermVectorsFormat for SimpleTextTermVectorsFormat {
    fn name(&self) -> &str {
        "SimpleTextTermVectorsFormat"
    }

    fn vectors_reader(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        _field_infos: &FieldInfos,
        context: &dyn IOContext,
    ) -> Result<Box<dyn TermVectorsReader>> {
        Ok(Box::new(SimpleTextTermVectorsReader::new(
            directory,
            segment_info,
            context,
        )?))
    }

    fn vectors_writer(
        &self,
        directory: &Arc<dyn Directory>,
        segment_info: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<Box<dyn TermVectorsWriter>> {
        Ok(Box::new(SimpleTextTermVectorsWriter::new(
            directory.as_ref(),
            &segment_info.name,
            context,
        )?))
    }
}
