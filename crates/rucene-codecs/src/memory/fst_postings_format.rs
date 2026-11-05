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

//! Port of `org.apache.lucene.codecs.memory.FSTPostingsFormat`.

use std::fmt;

use rucene::codecs::{
    FieldsConsumer, FieldsProducer, Lucene104PostingsReader, Lucene104PostingsWriter,
    PostingsFormat, SegmentReadState, SegmentWriteState,
};
use rucene::error::Result;

use super::fst_terms_reader::FSTTermsReader;
use super::fst_terms_writer::FSTTermsWriter;

/// FST term dict + Lucene50PBF.
///
/// Port of `org.apache.lucene.codecs.memory.FSTPostingsFormat`, named
/// `FST50`.
///
/// **Divergence.** Java overrides `toString()` to return the bare name.
/// This type's [`fmt::Display`] does; but `PostingsFormat`'s `toString()`
/// is the trait object's `Display` (`impl Display for dyn PostingsFormat`,
/// `PostingsFormat(name=...)`), which a format cannot override, so a
/// `dyn PostingsFormat` holding this format still prints
/// `PostingsFormat(name=FST50)`.
#[derive(Debug, Default, Clone, Copy)]
pub struct FSTPostingsFormat;

impl FSTPostingsFormat {
    /// Port of `FSTPostingsFormat()`.
    pub fn new() -> Self {
        Self
    }
}

impl fmt::Display for FSTPostingsFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl PostingsFormat for FSTPostingsFormat {
    /// `toString()`: this format's [`fmt::Display`].
    fn to_java_string(&self) -> String {
        self.to_string()
    }

    fn name(&self) -> &str {
        "FST50"
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        let postings_writer = Box::new(Lucene104PostingsWriter::new(state)?);
        // `FSTTermsWriter::new` closes the postings writer when it fails.
        Ok(Box::new(FSTTermsWriter::new(state, postings_writer)?))
    }

    fn fields_producer<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn FieldsProducer>> {
        let postings_reader = Box::new(Lucene104PostingsReader::new(state)?);
        // `FSTTermsReader::new` closes the postings reader when it fails.
        Ok(Box::new(FSTTermsReader::new(state, postings_reader)?))
    }
}
