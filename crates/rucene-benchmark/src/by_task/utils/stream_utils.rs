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

//! The types declared in `StreamUtils.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.utils`.

use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::path::Path;

use rucene::error::{LuceneError, Result};

/// Equivalent to `org.apache.lucene.benchmark.byTask.utils.StreamUtils`:
/// stream utilities.
///
/// **Divergence.** Lucene builds the GZIP and BZIP2 streams with Apache
/// commons-compress's `CompressorStreamFactory` (`StreamUtils.java:46-66`).
/// The port uses `flate2` (on the workspace's `zlib-rs` backend) and
/// `bzip2` 0.6.1 (user decision on rmp #611): decoding is equal both ways,
/// but the compressed bytes written may differ from commons-compress's. The
/// writers use commons-compress's defaults: GZIP level 6 with no file name,
/// modification time 0 and operating system 255; BZIP2 block size 9.
pub struct StreamUtils;

impl StreamUtils {
    /// `BUFFER_SIZE`: buffer size used across the benchmark package.
    pub const BUFFER_SIZE: usize = 1 << 16; // 64K

    /// `inputStream(Path file)`: an input stream over the requested file.
    /// This method attempts to identify the appropriate stream to return
    /// based on the file name (e.g., if it ends with .bz2 or .bzip, return a
    /// 'bzip' stream).
    ///
    /// # Errors
    ///
    /// The `IOException` of opening the file, or `IOException("Could not
    /// create CompressorInputStream.")` when a compressed file does not
    /// start with its format's header.
    pub fn input_stream(file: &Path) -> Result<Box<dyn Read + Send>> {
        // First, create a FileInputStream, as this will be required by all types.
        // Wrap with BufferedInputStream for better performance
        let input = BufReader::with_capacity(Self::BUFFER_SIZE, File::open(file)?);
        Self::file_type(file).input_stream(input)
    }

    /// `fileType(Path file)`: the type of the file, `PLAIN` if unknown.
    fn file_type(file: &Path) -> Type {
        let file_name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ty = file_name.rfind('.').and_then(|idx| {
            // these are in lower case, we will lower case at the test as well
            match file_name[idx..].to_lowercase().as_str() {
                ".bz2" | ".bzip" => Some(Type::Bzip2),
                ".gz" | ".gzip" => Some(Type::Gzip),
                _ => None,
            }
        });
        ty.unwrap_or(Type::Plain)
    }

    /// `outputStream(Path file)`: an output stream over the requested file,
    /// identifying the appropriate stream similar to
    /// [`input_stream`](Self::input_stream).
    ///
    /// # Errors
    ///
    /// The `IOException` of creating the file.
    pub fn output_stream(file: &Path) -> Result<OutputStream> {
        // First, create a FileInputStream, as this will be required by all types.
        // Wrap with BufferedInputStream for better performance
        let os = BufWriter::with_capacity(Self::BUFFER_SIZE, File::create(file)?);
        Ok(Self::file_type(file).output_stream(os))
    }
}

/// Equivalent to `org.apache.lucene.benchmark.byTask.utils.StreamUtils.Type`:
/// file format type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    /// `BZIP2`: automatically used for **.bz2** and **.bzip2** extensions.
    Bzip2,
    /// `GZIP`: automatically used for **.gz** and **.gzip** extensions.
    Gzip,
    /// `PLAIN`: plain text is used for anything which is not GZIP or BZIP.
    Plain,
}

/// The `IOException(e.getMessage(), e)` of a `CompressorException("Could
/// not create CompressorInputStream.", cause)`.
fn could_not_create(cause: &str) -> LuceneError {
    let _ = cause;
    LuceneError::from(io::Error::other("Could not create CompressorInputStream."))
}

impl Type {
    /// `inputStream(InputStream in)`: commons-compress's constructors read
    /// the stream header, so a missing header fails here.
    fn input_stream(self, mut input: BufReader<File>) -> Result<Box<dyn Read + Send>> {
        match self {
            Type::Plain => Ok(Box::new(input)),
            Type::Gzip => {
                let head = input.fill_buf()?;
                if head.len() < 2 || head[0] != 31 || head[1] != 139 {
                    return Err(could_not_create("Input is not in the .gz format"));
                }
                Ok(Box::new(flate2::bufread::GzDecoder::new(input)))
            }
            Type::Bzip2 => {
                // A file read through a fresh 64K buffer yields its first bytes
                // (all of them when shorter) in one fill.
                let magic = input.fill_buf()?.to_vec();
                if magic.len() < 3 || &magic[..3] != b"BZh" {
                    return Err(could_not_create("Stream is not in the BZip2 format"));
                }
                if magic.len() < 4 || !(b'1'..=b'9').contains(&magic[3]) {
                    return Err(could_not_create("BZip2 block size is invalid"));
                }
                Ok(Box::new(bzip2::bufread::BzDecoder::new(input)))
            }
        }
    }

    /// `outputStream(OutputStream os)`.
    fn output_stream(self, os: BufWriter<File>) -> OutputStream {
        OutputStream(match self {
            Type::Plain => Sink::Plain(os),
            Type::Gzip => Sink::Gzip(flate2::write::GzEncoder::new(
                os,
                flate2::Compression::new(6),
            )),
            Type::Bzip2 => {
                Sink::Bzip2(bzip2::write::BzEncoder::new(os, bzip2::Compression::best()))
            }
        })
    }
}

enum Sink {
    Plain(BufWriter<File>),
    Gzip(flate2::write::GzEncoder<BufWriter<File>>),
    Bzip2(bzip2::write::BzEncoder<BufWriter<File>>),
}

/// The `OutputStream` [`StreamUtils::output_stream`] returns: Java's
/// `close()` (which finishes the compressed stream and flushes the file) is
/// [`close`](Self::close); dropping without closing finishes it too,
/// ignoring errors.
pub struct OutputStream(Sink);

impl OutputStream {
    /// `close()`.
    ///
    /// # Errors
    ///
    /// The `IOException` of finishing or flushing.
    pub fn close(self) -> io::Result<()> {
        match self.0 {
            Sink::Plain(mut w) => w.flush(),
            Sink::Gzip(w) => w.finish()?.flush(),
            Sink::Bzip2(w) => w.finish()?.flush(),
        }
    }
}

impl Write for OutputStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &mut self.0 {
            Sink::Plain(w) => w.write(buf),
            Sink::Gzip(w) => w.write(buf),
            Sink::Bzip2(w) => w.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.0 {
            Sink::Plain(w) => w.flush(),
            Sink::Gzip(w) => w.flush(),
            Sink::Bzip2(w) => w.flush(),
        }
    }
}
