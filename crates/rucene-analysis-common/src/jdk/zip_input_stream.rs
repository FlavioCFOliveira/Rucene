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

//! `java.util.zip.ZipInputStream` as JDK 25 implements it, reading the
//! entries of a ZIP stream one after the other (see the
//! [`jdk`](crate::jdk) module documentation).
//!
//! The pieces the JDK stacks are reproduced: the `PushbackInputStream` of
//! 512 bytes over the stream, the `InflaterInputStream` with its 512-byte
//! input buffer and a raw (`nowrap`) `Inflater`, here `flate2`'s
//! `Decompress` without a zlib header, and the entry `CRC32`.
//!
//! **Divergence from JDK 25.** The JDK's `Inflater` is zlib, whose
//! `DataFormatException` messages (`invalid block type`, ...) become the
//! `ZipException` message; the messages here are those `flate2`'s backend
//! (`zlib-rs`, a port of zlib) reports, and `Invalid ZLIB data format`
//! when it reports none, as the JDK does for a `null` message.

use std::fmt::{self, Debug};
use std::io::Read;

use flate2::{Crc, Decompress, FlushDecompress, Status};
use rucene::error::{LuceneError, LuceneErrorKind, Result};

use super::io::io_exception;

/// `ZipConstants.LOCSIG`, "PK\003\004".
const LOCSIG: u64 = 0x0403_4b50;
/// `ZipConstants.EXTSIG`, "PK\007\008".
const EXTSIG: u64 = 0x0807_4b50;
/// `ZipConstants.LOCHDR`.
const LOCHDR: usize = 30;
/// `ZipConstants.EXTHDR`.
const EXTHDR: usize = 16;
/// `ZipConstants.LOCFLG`.
const LOCFLG: usize = 6;
/// `ZipConstants.LOCHOW`.
const LOCHOW: usize = 8;
/// `ZipConstants.LOCCRC`.
const LOCCRC: usize = 14;
/// `ZipConstants.LOCSIZ`.
const LOCSIZ: usize = 18;
/// `ZipConstants.LOCLEN`.
const LOCLEN: usize = 22;
/// `ZipConstants.LOCNAM`.
const LOCNAM: usize = 26;
/// `ZipConstants.LOCEXT`.
const LOCEXT: usize = 28;
/// `ZipConstants.EXTCRC`.
const EXTCRC: usize = 4;
/// `ZipConstants.EXTSIZ`.
const EXTSIZ: usize = 8;
/// `ZipConstants.EXTLEN`.
const EXTLEN: usize = 12;
/// `ZipConstants.CENHDR`.
const CENHDR: usize = 46;
/// `ZipConstants64.ZIP64_EXTHDR`.
const ZIP64_EXTHDR: usize = 24;
/// `ZipConstants64.ZIP64_EXTCRC`.
const ZIP64_EXTCRC: usize = 4;
/// `ZipConstants64.ZIP64_EXTSIZ`.
const ZIP64_EXTSIZ: usize = 8;
/// `ZipConstants64.ZIP64_EXTLEN`.
const ZIP64_EXTLEN: usize = 16;
/// `ZipConstants64.ZIP64_MAGICVAL`.
const ZIP64_MAGICVAL: i64 = 0xFFFF_FFFF;
/// `ZipConstants64.EXTID_ZIP64`.
const EXTID_ZIP64: usize = 0x0001;
/// `ZipConstants64.USE_UTF8`.
#[allow(dead_code)] // A member of the Java class that Lucene 10.5.0's main code does not use.
const USE_UTF8: usize = 0x800;
/// `ZipEntry.MAX_COMBINED_CEN_HEADER_SIZE`.
const MAX_COMBINED_CEN_HEADER_SIZE: usize = 0xFFFF;
/// `ZipEntry.STORED`.
pub const STORED: i32 = 0;
/// `ZipEntry.DEFLATED`.
pub const DEFLATED: i32 = 8;

/// `ZipUtils.get16`: the little-endian unsigned 16-bit value at `off`.
fn get16(b: &[u8], off: usize) -> usize {
    usize::from(b[off]) | (usize::from(b[off + 1]) << 8)
}

/// `ZipUtils.get32`: the little-endian unsigned 32-bit value at `off`.
fn get32(b: &[u8], off: usize) -> u64 {
    (get16(b, off) as u64) | ((get16(b, off + 2) as u64) << 16)
}

/// `ZipUtils.get64S`: the little-endian signed 64-bit value at `off`.
fn get64s(b: &[u8], off: usize) -> i64 {
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&b[off..off + 8]);
    i64::from_le_bytes(bytes)
}

/// `new ZipException(message)`.
fn zip_exception(message: impl Into<String>) -> LuceneError {
    io_exception("java.util.zip.ZipException", Some(message.into()))
}

/// The entry of a ZIP stream: `java.util.zip.ZipEntry`, the fields
/// `ZipInputStream` reads and checks.
#[derive(Debug, Clone)]
pub struct ZipEntry {
    name: String,
    method: i32,
    crc: i64,
    size: i64,
    csize: i64,
}

impl ZipEntry {
    /// Equivalent to `ZipEntry(String)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("entry name too long")` when the name and
    /// a central header exceed 65,535 bytes.
    fn new(name: String) -> Result<Self> {
        if CENHDR + name.encode_utf16().count() > MAX_COMBINED_CEN_HEADER_SIZE {
            return Err(LuceneError::illegal_argument("entry name too long"));
        }
        Ok(Self {
            name,
            method: -1,
            crc: -1,
            size: -1,
            csize: -1,
        })
    }

    /// Equivalent to `getName()`.
    pub fn get_name(&self) -> &str {
        &self.name
    }

    /// `setExtra0(byte[], boolean, boolean)` for a local header: the ZIP64
    /// sizes, when `do_zip64`; the time fields Java also reads are not kept.
    fn set_extra0(&mut self, extra: &[u8], do_zip64: bool) -> Result<()> {
        if CENHDR + self.name.encode_utf16().count() + extra.len() > MAX_COMBINED_CEN_HEADER_SIZE {
            return Err(LuceneError::illegal_argument("invalid extra field length"));
        }
        // extra fields are in "HeaderID(2)DataSize(2)Data... format
        let mut off = 0;
        let len = extra.len();
        while off + 4 < len {
            let tag = get16(extra, off);
            let sz = get16(extra, off + 2);
            off += 4;
            if off + sz > len {
                // invalid data
                break;
            }
            // LOC extra zip64 entry MUST include BOTH original and
            // compressed file size fields. If invalid zip64 extra fields,
            // simply skip.
            if tag == EXTID_ZIP64 && do_zip64 && sz >= 16 {
                self.size = get64s(extra, off);
                self.csize = get64s(extra, off + 8);
            }
            off += sz;
        }
        Ok(())
    }
}

/// `java.io.PushbackInputStream(in, 512)`.
struct PushbackInputStream<R: Read> {
    input: R,
    buf: [u8; 512],
    pos: usize,
}

impl<R: Read> PushbackInputStream<R> {
    fn new(input: R) -> Self {
        Self {
            input,
            buf: [0; 512],
            pos: 512,
        }
    }

    /// `read(byte[], int, int)`: the pushed-back bytes, then what the
    /// stream gives; `None` at the end of both.
    fn read(&mut self, b: &mut [u8]) -> Result<Option<usize>> {
        if b.is_empty() {
            return Ok(Some(0));
        }
        let mut avail = self.buf.len() - self.pos;
        let mut off = 0;
        if avail > 0 {
            avail = avail.min(b.len());
            b[..avail].copy_from_slice(&self.buf[self.pos..self.pos + avail]);
            self.pos += avail;
            off = avail;
        }
        if off < b.len() {
            let len = self.input.read(&mut b[off..])?;
            if len == 0 {
                return Ok(if avail == 0 { None } else { Some(avail) });
            }
            return Ok(Some(avail + len));
        }
        Ok(Some(avail))
    }

    /// `unread(byte[], int, int)`.
    fn unread(&mut self, b: &[u8]) -> Result<()> {
        if b.len() > self.pos {
            return Err(io_exception(
                "java.io.IOException",
                Some("Push back buffer is full".to_string()),
            ));
        }
        self.pos -= b.len();
        self.buf[self.pos..self.pos + b.len()].copy_from_slice(b);
        Ok(())
    }
}

/// An input stream for reading files in the ZIP file format.
///
/// Equivalent to `java.util.zip.ZipInputStream(InputStream)`, whose entry
/// names are decoded as UTF-8.
pub struct ZipInputStream<R: Read> {
    input: PushbackInputStream<R>,
    /// `inf`: a raw (`nowrap`) inflater.
    inf: Decompress,
    /// `Inflater.getBytesRead()` and `getBytesWritten()` since the last reset.
    inf_bytes_read: i64,
    inf_bytes_written: i64,
    /// `Inflater.finished()`.
    inf_finished: bool,
    /// `Inflater.hasPendingOutput()`: the last inflate filled its buffer.
    inf_pending_output: bool,
    /// `InflaterInputStream.buf` and `len`, the inflater's input, of which
    /// `buf[inf_input_pos..len]` is not consumed yet.
    buf: [u8; 512],
    len: usize,
    inf_input_pos: usize,
    entry: Option<ZipEntry>,
    flag: usize,
    crc: Crc,
    remaining: i64,
    tmpbuf: [u8; 512],
    closed: bool,
    entry_eof: bool,
    expect_64bit_data_descriptor: bool,
}

impl<R: Read> Debug for ZipInputStream<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ZipInputStream")
            .field("entry", &self.entry)
            .field("closed", &self.closed)
            .field("entry_eof", &self.entry_eof)
            .finish_non_exhaustive()
    }
}

impl<R: Read> ZipInputStream<R> {
    /// Creates a new ZIP input stream.
    ///
    /// Equivalent to `ZipInputStream(InputStream)`.
    pub fn new(input: R) -> Self {
        Self {
            input: PushbackInputStream::new(input),
            inf: Decompress::new(false),
            inf_bytes_read: 0,
            inf_bytes_written: 0,
            inf_finished: false,
            inf_pending_output: false,
            buf: [0; 512],
            len: 0,
            inf_input_pos: 0,
            entry: None,
            flag: 0,
            crc: Crc::new(),
            remaining: 0,
            tmpbuf: [0; 512],
            closed: false,
            entry_eof: false,
            expect_64bit_data_descriptor: false,
        }
    }

    /// `ensureOpen()`.
    fn ensure_open(&self) -> Result<()> {
        if self.closed {
            return Err(io_exception(
                "java.io.IOException",
                Some("Stream closed".to_string()),
            ));
        }
        Ok(())
    }

    /// `Inflater.reset()`.
    fn inf_reset(&mut self) {
        self.inf.reset(false);
        self.inf_bytes_read = 0;
        self.inf_bytes_written = 0;
        self.inf_finished = false;
        self.inf_pending_output = false;
        self.inf_input_pos = self.len;
    }

    /// Reads the next ZIP file entry and positions the stream at the
    /// beginning of the entry data; `None` when there are no more entries.
    ///
    /// Equivalent to `getNextEntry()`.
    ///
    /// # Errors
    ///
    /// `ZipException` for a malformed entry header, `IOException` for a
    /// closed stream or a read failure, and `IllegalArgumentException` for
    /// an entry whose name and extra field are too long.
    pub fn get_next_entry(&mut self) -> Result<Option<ZipEntry>> {
        self.ensure_open()?;
        if self.entry.is_some() {
            self.close_entry()?;
        }
        self.crc.reset();
        self.inf_reset();
        self.entry = self.read_loc()?;
        let Some(entry) = &self.entry else {
            return Ok(None);
        };
        if entry.method == STORED {
            self.remaining = entry.size;
        }
        self.entry_eof = false;
        Ok(self.entry.clone())
    }

    /// Closes the current ZIP entry and positions the stream for reading
    /// the next entry.
    ///
    /// Equivalent to `closeEntry()`.
    ///
    /// # Errors
    ///
    /// What reading the rest of the entry raises.
    pub fn close_entry(&mut self) -> Result<()> {
        self.ensure_open()?;
        let mut tmpbuf = self.tmpbuf;
        while self.read_entry(&mut tmpbuf)?.is_some() {}
        self.entry_eof = true;
        Ok(())
    }

    /// Equivalent to `available()`: 0 after the end of the entry, 1 before.
    ///
    /// # Errors
    ///
    /// `IOException("Stream closed")`.
    pub fn available(&self) -> Result<i32> {
        self.ensure_open()?;
        Ok(if self.entry_eof { 0 } else { 1 })
    }

    /// `InflaterInputStream.fill()`.
    fn fill(&mut self) -> Result<()> {
        self.ensure_open()?;
        match self.input.read(&mut self.buf)? {
            None => Err(io_exception(
                "java.io.EOFException",
                Some("Unexpected end of ZLIB input stream".to_string()),
            )),
            Some(len) => {
                self.len = len;
                self.inf_input_pos = 0;
                Ok(())
            }
        }
    }

    /// `Inflater.inflate(byte[], int, int)`.
    fn inflate(&mut self, b: &mut [u8]) -> Result<usize> {
        let before_in = self.inf.total_in();
        let before_out = self.inf.total_out();
        let status = self
            .inf
            .decompress(
                &self.buf[self.inf_input_pos..self.len],
                b,
                FlushDecompress::None,
            )
            .map_err(|e| {
                // `throw new ZipException(s != null ? s : "Invalid ZLIB data format")`
                zip_exception(e.message().unwrap_or("Invalid ZLIB data format"))
            })?;
        let consumed = (self.inf.total_in() - before_in) as usize;
        let written = (self.inf.total_out() - before_out) as usize;
        self.inf_input_pos += consumed;
        self.inf_bytes_read += consumed as i64;
        self.inf_bytes_written += written as i64;
        if status == Status::StreamEnd {
            self.inf_finished = true;
        }
        self.inf_pending_output = !self.inf_finished && written == b.len();
        Ok(written)
    }

    /// `InflaterInputStream.read(byte[], int, int)`: `None` at the end of
    /// the deflated data.
    fn inflater_read(&mut self, b: &mut [u8]) -> Result<Option<usize>> {
        self.ensure_open()?;
        if b.is_empty() {
            return Ok(Some(0));
        }
        loop {
            if self.inf_finished {
                return Ok(None);
            }
            if self.inf_input_pos >= self.len && !self.inf_pending_output {
                self.fill()?;
            }
            let n = self.inflate(b)?;
            if n != 0 {
                return Ok(Some(n));
            }
        }
    }

    /// Reads from the current ZIP entry into `b`; `None` at the end of the
    /// entry.
    ///
    /// Equivalent to `read(byte[], int, int)`, whose `-1` is `None`.
    fn read_entry(&mut self, b: &mut [u8]) -> Result<Option<usize>> {
        self.ensure_open()?;
        if b.is_empty() {
            return Ok(Some(0));
        }
        let (method, entry_crc) = match &self.entry {
            None => return Ok(None),
            Some(entry) => (entry.method, entry.crc),
        };
        match method {
            DEFLATED => match self.inflater_read(b)? {
                None => {
                    let mut entry = self.entry.take().expect("INVARIANT: checked above");
                    if let Err(e) = self.read_end(&mut entry) {
                        self.entry = Some(entry);
                        return Err(e);
                    }
                    self.entry_eof = true;
                    Ok(None)
                }
                Some(len) => {
                    self.crc.update(&b[..len]);
                    Ok(Some(len))
                }
            },
            STORED => {
                if self.remaining <= 0 {
                    self.entry_eof = true;
                    self.entry = None;
                    return Ok(None);
                }
                let len = (b.len() as i64).min(self.remaining) as usize;
                let Some(len) = self.input.read(&mut b[..len])? else {
                    return Err(zip_exception("unexpected EOF"));
                };
                self.crc.update(&b[..len]);
                self.remaining -= len as i64;
                if self.remaining == 0 && entry_crc != i64::from(self.crc.sum()) {
                    return Err(zip_exception(format!(
                        "invalid entry CRC (expected 0x{:x} but got 0x{:x})",
                        entry_crc,
                        self.crc.sum()
                    )));
                }
                Ok(Some(len))
            }
            _ => Err(zip_exception("invalid compression method")),
        }
    }

    /// Closes this input stream and releases any system resources
    /// associated with the stream.
    ///
    /// Equivalent to `close()`; the stream is dropped with this reader.
    pub fn close(&mut self) {
        self.closed = true;
    }

    /// `readFully(byte[], int, int)`: `EOFException` at a premature end.
    fn read_fully(&mut self, b: &mut [u8]) -> Result<()> {
        let mut off = 0;
        while off < b.len() {
            match self.input.read(&mut b[off..])? {
                None => return Err(io_exception("java.io.EOFException", None)),
                Some(n) => off += n,
            }
        }
        Ok(())
    }

    /// `readLOC()`: the next local file header, `None` at the end of the
    /// stream or at a record that is not one.
    fn read_loc(&mut self) -> Result<Option<ZipEntry>> {
        let mut tmpbuf = self.tmpbuf;
        match self.read_fully(&mut tmpbuf[..LOCHDR]) {
            Ok(()) => {}
            Err(e) if e.java_simple_name() == "EOFException" => return Ok(None),
            Err(e) => return Err(e),
        }
        self.tmpbuf = tmpbuf;
        if get32(&tmpbuf, 0) != LOCSIG {
            return Ok(None);
        }
        self.flag = get16(&tmpbuf, LOCFLG);
        let len = get16(&tmpbuf, LOCNAM);
        let mut b = vec![0u8; len];
        self.read_fully(&mut b)?;
        // `UTF8.toString` and the UTF-8 `zc.toString` both refuse malformed input.
        let entry_name = String::from_utf8(b)
            .map_err(|_| zip_exception("invalid LOC header (bad entry name)"))?;
        let mut e = ZipEntry::new(entry_name)?;
        if (self.flag & 1) == 1 {
            return Err(zip_exception("encrypted ZIP entry not supported"));
        }
        e.method = get16(&tmpbuf, LOCHOW) as i32;
        self.expect_64bit_data_descriptor = false;
        let csize = get32(&tmpbuf, LOCSIZ) as i64;
        let size = get32(&tmpbuf, LOCLEN) as i64;
        if (self.flag & 8) == 8 {
            // "Data Descriptor" present
            if e.method != DEFLATED {
                return Err(zip_exception(
                    "only DEFLATED entries can have EXT descriptor",
                ));
            }
        } else {
            e.crc = get32(&tmpbuf, LOCCRC) as i64;
            e.csize = csize;
            e.size = size;
        }
        let len = get16(&tmpbuf, LOCEXT);
        if len > 0 {
            let mut extra = vec![0u8; len];
            self.read_fully(&mut extra)?;
            e.set_extra0(
                &extra,
                e.csize == ZIP64_MAGICVAL || e.size == ZIP64_MAGICVAL,
            )?;
            self.expect_64bit_data_descriptor =
                Self::expect_64bit_data_descriptor(&extra, self.flag, csize, size);
        }
        Ok(Some(e))
    }

    /// `readEnd(ZipEntry)`: pushes back the bytes the inflater did not use,
    /// reads the data descriptor, and checks the sizes and the CRC.
    fn read_end(&mut self, e: &mut ZipEntry) -> Result<()> {
        let n = self.len - self.inf_input_pos;
        if n > 0 {
            let buf = self.buf;
            self.input.unread(&buf[self.len - n..self.len])?;
            self.inf_input_pos = self.len;
        }
        let mut tmpbuf = self.tmpbuf;
        if (self.flag & 8) == 8 {
            // "Data Descriptor" present
            if self.inf_bytes_written > ZIP64_MAGICVAL
                || self.inf_bytes_read > ZIP64_MAGICVAL
                || self.expect_64bit_data_descriptor
            {
                self.read_fully(&mut tmpbuf[..ZIP64_EXTHDR])?;
                let sig = get32(&tmpbuf, 0);
                if sig != EXTSIG {
                    // no EXTSIG present
                    e.crc = sig as i64;
                    e.csize = get64s(&tmpbuf, ZIP64_EXTSIZ - ZIP64_EXTCRC);
                    e.size = get64s(&tmpbuf, ZIP64_EXTLEN - ZIP64_EXTCRC);
                    self.input
                        .unread(&tmpbuf[ZIP64_EXTHDR - ZIP64_EXTCRC..ZIP64_EXTHDR])?;
                } else {
                    e.crc = get32(&tmpbuf, ZIP64_EXTCRC) as i64;
                    e.csize = get64s(&tmpbuf, ZIP64_EXTSIZ);
                    e.size = get64s(&tmpbuf, ZIP64_EXTLEN);
                }
            } else {
                self.read_fully(&mut tmpbuf[..EXTHDR])?;
                let sig = get32(&tmpbuf, 0);
                if sig != EXTSIG {
                    // no EXTSIG present
                    e.crc = sig as i64;
                    e.csize = get32(&tmpbuf, EXTSIZ - EXTCRC) as i64;
                    e.size = get32(&tmpbuf, EXTLEN - EXTCRC) as i64;
                    self.input.unread(&tmpbuf[EXTHDR - EXTCRC..EXTHDR])?;
                } else {
                    e.crc = get32(&tmpbuf, EXTCRC) as i64;
                    e.csize = get32(&tmpbuf, EXTSIZ) as i64;
                    e.size = get32(&tmpbuf, EXTLEN) as i64;
                }
            }
        }
        self.tmpbuf = tmpbuf;
        if e.size != self.inf_bytes_written {
            return Err(zip_exception(format!(
                "invalid entry size (expected {} but got {} bytes)",
                e.size, self.inf_bytes_written
            )));
        }
        if e.csize != self.inf_bytes_read {
            return Err(zip_exception(format!(
                "invalid entry compressed size (expected {} but got {} bytes)",
                e.csize, self.inf_bytes_read
            )));
        }
        if e.crc != i64::from(self.crc.sum()) {
            return Err(zip_exception(format!(
                "invalid entry CRC (expected 0x{:x} but got 0x{:x})",
                e.crc,
                self.crc.sum()
            )));
        }
        Ok(())
    }

    /// `expect64BitDataDescriptor(byte[], int, long, long)`: whether the
    /// data descriptor of an entry with ZIP64 magic sizes and a ZIP64 extra
    /// field has 8-byte sizes.
    fn expect_64bit_data_descriptor(extra: &[u8], flag: usize, csize: i64, size: i64) -> bool {
        if (flag & 8) == 0 {
            return false;
        }
        if csize != ZIP64_MAGICVAL && size != ZIP64_MAGICVAL {
            return false;
        }
        let header_size = 2 * 2; // id + size
        let mut i = 0;
        while i + header_size < extra.len() {
            let id = get16(extra, i);
            let dsize = get16(extra, i + 2);
            if i + header_size + dsize > extra.len() {
                return false; // Invalid size
            }
            if id == EXTID_ZIP64 {
                return true;
            }
            i += header_size + dsize;
        }
        false
    }
}

impl<R: Read> Read for ZipInputStream<R> {
    /// The current entry's data, as Java's `read(byte[], int, int)`: `Ok(0)`
    /// at the end of the entry (Java's `-1`), and the Java exceptions as
    /// `std::io::Error`s that carry their class.
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.read_entry(buf) {
            Ok(n) => Ok(n.unwrap_or(0)),
            Err(e) => Err(match e.into_kind() {
                LuceneErrorKind::Io(error) => error,
                other => std::io::Error::other(LuceneError::from(other)),
            }),
        }
    }
}
