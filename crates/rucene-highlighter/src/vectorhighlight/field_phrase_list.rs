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

//! Port of `FieldPhraseList.java`, of the Java package
//! `org.apache.lucene.search.vectorhighlight`.

use std::fmt;
use std::hash::{Hash, Hasher};

use rucene::error::{LuceneError, Result};
use rucene::util::java_fmt::float_to_string;

use crate::vectorhighlight::field_query::{FieldQuery, QueryPhraseMap};
use crate::vectorhighlight::field_term_stack::{FieldTermStack, TermInfo};

/// `new MergedIterator<>(false, iterators)` drained: the elements of the
/// sorted `lists` in merged order, ties going to the earlier list.
pub(crate) fn merged<T: Clone>(lists: &[&[T]], cmp: impl Fn(&T, &T) -> i32) -> Vec<T> {
    let mut positions = vec![0usize; lists.len()];
    let mut out = Vec::with_capacity(lists.iter().map(|l| l.len()).sum());
    loop {
        let mut best: Option<usize> = None;
        for (i, list) in lists.iter().enumerate() {
            if positions[i] >= list.len() {
                continue;
            }
            match best {
                None => best = Some(i),
                Some(b) => {
                    if cmp(&list[positions[i]], &lists[b][positions[b]]) < 0 {
                        best = Some(i);
                    }
                }
            }
        }
        let Some(b) = best else {
            return out;
        };
        out.push(lists[b][positions[b]].clone());
        positions[b] += 1;
    }
}

/// FieldPhraseList has a list of WeightedPhraseInfo that is used by
/// FragListBuilder to create a FieldFragList object.
///
/// Equivalent to `org.apache.lucene.search.vectorhighlight.FieldPhraseList`.
#[derive(Debug, Clone, Default)]
pub struct FieldPhraseList {
    /// List of non-overlapping WeightedPhraseInfo objects.
    pub(crate) phrase_list: Vec<WeightedPhraseInfo>,
}

impl FieldPhraseList {
    /// Equivalent to `new FieldPhraseList(FieldTermStack, FieldQuery)`.
    pub fn new(field_term_stack: &mut FieldTermStack, field_query: &FieldQuery) -> Self {
        Self::with_phrase_limit(field_term_stack, field_query, i32::MAX)
    }

    /// Return the list of WeightedPhraseInfo.
    ///
    /// Equivalent to `getPhraseList()`.
    pub fn get_phrase_list(&self) -> &Vec<WeightedPhraseInfo> {
        &self.phrase_list
    }

    /// `getPhraseList()` for a caller that changes the list Java returns by
    /// reference.
    pub fn get_phrase_list_mut(&mut self) -> &mut Vec<WeightedPhraseInfo> {
        &mut self.phrase_list
    }

    /// Equivalent to `new FieldPhraseList(FieldTermStack, FieldQuery, int)`:
    /// `phrase_limit` is the maximum size of the phrase list.
    pub fn with_phrase_limit(
        field_term_stack: &mut FieldTermStack,
        field_query: &FieldQuery,
        phrase_limit: i32,
    ) -> Self {
        let mut this = Self::default();
        let field = field_term_stack.get_field_name().to_string();

        let mut phrase_candidate: Vec<TermInfo> = Vec::new();
        while !field_term_stack.is_empty() && (this.phrase_list.len() as i64) < phrase_limit as i64
        {
            phrase_candidate.clear();

            let mut ti = field_term_stack
                .pop()
                .expect("INVARIANT: the stack is not empty");
            let first = ti.clone();
            let mut curr_map: Option<&QueryPhraseMap> =
                field_query.get_field_term_map(&field, ti.get_text());
            while curr_map.is_none() && !ti.get_next().same(&first) {
                ti = ti.get_next();
                curr_map = field_query.get_field_term_map(&field, ti.get_text());
            }

            // if not found, discard top TermInfo from stack, then try next
            // element
            let Some(mut curr) = curr_map else {
                continue;
            };

            // if found, search the longest phrase
            phrase_candidate.push(ti);
            loop {
                let popped = field_term_stack.pop();
                let mut next_map: Option<&QueryPhraseMap> = None;
                let mut ti_opt = popped.clone();
                if let Some(mut t) = popped {
                    let first = t.clone();
                    next_map = curr.get_term_map(t.get_text());
                    while next_map.is_none() && !t.get_next().same(&first) {
                        t = t.get_next();
                        next_map = curr.get_term_map(t.get_text());
                    }
                    ti_opt = Some(t);
                }
                match (ti_opt, next_map) {
                    (Some(t), Some(next)) => {
                        phrase_candidate.push(t);
                        curr = next;
                    }
                    (ti_opt, _) => {
                        if let Some(t) = ti_opt {
                            field_term_stack.push(t);
                        }
                        if curr.is_valid_term_or_phrase(&phrase_candidate) {
                            this.add_if_no_overlap(WeightedPhraseInfo::with_seqnum(
                                &phrase_candidate,
                                curr.get_boost(),
                                curr.get_term_or_phrase_number(),
                            ));
                        } else {
                            while phrase_candidate.len() > 1 {
                                field_term_stack.push(
                                    phrase_candidate.pop().expect("INVARIANT: more than one"),
                                );
                                if let Some(found) =
                                    field_query.search_phrase(&field, &phrase_candidate)
                                {
                                    this.add_if_no_overlap(WeightedPhraseInfo::with_seqnum(
                                        &phrase_candidate,
                                        found.get_boost(),
                                        found.get_term_or_phrase_number(),
                                    ));
                                    break;
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }
        this
    }

    /// Merging constructor.
    ///
    /// Equivalent to `new FieldPhraseList(FieldPhraseList[])`: `to_merge`
    /// holds the FieldPhraseLists to merge to build this one.
    pub fn from_merge(to_merge: &[FieldPhraseList]) -> Self {
        // Merge all overlapping WeightedPhraseInfos
        // Step 1.  Sort by startOffset, endOffset, and boost, in that order.
        let all_infos: Vec<&[WeightedPhraseInfo]> = to_merge
            .iter()
            .map(|fpl| fpl.phrase_list.as_slice())
            .collect();
        let itr = merged(&all_infos, WeightedPhraseInfo::compare_to);
        // Step 2.  Walk the sorted list merging infos that overlap
        let mut phrase_list = Vec::new();
        let mut itr = itr.into_iter();
        let Some(first) = itr.next() else {
            return Self { phrase_list };
        };
        let mut work: Vec<WeightedPhraseInfo> = Vec::new();
        let mut work_end_offset = first.get_end_offset();
        work.push(first);
        for current in itr {
            if current.get_start_offset() <= work_end_offset {
                work_end_offset = work_end_offset.max(current.get_end_offset());
                work.push(current);
            } else {
                let current_end = current.get_end_offset();
                if work.len() == 1 {
                    phrase_list.push(std::mem::replace(&mut work[0], current));
                } else {
                    phrase_list.push(
                        WeightedPhraseInfo::from_merge(&work)
                            .expect("INVARIANT: the work list is not empty"),
                    );
                    work.clear();
                    work.push(current);
                }
                work_end_offset = current_end;
            }
        }
        if work.len() == 1 {
            phrase_list.push(work.pop().expect("INVARIANT: one element"));
        } else {
            phrase_list.push(
                WeightedPhraseInfo::from_merge(&work)
                    .expect("INVARIANT: the work list is not empty"),
            );
            work.clear();
        }
        Self { phrase_list }
    }

    /// Equivalent to `addIfNoOverlap(WeightedPhraseInfo)`.
    pub fn add_if_no_overlap(&mut self, wpi: WeightedPhraseInfo) {
        for exist_wpi in self.phrase_list.iter_mut() {
            if exist_wpi.is_offset_overlap(&wpi) {
                // WeightedPhraseInfo.addIfNoOverlap() dumps the second part
                // of, for example, hyphenated words (social-economics). The
                // result is that all informations in TermInfo are lost and
                // not available for further operations.
                exist_wpi
                    .terms_infos
                    .extend(wpi.terms_infos.iter().cloned());
                exist_wpi.boost += wpi.get_boost(); // Accumulate boost
                return;
            }
        }
        self.phrase_list.push(wpi);
    }
}

/// Represents the list of term offsets and boost for some text.
///
/// Equivalent to `org.apache.lucene.search.vectorhighlight.FieldPhraseList.WeightedPhraseInfo`.
#[derive(Debug, Clone)]
pub struct WeightedPhraseInfo {
    terms_offsets: Vec<Toffs>, // usually termsOffsets.size() == 1,
    // but if position-gap > 1 and slop > 0 then size() could be greater
    // than 1
    pub(crate) boost: f32, // query boost
    seqnum: i32,

    pub(crate) terms_infos: Vec<TermInfo>,
}

impl WeightedPhraseInfo {
    /// Text of the match, calculated on the fly. Use for debugging only.
    ///
    /// Equivalent to `getText()`.
    pub fn get_text(&self) -> String {
        let mut text = String::new();
        for ti in &self.terms_infos {
            text.push_str(ti.get_text());
        }
        text
    }

    /// Equivalent to `getTermsOffsets()`: the termsOffsets.
    pub fn get_terms_offsets(&self) -> &Vec<Toffs> {
        &self.terms_offsets
    }

    /// Equivalent to `getBoost()`.
    pub fn get_boost(&self) -> f32 {
        self.boost
    }

    /// Equivalent to `getTermsInfos()`.
    pub fn get_terms_infos(&self) -> &Vec<TermInfo> {
        &self.terms_infos
    }

    /// `getTermsInfos()` for a caller that changes the list Java returns by
    /// reference.
    pub fn get_terms_infos_mut(&mut self) -> &mut Vec<TermInfo> {
        &mut self.terms_infos
    }

    /// Equivalent to `new WeightedPhraseInfo(LinkedList, float)`.
    pub fn new(terms: &[TermInfo], boost: f32) -> Self {
        Self::with_seqnum(terms, boost, 0)
    }

    /// Equivalent to `new WeightedPhraseInfo(LinkedList, float, int)`.
    ///
    /// # Panics
    ///
    /// When `terms` is empty, where Java throws `IndexOutOfBoundsException`.
    pub fn with_seqnum(terms: &[TermInfo], boost: f32, seqnum: i32) -> Self {
        let terms_infos = terms.to_vec();
        let mut terms_offsets: Vec<Toffs> = Vec::with_capacity(terms.len());
        let mut ti = &terms[0];

        terms_offsets.push(Toffs::new(ti.get_start_offset(), ti.get_end_offset()));
        if terms.len() > 1 {
            let mut pos = ti.get_position();
            for next in &terms[1..] {
                ti = next;
                if ti.get_position() - pos == 1 {
                    let to = terms_offsets.last_mut().expect("INVARIANT: not empty");
                    to.set_end_offset(ti.get_end_offset());
                } else {
                    terms_offsets.push(Toffs::new(ti.get_start_offset(), ti.get_end_offset()));
                }
                pos = ti.get_position();
            }
        }
        Self {
            terms_offsets,
            boost,
            seqnum,
            terms_infos,
        }
    }

    /// Merging constructor. Note that this just grabs seqnum from the first
    /// info.
    ///
    /// Equivalent to `new WeightedPhraseInfo(Collection)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when `to_merge` is empty.
    pub fn from_merge(to_merge: &[WeightedPhraseInfo]) -> Result<Self> {
        // Pretty much the same idea as merging FieldPhraseLists:
        // Step 1.  Sort by startOffset, endOffset
        //          While we are here merge the boosts and termInfos
        let Some(first) = to_merge.first() else {
            return Err(LuceneError::illegal_argument(
                "toMerge must contain at least one WeightedPhraseInfo.",
            ));
        };
        let mut terms_infos = Vec::new();
        let seqnum = first.seqnum;
        let mut boost = first.boost;
        let mut all_toffs: Vec<&[Toffs]> = vec![first.terms_offsets.as_slice()];
        for info in &to_merge[1..] {
            boost += info.boost;
            terms_infos.extend(info.terms_infos.iter().cloned());
            all_toffs.push(info.terms_offsets.as_slice());
        }
        let itr = merged(&all_toffs, Toffs::compare_to);
        // Step 2.  Walk the sorted list merging overlaps
        let mut terms_offsets = Vec::new();
        let mut itr = itr.into_iter();
        if let Some(mut work) = itr.next() {
            for current in itr {
                if current.start_offset <= work.end_offset {
                    work.end_offset = work.end_offset.max(current.end_offset);
                } else {
                    terms_offsets.push(work);
                    work = current;
                }
            }
            terms_offsets.push(work);
        }
        Ok(Self {
            terms_offsets,
            boost,
            seqnum,
            terms_infos,
        })
    }

    /// Equivalent to `getStartOffset()`.
    pub fn get_start_offset(&self) -> i32 {
        self.terms_offsets[0].start_offset
    }

    /// Equivalent to `getEndOffset()`.
    pub fn get_end_offset(&self) -> i32 {
        self.terms_offsets[self.terms_offsets.len() - 1].end_offset
    }

    /// Equivalent to `isOffsetOverlap(WeightedPhraseInfo)`.
    pub fn is_offset_overlap(&self, other: &WeightedPhraseInfo) -> bool {
        let so = self.get_start_offset();
        let eo = self.get_end_offset();
        let oso = other.get_start_offset();
        let oeo = other.get_end_offset();
        if so <= oso && oso < eo {
            return true;
        }
        if so < oeo && oeo <= eo {
            return true;
        }
        if oso <= so && so < oeo {
            return true;
        }
        if oso < eo && eo <= oeo {
            return true;
        }
        false
    }

    /// Equivalent to `getSeqnum()`: the seqnum.
    pub fn get_seqnum(&self) -> i32 {
        self.seqnum
    }

    /// Equivalent to `compareTo(WeightedPhraseInfo)`.
    pub fn compare_to(&self, other: &WeightedPhraseInfo) -> i32 {
        let diff = self
            .get_start_offset()
            .wrapping_sub(other.get_start_offset());
        if diff != 0 {
            return diff;
        }
        let diff = self.get_end_offset().wrapping_sub(other.get_end_offset());
        if diff != 0 {
            return diff;
        }
        let d = self.get_boost() - other.get_boost();
        if d > 0.0 {
            1
        } else if d < 0.0 {
            -1
        } else {
            0
        }
    }

    /// Equivalent to `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        let prime = 31i32;
        let mut result = 1i32;
        result = prime
            .wrapping_mul(result)
            .wrapping_add(self.get_start_offset());
        result = prime
            .wrapping_mul(result)
            .wrapping_add(self.get_end_offset());
        let b = java_double_to_long_bits(self.get_boost() as f64);
        prime
            .wrapping_mul(result)
            .wrapping_add((b ^ ((b as u64) >> 32) as i64) as i32)
    }
}

/// `Double.doubleToLongBits(d)`.
fn java_double_to_long_bits(d: f64) -> i64 {
    if d.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        d.to_bits() as i64
    }
}

impl fmt::Display for WeightedPhraseInfo {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({})(", self.get_text(), float_to_string(self.boost))?;
        for to in &self.terms_offsets {
            write!(f, "{to}")?;
        }
        f.write_str(")")
    }
}

impl PartialEq for WeightedPhraseInfo {
    /// Equivalent to `equals(Object)`: equal offsets and boost.
    fn eq(&self, other: &Self) -> bool {
        if self.get_start_offset() != other.get_start_offset() {
            return false;
        }
        if self.get_end_offset() != other.get_end_offset() {
            return false;
        }
        if self.get_boost() != other.get_boost() {
            return false;
        }
        true
    }
}

impl Hash for WeightedPhraseInfo {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash_code().hash(state);
    }
}

/// Term offsets (start + end).
///
/// Equivalent to `org.apache.lucene.search.vectorhighlight.FieldPhraseList.WeightedPhraseInfo.Toffs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Toffs {
    pub(crate) start_offset: i32,
    pub(crate) end_offset: i32,
}

impl Toffs {
    /// Equivalent to `new Toffs(int, int)`.
    pub fn new(start_offset: i32, end_offset: i32) -> Self {
        Self {
            start_offset,
            end_offset,
        }
    }

    /// Equivalent to `setEndOffset(int)`.
    pub fn set_end_offset(&mut self, end_offset: i32) {
        self.end_offset = end_offset;
    }

    /// Equivalent to `getStartOffset()`.
    pub fn get_start_offset(&self) -> i32 {
        self.start_offset
    }

    /// Equivalent to `getEndOffset()`.
    pub fn get_end_offset(&self) -> i32 {
        self.end_offset
    }

    /// Equivalent to `compareTo(Toffs)`.
    pub fn compare_to(&self, other: &Toffs) -> i32 {
        let diff = self
            .get_start_offset()
            .wrapping_sub(other.get_start_offset());
        if diff != 0 {
            return diff;
        }
        self.get_end_offset().wrapping_sub(other.get_end_offset())
    }

    /// Equivalent to `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        let prime = 31i32;
        let mut result = 1i32;
        result = prime
            .wrapping_mul(result)
            .wrapping_add(self.get_start_offset());
        prime
            .wrapping_mul(result)
            .wrapping_add(self.get_end_offset())
    }
}

impl fmt::Display for Toffs {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({},{})", self.start_offset, self.end_offset)
    }
}
