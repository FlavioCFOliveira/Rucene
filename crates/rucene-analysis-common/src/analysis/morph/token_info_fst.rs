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

//! The type declared in `TokenInfoFST.java`, of the Java package
//! `org.apache.lucene.analysis.morph`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::util::fst::{Arc as FstArc, BytesReader, Outputs, PositiveIntOutputs, FST};

/// Thin wrapper around an FST with root-arc caching.
///
/// Root arcs between `cacheFloor` and `cacheFloor` are cached.
///
/// Equivalent to `org.apache.lucene.analysis.morph.TokenInfoFST`, an
/// abstract class without abstract methods: a concrete wrapper holds a
/// `TokenInfoFST`. Java's `FST<Long>` is an `FST<PositiveIntOutputs>`,
/// shared through an [`Arc`]; a missing cached arc (Java's `null`) is
/// `None`.
#[derive(Debug)]
pub struct TokenInfoFST {
    /// Equivalent to the protected `TokenInfoFST.fst`.
    pub fst: Arc<FST<PositiveIntOutputs>>,
    cache_ceiling: i32,
    cache_floor: i32,
    root_cache: Vec<Option<FstArc<i64>>>,
    /// Equivalent to `TokenInfoFST.NO_OUTPUT`.
    pub no_output: i64,
}

impl TokenInfoFST {
    /// Equivalent to the protected `TokenInfoFST(FST<Long> fst, int
    /// cacheCeiling, int cacheFloor)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` (`cacheCeiling must be larger than
    /// cacheFloor; …`), and what reading the root arcs raises.
    pub fn new(
        fst: Arc<FST<PositiveIntOutputs>>,
        cache_ceiling: i32,
        cache_floor: i32,
    ) -> Result<Self> {
        if cache_ceiling < cache_floor {
            return Err(LuceneError::illegal_argument(format!(
                "cacheCeiling must be larger than cacheFloor; cacheCeiling={cache_ceiling}, cacheFloor={cache_floor}"
            )));
        }
        let no_output = fst.outputs().no_output();
        let mut token_info_fst = Self {
            fst,
            cache_ceiling,
            cache_floor,
            root_cache: Vec::new(),
            no_output,
        };
        token_info_fst.root_cache = token_info_fst.cache_root_arcs()?;
        Ok(token_info_fst)
    }

    /// Equivalent to the private `TokenInfoFST.cacheRootArcs()`.
    fn cache_root_arcs(&self) -> Result<Vec<Option<FstArc<i64>>>> {
        let len = (1 + (self.cache_ceiling - self.cache_floor)) as usize;
        let mut root_cache: Vec<Option<FstArc<i64>>> = vec![None; len];
        let mut first_arc = FstArc::default();
        self.fst.get_first_arc(&mut first_arc);
        let mut arc = FstArc::default();
        let mut fst_reader = self.fst.get_bytes_reader()?;
        // TODO: jump to cacheFloor, readNextRealArc to ceiling? (just be careful we don't add bugs)
        for (i, slot) in root_cache.iter_mut().enumerate() {
            if self.fst.find_target_arc(
                self.cache_floor + i as i32,
                &first_arc,
                &mut arc,
                fst_reader.as_mut(),
            )? {
                let mut cached = FstArc::default();
                cached.copy_from(&arc);
                *slot = Some(cached);
            }
        }
        Ok(root_cache)
    }

    /// Equivalent to `TokenInfoFST.findTargetArc(int ch, Arc<Long> follow,
    /// Arc<Long> arc, boolean useCache, FST.BytesReader fstReader)`: `true`
    /// with `arc` filled where Java returns `arc`, `false` where it returns
    /// `null`.
    ///
    /// # Errors
    ///
    /// What reading the FST raises.
    pub fn find_target_arc(
        &self,
        ch: i32,
        follow: &FstArc<i64>,
        arc: &mut FstArc<i64>,
        use_cache: bool,
        fst_reader: &mut dyn BytesReader,
    ) -> Result<bool> {
        if use_cache && ch >= self.cache_floor && ch <= self.cache_ceiling {
            match &self.root_cache[(ch - self.cache_floor) as usize] {
                None => Ok(false),
                Some(result) => {
                    arc.copy_from(result);
                    Ok(true)
                }
            }
        } else {
            self.fst.find_target_arc(ch, follow, arc, fst_reader)
        }
    }

    /// [`find_target_arc`](Self::find_target_arc) with the same arc as
    /// `follow` and `arc`, as `Viterbi` calls it (`findTargetArc(ch, arc, arc,
    /// …)`).
    ///
    /// # Errors
    ///
    /// What reading the FST raises.
    pub fn find_target_arc_in_place(
        &self,
        ch: i32,
        arc: &mut FstArc<i64>,
        use_cache: bool,
        fst_reader: &mut dyn BytesReader,
    ) -> Result<bool> {
        if use_cache && ch >= self.cache_floor && ch <= self.cache_ceiling {
            match &self.root_cache[(ch - self.cache_floor) as usize] {
                None => Ok(false),
                Some(result) => {
                    arc.copy_from(result);
                    Ok(true)
                }
            }
        } else {
            self.fst.find_target_arc_in_place(ch, arc, fst_reader)
        }
    }

    /// Equivalent to `TokenInfoFST.getFirstArc(Arc<Long>)`.
    pub fn get_first_arc<'a>(&self, arc: &'a mut FstArc<i64>) -> &'a mut FstArc<i64> {
        self.fst.get_first_arc(arc);
        arc
    }

    /// Equivalent to `TokenInfoFST.getBytesReader()`.
    ///
    /// # Errors
    ///
    /// What opening the FST's store raises.
    pub fn get_bytes_reader(&self) -> Result<Box<dyn BytesReader + '_>> {
        self.fst.get_bytes_reader()
    }
}
