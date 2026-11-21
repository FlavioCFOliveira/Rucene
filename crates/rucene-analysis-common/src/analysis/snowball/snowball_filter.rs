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

//! A filter that stems words using a Snowball-generated stemmer, ported from
//! `org.apache.lucene.analysis.snowball.SnowballFilter`.

use std::sync::Arc;

use rucene::analysis::{
    CharTermAttribute, KeywordAttribute, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::AttributeSource;

use crate::jdk::character::{chars_to_utf16, utf16_to_term_chars};
use crate::spi::snowball_stemmer_classes;
use crate::tartarus::snowball::snowball_program::SnowballProgram;
use crate::tartarus::snowball::snowball_stemmer::SnowballStemmer;

/// The body of [`SnowballFilter`]: what Lucene's `SnowballFilter` adds to
/// `TokenFilter`, its `stemmer`.
#[derive(Debug)]
pub struct SnowballFilterLogic {
    stemmer: Box<dyn SnowballStemmer>,
}

impl TokenFilterLogic for SnowballFilterLogic {
    /// Returns the next input Token, after being stemmed.
    ///
    /// Equivalent to `SnowballFilter.incrementToken()`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java hands the term buffer itself
    /// to the stemmer (`setCurrent(termBuffer, length)`), which edits it in
    /// place, and copies the stemmer's buffer back only when the stemmer
    /// replaced it with a longer one; otherwise it just sets the new length.
    /// `rucene`'s term buffer holds `char`s (see `CharTermAttribute`), not
    /// UTF-16 units, so the stemmer works on the term's UTF-16 units and
    /// the result is always copied back: the term is the same. A lone
    /// surrogate the stemmer leaves (by deleting one unit of a surrogate
    /// pair) becomes U+FFFD, the core's replacement for a lone surrogate.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let is_keyword = source
                .get_attribute_interface::<dyn KeywordAttribute>()
                .expect("INVARIANT: the constructor added the KeywordAttribute")
                .is_keyword();
            if !is_keyword {
                let mut term_att = source
                    .get_attribute_interface_mut::<dyn CharTermAttribute>()
                    .expect("INVARIANT: the constructor added the CharTermAttribute");
                let term_buffer = chars_to_utf16(&term_att.buffer()[..term_att.length()]);
                let length = term_buffer.len() as i32;
                AsMut::<SnowballProgram>::as_mut(&mut *self.stemmer)
                    .set_current_buffer(term_buffer, length);
                self.stemmer.stem();
                let program = AsRef::<SnowballProgram>::as_ref(&*self.stemmer);
                let final_term = program.get_current_buffer();
                let new_length = program.get_current_buffer_length() as usize;
                let chars = utf16_to_term_chars(&final_term[..new_length]);
                term_att.copy_buffer(&chars, 0, chars.len());
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// A filter that stems words using a Snowball-generated stemmer.
///
/// Available stemmers are listed in `org.tartarus.snowball.ext`.
///
/// **NOTE**: `SnowballFilter` expects lowercased text.
///
/// * For the Turkish language, see `TurkishLowerCaseFilter`.
/// * For other languages, see `LowerCaseFilter`.
///
/// Note: This filter is aware of the `KeywordAttribute`. To prevent certain
/// terms from being passed to the stemmer `KeywordAttribute.isKeyword()`
/// should be set to `true` in a previous `TokenStream`.
///
/// Note: For including the original term as well as the stemmed version,
/// see `org.apache.lucene.analysis.miscellaneous.KeywordRepeatFilterFactory`.
///
/// Equivalent to `org.apache.lucene.analysis.snowball.SnowballFilter`.
pub type SnowballFilter = TokenFilter<SnowballFilterLogic>;

/// Equivalent to `SnowballFilter(TokenStream input, SnowballStemmer
/// stemmer)`, whose fields add the `CharTermAttribute` and the
/// `KeywordAttribute`, in that order. Java's `Objects.requireNonNull(stemmer,
/// "stemmer")` has no counterpart: a `Box` is never null.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
pub fn new_snowball_filter(
    input: Box<dyn TokenStream>,
    stemmer: Box<dyn SnowballStemmer>,
) -> Result<SnowballFilter> {
    let mut filter = TokenFilter::new(input, SnowballFilterLogic { stemmer });
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn KeywordAttribute>()?;
    Ok(filter)
}

/// Construct the named stemming filter.
///
/// Available stemmers are listed in `org.tartarus.snowball.ext`. The name of
/// a stemmer is the part of the class name before "Stemmer", e.g., the
/// stemmer in `org.tartarus.snowball.ext.EnglishStemmer` is named
/// "English". `in` is the input tokens to stem, `name` the name of a
/// stemmer.
///
/// Equivalent to `SnowballFilter(TokenStream in, String name)`. The name
/// "German2" stands for "German", as in Java. The stemmer is resolved
/// before the filter adds its attributes, where Java adds them first: the
/// input is consumed either way, so the order is not observable.
///
/// **Divergence from Lucene 10.5.0.** Java loads the class
/// `"org.tartarus.snowball.ext." + name + "Stemmer"` with `Class.forName`
/// and calls its public no-argument constructor; Rust has no runtime class
/// loading, so the class is looked up in [`snowball_stemmer_classes`], which
/// holds the 30 stemmers of `org.tartarus.snowball.ext` and what an
/// application registers. A name the table lacks fails as Java's
/// `ReflectiveOperationException` (`ClassNotFoundException`) path does.
///
/// # Errors
///
/// `IllegalArgumentException("Invalid stemmer class specified: " + name)`
/// when no stemmer class has that name (Java's cause, the reflective
/// exception, is rendered as `ClassNotFoundException` with the class name);
/// and what [`new_snowball_filter`] raises.
pub fn new_snowball_filter_by_name(
    input: Box<dyn TokenStream>,
    name: &str,
) -> Result<SnowballFilter> {
    // it was called "German2" for eons, but snowball folded it into "German" and deleted "German2"
    // for now, don't annoy our users...
    let name = if name == "German2" { "German" } else { name };
    let class_name = format!("org.tartarus.snowball.ext.{name}Stemmer");
    let stem_class = snowball_stemmer_classes()
        .read()
        .expect("INVARIANT: no thread panics while holding the stemmer class table")
        .find_class(&class_name)
        .cloned();
    match stem_class {
        Ok(stem_class) => new_snowball_filter(input, stem_class()()),
        Err(_) => Err(LuceneError::illegal_argument_with_cause(
            format!("Invalid stemmer class specified: {name}"),
            Arc::new(LuceneError::unchecked(
                "java.lang.ClassNotFoundException",
                class_name,
            )),
        )),
    }
}
