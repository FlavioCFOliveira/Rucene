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

//! Port of `VectorValueSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;

use crate::function::function_values::FunctionValues;
use crate::function::value_source::{sources_eq, sources_hash, ValueSource, ValueSourceContext};
use crate::function::valuesource::multi_value_source::MultiValueSource;

/// `vals[i] = value`, with the `ArrayIndexOutOfBoundsException` Java raises
/// for an array too short.
fn set_at<T>(vals: &mut [T], i: usize, value: T) -> Result<()> {
    let len = vals.len();
    match vals.get_mut(i) {
        Some(slot) => {
            *slot = value;
            Ok(())
        }
        None => Err(LuceneError::unchecked(
            "java.lang.ArrayIndexOutOfBoundsException",
            format!("Index {i} out of bounds for length {len}"),
        )),
    }
}

/// Converts individual value sources to a multi-valued representation.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.VectorValueSource`.
#[derive(Debug, Clone)]
pub struct VectorValueSource {
    /// The protected `sources` field.
    sources: Vec<Arc<dyn ValueSource>>,
}

impl VectorValueSource {
    /// Equivalent to `VectorValueSource(List<ValueSource>)`.
    pub fn new(sources: Vec<Arc<dyn ValueSource>>) -> Self {
        Self { sources }
    }

    /// Equivalent to `VectorValueSource.getSources()`.
    pub fn get_sources(&self) -> &[Arc<dyn ValueSource>] {
        &self.sources
    }

    /// Equivalent to `VectorValueSource.name()`.
    pub fn name(&self) -> String {
        "vector".to_string()
    }
}

impl MultiValueSource for VectorValueSource {
    fn dimension(&self) -> i32 {
        self.sources.len() as i32
    }
}

/// The anonymous `FunctionValues` of `VectorValueSource.getValues` for two
/// sources.
struct Vector2Values<'a> {
    this: Arc<VectorValueSource>,
    x: Box<dyn FunctionValues + 'a>,
    y: Box<dyn FunctionValues + 'a>,
}

impl FunctionValues for Vector2Values<'_> {
    fn byte_vals(&mut self, doc: i32, vals: &mut [i8]) -> Result<()> {
        set_at(vals, 0, self.x.byte_val(doc)?)?;
        set_at(vals, 1, self.y.byte_val(doc)?)
    }

    fn short_vals(&mut self, doc: i32, vals: &mut [i16]) -> Result<()> {
        set_at(vals, 0, self.x.short_val(doc)?)?;
        set_at(vals, 1, self.y.short_val(doc)?)
    }

    fn int_vals(&mut self, doc: i32, vals: &mut [i32]) -> Result<()> {
        set_at(vals, 0, self.x.int_val(doc)?)?;
        set_at(vals, 1, self.y.int_val(doc)?)
    }

    fn long_vals(&mut self, doc: i32, vals: &mut [i64]) -> Result<()> {
        set_at(vals, 0, self.x.long_val(doc)?)?;
        set_at(vals, 1, self.y.long_val(doc)?)
    }

    fn float_vals(&mut self, doc: i32, vals: &mut [f32]) -> Result<()> {
        set_at(vals, 0, self.x.float_val(doc)?)?;
        set_at(vals, 1, self.y.float_val(doc)?)
    }

    fn double_vals(&mut self, doc: i32, vals: &mut [f64]) -> Result<()> {
        set_at(vals, 0, self.x.double_val(doc)?)?;
        set_at(vals, 1, self.y.double_val(doc)?)
    }

    fn str_vals(&mut self, doc: i32, vals: &mut [Option<String>]) -> Result<()> {
        set_at(vals, 0, self.x.str_val(doc)?)?;
        set_at(vals, 1, self.y.str_val(doc)?)
    }

    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        Ok(format!(
            "{}({},{})",
            self.this.name(),
            self.x.to_doc_string(doc)?,
            self.y.to_doc_string(doc)?
        ))
    }
}

/// The anonymous `FunctionValues` of `VectorValueSource.getValues` for any
/// other number of sources.
struct VectorValues<'a> {
    this: Arc<VectorValueSource>,
    vals_arr: Vec<Box<dyn FunctionValues + 'a>>,
}

impl FunctionValues for VectorValues<'_> {
    fn byte_vals(&mut self, doc: i32, vals: &mut [i8]) -> Result<()> {
        for i in 0..self.vals_arr.len() {
            let v = self.vals_arr[i].byte_val(doc)?;
            set_at(vals, i, v)?;
        }
        Ok(())
    }

    fn short_vals(&mut self, doc: i32, vals: &mut [i16]) -> Result<()> {
        for i in 0..self.vals_arr.len() {
            let v = self.vals_arr[i].short_val(doc)?;
            set_at(vals, i, v)?;
        }
        Ok(())
    }

    fn float_vals(&mut self, doc: i32, vals: &mut [f32]) -> Result<()> {
        for i in 0..self.vals_arr.len() {
            let v = self.vals_arr[i].float_val(doc)?;
            set_at(vals, i, v)?;
        }
        Ok(())
    }

    fn int_vals(&mut self, doc: i32, vals: &mut [i32]) -> Result<()> {
        for i in 0..self.vals_arr.len() {
            let v = self.vals_arr[i].int_val(doc)?;
            set_at(vals, i, v)?;
        }
        Ok(())
    }

    fn long_vals(&mut self, doc: i32, vals: &mut [i64]) -> Result<()> {
        for i in 0..self.vals_arr.len() {
            let v = self.vals_arr[i].long_val(doc)?;
            set_at(vals, i, v)?;
        }
        Ok(())
    }

    fn double_vals(&mut self, doc: i32, vals: &mut [f64]) -> Result<()> {
        for i in 0..self.vals_arr.len() {
            let v = self.vals_arr[i].double_val(doc)?;
            set_at(vals, i, v)?;
        }
        Ok(())
    }

    fn str_vals(&mut self, doc: i32, vals: &mut [Option<String>]) -> Result<()> {
        for i in 0..self.vals_arr.len() {
            let v = self.vals_arr[i].str_val(doc)?;
            set_at(vals, i, v)?;
        }
        Ok(())
    }

    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        let mut sb = self.this.name();
        sb.push('(');
        let mut first_time = true;
        for vals in &mut self.vals_arr {
            if first_time {
                first_time = false;
            } else {
                sb.push(',');
            }
            sb.push_str(&vals.to_doc_string(doc)?);
        }
        sb.push(')');
        Ok(sb)
    }
}

impl ValueSource for VectorValueSource {
    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let size = self.sources.len();

        // special-case x,y and lat,lon since it's so common
        if size == 2 {
            let x = Arc::clone(&self.sources[0]).get_values(context, reader_context)?;
            let y = Arc::clone(&self.sources[1]).get_values(context, reader_context)?;
            return Ok(Box::new(Vector2Values { this: self, x, y }));
        }

        let mut vals_arr = Vec::with_capacity(size);
        for source in &self.sources {
            vals_arr.push(Arc::clone(source).get_values(context, reader_context)?);
        }
        Ok(Box::new(VectorValues {
            this: self,
            vals_arr,
        }))
    }

    fn create_weight(
        &self,
        context: &ValueSourceContext<'_>,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        for source in &self.sources {
            source.create_weight(context, searcher)?;
        }
        Ok(())
    }

    fn description(&self) -> String {
        let mut sb = self.name();
        sb.push('(');
        let mut first_time = true;
        for source in &self.sources {
            if first_time {
                first_time = false;
            } else {
                sb.push(',');
            }
            sb.push_str(&source.to_string());
        }
        sb.push(')');
        sb
    }

    /// `o instanceof VectorValueSource` with equal sources.
    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other
            .as_any()
            .downcast_ref::<VectorValueSource>()
            .is_some_and(|that| sources_eq(&self.sources, &that.sources))
    }

    /// `sources.hashCode()`.
    fn value_source_hash(&self) -> i32 {
        sources_hash(&self.sources)
    }
}
