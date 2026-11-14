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

//! Constructs bigrams for frequently occurring terms, ported from
//! `org.apache.lucene.analysis.commongrams.CommonGramsFilter`.

use rucene::analysis::{
    CharArraySet, CharTermAttribute, OffsetAttribute, PositionIncrementAttribute,
    PositionLengthAttribute, TokenFilter, TokenFilterLogic, TokenStream, TypeAttribute,
};
use rucene::error::Result;
use rucene::util::attribute::{AttributeSource, CapturedState};

use crate::jdk::character::chars_to_utf16;

/// Equivalent to `CommonGramsFilter.GRAM_TYPE`.
pub const GRAM_TYPE: &str = "gram";

/// Equivalent to the private `CommonGramsFilter.SEPARATOR`.
const SEPARATOR: char = '_';

/// The body of [`CommonGramsFilter`]: what Lucene's `CommonGramsFilter` adds
/// to `TokenFilter`.
#[derive(Debug)]
pub struct CommonGramsFilterLogic {
    common_words: Option<CharArraySet>,
    buffer: Vec<char>,
    last_start_offset: i32,
    last_was_common: bool,
    saved_state: Option<CapturedState>,
}

fn term_of(source: &AttributeSource) -> Vec<char> {
    let term = source
        .get_attribute_interface::<dyn CharTermAttribute>()
        .expect("INVARIANT: the constructor added the CharTermAttribute");
    term.buffer()[..term.length()].to_vec()
}

impl CommonGramsFilterLogic {
    /// Determines if the current token is a common term
    ///
    /// Equivalent to the private `isCommon()`.
    fn is_common(&self, source: &AttributeSource) -> bool {
        match &self.common_words {
            None => false,
            Some(common_words) => {
                let term = chars_to_utf16(&term_of(source));
                common_words.contains_chars(Some(&term), 0, term.len())
            }
        }
    }

    /// Saves this information to form the left part of a gram
    ///
    /// Equivalent to the private `saveTermBuffer()`.
    fn save_term_buffer(&mut self, source: &AttributeSource) {
        self.buffer.clear();
        self.buffer.extend(term_of(source));
        self.buffer.push(SEPARATOR);
        self.last_start_offset = source
            .get_attribute_interface::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .start_offset();
        self.last_was_common = self.is_common(source);
    }

    /// Constructs a compound token.
    ///
    /// Equivalent to the private `gramToken()`.
    fn gram_token(&mut self, source: &AttributeSource) {
        self.buffer.extend(term_of(source));
        let end_offset = source
            .get_attribute_interface::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .end_offset();

        source.clear_attributes();

        source
            .get_attribute_interface_mut::<dyn CharTermAttribute>()
            .expect("INVARIANT: the constructor added the CharTermAttribute")
            .copy_buffer(&self.buffer, 0, self.buffer.len());
        source
            .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
            .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
            .set_position_increment(0);
        source
            .get_attribute_interface_mut::<dyn PositionLengthAttribute>()
            .expect("INVARIANT: the constructor added the PositionLengthAttribute")
            .set_position_length(2); // bigram
        source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .set_offset(self.last_start_offset, end_offset);
        source
            .get_attribute_interface_mut::<dyn TypeAttribute>()
            .expect("INVARIANT: the constructor added the TypeAttribute")
            .set_type_static(GRAM_TYPE);
        self.buffer.clear();
    }
}

impl TokenFilterLogic for CommonGramsFilterLogic {
    /// Inserts bigrams for common words into a token stream. For each input
    /// token, output the token. If the token and/or the following token are
    /// in the list of common words also output a bigram with position
    /// increment 0 and type="gram"
    ///
    /// TODO:Consider adding an option to not emit unigram stopwords as in
    /// CDL XTF BigramStopFilter, CommonGramsQueryFilter would need to be
    /// changed to work with this.
    ///
    /// TODO: Consider optimizing for the case of three commongrams i.e "man
    /// of the year" normally produces 3 bigrams: "man-of", "of-the",
    /// "the-year" but with proper management of positions we could eliminate
    /// the middle bigram "of-the"and save a disk seek and a whole set of
    /// position lookups.
    ///
    /// Equivalent to `CommonGramsFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        // get the next piece of input
        if let Some(saved_state) = self.saved_state.take() {
            source.restore_state(&saved_state)?;
            self.save_term_buffer(source);
            return Ok(true);
        } else if !input.increment_token()? {
            return Ok(false);
        }

        /* We build n-grams before and after stopwords.
         * When valid, the buffer always contains at least the separator.
         * If it's empty, there is nothing before this stopword.
         */
        if self.last_was_common || (self.is_common(source) && !self.buffer.is_empty()) {
            self.saved_state = source.capture_state();
            self.gram_token(source);
            return Ok(true);
        }

        self.save_term_buffer(source);
        Ok(true)
    }

    /// Equivalent to `CommonGramsFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.last_was_common = false;
        self.saved_state = None;
        self.buffer.clear();
        Ok(())
    }
}

/// Construct bigrams for frequently occurring terms while indexing. Single
/// terms are still indexed too, with bigrams overlaid. This is achieved
/// through the use of `PositionIncrementAttribute#setPositionIncrement(int)`.
/// Bigrams have a type of [`GRAM_TYPE`] Example:
///
/// * input:"the quick brown fox"
/// * output:|"the","the-quick"|"brown"|"fox"|
/// * "the-quick" has a position increment of 0 so it is in the same position
///   as "the" "the-quick" has a term.type() of "gram"
///
/// Equivalent to `org.apache.lucene.analysis.commongrams.CommonGramsFilter`.
pub type CommonGramsFilter = TokenFilter<CommonGramsFilterLogic>;

/// Construct a token stream filtering the given input using a Set of common
/// words to create bigrams. Outputs both unigrams with position increment
/// and bigrams with position increment 0 type=gram where one or both of the
/// words in a potential bigram are in the set of common words .
///
/// * `input` TokenStream input in filter chain
/// * `common_words` The set of common words (`None` for Java's `null`, which
///   makes no word common).
///
/// Equivalent to `CommonGramsFilter(TokenStream, CharArraySet)`, which adds
/// the `CharTermAttribute`, `OffsetAttribute`, `TypeAttribute`,
/// `PositionIncrementAttribute` and `PositionLengthAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_common_grams_filter(
    input: Box<dyn TokenStream>,
    common_words: Option<CharArraySet>,
) -> Result<CommonGramsFilter> {
    let mut filter = TokenFilter::new(
        input,
        CommonGramsFilterLogic {
            common_words,
            buffer: Vec::new(),
            last_start_offset: 0,
            last_was_common: false,
            saved_state: None,
        },
    );
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    source.add_attribute_interface::<dyn TypeAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    source.add_attribute_interface::<dyn PositionLengthAttribute>()?;
    Ok(filter)
}
