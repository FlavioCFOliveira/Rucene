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

//! Port of `CharArrayMatcher.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use std::fmt::Debug;
use std::sync::Arc;

use rucene::error::Result;
use rucene::util::automaton::automata::Automata;
use rucene::util::automaton::character_run_automaton::CharacterRunAutomaton;
use rucene::util::BytesRef;

/// Matches a character array.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.CharArrayMatcher`.
/// The characters are Java `char`s: UTF-16 code units.
pub trait CharArrayMatcher: Debug + Send + Sync {
    /// Return `true` if the passed-in character array matches.
    ///
    /// Equivalent to `match(char[], int, int)`.
    fn match_chars(&self, s: &[u16], offset: usize, length: usize) -> bool;

    /// Return `true` if the passed-in CharsRef matches.
    ///
    /// Equivalent to `match(CharsRef)`: the whole of `chars`.
    fn match_chars_ref(&self, chars: &[u16]) -> bool {
        self.match_chars(chars, 0, chars.len())
    }

    /// `Object.toString()` of the matcher: its class name and an identity
    /// hash code.
    ///
    /// **Divergence.** Java's identity hash code is replaced by the address
    /// of the matcher, the port's convention for `Object.hashCode()`.
    fn java_to_string(&self) -> String {
        format!(
            "{}@{:x}",
            std::any::type_name::<Self>(),
            (self as *const Self as *const () as usize) as u32
        )
    }
}

/// The method reference `CharacterRunAutomaton::run` as a
/// `CharArrayMatcher`.
#[derive(Debug)]
pub(crate) struct RunAutomatonMatcher {
    pub(crate) automaton: CharacterRunAutomaton,
}

impl CharArrayMatcher for RunAutomatonMatcher {
    fn match_chars(&self, s: &[u16], offset: usize, length: usize) -> bool {
        self.automaton.run_units(s, offset, length)
    }

    fn java_to_string(&self) -> String {
        format!(
            "org.apache.lucene.search.uhighlight.CharArrayMatcher$$Lambda@{:x}",
            (self as *const Self as usize) as u32
        )
    }
}

/// Returns a matcher that matches any of the given terms.
///
/// Equivalent to the static `CharArrayMatcher.fromTerms(List)`.
///
/// # Errors
///
/// What building the string-union automaton raises (an unsorted list, a
/// term too long).
pub fn from_terms(terms: &[BytesRef]) -> Result<Arc<dyn CharArrayMatcher>> {
    let a = CharacterRunAutomaton::new(Automata::make_string_union(terms.iter().cloned())?)?;
    Ok(Arc::new(RunAutomatonMatcher { automaton: a }))
}
