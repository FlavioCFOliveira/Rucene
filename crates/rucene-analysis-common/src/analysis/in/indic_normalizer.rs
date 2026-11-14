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

//! Normalizes the Unicode representation of text in Indian languages,
//! ported from `org.apache.lucene.analysis.in.IndicNormalizer`.

use std::collections::HashMap;
use std::sync::LazyLock;

use crate::analysis::util::stemmer_util::StemmerUtil;
use crate::jdk::character::UnicodeBlock;

/// Equivalent to `org.apache.lucene.analysis.in.IndicNormalizer.ScriptData`.
///
/// Java's `decompMask` is a `BitSet` of the offsets (below `0x80`, the
/// size of a block) that start a decomposition; here it is the same bits in
/// a `u128`, whose `get` answers `false` beyond them as `BitSet.get`
/// does.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ScriptData {
    pub(crate) flag: i32,
    pub(crate) base: i32,
    pub(crate) decomp_mask: u128,
}

impl ScriptData {
    /// Equivalent to `ScriptData(int, int)`: an empty `decompMask`.
    pub(crate) fn new(flag: i32, base: i32) -> Self {
        Self {
            flag,
            base,
            decomp_mask: 0,
        }
    }

    /// `decompMask.get(ch)`.
    fn decomp_mask_get(&self, ch: i32) -> bool {
        (0..128).contains(&ch) && (self.decomp_mask >> ch) & 1 != 0
    }
}

/// The flag and base of each script, as the first static initializer of
/// `IndicNormalizer` puts them into `scripts`, in that order.
const SCRIPT_FLAGS: [(UnicodeBlock, i32, i32); 9] = [
    (UnicodeBlock::DEVANAGARI, 1, 0x0900),
    (UnicodeBlock::BENGALI, 2, 0x0980),
    (UnicodeBlock::GURMUKHI, 4, 0x0A00),
    (UnicodeBlock::GUJARATI, 8, 0x0A80),
    (UnicodeBlock::ORIYA, 16, 0x0B00),
    (UnicodeBlock::TAMIL, 32, 0x0B80),
    (UnicodeBlock::TELUGU, 64, 0x0C00),
    (UnicodeBlock::KANNADA, 128, 0x0C80),
    (UnicodeBlock::MALAYALAM, 256, 0x0D00),
];

/// Equivalent to the private `flag(Character.UnicodeBlock)`,
/// `scripts.get(ub).flag`.
///
/// Java reads the flag from `scripts` while the class initializes, after
/// the first static initializer filled it; here it is read from the same
/// values, so that `decompositions` and the `decompMask` the second static
/// initializer derives from it do not depend on each other's `LazyLock`.
fn flag(ub: UnicodeBlock) -> i32 {
    SCRIPT_FLAGS
        .iter()
        .find(|(block, _, _)| *block == ub)
        .map(|&(_, flag, _)| flag)
        .expect("INVARIANT: flag is called on the nine scripts only")
}

/// Decompositions according to Unicode 5.2, and
/// http://ldc.upenn.edu/myl/IndianScriptsUnicode.html
///
/// Most of these are not handled by unicode normalization anyway.
///
/// The numbers here represent offsets into the respective codepages, with -1
/// representing null and 0xFF representing zero-width joiner.
///
/// the columns are: ch1, ch2, ch3, res, flags ch1, ch2, and ch3 are the
/// decomposition res is the composition, and flags are the scripts to which
/// it applies.
///
/// Equivalent to the private `IndicNormalizer.decompositions`.
static DECOMPOSITIONS: LazyLock<Vec<[i32; 5]>> = LazyLock::new(|| {
    vec![
        // devanagari, gujarati vowel candra O
        [
            0x05,
            0x3E,
            0x45,
            0x11,
            flag(UnicodeBlock::DEVANAGARI) | flag(UnicodeBlock::GUJARATI),
        ],
        // devanagari short O
        [0x05, 0x3E, 0x46, 0x12, flag(UnicodeBlock::DEVANAGARI)],
        // devanagari, gujarati letter O
        [
            0x05,
            0x3E,
            0x47,
            0x13,
            flag(UnicodeBlock::DEVANAGARI) | flag(UnicodeBlock::GUJARATI),
        ],
        // devanagari letter AI, gujarati letter AU
        [
            0x05,
            0x3E,
            0x48,
            0x14,
            flag(UnicodeBlock::DEVANAGARI) | flag(UnicodeBlock::GUJARATI),
        ],
        // devanagari, bengali, gurmukhi, gujarati, oriya AA
        [
            0x05,
            0x3E,
            -1,
            0x06,
            flag(UnicodeBlock::DEVANAGARI)
                | flag(UnicodeBlock::BENGALI)
                | flag(UnicodeBlock::GURMUKHI)
                | flag(UnicodeBlock::GUJARATI)
                | flag(UnicodeBlock::ORIYA),
        ],
        // devanagari letter candra A
        [0x05, 0x45, -1, 0x72, flag(UnicodeBlock::DEVANAGARI)],
        // gujarati vowel candra E
        [0x05, 0x45, -1, 0x0D, flag(UnicodeBlock::GUJARATI)],
        // devanagari letter short A
        [0x05, 0x46, -1, 0x04, flag(UnicodeBlock::DEVANAGARI)],
        // gujarati letter E
        [0x05, 0x47, -1, 0x0F, flag(UnicodeBlock::GUJARATI)],
        // gurmukhi, gujarati letter AI
        [
            0x05,
            0x48,
            -1,
            0x10,
            flag(UnicodeBlock::GURMUKHI) | flag(UnicodeBlock::GUJARATI),
        ],
        // devanagari, gujarati vowel candra O
        [
            0x05,
            0x49,
            -1,
            0x11,
            flag(UnicodeBlock::DEVANAGARI) | flag(UnicodeBlock::GUJARATI),
        ],
        // devanagari short O
        [0x05, 0x4A, -1, 0x12, flag(UnicodeBlock::DEVANAGARI)],
        // devanagari, gujarati letter O
        [
            0x05,
            0x4B,
            -1,
            0x13,
            flag(UnicodeBlock::DEVANAGARI) | flag(UnicodeBlock::GUJARATI),
        ],
        // devanagari letter AI, gurmukhi letter AU, gujarati letter AU
        [
            0x05,
            0x4C,
            -1,
            0x14,
            flag(UnicodeBlock::DEVANAGARI)
                | flag(UnicodeBlock::GURMUKHI)
                | flag(UnicodeBlock::GUJARATI),
        ],
        // devanagari, gujarati vowel candra O
        [
            0x06,
            0x45,
            -1,
            0x11,
            flag(UnicodeBlock::DEVANAGARI) | flag(UnicodeBlock::GUJARATI),
        ],
        // devanagari short O
        [0x06, 0x46, -1, 0x12, flag(UnicodeBlock::DEVANAGARI)],
        // devanagari, gujarati letter O
        [
            0x06,
            0x47,
            -1,
            0x13,
            flag(UnicodeBlock::DEVANAGARI) | flag(UnicodeBlock::GUJARATI),
        ],
        // devanagari letter AI, gujarati letter AU
        [
            0x06,
            0x48,
            -1,
            0x14,
            flag(UnicodeBlock::DEVANAGARI) | flag(UnicodeBlock::GUJARATI),
        ],
        // malayalam letter II
        [0x07, 0x57, -1, 0x08, flag(UnicodeBlock::MALAYALAM)],
        // devanagari letter UU
        [0x09, 0x41, -1, 0x0A, flag(UnicodeBlock::DEVANAGARI)],
        // tamil, malayalam letter UU (some styles)
        [
            0x09,
            0x57,
            -1,
            0x0A,
            flag(UnicodeBlock::TAMIL) | flag(UnicodeBlock::MALAYALAM),
        ],
        // malayalam letter AI
        [0x0E, 0x46, -1, 0x10, flag(UnicodeBlock::MALAYALAM)],
        // devanagari candra E
        [0x0F, 0x45, -1, 0x0D, flag(UnicodeBlock::DEVANAGARI)],
        // devanagari short E
        [0x0F, 0x46, -1, 0x0E, flag(UnicodeBlock::DEVANAGARI)],
        // devanagari AI
        [0x0F, 0x47, -1, 0x10, flag(UnicodeBlock::DEVANAGARI)],
        // oriya AI
        [0x0F, 0x57, -1, 0x10, flag(UnicodeBlock::ORIYA)],
        // malayalam letter OO
        [0x12, 0x3E, -1, 0x13, flag(UnicodeBlock::MALAYALAM)],
        // telugu, kannada letter AU
        [
            0x12,
            0x4C,
            -1,
            0x14,
            flag(UnicodeBlock::TELUGU) | flag(UnicodeBlock::KANNADA),
        ],
        // telugu letter OO
        [0x12, 0x55, -1, 0x13, flag(UnicodeBlock::TELUGU)],
        // tamil, malayalam letter AU
        [
            0x12,
            0x57,
            -1,
            0x14,
            flag(UnicodeBlock::TAMIL) | flag(UnicodeBlock::MALAYALAM),
        ],
        // oriya letter AU
        [0x13, 0x57, -1, 0x14, flag(UnicodeBlock::ORIYA)],
        // devanagari qa
        [0x15, 0x3C, -1, 0x58, flag(UnicodeBlock::DEVANAGARI)],
        // devanagari, gurmukhi khha
        [
            0x16,
            0x3C,
            -1,
            0x59,
            flag(UnicodeBlock::DEVANAGARI) | flag(UnicodeBlock::GURMUKHI),
        ],
        // devanagari, gurmukhi ghha
        [
            0x17,
            0x3C,
            -1,
            0x5A,
            flag(UnicodeBlock::DEVANAGARI) | flag(UnicodeBlock::GURMUKHI),
        ],
        // devanagari, gurmukhi za
        [
            0x1C,
            0x3C,
            -1,
            0x5B,
            flag(UnicodeBlock::DEVANAGARI) | flag(UnicodeBlock::GURMUKHI),
        ],
        // devanagari dddha, bengali, oriya rra
        [
            0x21,
            0x3C,
            -1,
            0x5C,
            flag(UnicodeBlock::DEVANAGARI)
                | flag(UnicodeBlock::BENGALI)
                | flag(UnicodeBlock::ORIYA),
        ],
        // devanagari, bengali, oriya rha
        [
            0x22,
            0x3C,
            -1,
            0x5D,
            flag(UnicodeBlock::DEVANAGARI)
                | flag(UnicodeBlock::BENGALI)
                | flag(UnicodeBlock::ORIYA),
        ],
        // malayalam chillu nn
        [0x23, 0x4D, 0xFF, 0x7A, flag(UnicodeBlock::MALAYALAM)],
        // bengali khanda ta
        [0x24, 0x4D, 0xFF, 0x4E, flag(UnicodeBlock::BENGALI)],
        // devanagari nnna
        [0x28, 0x3C, -1, 0x29, flag(UnicodeBlock::DEVANAGARI)],
        // malayalam chillu n
        [0x28, 0x4D, 0xFF, 0x7B, flag(UnicodeBlock::MALAYALAM)],
        // devanagari, gurmukhi fa
        [
            0x2B,
            0x3C,
            -1,
            0x5E,
            flag(UnicodeBlock::DEVANAGARI) | flag(UnicodeBlock::GURMUKHI),
        ],
        // devanagari, bengali yya
        [
            0x2F,
            0x3C,
            -1,
            0x5F,
            flag(UnicodeBlock::DEVANAGARI) | flag(UnicodeBlock::BENGALI),
        ],
        // telugu letter vocalic R
        [0x2C, 0x41, 0x41, 0x0B, flag(UnicodeBlock::TELUGU)],
        // devanagari rra
        [0x30, 0x3C, -1, 0x31, flag(UnicodeBlock::DEVANAGARI)],
        // malayalam chillu rr
        [0x30, 0x4D, 0xFF, 0x7C, flag(UnicodeBlock::MALAYALAM)],
        // malayalam chillu l
        [0x32, 0x4D, 0xFF, 0x7D, flag(UnicodeBlock::MALAYALAM)],
        // devanagari llla
        [0x33, 0x3C, -1, 0x34, flag(UnicodeBlock::DEVANAGARI)],
        // malayalam chillu ll
        [0x33, 0x4D, 0xFF, 0x7E, flag(UnicodeBlock::MALAYALAM)],
        // telugu letter MA
        [0x35, 0x41, -1, 0x2E, flag(UnicodeBlock::TELUGU)],
        // devanagari, gujarati vowel sign candra O
        [
            0x3E,
            0x45,
            -1,
            0x49,
            flag(UnicodeBlock::DEVANAGARI) | flag(UnicodeBlock::GUJARATI),
        ],
        // devanagari vowel sign short O
        [0x3E, 0x46, -1, 0x4A, flag(UnicodeBlock::DEVANAGARI)],
        // devanagari, gujarati vowel sign O
        [
            0x3E,
            0x47,
            -1,
            0x4B,
            flag(UnicodeBlock::DEVANAGARI) | flag(UnicodeBlock::GUJARATI),
        ],
        // devanagari, gujarati vowel sign AU
        [
            0x3E,
            0x48,
            -1,
            0x4C,
            flag(UnicodeBlock::DEVANAGARI) | flag(UnicodeBlock::GUJARATI),
        ],
        // kannada vowel sign II
        [0x3F, 0x55, -1, 0x40, flag(UnicodeBlock::KANNADA)],
        // gurmukhi vowel sign UU (when stacking)
        [0x41, 0x41, -1, 0x42, flag(UnicodeBlock::GURMUKHI)],
        // tamil, malayalam vowel sign O
        [
            0x46,
            0x3E,
            -1,
            0x4A,
            flag(UnicodeBlock::TAMIL) | flag(UnicodeBlock::MALAYALAM),
        ],
        // kannada vowel sign OO
        [0x46, 0x42, 0x55, 0x4B, flag(UnicodeBlock::KANNADA)],
        // kannada vowel sign O
        [0x46, 0x42, -1, 0x4A, flag(UnicodeBlock::KANNADA)],
        // malayalam vowel sign AI (if reordered twice)
        [0x46, 0x46, -1, 0x48, flag(UnicodeBlock::MALAYALAM)],
        // telugu, kannada vowel sign EE
        [
            0x46,
            0x55,
            -1,
            0x47,
            flag(UnicodeBlock::TELUGU) | flag(UnicodeBlock::KANNADA),
        ],
        // telugu, kannada vowel sign AI
        [
            0x46,
            0x56,
            -1,
            0x48,
            flag(UnicodeBlock::TELUGU) | flag(UnicodeBlock::KANNADA),
        ],
        // tamil, malayalam vowel sign AU
        [
            0x46,
            0x57,
            -1,
            0x4C,
            flag(UnicodeBlock::TAMIL) | flag(UnicodeBlock::MALAYALAM),
        ],
        // bengali, oriya vowel sign O, tamil, malayalam vowel sign OO
        [
            0x47,
            0x3E,
            -1,
            0x4B,
            flag(UnicodeBlock::BENGALI)
                | flag(UnicodeBlock::ORIYA)
                | flag(UnicodeBlock::TAMIL)
                | flag(UnicodeBlock::MALAYALAM),
        ],
        // bengali, oriya vowel sign AU
        [
            0x47,
            0x57,
            -1,
            0x4C,
            flag(UnicodeBlock::BENGALI) | flag(UnicodeBlock::ORIYA),
        ],
        // kannada vowel sign OO
        [0x4A, 0x55, -1, 0x4B, flag(UnicodeBlock::KANNADA)],
        // gurmukhi letter I
        [0x72, 0x3F, -1, 0x07, flag(UnicodeBlock::GURMUKHI)],
        // gurmukhi letter II
        [0x72, 0x40, -1, 0x08, flag(UnicodeBlock::GURMUKHI)],
        // gurmukhi letter EE
        [0x72, 0x47, -1, 0x0F, flag(UnicodeBlock::GURMUKHI)],
        // gurmukhi letter U
        [0x73, 0x41, -1, 0x09, flag(UnicodeBlock::GURMUKHI)],
        // gurmukhi letter UU
        [0x73, 0x42, -1, 0x0A, flag(UnicodeBlock::GURMUKHI)],
        // gurmukhi letter OO
        [0x73, 0x4B, -1, 0x13, flag(UnicodeBlock::GURMUKHI)],
    ]
});

/// Equivalent to the private `IndicNormalizer.scripts`, an
/// `IdentityHashMap<Character.UnicodeBlock, ScriptData>` filled by the first
/// static initializer, whose `decompMask`s the second one sets: the bit of
/// every decomposition's first offset whose flags include the script's.
static SCRIPTS: LazyLock<HashMap<UnicodeBlock, ScriptData>> = LazyLock::new(|| {
    let mut scripts = HashMap::with_capacity(9);
    for (block, flag, base) in SCRIPT_FLAGS {
        scripts.insert(block, ScriptData::new(flag, base));
    }
    for sd in scripts.values_mut() {
        for decomposition in DECOMPOSITIONS.iter() {
            let ch = decomposition[0];
            let flags = decomposition[4];
            if (flags & sd.flag) != 0 {
                sd.decomp_mask |= 1u128 << ch;
            }
        }
    }
    scripts
});

/// Normalizes the Unicode representation of text in Indian languages.
///
/// Follows guidelines from Unicode 5.2, chapter 6, South Asian Scripts I and
/// graphical decompositions from
/// http://ldc.upenn.edu/myl/IndianScriptsUnicode.html
///
/// Equivalent to `org.apache.lucene.analysis.in.IndicNormalizer`. The text
/// it normalizes is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct IndicNormalizer;

impl IndicNormalizer {
    /// Equivalent to the implicit `IndicNormalizer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Normalizes input text, and returns the new length. The length will
    /// always be less than or equal to the existing length.
    ///
    /// * `text`: input text
    /// * `len`: valid length
    /// * returns: normalized length
    ///
    /// Equivalent to `normalize(char[], int)`.
    pub(crate) fn normalize(&self, text: &mut [u16], len: usize) -> usize {
        let mut len = len;
        let mut i = 0;
        while i < len {
            let block = UnicodeBlock::of_char(text[i]);
            if let Some(sd) = block.and_then(|block| SCRIPTS.get(&block)) {
                let ch = i32::from(text[i]) - sd.base;
                if sd.decomp_mask_get(ch) {
                    len = self.compose(ch, block, sd, text, i, len);
                }
            }
            i += 1;
        }
        len
    }

    /// Compose into standard form any compositions in the decompositions
    /// table.
    ///
    /// Equivalent to `compose(int, Character.UnicodeBlock, ScriptData,
    /// char[], int, int)`; a block is `None` where Java's is `null`.
    fn compose(
        &self,
        ch0: i32,
        block0: Option<UnicodeBlock>,
        sd: &ScriptData,
        text: &mut [u16],
        pos: usize,
        len: usize,
    ) -> usize {
        if pos + 1 >= len {
            /* need at least 2 chars! */
            return len;
        }

        let ch1 = i32::from(text[pos + 1]) - sd.base;
        let block1 = UnicodeBlock::of_char(text[pos + 1]);
        if block1 != block0 {
            /* needs to be the same writing system */
            return len;
        }

        let mut ch2 = -1;

        if pos + 2 < len {
            ch2 = i32::from(text[pos + 2]) - sd.base;
            let block2 = UnicodeBlock::of_char(text[pos + 2]);
            if text[pos + 2] == 0x200D {
                // ZWJ
                ch2 = 0xFF;
            } else if block2 != block1 {
                // still allow a 2-char match
                ch2 = -1;
            }
        }

        let mut len = len;
        for decomposition in DECOMPOSITIONS.iter() {
            if decomposition[0] == ch0
                && (decomposition[4] & sd.flag) != 0
                && decomposition[1] == ch1
                && (decomposition[2] < 0 || decomposition[2] == ch2)
            {
                text[pos] = (sd.base + decomposition[3]) as u16;
                len = StemmerUtil::delete(text, pos + 1, len);
                if decomposition[2] >= 0 {
                    len = StemmerUtil::delete(text, pos + 1, len);
                }
                return len;
            }
        }

        len
    }
}
