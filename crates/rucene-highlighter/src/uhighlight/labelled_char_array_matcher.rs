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

//! Port of `LabelledCharArrayMatcher.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use std::fmt;
use std::sync::Arc;

use rucene::util::automaton::byte_run_automaton::ByteRunAutomaton;

use crate::uhighlight::char_array_matcher::CharArrayMatcher;

/// Associates a label with a CharArrayMatcher to distinguish different
/// sources for terms in highlighting.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.LabelledCharArrayMatcher`.
pub trait LabelledCharArrayMatcher: CharArrayMatcher {
    /// Equivalent to `getLabel()`: the label for this matcher.
    fn get_label(&self) -> &str;
}

/// The anonymous class `LabelledCharArrayMatcher.wrap(String,
/// CharArrayMatcher)` returns.
struct Wrapped {
    label: String,
    input: Arc<dyn CharArrayMatcher>,
}

impl fmt::Debug for Wrapped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LabelledCharArrayMatcher")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl CharArrayMatcher for Wrapped {
    fn match_chars(&self, s: &[u16], offset: usize, length: usize) -> bool {
        self.input.match_chars(s, offset, length)
    }

    fn java_to_string(&self) -> String {
        format!(
            "org.apache.lucene.search.uhighlight.LabelledCharArrayMatcher$1@{:x}",
            (self as *const Self as usize) as u32
        )
    }
}

impl LabelledCharArrayMatcher for Wrapped {
    fn get_label(&self) -> &str {
        &self.label
    }
}

/// Wraps a CharArrayMatcher with a label.
///
/// Equivalent to the static `LabelledCharArrayMatcher.wrap(String,
/// CharArrayMatcher)`.
pub fn wrap(label: &str, input: Arc<dyn CharArrayMatcher>) -> Arc<dyn LabelledCharArrayMatcher> {
    Arc::new(Wrapped {
        label: label.to_string(),
        input,
    })
}

/// The lambda of `wrap(String, ByteRunAutomaton)`: runs UTF-16 input
/// through a UTF-8 byte automaton.
struct ByteRunMatcher {
    run_automaton: ByteRunAutomaton,
}

impl fmt::Debug for ByteRunMatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ByteRunMatcher").finish_non_exhaustive()
    }
}

/// `UnicodeUtil.UTF16toUTF8(char[], int, int, byte[])`: the UTF-8 bytes of
/// `chars`, an unpaired surrogate becoming U+FFFD.
fn utf16_to_utf8(chars: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(chars.len() * 4);
    let mut i = 0;
    while i < chars.len() {
        let code = chars[i] as u32;
        i += 1;
        if code < 0x80 {
            out.push(code as u8);
        } else if code < 0x800 {
            out.push((0xC0 | (code >> 6)) as u8);
            out.push((0x80 | (code & 0x3F)) as u8);
        } else if !(0xD800..=0xDFFF).contains(&code) {
            out.push((0xE0 | (code >> 12)) as u8);
            out.push((0x80 | ((code >> 6) & 0x3F)) as u8);
            out.push((0x80 | (code & 0x3F)) as u8);
        } else {
            // surrogate pair; confirm valid high surrogate
            if code < 0xDC00 && i < chars.len() {
                let utf32 = chars[i] as u32;
                // confirm valid low surrogate and write pair
                if (0xDC00..=0xDFFF).contains(&utf32) {
                    let utf32 = (code << 10) + utf32 + (0x10000 - (0xD800 << 10) - 0xDC00);
                    i += 1;
                    out.push((0xF0 | (utf32 >> 18)) as u8);
                    out.push((0x80 | ((utf32 >> 12) & 0x3F)) as u8);
                    out.push((0x80 | ((utf32 >> 6) & 0x3F)) as u8);
                    out.push((0x80 | (utf32 & 0x3F)) as u8);
                    continue;
                }
            }
            // replace unpaired surrogate or out-of-order low surrogate with
            // substitution character
            out.push(0xEF);
            out.push(0xBF);
            out.push(0xBD);
        }
    }
    out
}

impl CharArrayMatcher for ByteRunMatcher {
    fn match_chars(&self, chars: &[u16], offset: usize, length: usize) -> bool {
        let run_automaton = &self.run_automaton;
        let mut state = 0;
        let max_idx = offset + length;
        let mut i = offset;
        while i < max_idx {
            let code = chars[i] as i32;
            if code < 0x80 {
                state = run_automaton.step(state, code);
                if state == -1 {
                    return false;
                }
            } else if code < 0x800 {
                let b = 0xC0 | (code >> 6);
                state = run_automaton.step(state, b);
                if state == -1 {
                    return false;
                }
                let b = 0x80 | (code & 0x3F);
                state = run_automaton.step(state, b);
                if state == -1 {
                    return false;
                }
            } else {
                // more complex
                let utf8_bytes = utf16_to_utf8(&chars[i..max_idx]);
                for &byte in &utf8_bytes {
                    state = run_automaton.step(state, byte as i32);
                    if state == -1 {
                        return false;
                    }
                }
                break;
            }
            i += 1;
        }
        run_automaton.is_accept(state)
    }
}

/// Wraps a ByteRunAutomaton with a label.
///
/// Equivalent to the static `LabelledCharArrayMatcher.wrap(String,
/// ByteRunAutomaton)`.
pub fn wrap_byte_run_automaton(
    label: &str,
    run_automaton: ByteRunAutomaton,
) -> Arc<dyn LabelledCharArrayMatcher> {
    wrap(label, Arc::new(ByteRunMatcher { run_automaton }))
}