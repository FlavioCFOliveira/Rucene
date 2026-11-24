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

//! A `TokenFilter` that transforms text with ICU, ported from
//! `org.apache.lucene.analysis.icu.ICUTransformFilter`.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use icu4j::text::replaceable::Replaceable;
use icu4j::text::rule_based_transliterator::RuleBasedTransliterator;
use icu4j::text::transliterator::{Position, Transliterator};
use icu4j::text::unicode_filter::UnicodeFilter;
use icu4j::text::utf16::UTF16;
use rucene::analysis::character_utils::{set_term_utf16, term_to_utf16};
use rucene::analysis::{CharTermAttribute, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;

/// A `TokenFilter` that transforms text with ICU.
///
/// ICU provides text-transformation functionality via its Transliteration
/// API. Although script conversion is its most common use, a Transliterator
/// can actually perform a more general class of tasks. In fact,
/// Transliterator defines a very general API which specifies only that a
/// segment of the input text is replaced by new text. The particulars of
/// this conversion are determined entirely by subclasses of Transliterator.
///
/// Some useful transformations for search are built-in:
///
/// * Conversion from Traditional to Simplified Chinese characters
/// * Conversion from Hiragana to Katakana
/// * Conversion from Fullwidth to Halfwidth forms.
/// * Script conversions, for example Serbian Cyrillic to Latin
///
/// Example usage:
///
/// ```text
/// stream = new ICUTransformFilter(stream, Transliterator.getInstance("Traditional-Simplified"));
/// ```
///
/// For more details, see the [ICU User
/// Guide](http://userguide.icu-project.org/transforms/general).
///
/// Equivalent to the body of `org.apache.lucene.analysis.icu.ICUTransformFilter`
/// (a final `TokenFilter`): its fields and `incrementToken()`. The filter is
/// [`ICUTransformFilter`].
#[derive(Debug)]
pub struct ICUTransformFilterLogic {
    // Transliterator to transform the text
    transform: Arc<dyn Transliterator>,

    // Reusable position object
    position: Position,

    // Wraps a termAttribute around the replaceable interface.
    replaceable_attribute: ReplaceableTermAttribute,

    /// The term's UTF-16 units, read once per token.
    units: Vec<u16>,
}

impl TokenFilterLogic for ICUTransformFilterLogic {
    /// Equivalent to `ICUTransformFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        /*
         * Wrap around replaceable. clear the positions, and transliterate.
         */
        if input.increment_token()? {
            let mut term_att = source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            term_to_utf16(&term_att.buffer()[..term_att.length()], &mut self.units);
            self.replaceable_attribute.set_text(&self.units);

            let length = self.units.len() as i32;
            self.position.start = 0;
            self.position.limit = length;
            self.position.context_start = 0;
            self.position.context_limit = length;

            self.transform.filtered_transliterate(
                &self.replaceable_attribute,
                &mut self.position,
                false,
            )?;
            set_term_utf16(&mut *term_att, &self.replaceable_attribute.text());
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// A `TokenFilter` that transforms text with ICU.
///
/// Equivalent to `org.apache.lucene.analysis.icu.ICUTransformFilter`; see
/// [`ICUTransformFilterLogic`].
pub type ICUTransformFilter = TokenFilter<ICUTransformFilterLogic>;

/// Create a new ICUTransformFilter that transforms text on the given
/// stream.
///
/// * `input`: `TokenStream` to filter.
/// * `transform`: Transliterator to transform the text.
///
/// Equivalent to `ICUTransformFilter(TokenStream, Transliterator)`.
///
/// # Errors
///
/// The errors of `getSourceSet()` and `setFilter` on a rule-based
/// transliterator, and of adding the `CharTermAttribute`.
#[allow(deprecated)]
pub fn new_icu_transform_filter(
    input: Box<dyn TokenStream>,
    transform: Arc<dyn Transliterator>,
) -> Result<ICUTransformFilter> {
    /*
     * This is cheating, but speeds things up a lot.
     * If we wanted to use pkg-private APIs we could probably do better.
     */
    if transform.get_filter().is_none()
        && transform
            .as_any()
            .downcast_ref::<RuleBasedTransliterator>()
            .is_some()
    {
        let source_set = transform.get_source_set()?;
        if !source_set.is_empty() {
            transform.set_filter(Some(&source_set as &dyn UnicodeFilter))?;
        }
    }
    let mut filter = TokenFilter::new(
        input,
        ICUTransformFilterLogic {
            transform,
            position: Position::new(),
            replaceable_attribute: ReplaceableTermAttribute::default(),
            units: Vec::new(),
        },
    );
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(filter)
}

/// Wrap a `CharTermAttribute` with the Replaceable API.
///
/// Equivalent to the nested class `ICUTransformFilter.ReplaceableTermAttribute`.
///
/// **Divergence from Lucene 10.5.0 (inherited).** Java wraps the term
/// buffer itself (`token.buffer()`), edits it in place, and keeps the
/// token's length in step (`token.setLength`, `token.resizeBuffer`).
/// `rucene`'s term buffer holds Rust `char`s, not UTF-16 units, so the
/// wrapper holds the term's UTF-16 units ([`set_text`](Self::set_text)) and
/// the filter writes the edited units back to the term when the
/// transliteration ends: nothing reads the term in between, so the term the
/// filter leaves is the same. The units sit behind a lock because
/// `Replaceable`'s methods take `&self` (see `icu4j::text::replaceable`).
#[derive(Debug, Default)]
pub(crate) struct ReplaceableTermAttribute {
    state: Mutex<ReplaceableState>,
}

/// The fields `buffer` and `length` of `ReplaceableTermAttribute`.
#[derive(Debug, Default)]
struct ReplaceableState {
    buffer: Vec<u16>,
    length: i32,
}

impl ReplaceableState {
    /// `getChars(int, int, char[], int)`.
    fn get_chars(&self, src_start: i32, src_limit: i32, dst: &mut [u16], dst_start: i32) {
        let n = (src_limit - src_start) as usize;
        let (s, d) = (src_start as usize, dst_start as usize);
        dst[d..d + n].copy_from_slice(&self.buffer[s..s + n]);
    }

    /// `replace(int, int, char[], int, int)`.
    fn replace_chars(
        &mut self,
        start: i32,
        limit: i32,
        text: &[u16],
        chars_start: i32,
        chars_len: i32,
    ) {
        // shift text if necessary for the replacement
        let new_length = self.shift_for_replace(start, limit, chars_len);
        // insert the replacement text
        let (s, cs, n) = (start as usize, chars_start as usize, chars_len as usize);
        self.buffer[s..s + n].copy_from_slice(&text[cs..cs + n]);
        self.length = new_length;
    }

    /// `shiftForReplace(int, int, int)`.
    fn shift_for_replace(&mut self, start: i32, limit: i32, chars_len: i32) -> i32 {
        let replacement_length = limit - start;
        let new_length = self.length - replacement_length + chars_len;
        // resize if necessary
        if new_length > self.length && (self.buffer.len() as i32) < new_length {
            self.buffer.resize(new_length as usize, 0);
        }
        // if the substring being replaced is longer or shorter than the
        // replacement, need to shift things around
        if replacement_length != chars_len && limit < self.length {
            self.buffer.copy_within(
                limit as usize..self.length as usize,
                (start + chars_len) as usize,
            );
        }
        new_length
    }
}

impl ReplaceableTermAttribute {
    /// Equivalent to `setText(CharTermAttribute)`, over the term's UTF-16
    /// units.
    pub(crate) fn set_text(&mut self, token: &[u16]) {
        let state = self.state.get_mut().unwrap_or_else(PoisonError::into_inner);
        state.buffer.clear();
        state.buffer.extend_from_slice(token);
        state.length = token.len() as i32;
    }

    /// The term the wrapper holds: the units `buffer[0..length]`, what
    /// Java's `token` holds after the edits.
    pub(crate) fn text(&self) -> Vec<u16> {
        let state = self.lock();
        state.buffer[..state.length as usize].to_vec()
    }

    fn lock(&self) -> MutexGuard<'_, ReplaceableState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Replaceable for ReplaceableTermAttribute {
    /// Equivalent to `char32At(int)`, `UTF16.charAt(buffer, 0, length, pos)`.
    fn char32_at(&self, pos: i32) -> i32 {
        let state = self.lock();
        UTF16::char_at_chars(&state.buffer, 0, state.length, pos)
    }

    /// Equivalent to `charAt(int)`.
    fn char_at(&self, pos: i32) -> u16 {
        self.lock().buffer[pos as usize]
    }

    /// Equivalent to `copy(int, int, int)`.
    fn copy(&self, start: i32, limit: i32, dest: i32) {
        let mut state = self.lock();
        let mut text = vec![0u16; (limit - start) as usize];
        state.get_chars(start, limit, &mut text, 0);
        state.replace_chars(dest, dest, &text, 0, limit - start);
    }

    /// Equivalent to `getChars(int, int, char[], int)`.
    fn get_chars(&self, src_start: i32, src_limit: i32, dst: &mut [u16], dst_start: i32) {
        self.lock().get_chars(src_start, src_limit, dst, dst_start);
    }

    /// Equivalent to `hasMetaData()`: `false`.
    fn has_meta_data(&self) -> bool {
        false
    }

    /// Equivalent to `length()`.
    fn length(&self) -> i32 {
        self.lock().length
    }

    /// Equivalent to `replace(int, int, String)`.
    fn replace(&self, start: i32, limit: i32, text: &[u16]) {
        let mut state = self.lock();
        let chars_len = text.len() as i32;
        let new_length = state.shift_for_replace(start, limit, chars_len);
        // insert the replacement text
        let s = start as usize;
        state.buffer[s..s + text.len()].copy_from_slice(text);
        state.length = new_length;
    }

    /// Equivalent to `replace(int, int, char[], int, int)`.
    fn replace_chars(
        &self,
        start: i32,
        limit: i32,
        text: &[u16],
        chars_start: i32,
        chars_len: i32,
    ) {
        self.lock()
            .replace_chars(start, limit, text, chars_start, chars_len);
    }
}
