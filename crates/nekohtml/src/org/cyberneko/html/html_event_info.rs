/*
 * Copyright 2002-2009 Andy Clark, Marc Guillemot
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `HTMLEventInfo.java`, of the Java package `org.cyberneko.html` (NekoHTML 1.9.17).

/// This interface is used to pass augmentated information to the
/// application through the XNI pipeline.
///
/// Equivalent to `org.cyberneko.html.HTMLEventInfo`.
pub trait HTMLEventInfo {
    /// Returns the line number of the beginning of this event.
    fn get_begin_line_number(&self) -> i32;
    /// Returns the column number of the beginning of this event.
    fn get_begin_column_number(&self) -> i32;
    /// Returns the character offset of the beginning of this event.
    fn get_begin_character_offset(&self) -> i32;
    /// Returns the line number of the end of this event.
    fn get_end_line_number(&self) -> i32;
    /// Returns the column number of the end of this event.
    fn get_end_column_number(&self) -> i32;
    /// Returns the character offset of the end of this event.
    fn get_end_character_offset(&self) -> i32;
    /// Returns true if this corresponding event was synthesized.
    fn is_synthesized(&self) -> bool;
}

/// Synthesized infoset item.
///
/// Equivalent to the nested class `HTMLEventInfo.SynthesizedItem`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SynthesizedItem;

impl HTMLEventInfo for SynthesizedItem {
    fn get_begin_line_number(&self) -> i32 {
        -1
    }

    fn get_begin_column_number(&self) -> i32 {
        -1
    }

    fn get_begin_character_offset(&self) -> i32 {
        -1
    }

    fn get_end_line_number(&self) -> i32 {
        -1
    }

    fn get_end_column_number(&self) -> i32 {
        -1
    }

    fn get_end_character_offset(&self) -> i32 {
        -1
    }

    fn is_synthesized(&self) -> bool {
        true
    }
}

impl std::fmt::Display for SynthesizedItem {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("synthesized")
    }
}
