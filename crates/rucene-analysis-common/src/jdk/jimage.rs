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

//! A reader of one resource of a JDK run-time image (`lib/modules`, the
//! "jimage" format), written from JDK 25's `jdk.internal.jimage`
//! (`ImageHeader`, `BasicImageReader`, `ImageLocation`, `ImageStringsReader`,
//! `decompressor.Decompressor`, `CompressedResourceHeader`,
//! `ZipDecompressor`) read as a specification. No JDK code or data is
//! copied; the reader opens the image of a JDK installed on the machine.
//!
//! The format, as implemented here:
//!
//! * a header of seven 32-bit integers in the image's byte order (the order
//!   of the platform that built it; the magic `0xCAFEDADA` tells which):
//!   magic, version (`major << 16 | minor`, 1.0), flags, resource count,
//!   table length, locations size, strings size;
//! * then the index: the redirect table and the offsets table (table length
//!   32-bit integers each), the location attributes and the strings (both
//!   byte arrays); the resources follow the index;
//! * a resource is found by its full name (`/module/parent/base.extension`):
//!   `redirect[hash(name) % length]` is 0 (absent), negative (`-index - 1`
//!   is the location index) or a seed (`hash(name, seed) % length` is the
//!   index); the hash is FNV-like (`seed * 0x01000193 ^ byte`) over the
//!   name's modified UTF-8, masked to 31 bits;
//! * `offsets[index]` points at the location's attributes: a byte
//!   `kind << 3 | (length - 1)` then `length` big-endian bytes, until a byte
//!   `<= 7`; the kinds are module, parent, base and extension (offsets of
//!   NUL-terminated modified UTF-8 strings), content offset (from the end of
//!   the index), compressed size and uncompressed size;
//! * the name is verified against the strings the attributes name (the hash
//!   table answers some index for any name);
//! * a compressed resource (compressed size `!= 0`) starts with headers of
//!   29 bytes (magic `0xCAFEFAFA`, compressed size, uncompressed size, the
//!   offset of the decompressor's name in the strings, a terminal flag),
//!   each undone in turn; the `zip` decompressor (zlib) is implemented, any
//!   other plugin (`compact-cp`, for class files) is an error.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// `ImageHeader.MAGIC`.
const MAGIC: u32 = 0xCAFE_DADA;
/// `ImageHeader.MAJOR_VERSION` and `MINOR_VERSION`.
const MAJOR_VERSION: u32 = 1;
const MINOR_VERSION: u32 = 0;
/// `ImageHeader.getHeaderSize()`: seven 32-bit slots.
const HEADER_SIZE: usize = 7 * 4;
/// `ImageStringsReader.HASH_MULTIPLIER` and `POSITIVE_MASK`.
const HASH_MULTIPLIER: i32 = 0x0100_0193;
const POSITIVE_MASK: i32 = 0x7FFF_FFFF;
/// The `ImageLocation` attribute kinds.
const ATTRIBUTE_MODULE: usize = 1;
const ATTRIBUTE_PARENT: usize = 2;
const ATTRIBUTE_BASE: usize = 3;
const ATTRIBUTE_EXTENSION: usize = 4;
const ATTRIBUTE_OFFSET: usize = 5;
const ATTRIBUTE_COMPRESSED: usize = 6;
const ATTRIBUTE_UNCOMPRESSED: usize = 7;
const ATTRIBUTE_COUNT: usize = 8;
/// `CompressedResourceHeader.MAGIC` and `getSize()`.
const COMPRESSED_MAGIC: u32 = 0xCAFE_FAFA;
const COMPRESSED_HEADER_SIZE: usize = 29;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// The index of an image and the means to read its resources.
struct Image {
    file: File,
    big_endian: bool,
    table_length: u32,
    index: Vec<u8>,
    redirect_offset: usize,
    offsets_offset: usize,
    locations: (usize, usize),
    strings: (usize, usize),
}

impl Image {
    /// `BasicImageReader(path)`: reads and checks the header, then the index.
    fn open(path: &Path) -> io::Result<Image> {
        let mut file = File::open(path)?;
        let mut header = [0u8; HEADER_SIZE];
        file.read_exact(&mut header)
            .map_err(|_| invalid(format!("\"{}\" is not an image file", path.display())))?;
        let le = |i: usize| u32::from_le_bytes(header[i * 4..i * 4 + 4].try_into().unwrap());
        let be = |i: usize| u32::from_be_bytes(header[i * 4..i * 4 + 4].try_into().unwrap());
        let big_endian = if le(0) == MAGIC {
            false
        } else if be(0) == MAGIC {
            true
        } else {
            return Err(invalid(format!(
                "\"{}\" is not an image file",
                path.display()
            )));
        };
        let slot = |i: usize| if big_endian { be(i) } else { le(i) };
        let version = slot(1);
        if version >> 16 != MAJOR_VERSION || version & 0xFFFF != MINOR_VERSION {
            return Err(invalid(format!(
                "The image file \"{}\" is not the correct version",
                path.display()
            )));
        }
        let table_length = slot(4);
        let locations_size = slot(5) as usize;
        let strings_size = slot(6) as usize;
        let redirect_offset = HEADER_SIZE;
        let offsets_offset = redirect_offset + table_length as usize * 4;
        let locations_offset = offsets_offset + table_length as usize * 4;
        let strings_offset = locations_offset + locations_size;
        let index_size = strings_offset + strings_size;
        let mut index = vec![0u8; index_size];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut index).map_err(|_| {
            invalid(format!(
                "The image file \"{}\" is corrupted",
                path.display()
            ))
        })?;
        Ok(Image {
            file,
            big_endian,
            table_length,
            index,
            redirect_offset,
            offsets_offset,
            locations: (locations_offset, locations_size),
            strings: (strings_offset, strings_size),
        })
    }

    fn int_at(&self, offset: usize) -> i32 {
        let b: [u8; 4] = self.index[offset..offset + 4].try_into().unwrap();
        if self.big_endian {
            i32::from_be_bytes(b)
        } else {
            i32::from_le_bytes(b)
        }
    }

    /// `getLocationIndex(String)`: the location index of `name`, if any.
    fn location_index(&self, name: &str) -> Option<usize> {
        let count = self.table_length as i32;
        if count == 0 {
            return None;
        }
        let slot = hash_code(name, HASH_MULTIPLIER) % count;
        let index = self.int_at(self.redirect_offset + slot as usize * 4);
        if index < 0 {
            Some((-index - 1) as usize)
        } else if index > 0 {
            Some((hash_code(name, index) % count) as usize)
        } else {
            None
        }
    }

    /// `ImageLocation.decompress(locations, offset)`: the attributes at
    /// `offset` of the locations.
    fn attributes(&self, offset: usize) -> io::Result<[u64; ATTRIBUTE_COUNT]> {
        let (start, size) = self.locations;
        let bytes = &self.index[start..start + size];
        if offset >= bytes.len() {
            return Err(invalid("offset"));
        }
        let mut attributes = [0u64; ATTRIBUTE_COUNT];
        let mut i = offset;
        while i < bytes.len() {
            let data = bytes[i];
            i += 1;
            if data <= 0x7 {
                break; // ATTRIBUTE_END
            }
            let kind = (data >> 3) as usize;
            if kind >= ATTRIBUTE_COUNT {
                return Err(invalid(format!("Invalid jimage attribute kind: {kind}")));
            }
            let length = (data & 0x7) as usize + 1;
            if i + length > bytes.len() {
                return Err(invalid("Missing jimage attribute data"));
            }
            attributes[kind] = bytes[i..i + length]
                .iter()
                .fold(0u64, |v, &b| (v << 8) | u64::from(b));
            i += length;
        }
        Ok(attributes)
    }

    /// `ImageStringsReader.stringFromByteBufferMatches(strings, offset,
    /// string, stringOffset)`: how many units of `name` from `from` the
    /// string at `offset` matches, or `None`. The string is NUL-terminated
    /// modified UTF-8; it matches when its UTF-16 units equal `name`'s from
    /// `from`.
    fn match_string(&self, offset: u64, name: &[u16], from: usize) -> Option<usize> {
        let (start, size) = self.strings;
        let bytes = &self.index[start..start + size];
        let offset = offset as usize;
        if offset >= bytes.len() {
            return None;
        }
        let end = bytes[offset..].iter().position(|&b| b == 0)? + offset;
        let units = mutf8_to_utf16(&bytes[offset..end])?;
        if name.len() >= from + units.len() && name[from..from + units.len()] == units[..] {
            Some(units.len())
        } else {
            None
        }
    }

    /// `ImageLocation.verify(String name, long[] attributes, ...)`.
    fn verify(&self, name: &[u16], attributes: &[u64; ATTRIBUTE_COUNT]) -> bool {
        let length = name.len();
        let mut index = 0;
        let slash = u16::from(b'/');
        let module = attributes[ATTRIBUTE_MODULE];
        if module != 0 && length >= 1 {
            let Some(module_len) = self.match_string(module, name, 1) else {
                return false;
            };
            index = module_len + 1;
            if length <= index || name[0] != slash || name[index] != slash {
                return false;
            }
            index += 1;
        }
        let parent = attributes[ATTRIBUTE_PARENT];
        if parent != 0 {
            let Some(parent_len) = self.match_string(parent, name, index) else {
                return false;
            };
            index += parent_len;
            if length <= index || name[index] != slash {
                return false;
            }
            index += 1;
        }
        let Some(base_len) = self.match_string(attributes[ATTRIBUTE_BASE], name, index) else {
            return false;
        };
        index += base_len;
        let extension = attributes[ATTRIBUTE_EXTENSION];
        if extension != 0 {
            if length <= index || name[index] != u16::from(b'.') {
                return false;
            }
            index += 1;
            let Some(ext_len) = self.match_string(extension, name, index) else {
                return false;
            };
            index += ext_len;
        }
        length == index
    }

    /// `findLocation(name)` then `getResource(location)`.
    fn resource(&mut self, name: &str) -> io::Result<Option<Vec<u8>>> {
        let Some(index) = self.location_index(name) else {
            return Ok(None);
        };
        if index >= self.table_length as usize {
            return Ok(None);
        }
        let offset = self.int_at(self.offsets_offset + index * 4);
        if offset < 0 {
            return Err(invalid("offset"));
        }
        let attributes = self.attributes(offset as usize)?;
        let units: Vec<u16> = name.encode_utf16().collect();
        if !self.verify(&units, &attributes) {
            return Ok(None);
        }
        let content_offset = attributes[ATTRIBUTE_OFFSET] + self.index.len() as u64;
        let compressed = attributes[ATTRIBUTE_COMPRESSED];
        let uncompressed = attributes[ATTRIBUTE_UNCOMPRESSED];
        if compressed > i32::MAX as u64 || uncompressed > i32::MAX as u64 {
            return Err(invalid("Bad size"));
        }
        let size = if compressed == 0 {
            uncompressed
        } else {
            compressed
        };
        let mut content = vec![0u8; size as usize];
        self.file.seek(SeekFrom::Start(content_offset))?;
        self.file.read_exact(&mut content)?;
        if compressed != 0 {
            content = self.decompress(content)?;
        }
        Ok(Some(content))
    }

    /// `Decompressor.decompressResource`: undoes each compression header.
    fn decompress(&self, mut content: Vec<u8>) -> io::Result<Vec<u8>> {
        loop {
            if content.len() < COMPRESSED_HEADER_SIZE {
                return Ok(content);
            }
            let u32_at = |i: usize| {
                let b: [u8; 4] = content[i..i + 4].try_into().unwrap();
                if self.big_endian {
                    u32::from_be_bytes(b)
                } else {
                    u32::from_le_bytes(b)
                }
            };
            let u64_at = |i: usize| {
                let b: [u8; 8] = content[i..i + 8].try_into().unwrap();
                if self.big_endian {
                    u64::from_be_bytes(b)
                } else {
                    u64::from_le_bytes(b)
                }
            };
            if u32_at(0) != COMPRESSED_MAGIC {
                return Ok(content);
            }
            let uncompressed = u64_at(12);
            let name_offset = u32_at(20);
            let plugin = self.string_at(name_offset as usize)?;
            if plugin != "zip" {
                return Err(invalid(format!("Plugin not found: {plugin}")));
            }
            if uncompressed > i32::MAX as u64 {
                return Err(invalid("Required array size too large"));
            }
            content = zip_decompress(&content[COMPRESSED_HEADER_SIZE..], uncompressed as usize)?;
        }
    }

    /// `getString(offset)` for an ASCII or modified UTF-8 string.
    fn string_at(&self, offset: usize) -> io::Result<String> {
        let (start, size) = self.strings;
        let bytes = &self.index[start..start + size];
        if offset >= bytes.len() {
            return Err(invalid("offset"));
        }
        let end = bytes[offset..]
            .iter()
            .position(|&b| b == 0)
            .map_or(bytes.len(), |p| p + offset);
        let units = mutf8_to_utf16(&bytes[offset..end]).ok_or_else(|| invalid("bad string"))?;
        Ok(String::from_utf16_lossy(&units))
    }
}

/// `ZipDecompressor.decompress`: inflates zlib `input` into exactly
/// `original_size` bytes ("Resource content size mismatch" otherwise).
fn zip_decompress(input: &[u8], original_size: usize) -> io::Result<Vec<u8>> {
    let mut out = vec![0u8; original_size];
    let mut inflater = flate2::Decompress::new(true);
    loop {
        let before_in = inflater.total_in() as usize;
        let before_out = inflater.total_out() as usize;
        if before_out >= original_size {
            break;
        }
        let status = inflater
            .decompress(
                &input[before_in..],
                &mut out[before_out..],
                flate2::FlushDecompress::None,
            )
            .map_err(|e| invalid(e.to_string()))?;
        if status == flate2::Status::StreamEnd {
            break;
        }
        if inflater.total_in() as usize == before_in && inflater.total_out() as usize == before_out
        {
            break;
        }
    }
    if inflater.total_out() as usize != original_size {
        return Err(invalid("Resource content size mismatch"));
    }
    Ok(out)
}

/// The UTF-16 units of modified UTF-8 `bytes`, `None` when malformed.
fn mutf8_to_utf16(bytes: &[u8]) -> Option<Vec<u16>> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b & 0x80 == 0 {
            out.push(u16::from(b));
            i += 1;
        } else if b & 0xE0 == 0xC0 {
            let b2 = *bytes.get(i + 1)?;
            out.push((u16::from(b & 0x1F) << 6) | u16::from(b2 & 0x3F));
            i += 2;
        } else if b & 0xF0 == 0xE0 {
            let b2 = *bytes.get(i + 1)?;
            let b3 = *bytes.get(i + 2)?;
            out.push(
                (u16::from(b & 0x0F) << 12) | (u16::from(b2 & 0x3F) << 6) | u16::from(b3 & 0x3F),
            );
            i += 3;
        } else {
            return None;
        }
    }
    Some(out)
}

/// `ImageStringsReader.hashCode(name, seed)`: `unmaskedHashCode` masked to
/// 31 bits.
fn hash_code(name: &str, seed: i32) -> i32 {
    unmasked_hash_code(name, seed) & POSITIVE_MASK
}

/// `ImageStringsReader.unmaskedHashCode(s, seed)` over the UTF-16 units of
/// `s` (modified UTF-8 bytes).
fn unmasked_hash_code(s: &str, mut seed: i32) -> i32 {
    let mut buffer = [0u8; 8];
    for unit in s.encode_utf16() {
        let mut uch = i32::from(unit);
        if uch & !0x7F != 0 {
            let mut mask: i32 = !0x3F;
            let mut n = 0usize;
            loop {
                buffer[n] = (0x80 | (uch & 0x3F)) as u8;
                n += 1;
                uch >>= 6;
                mask >>= 1;
                if uch & mask == 0 {
                    break;
                }
            }
            buffer[n] = ((mask << 1) | uch) as u8;
            loop {
                seed = seed.wrapping_mul(HASH_MULTIPLIER) ^ i32::from(buffer[n]);
                if n == 0 {
                    break;
                }
                n -= 1;
            }
        } else if uch == 0 {
            seed = seed.wrapping_mul(HASH_MULTIPLIER) ^ 0xC0;
            seed = seed.wrapping_mul(HASH_MULTIPLIER) ^ 0x80;
        } else {
            seed = seed.wrapping_mul(HASH_MULTIPLIER) ^ uch;
        }
    }
    seed
}

/// Reads the resource `name` (`/module/path`) of the image at `path`:
/// `Ok(None)` when the image has no such resource.
///
/// # Errors
///
/// When the file cannot be read, is not a jimage of version 1.0, is
/// corrupted, or compresses the resource with a decompressor other than
/// `zip`.
pub(crate) fn read_resource(path: &Path, name: &str) -> io::Result<Option<Vec<u8>>> {
    Image::open(path)?.resource(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// One resource of a synthetic image: module, parent, base, extension,
    /// content, and whether it is stored `zip`-compressed.
    type Entry<'a> = (&'a str, &'a str, &'a str, &'a str, &'a [u8], bool);

    /// Writes a jimage holding `entries` in the byte order `big_endian`, as
    /// `ImageHeader.writeTo`, `ImageLocation.compress` and
    /// `CompressedResourceHeader.getBytes` lay one out; the table length is
    /// the entry count and every slot is a direct (negative) redirect.
    fn image(entries: &[Entry], big_endian: bool) -> Vec<u8> {
        let put32 = |v: u32| {
            if big_endian {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        };
        let put64 = |v: u64| {
            if big_endian {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        };
        let mut strings: Vec<u8> = vec![0]; // offset 0: the empty string
        let mut add = |s: &str| -> u64 {
            if s.is_empty() {
                return 0;
            }
            let o = strings.len() as u64;
            strings.extend_from_slice(s.as_bytes());
            strings.push(0);
            o
        };
        let zip_name = add("zip");
        let n = entries.len();
        let full_name = |(module, parent, base, ext, _, _): &Entry| {
            let mut name = format!("/{module}/");
            if !parent.is_empty() {
                name.push_str(parent);
                name.push('/');
            }
            name.push_str(base);
            if !ext.is_empty() {
                name.push('.');
                name.push_str(ext);
            }
            name
        };
        // the smallest table whose direct slots do not collide
        let table = (n..64)
            .find(|&t| {
                let mut slots: Vec<i32> = entries
                    .iter()
                    .map(|e| hash_code(&full_name(e), HASH_MULTIPLIER) % t as i32)
                    .collect();
                slots.sort_unstable();
                slots.dedup();
                slots.len() == n
            })
            .expect("a table without collisions");
        let mut locations: Vec<u8> = vec![0];
        let mut offsets = vec![0u32; table];
        let mut redirect = vec![0i32; table];
        let mut content: Vec<u8> = Vec::new();
        for (i, entry) in entries.iter().enumerate() {
            let (module, parent, base, ext, data, zip) = entry;
            let name = full_name(entry);
            let slot = (hash_code(&name, HASH_MULTIPLIER) % table as i32) as usize;
            redirect[slot] = -(i as i32) - 1;
            let stored: Vec<u8> = if *zip {
                let mut e =
                    flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(data).unwrap();
                let z = e.finish().unwrap();
                let mut h = Vec::new();
                h.extend_from_slice(&put32(COMPRESSED_MAGIC));
                h.extend_from_slice(&put64(z.len() as u64));
                h.extend_from_slice(&put64(data.len() as u64));
                h.extend_from_slice(&put32(zip_name as u32));
                h.extend_from_slice(&put32(u32::MAX));
                h.push(1);
                h.extend_from_slice(&z);
                h
            } else {
                data.to_vec()
            };
            let attrs = [
                (ATTRIBUTE_MODULE, add(module)),
                (ATTRIBUTE_PARENT, add(parent)),
                (ATTRIBUTE_BASE, add(base)),
                (ATTRIBUTE_EXTENSION, add(ext)),
                (ATTRIBUTE_OFFSET, content.len() as u64),
                (
                    ATTRIBUTE_COMPRESSED,
                    if *zip { stored.len() as u64 } else { 0 },
                ),
                (ATTRIBUTE_UNCOMPRESSED, data.len() as u64),
            ];
            offsets[i] = locations.len() as u32;
            for (kind, value) in attrs {
                if value != 0 {
                    let len = (64 - value.leading_zeros() as usize).div_ceil(8);
                    locations.push(((kind << 3) | (len - 1)) as u8);
                    for k in (0..len).rev() {
                        locations.push((value >> (k * 8)) as u8);
                    }
                }
            }
            locations.push(0);
            content.extend_from_slice(&stored);
        }
        let mut out = Vec::new();
        for v in [
            MAGIC,
            (MAJOR_VERSION << 16) | MINOR_VERSION,
            0,
            n as u32,
            table as u32,
            locations.len() as u32,
            strings.len() as u32,
        ] {
            out.extend_from_slice(&put32(v));
        }
        for r in redirect {
            out.extend_from_slice(&put32(r as u32));
        }
        for o in offsets {
            out.extend_from_slice(&put32(o));
        }
        out.extend_from_slice(&locations);
        out.extend_from_slice(&strings);
        out.extend_from_slice(&content);
        out
    }

    fn write_temp(bytes: &[u8], tag: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("rucene-jimage-{tag}-{}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn reads_stored_and_zip_resources_in_both_byte_orders() {
        let payload: Vec<u8> = (0..5000u32).map(|i| (i * 7 % 251) as u8).collect();
        let entries: [Entry; 2] = [
            (
                "jdk.localedata",
                "sun/text/resources/ext",
                "thai_dict",
                "",
                b"stored bytes",
                false,
            ),
            ("java.base", "a/b", "c", "txt", &payload, true),
        ];
        for big_endian in [false, true] {
            let path = write_temp(&image(&entries, big_endian), &format!("ok{big_endian}"));
            let a = read_resource(&path, "/jdk.localedata/sun/text/resources/ext/thai_dict");
            let b = read_resource(&path, "/java.base/a/b/c.txt");
            // a name that hashes to a slot but does not verify
            let c = read_resource(&path, "/java.base/a/b/c.txx");
            let _ = std::fs::remove_file(&path);
            assert_eq!(Some(b"stored bytes".to_vec()), a.unwrap());
            assert_eq!(Some(payload.clone()), b.unwrap());
            assert_eq!(None, c.unwrap());
        }
    }

    #[test]
    fn rejects_a_file_that_is_not_an_image() {
        let path = write_temp(b"not an image at all, longer than a header", "bad");
        let r = read_resource(&path, "/m/x");
        let _ = std::fs::remove_file(&path);
        assert!(r.is_err());
    }

    #[test]
    fn hash_code_of_modified_utf8_units() {
        // ASCII: seed * M ^ byte per unit; U+0000 as C0 80; U+00E9 as C3 A9.
        let m = HASH_MULTIPLIER;
        let ascii = (m.wrapping_mul(m) ^ 0x61) & POSITIVE_MASK;
        assert_eq!(ascii, hash_code("a", m));
        let nul = ((m.wrapping_mul(m) ^ 0xC0).wrapping_mul(m) ^ 0x80) & POSITIVE_MASK;
        assert_eq!(nul, hash_code("\u{0}", m));
        let e = ((m.wrapping_mul(m) ^ 0xC3).wrapping_mul(m) ^ 0xA9) & POSITIVE_MASK;
        assert_eq!(e, hash_code("\u{e9}", m));
    }
}
