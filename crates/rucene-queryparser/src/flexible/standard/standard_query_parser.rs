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

//! Port of `StandardQueryParser.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard`.

use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, RwLock};

use rucene::analysis::Analyzer;
use rucene::document::Resolution;
use rucene::error::{LuceneError, Result};
use rucene::search::fuzzy_query;
use rucene::search::multi_term_query::RewriteMethod;
use rucene::search::Query;

use super::builders::standard_query_builder::query_of;
use super::builders::standard_query_tree_builder::StandardQueryTreeBuilder;
use super::common_query_parser_configuration::CommonQueryParserConfiguration;
use super::config::fuzzy_config::FuzzyConfig;
use super::config::standard_query_config_handler::{
    ConfigurationKeys, FieldBoostMap, FieldDateResolutionMap, MultiFields, Operator,
    PointsConfigMap, StandardQueryConfigHandler,
};
use super::parser::standard_syntax_parser::StandardSyntaxParser;
use super::processors::standard_query_node_processor_pipeline::StandardQueryNodeProcessorPipeline;
use crate::flexible::core::config::query_config_handler::QueryConfigHandler;
use crate::flexible::core::query_parser_helper::QueryParserHelper;
use crate::jdk::locale::Locale;
use crate::jdk::time_zone::TimeZone;

/// The [`StandardQueryParser`] is a pre-assembled query parser that supports
/// most features of the classic Lucene query parser, allows dynamic
/// configuration of some of its features (like multi-field expansion or
/// wildcard query restrictions) and adds support for new query types and
/// expressions.
///
/// The `StandardSyntaxParser` is an extension of the [`QueryParserHelper`]
/// with reasonable defaults for syntax tree parsing (`StandardSyntaxParser`,
/// node processor pipeline (`StandardQueryNodeProcessorPipeline`) and node
/// tree to [`Query`] builder (`StandardQueryTreeBuilder`).
///
/// See the Java class's documentation for the supported query syntax: basic
/// clauses, field specifications, boolean operators and grouping, range
/// operators, term boosting, special character escaping, the
/// minimum-should-match constraint for boolean disjunction groups (`(blue
/// crab fish)@2`) and interval function clauses (`fn:ordered(quick brown
/// fox)`).
///
/// Equivalent to `org.apache.lucene.queryparser.flexible.standard.StandardQueryParser`,
/// which extends `QueryParserHelper` (embedded, and reached by dereference)
/// and implements `CommonQueryParserConfiguration`. Each configuration method
/// reads or writes the configuration handler, as Java's does: without one
/// (Java's `null`) it throws Java's `NullPointerException`.
#[derive(Debug, Clone)]
pub struct StandardQueryParser {
    helper: QueryParserHelper,
}

impl StandardQueryParser {
    /// Constructs a [`StandardQueryParser`] object.
    ///
    /// Equivalent to `new StandardQueryParser()`: a `StandardQueryConfigHandler`,
    /// a `StandardSyntaxParser`, a `StandardQueryNodeProcessorPipeline` and a
    /// `StandardQueryTreeBuilder`, then position increments enabled.
    pub fn new() -> Self {
        let config = StandardQueryConfigHandler::create();
        let helper = QueryParserHelper::new(
            Some(Arc::clone(&config)),
            Some(Arc::new(Mutex::new(StandardSyntaxParser::new()))),
            Some(Arc::new(StandardQueryNodeProcessorPipeline::new(None))),
            Some(Arc::new(StandardQueryTreeBuilder::new())),
        );
        // setEnablePositionIncrements(true), on the handler just set
        config.set(ConfigurationKeys::ENABLE_POSITION_INCREMENTS, Some(true));
        Self { helper }
    }

    /// Constructs a [`StandardQueryParser`] object and sets an [`Analyzer`]
    /// to it. The same as: `StandardQueryParser qp = new
    /// StandardQueryParser(); qp.getQueryConfigHandler().setAnalyzer(analyzer);`
    ///
    /// Equivalent to `new StandardQueryParser(Analyzer analyzer)`.
    pub fn with_analyzer(analyzer: Option<Arc<dyn Analyzer>>) -> Self {
        let parser = Self::new();
        if let Some(config) = parser.helper.get_query_config_handler() {
            config.set(ConfigurationKeys::ANALYZER, analyzer);
        }
        parser
    }

    /// The `QueryParserHelper` this parser extends.
    pub fn query_parser_helper(&self) -> &QueryParserHelper {
        &self.helper
    }

    /// The `QueryParserHelper` this parser extends, for writing.
    pub fn query_parser_helper_mut(&mut self) -> &mut QueryParserHelper {
        &mut self.helper
    }

    /// `getQueryConfigHandler()`, dereferenced as Java's methods do.
    fn config(&self) -> Result<Arc<QueryConfigHandler>> {
        self.helper
            .get_query_config_handler()
            .ok_or_else(LuceneError::null_pointer)
    }

    /// Equivalent to `toString()`: the configuration handler as
    /// `Object.toString()` renders it.
    pub fn to_java_string(&self) -> String {
        let config = match self.helper.get_query_config_handler() {
            None => "null".to_string(),
            Some(config) => rucene::index::index_reader::java_object_to_string(
                "org.apache.lucene.queryparser.flexible.standard.config.StandardQueryConfigHandler",
                &*config,
            ),
        };
        format!("<StandardQueryParser config=\"{config}\"/>")
    }

    /// Overrides `QueryParserHelper.parse(String, String)` so it casts the
    /// return object to [`Query`]. For more reference about this method,
    /// check [`QueryParserHelper::parse`].
    ///
    /// Equivalent to `parse(String query, String defaultField)`: the query
    /// built (`None` for Java's `null`).
    ///
    /// # Errors
    ///
    /// A `QueryNodeException` if something wrong happens along the three
    /// phases, and the `ClassCastException` of a built object that is not a
    /// query.
    pub fn parse(
        &self,
        query: &str,
        default_field: Option<&str>,
    ) -> Result<Option<Arc<dyn Query>>> {
        self.helper
            .parse(query, default_field)?
            .map(|obj| query_of(&obj))
            .transpose()
    }

    /// Gets implicit operator setting, which will be either `Operator.AND`
    /// or `Operator.OR`.
    ///
    /// Equivalent to `getDefaultOperator()`.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of a missing configuration handler.
    pub fn get_default_operator(&self) -> Result<Option<Operator>> {
        Ok(self.config()?.get(ConfigurationKeys::DEFAULT_OPERATOR))
    }

    /// Sets the boolean operator of the QueryParser. In default mode
    /// (`Operator.OR`) terms without any modifiers are considered optional:
    /// for example `capital of Hungary` is equal to `capital OR of OR
    /// Hungary`. In `Operator.AND` mode terms are considered to be in
    /// conjunction: the above mentioned query is parsed as `capital AND of
    /// AND Hungary`
    ///
    /// Equivalent to `setDefaultOperator(StandardQueryConfigHandler.Operator
    /// operator)`; `None` (Java's `null`) unsets it.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of a missing configuration handler.
    pub fn set_default_operator(&mut self, operator: Option<Operator>) -> Result<()> {
        self.config()?
            .set(ConfigurationKeys::DEFAULT_OPERATOR, operator);
        Ok(())
    }

    /// Set the fields a query should be expanded to when the field is
    /// `null`
    ///
    /// Equivalent to `setMultiFields(CharSequence[] fields)`: `None` (Java's
    /// `null`) is an empty array; the array is held by reference.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of a missing configuration handler.
    pub fn set_multi_fields(&mut self, fields: Option<MultiFields>) -> Result<()> {
        let fields = fields.unwrap_or_else(|| Arc::new(RwLock::new(Vec::new())));
        self.config()?
            .set(ConfigurationKeys::MULTI_FIELDS, Some(fields));
        Ok(())
    }

    /// Returns the fields used to expand the query when the field for a
    /// certain query is `null`
    ///
    /// Equivalent to `getMultiFields()`.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of a missing configuration handler.
    pub fn get_multi_fields(&self) -> Result<Option<MultiFields>> {
        Ok(self.config()?.get(ConfigurationKeys::MULTI_FIELDS))
    }

    /// Equivalent to `setPointsConfigMap(Map<String, PointsConfig>
    /// pointsConfigMap)`; `None` (Java's `null`) unsets it.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of a missing configuration handler.
    pub fn set_points_config_map(
        &mut self,
        points_config_map: Option<PointsConfigMap>,
    ) -> Result<()> {
        self.config()?
            .set(ConfigurationKeys::POINTS_CONFIG_MAP, points_config_map);
        Ok(())
    }

    /// Equivalent to `getPointsConfigMap()`.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of a missing configuration handler.
    pub fn get_points_config_map(&self) -> Result<Option<PointsConfigMap>> {
        Ok(self.config()?.get(ConfigurationKeys::POINTS_CONFIG_MAP))
    }

    /// Equivalent to `setAnalyzer(Analyzer analyzer)`; `None` (Java's
    /// `null`) unsets it.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of a missing configuration handler.
    pub fn set_analyzer(&mut self, analyzer: Option<Arc<dyn Analyzer>>) -> Result<()> {
        self.config()?.set(ConfigurationKeys::ANALYZER, analyzer);
        Ok(())
    }

    /// Sets the boost used for each field.
    ///
    /// Equivalent to `setFieldsBoost(Map<String, Float> boosts)`; the map is
    /// held by reference, `None` (Java's `null`) unsets it.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of a missing configuration handler.
    pub fn set_fields_boost(&mut self, boosts: Option<FieldBoostMap>) -> Result<()> {
        self.config()?
            .set(ConfigurationKeys::FIELD_BOOST_MAP, boosts);
        Ok(())
    }

    /// Returns the field to boost map used to set boost for each field.
    ///
    /// Equivalent to `getFieldsBoost()`.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of a missing configuration handler.
    pub fn get_fields_boost(&self) -> Result<Option<FieldBoostMap>> {
        Ok(self.config()?.get(ConfigurationKeys::FIELD_BOOST_MAP))
    }

    /// Returns the default `Resolution` used for certain field when no
    /// `Resolution` is defined for this field.
    ///
    /// Equivalent to `getDateResolution()`.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of a missing configuration handler.
    pub fn get_date_resolution(&self) -> Result<Option<Resolution>> {
        Ok(self.config()?.get(ConfigurationKeys::DATE_RESOLUTION))
    }

    /// Returns the field to `Resolution` map used to normalize each date
    /// field.
    ///
    /// Equivalent to `getDateResolutionMap()`.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of a missing configuration handler.
    pub fn get_date_resolution_map(&self) -> Result<Option<FieldDateResolutionMap>> {
        Ok(self
            .config()?
            .get(ConfigurationKeys::FIELD_DATE_RESOLUTION_MAP))
    }

    /// Sets the `Resolution` used for each field
    ///
    /// Equivalent to `setDateResolutionMap(Map<CharSequence,
    /// DateTools.Resolution> dateRes)`; the map is held by reference, `None`
    /// (Java's `null`) unsets it.
    ///
    /// # Errors
    ///
    /// The `NullPointerException` of a missing configuration handler.
    pub fn set_date_resolution_map(
        &mut self,
        date_res: Option<FieldDateResolutionMap>,
    ) -> Result<()> {
        self.config()?
            .set(ConfigurationKeys::FIELD_DATE_RESOLUTION_MAP, date_res);
        Ok(())
    }

    /// The fuzzy configuration, created and set when there is none, as
    /// `setFuzzyPrefixLength` and `setFuzzyMinSim` do.
    fn fuzzy_config(&self) -> Result<Arc<FuzzyConfig>> {
        let config = self.config()?;
        match config.get(ConfigurationKeys::FUZZY_CONFIG) {
            Some(fuzzy_config) => Ok(fuzzy_config),
            None => {
                let fuzzy_config = Arc::new(FuzzyConfig::new());
                config.set(
                    ConfigurationKeys::FUZZY_CONFIG,
                    Some(Arc::clone(&fuzzy_config)),
                );
                Ok(fuzzy_config)
            }
        }
    }
}

impl Default for StandardQueryParser {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for StandardQueryParser {
    type Target = QueryParserHelper;

    fn deref(&self) -> &QueryParserHelper {
        &self.helper
    }
}

impl DerefMut for StandardQueryParser {
    fn deref_mut(&mut self) -> &mut QueryParserHelper {
        &mut self.helper
    }
}

impl CommonQueryParserConfiguration for StandardQueryParser {
    /// Set to `true` to allow leading wildcard characters.
    ///
    /// When set, `*` or `?` are allowed as the first character of a
    /// PrefixQuery and WildcardQuery. Note that this can produce very slow
    /// queries on big indexes.
    ///
    /// Default: false.
    fn set_allow_leading_wildcard(&mut self, allow_leading_wildcard: bool) -> Result<()> {
        self.config()?.set(
            ConfigurationKeys::ALLOW_LEADING_WILDCARD,
            Some(allow_leading_wildcard),
        );
        Ok(())
    }

    /// Set to `true` to enable position increments in result query.
    ///
    /// When set, result phrase and multi-phrase queries will be aware of
    /// position increments. Useful when e.g. a StopFilter increases the
    /// position increment of the token that follows an omitted token.
    ///
    /// Default: false.
    fn set_enable_position_increments(&mut self, enabled: bool) -> Result<()> {
        self.config()?
            .set(ConfigurationKeys::ENABLE_POSITION_INCREMENTS, Some(enabled));
        Ok(())
    }

    fn get_enable_position_increments(&self) -> Result<bool> {
        Ok(self
            .config()?
            .get(ConfigurationKeys::ENABLE_POSITION_INCREMENTS)
            .unwrap_or(false))
    }

    /// Equivalent to `setMultiTermRewriteMethod(MultiTermQuery.RewriteMethod
    /// method)`; `None` (Java's `null`) unsets the key.
    fn set_multi_term_rewrite_method(
        &mut self,
        method: Option<Arc<dyn RewriteMethod>>,
    ) -> Result<()> {
        self.config()?
            .set(ConfigurationKeys::MULTI_TERM_REWRITE_METHOD, method);
        Ok(())
    }

    fn get_multi_term_rewrite_method(&self) -> Result<Option<Arc<dyn RewriteMethod>>> {
        Ok(self
            .config()?
            .get(ConfigurationKeys::MULTI_TERM_REWRITE_METHOD))
    }

    /// Set the prefix length for fuzzy queries. Default is 0.
    fn set_fuzzy_prefix_length(&mut self, fuzzy_prefix_length: i32) -> Result<()> {
        self.fuzzy_config()?.set_prefix_length(fuzzy_prefix_length);
        Ok(())
    }

    /// Set locale used by date range parsing.
    ///
    /// `None` (Java's `null`) unsets the key: `TermRangeQueryNodeProcessor`
    /// then uses `Locale.getDefault()`.
    fn set_locale(&mut self, locale: Option<Locale>) -> Result<()> {
        self.config()?.set(ConfigurationKeys::LOCALE, locale);
        Ok(())
    }

    /// Returns current locale, allowing access by subclasses.
    fn get_locale(&self) -> Result<Option<Locale>> {
        Ok(self.config()?.get(ConfigurationKeys::LOCALE))
    }

    /// Equivalent to `setTimeZone(TimeZone timeZone)`; `None` (Java's
    /// `null`) unsets the key: `TermRangeQueryNodeProcessor` then uses
    /// `TimeZone.getDefault()`.
    fn set_time_zone(&mut self, time_zone: Option<TimeZone>) -> Result<()> {
        self.config()?.set(ConfigurationKeys::TIMEZONE, time_zone);
        Ok(())
    }

    fn get_time_zone(&self) -> Result<Option<TimeZone>> {
        Ok(self.config()?.get(ConfigurationKeys::TIMEZONE))
    }

    /// Sets the default slop for phrases. If zero, then exact phrase matches
    /// are required. Default value is zero.
    fn set_phrase_slop(&mut self, default_phrase_slop: i32) -> Result<()> {
        self.config()?
            .set(ConfigurationKeys::PHRASE_SLOP, Some(default_phrase_slop));
        Ok(())
    }

    fn get_analyzer(&self) -> Result<Option<Arc<dyn Analyzer>>> {
        Ok(self.config()?.get(ConfigurationKeys::ANALYZER))
    }

    fn get_allow_leading_wildcard(&self) -> Result<bool> {
        Ok(self
            .config()?
            .get(ConfigurationKeys::ALLOW_LEADING_WILDCARD)
            .unwrap_or(false))
    }

    /// Get the minimal similarity for fuzzy queries.
    fn get_fuzzy_min_sim(&self) -> Result<f32> {
        match self.config()?.get(ConfigurationKeys::FUZZY_CONFIG) {
            None => Ok(fuzzy_query::DEFAULT_MAX_EDITS as f32),
            Some(fuzzy_config) => Ok(fuzzy_config.get_min_similarity()),
        }
    }

    /// Get the prefix length for fuzzy queries.
    fn get_fuzzy_prefix_length(&self) -> Result<i32> {
        match self.config()?.get(ConfigurationKeys::FUZZY_CONFIG) {
            None => Ok(fuzzy_query::DEFAULT_PREFIX_LENGTH),
            Some(fuzzy_config) => Ok(fuzzy_config.get_prefix_length()),
        }
    }

    /// Gets the default slop for phrases.
    fn get_phrase_slop(&self) -> Result<i32> {
        Ok(self
            .config()?
            .get(ConfigurationKeys::PHRASE_SLOP)
            .unwrap_or(0))
    }

    /// Set the minimum similarity for fuzzy queries. Default is defined on
    /// `FuzzyQuery.defaultMaxEdits`.
    fn set_fuzzy_min_sim(&mut self, fuzzy_min_sim: f32) -> Result<()> {
        self.fuzzy_config()?.set_min_similarity(fuzzy_min_sim);
        Ok(())
    }

    /// Sets the default `Resolution` used for certain field when no
    /// `Resolution` is defined for this field.
    fn set_date_resolution(&mut self, date_resolution: Option<Resolution>) -> Result<()> {
        self.config()?
            .set(ConfigurationKeys::DATE_RESOLUTION, date_resolution);
        Ok(())
    }
}
