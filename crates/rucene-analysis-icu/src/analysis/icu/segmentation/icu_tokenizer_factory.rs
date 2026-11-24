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

//! Factory for `ICUTokenizer`, ported from
//! `org.apache.lucene.analysis.icu.segmentation.ICUTokenizerFactory`.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use icu4j::lang::u_character::UCharacter;
use icu4j::lang::u_property::UProperty;
use icu4j::text::rule_based_break_iterator::RuleBasedBreakIterator;
use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, FactoryTokenizer, SPINameField, TokenizerFactory,
};
use rucene::analysis::{Tokenizer, TokenizerLogic};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::internal::hppc::IntObjectHashMap;
use rucene::util::attribute::AttributeFactory;
use rucene::util::resource_loader::{ResourceLoader, ResourceLoaderAware};
use rucene_analysis_common::jdk::io::{
    BufferedReader, CodingErrorAction, InputStreamReader, SyncInputStream,
};

use super::default_icu_tokenizer_config::DefaultICUTokenizerConfig;
use super::icu_tokenizer::icu_tokenizer_parts;
use super::icu_tokenizer_config::ICUTokenizerConfig;
use crate::spi::check_unknown_parameters;

/// `RULEFILES`: the name of the argument holding the per-script rule files.
pub(crate) const RULEFILES: &str = "rulefiles";

/// Factory for `ICUTokenizer`. Words are broken across script boundaries,
/// then segmented according to the BreakIterator and typing provided by the
/// [`DefaultICUTokenizerConfig`].
///
/// To use the default set of per-script rules:
///
/// ```text
/// <fieldType name="text_icu" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.ICUTokenizerFactory"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// You can customize this tokenizer's behavior by specifying per-script
/// rule files, which are compiled by the ICU RuleBasedBreakIterator. See the
/// [ICU RuleBasedBreakIterator syntax
/// reference](http://userguide.icu-project.org/boundaryanalysis#TOC-RBBI-Rules).
///
/// To add per-script rules, add a "rulefiles" argument, which should contain
/// a comma-separated list of `code:rulefile` pairs in the following format:
/// [four-letter ISO 15924 script
/// code](http://unicode.org/iso15924/iso15924-codes.html), followed by a
/// colon, then a resource path. E.g. to specify rules for Latin (script code
/// "Latn") and Cyrillic (script code "Cyrl"):
///
/// ```text
/// <fieldType name="text_icu_custom" class="solr.TextField" positionIncrementGap="100">
///   <analyzer>
///     <tokenizer class="solr.ICUTokenizerFactory" cjkAsWords="true"
///                rulefiles="Latn:my.Latin.rules.rbbi,Cyrl:my.Cyrillic.rules.rbbi"/>
///   </analyzer>
/// </fieldType>
/// ```
///
/// Equivalent to `org.apache.lucene.analysis.icu.segmentation.ICUTokenizerFactory`
/// (since 3.1, `@lucene.spi icu`), which implements `ResourceLoaderAware`.
#[derive(Debug)]
pub struct ICUTokenizerFactory {
    base: AbstractAnalysisFactory,
    tailored: IntObjectHashMap<String>,
    config: Option<Arc<dyn ICUTokenizerConfig>>,
    cjk_as_words: bool,
    myanmar_as_words: bool,
}

/// `String.trim()`: the string without its leading and trailing characters
/// at or below U+0020.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= '\u{20}')
}

impl ICUTokenizerFactory {
    /// SPI name.
    ///
    /// Equivalent to `ICUTokenizerFactory.NAME`.
    pub const NAME: &'static str = "icu";

    /// Creates a new ICUTokenizerFactory.
    ///
    /// Equivalent to `ICUTokenizerFactory(Map<String, String>)`.
    ///
    /// # Errors
    ///
    /// Returns what [`AbstractAnalysisFactory::new`] returns; the
    /// `StringIndexOutOfBoundsException` of a rule file entry without a
    /// colon (`substring(0, -1)`); the `IllegalArgumentException` of an
    /// unknown script code (`UCharacter.getPropertyValueEnum`); and
    /// `IllegalArgumentException` for a leftover argument.
    pub fn new(args: &mut HashMap<String, String>) -> Result<Self> {
        let base = AbstractAnalysisFactory::new(args)?;
        let mut tailored = IntObjectHashMap::new();
        let rulefiles_arg = AbstractAnalysisFactory::get(args, RULEFILES);
        if let Some(rulefiles_arg) = rulefiles_arg {
            let script_and_resource_paths =
                AbstractAnalysisFactory::split_file_names(Some(&rulefiles_arg));
            for script_and_resource_path in &script_and_resource_paths {
                let Some(colon_pos) = script_and_resource_path.find(':') else {
                    let length = script_and_resource_path.encode_utf16().count();
                    return Err(LuceneError::from(LuceneErrorKind::StringIndexOutOfBounds(
                        format!("begin 0, end -1, length {length}"),
                    )));
                };
                let script_code = java_trim(&script_and_resource_path[..colon_pos]);
                let resource_path = java_trim(&script_and_resource_path[colon_pos + 1..]);
                let script_units: Vec<u16> = script_code.encode_utf16().collect();
                tailored.put(
                    UCharacter::get_property_value_enum(UProperty::SCRIPT, &script_units)?,
                    resource_path.to_string(),
                );
            }
        }
        let cjk_as_words = AbstractAnalysisFactory::get_boolean(args, "cjkAsWords", true);
        let myanmar_as_words = AbstractAnalysisFactory::get_boolean(args, "myanmarAsWords", true);
        check_unknown_parameters(args)?;
        Ok(Self {
            base,
            tailored,
            config: None,
            cjk_as_words,
            myanmar_as_words,
        })
    }

    /// Default ctor for compatibility with SPI.
    ///
    /// Equivalent to `ICUTokenizerFactory()`.
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

    /// The config `inform` built, `None` before (Java's private field
    /// `config`, `null` before).
    pub fn config(&self) -> Option<&Arc<dyn ICUTokenizerConfig>> {
        self.config.as_ref()
    }

    /// Equivalent to `parseRules(String, ResourceLoader)`: the rule file
    /// `filename`, its lines starting with `#` emptied, compiled into a
    /// break iterator.
    ///
    /// # Errors
    ///
    /// The `IOException` of opening or decoding the file (strictly, as
    /// `IOUtils.getDecodingReader` decodes), and the
    /// `IllegalArgumentException` of a rule syntax error.
    fn parse_rules(filename: &str, loader: &dyn ResourceLoader) -> Result<RuleBasedBreakIterator> {
        let mut rules: Vec<u16> = Vec::new();
        let rules_stream = loader.open_resource(filename)?;
        let mut reader = BufferedReader::new(InputStreamReader::with_action(
            SyncInputStream::new(rules_stream),
            CodingErrorAction::Report,
        ));
        while let Some(line) = reader.read_line()? {
            if !line.starts_with(&[u16::from(b'#')]) {
                rules.extend_from_slice(&line);
            }
            rules.push(u16::from(b'\n'));
        }
        reader.close()?;
        RuleBasedBreakIterator::new(&rules)
    }

    /// The factory's class, as `META-INF/services/org.apache.lucene.analysis.TokenizerFactory`
    /// names it: `ICUTokenizerFactory.class`, with its `NAME` field and its
    /// `Map<String, String>` constructor.
    pub const fn class() -> AnalysisFactoryClass<dyn TokenizerFactory> {
        AnalysisFactoryClass::new(
            "org.apache.lucene.analysis.icu.segmentation.ICUTokenizerFactory",
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

/// The anonymous subclass of `DefaultICUTokenizerConfig` that
/// `ICUTokenizerFactory.inform` builds over the tailored rules: its
/// `getBreakIterator` answers a clone of the tailored iterator of a script
/// when there is one, and `super.getBreakIterator(script)` otherwise.
#[derive(Debug)]
struct TailoredICUTokenizerConfig {
    base: DefaultICUTokenizerConfig,
    breakers: Vec<Option<RuleBasedBreakIterator>>,
}

impl ICUTokenizerConfig for TailoredICUTokenizerConfig {
    fn get_break_iterator(&self, script: i32) -> RuleBasedBreakIterator {
        match &self.breakers[script as usize] {
            Some(breaker) => breaker.clone(),
            None => self.base.get_break_iterator(script),
        }
    }

    // TODO: we could also allow codes->types mapping

    /// The inherited `DefaultICUTokenizerConfig.getType(int, int)`.
    fn get_type(&self, script: i32, rule_status: i32) -> Cow<'static, str> {
        self.base.get_type(script, rule_status)
    }

    /// The inherited `DefaultICUTokenizerConfig.combineCJ()`.
    fn combine_cj(&self) -> bool {
        self.base.combine_cj()
    }
}

impl ResourceLoaderAware for ICUTokenizerFactory {
    /// Equivalent to `ICUTokenizerFactory.inform(ResourceLoader)`.
    fn inform(&mut self, loader: &dyn ResourceLoader) -> Result<()> {
        if self.tailored.is_empty() {
            self.config = Some(Arc::new(DefaultICUTokenizerConfig::new(
                self.cjk_as_words,
                self.myanmar_as_words,
            )));
        } else {
            let size = 1 + UCharacter::get_int_property_max_value(UProperty::SCRIPT);
            let mut breakers: Vec<Option<RuleBasedBreakIterator>> =
                (0..size).map(|_| None).collect();
            for entry in self.tailored.iter() {
                let code = entry.key;
                let resource_path = entry.value;
                breakers[code as usize] = Some(Self::parse_rules(resource_path, loader)?);
            }
            self.config = Some(Arc::new(TailoredICUTokenizerConfig {
                base: DefaultICUTokenizerConfig::new(self.cjk_as_words, self.myanmar_as_words),
                breakers,
            }));
        }
        Ok(())
    }
}

impl TokenizerFactory for ICUTokenizerFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.icu.segmentation.ICUTokenizerFactory"
    }

    /// Equivalent to `ICUTokenizerFactory.create(AttributeFactory)`, `new
    /// ICUTokenizer(factory, config)`.
    ///
    /// # Panics
    ///
    /// Before `inform`, with Java's `assert config != null : "inform must be
    /// called first!"`.
    fn create_with_factory(&self, factory: Arc<dyn AttributeFactory>) -> Result<FactoryTokenizer> {
        let config = self.config.as_ref().expect("inform must be called first!");
        let (source, logic) = icu_tokenizer_parts(factory, Arc::clone(config))?;
        let logic: Box<dyn TokenizerLogic> = Box::new(logic);
        Ok(Tokenizer::with_source(source, logic))
    }

    fn as_resource_loader_aware(&mut self) -> Option<&mut dyn ResourceLoaderAware> {
        Some(self)
    }
}
