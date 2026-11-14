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

//! Port of `RangeMapFloatFunction.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;
use rucene::util::java_fmt::float_to_string;

use crate::function::docvalues::float_doc_values::{float_doc_values_members, FloatDocValues};
use crate::function::function_values::FunctionValues;
use crate::function::value_source::{float_to_int_bits, ValueSource, ValueSourceContext};
use crate::function::valuesource::const_value_source::ConstValueSource;

/// `RangeMapFloatFunction` implements a map function over another value
/// source whose values fall within `min` and `max` inclusive to `target`;
/// other values map to `default`, or are kept when there is no default.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.RangeMapFloatFunction`.
/// Normally used as an argument to a `FunctionQuery`.
#[derive(Debug, Clone)]
pub struct RangeMapFloatFunction {
    source: Arc<dyn ValueSource>,
    min: f32,
    max: f32,
    target: Arc<dyn ValueSource>,
    /// The protected `defaultVal` field, `None` for `null`.
    default_val: Option<Arc<dyn ValueSource>>,
}

impl RangeMapFloatFunction {
    /// Equivalent to `RangeMapFloatFunction(ValueSource, float, float, float,
    /// Float)`: the target and the default become `ConstValueSource`s.
    pub fn with_floats(
        source: Arc<dyn ValueSource>,
        min: f32,
        max: f32,
        target: f32,
        def: Option<f32>,
    ) -> Self {
        Self::new(
            source,
            min,
            max,
            Arc::new(ConstValueSource::new(target)),
            def.map(|d| Arc::new(ConstValueSource::new(d)) as Arc<dyn ValueSource>),
        )
    }

    /// Equivalent to `RangeMapFloatFunction(ValueSource, float, float,
    /// ValueSource, ValueSource)`.
    pub fn new(
        source: Arc<dyn ValueSource>,
        min: f32,
        max: f32,
        target: Arc<dyn ValueSource>,
        def: Option<Arc<dyn ValueSource>>,
    ) -> Self {
        Self {
            source,
            min,
            max,
            target,
            default_val: def,
        }
    }
}

/// The anonymous `FloatDocValues` of `RangeMapFloatFunction.getValues`.
struct RangeMapFloatValues<'a> {
    this: Arc<RangeMapFloatFunction>,
    vals: Box<dyn FunctionValues + 'a>,
    targets: Box<dyn FunctionValues + 'a>,
    defaults: Option<Box<dyn FunctionValues + 'a>>,
}

impl FunctionValues for RangeMapFloatValues<'_> {
    fn float_val(&mut self, doc: i32) -> Result<f32> {
        let val = self.vals.float_val(doc)?;
        if val >= self.this.min && val <= self.this.max {
            self.targets.float_val(doc)
        } else if self.this.default_val.is_none() {
            Ok(val)
        } else {
            match self.defaults.as_mut() {
                Some(defaults) => defaults.float_val(doc),
                None => Ok(val),
            }
        }
    }

    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        let defaults = match self.defaults.as_mut() {
            None => "null".to_string(),
            Some(defaults) => defaults.to_doc_string(doc)?,
        };
        Ok(format!(
            "map({},min={},max={},target={},defaultVal={})",
            self.vals.to_doc_string(doc)?,
            float_to_string(self.this.min),
            float_to_string(self.this.max),
            self.targets.to_doc_string(doc)?,
            defaults
        ))
    }

    float_doc_values_members!(
        byte_val,
        short_val,
        int_val,
        long_val,
        bool_val,
        double_val,
        str_val,
        object_val,
        get_value_filler
    );
}

impl FloatDocValues for RangeMapFloatValues<'_> {
    fn vs(&self) -> &dyn ValueSource {
        &*self.this
    }
}

impl ValueSource for RangeMapFloatFunction {
    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let vals = Arc::clone(&self.source).get_values(context, reader_context)?;
        let targets = Arc::clone(&self.target).get_values(context, reader_context)?;
        let defaults = match &self.default_val {
            None => None,
            Some(default_val) => Some(Arc::clone(default_val).get_values(context, reader_context)?),
        };
        Ok(Box::new(RangeMapFloatValues {
            this: self,
            vals,
            targets,
            defaults,
        }))
    }

    /// Propagates to the source only, as Java does.
    fn create_weight(
        &self,
        context: &ValueSourceContext<'_>,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        self.source.create_weight(context, searcher)
    }

    fn value_source_hash(&self) -> i32 {
        let float_bits = float_to_int_bits;
        let mut h = self.source.value_source_hash();
        h ^= (h << 10) | ((h as u32) >> 23) as i32;
        h = h.wrapping_add(float_bits(self.min));
        h ^= (h << 14) | ((h as u32) >> 19) as i32;
        h = h.wrapping_add(float_bits(self.max));
        h = h.wrapping_add(self.target.value_source_hash());
        if let Some(default_val) = &self.default_val {
            h = h.wrapping_add(default_val.value_source_hash());
        }
        h
    }

    /// `RangeMapFloatFunction.class == o.getClass()` with equal bounds,
    /// target, source and default (both `null`, or equal).
    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other
            .as_any()
            .downcast_ref::<RangeMapFloatFunction>()
            .is_some_and(|other| {
                self.min == other.min
                    && self.max == other.max
                    && self.target.value_source_eq(&*other.target)
                    && self.source.value_source_eq(&*other.source)
                    && match (&self.default_val, &other.default_val) {
                        (None, None) => true,
                        (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a.value_source_eq(&**b),
                        _ => false,
                    }
            })
    }

    fn description(&self) -> String {
        format!(
            "map({},{},{},{},{})",
            self.source.description(),
            float_to_string(self.min),
            float_to_string(self.max),
            self.target.description(),
            self.default_val
                .as_ref()
                .map_or_else(|| "null".to_string(), |d| d.description())
        )
    }
}
