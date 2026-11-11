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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextStoredFieldsFormat`.

use std::sync::Arc;

use rucene::codecs::{StoredFieldsFormat, StoredFieldsReader, StoredFieldsWriter};
use rucene::error::Result;
use rucene::index::{FieldInfos, SegmentInfo};
use rucene::store::{Directory, IOContext};

use super::stored_fields_reader::SimpleTextStoredFieldsReader;
use super::stored_fields_writer::SimpleTextStoredFieldsWriter;

/// Plain text stored fields format.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextStoredFieldsFormat`.
///
/// **FOR RECREATIONAL USE ONLY**
#[derive(Debug, Default, Clone, Copy)]
pub struct SimpleTextStoredFieldsFormat;

impl SimpleTextStoredFieldsFormat {
    /// Creates the format.
    pub fn new() -> Self {
        Self
    }
}

impl StoredFieldsFormat for SimpleTextStoredFieldsFormat {
    fn name(&self) -> &str {
        "SimpleTextStoredFieldsFormat"
    }

    fn fields_reader(
        &self,
        directory: &dyn Directory,
        si: &SegmentInfo,
        fn_: &FieldInfos,
        context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsReader>> {
        Ok(Box::new(SimpleTextStoredFieldsReader::new(
            directory, si, fn_, context,
        )?))
    }

    fn fields_writer(
        &self,
        directory: &Arc<dyn Directory>,
        si: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsWriter>> {
        Ok(Box::new(SimpleTextStoredFieldsWriter::new(
            directory.as_ref(),
            &si.name,
            context,
        )?))
    }
}
