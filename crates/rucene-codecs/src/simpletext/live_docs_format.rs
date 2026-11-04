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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextLiveDocsFormat`.

use rucene::codecs::LiveDocsFormat;
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::file_name_from_generation;
use rucene::index::SegmentCommitInfo;
use rucene::store::{Directory, IOContext};
use rucene::util::Bits;

use super::util::{self, parse_int_at, read_line};

pub(crate) const LIVEDOCS_EXTENSION: &str = "liv";

pub(crate) const SIZE: &[u8] = b"size ";
pub(crate) const DOC: &[u8] = b"  doc ";
pub(crate) const END: &[u8] = b"END";

/// Reads/writes plaintext live docs.
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextLiveDocsFormat`.
///
/// **FOR RECREATIONAL USE ONLY**
#[derive(Debug, Default, Clone, Copy)]
pub struct SimpleTextLiveDocsFormat;

impl SimpleTextLiveDocsFormat {
    /// Creates the format.
    pub fn new() -> Self {
        Self
    }
}

/// `IndexFileNames.fileNameFromGeneration`, which returns `null` for
/// generation `-1`; the Java callers then fail on the `null` name.
fn generation_file_name(base: &str, gen: i64) -> Result<String> {
    file_name_from_generation(base, LIVEDOCS_EXTENSION, gen).ok_or_else(|| {
        LuceneError::IllegalArgument(format!("no live-docs file for generation {gen}"))
    })
}

impl LiveDocsFormat for SimpleTextLiveDocsFormat {
    fn name(&self) -> &str {
        "SimpleTextLiveDocsFormat"
    }

    /// Port of `SimpleTextLiveDocsFormat.readLiveDocs`.
    fn read_live_docs(
        &self,
        dir: &dyn Directory,
        info: &SegmentCommitInfo,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn Bits>> {
        debug_assert!(info.has_deletions());
        let mut scratch = Vec::new();

        let file_name = generation_file_name(&info.info.name, info.get_del_gen())?;
        let mut input = dir.open_checksum_input(&file_name)?;

        read_line(input.as_mut(), &mut scratch)?;
        debug_assert!(scratch.starts_with(SIZE));
        let size = parse_int_at(&scratch, SIZE.len())?;

        let mut bits = JavaBitSet::new(size);

        read_line(input.as_mut(), &mut scratch)?;
        while scratch.as_slice() != END {
            debug_assert!(scratch.starts_with(DOC));
            let docid = parse_int_at(&scratch, DOC.len())?;
            bits.set(docid)?;
            read_line(input.as_mut(), &mut scratch)?;
        }

        util::check_footer(input.as_mut())?;

        Ok(Box::new(SimpleTextBits { bits, size }))
    }

    /// Port of `SimpleTextLiveDocsFormat.writeLiveDocs`.
    fn write_live_docs(
        &self,
        bits: &dyn Bits,
        dir: &dyn Directory,
        info: &SegmentCommitInfo,
        _new_del_count: i32,
        context: &dyn IOContext,
    ) -> Result<()> {
        let size = bits.length();

        let file_name = generation_file_name(&info.info.name, info.get_next_del_gen())?;
        let mut output = dir.create_output(&file_name, context)?;
        let out = output.as_mut();
        util::write_bytes(out, SIZE)?;
        util::write_str(out, &size.to_string())?;
        util::write_newline(out)?;

        for i in 0..size {
            if bits.try_get(i)? {
                util::write_bytes(out, DOC)?;
                util::write_str(out, &i.to_string())?;
                util::write_newline(out)?;
            }
        }

        util::write_bytes(out, END)?;
        util::write_newline(out)?;
        util::write_checksum(out)?;
        output.close()
    }

    /// Port of `SimpleTextLiveDocsFormat.files`.
    fn files(&self, info: &SegmentCommitInfo, files: &mut Vec<String>) -> Result<()> {
        if info.has_deletions() {
            files.push(generation_file_name(&info.info.name, info.get_del_gen())?);
        }
        Ok(())
    }
}

/// The `java.util.BitSet` `readLiveDocs` fills: it grows on `set` and
/// answers `false` past its end.
#[derive(Debug, Clone)]
struct JavaBitSet {
    words: Vec<u64>,
}

impl JavaBitSet {
    /// `new BitSet(nbits)`, which throws `NegativeArraySizeException` for a
    /// negative size.
    fn new(nbits: i32) -> Self {
        let words = if nbits <= 0 {
            0
        } else {
            (nbits as usize).div_ceil(64)
        };
        Self {
            words: vec![0; words],
        }
    }

    /// `BitSet.set(int)`, which throws `IndexOutOfBoundsException` for a
    /// negative index.
    fn set(&mut self, index: i32) -> Result<()> {
        if index < 0 {
            return Err(LuceneError::unchecked(
                "java.lang.IndexOutOfBoundsException",
                format!("bitIndex < 0: {index}"),
            ));
        }
        let word = index as usize / 64;
        if word >= self.words.len() {
            self.words.resize(word + 1, 0);
        }
        self.words[word] |= 1u64 << (index as usize % 64);
        Ok(())
    }

    /// `BitSet.get(int)`.
    fn get(&self, index: usize) -> bool {
        let word = index / 64;
        word < self.words.len() && self.words[word] & (1u64 << (index % 64)) != 0
    }
}

/// Read-only live docs.
///
/// Port of the record `org.apache.lucene.codecs.simpletext.SimpleTextLiveDocsFormat.SimpleTextBits`.
#[derive(Debug, Clone)]
struct SimpleTextBits {
    bits: JavaBitSet,
    size: i32,
}

impl Bits for SimpleTextBits {
    fn get(&self, index: usize) -> bool {
        self.bits.get(index)
    }

    fn length(&self) -> usize {
        self.size.max(0) as usize
    }
}
