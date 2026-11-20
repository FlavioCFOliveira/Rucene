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

//! Port of `DocFreqValueSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::{LeafReaderContext, Term};
use rucene::search::index_searcher::IndexSearcher;
use rucene::util::java_collections::java_string_hash_code;
use rucene::util::java_fmt::double_to_string;
use rucene::util::BytesRef;

use crate::function::docvalues::double_doc_values::{double_doc_values_members, DoubleDocValues};
use crate::function::docvalues::int_doc_values::{int_doc_values_members, IntDocValues};
use crate::function::function_values::FunctionValues;
use crate::function::value_source::{class_hash, ValueSource, ValueSourceContext};

/// `DocFreqValueSource` returns the number of documents containing the term.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.function.valuesource.DocFreqValueSource`.
/// `IDFValueSource` and `TermFreqValueSource` extend it; in this port they
/// hold one as their base and reuse its state and members.
#[derive(Debug, Clone)]
pub struct DocFreqValueSource {
    /// The protected `field` field.
    pub(crate) field: String,
    /// The protected `indexedField` field.
    pub(crate) indexed_field: String,
    /// The protected `val` field.
    pub(crate) val: String,
    /// The protected `indexedBytes` field.
    pub(crate) indexed_bytes: BytesRef,
}

impl DocFreqValueSource {
    /// Equivalent to `DocFreqValueSource(String, String, String, BytesRef)`.
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

    /// Equivalent to `DocFreqValueSource.name()`.
    pub fn name(&self) -> String {
        "docfreq".to_string()
    }

    /// `name + '(' + field + ',' + val + ')'`, the `description()` of this
    /// class and its subclasses, each with its own `name()`.
    pub(crate) fn description_with(&self, name: &str) -> String {
        format!("{name}({},{})", self.field, self.val)
    }

    /// `getClass().hashCode() + indexedField.hashCode() * 29 +
    /// indexedBytes.hashCode()`, the `hashCode()` of this class and its
    /// subclasses, each with its own class.
    pub(crate) fn hash_with(&self, java_class_name: &str) -> i32 {
        class_hash(java_class_name)
            .wrapping_add(java_string_hash_code(&self.indexed_field).wrapping_mul(29))
            .wrapping_add(self.indexed_bytes.hash_code())
    }

    /// `this.indexedField.equals(other.indexedField) &&
    /// this.indexedBytes.equals(other.indexedBytes)`, once the classes are
    /// known to be the same.
    pub(crate) fn fields_eq(&self, other: &DocFreqValueSource) -> bool {
        self.indexed_field == other.indexed_field && self.indexed_bytes == other.indexed_bytes
    }

    /// `(IndexSearcher) context.get("searcher")`, dereferenced: Java's `null`
    /// is the `NullPointerException` of the dereference.
    pub(crate) fn searcher(context: &ValueSourceContext<'_>) -> Result<IndexSearcher> {
        context.searcher().ok_or_else(LuceneError::null_pointer)
    }

    /// The term whose statistics are read.
    pub(crate) fn term(&self) -> Term {
        Term::new(self.indexed_field.clone(), self.indexed_bytes.clone())
    }
}

impl ValueSource for DocFreqValueSource {
    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        _reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let searcher = Self::searcher(context)?;
        let docfreq = searcher.get_index_reader().doc_freq(&self.term())?;
        Ok(Box::new(ConstIntDocValues::new(docfreq, self)))
    }

    /// `context.put("searcher", searcher)`.
    fn create_weight(
        &self,
        context: &ValueSourceContext<'_>,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        context.put_searcher(searcher);
        Ok(())
    }

    fn description(&self) -> String {
        self.description_with(&self.name())
    }

    fn value_source_hash(&self) -> i32 {
        self.hash_with("org.apache.lucene.queries.function.valuesource.DocFreqValueSource")
    }

    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other
            .as_any()
            .downcast_ref::<DocFreqValueSource>()
            .is_some_and(|other| self.fields_eq(other))
    }
}

/// Integer values that are the same for every document.
///
/// Equivalent to the package-private static class
/// `org.apache.lucene.queries.function.valuesource.DocFreqValueSource.ConstIntDocValues`.
pub(crate) struct ConstIntDocValues {
    ival: i32,
    fval: f32,
    dval: f64,
    lval: i64,
    sval: String,
    parent: Arc<dyn ValueSource>,
}

impl ConstIntDocValues {
    /// Equivalent to `ConstIntDocValues(int, ValueSource)`.
    pub(crate) fn new(val: i32, parent: Arc<dyn ValueSource>) -> Self {
        Self {
            ival: val,
            fval: val as f32,
            dval: f64::from(val),
            lval: i64::from(val),
            sval: val.to_string(),
            parent,
        }
    }
}

impl FunctionValues for ConstIntDocValues {
    fn float_val(&mut self, _doc: i32) -> Result<f32> {
        Ok(self.fval)
    }

    fn int_val(&mut self, _doc: i32) -> Result<i32> {
        Ok(self.ival)
    }

    fn long_val(&mut self, _doc: i32) -> Result<i64> {
        Ok(self.lval)
    }

    fn double_val(&mut self, _doc: i32) -> Result<f64> {
        Ok(self.dval)
    }

    fn str_val(&mut self, _doc: i32) -> Result<Option<String>> {
        Ok(Some(self.sval.clone()))
    }

    /// Equivalent to `ConstIntDocValues.toString(int)`.
    fn to_doc_string(&mut self, _doc: i32) -> Result<String> {
        Ok(format!("{}={}", self.parent.description(), self.sval))
    }

    int_doc_values_members!(
        byte_val,
        short_val,
        object_val,
        get_range_matcher,
        get_value_filler
    );
}

impl IntDocValues for ConstIntDocValues {
    fn vs(&self) -> &dyn ValueSource {
        &*self.parent
    }
}

/// Double values that are the same for every document.
///
/// Equivalent to the package-private static class
/// `org.apache.lucene.queries.function.valuesource.DocFreqValueSource.ConstDoubleDocValues`.
pub(crate) struct ConstDoubleDocValues {
    ival: i32,
    fval: f32,
    dval: f64,
    lval: i64,
    sval: String,
    parent: Arc<dyn ValueSource>,
}

impl ConstDoubleDocValues {
    /// Equivalent to `ConstDoubleDocValues(double, ValueSource)`.
    pub(crate) fn new(val: f64, parent: Arc<dyn ValueSource>) -> Self {
        Self {
            ival: val as i32,
            fval: val as f32,
            dval: val,
            lval: val as i64,
            sval: double_to_string(val),
            parent,
        }
    }
}

impl FunctionValues for ConstDoubleDocValues {
    fn float_val(&mut self, _doc: i32) -> Result<f32> {
        Ok(self.fval)
    }

    fn int_val(&mut self, _doc: i32) -> Result<i32> {
        Ok(self.ival)
    }

    fn long_val(&mut self, _doc: i32) -> Result<i64> {
        Ok(self.lval)
    }

    fn double_val(&mut self, _doc: i32) -> Result<f64> {
        Ok(self.dval)
    }

    fn str_val(&mut self, _doc: i32) -> Result<Option<String>> {
        Ok(Some(self.sval.clone()))
    }

    /// Equivalent to `ConstDoubleDocValues.toString(int)`.
    fn to_doc_string(&mut self, _doc: i32) -> Result<String> {
        Ok(format!("{}={}", self.parent.description(), self.sval))
    }

    double_doc_values_members!(
        byte_val,
        short_val,
        bool_val,
        object_val,
        get_range_matcher,
        get_value_filler
    );
}

impl DoubleDocValues for ConstDoubleDocValues {
    fn vs(&self) -> &dyn ValueSource {
        &*self.parent
    }
}
