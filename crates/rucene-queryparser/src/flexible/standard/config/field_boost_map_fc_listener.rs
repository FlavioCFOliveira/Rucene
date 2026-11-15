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

//! Port of `FieldBoostMapFCListener.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.config`.

use std::sync::{Arc, PoisonError, Weak};

use rucene::error::{LuceneError, Result};

use super::standard_query_config_handler::ConfigurationKeys;
use crate::flexible::core::config::field_config::FieldConfig;
use crate::flexible::core::config::field_config_listener::FieldConfigListener;
use crate::flexible::core::config::query_config_handler::QueryConfigHandler;

/// This listener listens for every field configuration request and assign a
/// [`ConfigurationKeys::BOOST`] to the equivalent [`FieldConfig`] based on a
/// defined map: fieldName -&gt; boostValue stored in
/// [`ConfigurationKeys::FIELD_BOOST_MAP`].
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.standard.config.FieldBoostMapFCListener`.
///
/// **Divergence.** Java's listener holds its configuration handler, which
/// holds the listener: a cycle the collector frees. The port holds the
/// handler weakly (a strong reference would leak every handler); the handler
/// is alive whenever it calls its listeners. Called directly after the
/// handler is dropped, the listener throws the `NullPointerException` of a
/// `null` handler.
#[derive(Debug)]
pub struct FieldBoostMapFCListener {
    config: Weak<QueryConfigHandler>,
}

impl FieldBoostMapFCListener {
    /// Equivalent to `new FieldBoostMapFCListener(QueryConfigHandler
    /// config)`.
    pub fn new(config: &Arc<QueryConfigHandler>) -> Self {
        Self {
            config: Arc::downgrade(config),
        }
    }
}

impl FieldConfigListener for FieldBoostMapFCListener {
    fn build_field_config(&self, field_config: &FieldConfig) -> Result<()> {
        let config = self
            .config
            .upgrade()
            .ok_or_else(LuceneError::null_pointer)?;
        if let Some(field_boost_map) = config.get(ConfigurationKeys::FIELD_BOOST_MAP) {
            let boost = field_boost_map
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .get(field_config.get_field())
                .copied();
            if let Some(boost) = boost {
                field_config.set(ConfigurationKeys::BOOST, Some(boost));
            }
        }
        Ok(())
    }
}
