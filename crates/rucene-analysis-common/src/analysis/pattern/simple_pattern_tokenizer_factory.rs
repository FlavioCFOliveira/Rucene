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

//! Factory for `SimplePatternTokenizer`, ported from
//! `org.apache.lucene.analysis.pattern.SimplePatternTokenizerFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, FactoryTokenizer, SPINameField, TokenizerFactory,
};
use rucene::analysis::Tokenizer;
use rucene::error::Result;
use rucene::util::attribute::AttributeFactory;
use rucene::util::automaton::{Automaton, Operations, RegExp, DEFAULT_DETERMINIZE_WORK_LIMIT};

use super::simple_pattern_tokenizer::simple_pattern_tokenizer_parts;
use crate::spi::check_unknown_parameters;

/// Factory for `SimplePatternTokenizer`, for matching tokens based on the
/// provided regexp.
///
/// This tokenizer uses Lucene `RegExp` pattern matching to construct
/// distinct tokens for the input stream. The syntax is more limited than
/// `PatternTokenizer`, but the tokenization is quite a bit faster. It takes
/// two arguments:
///
/// * "pattern" (required) is the regular expression, according to the syntax
///   described at `RegExp`
/// * "determinizeWorkLimit" (optional, default 10000) the limit on total
///   effort spent to determinize the automaton computed from the regexp
///
/// The pattern matches the characters to include in a token (not the split
/// characters), and the matching is greedy such that the longest token
/// matching at a given point is created. Empty tokens are never created.
///
/// For example, to match tokens delimited by simple whitespace characters:
///
/// ```text
/// <fieldType name="text_ptn" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.SimplePatternTokenizerFactory" pattern="[^ \t\r\n]+"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.pattern.SimplePatternTokenizerFactory`
/// (since 6.5.0, `@lucene.experimental`).
#[derive(Debug)]
pub struct SimplePatternTokenizerFactory {
    base: AbstractAnalysisFactory,
    dfa: Automaton,
    determinize_work_limit: i32,
}

impl SimplePatternTokenizerFactory {
    /// SPI name.
    ///
    /// Equivalent to `SimplePatternTokenizerFactory.NAME`.
    pub const NAME: &'static str = "simplePattern";

    /// Equivalent to `SimplePatternTokenizerFactory.PATTERN`.
    pub const PATTERN: &'static str = "pattern";

    /// Creates a new SimplePatternTokenizerFactory.
    ///
    /// Equivalent to `SimplePatternTokenizerFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`],
    /// [`AbstractAnalysisFactory::get_int`] and
    /// [`AbstractAnalysisFactory::require`] return, what `RegExp` returns
    /// for an invalid regular expression, `TooComplexToDeterminizeException`
    /// past the work limit, and `IllegalArgumentException` for a leftover
    /// argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let determinize_work_limit = AbstractAnalysisFactory::get_int(
            args,
            "determinizeWorkLimit",
            DEFAULT_DETERMINIZE_WORK_LIMIT,
        )?;
        let automaton =
            RegExp::new(&AbstractAnalysisFactory::require(args, Self::PATTERN)?)?.to_automaton()?;
        let dfa = Operations::determinize(&automaton, determinize_work_limit)?;
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            dfa,
            determinize_work_limit,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `SimplePatternTokenizerFactory()`.
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

    /// The private `determinizeWorkLimit` field.
    pub fn determinize_work_limit(&self) -> i32 {
        self.determinize_work_limit
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenizerFactory`
    /// names it: `SimplePatternTokenizerFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenizerFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.pattern.SimplePatternTokenizerFactory",
            SPINameField::Declared(Self::NAME),
            Some(Self::new_from_map),
        )
    }

    /// The `Map<String, String>` constructor, as [`class`](Self::class)
    /// exposes it.
    fn new_from_map(args: &mut HashMap<String, String>) -> Result<Arc<dyn TokenizerFactory>> {
        Ok(Arc::new(Self::new(args)?))
    }
}

impl TokenizerFactory for SimplePatternTokenizerFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.pattern.SimplePatternTokenizerFactory"
    }

    /// Equivalent to `SimplePatternTokenizerFactory.create(AttributeFactory)`, `new
    /// SimplePatternTokenizer(factory, dfa)`.
    fn create_with_factory(&self, factory: Arc<dyn AttributeFactory>) -> Result<FactoryTokenizer> {
        let (source, logic) = simple_pattern_tokenizer_parts(factory, self.dfa.clone())?;
        Ok(Tokenizer::with_source(source, Box::new(logic)))
    }
}
