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

//! Port of `TotalTermFreqValueSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{IndexReaderContext, LeafReaderContext, Term};
use rucene::search::index_searcher::IndexSearcher;
use rucene::util::java_collections::java_string_hash_code;
use rucene::util::BytesRef;

use crate::function::function_values::FunctionValues;
use crate::function::value_source::{class_hash, ValueSource, ValueSourceContext};
use crate::function::valuesource::sum_total_term_freq_value_source::stored_values;

/// `TotalTermFreqValueSource` returns the total term frequency of the field,
/// across all documents in the index.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.TotalTermFreqValueSource`.
/// This value source requires that term frequencies are indexed.
#[derive(Debug, Clone)]
pub struct TotalTermFreqValueSource {
    /// The protected `field` field.
    field: String,
    /// The protected `indexedField` field.
    indexed_field: String,
    /// The protected `val` field.
    val: String,
    /// The protected `indexedBytes` field.
    indexed_bytes: BytesRef,
}

impl TotalTermFreqValueSource {
    /// Equivalent to `TotalTermFreqValueSource(String, String, String,
    /// BytesRef)`.
    pub fn new(
        field: impl Into<String>,
        val: impl Into<String>,
        indexed_field: impl Into<String>,
        indexed_bytes: BytesRef,
    ) -> Self {
        Self {
            field: field.into(),
            val: val.into(),
            indexed_field: indexed_field.into(),
            indexed_bytes,
        }
    }

    /// Equivalent to `TotalTermFreqValueSource.name()`.
    pub fn name(&self) -> String {
        "totaltermfreq".to_string()
    }
}

impl ValueSource for TotalTermFreqValueSource {
    fn description(&self) -> String {
        format!("{}({},{})", self.name(), self.field, self.val)
    }

    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        _reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        stored_values(context, self)
    }

    fn create_weight(
        &self,
        context: &ValueSourceContext<'_>,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        let mut total_term_freq = 0i64;
        let term = Term::new(self.indexed_field.clone(), self.indexed_bytes.clone());
        for reader_context in Arc::clone(searcher.get_top_reader_context()).leaves() {
            let val = IndexReaderContext::reader(&*reader_context).total_term_freq(&term)?;
            debug_assert!(val != -1);
            total_term_freq = total_term_freq.wrapping_add(val);
        }
        let ttf = total_term_freq;
        context.put(self, Arc::new(ttf));
        Ok(())
    }

    /// `getClass().hashCode() + indexedField.hashCode() * 29 +
    /// indexedBytes.hashCode()`.
    fn value_source_hash(&self) -> i32 {
        class_hash("org.apache.lucene.queries.function.valuesource.TotalTermFreqValueSource")
            .wrapping_add(java_string_hash_code(&self.indexed_field).wrapping_mul(29))
            .wrapping_add(self.indexed_bytes.hash_code())
    }

    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other
            .as_any()
            .downcast_ref::<TotalTermFreqValueSource>()
            .is_some_and(|other| {
                self.indexed_field == other.indexed_field
                    && self.indexed_bytes == other.indexed_bytes
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::function::value_source::new_context;
    use rucene::document::{Document, Store, StringField};
    use rucene::error::LuceneErrorKind;
    use rucene::index::directory_reader;
    use rucene::index::index_writer::IndexWriter;
    use rucene::index::index_writer_config::IndexWriterConfig;
    use rucene::store::{ByteBuffersDirectory, Directory};

    /// Regression for rmp #669: before `createWeight`, Java's `getValues`
    /// returns the `null` its context map holds
    /// (`TotalTermFreqValueSource.java:62-65`) and fails only when that
    /// reference is used, with a `NullPointerException`.
    #[test]
    fn get_values_before_create_weight_fails_only_on_use() {
        let _ = rucene::codecs::register_codec("Lucene104", rucene::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        let mut doc = Document::new();
        doc.add(StringField::new("f", "x".to_string(), Store::NO).unwrap());
        writer.add_document(&doc).unwrap();
        writer.commit().unwrap();
        writer.close().unwrap();
        let reader = directory_reader::open(dir).unwrap();
        let leaves = Arc::clone(&reader).leaves().unwrap();

        let vs = Arc::new(TotalTermFreqValueSource::new(
            "f",
            "x",
            "f",
            BytesRef::new(b"x".to_vec()),
        ));
        let context = new_context(None);
        let mut values = vs
            .get_values(&context, &leaves[0])
            .expect("getValues returns the map's null without failing");
        let error = values.long_val(0).unwrap_err();
        assert!(
            matches!(
                error.kind(),
                LuceneErrorKind::Unchecked { class, .. } if *class == "java.lang.NullPointerException"
            ),
            "{error:?}"
        );
        drop(values);
        reader.close().unwrap();
    }
}
