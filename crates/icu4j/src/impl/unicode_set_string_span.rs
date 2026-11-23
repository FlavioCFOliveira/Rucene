// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.UnicodeSetStringSpan`.

use std::sync::{Mutex, MutexGuard};

use crate::jdk::{code_point_at, code_point_before, is_high_surrogate, is_low_surrogate, offset_by_code_points, to_code_point};
use crate::text::unicode_set::{SpanCondition, UnicodeSet};
use crate::text::utf16::UTF16;
use crate::util::output_int::OutputInt;

/// `WITH_COUNT`.
pub const WITH_COUNT: i32 = 0x40;
/// `FWD`.
pub const FWD: i32 = 0x20;
/// `BACK`.
pub const BACK: i32 = 0x10;
/// `CONTAINED`.
pub const CONTAINED: i32 = 2;
/// `NOT_CONTAINED`.
pub const NOT_CONTAINED: i32 = 1;
/// `ALL`.
pub const ALL: i32 = 0x7f;
/// `FWD_UTF16_CONTAINED`.
pub const FWD_UTF16_CONTAINED: i32 = FWD | CONTAINED;
/// `FWD_UTF16_NOT_CONTAINED`.
pub const FWD_UTF16_NOT_CONTAINED: i32 = FWD | NOT_CONTAINED;
/// `BACK_UTF16_CONTAINED`.
pub const BACK_UTF16_CONTAINED: i32 = BACK | CONTAINED;
/// `BACK_UTF16_NOT_CONTAINED`.
pub const BACK_UTF16_NOT_CONTAINED: i32 = BACK | NOT_CONTAINED;

/// Special spanLength short values. (since Java has not unsigned byte type)
/// All code points in the string are contained in the parent set.
const ALL_CP_CONTAINED: i16 = 0xff;
/// The spanLength is >=0xfe.
const LONG_SPAN: i16 = ALL_CP_CONTAINED - 1;

/// `spanNotSet`: none, the same object as `spanSet`, or its own set.
#[derive(Debug, Clone)]
enum SpanNotSet {
    None,
    SameAsSpanSet,
    Own(UnicodeSet),
}

/// Implement span() etc. for a set with strings.
///
/// Equivalent to `com.ibm.icu.impl.UnicodeSetStringSpan`. Its
/// `synchronized` methods guard the shared `offsets` list; here that list
/// is behind a `Mutex`, so the span methods take `&self`.
#[derive(Debug)]
pub struct UnicodeSetStringSpan {
    /// Set of code points contained in all the strings, or the parent set
    /// when the span is ALL (frozen).
    span_set: UnicodeSet,
    /// Set of characters which may start or end a string (null or spanSet
    /// when unused).
    span_not_set: SpanNotSet,
    /// The strings of the parent set.
    strings: Vec<Vec<u16>>,
    /// The lengths of span(), spanBack() etc. for each string.
    span_lengths: Vec<i16>,
    /// Maximum lengths of relevant strings.
    max_length16: i32,
    /// Are there strings that are not fully contained in the code point set?
    some_relevant: bool,
    /// Set up for all variants of span()?
    all: bool,
    /// Memory for the offsets list.
    offsets: Mutex<OffsetList>,
}

impl Clone for UnicodeSetStringSpan {
    fn clone(&self) -> Self {
        Self {
            span_set: self.span_set.clone(),
            span_not_set: self.span_not_set.clone(),
            strings: self.strings.clone(),
            span_lengths: self.span_lengths.clone(),
            max_length16: self.max_length16,
            some_relevant: self.some_relevant,
            all: self.all,
            offsets: Mutex::new(OffsetList::new()),
        }
    }
}

impl UnicodeSetStringSpan {
    /// `UnicodeSetStringSpan(UnicodeSet set, ArrayList<String> setStrings,
    /// int which)`: removes the empty strings of `set_strings`.
    pub fn new(set: &UnicodeSet, set_strings: Vec<Vec<u16>>, which: i32) -> Self {
        let mut span_set = UnicodeSet::new_range_unchecked(0, 0x10ffff);
        // TODO: With Java 6, just take the parent set's strings as is,
        // as a NavigableSet<String>, rather than as an ArrayList copy of the set of strings.
        // Then iterate via the first() and higher() methods.
        // (We do not want to create multiple Iterator objects in each span().)
        // See ICU ticket #7454.
        let mut strings = set_strings;
        let all = which == ALL;
        span_set.retain_all_unchecked(set);
        let mut span_not_set = SpanNotSet::None;
        if 0 != (which & NOT_CONTAINED) {
            // Default to the same sets.
            // addToSpanNotSet() will create a separate set if necessary.
            span_not_set = SpanNotSet::SameAsSpanSet;
        }

        // Determine if the strings even need to be taken into account at all for span() etc.
        // If any string is relevant, then all strings need to be used for
        // span(longest match) but only the relevant ones for span(while contained).
        // TODO: Possible optimization: Distinguish CONTAINED vs. LONGEST_MATCH
        // and do not store UTF-8 strings if !thisRelevant and CONTAINED.
        // (Only store irrelevant UTF-8 strings for LONGEST_MATCH where they are relevant after all.)
        // Also count the lengths of the UTF-8 versions of the strings for memory allocation.
        let mut strings_length = strings.len();

        let mut max_length16 = 0;
        let mut some_relevant = false;
        let mut i = 0usize;
        while i < strings_length {
            let length16 = strings[i].len() as i32;
            if length16 == 0 {
                // Remove the empty string.
                strings.remove(i);
                strings_length -= 1;
                continue;
            }
            let span_length = span_set.span(&strings[i], 0, SpanCondition::Contained);
            if span_length < length16 {
                // Relevant string.
                some_relevant = true;
            }
            if length16 > max_length16 {
                max_length16 = length16;
            }
            i += 1;
        }
        let mut this = Self {
            span_set,
            span_not_set,
            strings,
            span_lengths: Vec::new(),
            max_length16,
            some_relevant,
            all,
            offsets: Mutex::new(OffsetList::new()),
        };
        if !some_relevant && (which & WITH_COUNT) == 0 {
            return this;
        }

        // Freeze after checking for the need to use strings at all because freezing
        // a set takes some time and memory which are wasted if there are no relevant strings.
        if all {
            this.span_set.freeze();
        }

        let span_back_lengths_offset;
        let alloc_size = if all {
            // Store span lengths for all span() variants.
            strings_length * 2
        } else {
            // Store span lengths for only one span() variant.
            strings_length
        };
        this.span_lengths = vec![0i16; alloc_size];

        if all {
            // Store span lengths for all span() variants.
            span_back_lengths_offset = strings_length;
        } else {
            // Store span lengths for only one span() variant.
            span_back_lengths_offset = 0;
        }

        // Set the meta data and spanNotSet and write the UTF-8 strings.

        for i in 0..strings_length {
            let string = this.strings[i].clone();
            let length16 = string.len() as i32;
            let mut span_length = this.span_set.span(&string, 0, SpanCondition::Contained);
            if span_length < length16 {
                // Relevant string.
                if 0 != (which & CONTAINED) {
                    if 0 != (which & FWD) {
                        this.span_lengths[i] = make_span_length_byte(span_length);
                    }
                    if 0 != (which & BACK) {
                        span_length = length16 - this.span_set.span_back(&string, length16, SpanCondition::Contained);
                        this.span_lengths[span_back_lengths_offset + i] = make_span_length_byte(span_length);
                    }
                } else
                /* not CONTAINED, not all, but NOT_CONTAINED */
                {
                    this.span_lengths[i] = 0;
                    this.span_lengths[span_back_lengths_offset + i] = 0; // Only store a relevant/irrelevant flag.
                }
                if 0 != (which & NOT_CONTAINED) {
                    // Add string start and end code points to the spanNotSet so that
                    // a span(while not contained) stops before any string.
                    if 0 != (which & FWD) {
                        let c = code_point_at(&string, 0);
                        this.add_to_span_not_set(c);
                    }
                    if 0 != (which & BACK) {
                        let c = code_point_before(&string, length16);
                        this.add_to_span_not_set(c);
                    }
                }
            } else {
                // Irrelevant string.
                if all {
                    this.span_lengths[i] = ALL_CP_CONTAINED;
                    this.span_lengths[span_back_lengths_offset + i] = ALL_CP_CONTAINED;
                } else {
                    // All spanXYZLengths pointers contain the same address.
                    this.span_lengths[i] = ALL_CP_CONTAINED;
                }
            }
        }

        // Finish.
        if all {
            if let SpanNotSet::Own(s) = &mut this.span_not_set {
                s.freeze();
            }
        }
        this
    }

    /// `UnicodeSetStringSpan(UnicodeSetStringSpan otherStringSpan,
    /// ArrayList<String> newParentSetStrings)`: a copy for a cloned parent.
    pub fn from_other(other_string_span: &UnicodeSetStringSpan, new_parent_set_strings: Vec<Vec<u16>>) -> Self {
        let span_not_set = match &other_string_span.span_not_set {
            SpanNotSet::SameAsSpanSet => SpanNotSet::SameAsSpanSet,
            SpanNotSet::Own(s) => SpanNotSet::Own(s.clone()),
            // Java clones a null spanNotSet and throws; with `all` set the
            // constructor always leaves it non-null.
            SpanNotSet::None => SpanNotSet::None,
        };
        Self {
            span_set: other_string_span.span_set.clone(),
            span_not_set,
            strings: new_parent_set_strings,
            span_lengths: other_string_span.span_lengths.clone(),
            max_length16: other_string_span.max_length16,
            some_relevant: other_string_span.some_relevant,
            all: true,
            offsets: Mutex::new(OffsetList::new()),
        }
    }

    /// `needsStringSpanUTF16()`: whether the strings must be used for spans.
    pub fn needs_string_span_utf16(&self) -> bool {
        self.some_relevant
    }

    /// `contains(int c)`: for code points, same as the parent set.
    pub fn contains(&self, c: i32) -> bool {
        self.span_set.contains_valid(c)
    }

    fn span_not_set(&self) -> &UnicodeSet {
        match &self.span_not_set {
            SpanNotSet::Own(s) => s,
            // Java dereferences a null spanNotSet (NullPointerException) only
            // for a set built without NOT_CONTAINED, which never spans so.
            _ => &self.span_set,
        }
    }

    /// Adds a starting or ending string character to the spanNotSet so that
    /// a character span ends before any string.
    fn add_to_span_not_set(&mut self, c: i32) {
        if !matches!(self.span_not_set, SpanNotSet::Own(_)) {
            if self.span_set.contains_valid(c) {
                return; // Nothing to do.
            }
            self.span_not_set = SpanNotSet::Own(self.span_set.clone_as_thawed());
        }
        if let SpanNotSet::Own(s) = &mut self.span_not_set {
            s.add_valid(c);
        }
    }

    fn offsets(&self) -> MutexGuard<'_, OffsetList> {
        self.offsets.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `span(CharSequence s, int start, SpanCondition spanCondition)`.
    pub fn span(&self, s: &[u16], start: i32, span_condition: SpanCondition) -> i32 {
        if span_condition == SpanCondition::NotContained {
            return self.span_not(s, start, None);
        }
        let span_limit = self.span_set.span(s, start, SpanCondition::Contained);
        if span_limit == s.len() as i32 {
            return span_limit;
        }
        self.span_with_strings(s, start, span_limit, span_condition)
    }

    /// Synchronized method for complicated spans using the offsets.
    /// Avoids synchronization for simple cases.
    fn span_with_strings(&self, s: &[u16], start: i32, mut span_limit: i32, span_condition: SpanCondition) -> i32 {
        let mut offsets = self.offsets();
        // Consider strings; they may overlap with the span,
        // and they may result in a longer match.
        let mut init_size = 0;
        if span_condition == SpanCondition::Contained {
            // Use offset list to try all possibilities.
            init_size = self.max_length16;
        }
        offsets.set_max_length(init_size);
        let length = s.len() as i32;
        let mut pos = span_limit;
        let mut rest = length - span_limit;
        let mut span_length = span_limit - start;
        let strings_length = self.strings.len();
        loop {
            if span_condition == SpanCondition::Contained {
                for i in 0..strings_length {
                    let mut overlap = self.span_lengths[i] as i32;
                    if overlap == ALL_CP_CONTAINED as i32 {
                        continue; // Irrelevant string.
                    }
                    let string = &self.strings[i];

                    let length16 = string.len() as i32;

                    // Try to match this string at pos-overlap..pos.
                    if overlap >= LONG_SPAN as i32 {
                        overlap = length16;
                        // While contained: No point matching fully inside the code point span.
                        overlap = offset_by_code_points(string, overlap, -1); // Length of the string minus the last code point.
                    }
                    if overlap > span_length {
                        overlap = span_length;
                    }
                    let mut inc = length16 - overlap; // Keep overlap+inc==length16.
                    loop {
                        if inc > rest {
                            break;
                        }
                        // Try to match if the increment is not listed already.
                        if !offsets.contains_offset(inc) && matches16_cpb(s, pos - overlap, length, string, length16) {
                            if inc == rest {
                                return length; // Reached the end of the string.
                            }
                            offsets.add_offset(inc);
                        }
                        if overlap == 0 {
                            break;
                        }
                        overlap -= 1;
                        inc += 1;
                    }
                }
            } else
            /* SIMPLE */
            {
                let mut max_inc = 0;
                let mut max_overlap = 0;
                for i in 0..strings_length {
                    let mut overlap = self.span_lengths[i] as i32;
                    // For longest match, we do need to try to match even an all-contained string
                    // to find the match from the earliest start.

                    let string = &self.strings[i];

                    let length16 = string.len() as i32;

                    // Try to match this string at pos-overlap..pos.
                    if overlap >= LONG_SPAN as i32 {
                        overlap = length16;
                        // Longest match: Need to match fully inside the code point span
                        // to find the match from the earliest start.
                    }
                    if overlap > span_length {
                        overlap = span_length;
                    }
                    let mut inc = length16 - overlap; // Keep overlap+inc==length16.
                    loop {
                        if inc > rest || overlap < max_overlap {
                            break;
                        }
                        // Try to match if the string is longer or starts earlier.
                        if (overlap > max_overlap || /* redundant overlap==maxOverlap && */ inc > max_inc)
                            && matches16_cpb(s, pos - overlap, length, string, length16)
                        {
                            max_inc = inc; // Longest match from earliest start.
                            max_overlap = overlap;
                            break;
                        }
                        overlap -= 1;
                        inc += 1;
                    }
                }

                if max_inc != 0 || max_overlap != 0 {
                    // Longest-match algorithm, and there was a string match.
                    // Simply continue after it.
                    pos += max_inc;
                    rest -= max_inc;
                    if rest == 0 {
                        return length; // Reached the end of the string.
                    }
                    span_length = 0; // Match strings from after a string match.
                    continue;
                }
            }
            // Finished trying to match all strings at pos.

            if span_length != 0 || pos == 0 {
                // The position is after an unlimited code point span (spanLength!=0),
                // not after a string match.
                // The only position where spanLength==0 after a span is pos==0.
                // Otherwise, an unlimited code point span is only tried again when no
                // strings match, and if such a non-initial span fails we stop.
                if offsets.is_empty() {
                    return pos; // No strings matched after a span.
                }
                // Match strings from after the next string match.
            } else {
                // The position is after a string match (or a single code point).
                if offsets.is_empty() {
                    // No more strings matched after a previous string match.
                    // Try another code point span from after the last string match.
                    span_limit = self.span_set.span(s, pos, SpanCondition::Contained);
                    span_length = span_limit - pos;
                    if span_length == rest || // Reached the end of the string, or
                            span_length == 0
                    // neither strings nor span progressed.
                    {
                        return span_limit;
                    }
                    pos += span_length;
                    rest -= span_length;
                    continue; // spanLength>0: Match strings from after a span.
                } else {
                    // Try to match only one code point from after a string match if some
                    // string matched beyond it, so that we try all possible positions
                    // and don't overshoot.
                    span_length = span_one(&self.span_set, s, pos, rest);
                    if span_length > 0 {
                        if span_length == rest {
                            return length; // Reached the end of the string.
                        }
                        // Match strings after this code point.
                        // There cannot be any increments below it because UnicodeSet strings
                        // contain multiple code points.
                        pos += span_length;
                        rest -= span_length;
                        offsets.shift(span_length);
                        span_length = 0;
                        continue; // Match strings from after a single code point.
                    }
                    // Match strings from after the next string match.
                }
            }
            let min_offset = offsets.pop_minimum(None);
            pos += min_offset;
            rest -= min_offset;
            span_length = 0; // Match strings from after a string match.
        }
    }

    /// `spanAndCount(CharSequence s, int start, SpanCondition spanCondition,
    /// OutputInt outCount)`: as `span`, counting code points and strings.
    pub fn span_and_count(&self, s: &[u16], start: i32, span_condition: SpanCondition, out_count: &mut OutputInt) -> i32 {
        if span_condition == SpanCondition::NotContained {
            return self.span_not(s, start, Some(out_count));
        }
        // Consider strings; they may overlap with the span,
        // and they may result in a longer match.
        if span_condition == SpanCondition::Contained {
            return self.span_contained_and_count(s, start, out_count);
        }
        // SIMPLE (not synchronized, does not use offsets)
        let strings_length = self.strings.len();
        let length = s.len() as i32;
        let mut pos = start;
        let mut rest = length - start;
        let mut count = 0;
        while rest != 0 {
            // Try to match the next code point.
            let cp_length = span_one(&self.span_set, s, pos, rest);
            let mut max_inc = if cp_length > 0 { cp_length } else { 0 };
            // Try to match all of the strings.
            for i in 0..strings_length {
                let string = &self.strings[i];
                let length16 = string.len() as i32;
                if max_inc < length16 && length16 <= rest && matches16_cpb(s, pos, length, string, length16) {
                    max_inc = length16;
                }
            }
            // We are done if there is no match beyond pos.
            if max_inc == 0 {
                out_count.value = count;
                return pos;
            }
            // Continue from the longest match.
            count += 1;
            pos += max_inc;
            rest -= max_inc;
        }
        out_count.value = count;
        pos
    }

    fn span_contained_and_count(&self, s: &[u16], start: i32, out_count: &mut OutputInt) -> i32 {
        let mut offsets = self.offsets();
        // Use offset list to try all possibilities.
        offsets.set_max_length(self.max_length16);
        let strings_length = self.strings.len();
        let length = s.len() as i32;
        let mut pos = start;
        let mut rest = length - start;
        let mut count = 0;
        while rest != 0 {
            // Try to match the next code point.
            let cp_length = span_one(&self.span_set, s, pos, rest);
            if cp_length > 0 {
                offsets.add_offset_and_count(cp_length, count + 1);
            }
            // Try to match all of the strings.
            for i in 0..strings_length {
                let string = &self.strings[i];
                let length16 = string.len() as i32;
                // Note: If the strings were sorted by length, then we could also
                // avoid trying to match if there is already a match of the same length.
                if length16 <= rest
                    && !offsets.has_count_at_offset(length16, count + 1)
                    && matches16_cpb(s, pos, length, string, length16)
                {
                    offsets.add_offset_and_count(length16, count + 1);
                }
            }
            // We are done if there is no match beyond pos.
            if offsets.is_empty() {
                out_count.value = count;
                return pos;
            }
            // Continue from the nearest match.
            let min_offset = offsets.pop_minimum(Some(out_count));
            count = out_count.value;
            pos += min_offset;
            rest -= min_offset;
        }
        out_count.value = count;
        pos
    }

    /// `spanBack(CharSequence s, int length, SpanCondition spanCondition)`.
    pub fn span_back(&self, s: &[u16], length: i32, span_condition: SpanCondition) -> i32 {
        let mut offsets = self.offsets();
        if span_condition == SpanCondition::NotContained {
            drop(offsets);
            return self.span_not_back(s, length);
        }
        let mut pos = self.span_set.span_back(s, length, SpanCondition::Contained);
        if pos == 0 {
            return 0;
        }
        let mut span_length = length - pos;

        // Consider strings; they may overlap with the span,
        // and they may result in a longer match.
        let mut init_size = 0;
        if span_condition == SpanCondition::Contained {
            // Use offset list to try all possibilities.
            init_size = self.max_length16;
        }
        offsets.set_max_length(init_size);
        let strings_length = self.strings.len();
        let mut span_back_lengths_offset = 0;
        if self.all {
            span_back_lengths_offset = strings_length;
        }
        loop {
            if span_condition == SpanCondition::Contained {
                for i in 0..strings_length {
                    let mut overlap = self.span_lengths[span_back_lengths_offset + i] as i32;
                    if overlap == ALL_CP_CONTAINED as i32 {
                        continue; // Irrelevant string.
                    }
                    let string = &self.strings[i];

                    let length16 = string.len() as i32;

                    // Try to match this string at pos-(length16-overlap)..pos-length16.
                    if overlap >= LONG_SPAN as i32 {
                        overlap = length16;
                        // While contained: No point matching fully inside the code point span.
                        let len1 = offset_by_code_points(string, 0, 1);
                        overlap -= len1; // Length of the string minus the first code point.
                    }
                    if overlap > span_length {
                        overlap = span_length;
                    }
                    let mut dec = length16 - overlap; // Keep dec+overlap==length16.
                    loop {
                        if dec > pos {
                            break;
                        }
                        // Try to match if the decrement is not listed already.
                        if !offsets.contains_offset(dec) && matches16_cpb(s, pos - dec, length, string, length16) {
                            if dec == pos {
                                return 0; // Reached the start of the string.
                            }
                            offsets.add_offset(dec);
                        }
                        if overlap == 0 {
                            break;
                        }
                        overlap -= 1;
                        dec += 1;
                    }
                }
            } else
            /* SIMPLE */
            {
                let mut max_dec = 0;
                let mut max_overlap = 0;
                for i in 0..strings_length {
                    let mut overlap = self.span_lengths[span_back_lengths_offset + i] as i32;
                    // For longest match, we do need to try to match even an all-contained string
                    // to find the match from the latest end.

                    let string = &self.strings[i];

                    let length16 = string.len() as i32;

                    // Try to match this string at pos-(length16-overlap)..pos-length16.
                    if overlap >= LONG_SPAN as i32 {
                        overlap = length16;
                        // Longest match: Need to match fully inside the code point span
                        // to find the match from the latest end.
                    }
                    if overlap > span_length {
                        overlap = span_length;
                    }
                    let mut dec = length16 - overlap; // Keep dec+overlap==length16.
                    loop {
                        if dec > pos || overlap < max_overlap {
                            break;
                        }
                        // Try to match if the string is longer or ends later.
                        if (overlap > max_overlap || /* redundant overlap==maxOverlap && */ dec > max_dec)
                            && matches16_cpb(s, pos - dec, length, string, length16)
                        {
                            max_dec = dec; // Longest match from latest end.
                            max_overlap = overlap;
                            break;
                        }
                        overlap -= 1;
                        dec += 1;
                    }
                }

                if max_dec != 0 || max_overlap != 0 {
                    // Longest-match algorithm, and there was a string match.
                    // Simply continue before it.
                    pos -= max_dec;
                    if pos == 0 {
                        return 0; // Reached the start of the string.
                    }
                    span_length = 0; // Match strings from before a string match.
                    continue;
                }
            }
            // Finished trying to match all strings at pos.

            if span_length != 0 || pos == length {
                // The position is before an unlimited code point span (spanLength!=0),
                // not before a string match.
                // The only position where spanLength==0 before a span is pos==length.
                // Otherwise, an unlimited code point span is only tried again when no
                // strings match, and if such a non-initial span fails we stop.
                if offsets.is_empty() {
                    return pos; // No strings matched before a span.
                }
                // Match strings from before the next string match.
            } else {
                // The position is before a string match (or a single code point).
                if offsets.is_empty() {
                    // No more strings matched before a previous string match.
                    // Try another code point span from before the last string match.
                    let old_pos = pos;
                    pos = self.span_set.span_back(s, old_pos, SpanCondition::Contained);
                    span_length = old_pos - pos;
                    if pos == 0 || // Reached the start of the string, or
                            span_length == 0
                    // neither strings nor span progressed.
                    {
                        return pos;
                    }
                    continue; // spanLength>0: Match strings from before a span.
                } else {
                    // Try to match only one code point from before a string match if some
                    // string matched beyond it, so that we try all possible positions
                    // and don't overshoot.
                    span_length = span_one_back(&self.span_set, s, pos);
                    if span_length > 0 {
                        if span_length == pos {
                            return 0; // Reached the start of the string.
                        }
                        // Match strings before this code point.
                        // There cannot be any decrements below it because UnicodeSet strings
                        // contain multiple code points.
                        pos -= span_length;
                        offsets.shift(span_length);
                        span_length = 0;
                        continue; // Match strings from before a single code point.
                    }
                    // Match strings from before the next string match.
                }
            }
            pos -= offsets.pop_minimum(None);
            span_length = 0; // Match strings from before a string match.
        }
    }

    /// Algorithm for String spanNot(): find the next position where some
    /// string or code point matches.
    fn span_not(&self, s: &[u16], start: i32, mut out_count: Option<&mut OutputInt>) -> i32 {
        let length = s.len() as i32;
        let mut pos = start;
        let mut rest = length - start;
        let strings_length = self.strings.len();
        let mut count = 0;
        loop {
            // Span until we find a code point from the set,
            // or a code point that starts or ends some string.
            let span_limit;
            match out_count.as_deref_mut() {
                None => span_limit = self.span_not_set().span(s, pos, SpanCondition::NotContained),
                Some(oc) => {
                    span_limit = self.span_not_set().span_and_count(s, pos, SpanCondition::NotContained, oc);
                    count += oc.value;
                    oc.value = count;
                }
            }
            if span_limit == length {
                return length; // Reached the end of the string.
            }
            pos = span_limit;
            rest = length - span_limit;

            // Check whether the current code point is in the original set,
            // without the string starts and ends.
            let cp_length = span_one(&self.span_set, s, pos, rest);
            if cp_length > 0 {
                return pos; // There is a set element at pos.
            }

            // Try to match the strings at pos.
            for i in 0..strings_length {
                if self.span_lengths[i] == ALL_CP_CONTAINED {
                    continue; // Irrelevant string.
                }
                let string = &self.strings[i];

                let length16 = string.len() as i32;
                if length16 <= rest && matches16_cpb(s, pos, length, string, length16) {
                    return pos; // There is a set element at pos.
                }
            }

            // The span(while not contained) ended on a string start/end which is
            // not in the original set. Skip this code point and continue.
            // cpLength<0
            pos -= cp_length;
            rest += cp_length;
            count += 1;
            if rest == 0 {
                break;
            }
        }
        if let Some(oc) = out_count {
            oc.value = count;
        }
        length // Reached the end of the string.
    }

    fn span_not_back(&self, s: &[u16], length: i32) -> i32 {
        let mut pos = length;
        let strings_length = self.strings.len();
        loop {
            // Span until we find a code point from the set,
            // or a code point that starts or ends some string.
            pos = self.span_not_set().span_back(s, pos, SpanCondition::NotContained);
            if pos == 0 {
                return 0; // Reached the start of the string.
            }

            // Check whether the current code point is in the original set,
            // without the string starts and ends.
            let cp_length = span_one_back(&self.span_set, s, pos);
            if cp_length > 0 {
                return pos; // There is a set element at pos.
            }

            // Try to match the strings at pos.
            for i in 0..strings_length {
                // Use spanLengths rather than a spanLengths pointer because
                // it is easier and we only need to know whether the string is irrelevant
                // which is the same in either array.
                if self.span_lengths[i] == ALL_CP_CONTAINED {
                    continue; // Irrelevant string.
                }
                let string = &self.strings[i];

                let length16 = string.len() as i32;
                if length16 <= pos && matches16_cpb(s, pos - length16, length, string, length16) {
                    return pos; // There is a set element at pos.
                }
            }

            // The span(while not contained) ended on a string start/end which is
            // not in the original set. Skip this code point and continue.
            // cpLength<0
            pos += cp_length;
            if pos == 0 {
                break;
            }
        }
        0 // Reached the start of the string.
    }
}

/// `makeSpanLengthByte(int spanLength)`.
fn make_span_length_byte(span_length: i32) -> i16 {
    // 0xfe==UnicodeSetStringSpan::LONG_SPAN
    if span_length < LONG_SPAN as i32 {
        span_length as i16
    } else {
        LONG_SPAN
    }
}

/// Compare strings without any argument checks. Requires length>0.
fn matches16(s: &[u16], start: i32, t: &[u16], mut length: i32) -> bool {
    let mut end = start + length;
    while length > 0 {
        length -= 1;
        end -= 1;
        if s[end as usize] != t[length as usize] {
            return false;
        }
    }
    true
}

/// `matches16CPB(CharSequence s, int start, int limit, String t, int
/// tlength)`: compare 16-bit Unicode strings (which may be malformed UTF-16)
/// at code point boundaries.
pub(crate) fn matches16_cpb(s: &[u16], start: i32, limit: i32, t: &[u16], tlength: i32) -> bool {
    matches16(s, start, t, tlength)
        && !(0 < start && is_high_surrogate(s[start as usize - 1]) && is_low_surrogate(s[start as usize]))
        && !((start + tlength) < limit
            && is_high_surrogate(s[(start + tlength - 1) as usize])
            && is_low_surrogate(s[(start + tlength) as usize]))
}

/// `spanOne(UnicodeSet set, CharSequence s, int start, int length)`: does
/// the set contain the next code point? If so, return its length; otherwise
/// return its negative length.
pub(crate) fn span_one(set: &UnicodeSet, s: &[u16], start: i32, length: i32) -> i32 {
    let c = s[start as usize];
    if (0xd800..=0xdbff).contains(&c) && length >= 2 {
        let c2 = s[start as usize + 1];
        if UTF16::is_trail_surrogate(c2 as i32) {
            let supplementary = to_code_point(c, c2);
            return if set.contains_valid(supplementary) { 2 } else { -2 };
        }
    }
    if set.contains_valid(c as i32) {
        1
    } else {
        -1
    }
}

/// `spanOneBack(UnicodeSet set, CharSequence s, int length)`.
pub(crate) fn span_one_back(set: &UnicodeSet, s: &[u16], length: i32) -> i32 {
    let c = s[length as usize - 1];
    if (0xdc00..=0xdfff).contains(&c) && length >= 2 {
        let c2 = s[length as usize - 2];
        if UTF16::is_lead_surrogate(c2 as i32) {
            let supplementary = to_code_point(c2, c);
            return if set.contains_valid(supplementary) { 2 } else { -2 };
        }
    }
    if set.contains_valid(c as i32) {
        1
    } else {
        -1
    }
}

/// Helper class for UnicodeSetStringSpan.
///
/// List of offsets from the current position from where to try matching
/// a code point or a string.
/// Stores offsets rather than indexes to simplify the code and use the same
/// list for both increments (in span()) and decrements (in spanBack()).
///
/// Assumption: The maximum offset is limited, and the offsets that are stored at any one time
/// are relatively dense, that is,
/// there are normally no gaps of hundreds or thousands of offset values.
///
/// This class optionally also tracks the minimum non-negative count for each position,
/// intended to count the smallest number of elements of any path leading to that position.
///
/// The implementation uses a circular buffer of count integers,
/// each indicating whether the corresponding offset is in the list,
/// and its path element count.
/// This avoids inserting into a sorted list of offsets (or absolute indexes)
/// and physically moving part of the list.
///
/// Note: In principle, the caller should setMaxLength() to
/// the maximum of the max string length and U16_LENGTH/U8_LENGTH
/// to account for "long" single code points.
///
/// Note: An earlier version did not track counts and stored only byte flags.
/// With boolean flags, if maxLength were guaranteed to be no more than 32 or 64,
/// the list could be stored as bit flags in a single integer.
/// Rather than handling a circular buffer with a start list index,
/// the integer would simply be shifted when lower offsets are removed.
/// UnicodeSet does not have a limit on the lengths of strings.
///
/// Equivalent to `UnicodeSetStringSpan.OffsetList` (private).
#[derive(Debug)]
struct OffsetList {
    list: Vec<i32>,
    length: i32,
    start: i32,
}

impl OffsetList {
    fn new() -> Self {
        Self { list: vec![0; 16], length: 0, start: 0 } // default size
    }

    fn set_max_length(&mut self, max_length: i32) {
        if max_length > self.list.len() as i32 {
            self.list = vec![0; max_length as usize];
        }
        self.clear();
    }

    fn clear(&mut self) {
        for x in self.list.iter_mut() {
            *x = 0;
        }
        self.start = 0;
        self.length = 0;
    }

    fn is_empty(&self) -> bool {
        self.length == 0
    }

    fn wrap(&self, offset: i32) -> usize {
        let mut i = self.start + offset;
        if i >= self.list.len() as i32 {
            i -= self.list.len() as i32;
        }
        i as usize
    }

    /// Reduces all stored offsets by delta, used when the current position
    /// moves by delta. There must not be any offsets lower than delta.
    /// If there is an offset equal to delta, it is removed.
    fn shift(&mut self, delta: i32) {
        let i = self.wrap(delta);
        if self.list[i] != 0 {
            self.list[i] = 0;
            self.length -= 1;
        }
        self.start = i as i32;
    }

    /// Adds an offset. The list must not contain it yet.
    fn add_offset(&mut self, offset: i32) {
        let i = self.wrap(offset);
        debug_assert!(self.list[i] == 0);
        self.list[i] = 1;
        self.length += 1;
    }

    /// Adds an offset and updates its count.
    /// The list may already contain the offset.
    fn add_offset_and_count(&mut self, offset: i32, count: i32) {
        debug_assert!(count > 0);
        let i = self.wrap(offset);
        if self.list[i] == 0 {
            self.list[i] = count;
            self.length += 1;
        } else if count < self.list[i] {
            self.list[i] = count;
        }
    }

    fn contains_offset(&self, offset: i32) -> bool {
        self.list[self.wrap(offset)] != 0
    }

    fn has_count_at_offset(&self, offset: i32, count: i32) -> bool {
        let old_count = self.list[self.wrap(offset)];
        old_count != 0 && old_count <= count
    }

    /// Finds the lowest stored offset from a non-empty list, removes it,
    /// and reduces all other offsets by this minimum.
    fn pop_minimum(&mut self, out_count: Option<&mut OutputInt>) -> i32 {
        // Look for the next offset in list[start+1..list.length-1].
        let mut i = self.start;
        let len = self.list.len() as i32;
        loop {
            i += 1;
            if i >= len {
                break;
            }
            let count = self.list[i as usize];
            if count != 0 {
                self.list[i as usize] = 0;
                self.length -= 1;
                let result = i - self.start;
                self.start = i;
                if let Some(oc) = out_count {
                    oc.value = count;
                }
                return result;
            }
        }
        // i==list.length

        // Wrap around and look for the next offset in list[0..start].
        // Since the list is not empty, there will be one.
        let result = len - self.start;
        let mut i = 0usize;
        let count;
        loop {
            let c = self.list[i];
            if c != 0 {
                count = c;
                break;
            }
            i += 1;
        }
        self.list[i] = 0;
        self.length -= 1;
        self.start = i as i32;
        if let Some(oc) = out_count {
            oc.value = count;
        }
        result + i as i32
    }
}
