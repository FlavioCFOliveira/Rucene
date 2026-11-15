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

//! Port of `QueryConfigHandler.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.config`.

use std::ops::Deref;
use std::sync::{Arc, Mutex, PoisonError};

use rucene::error::Result;

use super::abstract_query_config::AbstractQueryConfig;
use super::field_config::FieldConfig;
use super::field_config_listener::FieldConfigListener;

/// This class can be used to hold any query configuration and no field
/// configuration. For field configuration, it creates an empty
/// [`FieldConfig`] object and delegate it to field config listeners, these
/// are responsible for setting up all the field configuration.
///
/// [`QueryConfigHandler`] should be extended by classes that intends to
/// provide configuration to
/// [`QueryNodeProcessor`](crate::flexible::core::processors::query_node_processor::QueryNodeProcessor)
/// objects.
///
/// The class that extends [`QueryConfigHandler`] should also provide
/// [`FieldConfig`] objects for each collection field.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queryparser.flexible.core.config.QueryConfigHandler`,
/// which extends `AbstractQueryConfig` (it dereferences to it). It is shared
/// as an [`Arc`], as processors, listeners and the parser hold the same
/// object.
#[derive(Default)]
pub struct QueryConfigHandler {
    config: AbstractQueryConfig,
    listeners: Mutex<Vec<Arc<dyn FieldConfigListener>>>,
}

impl std::fmt::Debug for QueryConfigHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryConfigHandler")
            .field("config", &self.config)
            .finish()
    }
}

impl QueryConfigHandler {
    /// Equivalent to `new QueryConfigHandler() { }`: a handler without
    /// configuration or listeners.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            config: AbstractQueryConfig::new(),
            listeners: Mutex::new(Vec::new()),
        })
    }

    /// Returns an implementation of [`FieldConfig`] for a specific field
    /// name. If the implemented [`QueryConfigHandler`] does not know a
    /// specific field name, it may return `null`, indicating there is no
    /// configuration for that field.
    ///
    /// Equivalent to `getFieldConfig(String fieldName)`: a new
    /// [`FieldConfig`] that each listener builds, in the order they were
    /// added.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("field name must not be null!")` for a
    /// `None` field name, and what a listener throws.
    pub fn get_field_config(&self, field_name: Option<&str>) -> Result<FieldConfig> {
        let field_config = FieldConfig::new(field_name)?;
        let listeners: Vec<Arc<dyn FieldConfigListener>> = self
            .listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for listener in listeners {
            listener.build_field_config(&field_config)?;
        }
        Ok(field_config)
    }

    /// Adds a listener. The added listeners are called in the order they are
    /// added.
    ///
    /// Equivalent to `addFieldConfigListener(FieldConfigListener listener)`.
    pub fn add_field_config_listener(&self, listener: Arc<dyn FieldConfigListener>) {
        self.listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(listener);
    }
}

impl Deref for QueryConfigHandler {
    type Target = AbstractQueryConfig;

    fn deref(&self) -> &AbstractQueryConfig {
        &self.config
    }
}
