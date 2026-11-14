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

//! The type declared in `Token.java`, of the Java package
//! `org.apache.lucene.analysis.morph`.

use std::sync::Arc;

use super::token_type::TokenType;

/// Analyzed token with morphological data.
///
/// Equivalent to `org.apache.lucene.analysis.morph.Token`, an abstract
/// class: this struct holds its fields, and a concrete token type (a
/// subclass) holds a `Token` and implements [`TokenSubclass`]. The surface
/// form, a `char[]` Java shares between tokens, is a shared slice of UTF-16
/// units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    surface_form: Arc<[u16]>,
    offset: i32,
    length: i32,
    start_offset: i32,
    end_offset: i32,
    pos_incr: i32,
    pos_len: i32,
    token_type: TokenType,
}

impl Token {
    /// Equivalent to the protected `Token(char[] surfaceForm, int offset, int
    /// length, int startOffset, int endOffset, TokenType type)`.
    pub fn new(
        surface_form: Arc<[u16]>,
        offset: i32,
        length: i32,
        start_offset: i32,
        end_offset: i32,
        token_type: TokenType,
    ) -> Self {
        Self {
            surface_form,
            offset,
            length,
            start_offset,
            end_offset,
            pos_incr: 1,
            pos_len: 1,
            token_type,
        }
    }

    /// Returns surfaceForm
    ///
    /// Equivalent to `Token.getSurfaceForm()`.
    pub fn get_surface_form(&self) -> &Arc<[u16]> {
        &self.surface_form
    }

    /// Returns offset into surfaceForm
    ///
    /// Equivalent to `Token.getOffset()`.
    pub fn get_offset(&self) -> i32 {
        self.offset
    }

    /// Returns length of surfaceForm
    ///
    /// Equivalent to `Token.getLength()`.
    pub fn get_length(&self) -> i32 {
        self.length
    }

    /// Returns surfaceForm as a String, as UTF-16 units
    ///
    /// Equivalent to `Token.getSurfaceFormString()`, `new String(surfaceForm,
    /// offset, length)`.
    ///
    /// # Panics
    ///
    /// For a range outside the surface form, where Java throws
    /// `StringIndexOutOfBoundsException`.
    pub fn get_surface_form_string(&self) -> Vec<u16> {
        self.surface_form[self.offset as usize..(self.offset + self.length) as usize].to_vec()
    }

    /// Get the start offset of the term in the analyzed text.
    ///
    /// Equivalent to `Token.getStartOffset()`.
    pub fn get_start_offset(&self) -> i32 {
        self.start_offset
    }

    /// Get the end offset of the term in the analyzed text.
    ///
    /// Equivalent to `Token.getEndOffset()`.
    pub fn get_end_offset(&self) -> i32 {
        self.end_offset
    }

    /// Equivalent to `Token.setPositionIncrement(int)`.
    pub fn set_position_increment(&mut self, pos_incr: i32) {
        self.pos_incr = pos_incr;
    }

    /// Equivalent to `Token.getPositionIncrement()`.
    pub fn get_position_increment(&self) -> i32 {
        self.pos_incr
    }

    /// Set the position length (in tokens) of this token. For normal tokens
    /// this is 1; for compound tokens it's &gt; 1.
    ///
    /// Equivalent to `Token.setPositionLength(int)`.
    pub fn set_position_length(&mut self, pos_len: i32) {
        self.pos_len = pos_len;
    }

    /// Get the length (in tokens) of this token. For normal tokens this is 1;
    /// for compound tokens it's &gt; 1.
    ///
    /// Equivalent to `Token.getPositionLength()`.
    pub fn get_position_length(&self) -> i32 {
        self.pos_len
    }

    /// Returns the type of this token
    ///
    /// Equivalent to `Token.getType()`.
    pub fn get_type(&self) -> TokenType {
        self.token_type
    }
}

/// A concrete token type: a subclass of `Token`, reaching the fields it
/// inherits.
pub trait TokenSubclass: Send + Sync {
    /// The `Token` this token extends.
    fn token(&self) -> &Token;

    /// The `Token` this token extends, for its setters.
    fn token_mut(&mut self) -> &mut Token;
}

impl TokenSubclass for Token {
    fn token(&self) -> &Token {
        self
    }

    fn token_mut(&mut self) -> &mut Token {
        self
    }
}
