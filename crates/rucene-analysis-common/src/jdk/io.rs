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

//! The `java.io` readers the module reads text with, as JDK 25 implements
//! them: `InputStreamReader` (`sun.nio.cs.StreamDecoder` over a
//! [`CharsetDecoder`], the `sun.nio.cs.UTF_8` decoder for UTF-8),
//! `BufferedReader.readLine()`, `mark` and `reset`, and `LineNumberReader`.
//!
//! These are emulations of the JDK, written from its specification and
//! checked against its behaviour (see the [`jdk`](crate::jdk) module
//! documentation): what the JDK 25 classes do is stated at each item.

use std::fmt::{self, Debug};
use std::io::Read;
use std::sync::Mutex;

use rucene::analysis::CharReader;
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::store::java_nio_file::JavaIoException;

use super::charset::{Charset, CharsetDecoder, CoderResult};

/// `new IOException(message)` of class `class`, as a [`LuceneError`].
pub(crate) fn io_exception(class: &'static str, message: Option<String>) -> LuceneError {
    LuceneError::from(LuceneErrorKind::Io(
        JavaIoException::new(class, message).into_io_error(),
    ))
}

/// `sun.nio.cs.StreamDecoder.DEFAULT_BYTE_BUFFER_SIZE`.
const DEFAULT_BYTE_BUFFER_SIZE: usize = 8192;

/// `java.io.BufferedReader.DEFAULT_CHAR_BUFFER_SIZE`.
const DEFAULT_CHAR_BUFFER_SIZE: usize = 8192;

/// `java.nio.charset.CodingErrorAction`, for malformed input and unmappable
/// characters (`IGNORE` is not used by the module).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodingErrorAction {
    /// `CodingErrorAction.REPORT`: the decoder throws
    /// `MalformedInputException`.
    Report,
    /// `CodingErrorAction.REPLACE`: the decoder writes U+FFFD and goes on.
    Replace,
}

/// `isNotContinuation(int)` of the JDK's UTF-8 decoder.
fn is_not_continuation(b: u8) -> bool {
    (b & 0xc0) != 0x80
}

/// Whether `b1 b2` cannot start a three-byte sequence: an overlong `E0`
/// form or a second byte that is not a continuation.
fn is_malformed3_2(b1: u8, b2: u8) -> bool {
    (b1 == 0xe0 && (b2 & 0xe0) == 0x80) || (b2 & 0xc0) != 0x80
}

/// Whether `b1 b2` cannot start a four-byte sequence.
fn is_malformed4_2(b1: u8, b2: u8) -> bool {
    (b1 == 0xf0 && !(0x90..=0xbf).contains(&b2))
        || (b1 == 0xf4 && (b2 & 0xf0) != 0x80)
        || (b2 & 0xc0) != 0x80
}

/// The length the JDK reports for a malformed sequence of `nb` bytes
/// starting at `src[sp]` (`malformedN`).
fn malformed_n(src: &[u8], sp: usize, nb: usize) -> usize {
    match nb {
        3 => {
            let (b1, b2) = (src[sp], src[sp + 1]);
            if (b1 == 0xe0 && (b2 & 0xe0) == 0x80) || is_not_continuation(b2) {
                1
            } else {
                2
            }
        }
        4 => {
            let (b1, b2) = (src[sp], src[sp + 1]);
            if b1 > 0xf4
                || (b1 == 0xf0 && !(0x90..=0xbf).contains(&b2))
                || (b1 == 0xf4 && (b2 & 0xf0) != 0x80)
                || is_not_continuation(b2)
            {
                1
            } else if is_not_continuation(src[sp + 2]) {
                2
            } else {
                3
            }
        }
        _ => 1,
    }
}

/// `xflow`: underflow when the input lacks the `nb` bytes of the sequence
/// (or `nb` is 0), overflow when the output lacks room.
fn xflow(remaining: usize, nb: usize) -> CoderResult {
    if nb == 0 || remaining < nb {
        CoderResult::Underflow
    } else {
        CoderResult::Overflow
    }
}

/// The decoding loop of JDK 25's UTF-8 decoder (`UTF_8.Decoder`): decodes
/// `src[*sp..]` into `dst[*dp..]` until the input is used up, the output is
/// full, or a malformed sequence starts at `*sp`.
pub(crate) fn utf8_decode_loop(
    src: &[u8],
    sp: &mut usize,
    dst: &mut [u16],
    dp: &mut usize,
) -> CoderResult {
    let sl = src.len();
    let dl = dst.len();
    while *sp < sl {
        let b1 = src[*sp];
        let remaining = sl - *sp;
        if b1 < 0x80 {
            // 1 byte, 7 bits: 0xxxxxxx
            if *dp >= dl {
                return xflow(remaining, 1);
            }
            dst[*dp] = u16::from(b1);
            *dp += 1;
            *sp += 1;
        } else if (0xc2..=0xdf).contains(&b1) {
            // 2 bytes, 11 bits: 110xxxxx 10xxxxxx
            if remaining < 2 || *dp >= dl {
                return xflow(remaining, 2);
            }
            let b2 = src[*sp + 1];
            if is_not_continuation(b2) {
                return CoderResult::Malformed(1);
            }
            dst[*dp] = (u16::from(b1 & 0x1f) << 6) | u16::from(b2 & 0x3f);
            *dp += 1;
            *sp += 2;
        } else if (0xe0..=0xef).contains(&b1) {
            // 3 bytes, 16 bits: 1110xxxx 10xxxxxx 10xxxxxx
            if remaining < 3 || *dp >= dl {
                if remaining > 1 && is_malformed3_2(b1, src[*sp + 1]) {
                    return CoderResult::Malformed(1);
                }
                return xflow(remaining, 3);
            }
            let (b2, b3) = (src[*sp + 1], src[*sp + 2]);
            if (b1 == 0xe0 && (b2 & 0xe0) == 0x80) || (b2 & 0xc0) != 0x80 || (b3 & 0xc0) != 0x80 {
                return CoderResult::Malformed(malformed_n(src, *sp, 3));
            }
            let c =
                (u16::from(b1 & 0x0f) << 12) | (u16::from(b2 & 0x3f) << 6) | u16::from(b3 & 0x3f);
            if (0xd800..=0xdfff).contains(&c) {
                return CoderResult::Malformed(3);
            }
            dst[*dp] = c;
            *dp += 1;
            *sp += 3;
        } else if (0xf0..=0xf7).contains(&b1) {
            // 4 bytes, 21 bits: 11110xxx 10xxxxxx 10xxxxxx 10xxxxxx
            if remaining < 4 || dl - *dp < 2 {
                if b1 > 0xf4 || (remaining > 1 && is_malformed4_2(b1, src[*sp + 1])) {
                    return CoderResult::Malformed(1);
                }
                if remaining > 2 && is_not_continuation(src[*sp + 2]) {
                    return CoderResult::Malformed(2);
                }
                return xflow(remaining, 4);
            }
            let (b2, b3, b4) = (src[*sp + 1], src[*sp + 2], src[*sp + 3]);
            let uc = (u32::from(b1 & 0x07) << 18)
                | (u32::from(b2 & 0x3f) << 12)
                | (u32::from(b3 & 0x3f) << 6)
                | u32::from(b4 & 0x3f);
            if is_not_continuation(b2)
                || is_not_continuation(b3)
                || is_not_continuation(b4)
                // shortest form check
                || !(0x10000..=0x10ffff).contains(&uc)
            {
                return CoderResult::Malformed(malformed_n(src, *sp, 4));
            }
            let v = uc - 0x10000;
            dst[*dp] = 0xd800 | (v >> 10) as u16;
            dst[*dp + 1] = 0xdc00 | (v & 0x3ff) as u16;
            *dp += 2;
            *sp += 4;
        } else {
            return CoderResult::Malformed(1);
        }
    }
    xflow(0, 0)
}

/// A `java.io.InputStreamReader`: the `sun.nio.cs.StreamDecoder` of JDK 25
/// over `input`, with an 8192-byte buffer and a [`CharsetDecoder`].
///
/// `InputStreamReader(InputStream, Charset)` decodes with
/// [`CodingErrorAction::Replace`] for malformed input and unmappable
/// characters; `InputStreamReader(InputStream, CharsetDecoder)` with the
/// decoder's actions.
///
/// **Divergence from JDK 25.** `StreamDecoder` stops filling the caller's
/// buffer early when it holds some units and the stream's `available()`
/// answers 0 ("block at most once", `inReady()`); `std::io::Read` has no
/// `available()`, so the port answers whether a byte remains by reading one
/// byte ahead (kept for the next read). That is what `available() > 0`
/// answers for the streams the module reads with `REPORT`, where the early
/// stop is observable (a malformed sequence after a complete line is
/// reported by the read after the one that returns the line): the
/// `FileInputStream`, `Files.newInputStream` and `ByteArrayInputStream` of
/// the resource loaders. `ZipInputStream.available()` answers 1 until a read
/// meets the entry's end, but the module reads it with `REPLACE`, which
/// decodes the same text however the reads are split.
pub struct InputStreamReader<R: Read> {
    input: Option<R>,
    decoder: CharsetDecoder,
    /// `bb`: `bytes[position..limit]` hold the bytes not yet decoded.
    bytes: Vec<u8>,
    position: usize,
    limit: usize,
    /// `haveLeftoverChar` and `leftoverChar`.
    leftover: Option<u16>,
    /// The byte `in_ready` read ahead, not yet in `bytes`.
    peeked: Option<u8>,
}

impl<R: Read> Debug for InputStreamReader<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InputStreamReader")
            .field("decoder", &self.decoder)
            .field("pending_bytes", &(self.limit - self.position))
            .field("closed", &self.input.is_none())
            .finish()
    }
}

impl<R: Read> InputStreamReader<R> {
    /// Equivalent to `new InputStreamReader(input, StandardCharsets.UTF_8)`,
    /// which replaces malformed input with U+FFFD.
    pub fn new_utf8(input: R) -> Self {
        Self::with_charset(input, Charset::utf_8())
    }

    /// Equivalent to `new InputStreamReader(input, decoder)` for a UTF-8
    /// `CharsetDecoder` whose malformed-input action is `action`.
    pub fn with_action(input: R, action: CodingErrorAction) -> Self {
        let decoder = Charset::utf_8()
            .new_decoder()
            .expect("INVARIANT: UTF-8 is emulated")
            .on_malformed_input(action);
        Self::with_decoder(input, decoder)
    }

    /// Equivalent to `new InputStreamReader(input, charset)`: a decoder of
    /// `charset` that replaces malformed input and unmappable characters.
    ///
    pub fn with_charset(input: R, charset: Charset) -> Self {
        let decoder = charset
            .new_decoder()
            .expect("INVARIANT: every JDK 25 charset has a decoder")
            .on_malformed_input(CodingErrorAction::Replace)
            .on_unmappable_character(CodingErrorAction::Replace);
        Self::with_decoder(input, decoder)
    }

    /// Equivalent to `new InputStreamReader(input, decoder)`.
    pub fn with_decoder(input: R, decoder: CharsetDecoder) -> Self {
        Self {
            input: Some(input),
            decoder,
            bytes: vec![0; DEFAULT_BYTE_BUFFER_SIZE],
            position: 0,
            limit: 0,
            leftover: None,
            peeked: None,
        }
    }

    /// `StreamDecoder.inReady()`, `in.available() > 0`: whether a byte
    /// remains, read ahead into `peeked` (see the type's documentation); an
    /// I/O error answers `false`, as Java's `IOException` does.
    fn in_ready(&mut self) -> bool {
        if self.peeked.is_some() {
            return true;
        }
        let Some(input) = self.input.as_mut() else {
            return false;
        };
        let mut b = [0u8; 1];
        match input.read(&mut b) {
            Ok(1) => {
                self.peeked = Some(b[0]);
                true
            }
            _ => false,
        }
    }

    /// `ensureOpen()`.
    fn ensure_open(&self) -> Result<()> {
        if self.input.is_none() {
            return Err(io_exception(
                "java.io.IOException",
                Some("Stream closed".to_string()),
            ));
        }
        Ok(())
    }

    /// `CharsetDecoder.decode(ByteBuffer, CharBuffer, boolean)` for this
    /// reader's action.
    fn decode(&mut self, cbuf: &mut [u16], cp: &mut usize, end_of_input: bool) -> CoderResult {
        loop {
            let mut sp = self.position;
            let mut cr = self
                .decoder
                .decode_loop(&self.bytes[..self.limit], &mut sp, cbuf, cp);
            self.position = sp;
            if cr == CoderResult::Underflow {
                if end_of_input && self.position < self.limit {
                    cr = CoderResult::Malformed(self.limit - self.position);
                } else {
                    return cr;
                }
            }
            let (action, length) = match cr {
                CoderResult::Overflow => return cr,
                CoderResult::Malformed(length) => (self.decoder.malformed_input_action(), length),
                CoderResult::Unmappable(length) => {
                    (self.decoder.unmappable_character_action(), length)
                }
                CoderResult::Underflow => unreachable!("handled above"),
            };
            match action {
                CodingErrorAction::Report => return cr,
                CodingErrorAction::Replace => {
                    if *cp >= cbuf.len() {
                        return CoderResult::Overflow;
                    }
                    cbuf[*cp] = 0xfffd;
                    *cp += 1;
                    self.position += length;
                }
            }
        }
    }

    /// `StreamDecoder.readBytes()`: compacts the buffer and reads into its
    /// free space; `None` at the end of the stream.
    fn read_bytes(&mut self) -> Result<Option<usize>> {
        self.bytes.copy_within(self.position..self.limit, 0);
        let pos = self.limit - self.position;
        self.position = 0;
        self.limit = pos;
        let input = self.input.as_mut().expect("INVARIANT: ensure_open checked");
        let n = match self.peeked.take() {
            Some(b) => {
                self.bytes[pos] = b;
                1 + input.read(&mut self.bytes[pos + 1..])?
            }
            None => input.read(&mut self.bytes[pos..])?,
        };
        if n == 0 {
            return Ok(None);
        }
        self.limit = pos + n;
        Ok(Some(self.limit))
    }

    /// `StreamDecoder.implRead(char[], int, int)`; `None` at the end of the
    /// stream (Java's `-1`). `cbuf` holds at least two units.
    fn impl_read(&mut self, cbuf: &mut [u16]) -> Result<Option<usize>> {
        debug_assert!(cbuf.len() > 1);
        let mut cp = 0;
        let mut eof = false;
        loop {
            let cr = self.decode(cbuf, &mut cp, eof);
            match cr {
                CoderResult::Underflow => {
                    if eof || cp >= cbuf.len() {
                        break;
                    }
                    if cp > 0 && !self.in_ready() {
                        break; // Block at most once
                    }
                    if self.read_bytes()?.is_none() {
                        eof = true;
                        if cp == 0 && self.position >= self.limit {
                            break;
                        }
                    }
                }
                CoderResult::Overflow => break,
                CoderResult::Malformed(length) => {
                    // `cr.throwException()`
                    return Err(io_exception(
                        "java.nio.charset.MalformedInputException",
                        Some(format!("Input length = {length}")),
                    ));
                }
                CoderResult::Unmappable(length) => {
                    // `cr.throwException()`
                    return Err(io_exception(
                        "java.nio.charset.UnmappableCharacterException",
                        Some(format!("Input length = {length}")),
                    ));
                }
            }
        }
        if eof {
            // ## Need to flush decoder
            self.decoder.reset();
        }
        if cp == 0 && eof {
            return Ok(None);
        }
        Ok(Some(cp))
    }

    /// `StreamDecoder.read0()`: one unit, through a two-unit buffer whose
    /// second unit is kept as the leftover.
    fn read0(&mut self) -> Result<Option<u16>> {
        if let Some(c) = self.leftover.take() {
            return Ok(Some(c));
        }
        let mut cb = [0u16; 2];
        match self.impl_read(&mut cb)? {
            None => Ok(None),
            Some(n) => {
                if n == 2 {
                    self.leftover = Some(cb[1]);
                }
                Ok(Some(cb[0]))
            }
        }
    }

    /// Reads units into `cbuf`, returning how many; 0 at the end of the
    /// stream (Java's `-1`).
    ///
    /// Equivalent to `InputStreamReader.read(char[], int, int)`,
    /// `StreamDecoder.read`.
    ///
    /// # Errors
    ///
    /// `IOException("Stream closed")` after [`close`](Self::close_stream),
    /// `MalformedInputException("Input length = <n>")` for malformed input
    /// under [`CodingErrorAction::Report`], and what the stream raises.
    pub fn read_units(&mut self, cbuf: &mut [u16]) -> Result<usize> {
        self.ensure_open()?;
        if cbuf.is_empty() {
            return Ok(0);
        }
        let mut off = 0;
        let mut n = 0;
        if let Some(c) = self.leftover.take() {
            cbuf[0] = c;
            off = 1;
            n = 1;
            if cbuf.len() == 1 {
                return Ok(n);
            }
        }
        if cbuf.len() - off == 1 {
            return match self.read0()? {
                None => Ok(n),
                Some(c) => {
                    cbuf[off] = c;
                    Ok(n + 1)
                }
            };
        }
        match self.impl_read(&mut cbuf[off..])? {
            None => Ok(n),
            Some(nr) => Ok(n + nr),
        }
    }

    /// Equivalent to `InputStreamReader.close()`: drops the stream; closing
    /// twice does nothing.
    pub fn close_stream(&mut self) -> Result<()> {
        self.input = None;
        Ok(())
    }
}

/// A `Box<dyn Read + Send>` behind a lock, so that a reader over it is
/// `Sync` as a [`CharReader`] must be.
///
/// Not a JDK class: Java's `InputStream` has no thread-safety marker, while
/// `rucene`'s `CharReader` must be `Send + Sync`; the lock is never
/// contended, as a reader is read by one thread.
pub struct SyncInputStream {
    inner: Mutex<Box<dyn Read + Send>>,
}

impl SyncInputStream {
    /// Wraps `input`.
    pub fn new(input: Box<dyn Read + Send>) -> Self {
        Self {
            inner: Mutex::new(input),
        }
    }
}

impl Debug for SyncInputStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SyncInputStream")
    }
}

impl Read for SyncInputStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .read(buf)
    }
}

impl<R: Read + Debug + Send + Sync> CharReader for InputStreamReader<R> {
    fn read(&mut self, buf: &mut [u16]) -> Result<usize> {
        self.read_units(buf)
    }

    fn close(&mut self) -> Result<()> {
        self.close_stream()
    }
}

/// The source a [`BufferedReader`] reads: Java's `Reader`.
pub trait ReadChars {
    /// `Reader.read(char[], int, int)`: 0 at the end of the stream.
    ///
    /// # Errors
    ///
    /// What the reader raises.
    fn read_chars(&mut self, cbuf: &mut [u16]) -> Result<usize>;

    /// `Reader.close()`.
    ///
    /// # Errors
    ///
    /// What the reader raises.
    fn close_chars(&mut self) -> Result<()>;
}

impl ReadChars for dyn CharReader + '_ {
    fn read_chars(&mut self, cbuf: &mut [u16]) -> Result<usize> {
        self.read(cbuf)
    }

    fn close_chars(&mut self) -> Result<()> {
        self.close()
    }
}

impl<R: Read> ReadChars for InputStreamReader<R> {
    fn read_chars(&mut self, cbuf: &mut [u16]) -> Result<usize> {
        self.read_units(cbuf)
    }

    fn close_chars(&mut self) -> Result<()> {
        self.close_stream()
    }
}

impl<T: ReadChars + ?Sized> ReadChars for &mut T {
    fn read_chars(&mut self, cbuf: &mut [u16]) -> Result<usize> {
        (**self).read_chars(cbuf)
    }

    fn close_chars(&mut self) -> Result<()> {
        (**self).close_chars()
    }
}

/// `java.io.BufferedReader` with its default 8192-unit buffer, for
/// `readLine()`: a line ends at a line feed, a carriage return, or a
/// carriage return followed by a line feed, and is returned as its UTF-16
/// units without the terminator.
pub struct BufferedReader<S: ReadChars> {
    input: Option<S>,
    cb: Vec<u16>,
    n_chars: usize,
    next_char: usize,
    /// If the next character is a line feed, skip it.
    skip_lf: bool,
    /// `markedChar`: the marked position, or [`UNMARKED`] or
    /// [`INVALIDATED`].
    marked_char: i64,
    /// `readAheadLimit`, valid only when marked.
    read_ahead_limit: usize,
    /// `markedSkipLF`: `skip_lf` at the mark.
    marked_skip_lf: bool,
}

/// `BufferedReader.INVALIDATED`.
const INVALIDATED: i64 = -2;

/// `BufferedReader.UNMARKED`.
const UNMARKED: i64 = -1;

impl<S: ReadChars> Debug for BufferedReader<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BufferedReader")
            .field("n_chars", &self.n_chars)
            .field("next_char", &self.next_char)
            .field("skip_lf", &self.skip_lf)
            .finish_non_exhaustive()
    }
}

impl<S: ReadChars> BufferedReader<S> {
    /// Equivalent to `new BufferedReader(in)`.
    pub fn new(input: S) -> Self {
        Self {
            input: Some(input),
            cb: vec![0; DEFAULT_CHAR_BUFFER_SIZE],
            n_chars: 0,
            next_char: 0,
            skip_lf: false,
            marked_char: UNMARKED,
            read_ahead_limit: 0,
            marked_skip_lf: false,
        }
    }

    /// `ensureOpen()`.
    fn ensure_open(&self) -> Result<()> {
        if self.input.is_none() {
            return Err(io_exception(
                "java.io.IOException",
                Some("Stream closed".to_string()),
            ));
        }
        Ok(())
    }

    /// `fill()`: reads into the buffer, keeping the marked characters
    /// within the read-ahead limit.
    fn fill(&mut self) -> Result<()> {
        let dst;
        if self.marked_char <= UNMARKED {
            // No mark
            dst = 0;
        } else {
            // Marked
            let marked = self.marked_char as usize;
            let delta = self.next_char - marked;
            if delta >= self.read_ahead_limit {
                // Gone past read-ahead limit: Invalidate mark
                self.marked_char = INVALIDATED;
                self.read_ahead_limit = 0;
                dst = 0;
            } else {
                if self.read_ahead_limit <= self.cb.len() {
                    // Shuffle in the current buffer
                    self.cb.copy_within(marked..marked + delta, 0);
                } else {
                    // Reallocate buffer to accommodate read-ahead limit
                    let mut ncb = vec![0u16; self.read_ahead_limit];
                    ncb[..delta].copy_from_slice(&self.cb[marked..marked + delta]);
                    self.cb = ncb;
                }
                self.marked_char = 0;
                dst = delta;
                self.next_char = delta;
                self.n_chars = delta;
            }
        }
        let input = self.input.as_mut().expect("INVARIANT: ensure_open checked");
        let n = input.read_chars(&mut self.cb[dst..])?;
        if n > 0 {
            self.n_chars = dst + n;
            self.next_char = dst;
        }
        Ok(())
    }

    /// Marks the present position: [`reset`](Self::reset) returns to it
    /// while at most `read_ahead_limit` characters were read since.
    ///
    /// Equivalent to `BufferedReader.mark(int)`.
    ///
    /// # Errors
    ///
    /// `IOException("Stream closed")` after [`close`](Self::close).
    pub fn mark(&mut self, read_ahead_limit: usize) -> Result<()> {
        self.ensure_open()?;
        self.read_ahead_limit = read_ahead_limit;
        self.marked_char = self.next_char as i64;
        self.marked_skip_lf = self.skip_lf;
        Ok(())
    }

    /// Returns to the most recent mark.
    ///
    /// Equivalent to `BufferedReader.reset()`.
    ///
    /// # Errors
    ///
    /// `IOException("Stream closed")` after [`close`](Self::close), and
    /// `IOException("Mark invalid")` or `IOException("Stream not marked")`
    /// when there is no valid mark.
    pub fn reset(&mut self) -> Result<()> {
        self.ensure_open()?;
        if self.marked_char < 0 {
            return Err(io_exception(
                "java.io.IOException",
                Some(
                    if self.marked_char == INVALIDATED {
                        "Mark invalid"
                    } else {
                        "Stream not marked"
                    }
                    .to_string(),
                ),
            ));
        }
        self.next_char = self.marked_char as usize;
        self.skip_lf = self.marked_skip_lf;
        Ok(())
    }

    /// Equivalent to the package-private `readLine(boolean ignoreLF,
    /// boolean[] term)`: `term`, when given, is set to whether the line
    /// ended with a terminator.
    ///
    /// # Errors
    ///
    /// `IOException("Stream closed")` after [`close`](Self::close), and what
    /// the reader raises.
    pub fn read_line_with(
        &mut self,
        ignore_lf: bool,
        mut term: Option<&mut bool>,
    ) -> Result<Option<Vec<u16>>> {
        let mut s: Option<Vec<u16>> = None;

        self.ensure_open()?;
        let mut omit_lf = ignore_lf || self.skip_lf;
        if let Some(term) = term.as_deref_mut() {
            *term = false;
        }

        loop {
            if self.next_char >= self.n_chars {
                self.fill()?;
            }
            if self.next_char >= self.n_chars {
                // EOF
                return Ok(match s {
                    Some(s) if !s.is_empty() => Some(s),
                    _ => None,
                });
            }
            let mut eol = false;
            let mut c = 0u16;

            // Skip a leftover '\n', if necessary
            if omit_lf && self.cb[self.next_char] == u16::from(b'\n') {
                self.next_char += 1;
            }
            self.skip_lf = false;
            omit_lf = false;

            let mut i = self.next_char;
            while i < self.n_chars {
                c = self.cb[i];
                if c == u16::from(b'\n') || c == u16::from(b'\r') {
                    if let Some(term) = term.as_deref_mut() {
                        *term = true;
                    }
                    eol = true;
                    break;
                }
                i += 1;
            }

            let start_char = self.next_char;
            self.next_char = i;

            if eol {
                let line = match s {
                    None => self.cb[start_char..i].to_vec(),
                    Some(mut s) => {
                        s.extend_from_slice(&self.cb[start_char..i]);
                        s
                    }
                };
                self.next_char += 1;
                if c == u16::from(b'\r') {
                    self.skip_lf = true;
                }
                return Ok(Some(line));
            }

            s.get_or_insert_with(Vec::new)
                .extend_from_slice(&self.cb[start_char..i]);
        }
    }

    /// Reads a line of text, `None` at the end of the stream.
    ///
    /// Equivalent to `BufferedReader.readLine()`, `readLine(false, null)`.
    ///
    /// # Errors
    ///
    /// As [`read_line_with`](Self::read_line_with).
    pub fn read_line(&mut self) -> Result<Option<Vec<u16>>> {
        self.read_line_with(false, None)
    }

    /// Equivalent to `BufferedReader.close()`: closes the reader once.
    ///
    /// # Errors
    ///
    /// What closing the reader raises.
    pub fn close(&mut self) -> Result<()> {
        match self.input.take() {
            None => Ok(()),
            Some(mut input) => input.close_chars(),
        }
    }
}

/// `LineNumberReader.prevChar`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrevChar {
    /// no previous character
    None,
    /// non-line terminator
    Char,
    /// line terminator
    Eol,
    /// end-of-file
    Eof,
}

/// `java.io.LineNumberReader` of JDK 25, for `readLine()` and
/// `getLineNumber()`.
pub struct LineNumberReader<S: ReadChars> {
    inner: BufferedReader<S>,
    prev_char: PrevChar,
    line_number: i32,
    skip_lf: bool,
    /// `markedLineNumber`.
    marked_line_number: i32,
    /// `markedSkipLF`.
    marked_skip_lf: bool,
}

impl<S: ReadChars> Debug for LineNumberReader<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LineNumberReader")
            .field("line_number", &self.line_number)
            .field("prev_char", &self.prev_char)
            .finish_non_exhaustive()
    }
}

impl<S: ReadChars> LineNumberReader<S> {
    /// Equivalent to `new LineNumberReader(in)`.
    pub fn new(input: S) -> Self {
        Self {
            inner: BufferedReader::new(input),
            prev_char: PrevChar::None,
            line_number: 0,
            skip_lf: false,
            marked_line_number: 0,
            marked_skip_lf: false,
        }
    }

    /// Equivalent to `getLineNumber()`.
    pub fn get_line_number(&self) -> i32 {
        self.line_number
    }

    /// Equivalent to `setLineNumber(int)`.
    pub fn set_line_number(&mut self, line_number: i32) {
        self.line_number = line_number;
    }

    /// Reads a line of text, `None` at the end of the stream, counting it.
    ///
    /// Equivalent to `LineNumberReader.readLine()`.
    ///
    /// # Errors
    ///
    /// As [`BufferedReader::read_line_with`].
    pub fn read_line(&mut self) -> Result<Option<Vec<u16>>> {
        let mut term = false;
        let l = self.inner.read_line_with(self.skip_lf, Some(&mut term))?;
        self.skip_lf = false;
        if l.is_some() {
            self.line_number += 1;
            self.prev_char = if term { PrevChar::Eol } else { PrevChar::Eof };
        } else {
            if self.prev_char == PrevChar::Char {
                self.line_number += 1;
            }
            self.prev_char = PrevChar::Eof;
        }
        Ok(l)
    }

    /// Marks the present position and line number.
    ///
    /// Equivalent to `LineNumberReader.mark(int)`: the limit grows by one
    /// when the last character read was a carriage return.
    ///
    /// # Errors
    ///
    /// As [`BufferedReader::mark`].
    pub fn mark(&mut self, read_ahead_limit: usize) -> Result<()> {
        let mut read_ahead_limit = read_ahead_limit;
        // If the most recently read character is '\r', then increment the
        // read ahead limit as in this case if the next character is '\n',
        // two characters would actually be read by the next read().
        if self.skip_lf {
            read_ahead_limit += 1;
        }
        self.inner.mark(read_ahead_limit)?;
        self.marked_line_number = self.line_number;
        self.marked_skip_lf = self.skip_lf;
        Ok(())
    }

    /// Returns to the most recent mark and its line number.
    ///
    /// Equivalent to `LineNumberReader.reset()`.
    ///
    /// # Errors
    ///
    /// As [`BufferedReader::reset`].
    pub fn reset(&mut self) -> Result<()> {
        self.inner.reset()?;
        self.line_number = self.marked_line_number;
        self.skip_lf = self.marked_skip_lf;
        Ok(())
    }

    /// Equivalent to `LineNumberReader.close()`, the inherited
    /// `BufferedReader.close()`.
    ///
    /// # Errors
    ///
    /// What closing the reader raises.
    pub fn close(&mut self) -> Result<()> {
        self.inner.close()
    }
}
