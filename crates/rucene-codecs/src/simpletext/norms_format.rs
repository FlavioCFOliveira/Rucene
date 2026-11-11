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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextNormsFormat`.

use rucene::codecs::{
    DocValuesConsumer, DocValuesProducer, NormsConsumer, NormsFormat, NormsProducer,
    SegmentReadState, SegmentWriteState,
};
use rucene::error::{LuceneError, Result};
use rucene::index::{
    BinaryDocValues, DocValuesSkipper, FieldInfo, NumericDocValues, SortedDocValues,
    SortedNumericDocValues, SortedSetDocValues,
};

use super::doc_values_reader::SimpleTextDocValuesReader;
use super::doc_values_writer::SimpleTextDocValuesWriter;

const NORMS_SEG_EXTENSION: &str = "len";

/// Plain-text norms format.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextNormsFormat`.
///
/// **FOR RECREATIONAL USE ONLY**
#[derive(Debug, Default, Clone, Copy)]
pub struct SimpleTextNormsFormat;

impl SimpleTextNormsFormat {
    /// Creates the format.
    pub fn new() -> Self {
        Self
    }
}

impl NormsFormat for SimpleTextNormsFormat {
    fn name(&self) -> &str {
        "SimpleTextNormsFormat"
    }

    fn norms_consumer(&self, state: &SegmentWriteState) -> Result<Box<dyn NormsConsumer>> {
        Ok(Box::new(SimpleTextNormsConsumer::new(state)?))
    }

    fn norms_producer(&self, state: &SegmentReadState) -> Result<Box<dyn NormsProducer>> {
        Ok(Box::new(SimpleTextNormsProducer::new(state)?))
    }
}

/// Reads plain-text norms.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextNormsFormat.SimpleTextNormsProducer`.
///
/// **FOR RECREATIONAL USE ONLY**
pub struct SimpleTextNormsProducer {
    imp: SimpleTextDocValuesReader,
}

impl SimpleTextNormsProducer {
    /// All we do is change the extension from .dat -> .len; otherwise this is
    /// a normal simple doc values file.
    ///
    /// # Errors
    ///
    /// Propagates [`SimpleTextDocValuesReader::new`]'s errors.
    pub fn new(state: &SegmentReadState<'_>) -> Result<Self> {
        Ok(Self {
            imp: SimpleTextDocValuesReader::new(state, NORMS_SEG_EXTENSION)?,
        })
    }
}

impl std::fmt::Debug for SimpleTextNormsProducer {
    /// `getClass().getSimpleName() + "(" + impl + ")"`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SimpleTextNormsProducer({})", self.imp)
    }
}

impl NormsProducer for SimpleTextNormsProducer {
    fn get_norms(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.imp.get_numeric(field)
    }

    fn check_integrity(&self) -> Result<()> {
        self.imp.check_integrity()
    }

    /// Java's default `getMergeInstance()` returns the producer itself; this
    /// returns a handle that shares its reader.
    fn get_merge_instance(&self) -> Result<Box<dyn NormsProducer>> {
        Ok(Box::new(Self {
            imp: self.imp.clone(),
        }))
    }

    fn close(&self) -> Result<()> {
        self.imp.close()
    }
}

/// Writes plain-text norms.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextNormsFormat.SimpleTextNormsConsumer`.
///
/// **FOR RECREATIONAL USE ONLY**
#[derive(Debug)]
pub struct SimpleTextNormsConsumer {
    imp: SimpleTextDocValuesWriter,
}

impl SimpleTextNormsConsumer {
    /// All we do is change the extension from .dat -> .len; otherwise this is
    /// a normal simple doc values file.
    ///
    /// # Errors
    ///
    /// Propagates [`SimpleTextDocValuesWriter::new`]'s errors.
    pub fn new(state: &SegmentWriteState<'_>) -> Result<Self> {
        Ok(Self {
            imp: SimpleTextDocValuesWriter::new(state, NORMS_SEG_EXTENSION)?,
        })
    }
}

impl NormsConsumer for SimpleTextNormsConsumer {
    fn add_norms_field(
        &mut self,
        field: &FieldInfo,
        norms_producer: &dyn NormsProducer,
    ) -> Result<()> {
        self.imp
            .add_numeric_field(field, &NormsAsDocValues { norms_producer })
    }

    fn close(&mut self) -> Result<()> {
        self.imp.close()
    }
}

/// The anonymous `EmptyDocValuesProducer` of `addNormsField`, whose
/// `getNumeric` returns `normsProducer.getNorms(field)`.
struct NormsAsDocValues<'a> {
    norms_producer: &'a dyn NormsProducer,
}

impl std::fmt::Debug for NormsAsDocValues<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EmptyDocValuesProducer")
    }
}

fn unsupported(method: &str) -> LuceneError {
    LuceneError::unsupported_operation(format!("EmptyDocValuesProducer.{method}"))
}

impl DocValuesProducer for NormsAsDocValues<'_> {
    fn get_numeric(&self, field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        self.norms_producer.get_norms(field)
    }

    fn get_binary(&self, _field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
        Err(unsupported("getBinary"))
    }

    fn get_sorted(&self, _field: &FieldInfo) -> Result<Option<Box<dyn SortedDocValues>>> {
        Err(unsupported("getSorted"))
    }

    fn get_sorted_numeric(
        &self,
        _field: &FieldInfo,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        Err(unsupported("getSortedNumeric"))
    }

    fn get_sorted_set(&self, _field: &FieldInfo) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        Err(unsupported("getSortedSet"))
    }

    fn get_skipper(&self, _field: &FieldInfo) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        Err(unsupported("getSkipper"))
    }

    fn check_integrity(&self) -> Result<()> {
        Err(unsupported("checkIntegrity"))
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        Err(unsupported("getMergeInstance"))
    }

    fn close(&self) -> Result<()> {
        Err(unsupported("close"))
    }
}
