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

//! Default `ICUTokenizerConfig` that is generally applicable to many
//! languages, ported from
//! `org.apache.lucene.analysis.icu.segmentation.DefaultICUTokenizerConfig`.

use std::borrow::Cow;
use std::sync::LazyLock;

use icu4j::lang::u_script::UScript;
use icu4j::text::break_iterator::{
    self, BreakIterator, WORD_IDEO as RBBI_WORD_IDEO, WORD_KANA, WORD_LETTER as RBBI_WORD_LETTER,
    WORD_NUMBER as RBBI_WORD_NUMBER,
};
use icu4j::text::rule_based_break_iterator::RuleBasedBreakIterator;
use icu4j::util::u_locale::ULocale;
use rucene::analysis::standard::{ALPHANUM, EMOJI, HANGUL, HIRAGANA, IDEOGRAPHIC, KATAKANA, NUM};
use rucene::analysis::TOKEN_TYPES;
use rucene::error::{LuceneError, Result};
use rucene::util::IOUtils;

use super::icu_tokenizer_config::{ICUTokenizerConfig, EMOJI_SEQUENCE_STATUS};
use crate::resources::get_resource_as_stream;

/// Token type for words containing ideographic characters.
///
/// Equivalent to `DefaultICUTokenizerConfig.WORD_IDEO`,
/// `StandardTokenizer.TOKEN_TYPES[StandardTokenizer.IDEOGRAPHIC]` (the same
/// string object).
///
/// Each `WORD_*` static is a reference to the element of `TOKEN_TYPES`
/// itself, read at run time: a `&str` static initialized with
/// `TOKEN_TYPES[i]` would be const-evaluated in this crate into a copy of the
/// literal bytes, losing the string identity that `CJKBigramFilter`'s
/// `type == doHan` test needs (rmp #642).
pub static WORD_IDEO: &&str = &TOKEN_TYPES[IDEOGRAPHIC];

/// Token type for words containing Japanese hiragana.
///
/// Equivalent to `DefaultICUTokenizerConfig.WORD_HIRAGANA`.
pub static WORD_HIRAGANA: &&str = &TOKEN_TYPES[HIRAGANA];

/// Token type for words containing Japanese katakana.
///
/// Equivalent to `DefaultICUTokenizerConfig.WORD_KATAKANA`.
pub static WORD_KATAKANA: &&str = &TOKEN_TYPES[KATAKANA];

/// Token type for words containing Korean hangul.
///
/// Equivalent to `DefaultICUTokenizerConfig.WORD_HANGUL`.
pub static WORD_HANGUL: &&str = &TOKEN_TYPES[HANGUL];

/// Token type for words that contain letters.
///
/// Equivalent to `DefaultICUTokenizerConfig.WORD_LETTER`.
pub static WORD_LETTER: &&str = &TOKEN_TYPES[ALPHANUM];

/// Token type for words that appear to be numbers.
///
/// Equivalent to `DefaultICUTokenizerConfig.WORD_NUMBER`.
pub static WORD_NUMBER: &&str = &TOKEN_TYPES[NUM];

/// Token type for words that appear to be emoji sequences.
///
/// Equivalent to `DefaultICUTokenizerConfig.WORD_EMOJI`.
pub static WORD_EMOJI: &&str = &TOKEN_TYPES[EMOJI];

/// The class's three private static break iterators, initialized together
/// the first time the class is referenced, as Java's static initializer
/// runs them (a failure there is Java's `ExceptionInInitializerError`).
struct Statics {
    /*
     * the default breakiterators in use. these can be expensive to
     * instantiate, cheap to clone.
     */
    // we keep the cjk breaking separate, thats because it cannot be customized (because dictionary
    // is only triggered when kind = WORD, but kind = LINE by default and we have no non-evil way to
    // change it)
    cjk_break_iterator: Box<dyn BreakIterator>,

    // TODO: if the wrong version of the ICU jar is used, loading these data files may give a strange
    // error.
    // maybe add an explicit check?
    // http://icu-project.org/apiref/icu4j/com/ibm/icu/util/VersionInfo.html

    // the same as ROOT, except no dictionary segmentation for cjk
    default_break_iterator: RuleBasedBreakIterator,
    myanmar_syllable_iterator: RuleBasedBreakIterator,
}

static STATICS: LazyLock<Statics> = LazyLock::new(|| Statics {
    cjk_break_iterator: break_iterator::get_word_instance_ulocale(&ULocale::root())
        .unwrap_or_else(|e| IOUtils::rethrow_always(e)),
    default_break_iterator: read_break_iterator("Default.brk")
        .unwrap_or_else(|e| IOUtils::rethrow_always(e)),
    myanmar_syllable_iterator: read_break_iterator("MyanmarSyllable.brk")
        .unwrap_or_else(|e| IOUtils::rethrow_always(e)),
});

/// `(RuleBasedBreakIterator) iterator.clone()`.
///
/// # Panics
///
/// With Java's `ClassCastException` when the clone is not a
/// `RuleBasedBreakIterator`.
fn clone_as_rbbi(iterator: &dyn BreakIterator) -> RuleBasedBreakIterator {
    match iterator.as_any().downcast_ref::<RuleBasedBreakIterator>() {
        Some(rbbi) => rbbi.clone(),
        None => rucene::store::java_nio_file::throw_unchecked(
            "java.lang.ClassCastException",
            Some("class cannot be cast to class com.ibm.icu.text.RuleBasedBreakIterator"),
        ),
    }
}

/// Equivalent to `readBreakIterator(String)`: the compiled rules of the
/// resource `filename`, relative to this class's package.
///
/// # Errors
///
/// `RuntimeException` wrapping the `IOException` of reading the rules.
fn read_break_iterator(filename: &str) -> Result<RuleBasedBreakIterator> {
    let mut is = get_resource_as_stream(
        "org.apache.lucene.analysis.icu.segmentation.DefaultICUTokenizerConfig",
        filename,
    )
    .unwrap_or_else(|| {
        // `RuleBasedBreakIterator.getInstanceFromCompiledRules(null)` throws the
        // `NullPointerException` of reading a null stream.
        rucene::store::java_nio_file::throw_unchecked("java.lang.NullPointerException", None)
    });
    match RuleBasedBreakIterator::get_instance_from_compiled_rules_stream(&mut is) {
        Ok(bi) => {
            rucene::store::java_nio_file::InputStream::close(&mut is)
                .map_err(|e| LuceneError::runtime_exception(e.into()))?;
            Ok(bi)
        }
        Err(e) if e.is_io_exception() => Err(LuceneError::runtime_exception(e)),
        Err(e) => Err(e),
    }
}

/// Default [`ICUTokenizerConfig`] that is generally applicable to many
/// languages.
///
/// Generally tokenizes Unicode text according to UAX#29
/// (`BreakIterator.getWordInstance(ULocale.ROOT)`), but with the following
/// tailorings:
///
/// * Thai, Lao, Myanmar, Khmer, and CJK text is broken into words with a
///   dictionary.
///
/// Equivalent to `org.apache.lucene.analysis.icu.segmentation.DefaultICUTokenizerConfig`
/// (`@lucene.experimental`).
#[derive(Debug, Clone, Copy)]
pub struct DefaultICUTokenizerConfig {
    // TODO: deprecate this boolean? you only care if you are doing super-expert stuff...
    cjk_as_words: bool,
    myanmar_as_words: bool,
}

impl DefaultICUTokenizerConfig {
    /// Creates a new config. This object is lightweight, but the first time
    /// the class is referenced, breakiterators will be initialized.
    ///
    /// * `cjk_as_words`: true if cjk text should undergo dictionary-based
    ///   segmentation, otherwise text will be segmented according to UAX#29
    ///   defaults. If this is true, all Han+Hiragana+Katakana words will be
    ///   tagged as IDEOGRAPHIC.
    /// * `myanmar_as_words`: true if Myanmar text should undergo
    ///   dictionary-based segmentation, otherwise it will be tokenized as
    ///   syllables.
    ///
    /// Equivalent to `DefaultICUTokenizerConfig(boolean, boolean)`.
    ///
    /// # Panics
    ///
    /// The first time, when the static break iterators cannot be built
    /// (Java's `ExceptionInInitializerError`); the data is embedded.
    pub fn new(cjk_as_words: bool, myanmar_as_words: bool) -> Self {
        LazyLock::force(&STATICS);
        Self {
            cjk_as_words,
            myanmar_as_words,
        }
    }
}

impl ICUTokenizerConfig for DefaultICUTokenizerConfig {
    /// Equivalent to `DefaultICUTokenizerConfig.combineCJ()`.
    fn combine_cj(&self) -> bool {
        self.cjk_as_words
    }

    /// Equivalent to `DefaultICUTokenizerConfig.getBreakIterator(int)`: a
    /// clone of the static iterator for `script`.
    fn get_break_iterator(&self, script: i32) -> RuleBasedBreakIterator {
        match script {
            UScript::JAPANESE => clone_as_rbbi(STATICS.cjk_break_iterator.as_ref()),
            UScript::MYANMAR => {
                if self.myanmar_as_words {
                    STATICS.default_break_iterator.clone()
                } else {
                    STATICS.myanmar_syllable_iterator.clone()
                }
            }
            _ => STATICS.default_break_iterator.clone(),
        }
    }

    /// Equivalent to `DefaultICUTokenizerConfig.getType(int, int)`.
    fn get_type(&self, script: i32, rule_status: i32) -> Cow<'static, str> {
        Cow::Borrowed(match rule_status {
            RBBI_WORD_IDEO => *WORD_IDEO,
            WORD_KANA => {
                if script == UScript::HIRAGANA {
                    *WORD_HIRAGANA
                } else {
                    *WORD_KATAKANA
                }
            }
            RBBI_WORD_LETTER => {
                if script == UScript::HANGUL {
                    *WORD_HANGUL
                } else {
                    *WORD_LETTER
                }
            }
            RBBI_WORD_NUMBER => *WORD_NUMBER,
            EMOJI_SEQUENCE_STATUS => *WORD_EMOJI,
            _ => {
                /* some other custom code */
                "<OTHER>"
            }
        })
    }
}
