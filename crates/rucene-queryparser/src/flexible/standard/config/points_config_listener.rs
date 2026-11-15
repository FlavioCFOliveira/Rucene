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

//! Port of `PointsConfigListener.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.config`.

use std::sync::{Arc, PoisonError, Weak};

use rucene::error::{LuceneError, Result};

use super::standard_query_config_handler::ConfigurationKeys;
use crate::flexible::core::config::field_config::FieldConfig;
use crate::flexible::core::config::field_config_listener::FieldConfigListener;
use crate::flexible::core::config::query_config_handler::QueryConfigHandler;

/// This listener is used to listen to [`FieldConfig`] requests in
/// [`QueryConfigHandler`] and add [`ConfigurationKeys::POINTS_CONFIG`] based
/// on the [`ConfigurationKeys::POINTS_CONFIG_MAP`] set in the
/// [`QueryConfigHandler`].
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.standard.config.PointsConfigListener`.
///
/// **Divergence.** The configuration handler is held weakly, as by
/// [`FieldBoostMapFCListener`](super::field_boost_map_fc_listener::FieldBoostMapFCListener).
#[derive(Debug)]
pub struct PointsConfigListener {
    config: Weak<QueryConfigHandler>,
}

impl PointsConfigListener {
    /// Constructs a [`PointsConfigListener`] object using the given
    /// [`QueryConfigHandler`].
    ///
    /// Equivalent to `new PointsConfigListener(QueryConfigHandler config)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("config must not be null!")` for `None`.
    pub fn new(config: Option<&Arc<QueryConfigHandler>>) -> Result<Self> {
        let config =
            config.ok_or_else(|| LuceneError::illegal_argument("config must not be null!"))?;
        Ok(Self {
            config: Arc::downgrade(config),
        })
    }
}

impl FieldConfigListener for PointsConfigListener {
    fn build_field_config(&self, field_config: &FieldConfig) -> Result<()> {
        let config = self
            .config
            .upgrade()
            .ok_or_else(LuceneError::null_pointer)?;
        if let Some(points_config_map) = config.get(ConfigurationKeys::POINTS_CONFIG_MAP) {
            let points_config = points_config_map
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .get(field_config.get_field())
                .cloned();
            if let Some(points_config) = points_config {
                field_config.set(ConfigurationKeys::POINTS_CONFIG, Some(points_config));
            }
        }
        Ok(())
    }
}
