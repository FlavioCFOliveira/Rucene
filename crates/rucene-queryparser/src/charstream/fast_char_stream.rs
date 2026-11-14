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

//! The port of `FastCharStream.java`, of the Java package
//! `org.apache.lucene.queryparser.charstream`.

use rucene::analysis::CharReader;
use rucene::error::{LuceneError, LuceneErrorKind, Result};

use super::char_stream::CharStream;

/// An efficient implementation of JavaCC's [`CharStream`] interface.
///
/// Note that this does not do line-number counting, but instead keeps track
/// of the character position of the token in the input, as required by
/// Lucene's `OffsetAttribute` API.
///
/// Equivalent to `org.apache.lucene.queryparser.charstream.FastCharStream`.
#[derive(Debug)]
pub struct FastCharStream {
    /// `buffer`; `None` for Java's `null` before the first refill.
    buffer: Option<Vec<u16>>,
    /// End of valid chars.
    buffer_length: i32,
    /// Next char to read.
    buffer_position: i32,
    /// Offset in buffer.
    token_start: i32,
    /// Position in file of buffer.
    buffer_start: i32,
    /// Source of chars.
    input: Box<dyn CharReader>,
}

/// The message of `READ_PAST_EOF`, the `IOException` thrown at the end of the
/// input (SOLR-11314: one shared instance, without a stack trace).
const READ_PAST_EOF: &str = "Read past EOF.";

impl FastCharStream {
    /// Constructs from a reader.
    ///
    /// Equivalent to `new FastCharStream(Reader)`.
    pub fn new(r: Box<dyn CharReader>) -> Self {
        Self {
            buffer: None,
            buffer_length: 0,
            buffer_position: 0,
            token_start: 0,
            buffer_start: 0,
            input: r,
        }
    }

    /// `refill()`.
    fn refill(&mut self) -> Result<()> {
        let new_position = self.buffer_length - self.token_start;
        if self.token_start == 0 {
            // token won't fit in buffer
            match &mut self.buffer {
                None => {
                    // first time: alloc buffer
                    self.buffer = Some(vec![0u16; 2048]);
                }
                Some(buffer) => {
                    if self.buffer_length as usize == buffer.len() {
                        // grow buffer
                        let mut new_buffer = vec![0u16; buffer.len() * 2];
                        new_buffer[..self.buffer_length as usize]
                            .copy_from_slice(&buffer[..self.buffer_length as usize]);
                        *buffer = new_buffer;
                    }
                }
            }
        } else {
            // shift token to front
            let start = self.token_start as usize;
            let buffer = self.buffer.as_mut().ok_or_else(LuceneError::null_pointer)?;
            buffer.copy_within(start..start + new_position as usize, 0);
        }
        self.buffer_length = new_position; // update state
        self.buffer_position = new_position;
        self.buffer_start += self.token_start;
        self.token_start = 0;
        let buffer = self.buffer.as_mut().ok_or_else(LuceneError::null_pointer)?;
        let window = &mut buffer[new_position as usize..];
        // fill space in buffer; `Reader.read` answers -1 at the end, where
        // `CharReader.read` answers 0 (the window is never empty)
        let chars_read = self.input.read(window)?;
        if chars_read == 0 {
            return Err(LuceneError::from(LuceneErrorKind::Io(
                std::io::Error::other(READ_PAST_EOF),
            )));
        }
        self.buffer_length += chars_read as i32;
        Ok(())
    }
}

impl CharStream for FastCharStream {
    fn read_char(&mut self) -> Result<u16> {
        if self.buffer_position >= self.buffer_length {
            self.refill()?;
        }
        let c = self.buffer.as_ref().ok_or_else(LuceneError::null_pointer)?
            [self.buffer_position as usize];
        self.buffer_position += 1;
        Ok(c)
    }

    fn begin_token(&mut self) -> Result<u16> {
        self.token_start = self.buffer_position;
        self.read_char()
    }

    fn backup(&mut self, amount: i32) {
        self.buffer_position -= amount;
    }

    fn get_image(&self) -> String {
        match &self.buffer {
            Some(buffer) => String::from_utf16_lossy(
                &buffer[self.token_start as usize..self.buffer_position as usize],
            ),
            None => String::new(),
        }
    }

    fn get_suffix(&self, len: i32) -> Vec<u16> {
        let buffer = self.buffer.as_deref().unwrap_or(&[]);
        let end = self.buffer_position as usize;
        buffer[end - len as usize..end].to_vec()
    }

    /// Closes the reader; Java wraps its `IOException` in
    /// `UncheckedIOException`.
    fn done(&mut self) -> Result<()> {
        self.input
            .close()
            .map_err(rucene::error::LuceneError::unchecked_io_exception)
    }

    fn get_end_column(&self) -> i32 {
        self.buffer_start + self.buffer_position
    }

    fn get_end_line(&self) -> i32 {
        1
    }

    fn get_begin_column(&self) -> i32 {
        self.buffer_start + self.token_start
    }

    fn get_begin_line(&self) -> i32 {
        1
    }
}
