// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.TransliteratorIDParser`.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use rucene::analysis::factories::java_equals_ignore_case;
use rucene::error::{LuceneError, Result};

use crate::jdk::{to_rust_string, units, ParsePosition};
use crate::r#impl::pattern_props::PatternProps;
use crate::r#impl::utility::Utility;
use crate::text::transliterator::{self, Transliterator, FORWARD, REVERSE};
use crate::text::unicode_set::UnicodeSet;
use crate::util::case_insensitive_string::CaseInsensitiveString;

const ID_DELIM: u16 = b';' as u16;
const TARGET_SEP: u16 = b'-' as u16;
const VARIANT_SEP: u16 = b'/' as u16;
const OPEN_REV: u16 = b'(' as u16;
const CLOSE_REV: u16 = b')' as u16;
const ANY: &str = "Any";

/// `SPECIAL_INVERSES`: special inverses, keyed by target, case-insensitive
/// (`Collections.synchronizedMap(new HashMap<...>())`).
static SPECIAL_INVERSES: LazyLock<Mutex<HashMap<CaseInsensitiveString, Vec<u16>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn special_inverses() -> MutexGuard<'static, HashMap<CaseInsensitiveString, Vec<u16>>> {
    SPECIAL_INVERSES.lock().unwrap_or_else(|e| e.into_inner())
}

/// `String.equalsIgnoreCase(String)` of two UTF-16 strings.
pub(crate) fn equals_ignore_case(a: &[u16], b: &[u16]) -> bool {
    java_equals_ignore_case(&to_rust_string(a), &to_rust_string(b))
}

/// Parsing component for transliterator IDs. This class contains only
/// static members; it cannot be instantiated. Methods in this class parse
/// various ID formats, including the following:
///
/// * A basic ID, which contains source, target, and variant, but no filter
///   and no explicit inverse. Examples include "Latin-Greek/UNGEGN" and
///   "Null".
/// * A single ID, which is a basic ID plus optional filter and optional
///   explicit inverse. Examples include "[a-zA-Z] Latin-Greek" and
///   "Lower (Upper)".
/// * A compound ID, which is a sequence of one or more single IDs,
///   separated by semicolons, with optional forward and reverse global
///   filters. The global filters are UnicodeSet patterns prepended or
///   appended to the IDs, separated by semicolons. An appended filter must
///   be enclosed in parentheses and applies in the reverse direction.
///
/// Equivalent to the package-private class
/// `com.ibm.icu.text.TransliteratorIDParser`.
pub(crate) struct TransliteratorIDParser;

/// A structure containing the parsed data of a filtered ID, that is, a
/// basic ID optionally with a filter.
///
/// Equivalent to the nested class `TransliteratorIDParser.Specs`.
#[derive(Debug, Clone)]
struct Specs {
    source: Vec<u16>,          // not null
    target: Vec<u16>,          // not null
    variant: Option<Vec<u16>>, // may be null
    filter: Option<Vec<u16>>,  // may be null
    saw_source: bool,
}

/// A structure containing the canonicalized data of a filtered ID, that
/// is, a basic ID optionally with a filter.
///
/// Equivalent to the nested class `TransliteratorIDParser.SingleID`:
/// `canonID` is the canonicalized ID ("Any-Latin"), `basicID` the ID used
/// to look up the registry, `filter` the pattern of the filter or `None`.
#[derive(Debug, Clone)]
pub(crate) struct SingleID {
    pub canon_id: Vec<u16>,
    pub basic_id: Vec<u16>,
    pub filter: Option<Vec<u16>>,
}

impl SingleID {
    /// `SingleID(String c, String b, String f)`.
    fn new(c: Vec<u16>, b: Vec<u16>, f: Option<Vec<u16>>) -> Self {
        Self {
            canon_id: c,
            basic_id: b,
            filter: f,
        }
    }

    /// `getInstance()`: the transliterator for this ID, with its filter
    /// set, or `None` when the registry has no such transliterator.
    ///
    /// # Errors
    ///
    /// The errors of the instantiation and of the filter pattern.
    pub fn get_instance(&self) -> Result<Option<Arc<dyn Transliterator>>> {
        let t = if self.basic_id.is_empty() {
            transliterator::get_basic_instance(&units("Any-Null"), Some(&self.canon_id))?
        } else {
            transliterator::get_basic_instance(&self.basic_id, Some(&self.canon_id))?
        };
        if let Some(t) = &t {
            if let Some(filter) = &self.filter {
                t.set_filter(Some(&UnicodeSet::from_pattern(filter)?))?;
            }
        }
        Ok(t)
    }
}

impl TransliteratorIDParser {
    /// `parseFilterID(String id, int[] pos)`: parse a filter ID, that is,
    /// an ID of the general form "[f1] s1-t1/v1", with the filters
    /// optional, and the variants optional.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a malformed filter pattern.
    pub fn parse_filter_id(id: &[u16], pos: &mut [i32]) -> Result<Option<SingleID>> {
        let start = pos[0];
        let Some(specs) = Self::parse_filter_id_specs(id, pos, true)? else {
            pos[0] = start;
            return Ok(None);
        };

        // Assemble return results
        let mut single = Self::specs_to_id(Some(&specs), FORWARD);
        single.filter = specs.filter.clone();
        Ok(Some(single))
    }

    /// `parseSingleID(String id, int[] pos, int dir)`: parse a single ID,
    /// that is, an ID of the general form "[f1] s1-t1/v1 ([f2] s2-t3/v2)",
    /// with the parenthesized element optional, the filter optional, and
    /// the variants optional.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a malformed filter pattern.
    pub fn parse_single_id(id: &[u16], pos: &mut [i32], dir: i32) -> Result<Option<SingleID>> {
        let start = pos[0];

        // The ID will be of the form A, A(), A(B), or (B), where
        // A and B are filter IDs.
        let mut specs_a: Option<Specs> = None;
        let mut specs_b: Option<Specs> = None;
        let mut saw_paren = false;

        // On the first pass, look for (B) or ().  If this fails, then
        // on the second pass, look for A, A(B), or A().
        for pass in 1..=2 {
            if pass == 2 {
                specs_a = Self::parse_filter_id_specs(id, pos, true)?;
                if specs_a.is_none() {
                    pos[0] = start;
                    return Ok(None);
                }
            }
            if Utility::parse_char(id, pos, OPEN_REV) {
                saw_paren = true;
                if !Utility::parse_char(id, pos, CLOSE_REV) {
                    specs_b = Self::parse_filter_id_specs(id, pos, true)?;
                    // Must close with a ')'
                    if specs_b.is_none() || !Utility::parse_char(id, pos, CLOSE_REV) {
                        pos[0] = start;
                        return Ok(None);
                    }
                }
                break;
            }
        }

        // Assemble return results
        let mut single;
        if saw_paren {
            if dir == FORWARD {
                single = Self::specs_to_id(specs_a.as_ref(), FORWARD);
                let mut canon = single.canon_id.clone();
                canon.push(OPEN_REV);
                canon.extend_from_slice(&Self::specs_to_id(specs_b.as_ref(), FORWARD).canon_id);
                canon.push(CLOSE_REV);
                single.canon_id = canon;
                if let Some(a) = &specs_a {
                    single.filter = a.filter.clone();
                }
            } else {
                single = Self::specs_to_id(specs_b.as_ref(), FORWARD);
                let mut canon = single.canon_id.clone();
                canon.push(OPEN_REV);
                canon.extend_from_slice(&Self::specs_to_id(specs_a.as_ref(), FORWARD).canon_id);
                canon.push(CLOSE_REV);
                single.canon_id = canon;
                if let Some(b) = &specs_b {
                    single.filter = b.filter.clone();
                }
            }
        } else {
            let specs_a = specs_a.expect("pass 2 returned a Specs");
            // assert(specsA != null);
            if dir == FORWARD {
                single = Self::specs_to_id(Some(&specs_a), FORWARD);
            } else {
                single = match Self::specs_to_special_inverse(&specs_a) {
                    Some(s) => s,
                    None => Self::specs_to_id(Some(&specs_a), REVERSE),
                };
            }
            single.filter = specs_a.filter.clone();
        }

        Ok(Some(single))
    }

    /// `parseGlobalFilter(String id, int[] pos, int dir, int[] withParens,
    /// StringBuffer canonID)`: parse a global filter of the form "[f]" or
    /// "([f])", depending on `with_parens`.
    ///
    /// `with_parens[0]` is 0 for no parentheses, 1 for parentheses, or -1
    /// to accept either (it receives what was seen). `canon_id`, when
    /// given, receives the canonical filter text (appended for `FORWARD`,
    /// prepended for `REVERSE`).
    ///
    /// # Errors
    ///
    /// None in practice: an invalid filter pattern resets `pos` and returns
    /// `None`, as Java's `catch (IllegalArgumentException)` does.
    pub fn parse_global_filter(
        id: &[u16],
        pos: &mut [i32],
        dir: i32,
        with_parens: &mut [i32],
        canon_id: Option<&mut Vec<u16>>,
    ) -> Result<Option<UnicodeSet>> {
        let mut filter = None;
        let start = pos[0];

        if with_parens[0] == -1 {
            with_parens[0] = if Utility::parse_char(id, pos, OPEN_REV) {
                1
            } else {
                0
            };
        } else if with_parens[0] == 1 && !Utility::parse_char(id, pos, OPEN_REV) {
            pos[0] = start;
            return Ok(None);
        }

        pos[0] = PatternProps::skip_white_space(id, pos[0]);

        if UnicodeSet::resembles_pattern(id, pos[0]) {
            let mut ppos = ParsePosition::new(pos[0]);
            match UnicodeSet::from_pattern_pos(id, &mut ppos, None) {
                Ok(f) => filter = Some(f),
                Err(e) if e.is_illegal_argument_exception() => {
                    pos[0] = start;
                    return Ok(None);
                }
                Err(e) => return Err(e),
            }

            let mut pattern = id[pos[0] as usize..ppos.get_index() as usize].to_vec();
            pos[0] = ppos.get_index();

            if with_parens[0] == 1 && !Utility::parse_char(id, pos, CLOSE_REV) {
                pos[0] = start;
                return Ok(None);
            }

            // In the forward direction, append the pattern to the
            // canonID.  In the reverse, insert it at zero, and invert
            // the presence of parens ("A" <-> "(A)").
            if let Some(canon_id) = canon_id {
                if dir == FORWARD {
                    if with_parens[0] == 1 {
                        pattern.insert(0, OPEN_REV);
                        pattern.push(CLOSE_REV);
                    }
                    canon_id.extend_from_slice(&pattern);
                    canon_id.push(ID_DELIM);
                } else {
                    if with_parens[0] == 0 {
                        pattern.insert(0, OPEN_REV);
                        pattern.push(CLOSE_REV);
                    }
                    pattern.push(ID_DELIM);
                    canon_id.splice(0..0, pattern);
                }
            }
        }

        Ok(filter)
    }

    /// `parseCompoundID(String id, int dir, StringBuffer canonID,
    /// List<SingleID> list, UnicodeSet[] globalFilter)`: parse a compound
    /// ID, consisting of an optional forward global filter, a separator,
    /// one or more single IDs delimited by separators, an optional
    /// separator, and an optional reverse global filter. The separator is
    /// a semicolon. The global filters are UnicodeSet patterns. The reverse
    /// global filter must be enclosed in parentheses. Returns true if the
    /// ID is well-formed.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a malformed filter pattern inside a
    /// single ID.
    pub fn parse_compound_id(
        id: &[u16],
        dir: i32,
        canon_id: &mut Vec<u16>,
        list: &mut Vec<SingleID>,
        global_filter: &mut Option<UnicodeSet>,
    ) -> Result<bool> {
        let mut pos = [0i32];
        let mut with_parens = [0i32];
        list.clear();
        *global_filter = None;
        canon_id.clear();

        // Parse leading global filter, if any
        with_parens[0] = 0; // parens disallowed
        let filter =
            Self::parse_global_filter(id, &mut pos, dir, &mut with_parens, Some(&mut *canon_id))?;
        if let Some(filter) = filter {
            if !Utility::parse_char(id, &mut pos, ID_DELIM) {
                // Not a global filter; backup and resume
                canon_id.clear();
                pos[0] = 0;
            }
            if dir == FORWARD {
                *global_filter = Some(filter);
            }
        }

        let mut saw_delimiter = true;
        loop {
            let Some(single) = Self::parse_single_id(id, &mut pos, dir)? else {
                break;
            };
            if dir == FORWARD {
                list.push(single);
            } else {
                list.insert(0, single);
            }
            if !Utility::parse_char(id, &mut pos, ID_DELIM) {
                saw_delimiter = false;
                break;
            }
        }

        if list.is_empty() {
            return Ok(false);
        }

        // Construct canonical ID
        for (i, single) in list.iter().enumerate() {
            canon_id.extend_from_slice(&single.canon_id);
            if i != list.len() - 1 {
                canon_id.push(ID_DELIM);
            }
        }

        // Parse trailing global filter, if any, and only if we saw
        // a trailing delimiter after the IDs.
        if saw_delimiter {
            with_parens[0] = 1; // parens required
            let filter = Self::parse_global_filter(
                id,
                &mut pos,
                dir,
                &mut with_parens,
                Some(&mut *canon_id),
            )?;
            if let Some(filter) = filter {
                // Don't require trailing ';', but parse it if present
                Utility::parse_char(id, &mut pos, ID_DELIM);

                if dir == REVERSE {
                    *global_filter = Some(filter);
                }
            }
        }

        // Trailing unparsed text is a syntax error
        pos[0] = PatternProps::skip_white_space(id, pos[0]);
        Ok(pos[0] == id.len() as i32)
    }

    /// `instantiateList(List<SingleID> ids)`: instantiates the single IDs,
    /// skipping empty basic IDs (`Any-Null` when nothing remains).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` ("Illegal ID ...") for an ID the registry
    /// does not know, and the errors of the instantiation.
    pub fn instantiate_list(ids: &[SingleID]) -> Result<Vec<Arc<dyn Transliterator>>> {
        let mut translits: Vec<Arc<dyn Transliterator>> = Vec::new();
        for single in ids {
            if single.basic_id.is_empty() {
                continue;
            }
            match single.get_instance()? {
                Some(t) => translits.push(t),
                None => {
                    return Err(LuceneError::illegal_argument(format!(
                        "Illegal ID {}",
                        to_rust_string(&single.canon_id)
                    )));
                }
            }
        }

        // An empty list is equivalent to a Null transliterator.
        if translits.is_empty() {
            match transliterator::get_basic_instance(&units("Any-Null"), None)? {
                Some(t) => translits.push(t),
                None => {
                    // Should never happen
                    return Err(LuceneError::illegal_argument(
                        "Internal error; cannot instantiate Any-Null",
                    ));
                }
            }
        }
        Ok(translits)
    }

    /// `IDtoSTV(String id)`: parse an ID into pieces. Take IDs of the form
    /// T, T/V, S-T, S-T/V, or S/V-T. If the source is missing, return a
    /// source of ANY. Returns `(source, target, variant, sourcePresent)`,
    /// where `sourcePresent` is `Some("")` when the source is present and
    /// `None` otherwise (Java's fourth array element).
    pub fn id_to_stv(id: &[u16]) -> (Vec<u16>, Vec<u16>, Vec<u16>, Option<Vec<u16>>) {
        let mut source = units(ANY);
        let target;
        let mut variant;

        let mut sep = crate::jdk::index_of_unit(id, TARGET_SEP, 0);
        let mut var = crate::jdk::index_of_unit(id, VARIANT_SEP, 0);
        if var < 0 {
            var = id.len() as i32;
        }
        let mut is_source_present = false;

        if sep < 0 {
            // Form: T/V or T (or /V)
            target = id[..var as usize].to_vec();
            variant = id[var as usize..].to_vec();
        } else if sep < var {
            // Form: S-T/V or S-T (or -T/V or -T)
            if sep > 0 {
                source = id[..sep as usize].to_vec();
                is_source_present = true;
            }
            sep += 1;
            target = id[sep as usize..var as usize].to_vec();
            variant = id[var as usize..].to_vec();
        } else {
            // Form: (S/V-T or /V-T)
            if var > 0 {
                source = id[..var as usize].to_vec();
                is_source_present = true;
            }
            variant = id[var as usize..sep as usize].to_vec();
            sep += 1;
            target = id[sep as usize..].to_vec();
        }

        if !variant.is_empty() {
            variant = variant[1..].to_vec();
        }

        (
            source,
            target,
            variant,
            if is_source_present {
                Some(Vec::new())
            } else {
                None
            },
        )
    }

    /// `STVtoID(String source, String target, String variant)`: given
    /// source, target, and variant strings, concatenate them into a full
    /// ID. If the source is empty, then "Any" will be used for the source,
    /// so the ID will always be of the form s-t/v or s-t.
    pub fn stv_to_id(source: &[u16], target: &[u16], variant: Option<&[u16]>) -> Vec<u16> {
        let mut id = source.to_vec();
        if id.is_empty() {
            id = units(ANY);
        }
        id.push(TARGET_SEP);
        id.extend_from_slice(target);
        if let Some(variant) = variant {
            if !variant.is_empty() {
                id.push(VARIANT_SEP);
                id.extend_from_slice(variant);
            }
        }
        id
    }

    /// `registerSpecialInverse(String target, String inverseTarget, boolean
    /// bidirectional)`: register two targets as being inverses of one
    /// another. For example, calling `registerSpecialInverse("NFC", "NFD",
    /// true)` causes Transliterator to form the following inverse
    /// relationships: NFC => NFD, Any-NFC => Any-NFD, NFD => NFC, Any-NFD
    /// => Any-NFC.
    pub fn register_special_inverse(target: &[u16], inverse_target: &[u16], bidirectional: bool) {
        let mut map = special_inverses();
        map.insert(
            CaseInsensitiveString::new(target.to_vec()),
            inverse_target.to_vec(),
        );
        if bidirectional && !equals_ignore_case(target, inverse_target) {
            map.insert(
                CaseInsensitiveString::new(inverse_target.to_vec()),
                target.to_vec(),
            );
        }
    }

    /// The private `parseFilterID(String id, int[] pos, boolean
    /// allowFilter)`: parse an ID into component pieces. Take IDs of the
    /// form T, T/V, S-T, S-T/V, or S/V-T. If the source is missing, return
    /// a source of ANY.
    fn parse_filter_id_specs(
        id: &[u16],
        pos: &mut [i32],
        allow_filter: bool,
    ) -> Result<Option<Specs>> {
        let mut first: Option<Vec<u16>> = None;
        let mut source: Option<Vec<u16>> = None;
        let mut target: Option<Vec<u16>> = None;
        let mut variant: Option<Vec<u16>> = None;
        let mut filter: Option<Vec<u16>> = None;
        let mut delimiter: u16 = 0;
        let mut spec_count = 0;
        let start = pos[0];

        // This loop parses one of the following things with each
        // pass: a filter, a delimiter character (either '-' or '/'),
        // or a spec (source, target, or variant).
        loop {
            pos[0] = PatternProps::skip_white_space(id, pos[0]);
            if pos[0] == id.len() as i32 {
                break;
            }

            // Parse filters
            if allow_filter && filter.is_none() && UnicodeSet::resembles_pattern(id, pos[0]) {
                let mut ppos = ParsePosition::new(pos[0]);
                // Parse the set to get the position.
                UnicodeSet::from_pattern_pos(id, &mut ppos, None)?;
                filter = Some(id[pos[0] as usize..ppos.get_index() as usize].to_vec());
                pos[0] = ppos.get_index();
                continue;
            }

            if delimiter == 0 {
                let c = id[pos[0] as usize];
                if (c == TARGET_SEP && target.is_none()) || (c == VARIANT_SEP && variant.is_none())
                {
                    delimiter = c;
                    pos[0] += 1;
                    continue;
                }
            }

            // We are about to try to parse a spec with no delimiter
            // when we can no longer do so (we can only do so at the
            // start); break.
            if delimiter == 0 && spec_count > 0 {
                break;
            }

            let Some(spec) = Utility::parse_unicode_identifier(id, pos) else {
                // Note that if there was a trailing delimiter, we
                // consume it.  So Foo-, Foo/, Foo-Bar/, and Foo/Bar-
                // are legal.
                break;
            };

            match delimiter {
                0 => first = Some(spec),
                TARGET_SEP => target = Some(spec),
                VARIANT_SEP => variant = Some(spec),
                _ => {}
            }
            spec_count += 1;
            delimiter = 0;
        }

        // A spec with no prior character is either source or target,
        // depending on whether an explicit "-target" was seen.
        if let Some(first) = first {
            if target.is_none() {
                target = Some(first);
            } else {
                source = Some(first);
            }
        }

        // Must have either source or target
        if source.is_none() && target.is_none() {
            pos[0] = start;
            return Ok(None);
        }

        // Empty source or target defaults to ANY
        let mut saw_source = true;
        let source = match source {
            Some(s) => s,
            None => {
                saw_source = false;
                units(ANY)
            }
        };
        let target = target.unwrap_or_else(|| units(ANY));

        Ok(Some(Specs {
            source,
            target,
            variant,
            saw_source,
            filter,
        }))
    }

    /// `specsToID(Specs specs, int dir)`: givens a Specs object, convert it
    /// to a SingleID object. The Spec object is a more unprocessed parse
    /// result. The SingleID object contains information about canonical and
    /// basic IDs.
    fn specs_to_id(specs: Option<&Specs>, dir: i32) -> SingleID {
        let mut canon_id = Vec::new();
        let mut basic_id = Vec::new();
        let mut basic_prefix = Vec::new();
        if let Some(specs) = specs {
            let mut buf = Vec::new();
            if dir == FORWARD {
                if specs.saw_source {
                    buf.extend_from_slice(&specs.source);
                    buf.push(TARGET_SEP);
                } else {
                    basic_prefix = specs.source.clone();
                    basic_prefix.push(TARGET_SEP);
                }
                buf.extend_from_slice(&specs.target);
            } else {
                buf.extend_from_slice(&specs.target);
                buf.push(TARGET_SEP);
                buf.extend_from_slice(&specs.source);
            }
            if let Some(variant) = &specs.variant {
                buf.push(VARIANT_SEP);
                buf.extend_from_slice(variant);
            }
            basic_id = basic_prefix;
            basic_id.extend_from_slice(&buf);
            if let Some(filter) = &specs.filter {
                buf.splice(0..0, filter.iter().copied());
            }
            canon_id = buf;
        }
        SingleID::new(canon_id, basic_id, None)
    }

    /// `specsToSpecialInverse(Specs specs)`: given a Specs object, return a
    /// SingleID representing the special inverse of that ID. If there is
    /// no special inverse then return `None`.
    fn specs_to_special_inverse(specs: &Specs) -> Option<SingleID> {
        if !equals_ignore_case(&specs.source, &units(ANY)) {
            return None;
        }
        let inverse_target = special_inverses()
            .get(&CaseInsensitiveString::new(specs.target.clone()))
            .cloned()?;
        // If the original ID contained "Any-" then make the
        // special inverse "Any-Foo"; otherwise make it "Foo".
        // So "Any-NFC" => "Any-NFD" but "NFC" => "NFD".
        let mut buf = Vec::new();
        if let Some(filter) = &specs.filter {
            buf.extend_from_slice(filter);
        }
        if specs.saw_source {
            buf.extend_from_slice(&units(ANY));
            buf.push(TARGET_SEP);
        }
        buf.extend_from_slice(&inverse_target);

        let mut basic_id = units(ANY);
        basic_id.push(TARGET_SEP);
        basic_id.extend_from_slice(&inverse_target);

        if let Some(variant) = &specs.variant {
            buf.push(VARIANT_SEP);
            buf.extend_from_slice(variant);
            basic_id.push(VARIANT_SEP);
            basic_id.extend_from_slice(variant);
        }
        Some(SingleID::new(buf, basic_id, None))
    }
}
