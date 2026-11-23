// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.coll.CollationBuilder`.

use std::sync::{Arc, LazyLock};

use rucene::error::{LuceneError, Result};

use crate::jdk::{char_count, code_point_at, code_point_before, offset_by_code_points, units};
use crate::r#impl::coll::collation::Collation;
use crate::r#impl::coll::collation_data::CollationData;
use crate::r#impl::coll::collation_data_builder::{CEModifier, CollationDataBuilder};
use crate::r#impl::coll::collation_fast_latin::CollationFastLatin;
use crate::r#impl::coll::collation_iterator::CollationIterator;
use crate::r#impl::coll::collation_loader::CollationLoader;
use crate::r#impl::coll::collation_root_elements::CollationRootElements;
use crate::r#impl::coll::collation_rule_parser::{CollationRuleParser, Importer, Position, Sink, POSITION_VALUES, POS_BASE, POS_LEAD};
use crate::r#impl::coll::collation_tailoring::CollationTailoring;
use crate::r#impl::coll::collation_weights::CollationWeights;
use crate::r#impl::coll::u_vector32::UVector32;
use crate::r#impl::coll::u_vector64::UVector64;
use crate::r#impl::coll::utf16_collation_iterator::UTF16CollationIterator;
use crate::r#impl::norm2_all_modes::{Norm2AllModes, Normalizer2WithImpl};
use crate::r#impl::normalizer2_impl::{Hangul, Normalizer2Impl};
use crate::lang::u_script::UScript;
use crate::text::canonical_iterator::CanonicalIterator;
use crate::text::collator;
use crate::text::normalizer2::{self, Normalizer2};
use crate::text::unicode_set::UnicodeSet;
use crate::text::unicode_set_iterator::{UnicodeSetIterator, IS_STRING};
use crate::util::icu_input_too_long_exception::ICUInputTooLongException;
use crate::util::u_locale::ULocale;

const DEBUG: bool = false;

/// `CollationBuilder.BundleImporter` (private).
#[derive(Debug, Clone, Copy, Default)]
struct BundleImporter;

impl Importer for BundleImporter {
    fn get_rules(&self, locale_id: &str, collation_type: &str) -> Result<Vec<u16>> {
        CollationLoader::load_rules(&ULocale::new(locale_id)?, collation_type)
    }
}

// ICU-22517
// This constant defines a limit for the addOnlyClosure to return
// error, to avoid taking a long time for canonical closure expansion.
// Please let us know if you have a reasonable use case that needed
// for a practical Collation rule that needs to increase this limit.
// This value is needed for compiling a rule with eight Hangul syllables such as
// "&a=b쫊쫊쫊쫊쫊쫊쫊쫊" without error, which should be more than realistic
// usage.
const K_CLOSURE_LOOP_LIMIT: i32 = 6560;

/// `COMPOSITES`: `[:NFD_QC=N:]` minus Hangul (decomposed on the fly
/// during collation).
static COMPOSITES: LazyLock<UnicodeSet> = LazyLock::new(|| {
    let mut set = UnicodeSet::from_pattern(&units("[:NFD_QC=N:]"))
        .unwrap_or_else(|e| panic!("java.lang.ExceptionInInitializerError: {e}"));
    // Hangul is decomposed on the fly during collation.
    set.remove_range(Hangul::HANGUL_BASE, Hangul::HANGUL_END)
        .unwrap_or_else(|e| panic!("java.lang.ExceptionInInitializerError: {e}"));
    set
});

const MAX_INDEX: i32 = 0xfffff;
/// Node bit 6 is set on a primary node if there are nodes
/// with secondary values below the common secondary weight (05).
const HAS_BEFORE2: i32 = 0x40;
/// Node bit 5 is set on a primary or secondary node if there are nodes
/// with tertiary values below the common tertiary weight (05).
const HAS_BEFORE3: i32 = 0x20;
/// Node bit 3 distinguishes a tailored node, which has no weight value,
/// from a node with an explicit (root or default) weight.
const IS_TAILORED: i32 = 8;

fn ushr32(x: i64) -> i64 {
    ((x as u64) >> 32) as i64
}

fn node_from_weight32(weight32: i64) -> i64 {
    weight32 << 32
}
fn node_from_weight16(weight16: i32) -> i64 {
    (weight16 as i64) << 48
}
fn node_from_previous_index(previous: i32) -> i64 {
    (previous as i64) << 28
}
fn node_from_next_index(next: i32) -> i64 {
    (next << 8) as i64
}
fn node_from_strength(strength: i32) -> i64 {
    strength as i64
}
fn weight32_from_node(node: i64) -> i64 {
    ushr32(node)
}
fn weight16_from_node(node: i64) -> i32 {
    (node >> 48) as i32 & 0xffff
}
fn previous_index_from_node(node: i64) -> i32 {
    (node >> 28) as i32 & MAX_INDEX
}
fn next_index_from_node(node: i64) -> i32 {
    (node as i32 >> 8) & MAX_INDEX
}
fn strength_from_node(node: i64) -> i32 {
    node as i32 & 3
}
fn node_has_before2(node: i64) -> bool {
    (node & HAS_BEFORE2 as i64) != 0
}
fn node_has_before3(node: i64) -> bool {
    (node & HAS_BEFORE3 as i64) != 0
}
fn node_has_any_before(node: i64) -> bool {
    (node & (HAS_BEFORE2 | HAS_BEFORE3) as i64) != 0
}
fn is_tailored_node(node: i64) -> bool {
    (node & IS_TAILORED as i64) != 0
}
fn change_node_previous_index(node: i64, previous: i32) -> i64 {
    (node & (0xffff00000fffffffu64 as i64)) | node_from_previous_index(previous)
}
fn change_node_next_index(node: i64, next: i32) -> i64 {
    (node & (0xfffffffff00000ffu64 as i64)) | node_from_next_index(next)
}

fn temp_ce_from_index_and_strength(index: i32, strength: i32) -> i64 {
    // CE byte offsets, to ensure valid CE bytes, and case bits 11
    0x4040000006002000i64
        // index bits 19..13 -> primary byte 1 = CE bits 63..56 (byte values 40..BF)
        + (((index & 0xfe000) as i64) << 43)
        // index bits 12..6 -> primary byte 2 = CE bits 55..48 (byte values 40..BF)
        + (((index & 0x1fc0) as i64) << 42)
        // index bits 5..0 -> secondary byte 1 = CE bits 31..24 (byte values 06..45)
        + ((index & 0x3f) << 24) as i64
        // strength bits 1..0 -> tertiary byte 1 = CE bits 13..8 (byte values 20..23)
        + (strength << 8) as i64
}

fn index_from_temp_ce(mut temp_ce: i64) -> i32 {
    temp_ce = temp_ce.wrapping_sub(0x4040000006002000);
    ((temp_ce >> 43) as i32 & 0xfe000) | ((temp_ce >> 42) as i32 & 0x1fc0) | ((temp_ce >> 24) as i32 & 0x3f)
}

fn strength_from_temp_ce(temp_ce: i64) -> i32 {
    (temp_ce as i32 >> 8) & 3
}

fn is_temp_ce(ce: i64) -> bool {
    let sec = ((ce as i32 as u32) >> 24) as i32;
    (6..=0x45).contains(&sec)
}

fn index_from_temp_ce32(mut temp_ce32: i32) -> i32 {
    temp_ce32 = temp_ce32.wrapping_sub(0x40400620);
    ((temp_ce32 >> 11) & 0xfe000) | ((temp_ce32 >> 10) & 0x1fc0) | ((temp_ce32 >> 8) & 0x3f)
}

fn is_temp_ce32(ce32: i32) -> bool {
    (ce32 & 0xff) >= 2 // not a long-primary/long-secondary CE32
        && 6 <= ((ce32 >> 8) & 0xff)
        && ((ce32 >> 8) & 0xff) <= 0x45
}

fn ce_strength(ce: i64) -> i32 {
    if is_temp_ce(ce) {
        strength_from_temp_ce(ce)
    } else if (ce as u64 & 0xff00000000000000) != 0 {
        collator::PRIMARY
    } else if (ce as i32 as u32 & 0xff000000) != 0 {
        collator::SECONDARY
    } else if ce != 0 {
        collator::TERTIARY
    } else {
        collator::IDENTICAL
    }
}

fn align_weight_right(mut w: i32) -> i32 {
    if w != 0 {
        while (w & 0xff) == 0 {
            w = ((w as u32) >> 8) as i32;
        }
    }
    w
}

fn same_ces(ces1: &[i64], ces1_length: i32, ces2: &[i64], ces2_length: i32) -> bool {
    if ces1_length != ces2_length {
        return false;
    }
    debug_assert!(ces1_length <= Collation::MAX_EXPANSION_LENGTH);
    for i in 0..ces1_length as usize {
        if ces1[i] != ces2[i] {
            return false;
        }
    }
    true
}

fn equal_sub_sequences(left: &[u16], mut left_start: i32, right: &[u16], mut right_start: i32) -> bool {
    // C++ UnicodeString::compare(leftStart, 0x7fffffff, right, rightStart, 0x7fffffff) == 0
    let left_length = left.len() as i32;
    if (left_length - left_start) != (right.len() as i32 - right_start) {
        return false;
    }
    while left_start < left_length {
        if left[left_start as usize] != right[right_start as usize] {
            return false;
        }
        left_start += 1;
        right_start += 1;
    }
    true
}

fn count_tailored_nodes(nodes_array: &[i64], mut i: i32, strength: i32) -> i32 {
    let mut count = 0;
    loop {
        if i == 0 {
            break;
        }
        let node = nodes_array[i as usize];
        if strength_from_node(node) < strength {
            break;
        }
        if strength_from_node(node) == strength {
            if is_tailored_node(node) {
                count += 1;
            } else {
                break;
            }
        }
        i = next_index_from_node(node);
    }
    count
}

/// `CollationBuilder.CEFinalizer` (private).
struct CEFinalizer {
    final_ces: Vec<i64>,
}

impl CEModifier for CEFinalizer {
    fn modify_ce32(&mut self, ce32: i32) -> i64 {
        debug_assert!(!Collation::is_special_ce32(ce32));
        if is_temp_ce32(ce32) {
            // retain case bits
            self.final_ces[index_from_temp_ce32(ce32) as usize] | ((ce32 & 0xc0) << 8) as i64
        } else {
            Collation::NO_CE
        }
    }

    fn modify_ce(&mut self, ce: i64) -> i64 {
        if is_temp_ce(ce) {
            // retain case bits
            self.final_ces[index_from_temp_ce(ce) as usize] | (ce & 0xc000)
        } else {
            Collation::NO_CE
        }
    }
}

/// Collation tailoring builder: builds a tailoring from rules.
///
/// Equivalent to `com.ibm.icu.impl.coll.CollationBuilder` (a
/// `CollationRuleParser.Sink`).
#[derive(Debug)]
pub struct CollationBuilder {
    nfd: Arc<Normalizer2WithImpl>,
    fcd: Arc<Normalizer2WithImpl>,
    nfc_impl: Arc<Normalizer2Impl>,
    base: Arc<CollationTailoring>,
    base_data: Arc<CollationData>,
    root_elements: CollationRootElements,
    variable_top: i64,
    data_builder: Option<CollationDataBuilder>,
    fast_latin_enabled: bool,
    optimize_set: UnicodeSet,
    ces: [i64; Collation::MAX_EXPANSION_LENGTH as usize],
    ces_length: i32,
    /// Indexes of nodes with root primary weights, sorted by primary.
    root_primary_indexes: UVector32,
    /// Data structure for assigning tailored weights and CEs.
    nodes: UVector64,
}

impl CollationBuilder {
    /// `CollationBuilder(CollationTailoring b)`.
    ///
    /// # Errors
    ///
    /// The exceptions of loading the normalization data and of
    /// `ensureCanonIterData()`.
    pub fn new(b: Arc<CollationTailoring>) -> Result<Self> {
        let nfd = normalizer2::get_nfd_instance()?;
        let fcd = Norm2AllModes::get_fcd_normalizer2()?;
        let nfc_impl = Norm2AllModes::get_nfc_instance()?.impl_.clone();
        let base_data = b.data().clone();
        let root_elements = CollationRootElements::new(
            base_data.root_elements.clone().unwrap_or_else(|| panic!("java.lang.NullPointerException")),
        );
        let mut data_builder = CollationDataBuilder::new()?;
        nfc_impl.ensure_canon_iter_data()?;
        data_builder.init_for_tailoring(Some(base_data.clone()))?;
        Ok(Self {
            nfd,
            fcd,
            nfc_impl,
            base: b,
            base_data,
            root_elements,
            variable_top: 0,
            data_builder: Some(data_builder),
            fast_latin_enabled: true,
            optimize_set: UnicodeSet::new(),
            ces: [0; Collation::MAX_EXPANSION_LENGTH as usize],
            ces_length: 0,
            root_primary_indexes: UVector32::new(),
            nodes: UVector64::new(),
        })
    }

    fn db(&mut self) -> &mut CollationDataBuilder {
        self.data_builder.as_mut().unwrap_or_else(|| panic!("java.lang.NullPointerException"))
    }

    /// `parseAndBuild(String)`.
    ///
    /// # Errors
    ///
    /// `ParseException` for invalid rules; `UnsupportedOperationException`
    /// when the base has no root elements or a tailoring gap is too small.
    pub fn parse_and_build(&mut self, rule_string: &[u16]) -> Result<CollationTailoring> {
        if self.base_data.root_elements.is_none() {
            // C++ U_MISSING_RESOURCE_ERROR
            return Err(LuceneError::unsupported_operation("missing root elements data, tailoring not supported"));
        }
        let mut tailoring = CollationTailoring::new(Some(&self.base.settings));
        let mut parser = CollationRuleParser::new(self.base_data.clone())?;
        // Note: This always bases &[last variable] and &[first regular]
        // on the root collator's maxVariable/variableTop.
        // If we wanted this to change after [maxVariable x], then we would keep
        // the tailoring.settings pointer here and read its variableTop when we need it.
        // See http://unicode.org/cldr/trac/ticket/6070
        self.variable_top = self.base.settings().variable_top;
        // In Java, there is only one Importer implementation.
        // In C++, the importer is a parameter for this method.
        parser.set_importer(Some(Arc::new(BundleImporter)));
        {
            let owned_settings = tailoring.settings.copy_on_write();
            parser.parse(rule_string, owned_settings, self)?;
        }
        if self.db().has_mappings() {
            self.make_tailored_ces()?;
            self.close_over_composites()?;
            self.finalize_ces()?;
            // Copy all of ASCII, and Latin-1 letters, into each tailoring.
            self.optimize_set.add_range(0, 0x7f)?;
            self.optimize_set.add_range(0xc0, 0xff)?;
            // Hangul is decomposed on the fly during collation,
            // and the tailoring data is always built with HANGUL_TAG specials.
            self.optimize_set.remove_range(Hangul::HANGUL_BASE, Hangul::HANGUL_END)?;
            let optimize_set = self.optimize_set.clone();
            self.db().optimize(&optimize_set)?;
            let mut owned_data = tailoring.new_owned_data()?;
            if self.fast_latin_enabled {
                self.db().enable_fast_latin();
            }
            self.db().build(&mut owned_data)?;
            tailoring.set_owned_data(owned_data);
            // C++ tailoring.builder = dataBuilder;
            self.data_builder = None;
        } else {
            tailoring.data = Some(self.base_data.clone());
        }
        {
            let data = tailoring.data().clone();
            let owned_settings = tailoring.settings.copy_on_write();
            let mut primaries = std::mem::take(&mut owned_settings.fast_latin_primaries);
            owned_settings.fast_latin_options = CollationFastLatin::get_options(&data, owned_settings, &mut primaries);
            owned_settings.fast_latin_primaries = primaries;
        }
        tailoring.set_rules(rule_string.to_vec());
        // In Java, we do not have a rules version.
        // In C++, the genrb build tool reads and supplies one,
        // and the rulesVersion is a parameter for this method.
        tailoring.set_version(self.base.version, 0 /* rulesVersion */);
        Ok(tailoring)
    }

    #[allow(unused_assignments)] // Java's dead stores to `index`, kept as written
    fn get_weight16_before(&self, mut index: i32, mut node: i64, level: i32) -> i32 {
        debug_assert!(strength_from_node(node) < level || !is_tailored_node(node));
        // Collect the root CE weights if this node is for a root CE.
        // If it is not, then return the low non-primary boundary for a tailored CE.
        let t = if strength_from_node(node) == collator::TERTIARY {
            weight16_from_node(node)
        } else {
            Collation::COMMON_WEIGHT16 // Stronger node with implied common weight.
        };
        while strength_from_node(node) > collator::SECONDARY {
            index = previous_index_from_node(node);
            node = self.nodes.element_ati(index);
        }
        if is_tailored_node(node) {
            return Collation::BEFORE_WEIGHT16;
        }
        let s = if strength_from_node(node) == collator::SECONDARY {
            weight16_from_node(node)
        } else {
            Collation::COMMON_WEIGHT16 // Stronger node with implied common weight.
        };
        while strength_from_node(node) > collator::PRIMARY {
            index = previous_index_from_node(node);
            node = self.nodes.element_ati(index);
        }
        if is_tailored_node(node) {
            return Collation::BEFORE_WEIGHT16;
        }
        // [p, s, t] is a root CE. Return the preceding weight for the requested level.
        let p = weight32_from_node(node);
        let weight16;
        if level == collator::SECONDARY {
            weight16 = self.root_elements.get_secondary_before(p, s);
        } else {
            weight16 = self.root_elements.get_tertiary_before(p, s, t);
            debug_assert!((weight16 & !Collation::ONLY_TERTIARY_MASK) == 0);
        }
        weight16
    }

    fn get_special_reset_position(&mut self, str: &[u16]) -> Result<i64> {
        debug_assert!(str.len() == 2);
        let mut ce: i64;
        let mut strength = collator::PRIMARY;
        let mut is_boundary = false;
        let pos = POSITION_VALUES[(str[1] - POS_BASE) as usize];
        match pos {
            Position::FirstTertiaryIgnorable => {
                // Quaternary CEs are not supported.
                // Non-zero quaternary weights are possible only on tertiary or stronger CEs.
                return Ok(0);
            }
            Position::LastTertiaryIgnorable => return Ok(0),
            Position::FirstSecondaryIgnorable => {
                // Look for a tailored tertiary node after [0, 0, 0].
                let mut index = self.find_or_insert_node_for_root_ce(0, collator::TERTIARY);
                let mut node = self.nodes.element_ati(index);
                index = next_index_from_node(node);
                if index != 0 {
                    node = self.nodes.element_ati(index);
                    debug_assert!(strength_from_node(node) <= collator::TERTIARY);
                    if is_tailored_node(node) && strength_from_node(node) == collator::TERTIARY {
                        return Ok(temp_ce_from_index_and_strength(index, collator::TERTIARY));
                    }
                }
                return Ok(self.root_elements.get_first_tertiary_ce());
                // No need to look for nodeHasAnyBefore() on a tertiary node.
            }
            Position::LastSecondaryIgnorable => {
                ce = self.root_elements.get_last_tertiary_ce();
                strength = collator::TERTIARY;
            }
            Position::FirstPrimaryIgnorable => {
                // Look for a tailored secondary node after [0, 0, *].
                let mut index = self.find_or_insert_node_for_root_ce(0, collator::SECONDARY);
                let mut node = self.nodes.element_ati(index);
                loop {
                    index = next_index_from_node(node);
                    if index == 0 {
                        break;
                    }
                    node = self.nodes.element_ati(index);
                    strength = strength_from_node(node);
                    if strength < collator::SECONDARY {
                        break;
                    }
                    if strength == collator::SECONDARY {
                        if is_tailored_node(node) {
                            if node_has_before3(node) {
                                index = next_index_from_node(self.nodes.element_ati(next_index_from_node(node)));
                                debug_assert!(is_tailored_node(self.nodes.element_ati(index)));
                            }
                            return Ok(temp_ce_from_index_and_strength(index, collator::SECONDARY));
                        } else {
                            break;
                        }
                    }
                }
                ce = self.root_elements.get_first_secondary_ce();
                strength = collator::SECONDARY;
            }
            Position::LastPrimaryIgnorable => {
                ce = self.root_elements.get_last_secondary_ce();
                strength = collator::SECONDARY;
            }
            Position::FirstVariable => {
                ce = self.root_elements.get_first_primary_ce();
                is_boundary = true; // FractionalUCA.txt: FDD1 00A0, SPACE first primary
            }
            Position::LastVariable => {
                ce = self.root_elements.last_ce_with_primary_before(self.variable_top + 1);
            }
            Position::FirstRegular => {
                ce = self.root_elements.first_ce_with_primary_at_least(self.variable_top + 1);
                is_boundary = true; // FractionalUCA.txt: FDD1 263A, SYMBOL first primary
            }
            Position::LastRegular => {
                // Use the Hani-first-primary rather than the actual last "regular" CE before it,
                // for backward compatibility with behavior before the introduction of
                // script-first-primary CEs in the root collator.
                ce = self
                    .root_elements
                    .first_ce_with_primary_at_least(self.base_data.get_first_primary_for_group(UScript::HAN));
            }
            Position::FirstImplicit => {
                ce = self.base_data.get_single_ce(0x4e00)?;
            }
            Position::LastImplicit => {
                // We do not support tailoring to an unassigned-implicit CE.
                return Err(LuceneError::unsupported_operation("reset to [last implicit] not supported"));
            }
            Position::FirstTrailing => {
                ce = Collation::make_ce(Collation::FIRST_TRAILING_PRIMARY);
                is_boundary = true; // trailing first primary (there is no mapping for it)
            }
            Position::LastTrailing => {
                return Err(LuceneError::illegal_argument("LDML forbids tailoring to U+FFFF"));
            }
        }

        let mut index = self.find_or_insert_node_for_root_ce(ce, strength);
        let mut node = self.nodes.element_ati(index);
        if (pos.ordinal() & 1) == 0 {
            // even pos = [first xyz]
            if !node_has_any_before(node) && is_boundary {
                // A <group> first primary boundary is artificially added to FractionalUCA.txt.
                // It is reachable via its special contraction, but is not normally used.
                // Find the first character tailored after the boundary CE,
                // or the first real root CE after it.
                index = next_index_from_node(node);
                if index != 0 {
                    // If there is a following node, then it must be tailored
                    // because there are no root CEs with a boundary primary
                    // and non-common secondary/tertiary weights.
                    node = self.nodes.element_ati(index);
                    debug_assert!(is_tailored_node(node));
                    ce = temp_ce_from_index_and_strength(index, strength);
                } else {
                    debug_assert!(strength == collator::PRIMARY);
                    let mut p = ushr32(ce);
                    let p_index = self.root_elements.find_primary(p);
                    let is_compressible = self.base_data.is_compressible_primary(p);
                    p = self.root_elements.get_primary_after(p, p_index, is_compressible);
                    ce = Collation::make_ce(p);
                    index = self.find_or_insert_node_for_root_ce(ce, collator::PRIMARY);
                    node = self.nodes.element_ati(index);
                }
            }
            if node_has_any_before(node) {
                // Get the first node that was tailored before this one at a weaker strength.
                if node_has_before2(node) {
                    index = next_index_from_node(self.nodes.element_ati(next_index_from_node(node)));
                    node = self.nodes.element_ati(index);
                }
                if node_has_before3(node) {
                    index = next_index_from_node(self.nodes.element_ati(next_index_from_node(node)));
                }
                debug_assert!(is_tailored_node(self.nodes.element_ati(index)));
                ce = temp_ce_from_index_and_strength(index, strength);
            }
        } else {
            // odd pos = [last xyz]
            // Find the last node that was tailored after the [last xyz]
            // at a strength no greater than the position's strength.
            loop {
                let next_index = next_index_from_node(node);
                if next_index == 0 {
                    break;
                }
                let next_node = self.nodes.element_ati(next_index);
                if strength_from_node(next_node) < strength {
                    break;
                }
                index = next_index;
                node = next_node;
            }
            // Do not make a temporary CE for a root node.
            // This last node might be the node for the root CE itself,
            // or a node with a common secondary or tertiary weight.
            if is_tailored_node(node) {
                ce = temp_ce_from_index_and_strength(index, strength);
            }
        }
        Ok(ce)
    }

    fn find_or_insert_node_for_ces(&mut self, strength: i32) -> Result<i32> {
        debug_assert!((collator::PRIMARY..=collator::QUATERNARY).contains(&strength));

        // Find the last CE that is at least as "strong" as the requested difference.
        // Note: Stronger is smaller (Collator.PRIMARY=0).
        let ce;
        loop {
            let c;
            if self.ces_length == 0 {
                self.ces[0] = 0;
                c = 0;
                self.ces_length = 1;
                ce = c;
                break;
            } else {
                c = self.ces[(self.ces_length - 1) as usize];
            }
            if ce_strength(c) <= strength {
                ce = c;
                break;
            }
            self.ces_length -= 1;
        }

        if is_temp_ce(ce) {
            // No need to findCommonNode() here for lower levels
            // because insertTailoredNodeAfter() will do that anyway.
            return Ok(index_from_temp_ce(ce));
        }

        // root CE
        if ((ce as u64) >> 56) as i32 == Collation::UNASSIGNED_IMPLICIT_BYTE {
            return Err(LuceneError::unsupported_operation(
                "tailoring relative to an unassigned code point not supported",
            ));
        }
        Ok(self.find_or_insert_node_for_root_ce(ce, strength))
    }

    fn find_or_insert_node_for_root_ce(&mut self, ce: i64, strength: i32) -> i32 {
        debug_assert!(((ce as u64) >> 56) as i32 != Collation::UNASSIGNED_IMPLICIT_BYTE);

        // Find or insert the node for each of the root CE's weights,
        // down to the requested level/strength.
        // Root CEs must have common=zero quaternary weights (for which we never insert any nodes).
        debug_assert!((ce & 0xc0) == 0);
        let mut index = self.find_or_insert_node_for_primary(ushr32(ce));
        if strength >= collator::SECONDARY {
            let lower32 = ce as i32;
            index = self.find_or_insert_weak_node(index, ((lower32 as u32) >> 16) as i32, collator::SECONDARY);
            if strength >= collator::TERTIARY {
                index = self.find_or_insert_weak_node(index, lower32 & Collation::ONLY_TERTIARY_MASK, collator::TERTIARY);
            }
        }
        index
    }

    /// Like Java Collections.binarySearch(List, String, Comparator).
    fn binary_search_for_root_primary_node(root_primary_indexes: &[i32], length: i32, nodes: &[i64], p: i64) -> i32 {
        if length == 0 {
            return !0;
        }
        let mut start = 0;
        let mut limit = length;
        loop {
            let i = ((start as i64 + limit as i64) / 2) as i32;
            let node = nodes[root_primary_indexes[i as usize] as usize];
            let node_primary = ushr32(node); // weight32FromNode(node)
            if p == node_primary {
                return i;
            } else if p < node_primary {
                if i == start {
                    return !start; // insert s before i
                }
                limit = i;
            } else {
                if i == start {
                    return !(start + 1); // insert s after i
                }
                start = i;
            }
        }
    }

    /// Finds or inserts the node for a root CE's primary weight.
    fn find_or_insert_node_for_primary(&mut self, p: i64) -> i32 {
        let root_index = Self::binary_search_for_root_primary_node(
            self.root_primary_indexes.get_buffer(),
            self.root_primary_indexes.size(),
            self.nodes.get_buffer(),
            p,
        );
        if root_index >= 0 {
            self.root_primary_indexes.element_ati(root_index)
        } else {
            // Start a new list of nodes with this primary.
            let index = self.nodes.size();
            self.nodes.add_element(node_from_weight32(p));
            self.root_primary_indexes.insert_element_at(index, !root_index);
            index
        }
    }

    /// Finds or inserts the node for a secondary or tertiary weight.
    fn find_or_insert_weak_node(&mut self, mut index: i32, weight16: i32, level: i32) -> i32 {
        debug_assert!(0 <= index && index < self.nodes.size());
        debug_assert!((collator::SECONDARY..=collator::TERTIARY).contains(&level));

        if weight16 == Collation::COMMON_WEIGHT16 {
            return self.find_common_node(index, level);
        }

        // If this will be the first below-common weight for the parent node,
        // then we will also need to insert a common weight after it.
        let mut node = self.nodes.element_ati(index);
        debug_assert!(strength_from_node(node) < level); // parent node is stronger
        if weight16 != 0 && weight16 < Collation::COMMON_WEIGHT16 {
            let has_this_level_before = if level == collator::SECONDARY { HAS_BEFORE2 } else { HAS_BEFORE3 };
            if (node & has_this_level_before as i64) == 0 {
                // The parent node has an implied level-common weight.
                let mut common_node = node_from_weight16(Collation::COMMON_WEIGHT16) | node_from_strength(level);
                if level == collator::SECONDARY {
                    // Move the HAS_BEFORE3 flag from the parent node
                    // to the new secondary common node.
                    common_node |= node & HAS_BEFORE3 as i64;
                    node &= !(HAS_BEFORE3 as i64);
                }
                self.nodes.set_element_at(node | has_this_level_before as i64, index);
                // Insert below-common-weight node.
                let next_index = next_index_from_node(node);
                node = node_from_weight16(weight16) | node_from_strength(level);
                index = self.insert_node_between(index, next_index, node);
                // Insert common-weight node.
                self.insert_node_between(index, next_index, common_node);
                // Return index of below-common-weight node.
                return index;
            }
        }

        // Find the root CE's weight for this level.
        // Postpone insertion if not found:
        // Insert the new root node before the next stronger node,
        // or before the next root node with the same strength and a larger weight.
        let mut next_index;
        loop {
            next_index = next_index_from_node(node);
            if next_index == 0 {
                break;
            }
            node = self.nodes.element_ati(next_index);
            let next_strength = strength_from_node(node);
            if next_strength <= level {
                // Insert before a stronger node.
                if next_strength < level {
                    break;
                }
                // nextStrength == level
                if !is_tailored_node(node) {
                    let next_weight16 = weight16_from_node(node);
                    if next_weight16 == weight16 {
                        // Found the node for the root CE up to this level.
                        return next_index;
                    }
                    // Insert before a node with a larger same-strength weight.
                    if next_weight16 > weight16 {
                        break;
                    }
                }
            }
            // Skip the next node.
            index = next_index;
        }
        node = node_from_weight16(weight16) | node_from_strength(level);
        self.insert_node_between(index, next_index, node)
    }

    /// Makes and inserts a new tailored node into the list, after the one at index.
    fn insert_tailored_node_after(&mut self, mut index: i32, strength: i32) -> i32 {
        debug_assert!(0 <= index && index < self.nodes.size());
        if strength >= collator::SECONDARY {
            index = self.find_common_node(index, collator::SECONDARY);
            if strength >= collator::TERTIARY {
                index = self.find_common_node(index, collator::TERTIARY);
            }
        }
        // Postpone insertion:
        // Insert the new node before the next one with a strength at least as strong.
        let mut node = self.nodes.element_ati(index);
        let mut next_index;
        loop {
            next_index = next_index_from_node(node);
            if next_index == 0 {
                break;
            }
            node = self.nodes.element_ati(next_index);
            if strength_from_node(node) <= strength {
                break;
            }
            // Skip the next node which has a weaker (larger) strength than the new one.
            index = next_index;
        }
        node = IS_TAILORED as i64 | node_from_strength(strength);
        self.insert_node_between(index, next_index, node)
    }

    /// Inserts a new node into the list, between list-adjacent items.
    fn insert_node_between(&mut self, index: i32, next_index: i32, mut node: i64) -> i32 {
        debug_assert!(previous_index_from_node(node) == 0);
        debug_assert!(next_index_from_node(node) == 0);
        debug_assert!(next_index_from_node(self.nodes.element_ati(index)) == next_index);
        // Append the new node and link it to the existing nodes.
        let new_index = self.nodes.size();
        node |= node_from_previous_index(index) | node_from_next_index(next_index);
        self.nodes.add_element(node);
        // nodes[index].nextIndex = newIndex
        node = self.nodes.element_ati(index);
        self.nodes.set_element_at(change_node_next_index(node, new_index), index);
        // nodes[nextIndex].previousIndex = newIndex
        if next_index != 0 {
            node = self.nodes.element_ati(next_index);
            self.nodes.set_element_at(change_node_previous_index(node, new_index), next_index);
        }
        new_index
    }

    /// Finds the node which implies or contains a common=05 weight of the
    /// given strength (secondary or tertiary), if the current node is stronger.
    fn find_common_node(&self, mut index: i32, strength: i32) -> i32 {
        debug_assert!((collator::SECONDARY..=collator::TERTIARY).contains(&strength));
        let mut node = self.nodes.element_ati(index);
        if strength_from_node(node) >= strength {
            // The current node is no stronger.
            return index;
        }
        if if strength == collator::SECONDARY { !node_has_before2(node) } else { !node_has_before3(node) } {
            // The current node implies the strength-common weight.
            return index;
        }
        index = next_index_from_node(node);
        node = self.nodes.element_ati(index);
        debug_assert!(
            !is_tailored_node(node)
                && strength_from_node(node) == strength
                && weight16_from_node(node) < Collation::COMMON_WEIGHT16
        );
        // Skip to the explicit common node.
        loop {
            index = next_index_from_node(node);
            node = self.nodes.element_ati(index);
            debug_assert!(strength_from_node(node) >= strength);
            if !(is_tailored_node(node)
                || strength_from_node(node) > strength
                || weight16_from_node(node) < Collation::COMMON_WEIGHT16)
            {
                break;
            }
        }
        debug_assert!(weight16_from_node(node) == Collation::COMMON_WEIGHT16);
        index
    }

    fn set_case_bits(&mut self, nfd_string: &[u16]) -> Result<()> {
        let mut num_tailored_primaries = 0;
        for i in 0..self.ces_length as usize {
            if ce_strength(self.ces[i]) == collator::PRIMARY {
                num_tailored_primaries += 1;
            }
        }
        // We should not be able to get too many case bits because
        // cesLength<=31==MAX_EXPANSION_LENGTH.
        // 31 pairs of case bits fit into an long without setting its sign bit.
        debug_assert!(num_tailored_primaries <= 31);

        let mut cases: i64 = 0;
        if num_tailored_primaries > 0 {
            let s = nfd_string;
            let mut base_ces = UTF16CollationIterator::with_text(self.base_data.clone(), false, s, 0);
            let base_ces_length = base_ces.fetch_ces() - 1;
            debug_assert!(base_ces_length >= 0 && base_ces.base().get_ce(base_ces_length) == Collation::NO_CE);

            let mut last_case = 0;
            let mut num_base_primaries = 0;
            for i in 0..base_ces_length {
                let ce = base_ces.base().get_ce(i);
                if ushr32(ce) != 0 {
                    num_base_primaries += 1;
                    let c = (ce as i32 >> 14) & 3;
                    debug_assert!(c == 0 || c == 2); // lowercase or uppercase, no mixed case in any base CE
                    if num_base_primaries < num_tailored_primaries {
                        cases |= (c as i64) << ((num_base_primaries - 1) * 2);
                    } else if num_base_primaries == num_tailored_primaries {
                        last_case = c;
                    } else if c != last_case {
                        // There are more base primary CEs than tailored primaries.
                        // Set mixed case if the case bits of the remainder differ.
                        last_case = 1;
                        // Nothing more can change.
                        break;
                    }
                }
            }
            if num_base_primaries >= num_tailored_primaries {
                cases |= (last_case as i64) << ((num_tailored_primaries - 1) * 2);
            }
        }

        for i in 0..self.ces_length as usize {
            let mut ce = self.ces[i] & (0xffffffffffff3fffu64 as i64); // clear old case bits
            let strength = ce_strength(ce);
            if strength == collator::PRIMARY {
                ce |= (cases & 3) << 14;
                cases = ((cases as u64) >> 2) as i64;
            } else if strength == collator::TERTIARY {
                // Tertiary CEs must have uppercase bits.
                // See the LDML spec, and comments in class CollationCompare.
                ce |= 0x8000;
            }
            // Tertiary ignorable CEs must have 0 case bits.
            // We set 0 case bits for secondary CEs too
            // since currently only U+0345 is cased and maps to a secondary CE,
            // and it is lowercase. Other secondaries are uncased.
            // See [[:Cased:]&[:uca1=:]] where uca1 queries the root primary weight.
            self.ces[i] = ce;
        }
        Ok(())
    }

    fn add_with_closure(
        &mut self,
        nfd_prefix: &[u16],
        nfd_string: &[u16],
        new_ces: &[i64],
        new_ces_length: i32,
        mut ce32: i32,
    ) -> Result<i32> {
        // Map from the NFD input to the CEs.
        ce32 = self.add_if_different(nfd_prefix, nfd_string, new_ces, new_ces_length, ce32)?;
        ce32 = self.add_only_closure(nfd_prefix, nfd_string, new_ces, new_ces_length, ce32)?;
        self.add_tail_composites(nfd_prefix, nfd_string)?;
        Ok(ce32)
    }

    fn add_only_closure(
        &mut self,
        nfd_prefix: &[u16],
        nfd_string: &[u16],
        new_ces: &[i64],
        new_ces_length: i32,
        mut ce32: i32,
    ) -> Result<i32> {
        // Map from canonically equivalent input to the CEs. (But not from the all-NFD input.)
        // TODO: make CanonicalIterator work with CharSequence, or maybe change arguments here to String
        let mut loop_count = 0;
        if nfd_prefix.is_empty() {
            let mut string_iter = CanonicalIterator::new(nfd_string)?;
            let prefix: [u16; 0] = [];
            loop {
                let Some(str) = string_iter.next()? else {
                    break;
                };
                if self.ignore_string(&str)? || str[..] == nfd_string[..] {
                    continue;
                }
                let l = loop_count;
                loop_count += 1;
                if l > K_CLOSURE_LOOP_LIMIT {
                    return Err(ICUInputTooLongException::with_message("Too many closure").error());
                }
                ce32 = self.add_if_different(&prefix, &str, new_ces, new_ces_length, ce32)?;
            }
        } else {
            let mut prefix_iter = CanonicalIterator::new(nfd_prefix)?;
            let mut string_iter = CanonicalIterator::new(nfd_string)?;
            loop {
                let Some(prefix) = prefix_iter.next()? else {
                    break;
                };
                if self.ignore_prefix(&prefix)? {
                    continue;
                }
                let same_prefix = prefix[..] == nfd_prefix[..];
                loop {
                    let Some(str) = string_iter.next()? else {
                        break;
                    };
                    if self.ignore_string(&str)? || (same_prefix && str[..] == nfd_string[..]) {
                        continue;
                    }
                    let l = loop_count;
                    loop_count += 1;
                    if l > K_CLOSURE_LOOP_LIMIT {
                        return Err(ICUInputTooLongException::with_message("Too many closure").error());
                    }
                    ce32 = self.add_if_different(&prefix, &str, new_ces, new_ces_length, ce32)?;
                }
                string_iter.reset();
            }
        }
        Ok(ce32)
    }

    fn add_tail_composites(&mut self, nfd_prefix: &[u16], nfd_string: &[u16]) -> Result<()> {
        // Look for the last starter in the NFD string.
        let mut last_starter;
        let mut index_after_last_starter = nfd_string.len() as i32;
        loop {
            if index_after_last_starter == 0 {
                return Ok(());
            } // no starter at all
            last_starter = code_point_before(nfd_string, index_after_last_starter);
            if self.nfd.get_combining_class(last_starter) == 0 {
                break;
            }
            index_after_last_starter -= char_count(last_starter);
        }
        // No closure to Hangul syllables since we decompose them on the fly.
        if Hangul::is_jamo_l(last_starter) {
            return Ok(());
        }

        // Are there any composites whose decomposition starts with the lastStarter?
        // Note: Normalizer2Impl does not currently return start sets for NFC_QC=Maybe characters.
        // We might find some more equivalent mappings here if it did.
        let mut composites = UnicodeSet::new();
        if !self.nfc_impl.get_canon_start_set(last_starter, &mut composites)? {
            return Ok(());
        }

        let mut new_nfd_string: Vec<u16> = Vec::new();
        let mut new_string: Vec<u16> = Vec::new();
        let mut new_ces = [0i64; Collation::MAX_EXPANSION_LENGTH as usize];
        let mut iter = UnicodeSetIterator::new(&composites);
        while iter.next() {
            debug_assert!(iter.codepoint != IS_STRING);
            let composite = iter.codepoint;
            let decomp = self.nfd.get_decomposition(composite)?.unwrap_or_else(|| panic!("java.lang.NullPointerException"));
            if !self.merge_composite_into_string(
                nfd_string,
                index_after_last_starter,
                composite,
                &decomp,
                &mut new_nfd_string,
                &mut new_string,
            )? {
                continue;
            }
            let new_ces_length = self.db().get_ces_prefix(nfd_prefix, &new_nfd_string, &mut new_ces, 0)?;
            if new_ces_length > Collation::MAX_EXPANSION_LENGTH {
                // Ignore mappings that we cannot store.
                continue;
            }
            // Note: It is possible that the newCEs do not make use of the mapping
            // for which we are adding the tail composites, in which case we might be adding
            // unnecessary mappings.
            // For example, when we add tail composites for ae^ (^=combining circumflex),
            // UCA discontiguous-contraction matching does not find any matches
            // for ae_^ (_=any combining diacritic below) *unless* there is also
            // a contraction mapping for ae.
            // Thus, if there is no ae contraction, then the ae^ mapping is ignored
            // while fetching the newCEs for ae_^.
            // TODO: Try to detect this effectively.
            // (Alternatively, print a warning when prefix contractions are missing.)

            // We do not need an explicit mapping for the NFD strings.
            // It is fine if the NFD input collates like this via a sequence of mappings.
            // It also saves a little bit of space, and may reduce the set of characters with contractions.
            let ce32 = self.add_if_different(nfd_prefix, &new_string, &new_ces, new_ces_length, Collation::UNASSIGNED_CE32)?;
            if ce32 != Collation::UNASSIGNED_CE32 {
                // was different, was added
                self.add_only_closure(nfd_prefix, &new_nfd_string, &new_ces, new_ces_length, ce32)?;
            }
        }
        Ok(())
    }

    fn merge_composite_into_string(
        &self,
        nfd_string: &[u16],
        index_after_last_starter: i32,
        composite: i32,
        decomp: &[u16],
        new_nfd_string: &mut Vec<u16>,
        new_string: &mut Vec<u16>,
    ) -> Result<bool> {
        debug_assert!(code_point_before(nfd_string, index_after_last_starter) == code_point_at(decomp, 0));
        let last_starter_length = offset_by_code_points(decomp, 0, 1);
        if last_starter_length == decomp.len() as i32 {
            // Singleton decompositions should be found by addWithClosure()
            // and the CanonicalIterator, so we can ignore them here.
            return Ok(false);
        }
        if equal_sub_sequences(nfd_string, index_after_last_starter, decomp, last_starter_length) {
            // same strings, nothing new to be found here
            return Ok(false);
        }

        // Make new FCD strings that combine a composite, or its decomposition,
        // into the nfdString's last starter and the combining marks following it.
        // Make an NFD version, and a version with the composite.
        new_nfd_string.clear();
        new_nfd_string.extend_from_slice(&nfd_string[..index_after_last_starter as usize]);
        new_string.clear();
        new_string.extend_from_slice(&nfd_string[..(index_after_last_starter - last_starter_length) as usize]);
        crate::jdk::append_code_point(new_string, composite)?;

        // The following is related to discontiguous contraction matching,
        // but builds only FCD strings (or else returns false).
        let mut source_index = index_after_last_starter;
        let mut decomp_index = last_starter_length;
        // Small optimization: We keep the source character across loop iterations
        // because we do not always consume it,
        // and then need not fetch it again nor look up its combining class again.
        let mut source_char = Collation::SENTINEL_CP;
        // The cc variables need to be declared before the loop so that at the end
        // they are set to the last combining classes seen.
        let mut source_cc = 0;
        let mut decomp_cc = 0;
        loop {
            if source_char < 0 {
                if source_index >= nfd_string.len() as i32 {
                    break;
                }
                source_char = code_point_at(nfd_string, source_index);
                source_cc = self.nfd.get_combining_class(source_char);
                debug_assert!(source_cc != 0);
            }
            // We consume a decomposition character in each iteration.
            if decomp_index >= decomp.len() as i32 {
                break;
            }
            let decomp_char = code_point_at(decomp, decomp_index);
            decomp_cc = self.nfd.get_combining_class(decomp_char);
            // Compare the two characters and their combining classes.
            if decomp_cc == 0 {
                // Unable to merge because the source contains a non-zero combining mark
                // but the composite's decomposition contains another starter.
                // The strings would not be equivalent.
                return Ok(false);
            } else if source_cc < decomp_cc {
                // Composite + sourceChar would not be FCD.
                return Ok(false);
            } else if decomp_cc < source_cc {
                crate::jdk::append_code_point(new_nfd_string, decomp_char)?;
                decomp_index += char_count(decomp_char);
            } else if decomp_char != source_char {
                // Blocked because same combining class.
                return Ok(false);
            } else {
                // match: decompChar == sourceChar
                crate::jdk::append_code_point(new_nfd_string, decomp_char)?;
                decomp_index += char_count(decomp_char);
                source_index += char_count(decomp_char);
                source_char = Collation::SENTINEL_CP;
            }
        }
        // We are at the end of at least one of the two inputs.
        if source_char >= 0 {
            // more characters from nfdString but not from decomp
            if source_cc < decomp_cc {
                // Appending the next source character to the composite would not be FCD.
                return Ok(false);
            }
            new_nfd_string.extend_from_slice(&nfd_string[source_index as usize..]);
            new_string.extend_from_slice(&nfd_string[source_index as usize..]);
        } else if decomp_index < decomp.len() as i32 {
            // more characters from decomp, not from nfdString
            new_nfd_string.extend_from_slice(&decomp[decomp_index as usize..]);
        }
        debug_assert!(self.nfd.is_normalized(new_nfd_string)?);
        debug_assert!(self.fcd.is_normalized(new_string)?);
        debug_assert!(self.nfd.normalize_str(new_string)? == *new_nfd_string); // canonically equivalent
        Ok(true)
    }

    fn ignore_prefix(&self, s: &[u16]) -> Result<bool> {
        // Do not map non-FCD prefixes.
        Ok(!self.is_fcd(s)?)
    }

    fn ignore_string(&self, s: &[u16]) -> Result<bool> {
        // Do not map non-FCD strings.
        // Do not map strings that start with Hangul syllables: We decompose those on the fly.
        Ok(!self.is_fcd(s)? || Hangul::is_hangul(s[0] as i32))
    }

    fn is_fcd(&self, s: &[u16]) -> Result<bool> {
        self.fcd.is_normalized(s)
    }

    fn close_over_composites(&mut self) -> Result<()> {
        let prefix: [u16; 0] = []; // empty
        let mut iter = UnicodeSetIterator::new(&COMPOSITES);
        while iter.next() {
            debug_assert!(iter.codepoint != IS_STRING);
            let nfd_string =
                self.nfd.get_decomposition(iter.codepoint)?.unwrap_or_else(|| panic!("java.lang.NullPointerException"));
            let mut ces = self.ces;
            self.ces_length = self.db().get_ces(&nfd_string, &mut ces, 0)?;
            self.ces = ces;
            if self.ces_length > Collation::MAX_EXPANSION_LENGTH {
                // Too many CEs from the decomposition (unusual), ignore this composite.
                // We could add a capacity parameter to getCEs() and reallocate if necessary.
                // However, this can only really happen in contrived cases.
                continue;
            }
            let composite = iter.get_string()?;
            let ces = self.ces;
            let ces_length = self.ces_length;
            self.add_if_different(&prefix, &composite, &ces, ces_length, Collation::UNASSIGNED_CE32)?;
        }
        Ok(())
    }

    fn add_if_different(&mut self, prefix: &[u16], str: &[u16], new_ces: &[i64], new_ces_length: i32, mut ce32: i32) -> Result<i32> {
        let mut old_ces = [0i64; Collation::MAX_EXPANSION_LENGTH as usize];
        let old_ces_length = self.db().get_ces_prefix(prefix, str, &mut old_ces, 0)?;
        if !same_ces(new_ces, new_ces_length, &old_ces, old_ces_length) {
            if ce32 == Collation::UNASSIGNED_CE32 {
                ce32 = self.db().encode_ces(new_ces, new_ces_length)?;
            }
            self.db().add_ce32(prefix, str, ce32)?;
        }
        Ok(ce32)
    }

    fn make_tailored_ces(&mut self) -> Result<()> {
        let mut primaries = CollationWeights::new();
        let mut secondaries = CollationWeights::new();
        let mut tertiaries = CollationWeights::new();

        if DEBUG {
            println!("\nCollationBuilder.makeTailoredCEs()");
        }

        for rpi in 0..self.root_primary_indexes.size() {
            let mut i = self.root_primary_indexes.element_ati(rpi);
            let mut node = self.nodes.element_ati(i);
            let mut p = weight32_from_node(node);
            let mut s = if p == 0 { 0 } else { Collation::COMMON_WEIGHT16 };
            let mut t = s;
            let mut q = 0;
            let mut p_is_tailored = false;
            let mut s_is_tailored = false;
            let mut t_is_tailored = false;
            if DEBUG {
                println!("\nprimary     {:x}", align_weight_right(p as i32));
            }
            let p_index = if p == 0 { 0 } else { self.root_elements.find_primary(p) };
            let mut next_index = next_index_from_node(node);
            while next_index != 0 {
                i = next_index;
                node = self.nodes.element_ati(i);
                next_index = next_index_from_node(node);
                let strength = strength_from_node(node);
                if strength == collator::QUATERNARY {
                    debug_assert!(is_tailored_node(node));
                    if q == 3 {
                        // C++ U_BUFFER_OVERFLOW_ERROR
                        return Err(LuceneError::unsupported_operation("quaternary tailoring gap too small"));
                    }
                    q += 1;
                } else {
                    if strength == collator::TERTIARY {
                        if is_tailored_node(node) {
                            if !t_is_tailored {
                                // First tailored tertiary node for [p, s].
                                let t_count = count_tailored_nodes(self.nodes.get_buffer(), next_index, collator::TERTIARY) + 1;
                                let t_limit;
                                if t == 0 {
                                    // Gap at the beginning of the tertiary CE range.
                                    t = self.root_elements.get_tertiary_boundary() - 0x100;
                                    t_limit = self.root_elements.get_first_tertiary_ce() as i32 & Collation::ONLY_TERTIARY_MASK;
                                } else if !p_is_tailored && !s_is_tailored {
                                    // p and s are root weights.
                                    t_limit = self.root_elements.get_tertiary_after(p_index, s, t);
                                } else if t == Collation::BEFORE_WEIGHT16 {
                                    t_limit = Collation::COMMON_WEIGHT16;
                                } else {
                                    // [p, s] is tailored.
                                    debug_assert!(t == Collation::COMMON_WEIGHT16);
                                    t_limit = self.root_elements.get_tertiary_boundary();
                                }
                                debug_assert!(t_limit == 0x4000 || (t_limit & !Collation::ONLY_TERTIARY_MASK) == 0);
                                tertiaries.init_for_tertiary();
                                if !tertiaries.alloc_weights(t as i64, t_limit as i64, t_count) {
                                    // C++ U_BUFFER_OVERFLOW_ERROR
                                    return Err(LuceneError::unsupported_operation("tertiary tailoring gap too small"));
                                }
                                t_is_tailored = true;
                            }
                            t = tertiaries.next_weight() as i32;
                            debug_assert!(t != -1);
                        } else {
                            t = weight16_from_node(node);
                            t_is_tailored = false;
                        }
                    } else {
                        if strength == collator::SECONDARY {
                            if is_tailored_node(node) {
                                if !s_is_tailored {
                                    // First tailored secondary node for p.
                                    let s_count =
                                        count_tailored_nodes(self.nodes.get_buffer(), next_index, collator::SECONDARY) + 1;
                                    let s_limit;
                                    if s == 0 {
                                        // Gap at the beginning of the secondary CE range.
                                        s = self.root_elements.get_secondary_boundary() - 0x100;
                                        s_limit = (self.root_elements.get_first_secondary_ce() >> 16) as i32;
                                    } else if !p_is_tailored {
                                        // p is a root primary.
                                        s_limit = self.root_elements.get_secondary_after(p_index, s);
                                    } else if s == Collation::BEFORE_WEIGHT16 {
                                        s_limit = Collation::COMMON_WEIGHT16;
                                    } else {
                                        // p is a tailored primary.
                                        debug_assert!(s == Collation::COMMON_WEIGHT16);
                                        s_limit = self.root_elements.get_secondary_boundary();
                                    }
                                    if s == Collation::COMMON_WEIGHT16 {
                                        // Do not tailor into the getSortKey() range of
                                        // compressed common secondaries.
                                        s = self.root_elements.get_last_common_secondary();
                                    }
                                    secondaries.init_for_secondary();
                                    if !secondaries.alloc_weights(s as i64, s_limit as i64, s_count) {
                                        // C++ U_BUFFER_OVERFLOW_ERROR
                                        return Err(LuceneError::unsupported_operation("secondary tailoring gap too small"));
                                    }
                                    s_is_tailored = true;
                                }
                                s = secondaries.next_weight() as i32;
                                debug_assert!(s != -1);
                            } else {
                                s = weight16_from_node(node);
                                s_is_tailored = false;
                            }
                        } else {
                            debug_assert!(is_tailored_node(node));
                            if !p_is_tailored {
                                // First tailored primary node in this list.
                                let p_count = count_tailored_nodes(self.nodes.get_buffer(), next_index, collator::PRIMARY) + 1;
                                let is_compressible = self.base_data.is_compressible_primary(p);
                                let p_limit = self.root_elements.get_primary_after(p, p_index, is_compressible);
                                primaries.init_for_primary(is_compressible);
                                if !primaries.alloc_weights(p, p_limit, p_count) {
                                    // C++ U_BUFFER_OVERFLOW_ERROR  // TODO: introduce a more specific UErrorCode?
                                    return Err(LuceneError::unsupported_operation("primary tailoring gap too small"));
                                }
                                p_is_tailored = true;
                            }
                            p = primaries.next_weight();
                            debug_assert!(p != 0xffffffff);
                            s = Collation::COMMON_WEIGHT16;
                            s_is_tailored = false;
                        }
                        t = if s == 0 { 0 } else { Collation::COMMON_WEIGHT16 };
                        t_is_tailored = false;
                    }
                    q = 0;
                }
                if is_tailored_node(node) {
                    let ce = Collation::make_ce4(p, s, t, q);
                    self.nodes.set_element_at(ce, i);
                    if DEBUG {
                        println!("{ce:016x}");
                    }
                }
            }
        }
        Ok(())
    }

    fn finalize_ces(&mut self) -> Result<()> {
        let mut new_builder = CollationDataBuilder::new()?;
        new_builder.init_for_tailoring(Some(self.base_data.clone()))?;
        let mut finalizer = CEFinalizer { final_ces: self.nodes.get_buffer().to_vec() };
        let old = self.data_builder.take().unwrap_or_else(|| panic!("java.lang.NullPointerException"));
        new_builder.copy_from(&old, &mut finalizer)?;
        self.data_builder = Some(new_builder);
        Ok(())
    }
}

impl Sink for CollationBuilder {
    fn add_reset(&mut self, mut strength: i32, str: &[u16]) -> Result<()> {
        debug_assert!(!str.is_empty());
        if str[0] == POS_LEAD {
            self.ces[0] = self.get_special_reset_position(str)?;
            self.ces_length = 1;
            debug_assert!((self.ces[0] & Collation::CASE_AND_QUATERNARY_MASK as i64) == 0);
        } else {
            // normal reset to a character or string
            let nfd_string = self.nfd.normalize_str(str)?;
            let mut ces = self.ces;
            self.ces_length = self.db().get_ces(&nfd_string, &mut ces, 0)?;
            self.ces = ces;
            if self.ces_length > Collation::MAX_EXPANSION_LENGTH {
                return Err(LuceneError::illegal_argument(
                    "reset position maps to too many collation elements (more than 31)",
                ));
            }
        }
        if strength == collator::IDENTICAL {
            return Ok(());
        } // simple reset-at-position

        // &[before strength]position
        debug_assert!((collator::PRIMARY..=collator::TERTIARY).contains(&strength));
        let mut index = self.find_or_insert_node_for_ces(strength)?;

        let mut node = self.nodes.element_ati(index);
        // If the index is for a "weaker" node,
        // then skip backwards over this and further "weaker" nodes.
        while strength_from_node(node) > strength {
            index = previous_index_from_node(node);
            node = self.nodes.element_ati(index);
        }

        // Find or insert a node whose index we will put into a temporary CE.
        if strength_from_node(node) == strength && is_tailored_node(node) {
            // Reset to just before this same-strength tailored node.
            index = previous_index_from_node(node);
        } else if strength == collator::PRIMARY {
            // root primary node (has no previous index)
            let mut p = weight32_from_node(node);
            if p == 0 {
                return Err(LuceneError::unsupported_operation("reset primary-before ignorable not possible"));
            }
            if p <= self.root_elements.get_first_primary() {
                // There is no primary gap between ignorables and the space-first-primary.
                return Err(LuceneError::unsupported_operation(
                    "reset primary-before first non-ignorable not supported",
                ));
            }
            if p == Collation::FIRST_TRAILING_PRIMARY {
                // We do not support tailoring to an unassigned-implicit CE.
                return Err(LuceneError::unsupported_operation("reset primary-before [first trailing] not supported"));
            }
            p = self.root_elements.get_primary_before(p, self.base_data.is_compressible_primary(p));
            index = self.find_or_insert_node_for_primary(p);
            // Go to the last node in this list:
            // Tailor after the last node between adjacent root nodes.
            loop {
                node = self.nodes.element_ati(index);
                let next_index = next_index_from_node(node);
                if next_index == 0 {
                    break;
                }
                index = next_index;
            }
        } else {
            // &[before 2] or &[before 3]
            index = self.find_common_node(index, collator::SECONDARY);
            if strength >= collator::TERTIARY {
                index = self.find_common_node(index, collator::TERTIARY);
            }
            // findCommonNode() stayed on the stronger node or moved to
            // an explicit common-weight node of the reset-before strength.
            node = self.nodes.element_ati(index);
            if strength_from_node(node) == strength {
                // Found a same-strength node with an explicit weight.
                let mut weight16 = weight16_from_node(node);
                if weight16 == 0 {
                    return Err(LuceneError::unsupported_operation(if strength == collator::SECONDARY {
                        "reset secondary-before secondary ignorable not possible"
                    } else {
                        "reset tertiary-before completely ignorable not possible"
                    }));
                }
                debug_assert!(weight16 > Collation::BEFORE_WEIGHT16);
                // Reset to just before this node.
                // Insert the preceding same-level explicit weight if it is not there already.
                // Which explicit weight immediately precedes this one?
                weight16 = self.get_weight16_before(index, node, strength);
                // Does this preceding weight have a node?
                let previous_weight16;
                let previous_index = previous_index_from_node(node);
                let mut i = previous_index;
                loop {
                    node = self.nodes.element_ati(i);
                    let previous_strength = strength_from_node(node);
                    if previous_strength < strength {
                        debug_assert!(weight16 >= Collation::COMMON_WEIGHT16 || i == previous_index);
                        // Either the reset element has an above-common weight and
                        // the parent node provides the implied common weight,
                        // or the reset element has a weight<=common in the node
                        // right after the parent, and we need to insert the preceding weight.
                        previous_weight16 = Collation::COMMON_WEIGHT16;
                        break;
                    } else if previous_strength == strength && !is_tailored_node(node) {
                        previous_weight16 = weight16_from_node(node);
                        break;
                    }
                    // Skip weaker nodes and same-level tailored nodes.
                    i = previous_index_from_node(node);
                }
                if previous_weight16 == weight16 {
                    // The preceding weight has a node,
                    // maybe with following weaker or tailored nodes.
                    // Reset to the last of them.
                    index = previous_index;
                } else {
                    // Insert a node with the preceding weight, reset to that.
                    node = node_from_weight16(weight16) | node_from_strength(strength);
                    index = self.insert_node_between(previous_index, index, node);
                }
            } else {
                // Found a stronger node with implied strength-common weight.
                let weight16 = self.get_weight16_before(index, node, strength);
                index = self.find_or_insert_weak_node(index, weight16, strength);
            }
            // Strength of the temporary CE = strength of its reset position.
            // Code above raises an error if the before-strength is stronger.
            strength = ce_strength(self.ces[(self.ces_length - 1) as usize]);
        }
        self.ces[(self.ces_length - 1) as usize] = temp_ce_from_index_and_strength(index, strength);
        Ok(())
    }

    fn add_relation(&mut self, strength: i32, prefix: &[u16], str: &[u16], extension: &[u16]) -> Result<()> {
        let nfd_prefix: Vec<u16> = if prefix.is_empty() { Vec::new() } else { self.nfd.normalize_str(prefix)? };
        let nfd_string = self.nfd.normalize_str(str)?;

        // The runtime code decomposes Hangul syllables on the fly,
        // with recursive processing but without making the Jamo pieces visible for matching.
        // It does not work with certain types of contextual mappings.
        let nfd_length = nfd_string.len();
        if nfd_length >= 2 {
            let mut c = nfd_string[0] as i32;
            if Hangul::is_jamo_l(c) || Hangul::is_jamo_v(c) {
                // While handling a Hangul syllable, contractions starting with Jamo L or V
                // would not see the following Jamo of that syllable.
                return Err(LuceneError::unsupported_operation(
                    "contractions starting with conjoining Jamo L or V not supported",
                ));
            }
            c = nfd_string[nfd_length - 1] as i32;
            if Hangul::is_jamo_l(c) || (Hangul::is_jamo_v(c) && Hangul::is_jamo_l(nfd_string[nfd_length - 2] as i32)) {
                // A contraction ending with Jamo L or L+V would require
                // generating Hangul syllables in addTailComposites() (588 for a Jamo L),
                // or decomposing a following Hangul syllable on the fly, during contraction matching.
                return Err(LuceneError::unsupported_operation(
                    "contractions ending with conjoining Jamo L or L+V not supported",
                ));
            }
            // A Hangul syllable completely inside a contraction is ok.
        }
        // Note: If there is a prefix, then the parser checked that
        // both the prefix and the string begin with NFC boundaries (not Jamo V or T).
        // Therefore: prefix.isEmpty() || !isJamoVOrT(nfdString.charAt(0))
        // (While handling a Hangul syllable, prefixes on Jamo V or T
        // would not see the previous Jamo of that syllable.)

        if strength != collator::IDENTICAL {
            // Find the node index after which we insert the new tailored node.
            let mut index = self.find_or_insert_node_for_ces(strength)?;
            debug_assert!(self.ces_length > 0);
            let ce = self.ces[(self.ces_length - 1) as usize];
            if strength == collator::PRIMARY && !is_temp_ce(ce) && ushr32(ce) == 0 {
                // There is no primary gap between ignorables and the space-first-primary.
                return Err(LuceneError::unsupported_operation("tailoring primary after ignorables not supported"));
            }
            if strength == collator::QUATERNARY && ce == 0 {
                // The CE data structure does not support non-zero quaternary weights
                // on tertiary ignorables.
                return Err(LuceneError::unsupported_operation(
                    "tailoring quaternary after tertiary ignorables not supported",
                ));
            }
            // Insert the new tailored node.
            index = self.insert_tailored_node_after(index, strength);
            // Strength of the temporary CE:
            // The new relation may yield a stronger CE but not a weaker one.
            let mut temp_strength = ce_strength(ce);
            if strength < temp_strength {
                temp_strength = strength;
            }
            self.ces[(self.ces_length - 1) as usize] = temp_ce_from_index_and_strength(index, temp_strength);
        }

        self.set_case_bits(&nfd_string)?;

        let ces_length_before_extension = self.ces_length;
        if !extension.is_empty() {
            let nfd_extension = self.nfd.normalize_str(extension)?;
            let mut ces = self.ces;
            let ces_length = self.ces_length;
            self.ces_length = self.db().get_ces(&nfd_extension, &mut ces, ces_length)?;
            self.ces = ces;
            if self.ces_length > Collation::MAX_EXPANSION_LENGTH {
                return Err(LuceneError::illegal_argument(
                    "extension string adds too many collation elements (more than 31 total)",
                ));
            }
        }
        let mut ce32 = Collation::UNASSIGNED_CE32;
        let ces = self.ces;
        let ces_length = self.ces_length;
        if (nfd_prefix[..] != prefix[..] || nfd_string[..] != str[..]) && !self.ignore_prefix(prefix)? && !self.ignore_string(str)? {
            // Map from the original input to the CEs.
            // We do this in case the canonical closure is incomplete,
            // so that it is possible to explicitly provide the missing mappings.
            ce32 = self.add_if_different(prefix, str, &ces, ces_length, ce32)?;
        }
        self.add_with_closure(&nfd_prefix, &nfd_string, &ces, ces_length, ce32)?;
        self.ces_length = ces_length_before_extension;
        Ok(())
    }

    fn suppress_contractions(&mut self, set: &UnicodeSet) -> Result<()> {
        self.db().suppress_contractions(set)
    }

    fn optimize(&mut self, set: &UnicodeSet) -> Result<()> {
        self.optimize_set.add_all(set)?;
        Ok(())
    }
}
