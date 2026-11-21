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

//! The port of `CharStream.java`, of the Java package
//! `org.apache.lucene.queryparser.charstream`.

use rucene::error::Result;

/// A character stream that maintains the line and column positions of the
/// characters, and can back up to some extent. An implementation is used by
/// the token managers JavaCC generates.
///
/// All the methods except `backup` can be implemented in any fashion.
/// `backup` must be implemented correctly for the lexer to work; the others
/// only give information (line and column numbers, the text of a token) the
/// lexer does not rely on.
///
/// Equivalent to `org.apache.lucene.queryparser.charstream.CharStream`. The
/// characters are UTF-16 code units, Java's `char`; a method that throws
/// `java.io.IOException` returns a [`Result`], and Java's
/// `UncheckedIOException` of `Done()` too.
pub trait CharStream: std::fmt::Debug + Send + Sync {
    /// Returns the next character from the selected input. Can throw any
    /// `java.io.IOException`.
    ///
    /// Equivalent to `readChar()`.
    ///
    /// # Errors
    ///
    /// The `IOException` of the input, or of its end.
    fn read_char(&mut self) -> Result<u16>;

    /// Returns the column number of the last character of the current token
    /// (being matched after the last call to `BeginToken`).
    ///
    /// Equivalent to `getEndColumn()`.
    fn get_end_column(&self) -> i32;

    /// Returns the line number of the last character of the current token.
    ///
    /// Equivalent to `getEndLine()`.
    fn get_end_line(&self) -> i32;

    /// Returns the column number of the first character of the current token.
    ///
    /// Equivalent to `getBeginColumn()`.
    fn get_begin_column(&self) -> i32;

    /// Returns the line number of the first character of the current token.
    ///
    /// Equivalent to `getBeginLine()`.
    fn get_begin_line(&self) -> i32;

    /// Backs up the input stream by `amount` steps: the lexer had read some
    /// characters it could not use to match a longer token, so they are used
    /// again as the prefix of the next token.
    ///
    /// Equivalent to `backup(int)`.
    fn backup(&mut self, amount: i32);

    /// Returns the next character, which marks the beginning of the next
    /// token. All characters must remain in the buffer between two calls, to
    /// implement `backup` correctly.
    ///
    /// Equivalent to `BeginToken()`.
    ///
    /// # Errors
    ///
    /// As [`read_char`](Self::read_char).
    fn begin_token(&mut self) -> Result<u16>;

    /// Returns the string of characters from the marked token beginning to
    /// the current buffer position.
    ///
    /// Equivalent to `GetImage()`. **Divergence.** A Java `String` keeps an
    /// unpaired surrogate; a Rust `String` cannot, so each becomes U+FFFD (a
    /// Rust `&str` query text holds none).
    fn get_image(&self) -> String;

    /// Returns the characters of the suffix of length `len` of the currently
    /// matched token, which builds up the matched string for the actions of
    /// `MORE`.
    ///
    /// Equivalent to `GetSuffix(int)`.
    fn get_suffix(&self, len: i32) -> Vec<u16>;

    /// Indicates that the lexer is done with the stream, so that it can free
    /// its resources.
    ///
    /// Equivalent to `Done()`.
    ///
    /// # Errors
    ///
    /// An implementation's unchecked exception (`UncheckedIOException` for
    /// [`FastCharStream`](super::fast_char_stream::FastCharStream)).
    fn done(&mut self) -> Result<()>;
}
