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

//! Factory for `PatternTokenizer`, ported from
//! `org.apache.lucene.analysis.pattern.PatternTokenizerFactory`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, FactoryTokenizer, SPINameField, TokenizerFactory,
};
use rucene::analysis::java_regex::JavaPattern;
use rucene::analysis::Tokenizer;
use rucene::error::Result;
use rucene::util::attribute::AttributeFactory;

use super::pattern_tokenizer::pattern_tokenizer_parts;
use crate::spi::check_unknown_parameters;

/// Factory for `PatternTokenizer`. This tokenizer uses regex pattern matching
/// to construct distinct tokens for the input stream. It takes two
/// arguments: "pattern" and "group".
///
/// * "pattern" is the regular expression.
/// * "group" says which group to extract into tokens.
///
/// group=-1 (the default) is equivalent to "split". In this case, the tokens
/// will be equivalent to the output from (without empty tokens):
/// `String#split(java.lang.String)`
///
/// Using group >= 0 selects the matching group as the token. For example, if
/// you have:
///
/// ```text
///  pattern = \'([^\']+)\'
///  group = 0
///  input = aaa 'bbb' 'ccc'
/// ```
///
/// the output will be two tokens: 'bbb' and 'ccc' (including the ' marks).
/// With the same input but using group=1, the output would be: bbb and ccc
/// (no ' marks)
///
/// NOTE: This Tokenizer does not output tokens that are of zero length.
///
/// ```text
/// <fieldType name="text_ptn" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.PatternTokenizerFactory" pattern="\'([^\']+)\'" group="1"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.pattern.PatternTokenizerFactory`
/// (since solr1.2).
#[derive(Debug)]
pub struct PatternTokenizerFactory {
    base: AbstractAnalysisFactory,
    pattern: JavaPattern,
    group: i32,
}

impl PatternTokenizerFactory {
    /// SPI name.
    ///
    /// Equivalent to `PatternTokenizerFactory.NAME`.
    pub const NAME: &'static str = "pattern";

    /// Equivalent to `PatternTokenizerFactory.PATTERN`.
    pub const PATTERN: &'static str = "pattern";

    /// Equivalent to `PatternTokenizerFactory.GROUP`.
    pub const GROUP: &'static str = "group";

    /// Creates a new PatternTokenizerFactory.
    ///
    /// Equivalent to `PatternTokenizerFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns, what
    /// [`AbstractAnalysisFactory::get_java_pattern`] and
    /// [`AbstractAnalysisFactory::get_int`] return, and
    /// `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let pattern = AbstractAnalysisFactory::get_java_pattern(
            args,
            Self::PATTERN,
            "PatternTokenizerFactory",
        )?;
        let group = AbstractAnalysisFactory::get_int(args, Self::GROUP, -1)?;
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            pattern,
            group,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `PatternTokenizerFactory()`.
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

    /// The protected `pattern` field.
    pub fn pattern(&self) -> &JavaPattern {
        &self.pattern
    }

    /// The protected `group` field.
    pub fn group(&self) -> i32 {
        self.group
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenizerFactory`
    /// names it: `PatternTokenizerFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenizerFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.pattern.PatternTokenizerFactory",
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

impl TokenizerFactory for PatternTokenizerFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.pattern.PatternTokenizerFactory"
    }

    /// Split the input using configured pattern.
    ///
    /// Equivalent to `PatternTokenizerFactory.create(AttributeFactory)`, `new
    /// PatternTokenizer(factory, pattern, group)`.
    fn create_with_factory(&self, factory: Arc<dyn AttributeFactory>) -> Result<FactoryTokenizer> {
        let (source, logic) = pattern_tokenizer_parts(factory, &self.pattern, self.group)?;
        Ok(Tokenizer::with_source(source, Box::new(logic)))
    }
}
