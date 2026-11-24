// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.breakiter.CjkBreakEngine`.

use std::any::Any;
use std::collections::HashSet;

use rucene::error::Result;
use rucene::util::java_collections::java_identity_hash_code_stand_in;
use rucene_analysis_common::jdk::character_iterator::{CharacterIterator, StringCharacterIterator};

use crate::jdk::{char_count, code_point_at, offset_by_code_points, units};
use crate::r#impl::assert::Assert;
use crate::r#impl::breakiter::dictionary_break_engine::{
    dictionary_find_breaks, dictionary_handles, uset_contains, DequeI, DictionaryBreakEngine,
    DictionaryBreakEngineBase,
};
use crate::r#impl::breakiter::dictionary_data::DictionaryData;
use crate::r#impl::breakiter::dictionary_matcher::DictionaryMatcher;
use crate::r#impl::breakiter::language_break_engine::LanguageBreakEngine;
use crate::r#impl::breakiter::ml_break_engine::MlBreakEngine;
use crate::r#impl::character_iteration::{current32, next32, previous32, set_index, DONE32};
use crate::r#impl::icu_config::ICUConfig;
use crate::r#impl::icu_data::ICUData;
use crate::text::normalizer::Normalizer;
use crate::text::unicode_set::UnicodeSet;
use crate::text::unicode_set_iterator::UnicodeSetIterator;
use crate::util::u_resource_bundle::UResourceBundle;

/// `kMaxKatakanaLength`.
const K_MAX_KATAKANA_LENGTH: i32 = 8;
/// `kMaxKatakanaGroupLength`.
const K_MAX_KATAKANA_GROUP_LENGTH: i32 = 20;
/// `maxSnlp`.
const MAX_SNLP: i32 = 255;
/// `kint32max`.
const KINT32MAX: i32 = i32::MAX;

/// `Boolean.parseBoolean(ICUConfig.get("com.ibm.icu.impl.breakiter.useMLPhraseBreaking", "false"))`.
fn use_ml_phrase_breaking() -> bool {
    ICUConfig::get_or(
        "com.ibm.icu.impl.breakiter.useMLPhraseBreaking",
        Some("false"),
    )
    .is_some_and(|v| v.eq_ignore_ascii_case("true"))
}

/// The dictionary-based engine for Chinese, Japanese and Korean.
///
/// Equivalent to the class `com.ibm.icu.impl.breakiter.CjkBreakEngine`.
#[derive(Debug)]
pub struct CjkBreakEngine {
    base: DictionaryBreakEngineBase,
    f_hangul_word_set: UnicodeSet,
    f_digit_or_open_punctuation_or_alphabet_set: UnicodeSet,
    f_close_punctuation_set: UnicodeSet,
    f_dictionary: Option<Box<dyn DictionaryMatcher>>,
    f_skip_set: HashSet<Vec<u16>>,
    f_ml_break_engine: Option<MlBreakEngine>,
    is_cj: bool,
}

impl CjkBreakEngine {
    /// `CjkBreakEngine(boolean korean)`.
    ///
    /// # Errors
    ///
    /// The `IOException`/`MissingResourceException` of the dictionary and
    /// resource loads, and the exceptions of the `UnicodeSet` patterns.
    pub fn new(korean: bool) -> Result<Self> {
        let mut f_hangul_word_set = UnicodeSet::from_pattern(&units("[\\uac00-\\ud7a3]"))?;
        f_hangul_word_set.freeze();
        // Digit, open punctuation and Alphabetic characters.
        let mut f_digit_or_open_punctuation_or_alphabet_set =
            UnicodeSet::from_pattern(&units("[[:Nd:][:Pi:][:Ps:][:Alphabetic:]]"))?;
        f_digit_or_open_punctuation_or_alphabet_set.freeze();

        let mut f_close_punctuation_set =
            UnicodeSet::from_pattern(&units("[[:Pc:][:Pd:][:Pe:][:Pf:][:Po:]]"))?;
        f_close_punctuation_set.freeze();
        let f_skip_set = HashSet::new();

        let f_dictionary = DictionaryData::load_dictionary_for("Hira")?;
        let mut this = Self {
            base: DictionaryBreakEngineBase::new(),
            f_hangul_word_set,
            f_digit_or_open_punctuation_or_alphabet_set,
            f_close_punctuation_set,
            f_dictionary,
            f_skip_set,
            f_ml_break_engine: None,
            is_cj: false,
        };
        if korean {
            let set = this.f_hangul_word_set.clone();
            this.base.set_characters(&set)?;
        } else {
            //Chinese and Japanese
            this.is_cj = true;
            let cj_set = UnicodeSet::from_pattern(&units(
                "[[:Han:][:Hiragana:][:Katakana:]\\u30fc\\uff70\\uff9e\\uff9f]",
            ))?;
            this.base.set_characters(&cj_set)?;
            if use_ml_phrase_breaking() {
                this.f_ml_break_engine = Some(MlBreakEngine::new(
                    this.f_digit_or_open_punctuation_or_alphabet_set.clone(),
                    this.f_close_punctuation_set.clone(),
                )?);
            } else {
                this.initialize_japanese_phrase_paramater()?;
            }
        }
        Ok(this)
    }

    /// `initializeJapanesePhraseParamater()`.
    fn initialize_japanese_phrase_paramater(&mut self) -> Result<()> {
        self.load_japanese_extensions()?;
        self.load_hiragana()
    }

    /// `loadJapaneseExtensions()`: the `extensions` of the `ja` break
    /// iterator bundle join the skip set.
    fn load_japanese_extensions(&mut self) -> Result<()> {
        let rb = UResourceBundle::get_bundle_instance(Some(ICUData::ICU_BRKITR_BASE_NAME), "ja")?;
        let tag = "extensions";
        let bundle = rb.get(tag)?;
        let mut iterator = bundle.get_iterator();
        while iterator.has_next() {
            self.f_skip_set.insert(iterator.next_string()?);
        }
        Ok(())
    }

    /// `loadHiragana()`: every Hiragana character joins the skip set.
    fn load_hiragana(&mut self) -> Result<()> {
        let mut hiragana_word_set = UnicodeSet::from_pattern(&units("[:Hiragana:]"))?;
        hiragana_word_set.freeze();
        let mut iterator = UnicodeSetIterator::new(&hiragana_word_set);
        while iterator.next() {
            self.f_skip_set.insert(iterator.get_string()?);
        }
        Ok(())
    }

    /// `equals(Object obj)`: another `CjkBreakEngine` with an equal `fSet`.
    pub fn java_equals(&self, obj: &dyn Any) -> bool {
        if let Some(other) = obj.downcast_ref::<CjkBreakEngine>() {
            return self.base.f_set == other.base.f_set;
        }
        false
    }

    /// `hashCode()`: `getClass().hashCode()`.
    ///
    /// **Divergence.** Java answers the identity hash of the `Class` object
    /// (`CjkBreakEngine.java`, `hashCode`), which no Rust value has; the port
    /// answers the crate's stable stand-in for it.
    pub fn hash_code(&self) -> i32 {
        java_identity_hash_code_stand_in("com.ibm.icu.impl.breakiter.CjkBreakEngine")
    }

    /// `getKatakanaCost(int wordlength)`.
    fn get_katakana_cost(wordlength: i32) -> i32 {
        let katakana_cost: [i32; 9] = [8192, 984, 408, 240, 204, 252, 300, 372, 480];
        if wordlength > K_MAX_KATAKANA_LENGTH {
            8192
        } else {
            katakana_cost[wordlength as usize]
        }
    }

    /// `isKatakana(int value)`.
    fn is_katakana(value: i32) -> bool {
        ((0x30A1..=0x30FE).contains(&value) && value != 0x30FB)
            || (0xFF66..=0xFF9F).contains(&value)
    }

    /// `getPatternFromText(CharacterIterator text, StringBuffer sb, int
    /// start, int length)`: refills `sb` with `length` units of `text` from
    /// `start` and returns them.
    fn get_pattern_from_text(
        text: &mut dyn CharacterIterator,
        sb: &mut Vec<u16>,
        start: i32,
        length: i32,
    ) -> Vec<u16> {
        sb.clear();
        if length > 0 {
            set_index(text, start);
            sb.push(text.current());
            for _ in 1..length {
                sb.push(text.next());
            }
        }
        sb.clone()
    }

    /// `fDictionary`, dereferenced as Java does on each use.
    ///
    /// # Panics
    ///
    /// `NullPointerException` when the dictionary data had an unknown trie
    /// type (`DictionaryData.loadDictionaryFor` answered `null`).
    fn dictionary(&self) -> &dyn DictionaryMatcher {
        self.f_dictionary
            .as_deref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: fDictionary"))
    }
}

impl LanguageBreakEngine for CjkBreakEngine {
    fn handles(&self, c: i32) -> bool {
        dictionary_handles(self, c)
    }

    fn find_breaks(
        &self,
        text: &mut dyn CharacterIterator,
        start_pos: i32,
        end_pos: i32,
        found_breaks: &mut DequeI,
        is_phrase_breaking: bool,
    ) -> i32 {
        dictionary_find_breaks(
            self,
            text,
            start_pos,
            end_pos,
            found_breaks,
            is_phrase_breaking,
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl DictionaryBreakEngine for CjkBreakEngine {
    fn base(&self) -> &DictionaryBreakEngineBase {
        &self.base
    }

    /// `divideUpDictionaryRange`.
    ///
    /// # Panics
    ///
    /// With the `Assert.assrt` message when the best path does not start at
    /// the range start (`CjkBreakEngine.java`, `Assert.assrt(prev[...] ==
    /// 0)`): the internal-error exception reaches no caller that could
    /// catch it, the break-iterator signatures being infallible.
    fn divide_up_dictionary_range(
        &self,
        in_text: &mut dyn CharacterIterator,
        start_pos: i32,
        end_pos: i32,
        found_breaks: &mut DequeI,
        is_phrase_breaking: bool,
    ) -> i32 {
        if start_pos >= end_pos {
            return 0;
        }

        set_index(in_text, start_pos);

        let input_length = end_pos - start_pos;
        let mut char_positions = vec![0i32; (input_length + 1) as usize];
        let mut s: Vec<u16> = Vec::new();
        set_index(in_text, start_pos);
        while in_text.get_index() < end_pos {
            s.push(in_text.current());
            in_text.next();
        }
        let prenormstr = s.clone();
        let is_normalized = Normalizer::quick_check_str_mode(&prenormstr, Normalizer::NFKC)
            .unwrap_or_else(|e| panic!("{e}"))
            == Normalizer::YES
            || Normalizer::is_normalized_str(&prenormstr, Normalizer::NFKC, 0)
                .unwrap_or_else(|e| panic!("{e}"));
        let mut text: StringCharacterIterator;
        let mut num_code_pts: i32 = 0;
        if is_normalized {
            text = StringCharacterIterator::new(prenormstr.clone());
            let mut index = 0;
            char_positions[0] = 0;
            while index < prenormstr.len() as i32 {
                let codepoint = code_point_at(&prenormstr, index);
                index += char_count(codepoint);
                num_code_pts += 1;
                char_positions[num_code_pts as usize] = index;
            }
        } else {
            let norm_str = Normalizer::normalize_str_mode(&prenormstr, Normalizer::NFKC)
                .unwrap_or_else(|e| panic!("{e}"));
            text = StringCharacterIterator::new(norm_str.clone());
            char_positions = vec![0i32; norm_str.len() + 1];
            let mut normalizer = Normalizer::new_string(&prenormstr, Normalizer::NFKC, 0)
                .unwrap_or_else(|e| panic!("{e}"));
            let mut index = 0;
            char_positions[0] = 0;
            while index < normalizer.end_index() {
                normalizer.next().unwrap_or_else(|e| panic!("{e}"));
                num_code_pts += 1;
                index = normalizer.get_index();
                char_positions[num_code_pts as usize] = index;
            }
        }

        // Use ML phrase breaking
        if use_ml_phrase_breaking() {
            // PhraseBreaking is supported in ja and ko; MlBreakEngine only supports ja.
            if is_phrase_breaking && self.is_cj {
                return self
                    .f_ml_break_engine
                    .as_ref()
                    .unwrap_or_else(|| panic!("java.lang.NullPointerException: fMlBreakEngine"))
                    .divide_up_range(
                        in_text,
                        start_pos,
                        end_pos,
                        &mut text,
                        num_code_pts,
                        &char_positions,
                        found_breaks,
                    );
            }
        }

        // From here on out, do the algorithm. Note that our indices
        // refer to indices within the normalized string.
        let n = num_code_pts as usize;
        let mut best_snlp = vec![0i32; n + 1];
        best_snlp[0] = 0;
        for v in best_snlp.iter_mut().skip(1) {
            *v = KINT32MAX;
        }

        let mut prev = vec![-1i32; n + 1];

        let max_word_size: i32 = 20;
        let mut values = vec![0i32; n];
        let mut lengths = vec![0i32; n];

        let dict = self.dictionary();
        // dynamic programming to find the best segmentation

        // In outer loop, i  is the code point index,
        //                ix is the corresponding code unit index.
        //    They differ when the string contains supplementary characters.
        let mut ix = 0;
        set_index(&mut text, ix);
        let mut is_prev_katakana = false;
        let mut i: i32 = 0;
        while i < num_code_pts {
            ix = text.get_index();
            if best_snlp[i as usize] != KINT32MAX {
                let max_search_length = if i + max_word_size < num_code_pts {
                    max_word_size
                } else {
                    num_code_pts - i
                };
                let mut count_ = [0i32; 1];
                dict.matches(
                    &mut text,
                    max_search_length,
                    &mut lengths,
                    &mut count_,
                    max_search_length,
                    Some(&mut values),
                );
                let mut count = count_[0];

                // if there are no single character matches found in the dictionary
                // starting with this character, treat character as a 1-character word
                // with the highest value possible (i.e. the least likely to occur).
                // Exclude Korean characters from this treatment, as they should be
                // left together by default.
                set_index(&mut text, ix);
                if (count == 0 || lengths[0] != 1)
                    && current32(&mut text) != DONE32
                    && !uset_contains(&self.f_hangul_word_set, current32(&mut text))
                {
                    values[count as usize] = MAX_SNLP;
                    lengths[count as usize] = 1;
                    count += 1;
                }

                for j in 0..count as usize {
                    let new_snlp = best_snlp[i as usize].wrapping_add(values[j]);
                    let target = (lengths[j] + i) as usize;
                    if new_snlp < best_snlp[target] {
                        best_snlp[target] = new_snlp;
                        prev[target] = i;
                    }
                }

                // In Japanese, single-character Katakana words are pretty rare.
                // So we apply the following heuristic to Katakana: any continuous
                // run of Katakana characters is considered a candidate word with
                // a default cost specified in the katakanaCost table according
                // to its length.
                let is_katakana = Self::is_katakana(current32(&mut text));
                if !is_prev_katakana && is_katakana {
                    let mut j = i + 1;
                    next32(&mut text);
                    while j < num_code_pts
                        && (j - i) < K_MAX_KATAKANA_GROUP_LENGTH
                        && Self::is_katakana(current32(&mut text))
                    {
                        next32(&mut text);
                        j += 1;
                    }

                    if (j - i) < K_MAX_KATAKANA_GROUP_LENGTH {
                        let new_snlp =
                            best_snlp[i as usize].wrapping_add(Self::get_katakana_cost(j - i));
                        if new_snlp < best_snlp[j as usize] {
                            best_snlp[j as usize] = new_snlp;
                            prev[j as usize] = i;
                        }
                    }
                }
                is_prev_katakana = is_katakana;
            }
            i += 1;
            set_index(&mut text, ix);
            next32(&mut text);
        }

        let mut t_boundary = vec![0i32; n + 1];
        let mut num_breaks: usize = 0;
        if best_snlp[n] == KINT32MAX {
            t_boundary[num_breaks] = num_code_pts;
            num_breaks += 1;
        } else if is_phrase_breaking {
            t_boundary[num_breaks] = num_code_pts;
            num_breaks += 1;
            let mut prev_idx = num_code_pts;
            let mut i = prev[n];
            while i > 0 {
                let code_unit_idx = offset_by_code_points(&prenormstr, 0, i);
                let prev_code_unit_idx = offset_by_code_points(&prenormstr, 0, prev_idx);
                let length = prev_code_unit_idx - code_unit_idx;
                prev_idx = i;
                let pattern = Self::get_pattern_from_text(&mut text, &mut s, code_unit_idx, length);
                // Keep the breakpoint if the pattern is not in the fSkipSet and continuous Katakana
                // characters don't occur.
                set_index(&mut text, code_unit_idx);
                if !self.f_skip_set.contains(&pattern)
                    && (!Self::is_katakana(current32(&mut text))
                        || !Self::is_katakana(previous32(&mut text)))
                {
                    t_boundary[num_breaks] = i;
                    num_breaks += 1;
                }
                i = prev[i as usize];
            }
        } else {
            let mut i = num_code_pts;
            while i > 0 {
                t_boundary[num_breaks] = i;
                num_breaks += 1;
                i = prev[i as usize];
            }
            if let Err(e) = Assert::assrt(prev[t_boundary[num_breaks - 1] as usize] == 0) {
                panic!("{e}");
            }
        }

        // Add a break for the start of the dictionary range if there is not one
        // there already.
        if found_breaks.size() == 0 || found_breaks.peek() < start_pos {
            t_boundary[num_breaks] = 0;
            num_breaks += 1;
        }

        let mut corrected_num_breaks = 0;
        let mut previous = -1;
        for i in (0..num_breaks).rev() {
            let pos = char_positions[t_boundary[i] as usize] + start_pos;
            // In phrase breaking, there has to be a breakpoint between Cj character and close
            // punctuation.
            // E.g.［携帯電話］正しい選択 -> ［携帯▁電話］▁正しい▁選択 -> breakpoint between ］ and 正
            set_index(in_text, pos);
            if pos > previous
                && (pos != start_pos
                    || (is_phrase_breaking
                        && pos > 0
                        && uset_contains(&self.f_close_punctuation_set, previous32(in_text))))
            {
                found_breaks.push(char_positions[t_boundary[i] as usize] + start_pos);
                corrected_num_breaks += 1;
            }
            previous = pos;
        }

        if !found_breaks.is_empty() && found_breaks.peek() == end_pos {
            // In phrase breaking, there has to be a breakpoint between Cj character and
            // the number/open punctuation.
            // E.g. る文字「そうだ、京都」->る▁文字▁「そうだ、▁京都」-> breakpoint between 字 and「
            // E.g. 乗車率９０％程度だろうか -> 乗車▁率▁９０％▁程度だろうか -> breakpoint between 率 and ９
            // E.g. しかもロゴがＵｎｉｃｏｄｅ！ -> しかも▁ロゴが▁Ｕｎｉｃｏｄｅ！-> breakpoint between が and Ｕ
            if is_phrase_breaking {
                set_index(in_text, end_pos);
                let current = current32(in_text);
                if current != DONE32
                    && !uset_contains(&self.f_digit_or_open_punctuation_or_alphabet_set, current)
                {
                    found_breaks.pop();
                    corrected_num_breaks -= 1;
                }
            } else {
                found_breaks.pop();
                corrected_num_breaks -= 1;
            }
        }
        if !found_breaks.is_empty() {
            set_index(in_text, found_breaks.peek());
        }
        corrected_num_breaks
    }
}
