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
//! Factory for `TruncateTokenFilter`, ported from
//! `org.apache.lucene.analysis.miscellaneous.TruncateTokenFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::{LuceneError, Result};
use rucene::util::Version;

use super::truncate_token_filter::{
    truncate_after_chars, truncate_after_code_points, TruncateTokenFilter,
};
use crate::spi::java_map_to_string;

/// The `BiFunction<TokenStream, Integer, TruncateTokenFilter>` the factory
/// picks from its parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TruncateFactory {
    /// `TruncateTokenFilter::truncateAfterCodePoints`.
    CodePoints,
    /// `TruncateTokenFilter::truncateAfterChars`.
    Chars,
    /// `this::legacyPrefixLengthFactory`.
    LegacyPrefixLength,
}

/// Factory for `TruncateTokenFilter`. The following type is recommended for
/// "*diacritics-insensitive search*" for Turkish.
///
/// ```text
/// <fieldType name="text_tr_ascii_f5" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.StandardTokenizerFactory"/>
///     <filter class="solr.ApostropheFilterFactory"/>
///     <filter class="solr.TurkishLowerCaseFilterFactory"/>
///     <filter class="solr.ASCIIFoldingFilterFactory" preserveOriginal="true"/>
///     <filter class="solr.KeywordRepeatFilterFactory"/>
///     <filter class="solr.TruncateTokenFilterFactory" truncateAfterCodePoints="5"/>
///     <filter class="solr.RemoveDuplicatesTokenFilterFactory"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.TruncateTokenFilterFactory`
/// (since 4.8.0).
#[derive(Debug)]
pub struct TruncateTokenFilterFactory {
    base: AbstractAnalysisFactory,
    truncate_after: i32,
    factory: TruncateFactory,
}

impl TruncateTokenFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `TruncateTokenFilterFactory.NAME`.
    pub const NAME: &'static str = "truncate";

    /// Equivalent to the deprecated `TruncateTokenFilterFactory.PREFIX_LENGTH_KEY`:
    /// use [`TRUNCATE_AFTER_CODEPOINTS_KEY`](Self::TRUNCATE_AFTER_CODEPOINTS_KEY)
    /// or [`TRUNCATE_AFTER_CHARS_KEY`](Self::TRUNCATE_AFTER_CHARS_KEY).
    #[deprecated(note = "Use TRUNCATE_AFTER_CODEPOINTS_KEY or TRUNCATE_AFTER_CHARS_KEY")]
    pub const PREFIX_LENGTH_KEY: &'static str = "prefixLength";

    /// Equivalent to `TruncateTokenFilterFactory.TRUNCATE_AFTER_CODEPOINTS_KEY`.
    pub const TRUNCATE_AFTER_CODEPOINTS_KEY: &'static str = "truncateAfterCodePoints";

    /// Equivalent to `TruncateTokenFilterFactory.TRUNCATE_AFTER_CHARS_KEY`.
    pub const TRUNCATE_AFTER_CHARS_KEY: &'static str = "truncateAfterChars";

    /// The `PREFIX_LENGTH_KEY` value, for internal use without the
    /// deprecation warning.
    const PREFIX_LENGTH: &'static str = "prefixLength";

    /// Creates a new TruncateTokenFilterFactory.
    ///
    /// Equivalent to `TruncateTokenFilterFactory(Map<String, String>)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java renders the parameter names in
    /// the order of a `Map.of(...)` key set, which is randomized per JVM run
    /// (`ImmutableCollections.SALT`); the port lists them in declaration
    /// order, `[truncateAfterCodePoints, truncateAfterChars, prefixLength]`,
    /// one of the orders Java can print.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, and
    /// `IllegalArgumentException` with Java's messages for more than one
    /// length parameter, a length below one, an unparsable length, and a
    /// leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let param_mapping = [
            (
                Self::TRUNCATE_AFTER_CODEPOINTS_KEY,
                TruncateFactory::CodePoints,
            ),
            (Self::TRUNCATE_AFTER_CHARS_KEY, TruncateFactory::Chars),
            (Self::PREFIX_LENGTH, TruncateFactory::LegacyPrefixLength),
        ];
        let avail: Vec<(&str, TruncateFactory)> = param_mapping
            .iter()
            .copied()
            .filter(|(key, _)| args.contains_key(*key))
            .collect();
        if avail.len() > 1 {
            let keys: Vec<&str> = param_mapping.iter().map(|(key, _)| *key).collect();
            return Err(LuceneError::illegal_argument(format!(
                "Can only give one of the following parameters: [{}]",
                keys.join(", ")
            )));
        }
        let (param, factory) = avail
            .first()
            .copied()
            .unwrap_or((Self::PREFIX_LENGTH, TruncateFactory::LegacyPrefixLength));
        let truncate_after = AbstractAnalysisFactory::get_int(args, param, 5)?;
        if truncate_after < 1 {
            return Err(LuceneError::illegal_argument(format!(
                "{param} parameter must be a positive number: {truncate_after}"
            )));
        }
        if !args.is_empty() {
            return Err(LuceneError::illegal_argument(format!(
                "Unknown parameter(s): {}",
                java_map_to_string(args)
            )));
        }
        Ok(Self {
            base,
            truncate_after,
            factory,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `TruncateTokenFilterFactory()`.
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

    /// Equivalent to the private `legacyPrefixLengthFactory(TokenStream,
    /// int)`: code points from Lucene 10.5.0 on, chars before.
    fn legacy_prefix_length_factory(
        &self,
        input: Box<dyn TokenStream>,
        prefix_chars: i32,
    ) -> Result<TruncateTokenFilter> {
        if self
            .base
            .get_lucene_match_version()
            .on_or_after(&Version::LUCENE_10_5_0)
        {
            truncate_after_code_points(input, prefix_chars)
        } else {
            truncate_after_chars(input, prefix_chars)
        }
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`
    /// names it: `TruncateTokenFilterFactory.class`, with its `NAME` field
    /// and its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.miscellaneous.TruncateTokenFilterFactory",
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

impl TokenFilterFactory for TruncateTokenFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.miscellaneous.TruncateTokenFilterFactory"
    }

    /// Equivalent to `TruncateTokenFilterFactory.create(TokenStream)`,
    /// `factory.apply(input, truncateAfter)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        let filter = match self.factory {
            TruncateFactory::CodePoints => truncate_after_code_points(input, self.truncate_after)?,
            TruncateFactory::Chars => truncate_after_chars(input, self.truncate_after)?,
            TruncateFactory::LegacyPrefixLength => {
                self.legacy_prefix_length_factory(input, self.truncate_after)?
            }
        };
        Ok(Box::new(filter))
    }
}
