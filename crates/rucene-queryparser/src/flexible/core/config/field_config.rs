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

//! Port of `FieldConfig.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.config`.

use std::ops::Deref;

use rucene::error::{LuceneError, Result};

use super::abstract_query_config::AbstractQueryConfig;

/// This class represents a field configuration.
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.core.config.FieldConfig`,
/// which extends `AbstractQueryConfig` (it dereferences to it).
#[derive(Debug)]
pub struct FieldConfig {
    config: AbstractQueryConfig,
    field_name: String,
}

impl FieldConfig {
    /// Constructs a [`FieldConfig`]
    ///
    /// Equivalent to `new FieldConfig(String fieldName)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("field name must not be null!")` for a
    /// `None` field name.
    pub fn new(field_name: Option<&str>) -> Result<Self> {
        let field_name = field_name
            .ok_or_else(|| LuceneError::illegal_argument("field name must not be null!"))?;
        Ok(Self {
            config: AbstractQueryConfig::new(),
            field_name: field_name.to_string(),
        })
    }

    /// Returns the field name this configuration represents.
    ///
    /// Equivalent to `getField()`.
    pub fn get_field(&self) -> &str {
        &self.field_name
    }

    /// Equivalent to `toString()`: the name and `super.toString()`, which is
    /// `Object.toString()` (the class name and the identity hash, as
    /// [`java_object_to_string`](rucene::index::index_reader::java_object_to_string)
    /// renders it).
    pub fn to_java_string(&self) -> String {
        format!(
            "<fieldconfig name=\"{}\" configurations=\"{}\"/>",
            self.field_name,
            rucene::index::index_reader::java_object_to_string(
                "org.apache.lucene.queryparser.flexible.core.config.FieldConfig",
                self
            )
        )
    }
}

impl Deref for FieldConfig {
    type Target = AbstractQueryConfig;

    fn deref(&self) -> &AbstractQueryConfig {
        &self.config
    }
}
