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

//! The `META-INF/services` files of `lucene-analysis-common` 10.5.0: the
//! module's factory registry.
//!
//! Java finds the module's 130 analysis factories because
//! `lucene-analysis-common.jar` carries
//! `META-INF/services/org.apache.lucene.analysis.{CharFilterFactory,TokenFilterFactory,TokenizerFactory}`,
//! which `AnalysisSPILoader` reads through `java.util.ServiceLoader` from the
//! class path. Rucene models class loaders and their service files explicitly
//! ([`ClassLoader`]); [`class_loader`] is the loader that sees this crate's
//! service files, and an application puts the crate "on the class path" the
//! way Java does at run time, with the factories' `reload` methods:
//! `rucene::analysis::factories::reload_char_filters(class_loader())`,
//! `reload_token_filters(class_loader())` and `reload_tokenizers(class_loader())`.
//! The core lookups (`tokenizer_for_name`, `token_filter_for_name`,
//! `char_filter_for_name`, the `*_lookup_class` functions and the
//! `available_*` lists) then find every factory of the module under its
//! Lucene SPI `NAME`.
//!
//! The reload applies `AnalysisSPILoader`'s rules (`AnalysisSPILoader::reload`):
//! names are matched case-insensitively, must match `^[a-zA-Z][a-zA-Z0-9_]+$`,
//! and the first class registered under a name wins; the loader is a child of
//! `lucene-core`'s, so its scan lists `lucene-core`'s factories first, as a
//! class path holding both jars in that order does.
//!
//! The registry holds the factories ported so far: a factory is registered by
//! adding its line to the private `services` list, at the position its class has in its
//! Java service file.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use rucene::analysis::factories::{
    AnalysisFactoryClass, CharFilterFactory, TokenFilterFactory, TokenizerFactory,
};
use rucene::analysis::{Analyzer, StandardAnalyzer};
use rucene::error::{LuceneError, Result};
use rucene::util::java_collections::JavaHashMap;
use rucene::util::resource_loader::{ClassLoader, ClassRegistry, ServiceProvider};
use rucene::util::spi::lucene_core_class_loader;

use crate::analysis::core::decimal_digit_filter_factory::DecimalDigitFilterFactory;
use crate::analysis::core::flatten_graph_filter_factory::FlattenGraphFilterFactory;
use crate::analysis::core::keyword_analyzer::KeywordAnalyzer;
use crate::analysis::core::keyword_tokenizer_factory::KeywordTokenizerFactory;
use crate::analysis::core::letter_tokenizer_factory::LetterTokenizerFactory;
use crate::analysis::core::lower_case_filter_factory::LowerCaseFilterFactory;
use crate::analysis::core::simple_analyzer::SimpleAnalyzer;
use crate::analysis::core::stop_filter_factory::StopFilterFactory;
use crate::analysis::core::type_token_filter_factory::TypeTokenFilterFactory;
use crate::analysis::core::unicode_whitespace_analyzer::UnicodeWhitespaceAnalyzer;
use crate::analysis::core::upper_case_filter_factory::UpperCaseFilterFactory;
use crate::analysis::core::whitespace_analyzer::WhitespaceAnalyzer;
use crate::analysis::core::whitespace_tokenizer_factory::WhitespaceTokenizerFactory;
use crate::analysis::en::english_analyzer::EnglishAnalyzer;
use crate::analysis::en::english_minimal_stem_filter_factory::EnglishMinimalStemFilterFactory;
use crate::analysis::en::english_possessive_filter_factory::EnglishPossessiveFilterFactory;
use crate::analysis::en::kstem_filter_factory::KStemFilterFactory;
use crate::analysis::en::porter_stem_filter_factory::PorterStemFilterFactory;
use crate::analysis::miscellaneous::ascii_folding_filter_factory::ASCIIFoldingFilterFactory;
use crate::analysis::miscellaneous::capitalization_filter_factory::CapitalizationFilterFactory;
use crate::analysis::miscellaneous::codepoint_count_filter_factory::CodepointCountFilterFactory;
use crate::analysis::miscellaneous::concatenate_graph_filter_factory::ConcatenateGraphFilterFactory;
use crate::analysis::miscellaneous::date_recognizer_filter_factory::DateRecognizerFilterFactory;
use crate::analysis::miscellaneous::delimited_term_frequency_token_filter_factory::DelimitedTermFrequencyTokenFilterFactory;
use crate::analysis::miscellaneous::drop_if_flagged_filter_factory::DropIfFlaggedFilterFactory;
use crate::analysis::miscellaneous::fingerprint_filter_factory::FingerprintFilterFactory;
use crate::analysis::miscellaneous::fix_broken_offsets_filter_factory::FixBrokenOffsetsFilterFactory;
use crate::analysis::miscellaneous::hyphenated_words_filter_factory::HyphenatedWordsFilterFactory;
use crate::analysis::miscellaneous::keep_word_filter_factory::KeepWordFilterFactory;
use crate::analysis::miscellaneous::keyword_marker_filter_factory::KeywordMarkerFilterFactory;
use crate::analysis::miscellaneous::keyword_repeat_filter_factory::KeywordRepeatFilterFactory;
use crate::analysis::miscellaneous::length_filter_factory::LengthFilterFactory;
use crate::analysis::miscellaneous::limit_token_count_filter_factory::LimitTokenCountFilterFactory;
use crate::analysis::miscellaneous::limit_token_offset_filter_factory::LimitTokenOffsetFilterFactory;
use crate::analysis::miscellaneous::limit_token_position_filter_factory::LimitTokenPositionFilterFactory;
use crate::analysis::miscellaneous::protected_term_filter_factory::ProtectedTermFilterFactory;
use crate::analysis::miscellaneous::remove_duplicates_token_filter_factory::RemoveDuplicatesTokenFilterFactory;
use crate::analysis::miscellaneous::scandinavian_folding_filter_factory::ScandinavianFoldingFilterFactory;
use crate::analysis::miscellaneous::scandinavian_normalization_filter_factory::ScandinavianNormalizationFilterFactory;
use crate::analysis::miscellaneous::stemmer_override_filter_factory::StemmerOverrideFilterFactory;
use crate::analysis::miscellaneous::trim_filter_factory::TrimFilterFactory;
use crate::analysis::miscellaneous::truncate_token_filter_factory::TruncateTokenFilterFactory;
use crate::analysis::miscellaneous::type_as_synonym_filter_factory::TypeAsSynonymFilterFactory;
use crate::analysis::miscellaneous::word_delimiter_filter_factory::WordDelimiterFilterFactory;
use crate::analysis::miscellaneous::word_delimiter_graph_filter_factory::WordDelimiterGraphFilterFactory;
use crate::analysis::synonym::solr_synonym_parser::SolrSynonymParser;
use crate::analysis::synonym::synonym_filter_factory::SynonymFilterFactory;
use crate::analysis::synonym::synonym_graph_filter_factory::SynonymGraphFilterFactory;
use crate::analysis::synonym::synonym_map::Parser;
use crate::analysis::synonym::word2vec::word2vec_synonym_filter_factory::Word2VecSynonymFilterFactory;
use crate::analysis::synonym::wordnet_synonym_parser::WordnetSynonymParser;
use crate::analysis::util::elision_filter_factory::ElisionFilterFactory;

/// The binary name of the char filter factory service, the name of its file.
const CHAR_FILTER_FACTORY: &str = "org.apache.lucene.analysis.CharFilterFactory";

/// The binary name of the token filter factory service, the name of its file.
const TOKEN_FILTER_FACTORY: &str = "org.apache.lucene.analysis.TokenFilterFactory";

/// The binary name of the tokenizer factory service, the name of its file.
const TOKENIZER_FACTORY: &str = "org.apache.lucene.analysis.TokenizerFactory";

/// One line of `META-INF/services/org.apache.lucene.analysis.CharFilterFactory`,
/// naming `class`.
fn char_filter(class: AnalysisFactoryClass<dyn CharFilterFactory>) -> ServiceProvider {
    ServiceProvider::new(CHAR_FILTER_FACTORY, class.get_name(), class)
}

/// One line of `META-INF/services/org.apache.lucene.analysis.TokenFilterFactory`,
/// naming `class`.
fn token_filter(class: AnalysisFactoryClass<dyn TokenFilterFactory>) -> ServiceProvider {
    ServiceProvider::new(TOKEN_FILTER_FACTORY, class.get_name(), class)
}

/// One line of `META-INF/services/org.apache.lucene.analysis.TokenizerFactory`,
/// naming `class`.
fn tokenizer(class: AnalysisFactoryClass<dyn TokenizerFactory>) -> ServiceProvider {
    ServiceProvider::new(TOKENIZER_FACTORY, class.get_name(), class)
}

/// The lines of the module's three service files, each file's lines in its
/// order (`lucene/analysis/common/src/resources/META-INF/services/`), for the
/// factories ported so far.
fn services() -> Vec<ServiceProvider> {
    vec![
        // META-INF/services/org.apache.lucene.analysis.CharFilterFactory
        // META-INF/services/org.apache.lucene.analysis.TokenFilterFactory
        token_filter(DecimalDigitFilterFactory::class()),
        token_filter(LowerCaseFilterFactory::class()),
        token_filter(StopFilterFactory::class()),
        token_filter(TypeTokenFilterFactory::class()),
        token_filter(UpperCaseFilterFactory::class()),
        token_filter(EnglishMinimalStemFilterFactory::class()),
        token_filter(EnglishPossessiveFilterFactory::class()),
        token_filter(KStemFilterFactory::class()),
        token_filter(PorterStemFilterFactory::class()),
        token_filter(ASCIIFoldingFilterFactory::class()),
        token_filter(CapitalizationFilterFactory::class()),
        token_filter(CodepointCountFilterFactory::class()),
        token_filter(ConcatenateGraphFilterFactory::class()),
        token_filter(DateRecognizerFilterFactory::class()),
        token_filter(DelimitedTermFrequencyTokenFilterFactory::class()),
        token_filter(DropIfFlaggedFilterFactory::class()),
        token_filter(FingerprintFilterFactory::class()),
        token_filter(FixBrokenOffsetsFilterFactory::class()),
        token_filter(HyphenatedWordsFilterFactory::class()),
        token_filter(KeepWordFilterFactory::class()),
        token_filter(KeywordMarkerFilterFactory::class()),
        token_filter(KeywordRepeatFilterFactory::class()),
        token_filter(LengthFilterFactory::class()),
        token_filter(LimitTokenCountFilterFactory::class()),
        token_filter(LimitTokenOffsetFilterFactory::class()),
        token_filter(LimitTokenPositionFilterFactory::class()),
        token_filter(RemoveDuplicatesTokenFilterFactory::class()),
        token_filter(StemmerOverrideFilterFactory::class()),
        token_filter(ProtectedTermFilterFactory::class()),
        token_filter(TrimFilterFactory::class()),
        token_filter(TruncateTokenFilterFactory::class()),
        token_filter(TypeAsSynonymFilterFactory::class()),
        token_filter(WordDelimiterFilterFactory::class()),
        token_filter(WordDelimiterGraphFilterFactory::class()),
        token_filter(ScandinavianFoldingFilterFactory::class()),
        token_filter(ScandinavianNormalizationFilterFactory::class()),
        token_filter(SynonymFilterFactory::class()),
        token_filter(SynonymGraphFilterFactory::class()),
        token_filter(Word2VecSynonymFilterFactory::class()),
        token_filter(FlattenGraphFilterFactory::class()),
        token_filter(ElisionFilterFactory::class()),
        // META-INF/services/org.apache.lucene.analysis.TokenizerFactory
        tokenizer(KeywordTokenizerFactory::class()),
        tokenizer(LetterTokenizerFactory::class()),
        tokenizer(WhitespaceTokenizerFactory::class()),
    ]
}

/// The class loader that sees `lucene-analysis-common`'s
/// `META-INF/services` files, a child of `lucene-core`'s
/// ([`lucene_core_class_loader`]), so that a scan of it lists
/// `lucene-core`'s providers first, as a class path holding both jars in
/// that order does. See the module documentation for the factories it holds
/// and how an application registers them.
pub fn class_loader() -> &'static Arc<ClassLoader> {
    static LOADER: LazyLock<Arc<ClassLoader>> = LazyLock::new(|| {
        ClassLoader::with_services(
            "lucene-analysis-common",
            Some(Arc::clone(lucene_core_class_loader())),
            services(),
        )
    });
    &LOADER
}

/// The check every factory constructor of the module ends with, `if
/// (!args.isEmpty()) throw new IllegalArgumentException("Unknown parameters: "
/// + args)`.
///
/// **Divergence from Lucene 10.5.0.** Java prints the caller's `HashMap`, in
/// that map's iteration order. The leftovers are rendered here in the order
/// of a `java.util.HashMap` holding them alone, filled in key order: the same
/// order whenever the caller's map had the default capacity and no two
/// leftover keys share a bucket (as `rucene`'s `StandardTokenizerFactory`
/// renders them).
pub(crate) fn check_unknown_parameters(args: &HashMap<String, String>) -> Result<()> {
    if args.is_empty() {
        return Ok(());
    }
    Err(LuceneError::illegal_argument(format!(
        "Unknown parameters: {}",
        java_map_to_string(args)
    )))
}

/// The factory's argument map as a `java.util.HashMap` holding it alone,
/// filled in key order: the order [`check_unknown_parameters`] declares.
fn java_hash_map(args: &HashMap<String, String>) -> JavaHashMap<String> {
    let mut leftovers: Vec<(&String, &String)> = args.iter().collect();
    leftovers.sort();
    let mut map = JavaHashMap::new();
    for (key, value) in leftovers {
        map.insert(key.clone(), value.clone());
    }
    map
}

/// The `toString()` of the factory's argument map, `{key=value, ...}`, in
/// the order [`check_unknown_parameters`] declares.
pub(crate) fn java_map_to_string(args: &HashMap<String, String>) -> String {
    let rendered = java_hash_map(args)
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{{rendered}}}")
}

/// The keys of the factory's argument map in the iteration order of
/// `args.keySet()`, as [`check_unknown_parameters`] declares it.
pub(crate) fn java_map_keys(args: &HashMap<String, String>) -> Vec<String> {
    java_hash_map(args)
        .iter()
        .map(|(key, _)| key.clone())
        .collect()
}

/// The constructor `(boolean dedup, boolean expand, Analyzer analyzer)` a
/// `SynonymMap.Parser` class offers to the synonym factories
/// (`SynonymFilterFactory.loadSynonyms`).
pub type SynonymParserConstructor = fn(bool, bool, Arc<dyn Analyzer>) -> Box<dyn Parser>;

/// The `SynonymMap.Parser` classes the synonym factories' `format` names:
/// what `ResourceLoader.findClass(cname, SynonymMap.Parser.class)` finds.
///
/// **Divergence from Lucene 10.5.0.** Java loads any class of the class
/// path by name; Rust has no runtime class loading, so the classes are the
/// entries of this table (the `rucene` stand-in for `findClass`, see
/// `rucene::util::resource_loader`): this module's
/// `org.apache.lucene.analysis.synonym.SolrSynonymParser` and
/// `org.apache.lucene.analysis.synonym.WordnetSynonymParser`, and what an
/// application registers. A name the table lacks fails as
/// [`ClassRegistry::find_class`] does.
pub fn synonym_parser_classes() -> &'static RwLock<ClassRegistry<SynonymParserConstructor>> {
    static CLASSES: LazyLock<RwLock<ClassRegistry<SynonymParserConstructor>>> =
        LazyLock::new(|| {
            let mut classes =
                ClassRegistry::new("org.apache.lucene.analysis.synonym.SynonymMap$Parser");
            classes.register(
                "org.apache.lucene.analysis.synonym.SolrSynonymParser",
                Arc::new(|| -> SynonymParserConstructor {
                    |dedup, expand, analyzer| {
                        Box::new(SolrSynonymParser::new(dedup, expand, analyzer))
                    }
                }),
            );
            classes.register(
                "org.apache.lucene.analysis.synonym.WordnetSynonymParser",
                Arc::new(|| -> SynonymParserConstructor {
                    |dedup, expand, analyzer| {
                        Box::new(WordnetSynonymParser::new(dedup, expand, analyzer))
                    }
                }),
            );
            RwLock::new(classes)
        });
    &CLASSES
}

/// The `Analyzer` classes the synonym factories' `analyzer` names, each
/// instantiated through its public no-argument constructor: what
/// `ResourceLoader.findClass(cname, Analyzer.class)` and
/// `getConstructor().newInstance()` find.
///
/// **Divergence from Lucene 10.5.0.** As for [`synonym_parser_classes`]:
/// the table holds the analyzers with a public no-argument constructor of
/// `lucene-core` (`StandardAnalyzer`) and of this module ported so far
/// (`KeywordAnalyzer`, `SimpleAnalyzer`, `UnicodeWhitespaceAnalyzer`,
/// `WhitespaceAnalyzer`, `EnglishAnalyzer`), and what an application
/// registers; the module's later analyzers are added as they are ported.
pub fn analyzer_classes() -> &'static RwLock<ClassRegistry<Arc<dyn Analyzer>>> {
    static CLASSES: LazyLock<RwLock<ClassRegistry<Arc<dyn Analyzer>>>> = LazyLock::new(|| {
        let mut classes = ClassRegistry::new("org.apache.lucene.analysis.Analyzer");
        classes.register(
            "org.apache.lucene.analysis.standard.StandardAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(StandardAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.core.KeywordAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(KeywordAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.core.SimpleAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(SimpleAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.core.UnicodeWhitespaceAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(UnicodeWhitespaceAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.core.WhitespaceAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(WhitespaceAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.en.EnglishAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(EnglishAnalyzer::new()) }),
        );
        RwLock::new(classes)
    });
    &CLASSES
}

/// The `TokenizerFactory` classes the synonym factories'
/// `tokenizerFactory` names: what `ResourceLoader.findClass(cname,
/// TokenizerFactory.class)` finds.
///
/// **Divergence from Lucene 10.5.0.** As for [`synonym_parser_classes`]:
/// the classes are those the module's class loader ([`class_loader`]) lists
/// in its `org.apache.lucene.analysis.TokenizerFactory` service files, its
/// parents' included, keyed by binary name.
pub fn tokenizer_factory_classes(
) -> ClassRegistry<&'static AnalysisFactoryClass<dyn TokenizerFactory>> {
    let mut classes = ClassRegistry::new("org.apache.lucene.analysis.TokenizerFactory");
    for provider in class_loader().get_resources(TOKENIZER_FACTORY) {
        if let Some(class) = provider.class::<AnalysisFactoryClass<dyn TokenizerFactory>>() {
            classes.register(provider.class_name(), Arc::new(move || class));
        }
    }
    classes
}
