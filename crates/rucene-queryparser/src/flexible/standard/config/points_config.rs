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

//! Port of `PointsConfig.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.config`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use crate::flexible::core::nodes::query_node_impl::Mutable;
use crate::jdk::class::{JavaClass, DOUBLE, FLOAT, INTEGER, LONG};
use crate::jdk::number_format::NumberFormat;

/// This class holds the configuration used to parse numeric queries and
/// create `PointValues` queries.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.standard.config.PointsConfig`.
/// It is configured by reference, so its setters take `&self`; the number
/// type is its `Class` descriptor.
#[derive(Debug)]
pub struct PointsConfig {
    format: Mutable<Arc<dyn NumberFormat>>,
    type_: Mutable<&'static JavaClass>,
}

impl PointsConfig {
    /// Constructs a [`PointsConfig`] object.
    ///
    /// Equivalent to `new PointsConfig(NumberFormat format, Class<? extends
    /// Number> type)`.
    ///
    /// * `format` - the [`NumberFormat`] used to parse a `String` to
    ///   `Number`
    /// * `type_` - the numeric type used to index the numeric values
    ///
    /// # Errors
    ///
    /// The exceptions of [`set_number_format`](Self::set_number_format) and
    /// [`set_type`](Self::set_type).
    pub fn new(
        format: Option<Arc<dyn NumberFormat>>,
        type_: Option<&'static JavaClass>,
    ) -> Result<Self> {
        let format =
            format.ok_or_else(|| LuceneError::illegal_argument("format must not be null!"))?;
        let config = Self {
            format: Mutable::new(format),
            type_: Mutable::new(&INTEGER),
        };
        config.set_type(type_)?;
        Ok(config)
    }

    /// Returns the [`NumberFormat`] used to parse a `String` to `Number`
    ///
    /// Equivalent to `getNumberFormat()`.
    pub fn get_number_format(&self) -> Arc<dyn NumberFormat> {
        self.format.get()
    }

    /// Returns the numeric type used to index the numeric values
    ///
    /// Equivalent to `getType()`.
    pub fn get_type(&self) -> &'static JavaClass {
        self.type_.get()
    }

    /// Sets the numeric type used to index the numeric values
    ///
    /// Equivalent to `setType(Class<? extends Number> type)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("type must not be null!")` for `None`, and
    /// `IllegalArgumentException("unsupported numeric type: " + type)` for a
    /// class other than `Integer`, `Long`, `Float` and `Double`.
    pub fn set_type(&self, type_: Option<&'static JavaClass>) -> Result<()> {
        let type_ = type_.ok_or_else(|| LuceneError::illegal_argument("type must not be null!"))?;
        if *type_ != INTEGER && *type_ != LONG && *type_ != FLOAT && *type_ != DOUBLE {
            return Err(LuceneError::illegal_argument(format!(
                "unsupported numeric type: {}",
                type_.to_java_string()
            )));
        }
        self.type_.set(type_);
        Ok(())
    }

    /// Sets the [`NumberFormat`] used to parse a `String` to `Number`
    ///
    /// Equivalent to `setNumberFormat(NumberFormat format)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("format must not be null!")` for `None`.
    pub fn set_number_format(&self, format: Option<Arc<dyn NumberFormat>>) -> Result<()> {
        let format =
            format.ok_or_else(|| LuceneError::illegal_argument("format must not be null!"))?;
        self.format.set(format);
        Ok(())
    }

    /// Equivalent to `hashCode()`: `31 * (31 + format.hashCode()) +
    /// type.hashCode()`.
    pub fn hash_code(&self) -> i32 {
        let prime = 31i32;
        let mut result = 1i32;
        result = prime
            .wrapping_mul(result)
            .wrapping_add(self.format.get().java_hash_code());
        result = prime
            .wrapping_mul(result)
            .wrapping_add(self.type_.get().hash_code());
        result
    }

    /// Equivalent to `equals(Object obj)` against another `PointsConfig`:
    /// equal formats and types.
    pub fn equals(&self, other: &PointsConfig) -> bool {
        if std::ptr::eq(self, other) {
            return true;
        }
        if !self.format.get().java_equals(other.format.get().as_ref()) {
            return false;
        }
        if self.type_.get() != other.type_.get() {
            return false;
        }
        true
    }
}
