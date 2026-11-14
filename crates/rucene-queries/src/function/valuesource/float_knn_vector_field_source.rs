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

//! Port of `FloatKnnVectorFieldSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{DocIndexIterator, FloatVectorValues, LeafReaderContext, VectorEncoding};
use rucene::search::doc_id_set_iterator::{self, DocIdSetIterator, EmptyDocIdSetIterator};
use rucene::util::java_collections::java_string_hash_code;

use crate::function::function_values::FunctionValues;
use crate::function::value_source::{class_hash, objects_hash, ValueSource, ValueSourceContext};
use crate::function::valuesource::vector_field_function::{self, VectorFieldFunction};

/// An implementation for retrieving [`FunctionValues`] instances for float
/// kNN vector fields.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.FloatKnnVectorFieldSource`.
#[derive(Debug, Clone)]
pub struct FloatKnnVectorFieldSource {
    field_name: String,
}

impl FloatKnnVectorFieldSource {
    /// Equivalent to `FloatKnnVectorFieldSource(String)`.
    pub fn new(field_name: impl Into<String>) -> Self {
        Self {
            field_name: field_name.into(),
        }
    }
}

/// The anonymous `VectorFieldFunction` of `FloatKnnVectorFieldSource.getValues` for a leaf
/// without vectors for the field.
struct EmptyVectorValues {
    this: Arc<FloatKnnVectorFieldSource>,
    last_doc_id: i32,
    empty: EmptyDocIdSetIterator,
}

impl FunctionValues for EmptyVectorValues {
    fn float_vector_val(&mut self, _doc: i32) -> Result<Option<Vec<f32>>> {
        Ok(None)
    }

    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        vector_field_function::to_doc_string(self, doc)
    }

    fn exists(&mut self, doc: i32) -> Result<bool> {
        vector_field_function::exists(self, doc)
    }
}

impl VectorFieldFunction for EmptyVectorValues {
    fn value_source(&self) -> &dyn ValueSource {
        &*self.this
    }

    fn last_doc_id(&mut self) -> &mut i32 {
        &mut self.last_doc_id
    }

    fn get_vector_iterator(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.empty
    }
}

/// The anonymous `VectorFieldFunction` of `FloatKnnVectorFieldSource.getValues` over the
/// leaf's vectors.
struct VectorValues {
    this: Arc<FloatKnnVectorFieldSource>,
    last_doc_id: i32,
    vector_values: Box<dyn FloatVectorValues>,
    iterator: Box<dyn DocIndexIterator>,
}

impl FunctionValues for VectorValues {
    fn float_vector_val(&mut self, doc: i32) -> Result<Option<Vec<f32>>> {
        if self.exists(doc)? {
            Ok(Some(
                self.vector_values.vector_value(self.iterator.index())?,
            ))
        } else {
            Ok(None)
        }
    }

    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        vector_field_function::to_doc_string(self, doc)
    }

    fn exists(&mut self, doc: i32) -> Result<bool> {
        vector_field_function::exists(self, doc)
    }
}

impl VectorFieldFunction for VectorValues {
    fn value_source(&self) -> &dyn ValueSource {
        &*self.this
    }

    fn last_doc_id(&mut self) -> &mut i32 {
        &mut self.last_doc_id
    }

    fn get_vector_iterator(&mut self) -> &mut dyn DocIdSetIterator {
        &mut *self.iterator
    }
}

impl ValueSource for FloatKnnVectorFieldSource {
    fn get_values<'a>(
        self: Arc<Self>,
        _context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let reader = reader_context.leaf_reader();
        let Some(vector_values) = reader.get_float_vector_values(&self.field_name)? else {
            vector_field_function::check_field(
                reader.as_ref(),
                &self.field_name,
                VectorEncoding::FLOAT32,
            )?;
            return Ok(Box::new(EmptyVectorValues {
                this: self,
                last_doc_id: 0,
                empty: doc_id_set_iterator::empty(),
            }));
        };
        let iterator = vector_values.iterator()?;
        Ok(Box::new(VectorValues {
            this: self,
            last_doc_id: 0,
            vector_values,
            iterator,
        }))
    }

    /// The same class and an equal field name.
    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other
            .as_any()
            .downcast_ref::<FloatKnnVectorFieldSource>()
            .is_some_and(|other| self.field_name == other.field_name)
    }

    /// `Objects.hash(getClass().hashCode(), fieldName)`.
    fn value_source_hash(&self) -> i32 {
        objects_hash(&[
            class_hash("org.apache.lucene.queries.function.valuesource.FloatKnnVectorFieldSource"),
            java_string_hash_code(&self.field_name),
        ])
    }

    fn description(&self) -> String {
        format!("FloatKnnVectorFieldSource({})", self.field_name)
    }
}
