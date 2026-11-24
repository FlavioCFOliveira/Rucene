// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.CollationDataBuilder`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};

use crate::jdk::{
    char_count, code_point_at, code_point_before, index_of, offset_by_code_points, units,
};
use crate::lang::u_character::UCharacter;
use crate::r#impl::coll::collation::Collation;
use crate::r#impl::coll::collation_data::CollationData;
use crate::r#impl::coll::collation_fast_latin_builder::CollationFastLatinBuilder;
use crate::r#impl::coll::collation_iterator::{CollationIterator, CollationIteratorBase};
use crate::r#impl::coll::collation_settings::CollationSettings;
use crate::r#impl::coll::u_vector32::UVector32;
use crate::r#impl::coll::u_vector64::UVector64;
use crate::r#impl::norm2_all_modes::Norm2AllModes;
use crate::r#impl::normalizer2_impl::{Hangul, Normalizer2Impl};
use crate::r#impl::trie2_writable::Trie2Writable;
use crate::text::unicode_set::UnicodeSet;
use crate::text::unicode_set_iterator::{UnicodeSetIterator, IS_STRING};
use crate::util::chars_trie::CharsTrie;
use crate::util::chars_trie_builder::CharsTrieBuilder;
use crate::util::string_trie_builder::StringTrieBuilderOption;

/// `new IndexOutOfBoundsException(message)` that ICU throws explicitly
/// and its callers catch (`DataBuilderCollationIterator`, and the rule
/// parser's `catch(Exception)`), hence a `Result` error here.
fn index_out_of_bounds(message: &str) -> LuceneError {
    LuceneError::unchecked("java.lang.IndexOutOfBoundsException", message)
}

fn is_index_out_of_bounds(e: &LuceneError) -> bool {
    matches!(e.kind(), rucene::error::LuceneErrorKind::Unchecked { class, .. } if *class == "java.lang.IndexOutOfBoundsException")
}

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

/// Collation element modifier. Interface class for a modifier that
/// changes a tailoring builder's temporary CEs to final CEs. Called for
/// every non-special CE32 and every expansion CE.
///
/// Equivalent to `CollationDataBuilder.CEModifier`.
pub trait CEModifier {
    /// Returns a new CE to replace the non-special input CE32, or else Collation.NO_CE.
    fn modify_ce32(&mut self, ce32: i32) -> i64;
    /// Returns a new CE to replace the input CE, or else Collation.NO_CE.
    fn modify_ce(&mut self, ce: i64) -> i64;
}

/// Build-time context and CE32 for a code point. If a code point has
/// contextual mappings, then the default (no-context) mapping and all
/// conditional mappings are stored in a singly-linked list of
/// ConditionalCE32, sorted by context strings.
///
/// Equivalent to `CollationDataBuilder.ConditionalCE32` (private); the
/// list is linked through indexes into `conditionalCE32s`, as in Java.
#[derive(Debug, Clone)]
struct ConditionalCE32 {
    /// Pre-context string length, then pre-context string (reversed), then
    /// suffix string.
    context: Vec<u16>,
    /// CE32 for the code point and its context. Can be special
    /// (e.g., for an expansion) but not contextual (prefix or contraction tag).
    ce32: i32,
    /// Default CE32 for all contexts with this same prefix.
    default_ce32: i32,
    /// CE32 for the built contexts.
    built_ce32: i32,
    /// The "era" of building intermediate contexts when the above builtCE32 was set.
    era: i32,
    /// Index of the next ConditionalCE32. Negative for the end of the list.
    next: i32,
}

impl ConditionalCE32 {
    fn new(ct: Vec<u16>, ce: i32) -> Self {
        Self {
            context: ct,
            ce32: ce,
            default_ce32: Collation::NO_CE32,
            built_ce32: Collation::NO_CE32,
            era: -1,
            next: -1,
        }
    }

    fn has_context(&self) -> bool {
        self.context.len() > 1
    }

    fn prefix_length(&self) -> i32 {
        self.context[0] as i32
    }
}

/// A `ConditionalCE32` that `copyFromBaseCE32()` links from: the
/// temporary list head Java allocates outside the list, or a list node.
#[derive(Debug, Clone, Copy)]
enum Cond {
    Head,
    Idx(i32),
}

/// `IS_BUILDER_JAMO_CE32`; see `Collation.BUILDER_DATA_TAG`.
const IS_BUILDER_JAMO_CE32: i32 = 0x100;

/// Low-level CollationData builder. Takes (character, CE) pairs and
/// builds them into runtime data structures. Supports characters with
/// context prefixes and contraction suffixes.
///
/// Equivalent to `com.ibm.icu.impl.coll.CollationDataBuilder`.
#[derive(Debug)]
pub struct CollationDataBuilder {
    pub(crate) nfc_impl: Arc<Normalizer2Impl>,
    pub(crate) base: Option<Arc<CollationData>>,
    #[allow(dead_code)] // declared and never read in Java either
    pub(crate) base_settings: Option<Arc<CollationSettings>>,
    pub(crate) trie: Option<Trie2Writable>,
    pub(crate) ce32s: UVector32,
    pub(crate) ce64s: UVector64,
    conditional_ce32s: Vec<ConditionalCE32>, // vector of ConditionalCE32
    /// Characters that have context (prefixes or contraction suffixes).
    pub(crate) context_chars: UnicodeSet,
    /// Serialized UCharsTrie structures for finalized contexts.
    pub(crate) contexts: Vec<u16>,
    /// The "era" of building intermediate contexts.
    contexts_era: i32,
    pub(crate) unsafe_backward_set: UnicodeSet,
    pub(crate) modified: bool,

    pub(crate) fast_latin_enabled: bool,
    pub(crate) fast_latin_builder: Option<CollationFastLatinBuilder>,

    coll_iter: Option<DataBuilderCollationIterator>,

    /// `next` of the temporary list head of `copyFromBaseCE32()`.
    head_next: i32,
}

impl CollationDataBuilder {
    /// `CollationDataBuilder()`.
    ///
    /// # Errors
    ///
    /// The exception of loading the NFC data.
    pub fn new() -> Result<Self> {
        let mut b = Self {
            nfc_impl: Norm2AllModes::get_nfc_instance()?.impl_.clone(),
            base: None,
            base_settings: None,
            trie: None,
            ce32s: UVector32::new(),
            ce64s: UVector64::new(),
            conditional_ce32s: Vec::new(),
            context_chars: UnicodeSet::new(),
            contexts: Vec::new(),
            contexts_era: 0,
            unsafe_backward_set: UnicodeSet::new(),
            modified: false,
            fast_latin_enabled: false,
            fast_latin_builder: None,
            coll_iter: None,
            head_next: -1,
        };
        // Reserve the first CE32 for U+0000.
        b.ce32s.add_element(0);
        Ok(b)
    }

    fn base(&self) -> &Arc<CollationData> {
        self.base
            .as_ref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
    }

    fn trie(&self) -> &Trie2Writable {
        self.trie
            .as_ref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
    }

    fn trie_mut(&mut self) -> &mut Trie2Writable {
        self.trie
            .as_mut()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
    }

    /// `initForTailoring(CollationData)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` on reuse; `IllegalArgumentException` for a
    /// null base.
    pub fn init_for_tailoring(&mut self, b: Option<Arc<CollationData>>) -> Result<()> {
        if self.trie.is_some() {
            return Err(LuceneError::illegal_state(
                "attempt to reuse a CollationDataBuilder",
            ));
        }
        let Some(b) = b else {
            return Err(LuceneError::illegal_argument("null CollationData"));
        };
        self.base = Some(b.clone());

        // For a tailoring, the default is to fall back to the base.
        let mut trie = Trie2Writable::new(Collation::FALLBACK_CE32, Collation::FFFD_CE32);

        // Set the Latin-1 letters block so that it is allocated first in the data array,
        // to try to improve locality of reference when sorting Latin-1 text.
        // Do not use utrie2_setRange32() since that will not actually allocate blocks
        // that are filled with the default value.
        // ASCII (0..7F) is already preallocated anyway.
        for c in 0xc0..=0xff {
            trie.set(c, Collation::FALLBACK_CE32)?;
        }

        // Hangul syllables are not tailorable (except via tailoring Jamos).
        // Always set the Hangul tag to help performance.
        // Do this here, rather than in buildMappings(),
        // so that we see the HANGUL_TAG in various assertions.
        let hangul_ce32 = Collation::make_ce32_from_tag_and_index(Collation::HANGUL_TAG, 0);
        trie.set_range(Hangul::HANGUL_BASE, Hangul::HANGUL_END, hangul_ce32, true)?;
        self.trie = Some(trie);

        // Copy the set contents but don't copy/clone the set as a whole because
        // that would copy the isFrozen state too.
        let set = b
            .unsafe_backward_set
            .as_deref()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException"));
        self.unsafe_backward_set.add_all(set)?;
        Ok(())
    }

    /// `isCompressibleLeadByte(int)`.
    pub fn is_compressible_lead_byte(&self, b: i32) -> bool {
        self.base().is_compressible_lead_byte(b)
    }

    /// `isCompressiblePrimary(long)`.
    pub fn is_compressible_primary(&self, p: i64) -> bool {
        self.is_compressible_lead_byte(((p as i32 as u32) >> 24) as i32)
    }

    /// Returns true if this builder has mappings (e.g., add() has been called).
    pub fn has_mappings(&self) -> bool {
        self.modified
    }

    /// Returns true if c has CEs in this builder.
    pub fn is_assigned(&self, c: i32) -> bool {
        Collation::is_assigned_ce32(self.trie().get(c))
    }

    /// `add(CharSequence, CharSequence, long[], int)`.
    ///
    /// # Errors
    ///
    /// The exceptions of `encodeCEs` and `addCE32`.
    pub fn add(&mut self, prefix: &[u16], s: &[u16], ces: &[i64], ces_length: i32) -> Result<()> {
        let ce32 = self.encode_ces(ces, ces_length)?;
        self.add_ce32(prefix, s, ce32)
    }

    /// Encodes the ces as either the returned ce32 by itself, or by storing
    /// an expansion, with the returned ce32 referring to that.
    ///
    /// Equivalent to `encodeCEs(long[], int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException`, `IllegalStateException`, and the
    /// `IndexOutOfBoundsException` of too many mappings.
    pub fn encode_ces(&mut self, ces: &[i64], ces_length: i32) -> Result<i32> {
        if !(0..=Collation::MAX_EXPANSION_LENGTH).contains(&ces_length) {
            return Err(LuceneError::illegal_argument("mapping to too many CEs"));
        }
        if !self.is_mutable() {
            return Err(LuceneError::illegal_state(
                "attempt to add mappings after build()",
            ));
        }
        if ces_length == 0 {
            // Convenience: We cannot map to nothing, but we can map to a completely ignorable CE.
            // Do this here so that callers need not do it.
            return Ok(Self::encode_one_ce_as_ce32(0));
        } else if ces_length == 1 {
            return self.encode_one_ce(ces[0]);
        } else if ces_length == 2 {
            // Try to encode two CEs as one CE32.
            let ce0 = ces[0];
            let ce1 = ces[1];
            let p0 = ((ce0 as u64) >> 32) as i64;
            if (ce0 & 0xffffffffff00ff) == Collation::COMMON_SECONDARY_CE as i64
                && (ce1 & (0xffffffff00ffffffu64 as i64)) == Collation::COMMON_TERTIARY_CE as i64
                && p0 != 0
            {
                // Latin mini expansion
                return Ok(p0 as i32
                    | ((ce0 as i32 & 0xff00) << 8)
                    | ((ce1 as i32 >> 16) & 0xff00)
                    | Collation::SPECIAL_CE32_LOW_BYTE
                    | Collation::LATIN_EXPANSION_TAG);
            }
        }
        // Try to encode two or more CEs as CE32s.
        let mut new_ce32s = [0i32; Collation::MAX_EXPANSION_LENGTH as usize]; // TODO: instance field?
        let mut i = 0;
        loop {
            if i == ces_length {
                return self.encode_expansion32(&new_ce32s, 0, ces_length);
            }
            let ce32 = Self::encode_one_ce_as_ce32(ces[i as usize]);
            if ce32 == Collation::NO_CE32 {
                break;
            }
            new_ce32s[i as usize] = ce32;
            i += 1;
        }
        self.encode_expansion(ces, 0, ces_length)
    }

    /// `addCE32(CharSequence, CharSequence, int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException`, `IllegalStateException`,
    /// `UnsupportedOperationException` (Hangul), and the
    /// `IndexOutOfBoundsException` of too many mappings.
    pub fn add_ce32(&mut self, prefix: &[u16], s: &[u16], ce32: i32) -> Result<()> {
        if s.is_empty() {
            return Err(LuceneError::illegal_argument("mapping from empty string"));
        }
        if !self.is_mutable() {
            return Err(LuceneError::illegal_state(
                "attempt to add mappings after build()",
            ));
        }
        let c = code_point_at(s, 0);
        let c_length = char_count(c);
        let mut old_ce32 = self.trie().get(c);
        let has_context = !prefix.is_empty() || s.len() as i32 > c_length;
        if old_ce32 == Collation::FALLBACK_CE32 {
            // First tailoring for c.
            // If c has contextual base mappings or if we add a contextual mapping,
            // then copy the base mappings.
            // Otherwise we just override the base mapping.
            let base = self.base().clone();
            let base_ce32 = base.get_final_ce32(base.get_ce32(c));
            if has_context || Collation::ce32_has_context(base_ce32) {
                old_ce32 = self.copy_from_base_ce32(c, base_ce32, true)?;
                self.trie_mut().set(c, old_ce32)?;
            }
        }
        if !has_context {
            // No prefix, no contraction.
            if !Self::is_builder_context_ce32(old_ce32) {
                self.trie_mut().set(c, ce32)?;
            } else {
                let cond = self.get_conditional_ce32_for_ce32(old_ce32);
                let cond = &mut self.conditional_ce32s[cond as usize];
                cond.built_ce32 = Collation::NO_CE32;
                cond.ce32 = ce32;
            }
        } else {
            let mut cond: i32;
            if !Self::is_builder_context_ce32(old_ce32) {
                // Replace the simple oldCE32 with a builder context CE32
                // pointing to a new ConditionalCE32 list head.
                let index = self.add_conditional_ce32(vec![0], old_ce32)?;
                let context_ce32 = Self::make_builder_context_ce32(index);
                self.trie_mut().set(c, context_ce32)?;
                self.context_chars.add(c)?;
                cond = index;
            } else {
                cond = self.get_conditional_ce32_for_ce32(old_ce32);
                self.conditional_ce32s[cond as usize].built_ce32 = Collation::NO_CE32;
            }
            let suffix = &s[c_length as usize..];
            let mut context: Vec<u16> = Vec::with_capacity(1 + prefix.len() + suffix.len());
            context.push(prefix.len() as u16);
            context.extend_from_slice(prefix);
            context.extend_from_slice(suffix);
            self.unsafe_backward_set.add_all_str(suffix)?;
            loop {
                // invariant: context > cond.context
                let next = self.conditional_ce32s[cond as usize].next;
                if next < 0 {
                    // Append a new ConditionalCE32 after cond.
                    let index = self.add_conditional_ce32(context, ce32)?;
                    self.conditional_ce32s[cond as usize].next = index;
                    break;
                }
                let cmp = context.cmp(&self.conditional_ce32s[next as usize].context);
                if cmp.is_lt() {
                    // Insert a new ConditionalCE32 between cond and nextCond.
                    let index = self.add_conditional_ce32(context, ce32)?;
                    self.conditional_ce32s[cond as usize].next = index;
                    self.conditional_ce32s[index as usize].next = next;
                    break;
                } else if cmp.is_eq() {
                    // Same context as before, overwrite its ce32.
                    self.conditional_ce32s[next as usize].ce32 = ce32;
                    break;
                }
                cond = next;
            }
        }
        self.modified = true;
        Ok(())
    }

    /// Copies all mappings from the src builder, with modifications. This
    /// builder here must not be built yet, and should be empty.
    ///
    /// Equivalent to `copyFrom(CollationDataBuilder, CEModifier)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` after build(), and the exceptions of the
    /// encoders.
    pub fn copy_from(
        &mut self,
        src: &CollationDataBuilder,
        modifier: &mut dyn CEModifier,
    ) -> Result<()> {
        if !self.is_mutable() {
            return Err(LuceneError::illegal_state(
                "attempt to copyFrom() after build()",
            ));
        }
        let mut helper = CopyHelper {
            src,
            modifier,
            modified_ces: [0; Collation::MAX_EXPANSION_LENGTH as usize],
        };
        let ranges: Vec<_> = src
            .trie()
            .iterator()
            .take_while(|r| !r.lead_surrogate)
            .collect();
        for range in ranges {
            enum_range_for_copy(
                range.start_code_point,
                range.end_code_point,
                range.value,
                &mut helper,
                self,
            )?;
        }
        // Update the contextChars and the unsafeBackwardSet while copying,
        // in case a character had conditional mappings in the source builder
        // and they were removed later.
        self.modified |= src.modified;
        Ok(())
    }

    /// `optimize(UnicodeSet)`.
    ///
    /// # Errors
    ///
    /// The exceptions of `copyFromBaseCE32`.
    pub fn optimize(&mut self, set: &UnicodeSet) -> Result<()> {
        if set.is_empty() {
            return Ok(());
        }
        let mut iter = UnicodeSetIterator::new(set);
        while iter.next() && iter.codepoint != IS_STRING {
            let c = iter.codepoint;
            let mut ce32 = self.trie().get(c);
            if ce32 == Collation::FALLBACK_CE32 {
                let base = self.base().clone();
                ce32 = base.get_final_ce32(base.get_ce32(c));
                ce32 = self.copy_from_base_ce32(c, ce32, true)?;
                self.trie_mut().set(c, ce32)?;
            }
        }
        self.modified = true;
        Ok(())
    }

    /// `suppressContractions(UnicodeSet)`.
    ///
    /// # Errors
    ///
    /// The exceptions of `copyFromBaseCE32`.
    pub fn suppress_contractions(&mut self, set: &UnicodeSet) -> Result<()> {
        if set.is_empty() {
            return Ok(());
        }
        let mut iter = UnicodeSetIterator::new(set);
        while iter.next() && iter.codepoint != IS_STRING {
            let c = iter.codepoint;
            let mut ce32 = self.trie().get(c);
            if ce32 == Collation::FALLBACK_CE32 {
                let base = self.base().clone();
                ce32 = base.get_final_ce32(base.get_ce32(c));
                if Collation::ce32_has_context(ce32) {
                    ce32 = self.copy_from_base_ce32(c, ce32, false /* without context */)?;
                    self.trie_mut().set(c, ce32)?;
                }
            } else if Self::is_builder_context_ce32(ce32) {
                ce32 =
                    self.conditional_ce32s[self.get_conditional_ce32_for_ce32(ce32) as usize].ce32;
                // Simply abandon the list of ConditionalCE32.
                // The caller will copy this builder in the end,
                // eliminating unreachable data.
                self.trie_mut().set(c, ce32)?;
                self.context_chars.remove(c)?;
            }
        }
        self.modified = true;
        Ok(())
    }

    /// `enableFastLatin()`.
    pub fn enable_fast_latin(&mut self) {
        self.fast_latin_enabled = true;
    }

    /// `build(CollationData)`.
    ///
    /// # Errors
    ///
    /// The exceptions of `buildMappings` and the fast Latin builder.
    pub fn build(&mut self, data: &mut CollationData) -> Result<()> {
        self.build_mappings(data)?;
        if let Some(base) = &self.base {
            data.numeric_primary = base.numeric_primary;
            data.compressible_bytes = base.compressible_bytes.clone();
            data.num_scripts = base.num_scripts;
            data.scripts_index = base.scripts_index.clone();
            data.script_starts = base.script_starts.clone();
        }
        self.build_fast_latin_table(data)
    }

    /// Looks up CEs for s and appends them to the ces array.
    ///
    /// Equivalent to `getCEs(CharSequence, long[], int)`; returns the new
    /// length (which may be greater than the array capacity).
    ///
    /// # Errors
    ///
    /// The `IndexOutOfBoundsException` of too many context-sensitive
    /// mappings while building intermediate contexts.
    pub fn get_ces(&mut self, s: &[u16], ces: &mut [i64], ces_length: i32) -> Result<i32> {
        self.get_ces_from(s, 0, ces, ces_length)
    }

    /// `getCEs(CharSequence prefix, CharSequence s, long[], int)`.
    ///
    /// # Errors
    ///
    /// As [`get_ces`](Self::get_ces).
    pub fn get_ces_prefix(
        &mut self,
        prefix: &[u16],
        s: &[u16],
        ces: &mut [i64],
        ces_length: i32,
    ) -> Result<i32> {
        let prefix_length = prefix.len() as i32;
        if prefix_length == 0 {
            self.get_ces_from(s, 0, ces, ces_length)
        } else {
            let mut ps = prefix.to_vec();
            ps.extend_from_slice(s);
            self.get_ces_from(&ps, prefix_length, ces, ces_length)
        }
    }

    fn get_ce32_from_offset_ce32(&self, from_base: bool, c: i32, ce32: i32) -> i32 {
        let i = Collation::index_from_ce32(ce32);
        let data_ce = if from_base {
            self.base().ces[i as usize]
        } else {
            self.ce64s.element_ati(i)
        };
        let p = Collation::get_three_byte_primary_for_offset_data(c, data_ce);
        Collation::make_long_primary_ce32(p)
    }

    fn add_ce(&mut self, ce: i64) -> i32 {
        let length = self.ce64s.size();
        for i in 0..length {
            if ce == self.ce64s.element_ati(i) {
                return i;
            }
        }
        self.ce64s.add_element(ce);
        length
    }

    fn add_ce32_value(&mut self, ce32: i32) -> i32 {
        let length = self.ce32s.size();
        for i in 0..length {
            if ce32 == self.ce32s.element_ati(i) {
                return i;
            }
        }
        self.ce32s.add_element(ce32);
        length
    }

    fn add_conditional_ce32(&mut self, context: Vec<u16>, ce32: i32) -> Result<i32> {
        debug_assert!(!context.is_empty());
        let index = self.conditional_ce32s.len() as i32;
        if index > Collation::MAX_INDEX {
            return Err(index_out_of_bounds("too many context-sensitive mappings"));
            // BufferOverflowException is a better fit
            // but cannot be constructed with a message string.
        }
        self.conditional_ce32s
            .push(ConditionalCE32::new(context, ce32));
        Ok(index)
    }

    fn get_conditional_ce32_for_ce32(&self, ce32: i32) -> i32 {
        Collation::index_from_ce32(ce32)
    }

    fn make_builder_context_ce32(index: i32) -> i32 {
        Collation::make_ce32_from_tag_and_index(Collation::BUILDER_DATA_TAG, index)
    }

    fn is_builder_context_ce32(ce32: i32) -> bool {
        Collation::has_ce32_tag(ce32, Collation::BUILDER_DATA_TAG)
    }

    fn encode_one_ce_as_ce32(ce: i64) -> i32 {
        let p = ((ce as u64) >> 32) as i64;
        let lower32 = ce as i32;
        let t = lower32 & 0xffff;
        debug_assert!((t & 0xc000) != 0xc000); // Impossible case bits 11 mark special CE32s.
        if (ce & 0xffff00ff00ff) == 0 {
            // normal form ppppsstt
            p as i32 | ((lower32 as u32) >> 16) as i32 | (t >> 8)
        } else if (ce & 0xffffffffff) == Collation::COMMON_SEC_AND_TER_CE as i64 {
            // long-primary form ppppppC1
            Collation::make_long_primary_ce32(p)
        } else if p == 0 && (t & 0xff) == 0 {
            // long-secondary form ssssttC2
            Collation::make_long_secondary_ce32(lower32)
        } else {
            Collation::NO_CE32
        }
    }

    fn encode_one_ce(&mut self, ce: i64) -> Result<i32> {
        // Try to encode one CE as one CE32.
        let ce32 = Self::encode_one_ce_as_ce32(ce);
        if ce32 != Collation::NO_CE32 {
            return Ok(ce32);
        }
        let index = self.add_ce(ce);
        if index > Collation::MAX_INDEX {
            return Err(index_out_of_bounds("too many mappings"));
            // BufferOverflowException is a better fit
            // but cannot be constructed with a message string.
        }
        Ok(Collation::make_ce32_from_tag_index_and_length(
            Collation::EXPANSION_TAG,
            index,
            1,
        ))
    }

    fn encode_expansion(&mut self, ces: &[i64], start: i32, length: i32) -> Result<i32> {
        // See if this sequence of CEs has already been stored.
        let first = ces[start as usize];
        let ce64s_max = self.ce64s.size() - length;
        for i in 0..=ce64s_max {
            if first == self.ce64s.element_ati(i) {
                if i > Collation::MAX_INDEX {
                    return Err(index_out_of_bounds("too many mappings"));
                    // BufferOverflowException is a better fit
                    // but cannot be constructed with a message string.
                }
                let mut j = 1;
                loop {
                    if j == length {
                        return Ok(Collation::make_ce32_from_tag_index_and_length(
                            Collation::EXPANSION_TAG,
                            i,
                            length,
                        ));
                    }
                    if self.ce64s.element_ati(i + j) != ces[(start + j) as usize] {
                        break;
                    }
                    j += 1;
                }
            }
        }
        // Store the new sequence.
        let i = self.ce64s.size();
        if i > Collation::MAX_INDEX {
            return Err(index_out_of_bounds("too many mappings"));
            // BufferOverflowException is a better fit
            // but cannot be constructed with a message string.
        }
        for j in 0..length {
            self.ce64s.add_element(ces[(start + j) as usize]);
        }
        Ok(Collation::make_ce32_from_tag_index_and_length(
            Collation::EXPANSION_TAG,
            i,
            length,
        ))
    }

    fn encode_expansion32(&mut self, new_ce32s: &[i32], start: i32, length: i32) -> Result<i32> {
        // See if this sequence of CE32s has already been stored.
        let first = new_ce32s[start as usize];
        let ce32s_max = self.ce32s.size() - length;
        for i in 0..=ce32s_max {
            if first == self.ce32s.element_ati(i) {
                if i > Collation::MAX_INDEX {
                    return Err(index_out_of_bounds("too many mappings"));
                    // BufferOverflowException is a better fit
                    // but cannot be constructed with a message string.
                }
                let mut j = 1;
                loop {
                    if j == length {
                        return Ok(Collation::make_ce32_from_tag_index_and_length(
                            Collation::EXPANSION32_TAG,
                            i,
                            length,
                        ));
                    }
                    if self.ce32s.element_ati(i + j) != new_ce32s[(start + j) as usize] {
                        break;
                    }
                    j += 1;
                }
            }
        }
        // Store the new sequence.
        let i = self.ce32s.size();
        if i > Collation::MAX_INDEX {
            return Err(index_out_of_bounds("too many mappings"));
            // BufferOverflowException is a better fit
            // but cannot be constructed with a message string.
        }
        for j in 0..length {
            self.ce32s.add_element(new_ce32s[(start + j) as usize]);
        }
        Ok(Collation::make_ce32_from_tag_index_and_length(
            Collation::EXPANSION32_TAG,
            i,
            length,
        ))
    }

    fn set_cond_next(&mut self, cond: Cond, next: i32) {
        match cond {
            Cond::Head => self.head_next = next,
            Cond::Idx(i) => self.conditional_ce32s[i as usize].next = next,
        }
    }

    fn copy_from_base_ce32(&mut self, c: i32, mut ce32: i32, with_context: bool) -> Result<i32> {
        if !Collation::is_special_ce32(ce32) {
            return Ok(ce32);
        }
        let base = self.base().clone();
        match Collation::tag_from_ce32(ce32) {
            Collation::LONG_PRIMARY_TAG | Collation::LONG_SECONDARY_TAG | Collation::LATIN_EXPANSION_TAG => {
                // copy as is
            }
            Collation::EXPANSION32_TAG => {
                let index = Collation::index_from_ce32(ce32);
                let length = Collation::length_from_ce32(ce32);
                ce32 = self.encode_expansion32(&base.ce32s, index, length)?;
            }
            Collation::EXPANSION_TAG => {
                let index = Collation::index_from_ce32(ce32);
                let length = Collation::length_from_ce32(ce32);
                ce32 = self.encode_expansion(&base.ces, index, length)?;
            }
            Collation::PREFIX_TAG => {
                // Flatten prefixes and nested suffixes (contractions)
                // into a linear list of ConditionalCE32.
                let trie_index = Collation::index_from_ce32(ce32);
                ce32 = base.get_ce32_from_contexts(trie_index); // Default if no prefix match.
                if !with_context {
                    return self.copy_from_base_ce32(c, ce32, false);
                }
                let saved_head_next = std::mem::replace(&mut self.head_next, -1); // head = new ConditionalCE32("", 0)
                let mut context: Vec<u16> = vec![0];
                let mut index;
                if Collation::is_contraction_ce32(ce32) {
                    index = self.copy_contractions_from_base_ce32(&mut context, c, ce32, Cond::Head)?;
                } else {
                    ce32 = self.copy_from_base_ce32(c, ce32, true)?;
                    index = self.add_conditional_ce32(context.clone(), ce32)?;
                    self.head_next = index;
                }
                let mut cond = index; // the last ConditionalCE32 so far
                let prefixes = CharsTrie::iterator_of(base.contexts.clone(), trie_index + 2, 0);
                for entry in prefixes {
                    context.clear();
                    context.extend_from_slice(&entry.chars);
                    string_builder_reverse(&mut context);
                    context.insert(0, entry.chars.len() as u16);
                    ce32 = entry.value;
                    if Collation::is_contraction_ce32(ce32) {
                        index = self.copy_contractions_from_base_ce32(&mut context, c, ce32, Cond::Idx(cond))?;
                    } else {
                        ce32 = self.copy_from_base_ce32(c, ce32, true)?;
                        index = self.add_conditional_ce32(context.clone(), ce32)?;
                        self.conditional_ce32s[cond as usize].next = index;
                    }
                    cond = index;
                }
                ce32 = Self::make_builder_context_ce32(self.head_next);
                self.head_next = saved_head_next;
                self.context_chars.add(c)?;
            }
            Collation::CONTRACTION_TAG => {
                if !with_context {
                    let index = Collation::index_from_ce32(ce32);
                    ce32 = base.get_ce32_from_contexts(index); // Default if no suffix match.
                    return self.copy_from_base_ce32(c, ce32, false);
                }
                let saved_head_next = std::mem::replace(&mut self.head_next, -1); // head = new ConditionalCE32("", 0)
                let mut context: Vec<u16> = vec![0];
                self.copy_contractions_from_base_ce32(&mut context, c, ce32, Cond::Head)?;
                ce32 = Self::make_builder_context_ce32(self.head_next);
                self.head_next = saved_head_next;
                self.context_chars.add(c)?;
            }
            Collation::HANGUL_TAG => {
                return Err(LuceneError::unsupported_operation("We forbid tailoring of Hangul syllables."));
            }
            Collation::OFFSET_TAG => {
                ce32 = self.get_ce32_from_offset_ce32(true, c, ce32);
            }
            Collation::IMPLICIT_TAG => {
                ce32 = self.encode_one_ce(Collation::unassigned_ce_from_code_point(c))?;
            }
            _ => panic!(
                "java.lang.AssertionError: copyFromBaseCE32(c, ce32, withContext) requires ce32 == base.getFinalCE32(ce32)"
            ),
        }
        Ok(ce32)
    }

    /// Copies base contractions to a list of ConditionalCE32. Sets cond.next
    /// to the index of the first new item and returns the index of the last
    /// new item.
    fn copy_contractions_from_base_ce32(
        &mut self,
        context: &mut Vec<u16>,
        c: i32,
        mut ce32: i32,
        mut cond: Cond,
    ) -> Result<i32> {
        let base = self.base().clone();
        let trie_index = Collation::index_from_ce32(ce32);
        let mut index;
        if (ce32 & Collation::CONTRACT_SINGLE_CP_NO_MATCH) != 0 {
            // No match on the single code point.
            // We are underneath a prefix, and the default mapping is just
            // a fallback to the mappings for a shorter prefix.
            debug_assert!(context.len() > 1);
            index = -1;
        } else {
            ce32 = base.get_ce32_from_contexts(trie_index); // Default if no suffix match.
            debug_assert!(!Collation::is_contraction_ce32(ce32));
            ce32 = self.copy_from_base_ce32(c, ce32, true)?;
            index = self.add_conditional_ce32(context.clone(), ce32)?;
            self.set_cond_next(cond, index);
            cond = Cond::Idx(index);
        }

        let suffix_start = context.len();
        let suffixes = CharsTrie::iterator_of(base.contexts.clone(), trie_index + 2, 0);
        for entry in suffixes {
            context.extend_from_slice(&entry.chars);
            ce32 = self.copy_from_base_ce32(c, entry.value, true)?;
            index = self.add_conditional_ce32(context.clone(), ce32)?;
            self.set_cond_next(cond, index);
            // No need to update the unsafeBackwardSet because the tailoring set
            // is already a copy of the base set.
            cond = Cond::Idx(index);
            context.truncate(suffix_start);
        }
        debug_assert!(index >= 0);
        Ok(index)
    }

    fn get_jamo_ce32s(&mut self, jamo_ce32s: &mut [i32]) -> Result<bool> {
        let mut any_jamo_assigned = self.base.is_none(); // always set jamoCE32s in the base data
        let mut need_to_copy_from_base = false;
        for j in 0..CollationData::JAMO_CE32S_LENGTH {
            // Count across Jamo types.
            let jamo = Self::jamo_cp_from_index(j);
            let mut from_base = false;
            let mut ce32 = self.trie().get(jamo);
            any_jamo_assigned |= Collation::is_assigned_ce32(ce32);
            // TODO: Try to prevent [optimize [Jamo]] from counting as anyJamoAssigned.
            // (As of CLDR 24 [2013] the Korean tailoring does not optimize conjoining Jamo.)
            if ce32 == Collation::FALLBACK_CE32 {
                from_base = true;
                ce32 = self.base().get_ce32(jamo);
            }
            if Collation::is_special_ce32(ce32) {
                match Collation::tag_from_ce32(ce32) {
                    Collation::LONG_PRIMARY_TAG
                    | Collation::LONG_SECONDARY_TAG
                    | Collation::LATIN_EXPANSION_TAG => {
                        // Copy the ce32 as-is.
                    }
                    Collation::EXPANSION32_TAG
                    | Collation::EXPANSION_TAG
                    | Collation::PREFIX_TAG
                    | Collation::CONTRACTION_TAG => {
                        if from_base {
                            // Defer copying until we know if anyJamoAssigned.
                            ce32 = Collation::FALLBACK_CE32;
                            need_to_copy_from_base = true;
                        }
                    }
                    Collation::IMPLICIT_TAG => {
                        // An unassigned Jamo should only occur in tests with incomplete bases.
                        debug_assert!(from_base);
                        ce32 = Collation::FALLBACK_CE32;
                        need_to_copy_from_base = true;
                    }
                    Collation::OFFSET_TAG => {
                        ce32 = self.get_ce32_from_offset_ce32(from_base, jamo, ce32);
                    }
                    _ => {
                        // FALLBACK_TAG, RESERVED_TAG_3, BUILDER_DATA_TAG, DIGIT_TAG,
                        // U0000_TAG, HANGUL_TAG, LEAD_SURROGATE_TAG
                        panic!(
                            "java.lang.AssertionError: unexpected special tag in ce32=0x{:08x}",
                            ce32 as u32
                        );
                    }
                }
            }
            jamo_ce32s[j as usize] = ce32;
        }
        if any_jamo_assigned && need_to_copy_from_base {
            for j in 0..CollationData::JAMO_CE32S_LENGTH {
                if jamo_ce32s[j as usize] == Collation::FALLBACK_CE32 {
                    let jamo = Self::jamo_cp_from_index(j);
                    let base_ce32 = self.base().get_ce32(jamo);
                    jamo_ce32s[j as usize] = self.copy_from_base_ce32(jamo, base_ce32, true)?;
                }
            }
        }
        Ok(any_jamo_assigned)
    }

    fn set_digit_tags(&mut self) -> Result<()> {
        let digits = UnicodeSet::from_pattern(&units("[:Nd:]"))?;
        let mut iter = UnicodeSetIterator::new(&digits);
        while iter.next() {
            debug_assert!(iter.codepoint != IS_STRING);
            let c = iter.codepoint;
            let mut ce32 = self.trie().get(c);
            if ce32 != Collation::FALLBACK_CE32 && ce32 != Collation::UNASSIGNED_CE32 {
                let index = self.add_ce32_value(ce32);
                if index > Collation::MAX_INDEX {
                    return Err(index_out_of_bounds("too many mappings"));
                    // BufferOverflowException is a better fit
                    // but cannot be constructed with a message string.
                }
                ce32 = Collation::make_ce32_from_tag_index_and_length(
                    Collation::DIGIT_TAG,
                    index,
                    UCharacter::digit(c),
                ); // u_charDigitValue(c)
                self.trie_mut().set(c, ce32)?;
            }
        }
        Ok(())
    }

    fn set_lead_surrogates(&mut self) -> Result<()> {
        for lead in 0xd800u16..0xdc00 {
            let mut lead_value = -1;
            // utrie2_enumForLeadSurrogate(trie, lead, null, , &value);
            let ranges: Vec<_> = self.trie().iterator_for_lead_surrogate(lead)?.collect();
            for range in ranges {
                // The rest of this loop is equivalent to C++ enumRangeLeadValue().
                let mut value = range.value;
                if value == Collation::UNASSIGNED_CE32 {
                    value = Collation::LEAD_ALL_UNASSIGNED;
                } else if value == Collation::FALLBACK_CE32 {
                    value = Collation::LEAD_ALL_FALLBACK;
                } else {
                    lead_value = Collation::LEAD_MIXED;
                    break;
                }
                if lead_value < 0 {
                    lead_value = value;
                } else if lead_value != value {
                    lead_value = Collation::LEAD_MIXED;
                    break;
                }
            }
            self.trie_mut().set_for_lead_surrogate_code_unit(
                lead,
                Collation::make_ce32_from_tag_and_index(Collation::LEAD_SURROGATE_TAG, 0)
                    | lead_value,
            )?;
        }
        Ok(())
    }

    fn build_mappings(&mut self, data: &mut CollationData) -> Result<()> {
        if !self.is_mutable() {
            return Err(LuceneError::illegal_state(
                "attempt to build() after build()",
            ));
        }

        self.build_contexts()?;

        let mut jamo_ce32s = vec![0i32; CollationData::JAMO_CE32S_LENGTH as usize];
        let mut jamo_index = -1;
        if self.get_jamo_ce32s(&mut jamo_ce32s)? {
            jamo_index = self.ce32s.size();
            for &v in &jamo_ce32s {
                self.ce32s.add_element(v);
            }
            // Small optimization: Use a bit in the Hangul ce32
            // to indicate that none of the Jamo CE32s are isSpecialCE32()
            // (as it should be in the root collator).
            // It allows CollationIterator to avoid recursive function calls and per-Jamo tests.
            // In order to still have good trie compression and keep this code simple,
            // we only set this flag if a whole block of 588 Hangul syllables starting with
            // a common leading consonant (Jamo L) has this property.
            let mut is_any_jamo_vt_special = false;
            for &v in &jamo_ce32s[Hangul::JAMO_L_COUNT as usize..] {
                if Collation::is_special_ce32(v) {
                    is_any_jamo_vt_special = true;
                    break;
                }
            }
            let hangul_ce32 = Collation::make_ce32_from_tag_and_index(Collation::HANGUL_TAG, 0);
            let mut c = Hangul::HANGUL_BASE;
            for &jamo_l in &jamo_ce32s[..Hangul::JAMO_L_COUNT as usize] {
                // iterate over the Jamo L
                let mut ce32 = hangul_ce32;
                if !is_any_jamo_vt_special && !Collation::is_special_ce32(jamo_l) {
                    ce32 |= Collation::HANGUL_NO_SPECIAL_JAMO;
                }
                let limit = c + Hangul::JAMO_VT_COUNT;
                self.trie_mut().set_range(c, limit - 1, ce32, true)?;
                c = limit;
            }
        } else {
            // Copy the Hangul CE32s from the base in blocks per Jamo L,
            // assuming that HANGUL_NO_SPECIAL_JAMO is set or not set for whole blocks.
            let mut c = Hangul::HANGUL_BASE;
            while c < Hangul::HANGUL_LIMIT {
                let ce32 = self.base().get_ce32(c);
                debug_assert!(Collation::has_ce32_tag(ce32, Collation::HANGUL_TAG));
                let limit = c + Hangul::JAMO_VT_COUNT;
                self.trie_mut().set_range(c, limit - 1, ce32, true)?;
                c = limit;
            }
        }

        self.set_digit_tags()?;
        self.set_lead_surrogates()?;

        // For U+0000, move its normal ce32 into CE32s[0] and set U0000_TAG.
        let ce32_0 = self.trie().get(0);
        self.ce32s.set_element_at(ce32_0, 0);
        self.trie_mut().set(
            0,
            Collation::make_ce32_from_tag_and_index(Collation::U0000_TAG, 0),
        )?;

        data.trie = Some(Arc::new(self.trie_mut().to_trie2_32()?));

        // Mark each lead surrogate as "unsafe"
        // if any of its 1024 associated supplementary code points is "unsafe".
        let mut c = 0x10000;
        let mut lead = 0xd800;
        while lead < 0xdc00 {
            if self.unsafe_backward_set.contains_some_range(c, c + 0x3ff)? {
                self.unsafe_backward_set.add(lead)?;
            }
            lead += 1;
            c += 0x400;
        }
        self.unsafe_backward_set.freeze();

        data.ce32s = Arc::from(self.ce32s.get_buffer());
        data.ces = Arc::from(self.ce64s.get_buffer());
        data.contexts = Arc::from(&self.contexts[..]);

        data.base = self.base.clone();
        if jamo_index >= 0 {
            data.jamo_ce32s = Arc::from(jamo_ce32s); // C++: data.ce32s + jamoIndex
        } else {
            data.jamo_ce32s = self.base().jamo_ce32s.clone();
        }

        data.unsafe_backward_set = Some(Arc::new(self.unsafe_backward_set.clone()));
        Ok(())
    }

    fn clear_contexts(&mut self) {
        self.contexts.clear();
        // Incrementing the contexts build "era" invalidates all of the builtCE32
        // from before this clearContexts() call.
        // Simpler than finding and resetting all of those fields.
        self.contexts_era += 1;
    }

    fn build_contexts(&mut self) -> Result<()> {
        // Ignore abandoned lists and the cached builtCE32,
        // and build all contexts from scratch.
        self.clear_contexts();
        let context_chars = self.context_chars.clone();
        let mut iter = UnicodeSetIterator::new(&context_chars);
        while iter.next() {
            debug_assert!(iter.codepoint != IS_STRING);
            let c = iter.codepoint;
            let mut ce32 = self.trie().get(c);
            if !Self::is_builder_context_ce32(ce32) {
                panic!(
                    "java.lang.AssertionError: Impossible: No context data for c in contextChars."
                );
            }
            let cond = self.get_conditional_ce32_for_ce32(ce32);
            ce32 = self.build_context(cond)?;
            self.trie_mut().set(c, ce32)?;
        }
        Ok(())
    }

    fn cc(&self, i: i32) -> &ConditionalCE32 {
        &self.conditional_ce32s[i as usize]
    }

    fn build_context(&mut self, head: i32) -> Result<i32> {
        // The list head must have no context.
        debug_assert!(!self.cc(head).has_context());
        // The list head must be followed by one or more nodes that all do have context.
        debug_assert!(self.cc(head).next >= 0);
        let mut prefix_builder: Option<CharsTrieBuilder> = None;
        let mut contraction_builder: Option<CharsTrieBuilder> = None;
        // This outer loop goes from each prefix to the next.
        // For each prefix it finds the one or more same-prefix entries (firstCond..lastCond).
        // If there are multiple suffixes for the same prefix,
        // then an inner loop builds a contraction trie for them.
        let mut cond = head;
        loop {
            // After the list head, the prefix or suffix can be empty, but not both.
            debug_assert!(cond == head || self.cc(cond).has_context());
            let prefix_length = self.cc(cond).prefix_length();
            let mut prefix: Vec<u16> =
                self.cc(cond).context[..(prefix_length + 1) as usize].to_vec();
            let prefix_string = prefix.clone();
            // Collect all contraction suffixes for one prefix.
            let first_cond = cond;
            let mut last_cond;
            loop {
                last_cond = cond;
                // Clear the defaultCE32 fields as we go.
                // They are left over from building a previous version of this list of contexts.
                //
                // One of the code paths below may copy a preceding defaultCE32
                // into its emptySuffixCE32.
                // If a new suffix has been inserted before what used to be
                // the firstCond for its prefix, then that previous firstCond could still
                // contain an outdated defaultCE32 from an earlier buildContext() and
                // result in an incorrect emptySuffixCE32.
                // So we reset all defaultCE32 before reading and setting new values.
                self.conditional_ce32s[cond as usize].default_ce32 = Collation::NO_CE32;
                let next = self.cc(cond).next;
                if next < 0 {
                    break;
                }
                cond = next;
                if !self.cc(cond).context.starts_with(&prefix_string) {
                    break;
                }
            }
            let ce32;
            let suffix_start = prefix_length + 1; // == prefix.length()
            if self.cc(last_cond).context.len() as i32 == suffix_start {
                // One prefix without contraction suffix.
                debug_assert!(first_cond == last_cond);
                ce32 = self.cc(last_cond).ce32;
                cond = last_cond;
            } else {
                // Build the contractions trie.
                match contraction_builder.as_mut() {
                    None => contraction_builder = Some(CharsTrieBuilder::new()),
                    Some(b) => {
                        b.clear();
                    }
                }
                // Entry for an empty suffix, to be stored before the trie.
                let mut empty_suffix_ce32 = Collation::NO_CE32; // Will always be set to a real value.
                let mut flags = 0;
                if self.cc(first_cond).context.len() as i32 == suffix_start {
                    // There is a mapping for the prefix and the single character c. (p|c)
                    // If no other suffix matches, then we return this value.
                    empty_suffix_ce32 = self.cc(first_cond).ce32;
                    cond = self.cc(first_cond).next;
                } else {
                    // There is no mapping for the prefix and just the single character.
                    // (There is no p|c, only p|cd, p|ce etc.)
                    flags |= Collation::CONTRACT_SINGLE_CP_NO_MATCH;
                    // When the prefix matches but none of the prefix-specific suffixes,
                    // then we fall back to the mappings with the next-longest prefix,
                    // and ultimately to mappings with no prefix.
                    // Each fallback might be another set of contractions.
                    // For example, if there are mappings for ch, p|cd, p|ce, but not for p|c,
                    // then in text "pch" we find the ch contraction.
                    cond = head;
                    loop {
                        let length = self.cc(cond).prefix_length();
                        if length == prefix_length {
                            break;
                        }
                        let cc = self.cc(cond);
                        if cc.default_ce32 != Collation::NO_CE32
                            && (length == 0 || {
                                // prefixString.regionMatches(prefix.length() - length, cond.context, 1, length)
                                let off = prefix.len() as i32 - length;
                                off >= 0
                                    && prefix_string[off as usize..(off + length) as usize]
                                        == cc.context[1..(1 + length) as usize]
                            })
                        {
                            empty_suffix_ce32 = cc.default_ce32;
                        }
                        cond = cc.next;
                    }
                    cond = first_cond;
                }
                // Optimization: Set a flag when
                // the first character of every contraction suffix has lccc!=0.
                // Short-circuits contraction matching when a normal letter follows.
                flags |= Collation::CONTRACT_NEXT_CCC;
                // Add all of the non-empty suffixes into the contraction trie.
                loop {
                    let suffix: Vec<u16> = self.cc(cond).context[suffix_start as usize..].to_vec();
                    let mut fcd16 = self.nfc_impl.get_fcd16(code_point_at(&suffix, 0));
                    if fcd16 <= 0xff {
                        flags &= !Collation::CONTRACT_NEXT_CCC;
                    }
                    fcd16 = self
                        .nfc_impl
                        .get_fcd16(code_point_before(&suffix, suffix.len() as i32));
                    if fcd16 > 0xff {
                        // The last suffix character has lccc!=0, allowing for discontiguous contractions.
                        flags |= Collation::CONTRACT_TRAILING_CCC;
                    }
                    let v = self.cc(cond).ce32;
                    contraction_builder
                        .as_mut()
                        .expect("created")
                        .add(&suffix, v)?;
                    if cond == last_cond {
                        break;
                    }
                    cond = self.cc(cond).next;
                }
                let index = self.add_context_trie(
                    empty_suffix_ce32,
                    contraction_builder.as_mut().expect("created"),
                )?;
                if index > Collation::MAX_INDEX {
                    return Err(index_out_of_bounds("too many context-sensitive mappings"));
                    // BufferOverflowException is a better fit
                    // but cannot be constructed with a message string.
                }
                ce32 = Collation::make_ce32_from_tag_and_index(Collation::CONTRACTION_TAG, index)
                    | flags;
            }
            debug_assert!(cond == last_cond);
            self.conditional_ce32s[first_cond as usize].default_ce32 = ce32;
            if prefix_length == 0 {
                if self.cc(cond).next < 0 {
                    // No non-empty prefixes, only contractions.
                    return Ok(ce32);
                }
            } else {
                prefix.remove(0); // Remove the length unit.
                string_builder_reverse(&mut prefix);
                prefix_builder
                    .get_or_insert_with(CharsTrieBuilder::new)
                    .add(&prefix, ce32)?;
                if self.cc(cond).next < 0 {
                    break;
                }
            }
            cond = self.cc(cond).next;
        }
        debug_assert!(self.cc(head).default_ce32 != Collation::NO_CE32);
        let head_default = self.cc(head).default_ce32;
        let mut prefix_builder =
            prefix_builder.unwrap_or_else(|| panic!("java.lang.NullPointerException"));
        let index = self.add_context_trie(head_default, &mut prefix_builder)?;
        if index > Collation::MAX_INDEX {
            return Err(index_out_of_bounds("too many context-sensitive mappings"));
            // BufferOverflowException is a better fit
            // but cannot be constructed with a message string.
        }
        Ok(Collation::make_ce32_from_tag_and_index(
            Collation::PREFIX_TAG,
            index,
        ))
    }

    fn add_context_trie(
        &mut self,
        default_ce32: i32,
        trie_builder: &mut CharsTrieBuilder,
    ) -> Result<i32> {
        let mut context: Vec<u16> = Vec::new();
        context.push((default_ce32 >> 16) as u16);
        context.push(default_ce32 as u16);
        context
            .extend_from_slice(&trie_builder.build_char_sequence(StringTrieBuilderOption::Small)?);
        let mut index = index_of(&self.contexts, &context, 0);
        if index < 0 {
            index = self.contexts.len() as i32;
            self.contexts.extend_from_slice(&context);
        }
        Ok(index)
    }

    fn build_fast_latin_table(&mut self, data: &mut CollationData) -> Result<()> {
        if !self.fast_latin_enabled {
            return Ok(());
        }

        let mut fast_latin_builder = CollationFastLatinBuilder::new();
        if fast_latin_builder.for_data(data)? {
            let mut header: Arc<[u16]> = Arc::from(fast_latin_builder.get_header());
            let mut table: Arc<[u16]> = Arc::from(fast_latin_builder.get_table());
            self.fast_latin_builder = Some(fast_latin_builder);
            if let Some(base) = &self.base {
                if base.fast_latin_table_header.as_deref() == Some(&header[..])
                    && base.fast_latin_table.as_deref() == Some(&table[..])
                {
                    // Same fast Latin table as in the base, use that one instead.
                    self.fast_latin_builder = None;
                    header = base.fast_latin_table_header.clone().expect("checked");
                    table = base.fast_latin_table.clone().expect("checked");
                }
            }
            data.fast_latin_table_header = Some(header);
            data.fast_latin_table = Some(table);
        } else {
            self.fast_latin_builder = None;
        }
        Ok(())
    }

    fn get_ces_from(
        &mut self,
        s: &[u16],
        start: i32,
        ces: &mut [i64],
        ces_length: i32,
    ) -> Result<i32> {
        let mut coll_iter = match self.coll_iter.take() {
            Some(it) => it,
            None => {
                DataBuilderCollationIterator::new(self, CollationData::new(self.nfc_impl.clone()))
            }
        };
        let r = coll_iter.fetch_ces(self, s, start, ces, ces_length);
        self.coll_iter = Some(coll_iter);
        r
    }

    fn jamo_cp_from_index(mut i: i32) -> i32 {
        // 0 <= i < CollationData.JAMO_CE32S_LENGTH = 19 + 21 + 27
        if i < Hangul::JAMO_L_COUNT {
            return Hangul::JAMO_L_BASE + i;
        }
        i -= Hangul::JAMO_L_COUNT;
        if i < Hangul::JAMO_V_COUNT {
            return Hangul::JAMO_V_BASE + i;
        }
        i -= Hangul::JAMO_V_COUNT;
        // i < 27
        Hangul::JAMO_T_BASE + 1 + i
    }

    /// `isMutable()`.
    pub fn is_mutable(&self) -> bool {
        // C++ tests !(trie == NULL || utrie2_isFrozen(trie))
        // but Java Trie2Writable does not have an observable isFrozen() state.
        self.trie.is_some() && !self.unsafe_backward_set.is_frozen()
    }
}

/// `CollationDataBuilder.CopyHelper` (private).
struct CopyHelper<'a> {
    src: &'a CollationDataBuilder,
    modifier: &'a mut dyn CEModifier,
    modified_ces: [i64; Collation::MAX_EXPANSION_LENGTH as usize],
}

impl CopyHelper<'_> {
    fn copy_range_ce32(
        &mut self,
        dest: &mut CollationDataBuilder,
        start: i32,
        end: i32,
        mut ce32: i32,
    ) -> Result<()> {
        ce32 = self.copy_ce32(dest, ce32)?;
        dest.trie_mut().set_range(start, end, ce32, true)?;
        if CollationDataBuilder::is_builder_context_ce32(ce32) {
            dest.context_chars.add_range(start, end)?;
        }
        Ok(())
    }

    fn copy_ce32(&mut self, dest: &mut CollationDataBuilder, mut ce32: i32) -> Result<i32> {
        if !Collation::is_special_ce32(ce32) {
            let ce = self.modifier.modify_ce32(ce32);
            if ce != Collation::NO_CE {
                ce32 = dest.encode_one_ce(ce)?;
            }
        } else {
            let tag = Collation::tag_from_ce32(ce32);
            if tag == Collation::EXPANSION32_TAG {
                let src_ce32s = self.src.ce32s.get_buffer();
                let src_index = Collation::index_from_ce32(ce32);
                let length = Collation::length_from_ce32(ce32);
                // Inspect the source CE32s. Just copy them if none are modified.
                // Otherwise copy to modifiedCEs, with modifications.
                let mut is_modified = false;
                for i in 0..length {
                    ce32 = src_ce32s[(src_index + i) as usize];
                    let ce;
                    if Collation::is_special_ce32(ce32) || {
                        ce = self.modifier.modify_ce32(ce32);
                        ce == Collation::NO_CE
                    } {
                        if is_modified {
                            self.modified_ces[i as usize] = Collation::ce_from_ce32(ce32);
                        }
                    } else {
                        if !is_modified {
                            for j in 0..i {
                                self.modified_ces[j as usize] =
                                    Collation::ce_from_ce32(src_ce32s[(src_index + j) as usize]);
                            }
                            is_modified = true;
                        }
                        self.modified_ces[i as usize] = ce;
                    }
                }
                if is_modified {
                    let ces = self.modified_ces;
                    ce32 = dest.encode_ces(&ces, length)?;
                } else {
                    ce32 = dest.encode_expansion32(src_ce32s, src_index, length)?;
                }
            } else if tag == Collation::EXPANSION_TAG {
                let src_ces = self.src.ce64s.get_buffer();
                let src_index = Collation::index_from_ce32(ce32);
                let length = Collation::length_from_ce32(ce32);
                // Inspect the source CEs. Just copy them if none are modified.
                // Otherwise copy to modifiedCEs, with modifications.
                let mut is_modified = false;
                for i in 0..length {
                    let src_ce = src_ces[(src_index + i) as usize];
                    let ce = self.modifier.modify_ce(src_ce);
                    if ce == Collation::NO_CE {
                        if is_modified {
                            self.modified_ces[i as usize] = src_ce;
                        }
                    } else {
                        if !is_modified {
                            for j in 0..i {
                                self.modified_ces[j as usize] = src_ces[(src_index + j) as usize];
                            }
                            is_modified = true;
                        }
                        self.modified_ces[i as usize] = ce;
                    }
                }
                if is_modified {
                    let ces = self.modified_ces;
                    ce32 = dest.encode_ces(&ces, length)?;
                } else {
                    ce32 = dest.encode_expansion(src_ces, src_index, length)?;
                }
            } else if tag == Collation::BUILDER_DATA_TAG {
                // Copy the list of ConditionalCE32.
                let mut cond = self
                    .src
                    .cc(self.src.get_conditional_ce32_for_ce32(ce32))
                    .clone();
                debug_assert!(!cond.has_context());
                let copied = self.copy_ce32(dest, cond.ce32)?;
                let mut dest_index = dest.add_conditional_ce32(cond.context.clone(), copied)?;
                ce32 = CollationDataBuilder::make_builder_context_ce32(dest_index);
                while cond.next >= 0 {
                    cond = self.src.cc(cond.next).clone();
                    let prev_dest_cond = dest_index;
                    let copied = self.copy_ce32(dest, cond.ce32)?;
                    dest_index = dest.add_conditional_ce32(cond.context.clone(), copied)?;
                    let suffix_start = cond.prefix_length() + 1;
                    dest.unsafe_backward_set
                        .add_all_str(&cond.context[suffix_start as usize..])?;
                    dest.conditional_ce32s[prev_dest_cond as usize].next = dest_index;
                }
            } else {
                // Just copy long CEs and Latin mini expansions (and other expected values) as is,
                // assuming that the modifier would not modify them.
                debug_assert!(
                    tag == Collation::LONG_PRIMARY_TAG
                        || tag == Collation::LONG_SECONDARY_TAG
                        || tag == Collation::LATIN_EXPANSION_TAG
                        || tag == Collation::HANGUL_TAG
                );
            }
        }
        Ok(ce32)
    }
}

fn enum_range_for_copy(
    start: i32,
    end: i32,
    value: i32,
    helper: &mut CopyHelper<'_>,
    dest: &mut CollationDataBuilder,
) -> Result<()> {
    if value != Collation::UNASSIGNED_CE32 && value != Collation::FALLBACK_CE32 {
        helper.copy_range_ce32(dest, start, end, value)?;
    }
    Ok(())
}

/// Collation element iterator over the builder's data, for `getCEs()`.
/// Works on a builder whose trie is a `Trie2Writable` (not yet frozen),
/// so that the CollationIterator does not need the finished trie for
/// handling the LEAD_SURROGATE_TAG.
///
/// Equivalent to `CollationDataBuilder.DataBuilderCollationIterator`
/// (private). Java's iterator keeps a reference to its builder, which also
/// owns it; here the iterator is stored in the builder and borrows it only
/// while fetching ([`BoundIter`]).
#[derive(Debug)]
struct DataBuilderCollationIterator {
    base: CollationIteratorBase,
    jamo_ce32s: Arc<[i32]>,
    s: Vec<u16>,
    pos: i32,
}

/// The iterator while it fetches, with its builder.
struct BoundIter<'a> {
    it: &'a mut DataBuilderCollationIterator,
    builder: &'a mut CollationDataBuilder,
    /// The exception `getCE32FromBuilderData` throws out of the iteration
    /// (the second `IndexOutOfBoundsException` of `buildContext`); the
    /// iteration finishes the current code point and `fetchCEs` throws it.
    pending: Option<LuceneError>,
}

impl DataBuilderCollationIterator {
    fn new(b: &CollationDataBuilder, mut new_data: CollationData) -> Self {
        new_data.base = b.base.clone();
        // Set all of the jamoCE32s[] to indirection CE32s.
        let mut jamo_ce32s = vec![0i32; CollationData::JAMO_CE32S_LENGTH as usize];
        for (j, v) in jamo_ce32s.iter_mut().enumerate() {
            // Count across Jamo types.
            let jamo = CollationDataBuilder::jamo_cp_from_index(j as i32);
            *v = Collation::make_ce32_from_tag_and_index(Collation::BUILDER_DATA_TAG, jamo)
                | IS_BUILDER_JAMO_CE32;
        }
        let jamo_ce32s: Arc<[i32]> = Arc::from(jamo_ce32s);
        new_data.jamo_ce32s = jamo_ce32s.clone();
        Self {
            base: CollationIteratorBase::with_numeric(Arc::new(new_data), false),
            jamo_ce32s,
            s: Vec::new(),
            pos: 0,
        }
    }

    fn fetch_ces(
        &mut self,
        builder: &mut CollationDataBuilder,
        str: &[u16],
        start: i32,
        ces: &mut [i64],
        mut ces_length: i32,
    ) -> Result<i32> {
        // Set the pointers each time, in case they changed due to reallocation.
        {
            let d = Arc::make_mut(&mut self.base.data);
            d.ce32s = Arc::from(&builder.ce32s.get_buffer()[..builder.ce32s.size() as usize]);
            d.ces = Arc::from(&builder.ce64s.get_buffer()[..builder.ce64s.size() as usize]);
            d.contexts = Arc::from(&builder.contexts[..]);
            d.jamo_ce32s = self.jamo_ce32s.clone();
        }
        let mut bound = BoundIter {
            it: self,
            builder,
            pending: None,
        };
        // Modified copy of CollationIterator.nextCE() and CollationIterator.nextCEFromCE32().
        bound.it.base.reset();
        bound.it.s = str.to_vec();
        bound.it.pos = start;
        while bound.it.pos < bound.it.s.len() as i32 {
            // No need to keep all CEs in the iterator buffer.
            bound.it.base.clear_ces();
            let c = code_point_at(&bound.it.s, bound.it.pos);
            bound.it.pos += char_count(c);
            let mut ce32 = bound.builder.trie().get(c);
            let d: Arc<CollationData>;
            if ce32 == Collation::FALLBACK_CE32 {
                d = bound.builder.base().clone();
                ce32 = d.get_ce32(c);
            } else {
                d = bound.it.base.data.clone();
            }
            bound.append_ces_from_ce32(d, c, ce32, true);
            if let Some(e) = bound.pending.take() {
                return Err(e);
            }
            for i in 0..bound.it.base.get_ces_length() {
                let ce = bound.it.base.get_ce(i);
                if ce != 0 {
                    if ces_length < Collation::MAX_EXPANSION_LENGTH {
                        ces[ces_length as usize] = ce;
                    }
                    ces_length += 1;
                }
            }
        }
        Ok(ces_length)
    }
}

impl CollationIterator for BoundIter<'_> {
    fn base(&self) -> &CollationIteratorBase {
        &self.it.base
    }

    fn base_mut(&mut self) -> &mut CollationIteratorBase {
        &mut self.it.base
    }

    fn reset_to_offset(&mut self, new_offset: i32) {
        self.it.base.reset();
        self.it.pos = new_offset;
    }

    fn get_offset(&self) -> i32 {
        self.it.pos
    }

    fn next_code_point(&mut self) -> i32 {
        if self.it.pos == self.it.s.len() as i32 {
            return Collation::SENTINEL_CP;
        }
        let c = code_point_at(&self.it.s, self.it.pos);
        self.it.pos += char_count(c);
        c
    }

    fn previous_code_point(&mut self) -> i32 {
        if self.it.pos == 0 {
            return Collation::SENTINEL_CP;
        }
        let c = code_point_before(&self.it.s, self.it.pos);
        self.it.pos -= char_count(c);
        c
    }

    fn forward_num_code_points(&mut self, num: i32) {
        self.it.pos = offset_by_code_points(&self.it.s, self.it.pos, num);
    }

    fn backward_num_code_points(&mut self, num: i32) {
        self.it.pos = offset_by_code_points(&self.it.s, self.it.pos, -num);
    }

    fn get_data_ce32(&mut self, c: i32) -> i32 {
        self.builder.trie().get(c)
    }

    fn get_ce32_from_builder_data(&mut self, ce32: i32) -> i32 {
        debug_assert!(Collation::has_ce32_tag(ce32, Collation::BUILDER_DATA_TAG));
        if (ce32 & IS_BUILDER_JAMO_CE32) != 0 {
            let jamo = Collation::index_from_ce32(ce32);
            self.builder.trie().get(jamo)
        } else {
            let cond = self.builder.get_conditional_ce32_for_ce32(ce32);
            let c = self.builder.cc(cond);
            if c.built_ce32 == Collation::NO_CE32 || c.era != self.builder.contexts_era {
                // Build the context-sensitive mappings into their runtime form and cache the result.
                let built = match self.builder.build_context(cond) {
                    Ok(v) => Ok(v),
                    Err(e) if is_index_out_of_bounds(&e) => {
                        self.builder.clear_contexts();
                        self.builder.build_context(cond)
                    }
                    Err(e) => Err(e),
                };
                match built {
                    Ok(v) => {
                        let era = self.builder.contexts_era;
                        let c = &mut self.builder.conditional_ce32s[cond as usize];
                        c.built_ce32 = v;
                        c.era = era;
                        let contexts: Arc<[u16]> = Arc::from(&self.builder.contexts[..]);
                        Arc::make_mut(&mut self.it.base.data).contexts = contexts;
                    }
                    Err(e) => {
                        // Java throws out of the iteration; fall back to the base
                        // data for the rest of this code point (no builder change).
                        self.pending = Some(e);
                        return Collation::FALLBACK_CE32;
                    }
                }
            }
            self.builder.cc(cond).built_ce32
        }
    }
}
