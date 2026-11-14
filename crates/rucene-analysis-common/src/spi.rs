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

use crate::analysis::boost::delimited_boost_token_filter_factory::DelimitedBoostTokenFilterFactory;
use crate::analysis::br::brazilian_analyzer::BrazilianAnalyzer;
use crate::analysis::br::brazilian_stem_filter_factory::BrazilianStemFilterFactory;
use crate::analysis::ca::catalan_analyzer::CatalanAnalyzer;
use crate::analysis::charfilter::html_strip_char_filter_factory::HTMLStripCharFilterFactory;
use crate::analysis::charfilter::mapping_char_filter_factory::MappingCharFilterFactory;
use crate::analysis::cjk::cjk_analyzer::CJKAnalyzer;
use crate::analysis::cjk::cjk_bigram_filter_factory::CJKBigramFilterFactory;
use crate::analysis::cjk::cjk_width_char_filter_factory::CJKWidthCharFilterFactory;
use crate::analysis::cjk::cjk_width_filter_factory::CJKWidthFilterFactory;
use crate::analysis::classic::classic_analyzer::ClassicAnalyzer;
use crate::analysis::classic::classic_filter_factory::ClassicFilterFactory;
use crate::analysis::classic::classic_tokenizer_factory::ClassicTokenizerFactory;
use crate::analysis::commongrams::common_grams_filter_factory::CommonGramsFilterFactory;
use crate::analysis::commongrams::common_grams_query_filter_factory::CommonGramsQueryFilterFactory;
use crate::analysis::compound::dictionary_compound_word_token_filter_factory::DictionaryCompoundWordTokenFilterFactory;
use crate::analysis::compound::hyphenation_compound_word_token_filter_factory::HyphenationCompoundWordTokenFilterFactory;
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
use crate::analysis::da::danish_analyzer::DanishAnalyzer;
use crate::analysis::de::german_analyzer::GermanAnalyzer;
use crate::analysis::de::german_light_stem_filter_factory::GermanLightStemFilterFactory;
use crate::analysis::de::german_minimal_stem_filter_factory::GermanMinimalStemFilterFactory;
use crate::analysis::de::german_normalization_filter_factory::GermanNormalizationFilterFactory;
use crate::analysis::de::german_stem_filter_factory::GermanStemFilterFactory;
use crate::analysis::el::greek_analyzer::GreekAnalyzer;
use crate::analysis::el::greek_lower_case_filter_factory::GreekLowerCaseFilterFactory;
use crate::analysis::el::greek_stem_filter_factory::GreekStemFilterFactory;
use crate::analysis::email::uax29_url_email_analyzer::UAX29URLEmailAnalyzer;
use crate::analysis::email::uax29_url_email_tokenizer_factory::UAX29URLEmailTokenizerFactory;
use crate::analysis::en::english_analyzer::EnglishAnalyzer;
use crate::analysis::en::english_minimal_stem_filter_factory::EnglishMinimalStemFilterFactory;
use crate::analysis::en::english_possessive_filter_factory::EnglishPossessiveFilterFactory;
use crate::analysis::en::kstem_filter_factory::KStemFilterFactory;
use crate::analysis::en::porter_stem_filter_factory::PorterStemFilterFactory;
use crate::analysis::es::spanish_analyzer::SpanishAnalyzer;
use crate::analysis::es::spanish_light_stem_filter_factory::SpanishLightStemFilterFactory;
#[allow(deprecated)]
use crate::analysis::es::spanish_minimal_stem_filter_factory::SpanishMinimalStemFilterFactory;
use crate::analysis::es::spanish_plural_stem_filter_factory::SpanishPluralStemFilterFactory;
use crate::analysis::fr::french_analyzer::FrenchAnalyzer;
use crate::analysis::fr::french_light_stem_filter_factory::FrenchLightStemFilterFactory;
use crate::analysis::fr::french_minimal_stem_filter_factory::FrenchMinimalStemFilterFactory;
use crate::analysis::ga::irish_analyzer::IrishAnalyzer;
use crate::analysis::ga::irish_lower_case_filter_factory::IrishLowerCaseFilterFactory;
use crate::analysis::gl::galician_analyzer::GalicianAnalyzer;
use crate::analysis::gl::galician_minimal_stem_filter_factory::GalicianMinimalStemFilterFactory;
use crate::analysis::gl::galician_stem_filter_factory::GalicianStemFilterFactory;
use crate::analysis::hunspell::hunspell_stem_filter_factory::HunspellStemFilterFactory;
use crate::analysis::id::indonesian_analyzer::IndonesianAnalyzer;
use crate::analysis::id::indonesian_stem_filter_factory::IndonesianStemFilterFactory;
use crate::analysis::it::italian_analyzer::ItalianAnalyzer;
use crate::analysis::it::italian_light_stem_filter_factory::ItalianLightStemFilterFactory;
use crate::analysis::minhash::min_hash_filter_factory::MinHashFilterFactory;
use crate::analysis::miscellaneous::ascii_folding_filter_factory::ASCIIFoldingFilterFactory;
use crate::analysis::miscellaneous::capitalization_filter_factory::CapitalizationFilterFactory;
use crate::analysis::miscellaneous::codepoint_count_filter_factory::CodepointCountFilterFactory;
use crate::analysis::miscellaneous::concatenate_graph_filter_factory::ConcatenateGraphFilterFactory;
use crate::analysis::miscellaneous::conditional_token_filter_factory::ConditionalTokenFilterFactorySubclass;
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
use crate::analysis::ngram::edge_ngram_filter_factory::EdgeNGramFilterFactory;
use crate::analysis::ngram::edge_ngram_tokenizer_factory::EdgeNGramTokenizerFactory;
use crate::analysis::ngram::ngram_filter_factory::NGramFilterFactory;
use crate::analysis::ngram::ngram_tokenizer_factory::NGramTokenizerFactory;
use crate::analysis::nl::dutch_analyzer::DutchAnalyzer;
use crate::analysis::no::norwegian_analyzer::NorwegianAnalyzer;
use crate::analysis::no::norwegian_light_stem_filter_factory::NorwegianLightStemFilterFactory;
use crate::analysis::no::norwegian_minimal_stem_filter_factory::NorwegianMinimalStemFilterFactory;
use crate::analysis::no::norwegian_normalization_filter_factory::NorwegianNormalizationFilterFactory;
use crate::analysis::path::path_hierarchy_tokenizer_factory::PathHierarchyTokenizerFactory;
use crate::analysis::pattern::pattern_capture_group_filter_factory::PatternCaptureGroupFilterFactory;
use crate::analysis::pattern::pattern_replace_char_filter_factory::PatternReplaceCharFilterFactory;
use crate::analysis::pattern::pattern_replace_filter_factory::PatternReplaceFilterFactory;
use crate::analysis::pattern::pattern_tokenizer_factory::PatternTokenizerFactory;
use crate::analysis::pattern::pattern_typing_filter_factory::PatternTypingFilterFactory;
use crate::analysis::pattern::simple_pattern_split_tokenizer_factory::SimplePatternSplitTokenizerFactory;
use crate::analysis::pattern::simple_pattern_tokenizer_factory::SimplePatternTokenizerFactory;
use crate::analysis::payloads::delimited_payload_token_filter_factory::DelimitedPayloadTokenFilterFactory;
use crate::analysis::payloads::float_encoder::FloatEncoder;
use crate::analysis::payloads::identity_encoder::IdentityEncoder;
use crate::analysis::payloads::integer_encoder::IntegerEncoder;
use crate::analysis::payloads::numeric_payload_token_filter_factory::NumericPayloadTokenFilterFactory;
use crate::analysis::payloads::payload_encoder::PayloadEncoder;
use crate::analysis::payloads::token_offset_payload_token_filter_factory::TokenOffsetPayloadTokenFilterFactory;
use crate::analysis::payloads::type_as_payload_token_filter_factory::TypeAsPayloadTokenFilterFactory;
use crate::analysis::pt::portuguese_analyzer::PortugueseAnalyzer;
use crate::analysis::pt::portuguese_light_stem_filter_factory::PortugueseLightStemFilterFactory;
use crate::analysis::pt::portuguese_minimal_stem_filter_factory::PortugueseMinimalStemFilterFactory;
use crate::analysis::pt::portuguese_stem_filter_factory::PortugueseStemFilterFactory;
use crate::analysis::reverse::reverse_string_filter_factory::ReverseStringFilterFactory;
use crate::analysis::ro::romanian_analyzer::RomanianAnalyzer;
use crate::analysis::ro::romanian_normalization_filter_factory::RomanianNormalizationFilterFactory;
use crate::analysis::shingle::fixed_shingle_filter_factory::FixedShingleFilterFactory;
use crate::analysis::shingle::shingle_analyzer_wrapper::ShingleAnalyzerWrapper;
use crate::analysis::shingle::shingle_filter_factory::ShingleFilterFactory;
use crate::analysis::snowball::snowball_porter_filter_factory::SnowballPorterFilterFactory;
use crate::analysis::sv::swedish_analyzer::SwedishAnalyzer;
use crate::analysis::sv::swedish_light_stem_filter_factory::SwedishLightStemFilterFactory;
use crate::analysis::sv::swedish_minimal_stem_filter_factory::SwedishMinimalStemFilterFactory;
use crate::analysis::synonym::solr_synonym_parser::SolrSynonymParser;
use crate::analysis::synonym::synonym_filter_factory::SynonymFilterFactory;
use crate::analysis::synonym::synonym_graph_filter_factory::SynonymGraphFilterFactory;
use crate::analysis::synonym::synonym_map::Parser;
use crate::analysis::synonym::word2vec::word2vec_synonym_filter_factory::Word2VecSynonymFilterFactory;
use crate::analysis::synonym::wordnet_synonym_parser::WordnetSynonymParser;
use crate::analysis::th::thai_analyzer::ThaiAnalyzer;
use crate::analysis::th::thai_tokenizer_factory::ThaiTokenizerFactory;
use crate::analysis::tr::apostrophe_filter_factory::ApostropheFilterFactory;
use crate::analysis::tr::turkish_analyzer::TurkishAnalyzer;
use crate::analysis::tr::turkish_lower_case_filter_factory::TurkishLowerCaseFilterFactory;
use crate::analysis::util::elision_filter_factory::ElisionFilterFactory;
use crate::analysis::wikipedia::wikipedia_tokenizer_factory::WikipediaTokenizerFactory;
use crate::tartarus::snowball::ext::arabic_stemmer::ArabicStemmer;
use crate::tartarus::snowball::ext::armenian_stemmer::ArmenianStemmer;
use crate::tartarus::snowball::ext::basque_stemmer::BasqueStemmer;
use crate::tartarus::snowball::ext::catalan_stemmer::CatalanStemmer;
use crate::tartarus::snowball::ext::danish_stemmer::DanishStemmer;
use crate::tartarus::snowball::ext::dutch_stemmer::DutchStemmer;
use crate::tartarus::snowball::ext::english_stemmer::EnglishStemmer;
use crate::tartarus::snowball::ext::estonian_stemmer::EstonianStemmer;
use crate::tartarus::snowball::ext::finnish_stemmer::FinnishStemmer;
use crate::tartarus::snowball::ext::french_stemmer::FrenchStemmer;
use crate::tartarus::snowball::ext::german_stemmer::GermanStemmer;
use crate::tartarus::snowball::ext::greek_stemmer::GreekStemmer;
use crate::tartarus::snowball::ext::hindi_stemmer::HindiStemmer;
use crate::tartarus::snowball::ext::hungarian_stemmer::HungarianStemmer;
use crate::tartarus::snowball::ext::indonesian_stemmer::IndonesianStemmer;
use crate::tartarus::snowball::ext::irish_stemmer::IrishStemmer;
use crate::tartarus::snowball::ext::italian_stemmer::ItalianStemmer;
use crate::tartarus::snowball::ext::lithuanian_stemmer::LithuanianStemmer;
use crate::tartarus::snowball::ext::nepali_stemmer::NepaliStemmer;
use crate::tartarus::snowball::ext::norwegian_stemmer::NorwegianStemmer;
use crate::tartarus::snowball::ext::porter_stemmer::PorterStemmer;
use crate::tartarus::snowball::ext::portuguese_stemmer::PortugueseStemmer;
use crate::tartarus::snowball::ext::romanian_stemmer::RomanianStemmer;
use crate::tartarus::snowball::ext::russian_stemmer::RussianStemmer;
use crate::tartarus::snowball::ext::serbian_stemmer::SerbianStemmer;
use crate::tartarus::snowball::ext::spanish_stemmer::SpanishStemmer;
use crate::tartarus::snowball::ext::swedish_stemmer::SwedishStemmer;
use crate::tartarus::snowball::ext::tamil_stemmer::TamilStemmer;
use crate::tartarus::snowball::ext::turkish_stemmer::TurkishStemmer;
use crate::tartarus::snowball::ext::yiddish_stemmer::YiddishStemmer;
use crate::tartarus::snowball::snowball_stemmer::SnowballStemmer;

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
// `SpanishMinimalStemFilterFactory` is deprecated in Lucene 10.5.0, and still listed.
#[allow(deprecated)]
fn services() -> Vec<ServiceProvider> {
    vec![
        // META-INF/services/org.apache.lucene.analysis.CharFilterFactory
        char_filter(HTMLStripCharFilterFactory::class()),
        char_filter(MappingCharFilterFactory::class()),
        char_filter(CJKWidthCharFilterFactory::class()),
        char_filter(PatternReplaceCharFilterFactory::class()),
        // META-INF/services/org.apache.lucene.analysis.TokenFilterFactory
        token_filter(ApostropheFilterFactory::class()),
        token_filter(DelimitedBoostTokenFilterFactory::class()),
        token_filter(BrazilianStemFilterFactory::class()),
        token_filter(CJKBigramFilterFactory::class()),
        token_filter(CJKWidthFilterFactory::class()),
        token_filter(ClassicFilterFactory::class()),
        token_filter(CommonGramsFilterFactory::class()),
        token_filter(CommonGramsQueryFilterFactory::class()),
        token_filter(DictionaryCompoundWordTokenFilterFactory::class()),
        token_filter(HyphenationCompoundWordTokenFilterFactory::class()),
        token_filter(DecimalDigitFilterFactory::class()),
        token_filter(LowerCaseFilterFactory::class()),
        token_filter(StopFilterFactory::class()),
        token_filter(TypeTokenFilterFactory::class()),
        token_filter(UpperCaseFilterFactory::class()),
        token_filter(GermanLightStemFilterFactory::class()),
        token_filter(GermanMinimalStemFilterFactory::class()),
        token_filter(GermanNormalizationFilterFactory::class()),
        token_filter(GermanStemFilterFactory::class()),
        token_filter(GreekLowerCaseFilterFactory::class()),
        token_filter(GreekStemFilterFactory::class()),
        token_filter(EnglishMinimalStemFilterFactory::class()),
        token_filter(EnglishPossessiveFilterFactory::class()),
        token_filter(KStemFilterFactory::class()),
        token_filter(PorterStemFilterFactory::class()),
        token_filter(SpanishLightStemFilterFactory::class()),
        token_filter(SpanishMinimalStemFilterFactory::class()),
        token_filter(SpanishPluralStemFilterFactory::class()),
        token_filter(FrenchLightStemFilterFactory::class()),
        token_filter(FrenchMinimalStemFilterFactory::class()),
        token_filter(IrishLowerCaseFilterFactory::class()),
        token_filter(GalicianMinimalStemFilterFactory::class()),
        token_filter(GalicianStemFilterFactory::class()),
        token_filter(HunspellStemFilterFactory::class()),
        token_filter(IndonesianStemFilterFactory::class()),
        token_filter(ItalianLightStemFilterFactory::class()),
        token_filter(MinHashFilterFactory::class()),
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
        token_filter(EdgeNGramFilterFactory::class()),
        token_filter(NGramFilterFactory::class()),
        token_filter(NorwegianLightStemFilterFactory::class()),
        token_filter(NorwegianMinimalStemFilterFactory::class()),
        token_filter(NorwegianNormalizationFilterFactory::class()),
        token_filter(PatternReplaceFilterFactory::class()),
        token_filter(PatternCaptureGroupFilterFactory::class()),
        token_filter(PatternTypingFilterFactory::class()),
        token_filter(DelimitedPayloadTokenFilterFactory::class()),
        token_filter(NumericPayloadTokenFilterFactory::class()),
        token_filter(TokenOffsetPayloadTokenFilterFactory::class()),
        token_filter(TypeAsPayloadTokenFilterFactory::class()),
        token_filter(PortugueseLightStemFilterFactory::class()),
        token_filter(PortugueseMinimalStemFilterFactory::class()),
        token_filter(PortugueseStemFilterFactory::class()),
        token_filter(ReverseStringFilterFactory::class()),
        token_filter(RomanianNormalizationFilterFactory::class()),
        token_filter(ShingleFilterFactory::class()),
        token_filter(FixedShingleFilterFactory::class()),
        token_filter(SnowballPorterFilterFactory::class()),
        token_filter(SwedishLightStemFilterFactory::class()),
        token_filter(SwedishMinimalStemFilterFactory::class()),
        token_filter(SynonymFilterFactory::class()),
        token_filter(SynonymGraphFilterFactory::class()),
        token_filter(Word2VecSynonymFilterFactory::class()),
        token_filter(FlattenGraphFilterFactory::class()),
        token_filter(TurkishLowerCaseFilterFactory::class()),
        token_filter(ElisionFilterFactory::class()),
        // META-INF/services/org.apache.lucene.analysis.TokenizerFactory
        tokenizer(ClassicTokenizerFactory::class()),
        tokenizer(KeywordTokenizerFactory::class()),
        tokenizer(LetterTokenizerFactory::class()),
        tokenizer(WhitespaceTokenizerFactory::class()),
        tokenizer(UAX29URLEmailTokenizerFactory::class()),
        tokenizer(EdgeNGramTokenizerFactory::class()),
        tokenizer(NGramTokenizerFactory::class()),
        tokenizer(PathHierarchyTokenizerFactory::class()),
        tokenizer(PatternTokenizerFactory::class()),
        tokenizer(SimplePatternSplitTokenizerFactory::class()),
        tokenizer(SimplePatternTokenizerFactory::class()),
        tokenizer(ThaiTokenizerFactory::class()),
        tokenizer(WikipediaTokenizerFactory::class()),
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
/// `WhitespaceAnalyzer`, `EnglishAnalyzer`, `CJKAnalyzer`, `ClassicAnalyzer`,
/// `ThaiAnalyzer`, `UAX29URLEmailAnalyzer`, `ShingleAnalyzerWrapper`, and
/// the per-language analyzers of `ca`, `da`, `de`, `es`, `fr`, `gl`, `it`,
/// `nl`, `no`, `pt`, `sv`, `el`, `br`, `tr`, `ro`, `id` and `ga`), and what
/// an application registers; the module's later analyzers are added as they
/// are ported.
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
        classes.register(
            "org.apache.lucene.analysis.cjk.CJKAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(CJKAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.classic.ClassicAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(ClassicAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.th.ThaiAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(ThaiAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.email.UAX29URLEmailAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(UAX29URLEmailAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.shingle.ShingleAnalyzerWrapper",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(ShingleAnalyzerWrapper::new_standard()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.ca.CatalanAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(CatalanAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.da.DanishAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(DanishAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.de.GermanAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(GermanAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.es.SpanishAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(SpanishAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.fr.FrenchAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(FrenchAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.gl.GalicianAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(GalicianAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.it.ItalianAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(ItalianAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.nl.DutchAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(DutchAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.no.NorwegianAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(NorwegianAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.pt.PortugueseAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(PortugueseAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.sv.SwedishAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(SwedishAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.el.GreekAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(GreekAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.br.BrazilianAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(BrazilianAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.tr.TurkishAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(TurkishAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.ro.RomanianAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(RomanianAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.id.IndonesianAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(IndonesianAnalyzer::new()) }),
        );
        classes.register(
            "org.apache.lucene.analysis.ga.IrishAnalyzer",
            Arc::new(|| -> Arc<dyn Analyzer> { Arc::new(IrishAnalyzer::new()) }),
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

/// The public no-argument constructor of a `SnowballStemmer` class: what
/// `getConstructor().newInstance()` calls on the class `SnowballFilter` and
/// `SnowballPorterFilterFactory` load by name.
pub type SnowballStemmerConstructor = fn() -> Box<dyn SnowballStemmer>;

/// `new S()`, the constructor [`snowball_stemmer_classes`] registers for the
/// stemmer class `S`.
fn new_stemmer<S: SnowballStemmer + Default + 'static>() -> Box<dyn SnowballStemmer> {
    Box::new(S::default())
}

/// The `SnowballStemmer` classes of `org.tartarus.snowball.ext`, keyed by
/// binary name, each standing for its public no-argument constructor: what
/// `SnowballFilter(TokenStream, String)` finds with `Class.forName` and
/// `SnowballPorterFilterFactory.inform` with
/// `ResourceLoader.newInstance(className, SnowballStemmer.class)`.
///
/// **Divergence from Lucene 10.5.0.** As for [`synonym_parser_classes`]:
/// Java loads any class of the class path by name; the table holds the 30
/// stemmers of this module and what an application registers.
pub fn snowball_stemmer_classes() -> &'static RwLock<ClassRegistry<SnowballStemmerConstructor>> {
    static CLASSES: LazyLock<RwLock<ClassRegistry<SnowballStemmerConstructor>>> =
        LazyLock::new(|| {
            let mut classes = ClassRegistry::new("org.tartarus.snowball.SnowballStemmer");
            classes.register(
                "org.tartarus.snowball.ext.ArabicStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<ArabicStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.ArmenianStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<ArmenianStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.BasqueStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<BasqueStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.CatalanStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<CatalanStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.DanishStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<DanishStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.DutchStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<DutchStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.EnglishStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<EnglishStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.EstonianStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<EstonianStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.FinnishStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<FinnishStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.FrenchStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<FrenchStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.GermanStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<GermanStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.GreekStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<GreekStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.HindiStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<HindiStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.HungarianStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<HungarianStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.IndonesianStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<IndonesianStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.IrishStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<IrishStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.ItalianStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<ItalianStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.LithuanianStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<LithuanianStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.NepaliStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<NepaliStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.NorwegianStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<NorwegianStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.PorterStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<PorterStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.PortugueseStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<PortugueseStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.RomanianStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<RomanianStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.RussianStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<RussianStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.SerbianStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<SerbianStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.SpanishStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<SpanishStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.SwedishStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<SwedishStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.TamilStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<TamilStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.TurkishStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<TurkishStemmer> }),
            );
            classes.register(
                "org.tartarus.snowball.ext.YiddishStemmer",
                Arc::new(|| -> SnowballStemmerConstructor { new_stemmer::<YiddishStemmer> }),
            );
            RwLock::new(classes)
        });
    &CLASSES
}

/// The `PayloadEncoder` classes `DelimitedPayloadTokenFilterFactory`'s
/// `encoder` names, each instantiated through its public no-argument
/// constructor: what `ResourceLoader.newInstance(cname, PayloadEncoder.class)`
/// finds.
///
/// **Divergence from Lucene 10.5.0.** As for [`synonym_parser_classes`]:
/// Java loads any class of the class path by name; the table holds the
/// encoders of `org.apache.lucene.analysis.payloads` with a public
/// no-argument constructor (`FloatEncoder`, `IdentityEncoder`,
/// `IntegerEncoder`) and what an application registers.
pub fn payload_encoder_classes() -> &'static RwLock<ClassRegistry<Arc<dyn PayloadEncoder>>> {
    static CLASSES: LazyLock<RwLock<ClassRegistry<Arc<dyn PayloadEncoder>>>> =
        LazyLock::new(|| {
            let mut classes =
                ClassRegistry::new("org.apache.lucene.analysis.payloads.PayloadEncoder");
            classes.register(
                "org.apache.lucene.analysis.payloads.FloatEncoder",
                Arc::new(|| -> Arc<dyn PayloadEncoder> { Arc::new(FloatEncoder::new()) }),
            );
            classes.register(
                "org.apache.lucene.analysis.payloads.IdentityEncoder",
                Arc::new(|| -> Arc<dyn PayloadEncoder> { Arc::new(IdentityEncoder::new()) }),
            );
            classes.register(
                "org.apache.lucene.analysis.payloads.IntegerEncoder",
                Arc::new(|| -> Arc<dyn PayloadEncoder> { Arc::new(IntegerEncoder::new()) }),
            );
            RwLock::new(classes)
        });
    &CLASSES
}

/// The `Map<String, String>` constructor of a concrete
/// `ConditionalTokenFilterFactory` class, returning the factory as Java's
/// static type `ConditionalTokenFilterFactory` holds it: what
/// `CustomAnalyzer.Builder.when(Class, Map)` calls through
/// `AnalysisSPILoader.newFactoryClassInstance`.
pub type ConditionalTokenFilterFactoryConstructor =
    fn(&mut HashMap<String, String>) -> Result<Box<dyn ConditionalTokenFilterFactorySubclass>>;

/// The token filter factory classes that extend
/// `ConditionalTokenFilterFactory`, keyed by binary name, each standing for
/// its `Map<String, String>` constructor: the classes for which
/// `ConditionalTokenFilterFactory.class.isAssignableFrom(clazz)` holds in
/// `CustomAnalyzer.Builder.when(String, Map)`.
///
/// **Divergence from Lucene 10.5.0.** Rust has no class hierarchy to query
/// at run time: the table holds this module's concrete
/// `ConditionalTokenFilterFactory`, `ProtectedTermFilterFactory`, and what
/// an application registers.
pub fn conditional_token_filter_factory_classes(
) -> &'static RwLock<ClassRegistry<ConditionalTokenFilterFactoryConstructor>> {
    static CLASSES: LazyLock<RwLock<ClassRegistry<ConditionalTokenFilterFactoryConstructor>>> =
        LazyLock::new(|| {
            let mut classes = ClassRegistry::new(
                "org.apache.lucene.analysis.miscellaneous.ConditionalTokenFilterFactory",
            );
            classes.register(
                "org.apache.lucene.analysis.miscellaneous.ProtectedTermFilterFactory",
                Arc::new(|| -> ConditionalTokenFilterFactoryConstructor {
                    |args| Ok(Box::new(ProtectedTermFilterFactory::new(args)?))
                }),
            );
            RwLock::new(classes)
        });
    &CLASSES
}
