// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.TailoredSet`.

use std::sync::Arc;

use rucene::error::Result;

use crate::jdk::append_code_point;
use crate::r#impl::coll::collation::Collation;
use crate::r#impl::coll::collation_data::CollationData;
use crate::r#impl::normalizer2_impl::Hangul;
use crate::text::unicode_set::UnicodeSet;
use crate::util::chars_trie::{CharsTrie, Entry};

/// Java's `StringBuilder.reverse()`.
fn string_builder_reverse(s: &mut [u16]) {
    s.reverse();
    let mut i = 0;
    while i + 1 < s.len() {
        if crate::jdk::is_low_surrogate(s[i]) && crate::jdk::is_high_surrogate(s[i + 1]) {
            s.swap(i, i + 1);
            i += 1;
        }
        i += 1;
    }
}

/// A prefix or suffix of the parallel iteration: `None` for Java's `null`,
/// `Some((text, true))` for the identity-compared sentinel `none`.
type Fix = Option<(Vec<u16>, bool)>;

/// Finds the set of characters and strings that sort differently in the
/// tailoring from the base data.
///
/// Equivalent to `com.ibm.icu.impl.coll.TailoredSet`; the result set is
/// borrowed for its lifetime.
#[derive(Debug)]
pub struct TailoredSet<'a> {
    data: Option<Arc<CollationData>>,
    base_data: Option<Arc<CollationData>>,
    tailored: &'a mut UnicodeSet,
    unreversed_prefix: Vec<u16>,
    suffix: Option<Vec<u16>>,
}

impl<'a> TailoredSet<'a> {
    /// `TailoredSet(UnicodeSet t)`.
    pub fn new(t: &'a mut UnicodeSet) -> Self {
        Self {
            data: None,
            base_data: None,
            tailored: t,
            unreversed_prefix: Vec::new(),
            suffix: None,
        }
    }

    fn data(&self) -> Arc<CollationData> {
        self.data
            .clone()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
    }

    fn base_data(&self) -> Arc<CollationData> {
        self.base_data
            .clone()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
    }

    /// `forData(CollationData)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when the result set is frozen.
    pub fn for_data(&mut self, d: &Arc<CollationData>) -> Result<()> {
        self.data = Some(d.clone());
        self.base_data = d.base.clone();
        debug_assert!(self.base_data.is_some());
        // utrie2_enum(data->trie, NULL, enumTailoredRange, this);
        let ranges: Vec<_> = {
            let trie = d
                .trie
                .as_deref()
                .unwrap_or_else(|| panic!("java.lang.NullPointerException"));
            trie.iterator().take_while(|r| !r.lead_surrogate).collect()
        };
        for range in ranges {
            self.enum_tailored_range(range.start_code_point, range.end_code_point, range.value)?;
        }
        Ok(())
    }

    fn enum_tailored_range(&mut self, start: i32, end: i32, ce32: i32) -> Result<()> {
        if ce32 == Collation::FALLBACK_CE32 {
            return Ok(()); // fallback to base, not tailored
        }
        self.handle_ce32(start, end, ce32)
    }

    // Java porting note: ICU4C returns U_SUCCESS(error) and it's not applicable to ICU4J.
    //  Also, ICU4C requires handleCE32() to be public because it is used by the callback
    //  function (enumTailoredRange()). This is not necessary for Java implementation.
    fn handle_ce32(&mut self, mut start: i32, end: i32, mut ce32: i32) -> Result<()> {
        debug_assert!(ce32 != Collation::FALLBACK_CE32);
        if Collation::is_special_ce32(ce32) {
            ce32 = self.data().get_indirect_ce32(ce32);
            if ce32 == Collation::FALLBACK_CE32 {
                return Ok(());
            }
        }
        let base_data = self.base_data();
        loop {
            let base_ce32 = base_data.get_final_ce32(base_data.get_ce32(start));
            // Do not just continue if ce32 == baseCE32 because
            // contractions and expansions in different data objects
            // normally differ even if they have the same data offsets.
            if Collation::is_self_contained_ce32(ce32)
                && Collation::is_self_contained_ce32(base_ce32)
            {
                // fastpath
                if ce32 != base_ce32 {
                    self.tailored.add(start)?;
                }
            } else {
                self.compare(start, ce32, base_ce32)?;
            }
            start += 1;
            if start > end {
                break;
            }
        }
        Ok(())
    }

    fn compare(&mut self, c: i32, mut ce32: i32, mut base_ce32: i32) -> Result<()> {
        let data = self.data();
        let base_data = self.base_data();
        if Collation::is_prefix_ce32(ce32) {
            let data_index = Collation::index_from_ce32(ce32);
            ce32 = data.get_final_ce32(data.get_ce32_from_contexts(data_index));
            if Collation::is_prefix_ce32(base_ce32) {
                let base_index = Collation::index_from_ce32(base_ce32);
                base_ce32 = base_data.get_final_ce32(base_data.get_ce32_from_contexts(base_index));
                self.compare_prefixes(
                    c,
                    data.contexts.clone(),
                    data_index + 2,
                    base_data.contexts.clone(),
                    base_index + 2,
                )?;
            } else {
                self.add_prefixes(&data, c, data.contexts.clone(), data_index + 2)?;
            }
        } else if Collation::is_prefix_ce32(base_ce32) {
            let base_index = Collation::index_from_ce32(base_ce32);
            base_ce32 = base_data.get_final_ce32(base_data.get_ce32_from_contexts(base_index));
            self.add_prefixes(&base_data, c, base_data.contexts.clone(), base_index + 2)?;
        }

        if Collation::is_contraction_ce32(ce32) {
            let data_index = Collation::index_from_ce32(ce32);
            if (ce32 & Collation::CONTRACT_SINGLE_CP_NO_MATCH) != 0 {
                ce32 = Collation::NO_CE32;
            } else {
                ce32 = data.get_final_ce32(data.get_ce32_from_contexts(data_index));
            }
            if Collation::is_contraction_ce32(base_ce32) {
                let base_index = Collation::index_from_ce32(base_ce32);
                if (base_ce32 & Collation::CONTRACT_SINGLE_CP_NO_MATCH) != 0 {
                    base_ce32 = Collation::NO_CE32;
                } else {
                    base_ce32 =
                        base_data.get_final_ce32(base_data.get_ce32_from_contexts(base_index));
                }
                self.compare_contractions(
                    c,
                    data.contexts.clone(),
                    data_index + 2,
                    base_data.contexts.clone(),
                    base_index + 2,
                )?;
            } else {
                self.add_contractions(c, data.contexts.clone(), data_index + 2)?;
            }
        } else if Collation::is_contraction_ce32(base_ce32) {
            let base_index = Collation::index_from_ce32(base_ce32);
            base_ce32 = base_data.get_final_ce32(base_data.get_ce32_from_contexts(base_index));
            self.add_contractions(c, base_data.contexts.clone(), base_index + 2)?;
        }

        let tag;
        if Collation::is_special_ce32(ce32) {
            tag = Collation::tag_from_ce32(ce32);
            debug_assert!(tag != Collation::PREFIX_TAG);
            debug_assert!(tag != Collation::CONTRACTION_TAG);
            // Currently, the tailoring data builder does not write offset tags.
            // They might be useful for saving space,
            // but they would complicate the builder,
            // and in tailorings we assume that performance of tailored characters is more important.
            debug_assert!(tag != Collation::OFFSET_TAG);
        } else {
            tag = -1;
        }
        let base_tag;
        if Collation::is_special_ce32(base_ce32) {
            base_tag = Collation::tag_from_ce32(base_ce32);
            debug_assert!(base_tag != Collation::PREFIX_TAG);
            debug_assert!(base_tag != Collation::CONTRACTION_TAG);
        } else {
            base_tag = -1;
        }

        // Non-contextual mappings, expansions, etc.
        if base_tag == Collation::OFFSET_TAG {
            // We might be comparing a tailoring CE which is a copy of
            // a base offset-tag CE, via the [optimize [set]] syntax
            // or when a single-character mapping was copied for tailored contractions.
            // Offset tags always result in long-primary CEs,
            // with common secondary/tertiary weights.
            if !Collation::is_long_primary_ce32(ce32) {
                return self.add(c);
            }
            let data_ce = base_data.ces[Collation::index_from_ce32(base_ce32) as usize];
            let p = Collation::get_three_byte_primary_for_offset_data(c, data_ce);
            if Collation::primary_from_long_primary_ce32(ce32) != p {
                return self.add(c);
            }
        }

        if tag != base_tag {
            return self.add(c);
        }

        if tag == Collation::EXPANSION32_TAG {
            let length = Collation::length_from_ce32(ce32);
            let base_length = Collation::length_from_ce32(base_ce32);

            if length != base_length {
                return self.add(c);
            }

            let idx0 = Collation::index_from_ce32(ce32);
            let idx1 = Collation::index_from_ce32(base_ce32);

            for i in 0..length {
                if data.ce32s[(idx0 + i) as usize] != base_data.ce32s[(idx1 + i) as usize] {
                    self.add(c)?;
                    break;
                }
            }
        } else if tag == Collation::EXPANSION_TAG {
            let length = Collation::length_from_ce32(ce32);
            let base_length = Collation::length_from_ce32(base_ce32);

            if length != base_length {
                return self.add(c);
            }

            let idx0 = Collation::index_from_ce32(ce32);
            let idx1 = Collation::index_from_ce32(base_ce32);

            for i in 0..length {
                if data.ces[(idx0 + i) as usize] != base_data.ces[(idx1 + i) as usize] {
                    self.add(c)?;
                    break;
                }
            }
        } else if tag == Collation::HANGUL_TAG {
            let mut jamos: Vec<u16> = Vec::new();
            let length = Hangul::decompose(c, &mut jamos);
            if self.tailored.contains_valid(jamos[0] as i32)
                || self.tailored.contains_valid(jamos[1] as i32)
                || (length == 3 && self.tailored.contains_valid(jamos[2] as i32))
            {
                self.add(c)?;
            }
        } else if ce32 != base_ce32 {
            self.add(c)?;
        }
        Ok(())
    }

    fn next_fix(
        it: &mut crate::util::chars_trie::Iterator,
        sentinel: &[u16],
    ) -> (Option<Entry>, Fix) {
        if it.has_next() {
            let e = it.next().expect("hasNext");
            let s = e.chars.clone();
            (Some(e), Some((s, false)))
        } else {
            (None, Some((sentinel.to_vec(), true)))
        }
    }

    fn compare_prefixes(
        &mut self,
        c: i32,
        p: Arc<[u16]>,
        pidx: i32,
        q: Arc<[u16]>,
        qidx: i32,
    ) -> Result<()> {
        // Parallel iteration over prefixes of both tables.
        let mut prefixes = CharsTrie::new(p, pidx).iterator();
        let mut base_prefixes = CharsTrie::new(q, qidx).iterator();
        let mut tp: Fix = None; // Tailoring prefix.
        let mut bp: Fix = None; // Base prefix.
                                // Use a string with a U+FFFF as the limit sentinel.
                                // U+FFFF is untailorable and will not occur in prefixes.
        let none: [u16; 1] = [0xffff];
        let mut te: Option<Entry> = None;
        let mut be: Option<Entry> = None;
        loop {
            if tp.is_none() {
                (te, tp) = Self::next_fix(&mut prefixes, &none);
            }
            if bp.is_none() {
                (be, bp) = Self::next_fix(&mut base_prefixes, &none);
            }
            let (t, t_none) = tp.clone().expect("set");
            let (b, b_none) = bp.clone().expect("set");
            if t_none && b_none {
                break;
            }
            let cmp = t.cmp(&b);
            if cmp.is_lt() {
                // tp occurs in the tailoring but not in the base.
                let te_value = te.as_ref().expect("tailoring entry").value;
                let data = self.data();
                self.add_prefix(&data, &t, c, te_value)?;
                te = None;
                tp = None;
            } else if cmp.is_gt() {
                // bp occurs in the base but not in the tailoring.
                let be_value = be.as_ref().expect("base entry").value;
                let base_data = self.base_data();
                self.add_prefix(&base_data, &b, c, be_value)?;
                be = None;
                bp = None;
            } else {
                self.set_prefix(&t);
                let (tv, bv) = (
                    te.as_ref().expect("tailoring entry").value,
                    be.as_ref().expect("base entry").value,
                );
                self.compare(c, tv, bv)?;
                self.reset_prefix();
                te = None;
                be = None;
                tp = None;
                bp = None;
            }
        }
        Ok(())
    }

    fn compare_contractions(
        &mut self,
        c: i32,
        p: Arc<[u16]>,
        pidx: i32,
        q: Arc<[u16]>,
        qidx: i32,
    ) -> Result<()> {
        // Parallel iteration over suffixes of both tables.
        let mut suffixes = CharsTrie::new(p, pidx).iterator();
        let mut base_suffixes = CharsTrie::new(q, qidx).iterator();
        let mut ts: Fix = None; // Tailoring suffix.
        let mut bs: Fix = None; // Base suffix.
                                // Use a string with two U+FFFF as the limit sentinel.
                                // U+FFFF is untailorable and will not occur in contractions except maybe
                                // as a single suffix character for a root-collator boundary contraction.
        let none: [u16; 2] = [0xffff, 0xffff];
        let mut te: Option<Entry> = None;
        let mut be: Option<Entry> = None;
        loop {
            if ts.is_none() {
                (te, ts) = Self::next_fix(&mut suffixes, &none);
            }
            if bs.is_none() {
                (be, bs) = Self::next_fix(&mut base_suffixes, &none);
            }
            let (t, t_none) = ts.clone().expect("set");
            let (b, b_none) = bs.clone().expect("set");
            if t_none && b_none {
                break;
            }
            let cmp = t.cmp(&b);
            if cmp.is_lt() {
                // ts occurs in the tailoring but not in the base.
                self.add_suffix(c, &t)?;
                te = None;
                ts = None;
            } else if cmp.is_gt() {
                // bs occurs in the base but not in the tailoring.
                self.add_suffix(c, &b)?;
                be = None;
                bs = None;
            } else {
                self.suffix = Some(t.clone());
                let (tv, bv) = (
                    te.as_ref()
                        .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
                        .value,
                    be.as_ref()
                        .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
                        .value,
                );
                self.compare(c, tv, bv)?;
                self.suffix = None;
                te = None;
                be = None;
                ts = None;
                bs = None;
            }
        }
        let _ = (te, be);
        Ok(())
    }

    fn add_prefixes(&mut self, d: &CollationData, c: i32, p: Arc<[u16]>, pidx: i32) -> Result<()> {
        let prefixes = CharsTrie::new(p, pidx).iterator();
        for e in prefixes {
            self.add_prefix(d, &e.chars, c, e.value)?;
        }
        Ok(())
    }

    fn add_prefix(&mut self, d: &CollationData, pfx: &[u16], c: i32, mut ce32: i32) -> Result<()> {
        self.set_prefix(pfx);
        ce32 = d.get_final_ce32(ce32);
        if Collation::is_contraction_ce32(ce32) {
            let idx = Collation::index_from_ce32(ce32);
            self.add_contractions(c, d.contexts.clone(), idx + 2)?;
        }
        append_code_point(&mut self.unreversed_prefix, c)?;
        let s = self.unreversed_prefix.clone();
        self.tailored.add_str(&s)?;
        self.reset_prefix();
        Ok(())
    }

    fn add_contractions(&mut self, c: i32, p: Arc<[u16]>, pidx: i32) -> Result<()> {
        let suffixes = CharsTrie::new(p, pidx).iterator();
        for e in suffixes {
            self.add_suffix(c, &e.chars)?;
        }
        Ok(())
    }

    fn add_suffix(&mut self, c: i32, sfx: &[u16]) -> Result<()> {
        let mut s = self.unreversed_prefix.clone();
        append_code_point(&mut s, c)?;
        s.extend_from_slice(sfx);
        self.tailored.add_str(&s)?;
        Ok(())
    }

    fn add(&mut self, c: i32) -> Result<()> {
        if self.unreversed_prefix.is_empty() && self.suffix.is_none() {
            self.tailored.add(c)?;
        } else {
            let mut s = self.unreversed_prefix.clone();
            append_code_point(&mut s, c)?;
            if let Some(suffix) = &self.suffix {
                s.extend_from_slice(suffix);
            }
            self.tailored.add_str(&s)?;
        }
        Ok(())
    }

    // Prefixes are reversed in the data structure.
    fn set_prefix(&mut self, pfx: &[u16]) {
        self.unreversed_prefix.clear();
        self.unreversed_prefix.extend_from_slice(pfx);
        string_builder_reverse(&mut self.unreversed_prefix);
    }

    fn reset_prefix(&mut self) {
        self.unreversed_prefix.clear();
    }
}
