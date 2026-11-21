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

//! A `TokenFilter` that normalizes CJK width differences, ported from
//! `org.apache.lucene.analysis.cjk.CJKWidthFilter`.

use rucene::analysis::{CharTermAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

use crate::analysis::util::stemmer_util::StemmerUtil;

/* halfwidth kana mappings: 0xFF65-0xFF9D
 *
 * note: 0xFF9C and 0xFF9D are only mapped to 0x3099 and 0x309A
 * as a fallback when they cannot properly combine with a preceding
 * character into a composed form.
 */
/// `CJKWidthFilter.KANA_NORM`.
#[rustfmt::skip]
const KANA_NORM: [u16; 59] = [
    0x30fb, 0x30f2, 0x30a1, 0x30a3, 0x30a5, 0x30a7, 0x30a9, 0x30e3, 0x30e5,
    0x30e7, 0x30c3, 0x30fc, 0x30a2, 0x30a4, 0x30a6, 0x30a8, 0x30aa, 0x30ab,
    0x30ad, 0x30af, 0x30b1, 0x30b3, 0x30b5, 0x30b7, 0x30b9, 0x30bb, 0x30bd,
    0x30bf, 0x30c1, 0x30c4, 0x30c6, 0x30c8, 0x30ca, 0x30cb, 0x30cc, 0x30cd,
    0x30ce, 0x30cf, 0x30d2, 0x30d5, 0x30d8, 0x30db, 0x30de, 0x30df, 0x30e0,
    0x30e1, 0x30e2, 0x30e4, 0x30e6, 0x30e8, 0x30e9, 0x30ea, 0x30eb, 0x30ec,
    0x30ed, 0x30ef, 0x30f3, 0x3099, 0x309A,
];

/* kana combining diffs: 0x30A6-0x30FD */
/// `CJKWidthFilter.KANA_COMBINE_VOICED`.
#[rustfmt::skip]
const KANA_COMBINE_VOICED: [i8; 88] = [
    78, 0, 0, 0, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 0,
    1, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 8, 8, 8, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
];

/// `CJKWidthFilter.KANA_COMBINE_HALF_VOICED`.
#[rustfmt::skip]
const KANA_COMBINE_HALF_VOICED: [i8; 88] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 2, 0, 0, 2,
    0, 0, 2, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// The `char` of a BMP code unit that is not a surrogate.
fn unit_char(unit: u32) -> char {
    char::from_u32(unit).expect("INVARIANT: a kana or Basic Latin unit, not a surrogate")
}

/// returns true if we successfully combined the voice mark
///
/// Equivalent to the private static `combine(char[], int, char)`.
fn combine(text: &mut [char], pos: usize, ch: u32) -> bool {
    let prev = text[pos - 1] as u32;
    if (0x30A6..=0x30FD).contains(&prev) {
        let diff = if ch == 0xFF9F {
            KANA_COMBINE_HALF_VOICED[(prev - 0x30A6) as usize]
        } else {
            KANA_COMBINE_VOICED[(prev - 0x30A6) as usize]
        };
        text[pos - 1] = unit_char((prev as i32 + i32::from(diff)) as u32);
        return text[pos - 1] as u32 != prev;
    }
    false
}

/// The body of [`CJKWidthFilter`]: what Lucene's `CJKWidthFilter` adds to
/// `TokenFilter`.
#[derive(Clone, Copy, Debug, Default)]
pub struct CJKWidthFilterLogic;

impl TokenFilterLogic for CJKWidthFilterLogic {
    /// Equivalent to `CJKWidthFilter.incrementToken()`. Java walks the
    /// term's UTF-16 units; every unit it rewrites, and the unit before a
    /// voice mark it combines with, is a BMP character, so walking the
    /// term's `char`s gives the same term.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let mut term_att = source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            let mut length = term_att.length();
            let text = term_att.buffer_mut();
            let mut i = 0;
            while i < length {
                let ch = text[i] as u32;
                if (0xFF01..=0xFF5E).contains(&ch) {
                    // Fullwidth ASCII variants
                    text[i] = unit_char(ch - 0xFEE0);
                } else if (0xFF65..=0xFF9F).contains(&ch) {
                    // Halfwidth Katakana variants
                    if (ch == 0xFF9E || ch == 0xFF9F) && i > 0 && combine(text, i, ch) {
                        length = StemmerUtil::delete(text, i, length);
                        // `i--`, undone by the loop's `i++`
                        i -= 1;
                    } else {
                        text[i] = unit_char(u32::from(KANA_NORM[(ch - 0xFF65) as usize]));
                    }
                }
                i += 1;
            }
            term_att.set_length(length);
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// A `TokenFilter` that normalizes CJK width differences:
///
/// * Folds fullwidth ASCII variants into the equivalent basic latin
/// * Folds halfwidth Katakana variants into the equivalent kana
///
/// NOTE: this filter can be viewed as a (practical) subset of NFKC/NFKD
/// Unicode normalization. See the normalization support in the ICU package
/// for full normalization.
///
/// Equivalent to `org.apache.lucene.analysis.cjk.CJKWidthFilter`.
pub type CJKWidthFilter = TokenFilter<CJKWidthFilterLogic>;

/// Equivalent to `CJKWidthFilter(TokenStream)`.
///
/// # Errors
///
/// What `addAttribute` raises for an attribute the input's factory cannot
/// implement.
pub fn new_cjk_width_filter(input: Box<dyn TokenStream>) -> Result<CJKWidthFilter> {
    let mut filter = TokenFilter::new(input, CJKWidthFilterLogic);
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}
