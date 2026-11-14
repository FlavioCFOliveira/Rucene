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

//! Port of `VectorSimilarityFunction.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.
//!
//! The concrete members of the abstract class are the functions of this
//! module, applied to `this`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{LeafReaderContext, VectorSimilarityFunction as IndexVectorSimilarityFunction};
use rucene::util::java_fmt::float_to_string;

use crate::function::function_values::FunctionValues;
use crate::function::value_source::{class_hash, objects_hash, ValueSource, ValueSourceContext};
use crate::function::valuesource::multi_function;

/// The Java name of a similarity function, `similarityFunction.name()`.
pub(crate) fn similarity_name(similarity_function: IndexVectorSimilarityFunction) -> String {
    format!("{similarity_function:?}")
}

/// [`ValueSource`] implementation which wraps two value sources and applies a
/// `VectorSimilarityFunction` between their values.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.valuesource.VectorSimilarityFunction`.
/// Its members are [`get_values`], [`value_source_eq`],
/// [`value_source_hash`] and [`description`].
pub trait VectorSimilarityFunction: ValueSource {
    /// The protected `similarityFunction` field.
    fn similarity_function(&self) -> IndexVectorSimilarityFunction;

    /// The protected `vector1` field.
    fn vector1(&self) -> &Arc<dyn ValueSource>;

    /// The protected `vector2` field.
    fn vector2(&self) -> &Arc<dyn ValueSource>;

    /// Equivalent to the abstract protected
    /// `VectorSimilarityFunction.func(int, FunctionValues, FunctionValues)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading the vectors.
    fn func(
        &self,
        doc: i32,
        f1: &mut dyn FunctionValues,
        f2: &mut dyn FunctionValues,
    ) -> Result<f32>;
}

/// Equivalent to `VectorSimilarityFunction.getValues(Map, LeafReaderContext)`.
///
/// # Errors
///
/// Propagates the error of either source's `get_values`.
pub fn get_values<'a, T: VectorSimilarityFunction>(
    this: Arc<T>,
    context: &ValueSourceContext<'a>,
    reader_context: &LeafReaderContext,
) -> Result<Box<dyn FunctionValues + 'a>> {
    let vector1_vals = Arc::clone(this.vector1()).get_values(context, reader_context)?;
    let vector2_vals = Arc::clone(this.vector2()).get_values(context, reader_context)?;
    Ok(Box::new(VectorSimilarityValues {
        this,
        vector1_vals,
        vector2_vals,
    }))
}

/// The anonymous `FunctionValues` of `VectorSimilarityFunction.getValues`.
struct VectorSimilarityValues<'a, T: VectorSimilarityFunction> {
    this: Arc<T>,
    vector1_vals: Box<dyn FunctionValues + 'a>,
    vector2_vals: Box<dyn FunctionValues + 'a>,
}

impl<T: VectorSimilarityFunction> FunctionValues for VectorSimilarityValues<'_, T> {
    fn float_val(&mut self, doc: i32) -> Result<f32> {
        self.this
            .func(doc, &mut *self.vector1_vals, &mut *self.vector2_vals)
    }

    fn double_val(&mut self, doc: i32) -> Result<f64> {
        Ok(f64::from(self.float_val(doc)?))
    }

    fn str_val(&mut self, doc: i32) -> Result<Option<String>> {
        Ok(Some(float_to_string(self.float_val(doc)?)))
    }

    fn exists(&mut self, doc: i32) -> Result<bool> {
        multi_function::all_exists2(doc, &mut *self.vector1_vals, &mut *self.vector2_vals)
    }

    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        let value = self.str_val(doc)?;
        Ok(format!(
            "{} = {}",
            self.this.description(),
            value.as_deref().unwrap_or("null")
        ))
    }
}

/// Equivalent to `VectorSimilarityFunction.equals(Object)`: the same class and
/// equal vectors; the similarity function takes no part.
pub fn value_source_eq<T: VectorSimilarityFunction>(this: &T, other: &dyn ValueSource) -> bool {
    other.as_any().downcast_ref::<T>().is_some_and(|other| {
        this.vector1().value_source_eq(&**other.vector1())
            && this.vector2().value_source_eq(&**other.vector2())
    })
}

/// Equivalent to `VectorSimilarityFunction.hashCode()`:
/// `Objects.hash(similarityFunction, vector1, vector2)`, the enum constant's
/// identity hash being the stand-in of `class_hash`.
pub fn value_source_hash<T: VectorSimilarityFunction + ?Sized>(this: &T) -> i32 {
    objects_hash(&[
        class_hash(&format!(
            "org.apache.lucene.index.VectorSimilarityFunction${}",
            similarity_name(this.similarity_function())
        )),
        this.vector1().value_source_hash(),
        this.vector2().value_source_hash(),
    ])
}

/// Equivalent to `VectorSimilarityFunction.description()`:
/// `similarityFunction.name() + "(" + vector1.description() + ", " +
/// vector2.description() + ")"`.
pub fn description<T: VectorSimilarityFunction + ?Sized>(this: &T) -> String {
    format!(
        "{}({}, {})",
        similarity_name(this.similarity_function()),
        this.vector1().description(),
        this.vector2().description()
    )
}
