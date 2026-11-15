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

//! Port of `SpansQueryConfigHandler.java`.

use std::sync::Arc;

use rucene_queryparser::flexible::core::config::configuration_key::ConfigurationKey;
use rucene_queryparser::flexible::core::config::query_config_handler::QueryConfigHandler;

/// `public static final ConfigurationKey<String> UNIQUE_FIELD =
/// ConfigurationKey.newInstance()`.
static UNIQUE_FIELD_KEY: ConfigurationKey<String> = ConfigurationKey::new_static();

/// This query config handler only adds the `UniqueFieldAttribute` to it.
///
/// It does not return any configuration for a field in specific.
pub struct SpansQueryConfigHandler;

impl SpansQueryConfigHandler {
    /// `SpansQueryConfigHandler.UNIQUE_FIELD`.
    pub const UNIQUE_FIELD: &'static ConfigurationKey<String> = &UNIQUE_FIELD_KEY;

    /// `new SpansQueryConfigHandler()`: a `QueryConfigHandler` whose
    /// `getFieldConfig(String fieldName)` override returns `null` (there is
    /// no field configuration).
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> Arc<QueryConfigHandler> {
        // empty constructor
        QueryConfigHandler::with_get_field_config(Arc::new(|_this, _field_name| {
            // there is no field configuration, always return null
            Ok(None)
        }))
    }
}
