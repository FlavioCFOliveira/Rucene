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

//! Port of `IndexBuilder.java`: utility class for building an ephemeral
//! document index and running a block of code on its reader.
//!
//! As in Java, the `IndexWriter` is not closed: the reader is opened from it
//! (near-real-time), and the `ByteBuffersDirectory` is closed after the
//! block.

use std::sync::Arc;

use randomizedtesting::RandomizedTest;
use rucene::analysis::Analyzer;
use rucene::document::{Document, NumericDocValuesField, Store, StringField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::{DirectoryReader, IndexableField};
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::store::{ByteBuffersDirectory, Directory};

/// `IndexBuilder.FLD_ID`.
pub const FLD_ID: &str = "id";
/// `IndexBuilder.FLD_SORT_ORDER`.
pub const FLD_SORT_ORDER: &str = "id_order";

/// The `BiFunction<String, String, IndexableField> valueToField`.
pub type ToField = Arc<dyn Fn(&str, &str) -> Arc<dyn IndexableField> + Send + Sync>;

/// `class IndexBuilder`.
pub struct IndexBuilder {
    to_field: ToField,
    documents: Vec<Document>,
    seq: i32,
}

/// `IndexBuilder.DocFields`.
pub struct DocFields<'a> {
    document: &'a mut Document,
    to_field: &'a ToField,
}

impl DocFields<'_> {
    /// `add(field, values...)`.
    pub fn add(&mut self, field: &str, values: &[&str]) {
        assert!(!values.is_empty(), "At least one field value is required.");
        for value in values {
            self.document.add_shared((self.to_field)(field, value));
        }
    }
}

impl IndexBuilder {
    /// `new IndexBuilder(valueToField)`.
    pub fn new(value_to_field: ToField) -> Self {
        Self {
            to_field: value_to_field,
            documents: Vec::new(),
            seq: 0,
        }
    }

    /// `doc(field, values...)`.
    pub fn doc(self, field: &str, values: &[&str]) -> Self {
        self.doc_with(|fields| fields.add(field, values))
    }

    /// `doc(Consumer<DocFields> fields)`.
    pub fn doc_with(mut self, fields: impl FnOnce(&mut DocFields<'_>)) -> Self {
        let mut doc = Document::new();
        doc.add(NumericDocValuesField::new(FLD_SORT_ORDER, self.seq as i64));
        doc.add(
            StringField::new(FLD_ID, self.seq.to_string(), Store::YES)
                .expect("INVARIANT: a valid string field"),
        );
        self.seq += 1;
        {
            let mut doc_fields = DocFields {
                document: &mut doc,
                to_field: &self.to_field,
            };
            fields(&mut doc_fields);
        }
        self.documents.push(doc);
        self
    }

    /// `build(analyzer, block)`.
    pub fn build(
        self,
        analyzer: Arc<dyn Analyzer>,
        block: impl FnOnce(Arc<dyn DirectoryReader>) -> Result<()>,
    ) -> Result<Self> {
        let config = IndexWriterConfig::with_analyzer(Some(analyzer));
        config.set_index_sort(Sort::new_fields(vec![SortField::new(
            Some(FLD_SORT_ORDER.to_string()),
            SortFieldType::Long,
        )?])?)?;
        let directory: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let result = (|| -> Result<()> {
            let iw = IndexWriter::new(Arc::clone(&directory), config)?;
            for doc in &self.documents {
                iw.add_document(doc)?;
            }
            if RandomizedTest::random_boolean() {
                iw.commit()?;
            }
            iw.flush()?;

            let reader = directory_reader::open_with_writer(&*iw)?;
            let ran = block(Arc::clone(&reader));
            let closed = reader.close();
            ran?;
            closed
        })();
        let closed = directory.close();
        result?;
        closed?;
        Ok(self)
    }
}
