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

//! The port of `CommonQueryParserConfiguration.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.standard`.

use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::document::Resolution;
use rucene::search::multi_term_query::RewriteMethod;

use crate::jdk::locale::Locale;
use crate::jdk::time_zone::TimeZone;

/// Configuration options common across queryparser implementations.
///
/// Equivalent to
/// `org.apache.lucene.queryparser.flexible.standard.CommonQueryParserConfiguration`.
/// Every classic parser implements it through
/// [`QueryParserBaseOps`](crate::classic::query_parser_base::QueryParserBaseOps),
/// as Java's `QueryParserBase` does.
pub trait CommonQueryParserConfiguration {
    /// Set to `true` to allow leading wildcard characters.
    ///
    /// When set, `*` or `?` are allowed as the first character of a
    /// PrefixQuery and WildcardQuery. Note that this can produce very slow
    /// queries on big indexes.
    ///
    /// Default: false.
    ///
    /// Equivalent to `setAllowLeadingWildcard(boolean)`.
    fn set_allow_leading_wildcard(&mut self, allow_leading_wildcard: bool);

    /// Set to `true` to enable position increments in result query.
    ///
    /// When set, result phrase and multi-phrase queries will be aware of
    /// position increments. Useful when e.g. a StopFilter increases the
    /// position increment of the token that follows an omitted token.
    ///
    /// Default: false.
    ///
    /// Equivalent to `setEnablePositionIncrements(boolean)`.
    fn set_enable_position_increments(&mut self, enabled: bool);

    /// See [`set_enable_position_increments`](Self::set_enable_position_increments).
    ///
    /// Equivalent to `getEnablePositionIncrements()`.
    fn get_enable_position_increments(&self) -> bool;

    /// By default, it uses `MultiTermQuery.CONSTANT_SCORE_BLENDED_REWRITE`
    /// when creating a prefix, wildcard and range queries. This
    /// implementation is generally preferable because it a) Runs faster b)
    /// Does not have the scarcity of terms unduly influence score c) avoids
    /// any `TooManyListenersException` exception. However, if your
    /// application really needs to use the old-fashioned boolean queries
    /// expansion rewriting and the above points are not relevant then use
    /// this change the rewrite method.
    ///
    /// Equivalent to `setMultiTermRewriteMethod(MultiTermQuery.RewriteMethod)`.
    fn set_multi_term_rewrite_method(&mut self, method: Arc<dyn RewriteMethod>);

    /// See [`set_multi_term_rewrite_method`](Self::set_multi_term_rewrite_method).
    ///
    /// Equivalent to `getMultiTermRewriteMethod()`.
    fn get_multi_term_rewrite_method(&self) -> Arc<dyn RewriteMethod>;

    /// Set the prefix length for fuzzy queries. Default is 0.
    ///
    /// Equivalent to `setFuzzyPrefixLength(int)`.
    fn set_fuzzy_prefix_length(&mut self, fuzzy_prefix_length: i32);

    /// Set locale used by date range parsing.
    ///
    /// Equivalent to `setLocale(Locale)`.
    fn set_locale(&mut self, locale: Locale);

    /// Returns current locale, allowing access by subclasses.
    ///
    /// Equivalent to `getLocale()`.
    fn get_locale(&self) -> Locale;

    /// Equivalent to `setTimeZone(TimeZone)`.
    fn set_time_zone(&mut self, time_zone: TimeZone);

    /// Equivalent to `getTimeZone()`.
    fn get_time_zone(&self) -> TimeZone;

    /// Sets the default slop for phrases. If zero, then exact phrase matches
    /// are required. Default value is zero.
    ///
    /// Equivalent to `setPhraseSlop(int)`.
    fn set_phrase_slop(&mut self, default_phrase_slop: i32);

    /// Equivalent to `getAnalyzer()`; `None` for `null`.
    fn get_analyzer(&self) -> Option<Arc<dyn Analyzer>>;

    /// See [`set_allow_leading_wildcard`](Self::set_allow_leading_wildcard).
    ///
    /// Equivalent to `getAllowLeadingWildcard()`.
    fn get_allow_leading_wildcard(&self) -> bool;

    /// Get the minimal similarity for fuzzy queries.
    ///
    /// Equivalent to `getFuzzyMinSim()`.
    fn get_fuzzy_min_sim(&self) -> f32;

    /// Get the prefix length for fuzzy queries.
    ///
    /// Equivalent to `getFuzzyPrefixLength()`.
    fn get_fuzzy_prefix_length(&self) -> i32;

    /// Gets the default slop for phrases.
    ///
    /// Equivalent to `getPhraseSlop()`.
    fn get_phrase_slop(&self) -> i32;

    /// Set the minimum similarity for fuzzy queries. Default is defined on
    /// `FuzzyQuery.defaultMaxEdits`.
    ///
    /// Equivalent to `setFuzzyMinSim(float)`.
    fn set_fuzzy_min_sim(&mut self, fuzzy_min_sim: f32);

    /// Sets the default `Resolution` used for certain field when no
    /// `Resolution` is defined for this field; `None` for `null`.
    ///
    /// Equivalent to `setDateResolution(DateTools.Resolution)`.
    fn set_date_resolution(&mut self, date_resolution: Option<Resolution>);
}
