/*
 * Copyright (C) 1999-2010, International Business Machines
 * Corporation and others.  All Rights Reserved.
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, and/or sell copies of the
 * Software, and to permit persons to whom the Software is furnished to do so,
 * provided that the above copyright notice(s) and this permission notice appear
 * in all copies of the Software and that both the above copyright notice(s) and
 * this permission notice appear in supporting documentation.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF THIRD PARTY RIGHTS.
 * IN NO EVENT SHALL THE COPYRIGHT HOLDER OR HOLDERS INCLUDED IN THIS NOTICE BE
 * LIABLE FOR ANY CLAIM, OR ANY SPECIAL INDIRECT OR CONSEQUENTIAL DAMAGES, OR
 * ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER
 * IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT
 * OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 *
 * Except as contained in this notice, the name of a copyright holder shall not
 * be used in advertising or otherwise to promote the sale, use or other
 * dealings in this Software without prior written authorization of the
 * copyright holder.
 */

//! An iterator that locates ISO 15924 script boundaries in text, ported
//! from `org.apache.lucene.analysis.icu.segmentation.ScriptIterator`.

use std::sync::{Arc, LazyLock};

use icu4j::lang::u_character::UCharacter;
use icu4j::lang::u_character_enums::ECharacterCategory;
use icu4j::lang::u_script::UScript;
use icu4j::text::utf16::UTF16;

/// An iterator that locates ISO 15924 script boundaries in text.
///
/// This is not the same as simply looking at the Unicode block, or even the
/// Script property. Some characters are 'common' across multiple scripts,
/// and some 'inherit' the script value of text surrounding them.
///
/// This is similar to ICU (internal-only) UScriptRun, with the following
/// differences:
///
/// * Doesn't attempt to match paired punctuation. For tokenization
///   purposes, this is not necessary. It's also quite expensive.
/// * Non-spacing marks inherit the script of their base character,
///   following recommendations from UTR #24.
///
/// Equivalent to `org.apache.lucene.analysis.icu.segmentation.ScriptIterator`
/// (package-private, `@lucene.experimental`). The text is shared as an
/// `Arc<[u16]>`, as for [`CharArrayIterator`](super::char_array_iterator::CharArrayIterator).
#[derive(Debug)]
pub struct ScriptIterator {
    text: Arc<[u16]>,
    start: i32,
    limit: i32,
    index: i32,

    script_start: i32,
    script_limit: i32,
    script_code: i32,

    combine_cj: bool,
}

/// `basicLatin`: linear fast-path for basic latin case, filled by the
/// class's static initializer with `UScript.getScript(i)` for `i < 128`.
static BASIC_LATIN: LazyLock<[i32; 128]> = LazyLock::new(|| {
    let mut basic_latin = [0i32; 128];
    for (i, slot) in basic_latin.iter_mut().enumerate() {
        *slot = UScript::get_script(i as i32)
            .expect("INVARIANT: a Basic Latin code point is a valid code point");
    }
    basic_latin
});

impl ScriptIterator {
    /// * `combine_cj`: if true: Han,Hiragana,Katakana will all return as
    ///   `UScript.JAPANESE`
    ///
    /// Equivalent to `ScriptIterator(boolean)`.
    pub fn new(combine_cj: bool) -> Self {
        Self {
            text: Arc::from(Vec::new()),
            start: 0,
            limit: 0,
            index: 0,
            script_start: 0,
            script_limit: 0,
            script_code: 0,
            combine_cj,
        }
    }

    /// Get the start of this script run: the start position of the script
    /// run.
    ///
    /// Equivalent to `getScriptStart()`.
    pub fn get_script_start(&self) -> i32 {
        self.script_start
    }

    /// Get the index of the first character after the end of this script
    /// run: the position of the first character after this script run.
    ///
    /// Equivalent to `getScriptLimit()`.
    pub fn get_script_limit(&self) -> i32 {
        self.script_limit
    }

    /// Get the UScript script code for this script run: the code for the
    /// script of the current run.
    ///
    /// Equivalent to `getScriptCode()`.
    pub fn get_script_code(&self) -> i32 {
        self.script_code
    }

    /// Iterates to the next script run, returning true if one exists: true
    /// if there is another script run, false otherwise.
    ///
    /// Equivalent to `next()`.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> bool {
        if self.script_limit >= self.limit {
            return false;
        }

        self.script_code = UScript::COMMON;
        self.script_start = self.script_limit;

        while self.index < self.limit {
            let ch = UTF16::char_at_chars(&self.text, self.start, self.limit, self.index - self.start);
            let sc = self.get_script(ch);

            /*
             * From UTR #24: Implementations that determine the boundaries between
             * characters of given scripts should never break between a combining
             * mark and its base character. Thus for boundary determinations and
             * similar sorts of processing, a combining mark — whatever its script
             * value — should inherit the script value of its base character.
             */
            if Self::is_same_script(self.script_code, sc, ch) || Self::is_combining_mark(ch) {
                self.index += UTF16::get_char_count(ch);

                /*
                 * Inherited or Common becomes the script code of the surrounding text.
                 */
                if self.script_code <= UScript::INHERITED && sc > UScript::INHERITED {
                    self.script_code = sc;
                }
            } else {
                break;
            }
        }

        self.script_limit = self.index;
        true
    }

    /// Determine if two scripts are compatible.
    ///
    /// Equivalent to `isSameScript(int, int, int)`.
    fn is_same_script(current_script: i32, script: i32, codepoint: i32) -> bool {
        // same scripts match
        // inherited/common are compatible with any script
        // codepoints with the current script in Script_Extensions match
        current_script == script
            || current_script <= UScript::INHERITED
            || script <= UScript::INHERITED
            || UScript::has_script(codepoint, current_script)
    }

    /// Determine if codepoint is a combining mark (General_Category of Mc,
    /// Mn, Me).
    ///
    /// Equivalent to `isCombiningMark(int)`.
    fn is_combining_mark(codepoint: i32) -> bool {
        let ty = UCharacter::get_type(codepoint);
        ty == i32::from(ECharacterCategory::COMBINING_SPACING_MARK)
            || ty == i32::from(ECharacterCategory::NON_SPACING_MARK)
            || ty == i32::from(ECharacterCategory::ENCLOSING_MARK)
    }

    /// Set a new region of text to be examined by this iterator.
    ///
    /// * `text`: text buffer to examine
    /// * `start`: offset into buffer
    /// * `length`: maximum length to examine
    ///
    /// Equivalent to `setText(char[], int, int)`.
    pub fn set_text(&mut self, text: Arc<[u16]>, start: i32, length: i32) {
        self.text = text;
        self.start = start;
        self.index = start;
        self.limit = start + length;
        self.script_start = start;
        self.script_limit = start;
        self.script_code = UScript::INVALID_CODE;
    }

    /// fast version of UScript.getScript(). Basic Latin is an array lookup
    ///
    /// Equivalent to `getScript(int)`.
    fn get_script(&self, codepoint: i32) -> i32 {
        if (0..BASIC_LATIN.len() as i32).contains(&codepoint) {
            BASIC_LATIN[codepoint as usize]
        } else {
            let script = UScript::get_script(codepoint)
                .expect("INVARIANT: a code point read from UTF-16 text is a valid code point");
            if self.combine_cj {
                if script == UScript::HAN || script == UScript::HIRAGANA || script == UScript::KATAKANA {
                    UScript::JAPANESE
                } else if (0xFF10..=0xFF19).contains(&codepoint) {
                    // when using CJK dictionary breaking, don't let full width numbers go to it, otherwise
                    // they are treated as punctuation. we currently have no cleaner way to fix this!
                    UScript::LATIN
                } else {
                    script
                }
            } else {
                script
            }
        }
    }
}
