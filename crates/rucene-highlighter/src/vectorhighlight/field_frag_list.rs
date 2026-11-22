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

//! Port of `FieldFragList.java`, of the Java package
//! `org.apache.lucene.search.vectorhighlight`.

use std::fmt;
use std::hash::{Hash, Hasher};

use rucene::util::java_fmt::float_to_string;

use crate::vectorhighlight::field_phrase_list::{Toffs, WeightedPhraseInfo};

/// FieldFragList has a list of "frag info" that is used by FragmentsBuilder
/// class to create fragments (snippets).
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.vectorhighlight.FieldFragList`: the abstract
/// `add` and the `fragInfos` list, which an implementation keeps.
pub trait FieldFragList: fmt::Debug + Send + Sync {
    /// Convert the list of WeightedPhraseInfo to WeightedFragInfo, then add
    /// it to the fragInfos.
    ///
    /// Equivalent to the abstract `add(int, int, List)`.
    fn add(&mut self, start_offset: i32, end_offset: i32, phrase_info_list: &[WeightedPhraseInfo]);

    /// Return the list of WeightedFragInfos.
    ///
    /// Equivalent to `getFragInfos()`.
    fn get_frag_infos(&self) -> &Vec<WeightedFragInfo>;

    /// `getFragInfos()` for a caller that changes the list Java returns by
    /// reference.
    fn get_frag_infos_mut(&mut self) -> &mut Vec<WeightedFragInfo>;
}

/// List of term offsets + weight for a frag info.
///
/// Equivalent to `org.apache.lucene.search.vectorhighlight.FieldFragList.WeightedFragInfo`.
#[derive(Debug, Clone)]
pub struct WeightedFragInfo {
    sub_infos: Vec<SubInfo>,
    total_boost: f32,
    start_offset: i32,
    end_offset: i32,
}

impl WeightedFragInfo {
    /// Equivalent to `new WeightedFragInfo(int, int, List, float)`.
    pub fn new(start_offset: i32, end_offset: i32, sub_infos: Vec<SubInfo>, total_boost: f32) -> Self {
        Self {
            sub_infos,
            total_boost,
            start_offset,
            end_offset,
        }
    }

    /// Equivalent to `getSubInfos()`.
    pub fn get_sub_infos(&self) -> &Vec<SubInfo> {
        &self.sub_infos
    }

    /// `getSubInfos()` for a caller that changes the list Java returns by
    /// reference.
    pub fn get_sub_infos_mut(&mut self) -> &mut Vec<SubInfo> {
        &mut self.sub_infos
    }

    /// Equivalent to `getTotalBoost()`.
    pub fn get_total_boost(&self) -> f32 {
        self.total_boost
    }

    /// Equivalent to `getStartOffset()`.
    pub fn get_start_offset(&self) -> i32 {
        self.start_offset
    }

    /// Equivalent to `getEndOffset()`.
    pub fn get_end_offset(&self) -> i32 {
        self.end_offset
    }
}

impl fmt::Display for WeightedFragInfo {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("subInfos=(")?;
        for si in &self.sub_infos {
            write!(f, "{si}")?;
        }
        write!(
            f,
            ")/{}({},{})",
            float_to_string(self.total_boost),
            self.start_offset,
            self.end_offset
        )
    }
}

/// Represents a single phrase match.
///
/// Equivalent to the record
/// `org.apache.lucene.search.vectorhighlight.FieldFragList.WeightedFragInfo.SubInfo`:
/// `text` is the entire phrase as text, `terms_offsets` the list of
/// offsets of the terms, `seqnum` the sequence number used to determine
/// which highlight tag to use, and `boost` the boost of the phrase.
#[derive(Debug, Clone)]
pub struct SubInfo {
    text: String,
    terms_offsets: Vec<Toffs>,
    seqnum: i32,
    boost: f32,
}

impl SubInfo {
    /// The canonical record constructor.
    pub fn new(text: String, terms_offsets: Vec<Toffs>, seqnum: i32, boost: f32) -> Self {
        Self {
            text,
            terms_offsets,
            seqnum,
            boost,
        }
    }

    /// The record accessor `text()`.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The record accessor `termsOffsets()`.
    pub fn terms_offsets(&self) -> &Vec<Toffs> {
        &self.terms_offsets
    }

    /// `termsOffsets()` for a caller that changes the list Java returns by
    /// reference.
    pub fn terms_offsets_mut(&mut self) -> &mut Vec<Toffs> {
        &mut self.terms_offsets
    }

    /// The record accessor `seqnum()`.
    pub fn seqnum(&self) -> i32 {
        self.seqnum
    }

    /// The record accessor `boost()`.
    pub fn boost(&self) -> f32 {
        self.boost
    }
}

impl fmt::Display for SubInfo {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(", self.text)?;
        for to in &self.terms_offsets {
            write!(f, "{to}")?;
        }
        f.write_str(")")
    }
}

/// `Float.floatToIntBits(f)`: the record component comparison of a `float`.
fn float_bits(f: f32) -> u32 {
    if f.is_nan() {
        0x7fc0_0000
    } else {
        f.to_bits()
    }
}

impl PartialEq for SubInfo {
    /// The record `equals`: every component equal, the `float` compared by
    /// `Float.compare`.
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
            && self.terms_offsets == other.terms_offsets
            && self.seqnum == other.seqnum
            && float_bits(self.boost) == float_bits(other.boost)
    }
}

impl Eq for SubInfo {}

impl Hash for SubInfo {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
        self.terms_offsets.hash(state);
        self.seqnum.hash(state);
        float_bits(self.boost).hash(state);
    }
}