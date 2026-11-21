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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextPostingsFormat`.

use rucene::codecs::{
    FieldsConsumer, FieldsProducer, PostingsFormat, SegmentReadState, SegmentWriteState,
};
use rucene::error::Result;
use rucene::index::index_file_names::segment_file_name;

use super::fields_reader::SimpleTextFieldsReader;
use super::fields_writer::SimpleTextFieldsWriter;

/// Extension of freq postings file.
pub(crate) const POSTINGS_EXTENSION: &str = "pst";

/// For debugging only!
///
/// Stores all postings data in a single human-readable text file (`_N.pst`).
/// You can view this in any text editor, and even edit it to alter your
/// index.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextPostingsFormat`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SimpleTextPostingsFormat;

impl SimpleTextPostingsFormat {
    /// Creates the format, named `SimpleText`.
    pub fn new() -> Self {
        Self
    }
}

impl PostingsFormat for SimpleTextPostingsFormat {
    fn name(&self) -> &str {
        "SimpleText"
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        Ok(Box::new(SimpleTextFieldsWriter::new(state)?))
    }

    fn fields_producer<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn FieldsProducer>> {
        Ok(Box::new(SimpleTextFieldsReader::new(state)?))
    }
}

/// Port of `SimpleTextPostingsFormat.getPostingsFileName`.
pub(crate) fn get_postings_file_name(segment: &str, segment_suffix: &str) -> String {
    segment_file_name(segment, segment_suffix, POSTINGS_EXTENSION)
}
