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

//! Port of `CharBlockArray.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy.writercache`.

use rucene::error::{LuceneError, Result};

/// Similar to `StringBuilder`, but with a more efficient growing strategy.
/// This class uses char array blocks to grow.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.writercache.CharBlockArray`;
/// Java `char`s are UTF-16 code units.
// Java package-private class; public for the port of `TestCharBlockArray`.
#[derive(Debug, Clone)]
pub struct CharBlockArray {
    pub blocks: Vec<Block>,
    /// Index of the current block in `blocks` (Java's `current` reference).
    current: usize,
    pub block_size: usize,
    pub length: usize,
}

/// Equivalent to `org.apache.lucene.facet.taxonomy.writercache.CharBlockArray.Block`.
#[derive(Debug, Clone)]
pub struct Block {
    pub chars: Vec<u16>,
    pub length: usize,
}

impl Block {
    /// Equivalent to `Block(int)`.
    fn new(size: usize) -> Self {
        Self {
            chars: vec![0; size],
            length: 0,
        }
    }
}

impl CharBlockArray {
    /// `DefaultBlockSize`: 32 KB default size.
    const DEFAULT_BLOCK_SIZE: usize = 32 * 1024;

    /// Equivalent to `CharBlockArray()`.
    pub fn new() -> Result<Self> {
        Self::with_block_size(Self::DEFAULT_BLOCK_SIZE)
    }

    /// Equivalent to `CharBlockArray(int)`.
    pub fn with_block_size(block_size: usize) -> Result<Self> {
        let mut array = Self {
            blocks: Vec::new(),
            current: 0,
            block_size,
            length: 0,
        };
        array.add_block()?;
        Ok(array)
    }

    /// Equivalent to `addBlock()`.
    fn add_block(&mut self) -> Result<()> {
        if self.block_size as i64 * (self.blocks.len() as i64 + 1) > i32::MAX as i64 {
            return Err(LuceneError::illegal_state(
                "cannot store more than 2 GB in CharBlockArray",
            ));
        }
        self.blocks.push(Block::new(self.block_size));
        self.current = self.blocks.len() - 1;
        Ok(())
    }

    /// Equivalent to `blockIndex(int)`.
    pub fn block_index(&self, index: usize) -> usize {
        index / self.block_size
    }

    /// Equivalent to `indexInBlock(int)`.
    pub fn index_in_block(&self, index: usize) -> usize {
        index % self.block_size
    }

    /// Equivalent to `append(CharSequence)`.
    pub fn append_char_sequence(&mut self, chars: &[u16]) -> Result<&mut Self> {
        self.append_char_sequence_range(chars, 0, chars.len())
    }

    /// Equivalent to `append(char)`.
    pub fn append_char(&mut self, c: u16) -> Result<&mut Self> {
        if self.blocks[self.current].length == self.block_size {
            self.add_block()?;
        }
        let current = &mut self.blocks[self.current];
        current.chars[current.length] = c;
        current.length += 1;
        self.length += 1;
        Ok(self)
    }

    /// Equivalent to `append(CharSequence, int, int)`.
    pub fn append_char_sequence_range(
        &mut self,
        chars: &[u16],
        start: usize,
        length: usize,
    ) -> Result<&mut Self> {
        let end = start + length;
        for &c in &chars[start..end] {
            self.append_char(c)?;
        }
        Ok(self)
    }

    /// Equivalent to `append(char[], int, int)`.
    pub fn append_chars(
        &mut self,
        chars: &[u16],
        start: usize,
        length: usize,
    ) -> Result<&mut Self> {
        let mut offset = start;
        let mut remain = length;
        while remain > 0 {
            if self.blocks[self.current].length == self.block_size {
                self.add_block()?;
            }
            let mut to_copy = remain;
            let current = &mut self.blocks[self.current];
            let remaining_in_block = self.block_size - current.length;
            if remaining_in_block < to_copy {
                to_copy = remaining_in_block;
            }
            current.chars[current.length..current.length + to_copy]
                .copy_from_slice(&chars[offset..offset + to_copy]);
            offset += to_copy;
            remain -= to_copy;
            current.length += to_copy;
        }
        self.length += length;
        Ok(self)
    }

    /// Equivalent to `append(String)`.
    pub fn append_str(&mut self, s: &str) -> Result<&mut Self> {
        let units: Vec<u16> = s.encode_utf16().collect();
        self.append_chars(&units, 0, units.len())
    }

    /// Equivalent to `charAt(int)`.
    pub fn char_at(&self, index: usize) -> u16 {
        let b = &self.blocks[self.block_index(index)];
        b.chars[self.index_in_block(index)]
    }

    /// Equivalent to `length()`.
    pub fn length(&self) -> usize {
        self.length
    }

    /// Equivalent to `subSequence(int, int)`.
    pub fn sub_sequence(&self, start: usize, end: usize) -> String {
        let mut remaining = end - start;
        let mut sb: Vec<u16> = Vec::with_capacity(remaining);
        let mut block_idx = self.block_index(start);
        let mut index_in_block = self.index_in_block(start);
        while remaining > 0 {
            let b = &self.blocks[block_idx];
            block_idx += 1;
            let num_to_append = remaining.min(b.length - index_in_block);
            sb.extend_from_slice(&b.chars[index_in_block..index_in_block + num_to_append]);
            remaining -= num_to_append;
            index_in_block = 0; // 2nd+ iterations read from start of the block
        }
        String::from_utf16_lossy(&sb)
    }
}

impl std::fmt::Display for CharBlockArray {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sb: Vec<u16> = Vec::new();
        for b in &self.blocks {
            sb.extend_from_slice(&b.chars[..b.length]);
        }
        f.write_str(&String::from_utf16_lossy(&sb))
    }
}
