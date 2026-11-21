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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextDocValuesFormat`.

use rucene::codecs::{
    DocValuesConsumer, DocValuesFormat, DocValuesProducer, SegmentReadState, SegmentWriteState,
};
use rucene::error::Result;

use super::doc_values_reader::SimpleTextDocValuesReader;
use super::doc_values_writer::SimpleTextDocValuesWriter;

/// Plain-text doc values format, named `SimpleText`.
///
/// **FOR RECREATIONAL USE ONLY**
///
/// The `.dat` file holds, for each field, its header (name, type, document
/// count and the patterns below), then one fixed-width entry per document,
/// then its skip data:
///
/// * numeric: a delta from `origin`, zero-padded to `pattern`, and a `T`/`F`
///   line telling whether the document has a value;
/// * binary: `length ` and the zero-padded length, the value as
///   `BytesRef.toString()` padded with spaces to `maxlength`, and the
///   `T`/`F` line;
/// * sorted and sorted set: every distinct value as a binary entry, then one
///   ord (sorted, as `ord + 1`) or comma-separated ord list (sorted set) per
///   document;
/// * sorted numeric: a binary field whose values are the comma-separated
///   numbers.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextDocValuesFormat`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SimpleTextDocValuesFormat;

impl SimpleTextDocValuesFormat {
    /// Creates the format.
    pub fn new() -> Self {
        Self
    }
}

impl DocValuesFormat for SimpleTextDocValuesFormat {
    fn name(&self) -> &str {
        "SimpleText"
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn DocValuesConsumer + 'a>> {
        Ok(Box::new(SimpleTextDocValuesWriter::new(state, "dat")?))
    }

    fn fields_producer<'a>(
        &self,
        state: &SegmentReadState<'a>,
    ) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(SimpleTextDocValuesReader::new(state, "dat")?))
    }
}
