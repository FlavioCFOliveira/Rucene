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
//! Factory for `DateRecognizerFilter`, ported from
//! `org.apache.lucene.analysis.miscellaneous.DateRecognizerFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::Result;

use super::date_recognizer_filter::new_date_recognizer_filter_with;
use crate::jdk::locale::Locale;
use crate::jdk::simple_date_format::SimpleDateFormat;
use crate::spi::check_unknown_parameters;

/// Factory for `DateRecognizerFilter`.
///
/// ```text
/// <fieldType name="text_filter_none_date" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="solr.DateRecognizerFilterFactory" datePattern="yyyy/mm/dd" locale="en-US" />
///   </analyzer>
/// </fieldType>
/// ```
///
/// The `datePattern` is optional. If omitted, `DateRecognizerFilter` will be
/// created with the default date format of the system. The `locale` is
/// optional and if omitted the filter will be created with `Locale.ENGLISH`.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.DateRecognizerFilterFactory` (since 5.5.0).
#[derive(Debug)]
pub struct DateRecognizerFilterFactory {
    base: AbstractAnalysisFactory,
    date_format: Arc<SimpleDateFormat>,
    locale: Locale,
}

impl DateRecognizerFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `DateRecognizerFilterFactory.NAME`.
    pub const NAME: &'static str = "dateRecognizer";

    /// Equivalent to `DateRecognizerFilterFactory.DATE_PATTERN`.
    pub const DATE_PATTERN: &'static str = "datePattern";

    /// Equivalent to `DateRecognizerFilterFactory.LOCALE`.
    pub const LOCALE: &'static str = "locale";

    /// Creates a new DateRecognizerFilterFactory.
    ///
    /// Equivalent to `DateRecognizerFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, the `IllformedLocaleException` of an ill-formed `locale` tag or of its region override (`-u-rg-`), the `IllegalArgumentException` of an invalid `datePattern`, and `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let locale = Self::get_locale(AbstractAnalysisFactory::get(args, Self::LOCALE).as_deref())?;
        let date_format = Arc::new(Self::get_data_format_for(
            &locale,
            AbstractAnalysisFactory::get(args, Self::DATE_PATTERN).as_deref(),
        )?);
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            date_format,
            locale,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `DateRecognizerFilterFactory()`.
    ///
    /// # Errors
    ///
    /// Always: [`AbstractAnalysisFactory::default_ctor_exception`].
    pub fn new_without_args() -> Result<Self> {
        Err(AbstractAnalysisFactory::default_ctor_exception())
    }

    /// The members this factory inherits from `AbstractAnalysisFactory`.
    pub fn base(&self) -> &AbstractAnalysisFactory {
        &self.base
    }

    /// Equivalent to the private `getLocale(String)`: `Locale.ENGLISH` when
    /// absent, otherwise `new Locale.Builder().setLanguageTag(localeStr).build()`.
    fn get_locale(locale_str: Option<&str>) -> Result<Locale> {
        match locale_str {
            None => Ok(Locale::english()),
            Some(locale_str) => Locale::builder_from_language_tag(locale_str),
        }
    }

    /// Equivalent to `getDataFormat(String)`: `new SimpleDateFormat(datePattern,
    /// locale)`, or `DateFormat.getDateInstance(DateFormat.DEFAULT, locale)`
    /// when `date_pattern` is `None`, over this factory's locale.
    ///
    /// # Errors
    ///
    /// The `IllformedLocaleException` of the locale's region override
    /// (`-u-rg-`), and the `IllegalArgumentException` of an invalid pattern.
    pub fn get_data_format(&self, date_pattern: Option<&str>) -> Result<SimpleDateFormat> {
        Self::get_data_format_for(&self.locale, date_pattern)
    }

    /// [`get_data_format`](Self::get_data_format) for the constructor, which
    /// calls it before the factory exists.
    fn get_data_format_for(
        locale: &Locale,
        date_pattern: Option<&str>,
    ) -> Result<SimpleDateFormat> {
        match date_pattern {
            Some(date_pattern) => SimpleDateFormat::new(date_pattern, locale),
            None => SimpleDateFormat::get_date_instance_default(locale),
        }
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`
    /// names it: `DateRecognizerFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.miscellaneous.DateRecognizerFilterFactory",
            SPINameField::Declared(Self::NAME),
            Some(Self::new_from_map),
        )
    }

    /// The `Map<String, String>` constructor, as [`class`](Self::class)
    /// exposes it.
    fn new_from_map(args: &mut HashMap<String, String>) -> Result<Arc<dyn TokenFilterFactory>> {
        Ok(Arc::new(Self::new(args)?))
    }
}

impl TokenFilterFactory for DateRecognizerFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.miscellaneous.DateRecognizerFilterFactory"
    }

    /// Equivalent to `DateRecognizerFilterFactory.create(TokenStream)`, `new
    /// DateRecognizerFilter(input, dateFormat)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(new_date_recognizer_filter_with(
            input,
            Some(Arc::clone(&self.date_format)),
        )?))
    }
}
