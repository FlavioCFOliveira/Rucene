// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.CharacterPropertiesImpl`.

use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};

use crate::lang::u_character::UCharacter;
use crate::lang::u_property::UProperty;
use crate::r#impl::emoji_props::EmojiProps;
use crate::r#impl::norm2_all_modes::Norm2AllModes;
use crate::r#impl::u_bidi_props::UBiDiProps;
use crate::r#impl::u_case_props::UCaseProps;
use crate::r#impl::u_character_property::{self as ucp, UCharacterProperty};
use crate::text::unicode_set::UnicodeSet;

const NUM_INCLUSIONS: usize = (ucp::SRC_COUNT + UProperty::INT_LIMIT - UProperty::INT_START) as usize;

/// A set of UnicodeSet objects, one per property source and int property;
/// `null` until computed (Java's `synchronized` static methods guard it).
static INCLUSIONS: Mutex<Vec<Option<Arc<UnicodeSet>>>> = Mutex::new(Vec::new());

/// Properties functionality above class UCharacterProperty but below class
/// CharacterProperties and class UnicodeSet.
///
/// Equivalent to `com.ibm.icu.impl.CharacterPropertiesImpl`.
pub struct CharacterPropertiesImpl;

fn lock() -> std::sync::MutexGuard<'static, Vec<Option<Arc<UnicodeSet>>>> {
    let mut g = INCLUSIONS.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_empty() {
        g.resize(NUM_INCLUSIONS, None);
    }
    g
}

impl CharacterPropertiesImpl {
    /// `clear()`: for Unicode set default symbol table changes.
    pub fn clear() {
        let mut g = lock();
        for x in g.iter_mut() {
            *x = None;
        }
    }

    fn get_inclusions_for_source(inclusions: &mut [Option<Arc<UnicodeSet>>], src: i32) -> Result<Arc<UnicodeSet>> {
        if inclusions[src as usize].is_none() {
            let mut incl = UnicodeSet::new();
            match src {
                ucp::SRC_CHAR => {
                    UCharacterProperty::instance().add_property_starts(&mut incl)?;
                }
                ucp::SRC_PROPSVEC => UCharacterProperty::instance().upropsvec_add_property_starts(&mut incl)?,
                ucp::SRC_CHAR_AND_PROPSVEC => {
                    UCharacterProperty::instance().add_property_starts(&mut incl)?;
                    UCharacterProperty::instance().upropsvec_add_property_starts(&mut incl)?;
                }
                ucp::SRC_CASE_AND_NORM => {
                    Norm2AllModes::get_nfc_instance()?.impl_.add_property_starts(&mut incl)?;
                    UCaseProps::instance().add_property_starts(&mut incl)?;
                }
                ucp::SRC_NFC => Norm2AllModes::get_nfc_instance()?.impl_.add_property_starts(&mut incl)?,
                ucp::SRC_NFKC => Norm2AllModes::get_nfkc_instance()?.impl_.add_property_starts(&mut incl)?,
                ucp::SRC_NFKC_CF => Norm2AllModes::get_nfkc_cf_instance()?.impl_.add_property_starts(&mut incl)?,
                ucp::SRC_NFC_CANON_ITER => Norm2AllModes::get_nfc_instance()?.impl_.add_canon_iter_property_starts(&mut incl)?,
                ucp::SRC_CASE => UCaseProps::instance().add_property_starts(&mut incl)?,
                ucp::SRC_BIDI => UBiDiProps::instance().add_property_starts(&mut incl)?,
                ucp::SRC_INPC | ucp::SRC_INSC | ucp::SRC_VO => UCharacterProperty::ulayout_add_property_starts(src, &mut incl)?,
                ucp::SRC_EMOJI => {
                    EmojiProps::instance().add_property_starts(&mut incl)?;
                }
                ucp::SRC_IDSU => {
                    // New in Unicode 15.1 for just two characters.
                    incl.add(0x2FFE)?;
                    incl.add(0x2FFF + 1)?;
                }
                ucp::SRC_ID_COMPAT_MATH => UCharacterProperty::math_compat_add_property_starts(&mut incl)?,
                ucp::SRC_MCM => UCharacterProperty::mcm_add_property_starts(&mut incl)?,
                ucp::SRC_BLOCK => UCharacterProperty::instance().ublock_add_property_starts(&mut incl)?,
                _ => {
                    return Err(LuceneError::illegal_state(format!("getInclusions(unknown src {src})")));
                }
            }
            // We do not freeze() the set because we only iterate over it,
            // rather than testing contains(),
            // so the extra time and memory to optimize that are not necessary.
            incl.compact()?;
            inclusions[src as usize] = Some(Arc::new(incl));
        }
        Ok(inclusions[src as usize].clone().expect("set above"))
    }

    fn get_int_prop_inclusions(inclusions: &mut [Option<Arc<UnicodeSet>>], prop: i32) -> Result<Arc<UnicodeSet>> {
        debug_assert!(UProperty::INT_START <= prop && prop < UProperty::INT_LIMIT);
        let incl_index = (ucp::SRC_COUNT + prop - UProperty::INT_START) as usize;
        if let Some(i) = &inclusions[incl_index] {
            return Ok(i.clone());
        }
        let src = UCharacterProperty::instance().get_source(prop);
        let incl = Self::get_inclusions_for_source(inclusions, src)?;

        let mut int_prop_incl = UnicodeSet::new_range_unchecked(0, 0);
        let num_ranges = incl.get_range_count();
        let mut prev_value = 0;
        for i in 0..num_ranges {
            let range_end = incl.get_range_end(i);
            for c in incl.get_range_start(i)..=range_end {
                // TODO: Get a UCharacterProperty.IntProperty to avoid the property dispatch.
                let value = UCharacter::get_int_property_value(c, prop);
                if value != prev_value {
                    int_prop_incl.add(c)?;
                    prev_value = value;
                }
            }
        }

        // Compact for caching.
        int_prop_incl.compact()?;
        let r = Arc::new(int_prop_incl);
        inclusions[incl_index] = Some(r.clone());
        Ok(r)
    }

    /// `getInclusionsForProperty(int prop)`: a set of the code points that
    /// start same-value ranges for the property; do not modify it (shared).
    ///
    /// # Errors
    ///
    /// `IllegalStateException` for a property without a data source; the
    /// errors of the property data.
    pub fn get_inclusions_for_property(prop: i32) -> Result<Arc<UnicodeSet>> {
        let mut g = lock();
        if UProperty::INT_START <= prop && prop < UProperty::INT_LIMIT {
            Self::get_int_prop_inclusions(&mut g, prop)
        } else {
            let src = UCharacterProperty::instance().get_source(prop);
            Self::get_inclusions_for_source(&mut g, src)
        }
    }
}
