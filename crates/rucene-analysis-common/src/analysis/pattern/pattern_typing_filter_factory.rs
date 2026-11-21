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

//! Factory for `PatternTypingFilter`, ported from
//! `org.apache.lucene.analysis.pattern.PatternTypingFilterFactory`.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, SPINameField, TokenFilterFactory,
};
use rucene::analysis::java_regex::JavaPattern;
use rucene::analysis::TokenStream;
use rucene::error::{LuceneError, Result};
use rucene::util::extra::java_parse_int;
use rucene::util::resource_loader::{ResourceLoader, ResourceLoaderAware};

use super::pattern_typing_filter::{new_pattern_typing_filter, PatternTypingRule};
use crate::spi::check_unknown_parameters;

/// The separator `line.split(" ::: ")` compiles: five characters, outside
/// `String.split`'s one-character fast path, so `Pattern.compile(" ::: ")`.
static SEPARATOR: LazyLock<JavaPattern> = LazyLock::new(|| {
    JavaPattern::compile(" ::: ", 0).expect("INVARIANT: the separator is a valid pattern")
});

/// Provides a filter that will analyze tokens with the analyzer from an
/// arbitrary field type. By itself this filter is not very useful. Normally
/// it is combined with a filter that reacts to types or flags.
///
/// ```text
/// <fieldType name="text_taf" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.WhitespaceTokenizerFactory"/>
///     <filter class="com.example.PatternTypingFilter" patternFile="patterns.txt"/>
///     <filter class="solr.TokenAnalyzerFilter" asType="text_en" preserveType="true"/>
///     <filter class="solr.TypeAsSynonymFilterFactory" prefix="__TAS__"
///               ignore="word,&lt;ALPHANUM&gt;,&lt;NUM&gt;,&lt;SOUTHEAST_ASIAN&gt;,&lt;IDEOGRAPHIC&gt;,&lt;HIRAGANA&gt;,&lt;KATAKANA&gt;,&lt;HANGUL&gt;,&lt;EMOJI&gt;"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Note that a configuration such as above may interfere with multi-word
/// synonyms. The patterns file has the format:
///
/// ```text
/// (flags) (pattern) ::: (replacement)
/// ```
///
/// Therefore to set the first 2 flag bits on the original token matching
/// 401k or 401(k) and adding a type of 'legal2_401_k' whenever either one is
/// encountered one would use:
///
/// ```text
/// 3 (\d+)\(?([a-z])\)? ::: legal2_$1_$2
/// ```
///
/// Note that the number indicating the flag bits to set must not have
/// leading spaces and be followed by a single space, and must be 0 if no
/// flags should be set. The flags number should not contain commas or a
/// decimal point. Lines for which the first character is `#` will be
/// ignored as comments. Does not support producing a synonym textually
/// identical to the original term.
///
/// Equivalent to `org.apache.lucene.analysis.pattern.PatternTypingFilterFactory`
/// (since 8.8).
#[derive(Debug)]
pub struct PatternTypingFilterFactory {
    base: AbstractAnalysisFactory,
    pattern_file: String,
    rules: Option<Vec<PatternTypingRule>>,
}

impl PatternTypingFilterFactory {
    /// SPI name.
    ///
    /// Equivalent to `PatternTypingFilterFactory.NAME`.
    pub const NAME: &'static str = "patternTyping";

    /// Creates a new PatternTypingFilterFactory.
    ///
    /// Equivalent to `PatternTypingFilterFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] and
    /// [`AbstractAnalysisFactory::require`] return, and
    /// `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let pattern_file = AbstractAnalysisFactory::require(args, "patternFile")?;
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            pattern_file,
            rules: None,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `PatternTypingFilterFactory()`.
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

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`
    /// names it: `PatternTypingFilterFactory.class`, with its `NAME` field
    /// and its `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenFilterFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.pattern.PatternTypingFilterFactory",
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

impl ResourceLoaderAware for PatternTypingFilterFactory {
    /// Equivalent to `PatternTypingFilterFactory.inform(ResourceLoader)`.
    ///
    /// # Errors
    ///
    /// What [`AbstractAnalysisFactory::get_lines`] returns; Java's
    /// `StringIndexOutOfBoundsException` for a line without a space and
    /// `NumberFormatException` for flags that are not an `int`; the
    /// `RuntimeException` for a line that does not split into a pattern and
    /// a replacement; and the `PatternSyntaxException` (an
    /// `IllegalArgumentException`) of an invalid pattern, with its message.
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        let mut rule_list = Vec::new();
        let lines = AbstractAnalysisFactory::get_lines(loader, &self.pattern_file)?;
        // format: # regex ::: typename[_$1[_$2 ...]]    (technically _$1 does not need the '_' but it
        // usually makes sense)
        // eg: 2 (\d+\(?([a-z])\)?\(?(\d+)\)? ::: legal3_$1_$2_3
        // which yields legal3_501_c_3 for 501(c)(3) or 501c3 and sets the second lowest bit in flags
        for line in lines {
            // no leading spaces allowed; `line.substring(0, -1)` fails
            // without a space.
            let Some(first_space) = line.find(' ') else {
                return Err(LuceneError::unchecked(
                    "java.lang.StringIndexOutOfBoundsException",
                    format!("begin 0, end -1, length {}", line.encode_utf16().count()),
                ));
            };
            let flags_str = &line[..first_space];
            let flags_val = java_parse_int(flags_str)
                .ok_or_else(|| LuceneError::number_format_for_input_string(flags_str, 10))?;
            let line = &line[first_space + 1..];
            let units: Vec<u16> = line.encode_utf16().collect();
            let split = SEPARATOR.split(&units, 0); // arbitrary, unlikely to occur in a useful regex easy to read
            if split.len() != 2 {
                return Err(LuceneError::unchecked(
                    "java.lang.RuntimeException",
                    "The PatternTypingFilter: Always two there are, no more, no less, a pattern and a replacement (separated by ' ::: ' )",
                ));
            }
            let compiled = JavaPattern::compile(&String::from_utf16_lossy(&split[0]), 0)
                .map_err(|syntax| LuceneError::illegal_argument(syntax.to_string()))?;
            rule_list.push(PatternTypingRule::new(
                compiled,
                flags_val,
                String::from_utf16_lossy(&split[1]),
            ));
        }
        self.rules = Some(rule_list);
        Ok(())
    }
}

impl TokenFilterFactory for PatternTypingFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.pattern.PatternTypingFilterFactory"
    }

    /// Equivalent to `PatternTypingFilterFactory.create(TokenStream)`, `new
    /// PatternTypingFilter(input, rules)`.
    ///
    /// # Errors
    ///
    /// Java's `NullPointerException` "replacementAndFlagByPattern" when the
    /// factory was not informed (its `rules` is still `null`), and what the
    /// filter's construction returns.
    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        let Some(rules) = &self.rules else {
            return Err(LuceneError::null_pointer_with_message(
                "replacementAndFlagByPattern",
            ));
        };
        Ok(Box::new(new_pattern_typing_filter(input, rules)?))
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}
