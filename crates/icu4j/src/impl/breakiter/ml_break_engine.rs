// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.breakiter.MlBreakEngine`, with the package-private
//! enum `ModelIndex` of its source file.

use std::collections::HashMap;

use rucene::error::Result;
use rucene_analysis_common::jdk::character_iterator::{CharacterIterator, DONE};

use crate::jdk::char_count;
use crate::r#impl::breakiter::dictionary_break_engine::{uset_contains, DequeI};
use crate::r#impl::character_iteration::{current32, next32, previous32, set_index, DONE32};
use crate::r#impl::icu_data::ICUData;
use crate::text::unicode_set::UnicodeSet;
use crate::util::u_resource_bundle::UResourceBundle;

/// The start index of each feature group in the model list.
///
/// Equivalent to the package-private enum
/// `com.ibm.icu.impl.breakiter.ModelIndex`, declared in `MlBreakEngine.java`.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)] // variant names mirror the Java constants
enum ModelIndex {
    /// `kUWStart(0)`.
    kUWStart,
    /// `kBWStart(6)`.
    kBWStart,
    /// `kTWStart(9)`.
    kTWStart,
}

impl ModelIndex {
    /// `getValue()`.
    fn get_value(self) -> usize {
        match self {
            ModelIndex::kUWStart => 0,
            ModelIndex::kBWStart => 6,
            ModelIndex::kTWStart => 9,
        }
    }
}

/// `MAX_FEATURE`.
const MAX_FEATURE: usize = 13;

/// The machine-learning phrase breaker for Japanese (the `jaml` model).
///
/// Equivalent to the class `com.ibm.icu.impl.breakiter.MlBreakEngine`.
#[derive(Debug)]
pub struct MlBreakEngine {
    f_digit_or_open_punctuation_or_alphabet_set: UnicodeSet,
    f_close_punctuation_set: UnicodeSet,
    f_model: Vec<HashMap<Vec<u16>, i32>>,
    f_negative_sum: i32,
}

impl MlBreakEngine {
    /// `MlBreakEngine(UnicodeSet digitOrOpenPunctuationOrAlphabetSet,
    /// UnicodeSet closePunctuationSet)`: loads the `jaml` model.
    ///
    /// # Errors
    ///
    /// `MissingResourceException` when the model bundle or one of its keys
    /// is missing.
    pub fn new(
        digit_or_open_punctuation_or_alphabet_set: UnicodeSet,
        close_punctuation_set: UnicodeSet,
    ) -> Result<Self> {
        let mut this = Self {
            f_digit_or_open_punctuation_or_alphabet_set: digit_or_open_punctuation_or_alphabet_set,
            f_close_punctuation_set: close_punctuation_set,
            f_model: (0..MAX_FEATURE).map(|_| HashMap::new()).collect(),
            f_negative_sum: 0,
        };
        this.load_ml_model()?;
        Ok(this)
    }

    /// `divideUpRange(CharacterIterator inText, int startPos, int endPos,
    /// CharacterIterator inString, int codePointLength, int[] charPositions,
    /// DequeI foundBreaks)`: divides up a range of characters by the model,
    /// pushing the breaks onto `found_breaks`; returns their number.
    #[allow(clippy::too_many_arguments)]
    pub fn divide_up_range(
        &self,
        in_text: &mut dyn CharacterIterator,
        start_pos: i32,
        end_pos: i32,
        in_string: &mut dyn CharacterIterator,
        code_point_length: i32,
        char_positions: &[i32],
        found_breaks: &mut DequeI,
    ) -> i32 {
        if start_pos >= end_pos {
            return 0;
        }
        let mut boundary: Vec<i32> = Vec::with_capacity(code_point_length.max(0) as usize);
        let input_str = self.transform(in_string);
        // The ML algorithm groups six char and evaluates whether the 4th char is a breakpoint.
        // In each iteration, it evaluates the 4th char and then moves forward one char like
        // sliding window. Initially, the first six values in the indexList are
        // [-1, -1, 0, 1, 2, 3]. After moving forward, finally the last six values in the indexList
        // are [length-4, length-3, length-2, length-1, -1, -1]. The "+4" here means four extra
        // "-1".
        let index_size = code_point_length + 4;
        let mut index_list = vec![0i32; index_size as usize];
        let mut num_code_units =
            self.init_index_list(in_string, &mut index_list, code_point_length);

        // Add a break for the start.
        boundary.insert(0, 0);

        let mut idx = 0;
        while idx + 1 < code_point_length {
            self.evaluate_breakpoint(&input_str, &index_list, idx, num_code_units, &mut boundary);
            if idx + 4 < code_point_length {
                index_list[(idx + 6) as usize] = num_code_units;
                num_code_units += char_count(next32(in_string));
            }
            idx += 1;
        }

        // Add a break for the end if there is not one there already.
        if *boundary.last().expect("boundary starts with 0") != code_point_length {
            boundary.push(code_point_length);
        }

        let mut corrected_num_breaks = 0;
        let mut previous = -1;
        let num_breaks = boundary.len();
        for &b in boundary.iter().take(num_breaks) {
            let pos = char_positions[b as usize] + start_pos;
            // In phrase breaking, there has to be a breakpoint between Cj character and close
            // punctuation.
            // E.g.［携帯電話］正しい選択 -> ［携帯▁電話］▁正しい▁選択 -> breakpoint between ］ and 正
            set_index(in_text, pos);
            if pos > previous
                && (pos != start_pos
                    || (pos > 0
                        && uset_contains(&self.f_close_punctuation_set, previous32(in_text))))
            {
                found_breaks.push(pos);
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
            set_index(in_text, end_pos);
            let current = current32(in_text);
            if current != DONE32
                && !uset_contains(&self.f_digit_or_open_punctuation_or_alphabet_set, current)
            {
                found_breaks.pop();
                corrected_num_breaks -= 1;
            }
        }
        if !found_breaks.is_empty() {
            set_index(in_text, found_breaks.peek());
        }
        corrected_num_breaks
    }

    /// `transform(CharacterIterator inString)`: the whole text.
    fn transform(&self, in_string: &mut dyn CharacterIterator) -> Vec<u16> {
        let mut sb = Vec::new();
        set_index(in_string, 0);
        let mut c = in_string.first();
        while c != DONE {
            sb.push(c);
            c = in_string.next();
        }
        sb
    }

    /// `evaluateBreakpoint(String inputStr, int[] indexList, int startIdx,
    /// int numCodeUnits, ArrayList<Integer> boundary)`.
    fn evaluate_breakpoint(
        &self,
        input_str: &[u16],
        index_list: &[i32],
        start_idx: i32,
        num_code_units: i32,
        boundary: &mut Vec<i32>,
    ) {
        let mut score = self.f_negative_sum;
        let feature = |group: ModelIndex, i: usize, from: i32, to: i32| -> i32 {
            *self.f_model[group.get_value() + i]
                .get(&input_str[from as usize..to as usize])
                .unwrap_or(&0)
        };

        for i in 0..6 {
            // UW1 ~ UW6
            let start = (start_idx + i) as usize;
            if index_list[start] != -1 {
                let end = if index_list[start + 1] != -1 {
                    index_list[start + 1]
                } else {
                    num_code_units
                };
                score = score.wrapping_add(feature(
                    ModelIndex::kUWStart,
                    i as usize,
                    index_list[start],
                    end,
                ));
            }
        }
        for i in 0..3 {
            // BW1 ~ BW3
            let start = (start_idx + i + 1) as usize;
            if index_list[start] != -1 && index_list[start + 1] != -1 {
                let end = if index_list[start + 2] != -1 {
                    index_list[start + 2]
                } else {
                    num_code_units
                };
                score = score.wrapping_add(feature(
                    ModelIndex::kBWStart,
                    i as usize,
                    index_list[start],
                    end,
                ));
            }
        }
        for i in 0..4 {
            // TW1 ~ TW4
            let start = (start_idx + i) as usize;
            if index_list[start] != -1 && index_list[start + 1] != -1 && index_list[start + 2] != -1
            {
                let end = if index_list[start + 3] != -1 {
                    index_list[start + 3]
                } else {
                    num_code_units
                };
                score = score.wrapping_add(feature(
                    ModelIndex::kTWStart,
                    i as usize,
                    index_list[start],
                    end,
                ));
            }
        }
        if score > 0 {
            boundary.push(start_idx + 1);
        }
    }

    /// `initIndexList(CharacterIterator inString, int[] indexList, int
    /// codePointLength)`: the code-unit offsets of the first four code
    /// points; returns the code units they span.
    fn init_index_list(
        &self,
        in_string: &mut dyn CharacterIterator,
        index_list: &mut [i32],
        code_point_length: i32,
    ) -> i32 {
        let mut index = 0;
        set_index(in_string, index);
        index_list.fill(-1);
        if code_point_length > 0 {
            index_list[2] = 0;
            index += char_count(current32(in_string));
            if code_point_length > 1 {
                index_list[3] = index;
                index += char_count(next32(in_string));
                if code_point_length > 2 {
                    index_list[4] = index;
                    index += char_count(next32(in_string));
                    if code_point_length > 3 {
                        index_list[5] = index;
                        index += char_count(next32(in_string));
                    }
                }
            }
        }
        index
    }

    /// `loadMLModel()`: reads the 13 feature tables of the `jaml` bundle.
    fn load_ml_model(&mut self) -> Result<()> {
        let rb = UResourceBundle::get_bundle_instance(Some(ICUData::ICU_BRKITR_BASE_NAME), "jaml")?;
        let names = [
            ("UW1Keys", "UW1Values"),
            ("UW2Keys", "UW2Values"),
            ("UW3Keys", "UW3Values"),
            ("UW4Keys", "UW4Values"),
            ("UW5Keys", "UW5Values"),
            ("UW6Keys", "UW6Values"),
            ("BW1Keys", "BW1Values"),
            ("BW2Keys", "BW2Values"),
            ("BW3Keys", "BW3Values"),
            ("TW1Keys", "TW1Values"),
            ("TW2Keys", "TW2Values"),
            ("TW3Keys", "TW3Values"),
            ("TW4Keys", "TW4Values"),
        ];
        for (index, (key_name, value_name)) in names.iter().enumerate() {
            self.init_key_value(&rb, key_name, value_name, index)?;
        }
        self.f_negative_sum /= 2;
        Ok(())
    }

    /// `initKeyValue(UResourceBundle rb, String keyName, String valueName,
    /// HashMap<String, Integer> map)`: `map` is `fModel.get(model)`.
    fn init_key_value(
        &mut self,
        rb: &UResourceBundle,
        key_name: &str,
        value_name: &str,
        model: usize,
    ) -> Result<()> {
        let mut idx = 0usize;
        let key_bundle = rb.get(key_name)?;
        let value_bundle = rb.get(value_name)?;
        let value = value_bundle.get_int_vector()?;
        let mut iterator = key_bundle.get_iterator();
        while iterator.has_next() {
            self.f_negative_sum = self.f_negative_sum.wrapping_sub(value[idx]);
            let key = iterator.next_string()?;
            self.f_model[model].insert(key, value[idx]);
            idx += 1;
        }
        Ok(())
    }
}
