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

//! Port of `StandardQueryConfigHandler.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard.config`.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use rucene::analysis::Analyzer;
use rucene::document::Resolution;
use rucene::search::multi_term_query::{constant_score_blended_rewrite, RewriteMethod};

use super::field_boost_map_fc_listener::FieldBoostMapFCListener;
use super::field_date_resolution_fc_listener::FieldDateResolutionFCListener;
use super::fuzzy_config::FuzzyConfig;
use super::points_config::PointsConfig;
use super::points_config_listener::PointsConfigListener;
use crate::flexible::core::config::configuration_key::ConfigurationKey;
use crate::flexible::core::config::query_config_handler::QueryConfigHandler;
use crate::jdk::char_sequence::CharSequence;
use crate::jdk::locale::Locale;
use crate::jdk::time_zone::TimeZone;

/// The value of [`ConfigurationKeys::MULTI_FIELDS`]: Java's `CharSequence[]`,
/// shared by reference (`None` elements for `null`).
pub type MultiFields = Arc<RwLock<Vec<Option<CharSequence>>>>;

/// The value of [`ConfigurationKeys::FIELD_BOOST_MAP`]: Java's `Map<String,
/// Float>`, shared by reference.
pub type FieldBoostMap = Arc<RwLock<HashMap<String, f32>>>;

/// The value of [`ConfigurationKeys::FIELD_DATE_RESOLUTION_MAP`]: Java's
/// `Map<CharSequence, DateTools.Resolution>`, shared by reference.
pub type FieldDateResolutionMap = Arc<RwLock<HashMap<CharSequence, Resolution>>>;

/// The value of [`ConfigurationKeys::POINTS_CONFIG_MAP`]: Java's `Map<String,
/// PointsConfig>`, shared by reference.
pub type PointsConfigMap = Arc<RwLock<HashMap<String, Arc<PointsConfig>>>>;

static ENABLE_POSITION_INCREMENTS: ConfigurationKey<bool> = ConfigurationKey::new_static();
static ALLOW_LEADING_WILDCARD: ConfigurationKey<bool> = ConfigurationKey::new_static();
static ANALYZER: ConfigurationKey<Arc<dyn Analyzer>> = ConfigurationKey::new_static();
static DEFAULT_OPERATOR: ConfigurationKey<Operator> = ConfigurationKey::new_static();
static PHRASE_SLOP: ConfigurationKey<i32> = ConfigurationKey::new_static();
static LOCALE: ConfigurationKey<Locale> = ConfigurationKey::new_static();
static TIMEZONE: ConfigurationKey<TimeZone> = ConfigurationKey::new_static();
static MULTI_TERM_REWRITE_METHOD: ConfigurationKey<Arc<dyn RewriteMethod>> =
    ConfigurationKey::new_static();
static MULTI_FIELDS: ConfigurationKey<MultiFields> = ConfigurationKey::new_static();
static FIELD_BOOST_MAP: ConfigurationKey<FieldBoostMap> = ConfigurationKey::new_static();
static FIELD_DATE_RESOLUTION_MAP: ConfigurationKey<FieldDateResolutionMap> =
    ConfigurationKey::new_static();
static FUZZY_CONFIG: ConfigurationKey<Arc<FuzzyConfig>> = ConfigurationKey::new_static();
static DATE_RESOLUTION: ConfigurationKey<Resolution> = ConfigurationKey::new_static();
static BOOST: ConfigurationKey<f32> = ConfigurationKey::new_static();
static POINTS_CONFIG: ConfigurationKey<Arc<PointsConfig>> = ConfigurationKey::new_static();
static POINTS_CONFIG_MAP: ConfigurationKey<PointsConfigMap> = ConfigurationKey::new_static();

/// Class holding keys for StandardQueryNodeProcessorPipeline options.
///
/// Equivalent to the final static nested class
/// `org.apache.lucene.queryparser.flexible.standard.config.StandardQueryConfigHandler.ConfigurationKeys`;
/// its `public static final` keys are the associated constants. A value of a
/// mutable Java class is shared by reference ([`Arc`], with a lock for the
/// collections).
#[derive(Debug)]
pub struct ConfigurationKeys {
    _no_instances: (),
}

impl ConfigurationKeys {
    /// Key used to set whether position increments is enabled
    ///
    /// Equivalent to `ConfigurationKeys.ENABLE_POSITION_INCREMENTS`.
    pub const ENABLE_POSITION_INCREMENTS: &'static ConfigurationKey<bool> =
        &ENABLE_POSITION_INCREMENTS;

    /// Key used to set whether leading wildcards are supported
    ///
    /// Equivalent to `ConfigurationKeys.ALLOW_LEADING_WILDCARD`.
    pub const ALLOW_LEADING_WILDCARD: &'static ConfigurationKey<bool> = &ALLOW_LEADING_WILDCARD;

    /// Key used to set the `Analyzer` used for terms found in the query
    ///
    /// Equivalent to `ConfigurationKeys.ANALYZER`.
    pub const ANALYZER: &'static ConfigurationKey<Arc<dyn Analyzer>> = &ANALYZER;

    /// Key used to set the default boolean operator
    ///
    /// Equivalent to `ConfigurationKeys.DEFAULT_OPERATOR`.
    pub const DEFAULT_OPERATOR: &'static ConfigurationKey<Operator> = &DEFAULT_OPERATOR;

    /// Key used to set the default phrase slop
    ///
    /// Equivalent to `ConfigurationKeys.PHRASE_SLOP`.
    pub const PHRASE_SLOP: &'static ConfigurationKey<i32> = &PHRASE_SLOP;

    /// Key used to set the `Locale` used when parsing the query
    ///
    /// Equivalent to `ConfigurationKeys.LOCALE`.
    pub const LOCALE: &'static ConfigurationKey<Locale> = &LOCALE;

    /// Key used to set the time zone used when parsing dates
    ///
    /// Equivalent to `ConfigurationKeys.TIMEZONE`.
    pub const TIMEZONE: &'static ConfigurationKey<TimeZone> = &TIMEZONE;

    /// Key used to set the `MultiTermQuery.RewriteMethod` used when creating queries
    ///
    /// Equivalent to `ConfigurationKeys.MULTI_TERM_REWRITE_METHOD`.
    pub const MULTI_TERM_REWRITE_METHOD: &'static ConfigurationKey<Arc<dyn RewriteMethod>> =
        &MULTI_TERM_REWRITE_METHOD;

    /// Key used to set the fields a query should be expanded to when the field is `null`
    ///
    /// Equivalent to `ConfigurationKeys.MULTI_FIELDS`.
    pub const MULTI_FIELDS: &'static ConfigurationKey<MultiFields> = &MULTI_FIELDS;

    /// Key used to set a field to boost map that is used to set the boost for each field
    ///
    /// Equivalent to `ConfigurationKeys.FIELD_BOOST_MAP`.
    pub const FIELD_BOOST_MAP: &'static ConfigurationKey<FieldBoostMap> = &FIELD_BOOST_MAP;

    /// Key used to set a field to `Resolution` map that is used to normalize each date field value.
    ///
    /// Equivalent to `ConfigurationKeys.FIELD_DATE_RESOLUTION_MAP`.
    pub const FIELD_DATE_RESOLUTION_MAP: &'static ConfigurationKey<FieldDateResolutionMap> =
        &FIELD_DATE_RESOLUTION_MAP;

    /// Key used to set the [`FuzzyConfig`] used to create fuzzy queries.
    ///
    /// Equivalent to `ConfigurationKeys.FUZZY_CONFIG`.
    pub const FUZZY_CONFIG: &'static ConfigurationKey<Arc<FuzzyConfig>> = &FUZZY_CONFIG;

    /// Key used to set default `Resolution`.
    ///
    /// Equivalent to `ConfigurationKeys.DATE_RESOLUTION`.
    pub const DATE_RESOLUTION: &'static ConfigurationKey<Resolution> = &DATE_RESOLUTION;

    /// Key used to set the boost value in `FieldConfig` objects.
    ///
    /// Equivalent to `ConfigurationKeys.BOOST`.
    pub const BOOST: &'static ConfigurationKey<f32> = &BOOST;

    /// This option is used to configure numeric settings of a field.
    ///
    /// Equivalent to `ConfigurationKeys.POINTS_CONFIG`.
    pub const POINTS_CONFIG: &'static ConfigurationKey<Arc<PointsConfig>> = &POINTS_CONFIG;

    /// This option is used to configure numeric settings of each field.
    ///
    /// Equivalent to `ConfigurationKeys.POINTS_CONFIG_MAP`.
    pub const POINTS_CONFIG_MAP: &'static ConfigurationKey<PointsConfigMap> = &POINTS_CONFIG_MAP;
}

/// Boolean Operator: AND or OR
///
/// Equivalent to the enum
/// `org.apache.lucene.queryparser.flexible.standard.config.StandardQueryConfigHandler.Operator`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operator {
    /// Java constant `AND`.
    And,
    /// Java constant `OR`.
    Or,
}

/// This query configuration handler is used for almost every processor
/// defined in the `StandardQueryNodeProcessorPipeline` processor pipeline.
/// It holds configuration methods that reproduce the configuration methods
/// that could be set on the old lucene 2.4 QueryParser class.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.standard.config.StandardQueryConfigHandler`,
/// which extends `QueryConfigHandler` and adds only a constructor: its
/// instances are the [`QueryConfigHandler`]s [`create`](Self::create)
/// builds.
#[derive(Debug)]
pub struct StandardQueryConfigHandler {
    _no_instances: (),
}

impl StandardQueryConfigHandler {
    /// Equivalent to `new StandardQueryConfigHandler()`: the field
    /// configuration listeners (boost map, date resolution, points), then
    /// the default values.
    pub fn create() -> Arc<QueryConfigHandler> {
        let handler = QueryConfigHandler::new();
        // Add listener that will build the FieldConfig.
        handler.add_field_config_listener(Arc::new(FieldBoostMapFCListener::new(&handler)));
        handler.add_field_config_listener(Arc::new(FieldDateResolutionFCListener::new(&handler)));
        handler.add_field_config_listener(Arc::new(
            PointsConfigListener::new(Some(&handler)).expect("INVARIANT: the handler is not null"),
        ));
        // Default Values
        handler.set(ConfigurationKeys::ALLOW_LEADING_WILDCARD, Some(false)); // default in 2.9
        handler.set(ConfigurationKeys::ANALYZER, None); // default value 2.4
        handler.set(ConfigurationKeys::DEFAULT_OPERATOR, Some(Operator::Or));
        handler.set(ConfigurationKeys::PHRASE_SLOP, Some(0)); // default value 2.4
        handler.set(ConfigurationKeys::ENABLE_POSITION_INCREMENTS, Some(false)); // default value 2.4
        handler.set(
            ConfigurationKeys::FIELD_BOOST_MAP,
            Some(Arc::new(RwLock::new(HashMap::new()))),
        );
        handler.set(
            ConfigurationKeys::FUZZY_CONFIG,
            Some(Arc::new(FuzzyConfig::new())),
        );
        handler.set(ConfigurationKeys::LOCALE, Some(Locale::get_default()));
        handler.set(
            ConfigurationKeys::MULTI_TERM_REWRITE_METHOD,
            Some(constant_score_blended_rewrite()),
        );
        handler.set(
            ConfigurationKeys::FIELD_DATE_RESOLUTION_MAP,
            Some(Arc::new(RwLock::new(HashMap::new()))),
        );
        handler
    }
}
