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

//! Holds a map of String input to String output, to be used with
//! `MappingCharFilter`; ported from
//! `org.apache.lucene.analysis.charfilter.NormalizeCharMap`.

use std::collections::BTreeMap;

use rucene::error::{LuceneError, Result};
use rucene::internal::hppc::CharObjectHashMap;
use rucene::util::fst::{
    target_has_arcs, Arc as FSTArc, Builder as FSTCompilerBuilder, CharSequenceOutputs, InputType,
    END_LABEL, FST,
};
use rucene::util::{CharsRef, IntsRef, IntsRefBuilder};

/// Java catches the `IOException`s of its FST calls ("Bogus FST IOExceptions!!
/// (will never happen)") and rethrows them as `new RuntimeException(ioe)`;
/// any other error passes through.
fn rethrow_io(e: LuceneError) -> LuceneError {
    if e.is_io_exception() {
        LuceneError::runtime_exception(e)
    } else {
        e
    }
}

/// Holds a map of String input to String output, to be used with
/// `MappingCharFilter`. Use the [`Builder`] to create this.
///
/// Equivalent to `org.apache.lucene.analysis.charfilter.NormalizeCharMap`.
#[derive(Debug)]
pub struct NormalizeCharMap {
    /// Java's package-private `map`, `null` (an FST that accepts nothing) as
    /// `None`.
    pub(crate) map: Option<FST<CharSequenceOutputs>>,
    /// Java's package-private `cachedRootArcs`.
    pub(crate) cached_root_arcs: CharObjectHashMap<FSTArc<CharsRef>>,
}

impl NormalizeCharMap {
    /// Use the builder to create.
    ///
    /// Equivalent to the private `NormalizeCharMap(FST<CharsRef>)`, which
    /// pre-caches the root arcs.
    ///
    /// # Errors
    ///
    /// Java's `RuntimeException` around an `IOException` of the FST.
    fn new(map: Option<FST<CharSequenceOutputs>>) -> Result<Self> {
        let mut cached_root_arcs = CharObjectHashMap::new();
        if let Some(map) = &map {
            // Pre-cache root arcs:
            let mut scratch_arc: FSTArc<CharsRef> = FSTArc::default();
            let mut fst_reader = map.get_bytes_reader().map_err(rethrow_io)?;
            map.get_first_arc(&mut scratch_arc);
            if target_has_arcs(&scratch_arc) {
                map.read_first_real_target_arc(
                    scratch_arc.target(),
                    &mut scratch_arc,
                    &mut *fst_reader,
                )
                .map_err(rethrow_io)?;
                loop {
                    debug_assert!(scratch_arc.label() != END_LABEL);
                    let mut cached: FSTArc<CharsRef> = FSTArc::default();
                    cached.copy_from(&scratch_arc);
                    cached_root_arcs.put(scratch_arc.label() as u16, cached);
                    if scratch_arc.is_last() {
                        break;
                    }
                    map.read_next_real_arc(&mut scratch_arc, &mut *fst_reader)
                        .map_err(rethrow_io)?;
                }
            }
            // System.out.println("cached " + cachedRootArcs.size() + " root arcs");
        }
        Ok(Self {
            map,
            cached_root_arcs,
        })
    }
}

/// Builds an NormalizeCharMap.
///
/// Call add() until you have added all the mappings, then call build() to get
/// a NormalizeCharMap
///
/// Equivalent to `org.apache.lucene.analysis.charfilter.NormalizeCharMap$Builder`
/// (`@lucene.experimental`). The strings are UTF-16 code units, as Java's
/// `String`s are, so a mapping may hold a lone surrogate (a `\uD800` escape of
/// a mapping file); [`add`](Self::add) takes Rust strings.
#[derive(Debug, Default)]
pub struct Builder {
    /// Java's `TreeMap<String, String>`: ordered by `String.compareTo`, the
    /// lexicographic order of UTF-16 code units, which `Vec<u16>` keys have.
    pending_pairs: BTreeMap<Vec<u16>, Vec<u16>>,
}

/// `Util.toUTF16(CharSequence, IntsRefBuilder)` over UTF-16 code units: each
/// unit of `s` an int of `scratch`.
fn to_utf16(s: &[u16], scratch: &mut IntsRefBuilder) -> IntsRef {
    let char_limit = s.len();
    scratch.set_length(char_limit);
    scratch.grow_no_copy(char_limit);
    for (idx, &unit) in s.iter().enumerate() {
        scratch.set_int_at(idx, i32::from(unit));
    }
    scratch.get()
}

impl Builder {
    /// Equivalent to `new NormalizeCharMap.Builder()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a replacement to be applied to the input stream. Whenever
    /// `match_` occurs in the input, it will be replaced with `replacement`.
    ///
    /// Equivalent to `Builder.add(String, String)`; see
    /// [`add_utf16`](Self::add_utf16).
    ///
    /// # Errors
    ///
    /// As [`add_utf16`](Self::add_utf16).
    pub fn add(&mut self, match_: &str, replacement: &str) -> Result<()> {
        let match_units: Vec<u16> = match_.encode_utf16().collect();
        let replacement_units: Vec<u16> = replacement.encode_utf16().collect();
        self.add_utf16(&match_units, &replacement_units)
    }

    /// Records a replacement to be applied to the input stream. Whenever
    /// `match_` occurs in the input, it will be replaced with `replacement`.
    ///
    /// Equivalent to `Builder.add(String match, String replacement)` for
    /// Java strings given as their UTF-16 code units.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` if `match_` is the empty string (`cannot
    /// match the empty string`), or was already previously added (`match "…"
    /// was already added`).
    pub fn add_utf16(&mut self, match_: &[u16], replacement: &[u16]) -> Result<()> {
        if match_.is_empty() {
            return Err(LuceneError::illegal_argument(
                "cannot match the empty string".to_string(),
            ));
        }
        if self.pending_pairs.contains_key(match_) {
            return Err(LuceneError::illegal_argument(format!(
                "match \"{}\" was already added",
                String::from_utf16_lossy(match_)
            )));
        }
        self.pending_pairs
            .insert(match_.to_vec(), replacement.to_vec());
        Ok(())
    }

    /// Builds the NormalizeCharMap; call this once you are done calling
    /// [`add`](Self::add).
    ///
    /// Equivalent to `Builder.build()`: an FST over 2-byte labels (the UTF-16
    /// code units of the matches, `Util.toUTF16`) with `CharSequenceOutputs`,
    /// the matches added in `String` order; the pending pairs are cleared.
    ///
    /// # Errors
    ///
    /// Java's `RuntimeException` around an `IOException` of the FST, and the
    /// errors of [`FSTCompiler::add`](rucene::util::fst::FSTCompiler::add).
    pub fn build(&mut self) -> Result<NormalizeCharMap> {
        let map = {
            let outputs = CharSequenceOutputs::get_singleton();
            let mut fst_compiler = FSTCompilerBuilder::new(InputType::Byte2, outputs)
                .build()
                .map_err(rethrow_io)?;
            let mut scratch = IntsRefBuilder::new();
            for (key, value) in &self.pending_pairs {
                fst_compiler
                    .add(
                        &to_utf16(key, &mut scratch),
                        CharsRef::from_chars(value.clone()),
                    )
                    .map_err(rethrow_io)?;
            }
            let metadata = fst_compiler.compile().map_err(rethrow_io)?;
            let map = FST::from_fst_reader(
                metadata,
                fst_compiler.into_fst_reader().map_err(rethrow_io)?,
            );
            self.pending_pairs.clear();
            map
        };

        NormalizeCharMap::new(map)
    }
}
