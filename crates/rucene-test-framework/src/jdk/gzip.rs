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

//! `java.util.zip.GZIPInputStream` over an in-memory stream, and the raw
//! DEFLATE decoder (`java.util.zip.Inflater` with `nowrap`) under it.
//!
//! `LineFileDocs` reads its line file through a `GZIPInputStream` started at
//! one of the file's seek points (each the first byte of a gzip member), and
//! `TestUtil.unzip` reads `ZipInputStream` entries; the JDK runs both on its
//! bundled zlib. The decoder is a translation of `puff.c` (Mark Adler's
//! reference inflater, zlib distribution `contrib/puff`, zlib license, which
//! permits this use), decoding RFC 1951 exactly; any conforming decoder yields
//! the same bytes, so the output is the JDK's.
//!
//! The stream follows `GZIPInputStream`'s observable contract (RFC 1952): the
//! header's magic, method and flag fields are checked (`ZipException` `Not in
//! GZIP format`, `Unsupported compression method`), the trailer's CRC-32 and
//! size are checked (`Corrupt GZIP trailer`), and a member followed by
//! another gzip header is followed by that member, as JDK 25 reads
//! concatenated members; trailing bytes that are not a header end the stream.

use std::borrow::Cow;
use std::io::{self, Read};

use rucene::store::BufferedChecksum;

/// The `ZipException` (an `IOException`) of `java.util.zip` with `message`.
fn zip_exception(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

/// The bit reader and output window of one raw DEFLATE stream (`puff.c`'s
/// `struct state`).
struct Inflate<'a> {
    input: &'a [u8],
    in_pos: usize,
    bit_buf: u32,
    bit_cnt: u32,
    out: Vec<u8>,
}

/// A canonical Huffman code: the number of codes of each length, and the
/// symbols ordered by code (`puff.c`'s `struct huffman`).
struct Huffman {
    count: [u16; 16],
    symbol: Vec<u16>,
}

/// The base lengths of length codes 257..285.
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
/// The extra bits of length codes 257..285.
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
/// The base distances of distance codes 0..29.
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
/// The extra bits of distance codes 0..29.
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
/// The order of the code-length code lengths of a dynamic block.
const CODE_LENGTH_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

impl Huffman {
    /// `construct`: the code of the code `lengths`; `Err` for an
    /// over-subscribed set, `Ok(left)` with the number of unused codes.
    fn new(lengths: &[u8]) -> (Huffman, i32) {
        let mut count = [0u16; 16];
        for &len in lengths {
            count[len as usize] += 1;
        }
        let mut h = Huffman {
            count,
            symbol: vec![0; lengths.len()],
        };
        if h.count[0] as usize == lengths.len() {
            return (h, 0);
        }
        let mut left: i32 = 1;
        for len in 1..16 {
            left <<= 1;
            left -= i32::from(h.count[len]);
            if left < 0 {
                return (h, left);
            }
        }
        let mut offs = [0u16; 16];
        for len in 1..15 {
            offs[len + 1] = offs[len] + h.count[len];
        }
        for (symbol, &len) in lengths.iter().enumerate() {
            if len != 0 {
                h.symbol[offs[len as usize] as usize] = symbol as u16;
                offs[len as usize] += 1;
            }
        }
        (h, left)
    }
}

impl<'a> Inflate<'a> {
    fn new(input: &'a [u8]) -> Self {
        Inflate {
            input,
            in_pos: 0,
            bit_buf: 0,
            bit_cnt: 0,
            out: Vec::new(),
        }
    }

    fn truncated() -> io::Error {
        io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Unexpected end of ZLIB input stream",
        )
    }

    fn data_error(message: &str) -> io::Error {
        zip_exception(message)
    }

    /// `bits(s, need)`.
    fn bits(&mut self, need: u32) -> io::Result<u32> {
        let mut val = self.bit_buf;
        while self.bit_cnt < need {
            let byte = *self.input.get(self.in_pos).ok_or_else(Self::truncated)?;
            self.in_pos += 1;
            val |= u32::from(byte) << self.bit_cnt;
            self.bit_cnt += 8;
        }
        self.bit_buf = val >> need;
        self.bit_cnt -= need;
        Ok(val & ((1u32 << need) - 1))
    }

    /// `stored(s)`.
    fn stored(&mut self) -> io::Result<()> {
        self.bit_buf = 0;
        self.bit_cnt = 0;
        let header = self
            .input
            .get(self.in_pos..self.in_pos + 4)
            .ok_or_else(Self::truncated)?;
        let len = u16::from_le_bytes([header[0], header[1]]);
        let nlen = u16::from_le_bytes([header[2], header[3]]);
        if len != !nlen {
            return Err(Self::data_error("invalid stored block lengths"));
        }
        self.in_pos += 4;
        let data = self
            .input
            .get(self.in_pos..self.in_pos + len as usize)
            .ok_or_else(Self::truncated)?;
        self.out.extend_from_slice(data);
        self.in_pos += len as usize;
        Ok(())
    }

    /// `decode(s, h)`: the next symbol of code `h`, bit by bit.
    fn decode(&mut self, h: &Huffman) -> io::Result<u16> {
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;
        for len in 1..16 {
            code |= self.bits(1)? as i32;
            let count = i32::from(h.count[len]);
            if code - count < first {
                return Ok(h.symbol[(index + (code - first)) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(Self::data_error("invalid code"))
    }

    /// `codes(s, lencode, distcode)`.
    fn codes(&mut self, lencode: &Huffman, distcode: &Huffman) -> io::Result<()> {
        loop {
            let symbol = self.decode(lencode)?;
            if symbol < 256 {
                self.out.push(symbol as u8);
            } else if symbol == 256 {
                return Ok(());
            } else {
                let symbol = (symbol - 257) as usize;
                if symbol >= 29 {
                    return Err(Self::data_error("invalid literal/length code"));
                }
                let len = LENGTH_BASE[symbol] as usize
                    + self.bits(u32::from(LENGTH_EXTRA[symbol]))? as usize;
                let dsym = self.decode(distcode)? as usize;
                if dsym >= 30 {
                    return Err(Self::data_error("invalid distance code"));
                }
                let dist =
                    DIST_BASE[dsym] as usize + self.bits(u32::from(DIST_EXTRA[dsym]))? as usize;
                if dist > self.out.len() {
                    return Err(Self::data_error("invalid distance too far back"));
                }
                let start = self.out.len() - dist;
                for k in 0..len {
                    let b = self.out[start + k];
                    self.out.push(b);
                }
            }
        }
    }

    /// `fixed(s)`.
    fn fixed(&mut self) -> io::Result<()> {
        let mut lengths = [0u8; 288];
        lengths[..144].fill(8);
        lengths[144..256].fill(9);
        lengths[256..280].fill(7);
        lengths[280..].fill(8);
        let (lencode, _) = Huffman::new(&lengths);
        let (distcode, _) = Huffman::new(&[5u8; 30]);
        self.codes(&lencode, &distcode)
    }

    /// `dynamic(s)`.
    fn dynamic(&mut self) -> io::Result<()> {
        let nlen = self.bits(5)? as usize + 257;
        let ndist = self.bits(5)? as usize + 1;
        let ncode = self.bits(4)? as usize + 4;
        if nlen > 286 || ndist > 30 {
            return Err(Self::data_error("too many length or distance symbols"));
        }
        let mut lengths = [0u8; 316];
        for &index in &CODE_LENGTH_ORDER[..ncode] {
            lengths[index] = self.bits(3)? as u8;
        }
        let (lencode, left) = Huffman::new(&lengths[..19]);
        if left != 0 {
            return Err(Self::data_error("invalid code lengths set"));
        }
        let mut index = 0;
        while index < nlen + ndist {
            let symbol = self.decode(&lencode)?;
            if symbol < 16 {
                lengths[index] = symbol as u8;
                index += 1;
            } else {
                let (len, repeat) = match symbol {
                    16 => {
                        if index == 0 {
                            return Err(Self::data_error("invalid bit length repeat"));
                        }
                        (lengths[index - 1], 3 + self.bits(2)? as usize)
                    }
                    17 => (0, 3 + self.bits(3)? as usize),
                    _ => (0, 11 + self.bits(7)? as usize),
                };
                if index + repeat > nlen + ndist {
                    return Err(Self::data_error("invalid bit length repeat"));
                }
                lengths[index..index + repeat].fill(len);
                index += repeat;
            }
        }
        if lengths[256] == 0 {
            return Err(Self::data_error("invalid code -- missing end-of-block"));
        }
        let (lencode, left) = Huffman::new(&lengths[..nlen]);
        if left < 0 || (left > 0 && nlen as i32 - i32::from(lencode.count[0]) != 1) {
            return Err(Self::data_error("invalid literal/lengths set"));
        }
        let (distcode, left) = Huffman::new(&lengths[nlen..nlen + ndist]);
        if left < 0 || (left > 0 && ndist as i32 - i32::from(distcode.count[0]) != 1) {
            return Err(Self::data_error("invalid distances set"));
        }
        self.codes(&lencode, &distcode)
    }

    /// `puff`: decodes the raw DEFLATE stream at the start of the input, and
    /// returns its output and the number of input bytes it used (the last
    /// partial byte included).
    fn run(mut self) -> io::Result<(Vec<u8>, usize)> {
        loop {
            let last = self.bits(1)?;
            match self.bits(2)? {
                0 => self.stored()?,
                1 => self.fixed()?,
                2 => self.dynamic()?,
                _ => return Err(Self::data_error("invalid block type")),
            }
            if last == 1 {
                return Ok((self.out, self.in_pos));
            }
        }
    }
}

/// Decodes the raw DEFLATE stream (`new Inflater(true)`) at the start of
/// `input`: its output, and the number of input bytes the stream occupies.
///
/// # Errors
///
/// A `ZipException` (`InvalidData`) for an invalid stream; `UnexpectedEof`
/// (`Unexpected end of ZLIB input stream`, `InflaterInputStream.fill`'s
/// `EOFException`) for a truncated one.
pub fn inflate_raw(input: &[u8]) -> io::Result<(Vec<u8>, usize)> {
    Inflate::new(input).run()
}

/// `java.util.zip.GZIPInputStream` over an input stream held in memory.
pub struct GzipInputStream<'a> {
    input: Cow<'a, [u8]>,
    /// The position of the next member's header.
    pos: usize,
    /// The decoded bytes of the current member, and the read position.
    member: Vec<u8>,
    member_pos: usize,
    eos: bool,
}

/// `GZIPInputStream`'s header flags.
const FHCRC: u8 = 2;
const FEXTRA: u8 = 4;
const FNAME: u8 = 8;
const FCOMMENT: u8 = 16;

impl<'a> GzipInputStream<'a> {
    /// `new GZIPInputStream(in)`: reads the first member's header.
    ///
    /// # Errors
    ///
    /// `Not in GZIP format`, `Unsupported compression method`, or an
    /// `EOFException` for a header cut short, as Java's constructor.
    pub fn new(input: impl Into<Cow<'a, [u8]>>) -> io::Result<Self> {
        Self::with_offset(input, 0)
    }

    /// `new GZIPInputStream(in)` over `input` after `in.skip(offset)`: the
    /// first member's header starts at `offset` (an `offset` past the end is
    /// the end of the stream, as `InputStream.skip`).
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_offset(input: impl Into<Cow<'a, [u8]>>, offset: usize) -> io::Result<Self> {
        let input = input.into();
        let mut stream = GzipInputStream {
            pos: offset.min(input.len()),
            input,
            member: Vec::new(),
            member_pos: 0,
            eos: false,
        };
        stream.read_member()?;
        Ok(stream)
    }

    /// `readHeader`: the length of the header at `pos`.
    fn read_header(&self, pos: usize) -> io::Result<usize> {
        let eof = || io::Error::new(io::ErrorKind::UnexpectedEof, "Unexpected end of input");
        let byte = |p: usize| self.input.get(p).copied().ok_or_else(eof);
        if byte(pos)? != 0x1f || byte(pos + 1)? != 0x8b {
            return Err(zip_exception("Not in GZIP format"));
        }
        if byte(pos + 2)? != 8 {
            return Err(zip_exception("Unsupported compression method"));
        }
        let flg = byte(pos + 3)?;
        // MTIME, XFL and OS.
        let mut p = pos + 10;
        byte(p - 1)?;
        if flg & FEXTRA == FEXTRA {
            let len = usize::from(u16::from_le_bytes([byte(p)?, byte(p + 1)?]));
            p += 2 + len;
            byte(p - 1)?;
        }
        for flag in [FNAME, FCOMMENT] {
            if flg & flag == flag {
                while byte(p)? != 0 {
                    p += 1;
                }
                p += 1;
            }
        }
        if flg & FHCRC == FHCRC {
            let mut crc = BufferedChecksum::new();
            for b in &self.input[pos..p] {
                crc.update(*b);
            }
            let v = u16::from_le_bytes([byte(p)?, byte(p + 1)?]);
            if v != (crc.get_value() & 0xffff) as u16 {
                return Err(zip_exception("Corrupt GZIP header"));
            }
            p += 2;
        }
        Ok(p - pos)
    }

    /// Decodes the member at `pos` and checks its trailer (`readTrailer`).
    fn read_member(&mut self) -> io::Result<()> {
        let header = self.read_header(self.pos)?;
        let start = self.pos + header;
        let (data, used) = inflate_raw(self.input.get(start..).unwrap_or_default())?;
        let trailer = self
            .input
            .get(start + used..start + used + 8)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Unexpected end of ZLIB input stream",
                )
            })?;
        let mut crc = BufferedChecksum::new();
        for b in &data {
            crc.update(*b);
        }
        let expected_crc = u32::from_le_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);
        let expected_size = u32::from_le_bytes([trailer[4], trailer[5], trailer[6], trailer[7]]);
        if expected_crc != crc.get_value() as u32 || expected_size != data.len() as u32 {
            return Err(zip_exception("Corrupt GZIP trailer"));
        }
        self.pos = start + used + 8;
        self.member = data;
        self.member_pos = 0;
        Ok(())
    }

    /// At the end of a member: reads the next one when a gzip header
    /// follows, as `readTrailer` does; `false` at the end of the stream.
    fn next_member(&mut self) -> io::Result<bool> {
        // Java tries a header when more than the trailer remains, and ends
        // the stream when that header is not one.
        if self.pos >= self.input.len() || self.read_header(self.pos).is_err() {
            self.eos = true;
            return Ok(false);
        }
        self.read_member()?;
        Ok(true)
    }
}

impl Read for GzipInputStream<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        while self.member_pos == self.member.len() {
            if self.eos || !self.next_member()? {
                return Ok(0);
            }
        }
        let n = buf.len().min(self.member.len() - self.member_pos);
        buf[..n].copy_from_slice(&self.member[self.member_pos..self.member_pos + n]);
        self.member_pos += n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two concatenated members of one stored block each decode to their
    /// content; a corrupted trailer and a foreign magic are refused with
    /// Java's messages. (Fixed and dynamic blocks: the `LineFileDocs` oracle
    /// decodes the whole `europarl.lines.txt.gz`.)
    #[test]
    fn decodes_members_and_checks_trailers() {
        // A member holding one stored block "abc".
        let stored: Vec<u8> = vec![
            0x1f, 0x8b, 0x08, 0x00, 0, 0, 0, 0, 0x00, 0xff, // header
            0x01, 0x03, 0x00, 0xfc, 0xff, b'a', b'b', b'c', // stored block
            0xc2, 0x41, 0x24, 0x35, 0x03, 0x00, 0x00, 0x00, // crc32("abc"), size
        ];
        let mut two = stored.clone();
        two.extend_from_slice(&stored);
        let mut out = String::new();
        GzipInputStream::new(&two[..])
            .unwrap()
            .read_to_string(&mut out)
            .unwrap();
        assert_eq!(out, "abcabc");
        let mut bad = stored.clone();
        bad[18] ^= 1;
        let err = GzipInputStream::new(&bad[..]).err().unwrap();
        assert_eq!(err.to_string(), "Corrupt GZIP trailer");
        assert_eq!(
            GzipInputStream::new(&b"PK\x03\x04xxxxxxxxxx"[..])
                .err()
                .unwrap()
                .to_string(),
            "Not in GZIP format"
        );
    }
}
