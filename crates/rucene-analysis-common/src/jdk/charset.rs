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

//! `java.nio.charset.Charset.encode(CharBuffer)` for the charsets of
//! `java.nio.charset.StandardCharsets`, as JDK 25 encodes them.
//!
//! `Charset.encode` runs a fresh encoder of the charset with
//! `CodingErrorAction.REPLACE` for malformed input and unmappable
//! characters: each malformed or unmappable sequence becomes the encoder's
//! replacement bytes. From the JDK 25 sources, read as specification
//! (`java/nio/charset/Charset.java`, `sun/nio/cs/UnicodeEncoder.java`,
//! `Charset-X-Coder.java.template`):
//!
//! * a malformed sequence is an unpaired surrogate: a low surrogate, or a high
//!   surrogate not followed by a low one (one unit long), including a high
//!   surrogate at the end of the input;
//! * an unmappable character of `US-ASCII` (above U+007F) or `ISO-8859-1`
//!   (above U+00FF) is one character, a surrogate pair being one character;
//! * the replacement is `?` (`0x3F`), the `CharsetEncoder` default, but for
//!   the UTF-16 encoders, which replace with U+FFFD in their byte order
//!   (`FF FD` big-endian, `FD FF` little-endian);
//! * `UTF-16` writes the big-endian byte order mark `FE FF` before the first
//!   character, and nothing for an empty input (`UnicodeEncoder.encodeLoop`
//!   writes it only when input remains).

/// A charset of `java.nio.charset.StandardCharsets`, the charsets every Java
/// platform supports.
///
/// **Divergence.** Java's `Charset` stands for any charset of the platform;
/// this emulation covers the six standard ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StandardCharset {
    /// `StandardCharsets.US_ASCII`.
    UsAscii,
    /// `StandardCharsets.ISO_8859_1`.
    Iso8859_1,
    /// `StandardCharsets.UTF_8`.
    Utf8,
    /// `StandardCharsets.UTF_16BE`.
    Utf16Be,
    /// `StandardCharsets.UTF_16LE`.
    Utf16Le,
    /// `StandardCharsets.UTF_16`: big-endian, with a byte order mark.
    Utf16,
}

/// One step of the encoder's input: a character, or a malformed unit.
enum Unit {
    /// A code point, a surrogate pair decoded.
    Char(u32),
    /// An unpaired surrogate.
    Malformed,
}

/// The characters and malformed units of `units`, in order.
fn units_of(units: &[u16]) -> Vec<Unit> {
    let mut out = Vec::with_capacity(units.len());
    let mut i = 0;
    while i < units.len() {
        let unit = units[i];
        if (0xD800..0xDC00).contains(&unit) {
            if i + 1 < units.len() && (0xDC00..0xE000).contains(&units[i + 1]) {
                let code_point = 0x10000
                    + ((u32::from(unit) - 0xD800) << 10)
                    + (u32::from(units[i + 1]) - 0xDC00);
                out.push(Unit::Char(code_point));
                i += 2;
                continue;
            }
            out.push(Unit::Malformed);
        } else if (0xDC00..0xE000).contains(&unit) {
            out.push(Unit::Malformed);
        } else {
            out.push(Unit::Char(u32::from(unit)));
        }
        i += 1;
    }
    out
}

impl StandardCharset {
    /// The canonical name, `Charset.name()`.
    pub fn name(self) -> &'static str {
        match self {
            Self::UsAscii => "US-ASCII",
            Self::Iso8859_1 => "ISO-8859-1",
            Self::Utf8 => "UTF-8",
            Self::Utf16Be => "UTF-16BE",
            Self::Utf16Le => "UTF-16LE",
            Self::Utf16 => "UTF-16",
        }
    }

    /// Encodes `units`, UTF-16 units, into bytes in this charset.
    ///
    /// Equivalent to `Charset.encode(CharBuffer.wrap(units))`; the bytes are
    /// the remaining ones of the returned `ByteBuffer`.
    pub fn encode(self, units: &[u16]) -> Vec<u8> {
        let mut out = Vec::with_capacity(units.len() * 2);
        match self {
            Self::UsAscii | Self::Iso8859_1 => {
                let max = if self == Self::UsAscii { 0x7F } else { 0xFF };
                for unit in units_of(units) {
                    match unit {
                        Unit::Char(c) if c <= max => out.push(c as u8),
                        _ => out.push(b'?'),
                    }
                }
            }
            Self::Utf8 => {
                let mut buf = [0u8; 4];
                for unit in units_of(units) {
                    match unit {
                        Unit::Char(c) => {
                            let c = char::from_u32(c).expect("INVARIANT: not a surrogate");
                            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                        }
                        Unit::Malformed => out.push(b'?'),
                    }
                }
            }
            Self::Utf16Be | Self::Utf16Le | Self::Utf16 => {
                let big = self != Self::Utf16Le;
                let put = |out: &mut Vec<u8>, unit: u16| {
                    if big {
                        out.extend_from_slice(&unit.to_be_bytes());
                    } else {
                        out.extend_from_slice(&unit.to_le_bytes());
                    }
                };
                if self == Self::Utf16 && !units.is_empty() {
                    put(&mut out, 0xFEFF);
                }
                let mut buf = [0u16; 2];
                for unit in units_of(units) {
                    match unit {
                        Unit::Char(c) => {
                            let c = char::from_u32(c).expect("INVARIANT: not a surrogate");
                            for &u in c.encode_utf16(&mut buf).iter() {
                                put(&mut out, u);
                            }
                        }
                        Unit::Malformed => put(&mut out, 0xFFFD),
                    }
                }
            }
        }
        out
    }
}
