//! Port of `com.carrotsearch.randomizedtesting.WriterOutputStream`
//! (randomizedtesting 2.8.1, `WriterOutputStream.java`).

use std::io::{self, Write};

/// A character sink: `java.io.Writer`, writing UTF-16 code units.
pub trait CharWriter {
    /// `write(cbuf, 0, len)`.
    fn write_chars(&mut self, cbuf: &[u16]) -> io::Result<()>;
    /// `flush()`.
    fn flush_chars(&mut self) -> io::Result<()>;
    /// `close()`.
    fn close_chars(&mut self) -> io::Result<()>;
}

/// `java.io.StringWriter`: the code units written.
#[derive(Debug, Default, Clone)]
pub struct StringWriter {
    units: Vec<u16>,
    /// The size of each `write` call received, in order.
    pub writes: Vec<usize>,
}

impl StringWriter {
    /// `new StringWriter()`.
    pub fn new() -> StringWriter {
        StringWriter::default()
    }

    /// `toString()`.
    pub fn text(&self) -> String {
        String::from_utf16_lossy(&self.units)
    }
}

impl CharWriter for StringWriter {
    fn write_chars(&mut self, cbuf: &[u16]) -> io::Result<()> {
        self.units.extend_from_slice(cbuf);
        self.writes.push(cbuf.len());
        Ok(())
    }
    fn flush_chars(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn close_chars(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The charsets the stream decodes.
///
/// **Divergence.** Java accepts any `Charset` (`WriterOutputStream.java:121`); the port provides UTF-8,
/// ISO-8859-1 and US-ASCII, each replacing malformed input with `?` as the
/// Java constructors configure the decoder. Malformed UTF-8 is split into
/// replaced sequences as JDK 25's decoder splits it (Rust's
/// `str::from_utf8` boundaries, except that an encoded surrogate
/// `ED A0..BF xx` is one sequence).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charset {
    /// `UTF-8` (also `Charset.defaultCharset()` on JDK 18+).
    Utf8,
    /// `ISO-8859-1`.
    Iso8859_1,
    /// `US-ASCII`.
    UsAscii,
}

/// An `OutputStream` that decodes bytes into characters for a `Writer`
/// (`WriterOutputStream`): bytes collect in a 128-byte input buffer, decode
/// into a character buffer of `buffer_size` units, and reach the writer
/// when that buffer fills, on `flush`, on `close`, or after every write
/// when `write_immediately`.
pub struct WriterOutputStream<W: CharWriter> {
    writer: W,
    charset: Charset,
    write_immediately: bool,
    decoder_in: Vec<u8>,
    decoder_out: Vec<u16>,
    buffer_size: usize,
}

/// `DEFAULT_BUFFER_SIZE`.
const DEFAULT_BUFFER_SIZE: usize = 1024;
/// The input buffer's capacity (`ByteBuffer.allocate(128)`).
const IN_CAPACITY: usize = 128;

impl<W: CharWriter> WriterOutputStream<W> {
    /// `new WriterOutputStream(writer)`: UTF-8, 1024 units, not immediate.
    pub fn new(writer: W) -> WriterOutputStream<W> {
        WriterOutputStream::with_options(writer, Charset::Utf8, DEFAULT_BUFFER_SIZE, false)
    }

    /// `new WriterOutputStream(writer, charset, bufferSize, writeImmediately)`.
    pub fn with_options(
        writer: W,
        charset: Charset,
        buffer_size: usize,
        write_immediately: bool,
    ) -> WriterOutputStream<W> {
        WriterOutputStream {
            writer,
            charset,
            write_immediately,
            decoder_in: Vec::with_capacity(IN_CAPACITY),
            decoder_out: Vec::with_capacity(buffer_size),
            buffer_size: buffer_size.max(1),
        }
    }

    /// The writer.
    pub fn writer(&self) -> &W {
        &self.writer
    }

    /// `close()`: decodes the rest (a truncated sequence as `?`), flushes
    /// the characters and closes the writer.
    pub fn close(&mut self) -> io::Result<()> {
        self.process_input(true)?;
        self.flush_output()?;
        self.writer.close_chars()
    }

    fn push_char(&mut self, u: u16) -> io::Result<()> {
        if self.decoder_out.len() == self.buffer_size {
            self.flush_output()?;
        }
        self.decoder_out.push(u);
        Ok(())
    }

    fn push_scalar(&mut self, c: char) -> io::Result<()> {
        let mut buf = [0u16; 2];
        let units = c.encode_utf16(&mut buf);
        // A surrogate pair is decoded only when both units fit.
        if units.len() == 2 && self.buffer_size - self.decoder_out.len() < 2 {
            self.flush_output()?;
        }
        for &u in units.iter() {
            self.push_char(u)?;
        }
        Ok(())
    }

    /// `processInput(endOfInput)`: decodes what the input buffer holds.
    fn process_input(&mut self, end_of_input: bool) -> io::Result<()> {
        let input = std::mem::take(&mut self.decoder_in);
        let mut i = 0;
        while i < input.len() {
            match self.charset {
                Charset::Iso8859_1 => {
                    self.push_char(u16::from(input[i]))?;
                    i += 1;
                }
                Charset::UsAscii => {
                    let b = input[i];
                    self.push_char(if b < 0x80 {
                        u16::from(b)
                    } else {
                        u16::from(b'?')
                    })?;
                    i += 1;
                }
                Charset::Utf8 => match std::str::from_utf8(&input[i..]) {
                    Ok(s) => {
                        for c in s.chars() {
                            self.push_scalar(c)?;
                        }
                        i = input.len();
                    }
                    Err(e) => {
                        let valid = e.valid_up_to();
                        let s = std::str::from_utf8(&input[i..i + valid]).expect("valid prefix");
                        for c in s.chars() {
                            self.push_scalar(c)?;
                        }
                        i += valid;
                        // JDK's decoder reads ED A0..BF xx as a three-byte
                        // form whose value is a surrogate: one malformed
                        // sequence of the bytes read, where Rust reports
                        // each byte (observed against JDK 25).
                        if input[i] == 0xED
                            && input.get(i + 1).is_some_and(|b| (0xA0..=0xBF).contains(b))
                        {
                            match input.get(i + 2) {
                                Some(b) => {
                                    self.push_char(u16::from(b'?'))?;
                                    i += if (0x80..=0xBF).contains(b) { 3 } else { 2 };
                                }
                                None if end_of_input => {
                                    self.push_char(u16::from(b'?'))?;
                                    i = input.len();
                                }
                                None => {
                                    self.decoder_in.extend_from_slice(&input[i..]);
                                    return Ok(());
                                }
                            }
                            continue;
                        }
                        match e.error_len() {
                            Some(n) => {
                                self.push_char(u16::from(b'?'))?;
                                i += n;
                            }
                            None => {
                                // A truncated sequence: wait for more input,
                                // or replace it at the end of the input.
                                if end_of_input {
                                    self.push_char(u16::from(b'?'))?;
                                    i = input.len();
                                } else {
                                    self.decoder_in.extend_from_slice(&input[i..]);
                                    return Ok(());
                                }
                            }
                        }
                    }
                },
            }
        }
        Ok(())
    }

    /// `flushOutput()`.
    fn flush_output(&mut self) -> io::Result<()> {
        if !self.decoder_out.is_empty() {
            let out = std::mem::take(&mut self.decoder_out);
            self.writer.write_chars(&out)?;
        }
        Ok(())
    }
}

impl<W: CharWriter> Write for WriterOutputStream<W> {
    /// `write(b, off, len)`: fills the input buffer and decodes it, 128
    /// bytes at a time.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut off = 0;
        while off < buf.len() {
            let c = (buf.len() - off).min(IN_CAPACITY - self.decoder_in.len());
            self.decoder_in.extend_from_slice(&buf[off..off + c]);
            self.process_input(false)?;
            off += c;
        }
        if self.write_immediately {
            self.flush_output()?;
        }
        Ok(buf.len())
    }

    /// `flush()`: the decoded characters to the writer, then the writer.
    fn flush(&mut self) -> io::Result<()> {
        self.flush_output()?;
        self.writer.flush_chars()
    }
}
