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

//! Port of `AbstractQueryConfig.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.config`.

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::configuration_key::ConfigurationKey;

/// This class is the base of [`QueryConfigHandler`](super::query_config_handler::QueryConfigHandler)
/// and [`FieldConfig`](super::field_config::FieldConfig). It has operations
/// to set, unset and get configuration values.
///
/// Each configuration is is a key-&gt;value pair. The key should be an
/// unique [`ConfigurationKey`] instance and it also holds the value's type.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queryparser.flexible.core.config.AbstractQueryConfig`:
/// its subclasses embed it. A value is held as Java holds it, by reference:
/// a value of a mutable Java class is an [`Arc`] whose content is shared
/// with whoever set it.
#[derive(Debug, Default)]
pub struct AbstractQueryConfig {
    config_map: Mutex<HashMap<u64, Arc<dyn Any + Send + Sync>>>,
}

impl AbstractQueryConfig {
    /// Equivalent to the package-private `AbstractQueryConfig()`: although
    /// this class is public, it can only be constructed from package.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn map(&self) -> MutexGuard<'_, HashMap<u64, Arc<dyn Any + Send + Sync>>> {
        self.config_map
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Returns the value held by the given key.
    ///
    /// Equivalent to `get(ConfigurationKey<T> key)`: `None` for Java's `null`
    /// (no value).
    pub fn get<T: Clone + Send + Sync + 'static>(&self, key: &ConfigurationKey<T>) -> Option<T> {
        self.map()
            .get(&key.id())
            .and_then(|value| value.downcast_ref::<T>())
            .cloned()
    }

    /// Returns true if there is a value set with the given key, otherwise
    /// false.
    ///
    /// Equivalent to `has(ConfigurationKey<T> key)`.
    pub fn has<T>(&self, key: &ConfigurationKey<T>) -> bool {
        self.map().contains_key(&key.id())
    }

    /// Sets a key and its value.
    ///
    /// Equivalent to `set(ConfigurationKey<T> key, T value)`: a `None` value
    /// (Java's `null`) unsets the key.
    pub fn set<T: Send + Sync + 'static>(&self, key: &ConfigurationKey<T>, value: Option<T>) {
        match value {
            None => {
                self.unset(key);
            }
            Some(value) => {
                self.map().insert(key.id(), Arc::new(value));
            }
        }
    }

    /// Unsets the given key and its value.
    ///
    /// Equivalent to `unset(ConfigurationKey<T> key)`: true if the key and
    /// value was set and removed, otherwise false.
    pub fn unset<T>(&self, key: &ConfigurationKey<T>) -> bool {
        self.map().remove(&key.id()).is_some()
    }
}
