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
//! Factory for `ConcatenateGraphFilter`, ported from
//! `org.apache.lucene.analysis.miscellaneous.ConcatenateGraphFilterFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::TokenStream;
use rucene::error::LuceneError;
use rucene::error::Result;

use super::concatenate_graph_filter::{
    ConcatenateGraphFilter, DEFAULT_MAX_GRAPH_EXPANSIONS, DEFAULT_PRESERVE_POSITION_INCREMENTS,
    DEFAULT_TOKEN_SEPARATOR,
};
use crate::spi::check_unknown_parameters;

/// Factory for `ConcatenateGraphFilter`.
///
/// * `tokenSeparator`: separator to use when concatenating tokens; an empty
///   value means no separator; one character otherwise.
/// * `preservePositionIncrements`: Whether to add an empty token for missing
///   positions. The effect is a consecutive `SEP_LABEL`. When false, it's as
///   if there were no missing positions (we pretend the surrounding tokens were
///   adjacent).
/// * `maxGraphExpansions`: If the tokenStream graph has more than this many
///   possible paths through, then we'll throw `TooComplexToDeterminizeException`
///   to preserve the stability and memory of the machine.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.ConcatenateGraphFilterFactory` (since 7.4.0).
#[derive(Debug)]
pub struct ConcatenateGraphFilterFactory {
    base: AbstractAnalysisFactory,
    token_separator: Option<char>,
    preserve_position_increments: bool,
    max_graph_expansions: i32,
}

impl ConcatenateGraphFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `ConcatenateGraphFilterFactory.NAME`.
    pub const NAME: &'static str = "concatenateGraph";

    /// Creates a new ConcatenateGraphFilterFactory.
    ///
    /// Equivalent to `ConcatenateGraphFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, `IllegalArgumentException` for a `tokenSeparator` longer than one char, what an unparsable `maxGraphExpansions` raises, and `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let token_separator = Self::get_character(args, "tokenSeparator", DEFAULT_TOKEN_SEPARATOR)?;
        let preserve_position_increments = AbstractAnalysisFactory::get_boolean(
            args,
            "preservePositionIncrements",
            DEFAULT_PRESERVE_POSITION_INCREMENTS,
        );
        let max_graph_expansions = AbstractAnalysisFactory::get_int(
            args,
            "maxGraphExpansions",
            DEFAULT_MAX_GRAPH_EXPANSIONS,
        )?;
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            token_separator,
            preserve_position_increments,
            max_graph_expansions,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `ConcatenateGraphFilterFactory()`.
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

    /// Reads an optional `Character` argument: `default_val` when it is
    /// absent, `None` (Java's `null`) when it is empty, its only character
    /// otherwise.
    ///
    /// Equivalent to the protected `ConcatenateGraphFilterFactory.getCharacter(Map,
    /// String, Character)`, whose `s.length() != 1` counts UTF-16 units.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` "name should be a char. \"s\" is invalid"
    /// for a value longer than one UTF-16 unit.
    pub fn get_character(
        args: &mut HashMap<String, String>,
        name: &str,
        default_val: Option<char>,
    ) -> Result<Option<char>> {
        match args.remove(name) {
            None => Ok(default_val),
            Some(s) if s.is_empty() => Ok(None),
            Some(s) => {
                if s.encode_utf16().count() != 1 {
                    Err(LuceneError::illegal_argument(format!(
                        "{name} should be a char. \"{s}\" is invalid"
                    )))
                } else {
                    Ok(s.chars().next())
                }
            }
        }
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`
    /// names it: `ConcatenateGraphFilterFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.miscellaneous.ConcatenateGraphFilterFactory",
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

impl TokenFilterFactory for ConcatenateGraphFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.miscellaneous.ConcatenateGraphFilterFactory"
    }

    /// Equivalent to `ConcatenateGraphFilterFactory.create(TokenStream)`.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(ConcatenateGraphFilter::with_separator(
            input,
            self.token_separator,
            self.preserve_position_increments,
            self.max_graph_expansions,
        )?))
    }
}
