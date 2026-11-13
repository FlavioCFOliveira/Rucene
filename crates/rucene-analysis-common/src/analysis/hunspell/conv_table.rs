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

//! An `ICONV`/`OCONV` table, ported from
//! `org.apache.lucene.analysis.hunspell.ConvTable`.

use std::collections::BTreeMap;

use rucene::error::Result;
use rucene::util::fst::{
    Arc as FSTArc, Builder as FSTCompilerBuilder, CharSequenceOutputs, InputType, Outputs, FST,
};
use rucene::util::{CharsRef, FixedBitSet, IntsRef};

/// ICONV or OCONV replacement table.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.ConvTable`.
#[derive(Debug)]
pub(crate) struct ConvTable {
    fst: Option<FST<CharSequenceOutputs>>,
    first_char_hashes: FixedBitSet,
    modulo: usize,
}

impl ConvTable {
    /// Builds the table of `mappings`, sorted by key (Java's `TreeMap`
    /// order, UTF-16 unit order).
    ///
    /// Equivalent to `ConvTable(TreeMap<String, String>)`.
    ///
    /// # Errors
    ///
    /// What compiling the FST raises (Java wraps it in a `RuntimeException`).
    pub(crate) fn new(mappings: &BTreeMap<Vec<u16>, Vec<u16>>) -> Result<Self> {
        let modulo = 256usize.max(java_highest_one_bit(mappings.len()) << 1);
        let mut first_char_hashes = FixedBitSet::new(modulo);
        let outputs = CharSequenceOutputs::get_singleton();
        let mut fst_compiler = FSTCompilerBuilder::new(InputType::Byte2, outputs).build()?;
        for (key, value) in mappings {
            debug_assert!(!key.is_empty());
            first_char_hashes.set(usize::from(key[0]) % modulo);
            let scratch_ints = IntsRef::new(key.iter().map(|&c| i32::from(c)).collect());
            fst_compiler.add(&scratch_ints, CharsRef::from_chars(value.clone()))?;
        }
        let metadata = fst_compiler.compile()?;
        let fst = FST::from_fst_reader(metadata, fst_compiler.into_fst_reader()?);
        Ok(Self {
            fst,
            first_char_hashes,
            modulo,
        })
    }

    /// Replaces, left to right, the longest key starting at each position of
    /// `sb` with its value.
    ///
    /// Equivalent to `applyMappings(StringBuilder)`.
    ///
    /// # Errors
    ///
    /// What reading the FST raises (Java wraps it in a `RuntimeException`).
    pub(crate) fn apply_mappings(&self, sb: &mut Vec<u16>) -> Result<()> {
        let Some(fst) = &self.fst else {
            // an FST that accepts nothing: `mightReplaceChar` admits no
            // character, since no mapping set a hash
            return Ok(());
        };
        let outputs = fst.outputs();
        let mut first_arc: Option<FSTArc<CharsRef>> = None;
        let mut bytes_reader = None;
        let mut arc: FSTArc<CharsRef> = FSTArc::default();
        let mut i = 0;
        while i < sb.len() {
            if !self.might_replace_char(sb[i]) {
                i += 1;
                continue;
            }
            if first_arc.is_none() {
                let mut first = FSTArc::default();
                fst.get_first_arc(&mut first);
                first_arc = Some(first);
                bytes_reader = Some(fst.get_bytes_reader()?);
            }
            arc.copy_from(first_arc.as_ref().expect("INVARIANT: set above"));
            let reader = bytes_reader.as_deref_mut().expect("INVARIANT: set above");
            let mut output = outputs.no_output();
            let mut longest_match: i64 = -1;
            let mut longest_output: Option<CharsRef> = None;
            for (j, &ch) in sb.iter().enumerate().skip(i) {
                if !fst.find_target_arc_in_place(i32::from(ch), &mut arc, reader)? {
                    break;
                }
                output = outputs.add(&output, arc.output());
                if arc.is_final() {
                    longest_output = Some(outputs.add(&output, arc.next_final_output()));
                    longest_match = j as i64;
                }
            }
            if longest_match >= 0 {
                let longest_output = longest_output.expect("INVARIANT: set with longest_match");
                let replacement = longest_output.chars().to_vec();
                sb.splice(i..longest_match as usize + 1, replacement);
                // `i += (longestOutput.length - 1)`, then the loop's `i++`
                i = (i as i64 + longest_output.length as i64 - 1) as usize;
            }
            i = i.wrapping_add(1);
        }
        Ok(())
    }

    /// Equivalent to `mightReplaceChar(char)`.
    pub(crate) fn might_replace_char(&self, c: u16) -> bool {
        self.first_char_hashes.get(usize::from(c) % self.modulo)
    }
}

/// `Integer.highestOneBit(int)` of a non-negative count.
fn java_highest_one_bit(i: usize) -> usize {
    if i == 0 {
        0
    } else {
        1 << (usize::BITS - 1 - i.leading_zeros())
    }
}
