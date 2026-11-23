// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.CharacterIteration`: code-point iteration over a
//! `java.text.CharacterIterator`.

use rucene_analysis_common::jdk::character_iterator::{CharacterIterator, DONE};

use crate::text::utf16::UTF16;

/// The value the code-point functions return past either end of the text:
/// `DONE32`, `0x7fffffff`.
pub const DONE32: i32 = 0x7fff_ffff;

/// `CharacterIterator.setIndex(position)` for a position the ICU caller
/// keeps inside the text; returns the unit there.
///
/// # Panics
///
/// With the `IllegalArgumentException` text of the iterator when `position`
/// is outside the text: ICU never catches that exception (precedent
/// `CharacterIteratorWrapper.setToLimit`).
pub(crate) fn set_index(ci: &mut dyn CharacterIterator, position: i32) -> u16 {
    match ci.set_index(position) {
        Ok(c) => c,
        Err(e) => panic!("{e}"),
    }
}

/// Static code-point iteration helpers over a `CharacterIterator`.
///
/// Equivalent to the final class `com.ibm.icu.impl.CharacterIteration`; its
/// static methods are the functions of this module.
pub struct CharacterIteration;

/// `next32(CharacterIterator ci)`: moves past the code point at the index
/// and returns the following one, or [`DONE32`] at the end; the iterator is
/// left on the start of the returned code point.
pub fn next32(ci: &mut dyn CharacterIterator) -> i32 {
    // If the current position is at a surrogate pair, move to the trail surrogate
    //   which leaves it in position for underlying iterator's next() to work.
    let mut c = ci.current() as i32;
    if (UTF16::LEAD_SURROGATE_MIN_VALUE..=UTF16::LEAD_SURROGATE_MAX_VALUE).contains(&c) {
        c = ci.next() as i32;
        if !(UTF16::TRAIL_SURROGATE_MIN_VALUE..=UTF16::TRAIL_SURROGATE_MAX_VALUE).contains(&c) {
            ci.previous();
        }
    }

    // For BMP chars, this next() is the real deal.
    c = ci.next() as i32;

    // If we might have a lead surrogate, we need to peak ahead to get the trail
    //  even though we don't want to really be positioned there.
    if c >= UTF16::LEAD_SURROGATE_MIN_VALUE {
        c = next_trail32(ci, c);
    }

    if c >= UTF16::SUPPLEMENTARY_MIN_VALUE && c != DONE32 {
        // We got a supplementary char.  Back the iterator up to the postion
        // of the lead surrogate.
        ci.previous();
    }
    c
}

/// `nextTrail32(CharacterIterator ci, int lead)`: the code point that
/// starts with the unit `lead` just read; for a lead surrogate followed by a
/// trail surrogate the iterator is left on the trail.
pub fn next_trail32(ci: &mut dyn CharacterIterator, lead: i32) -> i32 {
    if lead == DONE as i32 && ci.get_index() >= ci.get_end_index() {
        return DONE32;
    }
    let mut ret_val = lead;
    if lead <= UTF16::LEAD_SURROGATE_MAX_VALUE {
        let c_trail = ci.next() as i32;
        if UTF16::is_trail_surrogate(c_trail) {
            ret_val = ((lead - UTF16::LEAD_SURROGATE_MIN_VALUE) << 10)
                + (c_trail - UTF16::TRAIL_SURROGATE_MIN_VALUE)
                + UTF16::SUPPLEMENTARY_MIN_VALUE;
        } else {
            ci.previous();
        }
    }
    ret_val
}

/// `previous32(CharacterIterator ci)`: moves back over one code point and
/// returns it, or [`DONE32`] at the start.
pub fn previous32(ci: &mut dyn CharacterIterator) -> i32 {
    if ci.get_index() <= ci.get_begin_index() {
        return DONE32;
    }
    let trail = ci.previous() as i32;
    let mut ret_val = trail;
    if UTF16::is_trail_surrogate(trail) && ci.get_index() > ci.get_begin_index() {
        let lead = ci.previous() as i32;
        if UTF16::is_lead_surrogate(lead) {
            ret_val = ((lead - UTF16::LEAD_SURROGATE_MIN_VALUE) << 10)
                + (trail - UTF16::TRAIL_SURROGATE_MIN_VALUE)
                + UTF16::SUPPLEMENTARY_MIN_VALUE;
        } else {
            ci.next();
        }
    }
    ret_val
}

/// `current32(CharacterIterator ci)`: the code point at the index, or
/// [`DONE32`] at the end; the index does not move.
pub fn current32(ci: &mut dyn CharacterIterator) -> i32 {
    let lead = ci.current() as i32;
    let mut ret_val = lead;
    if ret_val < UTF16::LEAD_SURROGATE_MIN_VALUE {
        return ret_val;
    }
    if UTF16::is_lead_surrogate(lead) {
        let trail = ci.next() as i32;
        ci.previous();
        if UTF16::is_trail_surrogate(trail) {
            ret_val = ((lead - UTF16::LEAD_SURROGATE_MIN_VALUE) << 10)
                + (trail - UTF16::TRAIL_SURROGATE_MIN_VALUE)
                + UTF16::SUPPLEMENTARY_MIN_VALUE;
        }
    } else if lead == DONE as i32 && ci.get_index() >= ci.get_end_index() {
        ret_val = DONE32;
    }
    ret_val
}
