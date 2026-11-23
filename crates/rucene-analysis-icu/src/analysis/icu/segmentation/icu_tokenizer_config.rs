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

//! Class that allows for tailored Unicode Text Segmentation on a
//! per-writing system basis, ported from
//! `org.apache.lucene.analysis.icu.segmentation.ICUTokenizerConfig`.

use std::borrow::Cow;
use std::fmt::Debug;

use icu4j::text::rule_based_break_iterator::RuleBasedBreakIterator;

/// Rule status for emoji sequences.
///
/// Equivalent to `ICUTokenizerConfig.EMOJI_SEQUENCE_STATUS`.
pub const EMOJI_SEQUENCE_STATUS: i32 = 299;

/// Class that allows for tailored Unicode Text Segmentation on a
/// per-writing system basis.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.analysis.icu.segmentation.ICUTokenizerConfig`
/// (`@lucene.experimental`), whose sole constructor does nothing: its three
/// abstract methods are this trait's methods. A config is shared by the
/// tokenizers built over it (`Arc<dyn ICUTokenizerConfig>`), as Java shares
/// the object.
pub trait ICUTokenizerConfig: Debug + Send + Sync {
    /// Return a breakiterator capable of processing a given script.
    ///
    /// Equivalent to `getBreakIterator(int)`.
    fn get_break_iterator(&self, script: i32) -> RuleBasedBreakIterator;

    /// Return a token type value for a given script and BreakIterator rule
    /// status.
    ///
    /// Equivalent to `getType(int, int)`. A borrowed `'static` string is a
    /// constant whose identity consumers may compare (the
    /// `StandardTokenizer.TOKEN_TYPES` strings, which `CJKBigramFilter`
    /// compares by reference), as Java returns the constant itself.
    fn get_type(&self, script: i32, rule_status: i32) -> Cow<'static, str>;

    /// true if Han, Hiragana, and Katakana scripts should all be returned as
    /// Japanese.
    ///
    /// Equivalent to `combineCJ()`.
    fn combine_cj(&self) -> bool;
}
