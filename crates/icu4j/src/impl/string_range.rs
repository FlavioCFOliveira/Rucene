// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.StringRange`.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use rucene::error::Result;

use crate::jdk::{append_code_point, char_count, code_point_at, code_point_before};
use crate::lang::char_sequences::CharSequences;
use crate::util::icu_exception::ICUException;

/// `StringRange.Adder`: receives the compacted ranges (`end` `None` for a
/// single string; `start` `None` only for an empty source, as in Java).
pub trait Adder {
    /// `add(String start, String end)`.
    fn add(&mut self, start: Option<&[u16]>, end: Option<&[u16]>);
}

/// Compacting and expanding sets of strings into ranges `start~end`.
///
/// Equivalent to `com.ibm.icu.impl.StringRange`.
pub struct StringRange;

/// `COMPARE_INT_ARRAYS`: element-wise, then by length.
pub fn compare_int_arrays(o1: &[i32], o2: &[i32]) -> i32 {
    let min_index = o1.len().min(o2.len());
    for i in 0..min_index {
        let diff = o1[i].wrapping_sub(o2[i]);
        if diff != 0 {
            return diff;
        }
    }
    o1.len() as i32 - o2.len() as i32
}

impl StringRange {
    /// `compact(Set<String> source, Adder adder, boolean shorterPairs,
    /// boolean moreCompact)`: feeds `adder` the ranges of `source`, in its
    /// iteration order.
    ///
    /// The `moreCompact` branch groups the strings by length in Java's
    /// `Relation.of(new TreeMap<Integer, Set<Ranges>>(), TreeSet.class)`;
    /// here that multimap is the equivalent `BTreeMap<i32, BTreeSet<Ranges>>`.
    pub fn compact(source: &[Vec<u16>], adder: &mut dyn Adder, shorter_pairs: bool, more_compact: bool) {
        if !more_compact {
            let mut start: Option<&[u16]> = None;
            let mut end: Option<&[u16]> = None;
            let mut last_cp = 0;
            let mut prefix_len = 0usize;
            for s in source {
                if let Some(st) = start {
                    // We have something queued up
                    if s.len() >= prefix_len && st.len() >= prefix_len && s[..prefix_len] == st[..prefix_len] {
                        let current_cp = code_point_at(s, prefix_len as i32);
                        if current_cp == 1 + last_cp && s.len() == prefix_len + char_count(current_cp) as usize {
                            end = Some(s);
                            last_cp = current_cp;
                            continue;
                        }
                    }
                    // We failed to find continuation. Add what we have and restart
                    let e = end.map(|e| if !shorter_pairs { e } else { &e[prefix_len..] });
                    adder.add(Some(st), e);
                }
                // new possible range
                start = Some(s);
                end = None;
                last_cp = code_point_before(s, s.len() as i32);
                prefix_len = s.len() - char_count(last_cp) as usize;
            }
            let e = end.map(|e| if !shorter_pairs { e } else { &e[prefix_len..] });
            adder.add(start, e);
        } else {
            // not a fast algorithm, but ok for now
            // TODO rewire to use the first (slower) algorithm to generate the ranges, then compact them from there.
            // first sort by lengths
            let mut length_to_arrays: BTreeMap<i32, BTreeSet<Ranges>> = BTreeMap::new();
            for s in source {
                let item = Ranges::new(s);
                length_to_arrays.entry(item.size()).or_default().insert(item);
            }
            // then compact items of each length and emit compacted sets
            for (key, value) in length_to_arrays {
                let compacted = Self::compact_ranges(key, value);
                for ranges in &compacted {
                    adder.add(Some(&ranges.start()), ranges.end(shorter_pairs).as_deref());
                }
            }
        }
    }

    /// `compact(Set<String> source, Adder adder, boolean shorterPairs)`.
    pub fn compact3(source: &[Vec<u16>], adder: &mut dyn Adder, shorter_pairs: bool) {
        Self::compact(source, adder, shorter_pairs, false);
    }

    fn compact_ranges(size: i32, input_ranges: BTreeSet<Ranges>) -> Vec<Ranges> {
        let mut ranges: Vec<Ranges> = input_ranges.into_iter().collect();
        let mut i = size - 1;
        while i >= 0 {
            let mut out: Vec<Ranges> = Vec::with_capacity(ranges.len());
            for item in ranges {
                match out.last_mut() {
                    None => out.push(item),
                    Some(last) => {
                        if !last.merge(i, &item) {
                            out.push(item);
                        }
                    }
                }
            }
            ranges = out;
            i -= 1;
        }
        ranges
    }

    /// `expand(String start, String end, boolean requireSameLength,
    /// Collection<String> output)`: adds every string of the range to
    /// `output`.
    ///
    /// # Errors
    ///
    /// `ICUException` with ICU's messages for an invalid range.
    pub fn expand(start: Option<&[u16]>, end: Option<&[u16]>, require_same_length: bool, output: &mut BTreeSet<Vec<u16>>) -> Result<()> {
        let (Some(start), Some(end)) = (start, end) else {
            return Err(ICUException::with_message("Range must have 2 valid strings").error());
        };
        let start_cps = CharSequences::code_points(start);
        let end_cps = CharSequences::code_points(end);
        let start_offset = start_cps.len() as i32 - end_cps.len() as i32;

        if require_same_length && start_offset != 0 {
            return Err(ICUException::with_message("Range must have equal-length strings").error());
        } else if start_offset < 0 {
            return Err(ICUException::with_message("Range must have start-length ≥ end-length").error());
        } else if end_cps.is_empty() {
            return Err(ICUException::with_message("Range must have end-length > 0").error());
        }

        let mut builder: Vec<u16> = Vec::new();
        for &cp in &start_cps[..start_offset as usize] {
            append_code_point(&mut builder, cp)?;
        }
        Self::add(0, start_offset, &start_cps, &end_cps, &mut builder, output)
    }

    fn add(end_index: i32, start_offset: i32, starts: &[i32], ends: &[i32], builder: &mut Vec<u16>, output: &mut BTreeSet<Vec<u16>>) -> Result<()> {
        let start = starts[(end_index + start_offset) as usize];
        let end = ends[end_index as usize];
        if start > end {
            return Err(ICUException::with_message("Range must have xᵢ ≤ yᵢ for each index i").error());
        }
        let last = end_index == ends.len() as i32 - 1;
        let start_len = builder.len();
        for i in start..=end {
            append_code_point(builder, i)?;
            if last {
                output.insert(builder.clone());
            } else {
                Self::add(end_index + 1, start_offset, starts, ends, builder, output)?;
            }
            builder.truncate(start_len);
        }
        Ok(())
    }
}

/// `StringRange.Range` (package-private): a code point range.
#[derive(Debug, Clone, Copy)]
struct Range {
    min: i32,
    max: i32,
}

impl Range {
    fn compare_to(&self, that: &Range) -> i32 {
        let diff = self.min - that.min;
        if diff != 0 {
            return diff;
        }
        self.max - that.max
    }
}

impl PartialEq for Range {
    fn eq(&self, other: &Self) -> bool {
        self.compare_to(other) == 0
    }
}

impl Range {
    /// `hashCode()`.
    #[allow(dead_code)]
    fn hash_code(&self) -> i32 {
        self.min.wrapping_mul(37).wrapping_add(self.max)
    }

    /// `toString()`.
    #[allow(dead_code)]
    fn to_units(&self) -> Vec<u16> {
        let mut result = Vec::new();
        let _ = append_code_point(&mut result, self.min);
        if self.min != self.max {
            result.push(b'~' as u16);
            let _ = append_code_point(&mut result, self.max);
        }
        result
    }
}

/// `StringRange.Ranges` (package-private): one range per code point.
#[derive(Debug, Clone)]
struct Ranges {
    ranges: Vec<Range>,
}

impl Ranges {
    fn new(s: &[u16]) -> Self {
        let array = CharSequences::code_points(s);
        Self { ranges: array.iter().map(|&c| Range { min: c, max: c }).collect() }
    }

    fn merge(&mut self, pivot: i32, other: &Ranges) -> bool {
        // We merge items if the pivot is adjacent, and all later ranges are equal.
        let mut i = self.ranges.len() as i32 - 1;
        while i >= 0 {
            let iu = i as usize;
            if i == pivot {
                if self.ranges[iu].max != other.ranges[iu].min - 1 {
                    // not adjacent
                    return false;
                }
            } else if self.ranges[iu] != other.ranges[iu] {
                return false;
            }
            i -= 1;
        }
        self.ranges[pivot as usize].max = other.ranges[pivot as usize].max;
        true
    }

    fn start(&self) -> Vec<u16> {
        let mut result = Vec::new();
        for r in &self.ranges {
            let _ = append_code_point(&mut result, r.min);
        }
        result
    }

    fn end(&self, most_compact: bool) -> Option<Vec<u16>> {
        let first_diff = self.first_difference();
        if first_diff == self.ranges.len() {
            return None;
        }
        let mut result = Vec::new();
        let from = if most_compact { first_diff } else { 0 };
        for r in &self.ranges[from..] {
            let _ = append_code_point(&mut result, r.max);
        }
        Some(result)
    }

    fn first_difference(&self) -> usize {
        for (i, r) in self.ranges.iter().enumerate() {
            if r.min != r.max {
                return i;
            }
        }
        self.ranges.len()
    }

    fn size(&self) -> i32 {
        self.ranges.len() as i32
    }

    fn compare_to(&self, other: &Ranges) -> i32 {
        let diff = self.ranges.len() as i32 - other.ranges.len() as i32;
        if diff != 0 {
            return diff;
        }
        for i in 0..self.ranges.len() {
            let diff = self.ranges[i].compare_to(&other.ranges[i]);
            if diff != 0 {
                return diff;
            }
        }
        0
    }
}

impl PartialEq for Ranges {
    fn eq(&self, other: &Self) -> bool {
        self.compare_to(other) == 0
    }
}
impl Eq for Ranges {}
impl PartialOrd for Ranges {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Ranges {
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare_to(other).cmp(&0)
    }
}
