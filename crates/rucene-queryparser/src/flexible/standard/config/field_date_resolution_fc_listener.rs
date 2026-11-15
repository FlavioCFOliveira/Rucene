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

//! Port of `FieldDateResolutionFCListener.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.config`.

use std::sync::{Arc, PoisonError, Weak};

use rucene::error::{LuceneError, Result};

use super::standard_query_config_handler::ConfigurationKeys;
use crate::flexible::core::config::field_config::FieldConfig;
use crate::flexible::core::config::field_config_listener::FieldConfigListener;
use crate::flexible::core::config::query_config_handler::QueryConfigHandler;
use crate::jdk::char_sequence::CharSequence;

/// This listener listens for every field configuration request and assign a
/// [`ConfigurationKeys::DATE_RESOLUTION`] to the equivalent [`FieldConfig`]
/// based on a defined map: fieldName -&gt; `DateTools.Resolution` stored in
/// [`ConfigurationKeys::FIELD_DATE_RESOLUTION_MAP`].
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.standard.config.FieldDateResolutionFCListener`.
///
/// **Divergence.** The configuration handler is held weakly, as by
/// [`FieldBoostMapFCListener`](super::field_boost_map_fc_listener::FieldBoostMapFCListener).
#[derive(Debug)]
pub struct FieldDateResolutionFCListener {
    config: Weak<QueryConfigHandler>,
}

impl FieldDateResolutionFCListener {
    /// Equivalent to `new FieldDateResolutionFCListener(QueryConfigHandler
    /// config)`.
    pub fn new(config: &Arc<QueryConfigHandler>) -> Self {
        Self {
            config: Arc::downgrade(config),
        }
    }
}

impl FieldConfigListener for FieldDateResolutionFCListener {
    fn build_field_config(&self, field_config: &FieldConfig) -> Result<()> {
        let config = self
            .config
            .upgrade()
            .ok_or_else(LuceneError::null_pointer)?;
        let mut date_res = None;
        if let Some(date_res_map) = config.get(ConfigurationKeys::FIELD_DATE_RESOLUTION_MAP) {
            date_res = date_res_map
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&CharSequence::from(field_config.get_field()))
                .copied();
        }
        if date_res.is_none() {
            date_res = config.get(ConfigurationKeys::DATE_RESOLUTION);
        }
        if date_res.is_some() {
            field_config.set(ConfigurationKeys::DATE_RESOLUTION, date_res);
        }
        Ok(())
    }
}
