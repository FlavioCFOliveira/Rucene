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

//! `java.text.Collator`, `RuleBasedCollator`, `CollationKey` and the
//! classes behind them (`RBCollationTables`, `RBTableBuilder`,
//! `MergeCollation`, `PatternEntry`, `EntryPair`,
//! `CollationElementIterator`), with the parts of
//! `jdk.internal.icu.impl.NormalizerImpl` (`canonicalDecomposeWithSingleQuotation`,
//! `getDecompose`), `sun.text.ComposedCharIter` and the NFD/NFKD
//! normalization they read, as `Collator.getInstance(Locale)` builds them on
//! JDK 25: the collator of `org.apache.lucene.collation` (whose keys
//! `CollationDocValuesField` writes to the index) and of the test
//! framework's `LuceneTestCase.collate`, which uses this module.
//!
//! Written from the JDK 25 sources read as specification (see
//! [`crate::jdk`]); the data (the root rules, the canonical and
//! compatibility decompositions, the canonical combining classes, the locale
//! tailorings) is generated from the JDK by `tools/gen/CollatorData.java`
//! into `collator_data.rs`, which also checks, on every available locale,
//! the locale resolution [`RuleBasedCollator::get_instance`] implements.
//! `crates/rucene-test-framework/tests/collator_oracle.rs` compares results
//! with the JDK on the grid of `CollatorOracle`.
//!
//! **Divergences.**
//! * Strings are Rust `&str`, so a Java string holding an unpaired
//!   surrogate cannot be passed; the algorithms run on the UTF-16 code units
//!   as Java's do.
//! * Java's `Collator` is abstract and `RuleBasedCollator` its only JDK
//!   implementation; this port has the one type [`RuleBasedCollator`]
//!   (`Collator`'s members included), and [`CollationKey`] is
//!   `RuleBasedCollationKey`.
//! * `Collator.getInstance(Locale)` reads the JDK's locale providers; here
//!   [`RuleBasedCollator::get_instance`] reads the tailorings those
//!   providers return (generated data) and resolves the locale as they do,
//!   a resolution the generator verifies against the JDK. Java caches one
//!   collator per locale (soft references) and returns clones; here the
//!   built tables are cached per tailoring and shared, which changes no
//!   result.
//! * Java's unchecked exceptions (an `ArrayIndexOutOfBoundsException` or
//!   `StringIndexOutOfBoundsException` some malformed rules raise, the
//!   `IllegalArgumentException` of `setStrength` and `setDecomposition`) are
//!   [`LuceneError`]s of the same class and message, and
//!   `java.text.ParseException` is `rucene`'s [`ParseException`].
//! * `compare` and `getCollationKey` are `synchronized` in Java to share
//!   cached iterators and buffers; here each call builds its own, which
//!   changes no result.
//! * `UCompactIntArray` and `IntHashtable` (`sun.text`) are a map with a
//!   default value and a set: the same lookups, without the JDK's
//!   compaction. `UCompactIntArray` keeps four planes (`PLANEMASK`
//!   `0x30000`), so code points `0x40000` apart share an order; the map
//!   reproduces it (`compact_index`).
//! * The NFD and NFKD texts the collation element iterator reads are
//!   normalized whole instead of segment by segment
//!   (`NormalizerBase.nextNormalize`); normalization is defined on the whole
//!   text, and ICU's segments end at normalization boundaries, so the code
//!   points the iterator reads are the same.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

use rucene::error::{LuceneError, Result};
use rucene::util::extra::{is_parse_exception, ParseException};

use super::collator_data::{
    COMBINING_CLASSES, COMPAT_DECOMPOSITIONS, DECOMPOSITIONS, ROOT_RULES, TAILORINGS,
    TAILORING_RULES,
};
use super::locale::Locale;

/// `Collator.PRIMARY`.
pub const PRIMARY: i32 = 0;
/// `Collator.SECONDARY`.
pub const SECONDARY: i32 = 1;
/// `Collator.TERTIARY`.
pub const TERTIARY: i32 = 2;
/// `Collator.IDENTICAL`.
pub const IDENTICAL: i32 = 3;
/// `Collator.NO_DECOMPOSITION`.
pub const NO_DECOMPOSITION: i32 = 0;
/// `Collator.CANONICAL_DECOMPOSITION`.
pub const CANONICAL_DECOMPOSITION: i32 = 1;
/// `Collator.FULL_DECOMPOSITION`.
pub const FULL_DECOMPOSITION: i32 = 2;

const LESS: i32 = -1;
const EQUAL: i32 = 0;
const GREATER: i32 = 1;

/// `CollationElementIterator.NULLORDER`.
pub const NULLORDER: i32 = -1;
/// `RBCollationTables.UNMAPPED`.
const UNMAPPED: i32 = -1;
/// `RBTableBuilder.CHARINDEX`.
const CHARINDEX: i32 = 0x7000_0000;
/// `RBCollationTables.EXPANDCHARINDEX`.
const EXPANDCHARINDEX: i32 = 0x7E00_0000;
/// `RBCollationTables.CONTRACTCHARINDEX`.
const CONTRACTCHARINDEX: i32 = 0x7F00_0000;
const PRIMARYORDERMASK: i32 = 0xffff_0000_u32 as i32;
const SECONDARYORDERMASK: i32 = 0x0000_ff00;
const TERTIARYORDERMASK: i32 = 0x0000_00ff;
const PRIMARYDIFFERENCEONLY: i32 = 0xffff_0000_u32 as i32;
const SECONDARYDIFFERENCEONLY: i32 = 0xffff_ff00_u32 as i32;
const IGNORABLEMASK: i32 = 0x0000_ffff;
const PRIMARYORDERINCREMENT: i32 = 0x0001_0000;
const SECONDARYORDERINCREMENT: i32 = 0x0000_0100;
const TERTIARYORDERINCREMENT: i32 = 0x0000_0001;
/// `CollationElementIterator.UNMAPPEDCHARVALUE`.
const UNMAPPEDCHARVALUE: i32 = 0x7FFF_0000;
/// `RuleBasedCollator.COLLATIONKEYOFFSET`.
const COLLATIONKEYOFFSET: i32 = 1;
/// `PatternEntry.RESET`.
const RESET: i32 = -2;
/// `PatternEntry.UNSET`.
const UNSET: i32 = -1;
/// `NormalizerBase.DONE`.
const DONE: i32 = -1;

// -----------------------------------------------------------------------------
// java.lang.Character / String helpers over UTF-16 code units.
// -----------------------------------------------------------------------------

fn is_high_surrogate(c: u16) -> bool {
    (0xD800..=0xDBFF).contains(&c)
}

fn is_low_surrogate(c: u16) -> bool {
    (0xDC00..=0xDFFF).contains(&c)
}

fn is_supplementary(cp: i32) -> bool {
    (0x10000..=0x10FFFF).contains(&cp)
}

fn to_code_point(high: u16, low: u16) -> i32 {
    ((i32::from(high) - 0xD800) << 10) + (i32::from(low) - 0xDC00) + 0x10000
}

/// `Character.charCount(cp)`.
fn char_count(cp: i32) -> usize {
    if cp >= 0x10000 {
        2
    } else {
        1
    }
}

/// `Character.toChars(cp)`.
fn to_chars(cp: i32) -> Vec<u16> {
    if is_supplementary(cp) {
        let v = cp - 0x10000;
        vec![(0xD800 + (v >> 10)) as u16, (0xDC00 + (v & 0x3FF)) as u16]
    } else {
        vec![cp as u16]
    }
}

/// `String.codePointAt(i)`.
fn code_point_at(s: &[u16], i: usize) -> i32 {
    let c = s[i];
    if is_high_surrogate(c) && i + 1 < s.len() && is_low_surrogate(s[i + 1]) {
        to_code_point(c, s[i + 1])
    } else {
        i32::from(c)
    }
}

/// The code points of `s`, as `UCharacterIterator.nextCodePoint` reads them.
fn code_points(s: &[u16]) -> Vec<i32> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let cp = code_point_at(s, i);
        out.push(cp);
        i += char_count(cp);
    }
    out
}

/// `String.compareTo`: the difference of the first unequal code units, else
/// of the lengths.
fn java_compare_to(a: &[u16], b: &[u16]) -> i32 {
    for (x, y) in a.iter().zip(b) {
        if x != y {
            return i32::from(*x) - i32::from(*y);
        }
    }
    a.len() as i32 - b.len() as i32
}

/// `String.hashCode()`.
fn java_hash_code(s: &[u16]) -> i32 {
    s.iter()
        .fold(0i32, |h, &c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
}

/// `new StringBuilder(s).reverse()`: the code units reversed, valid
/// surrogate pairs kept in order.
fn reverse_string(s: &[u16]) -> Vec<u16> {
    let mut v: Vec<u16> = s.iter().rev().copied().collect();
    let mut i = 0;
    while i + 1 < v.len() {
        if is_low_surrogate(v[i]) && is_high_surrogate(v[i + 1]) {
            v.swap(i, i + 1);
            i += 1;
        }
        i += 1;
    }
    v
}

/// `Character.isSpaceChar(c)` of a code unit: the space, line and paragraph
/// separators (Unicode 16).
fn is_space_char(c: u16) -> bool {
    matches!(
        c,
        0x0020 | 0x00A0 | 0x1680 | 0x2000..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000
    )
}

/// `a[i]` with Java's bounds check: `ArrayIndexOutOfBoundsException`.
fn array_get(a: &[u16], i: usize) -> Result<u16> {
    match a.get(i) {
        Some(&c) => Ok(c),
        None => Err(array_index_out_of_bounds(i, a.len())),
    }
}

/// `a[i] = v` with Java's bounds check.
fn array_set(a: &mut [u16], i: usize, v: u16) -> Result<()> {
    let len = a.len();
    match a.get_mut(i) {
        Some(c) => {
            *c = v;
            Ok(())
        }
        None => Err(array_index_out_of_bounds(i, len)),
    }
}

/// The JVM's `ArrayIndexOutOfBoundsException` for index `i` of an array of
/// `len` elements.
fn array_index_out_of_bounds(i: usize, len: usize) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.ArrayIndexOutOfBoundsException",
        format!("Index {i} out of bounds for length {len}"),
    )
}

fn to_java_string(s: &[u16]) -> String {
    String::from_utf16_lossy(s)
}

// -----------------------------------------------------------------------------
// Normalization data (jdk.internal.icu).
// -----------------------------------------------------------------------------

/// `Normalizer2.getNFDInstance().getDecomposition(cp)` (Hangul syllables
/// excepted, which no reader of this table asks for).
fn decomposition(cp: i32) -> Option<&'static [u16]> {
    let cp = u32::try_from(cp).ok()?;
    DECOMPOSITIONS
        .binary_search_by_key(&cp, |(c, _)| *c)
        .ok()
        .map(|i| DECOMPOSITIONS[i].1)
}

/// `Normalizer2.getNFKDInstance().getDecomposition(cp)` (Hangul syllables
/// excepted).
fn compat_decomposition(cp: i32) -> Option<&'static [u16]> {
    let c = u32::try_from(cp).ok()?;
    match COMPAT_DECOMPOSITIONS.binary_search_by_key(&c, |(x, _)| *x) {
        Ok(i) => Some(COMPAT_DECOMPOSITIONS[i].1),
        Err(_) => decomposition(cp),
    }
}

/// `UCharacter.getCombiningClass(cp)`.
fn combining_class(cp: i32) -> i32 {
    let Ok(c) = u32::try_from(cp) else {
        return 0;
    };
    COMBINING_CLASSES
        .binary_search_by_key(&c, |(x, _)| *x)
        .map_or(0, |i| i32::from(COMBINING_CLASSES[i].1))
}

const HANGUL_S_BASE: i32 = 0xAC00;
const HANGUL_L_BASE: i32 = 0x1100;
const HANGUL_V_BASE: i32 = 0x1161;
const HANGUL_T_BASE: i32 = 0x11A7;
const HANGUL_T_COUNT: i32 = 28;
const HANGUL_N_COUNT: i32 = 21 * 28;
const HANGUL_S_COUNT: i32 = 19 * 21 * 28;

/// `java.text.Normalizer.normalize(s, NFD)` (`compat` false) or `NFKD`:
/// the full decomposition of every code point (Hangul syllables
/// algorithmically), then the canonical ordering.
fn normalize(s: &[u16], compat: bool) -> Vec<u16> {
    let mut cps: Vec<i32> = Vec::with_capacity(s.len());
    for cp in code_points(s) {
        let s_index = cp - HANGUL_S_BASE;
        if (0..HANGUL_S_COUNT).contains(&s_index) {
            cps.push(HANGUL_L_BASE + s_index / HANGUL_N_COUNT);
            cps.push(HANGUL_V_BASE + (s_index % HANGUL_N_COUNT) / HANGUL_T_COUNT);
            let t = s_index % HANGUL_T_COUNT;
            if t != 0 {
                cps.push(HANGUL_T_BASE + t);
            }
            continue;
        }
        let d = if compat {
            compat_decomposition(cp)
        } else {
            decomposition(cp)
        };
        match d {
            Some(d) => cps.extend(code_points(d)),
            None => cps.push(cp),
        }
    }
    // Canonical ordering: a stable sort of each run of non-starters by
    // combining class.
    let mut i = 0;
    while i < cps.len() {
        if combining_class(cps[i]) == 0 {
            i += 1;
            continue;
        }
        let start = i;
        while i < cps.len() && combining_class(cps[i]) != 0 {
            i += 1;
        }
        cps[start..i].sort_by_key(|&c| combining_class(c));
    }
    let mut out = Vec::with_capacity(cps.len());
    for cp in cps {
        out.extend(to_chars(cp));
    }
    out
}

/// The text a collation element iterator reads for the decomposition mode
/// (`CollatorUtilities.toNormalizerMode`: `NONE`, `NFD`, `NFKD`; any other
/// value reads as `NONE`).
fn normalized_for_mode(s: &[u16], decomposition: i32) -> Vec<u16> {
    match decomposition {
        CANONICAL_DECOMPOSITION => normalize(s, false),
        FULL_DECOMPOSITION => normalize(s, true),
        _ => s.to_vec(),
    }
}

/// The precomposed characters `sun.text.ComposedCharIter` returns, with
/// their decompositions: `NormalizerImpl.getDecompose`'s walk over the code
/// points below U+2FA1E, which skips U+30FF..U+F8FF, U+115BC..U+1D15D and
/// U+1D1C1..U+2F7FF and keeps at most 2100 entries.
fn composed_chars() -> impl Iterator<Item = (i32, &'static [u16])> {
    DECOMPOSITIONS
        .iter()
        .map(|&(c, d)| (c as i32, d))
        .filter(|&(c, _)| {
            !(c >= 0x2fa1e
                || (0x30ff..0xf900).contains(&c)
                || (0x115bc..0x1d15e).contains(&c)
                || (0x1d1c1..0x2f800).contains(&c))
        })
        .take(2100)
}

/// `NormalizerImpl.needSingleQuotation(c)`.
fn need_single_quotation(c: u16) -> bool {
    (0x0009..=0x000D).contains(&c)
        || (0x0020..=0x002F).contains(&c)
        || (0x003A..=0x0040).contains(&c)
        || (0x005B..=0x0060).contains(&c)
        || (0x007B..=0x007E).contains(&c)
}

/// `NormalizerImpl.PrevArgs`.
struct PrevArgs {
    start: usize,
    current: usize,
    c1: u16,
    c2: u16,
}

/// `NormalizerImpl.getPrevCC(args)` over `src`.
fn get_prev_cc(src: &[u16], args: &mut PrevArgs) -> i32 {
    args.current -= 1;
    args.c1 = src[args.current];
    args.c2 = 0;
    // Below MIN_CCC_LCCC_CP (U+0300), or an unpaired first surrogate: 0.
    if args.c1 < 0x300 || is_high_surrogate(args.c1) {
        0
    } else if !is_low_surrogate(args.c1) {
        combining_class(i32::from(args.c1))
    } else if args.current != args.start && is_high_surrogate(src[args.current - 1]) {
        args.c2 = src[args.current - 1];
        args.current -= 1;
        combining_class(to_code_point(args.c2, args.c1))
    } else {
        args.c2 = 0;
        0
    }
}

/// `NormalizerImpl.NextCCArgs`.
struct NextCcArgs {
    next: usize,
    limit: usize,
    c1: u16,
    c2: u16,
}

/// `NormalizerImpl.getNextCC(args)` over `source`.
fn get_next_cc(source: &[u16], args: &mut NextCcArgs) -> i32 {
    args.c1 = source[args.next];
    args.next += 1;
    args.c2 = 0;
    if is_low_surrogate(args.c1) {
        0
    } else if !is_high_surrogate(args.c1) {
        combining_class(i32::from(args.c1))
    } else if args.next != args.limit && is_low_surrogate(source[args.next]) {
        args.c2 = source[args.next];
        args.next += 1;
        combining_class(to_code_point(args.c1, args.c2))
    } else {
        args.c2 = 0;
        0
    }
}

/// `NormalizerImpl.insertOrdered(source, start, current, p, c1, c2, cc)`.
fn insert_ordered(
    source: &mut [u16],
    start: usize,
    mut current: usize,
    p: usize,
    c1: u16,
    c2: u16,
    cc: i32,
) -> i32 {
    let mut trail_cc = cc;
    if start < current && cc != 0 {
        let mut args = PrevArgs {
            current,
            start,
            c1,
            c2,
        };
        let mut prev_cc = get_prev_cc(source, &mut args);
        let mut pre_back = args.current;
        if cc < prev_cc {
            trail_cc = prev_cc;
            let mut back = pre_back;
            while start < pre_back {
                prev_cc = get_prev_cc(source, &mut args);
                pre_back = args.current;
                if cc >= prev_cc {
                    break;
                }
                back = pre_back;
            }
            let mut r = p;
            loop {
                r -= 1;
                current -= 1;
                source[r] = source[current];
                if back == current {
                    break;
                }
            }
        }
    }
    source[current] = c1;
    if c2 != 0 {
        source[current + 1] = c2;
    }
    trail_cc
}

/// `NormalizerImpl.mergeOrdered(source, start, current, data, next, limit)`.
///
/// As in the JDK, `current` (an index into `source`) and `next` (an index
/// into `data`) are compared to decide adjacency, a starter that does not
/// bubble back is written into `data`, and the final combining class is read
/// from `data` from `start`, an index into `source`.
fn merge_ordered(
    source: &mut [u16],
    start: usize,
    mut current: usize,
    data: &mut [u16],
    next: usize,
    limit: usize,
) -> Result<i32> {
    let mut trail_cc = 0;
    let adjacent = current == next;
    let mut nc = NextCcArgs {
        next,
        limit,
        c1: 0,
        c2: 0,
    };
    if start != current {
        while nc.next < nc.limit {
            let cc = get_next_cc(data, &mut nc);
            if cc == 0 {
                trail_cc = 0;
                if adjacent {
                    current = nc.next;
                } else {
                    array_set(data, current, nc.c1)?;
                    current += 1;
                    if nc.c2 != 0 {
                        array_set(data, current, nc.c2)?;
                        current += 1;
                    }
                }
                break;
            } else {
                let r = current + if nc.c2 == 0 { 1 } else { 2 };
                trail_cc = insert_ordered(source, start, current, r, nc.c1, nc.c2, cc);
                current = r;
            }
        }
    }
    if nc.next == nc.limit {
        Ok(trail_cc)
    } else {
        if !adjacent {
            loop {
                source[current] = data[nc.next];
                current += 1;
                nc.next += 1;
                if nc.next == nc.limit {
                    break;
                }
            }
            nc.limit = current;
        }
        let mut args = PrevArgs {
            start,
            current: nc.limit,
            c1: 0,
            c2: 0,
        };
        Ok(get_prev_cc(data, &mut args))
    }
}

/// `NormalizerImpl.canonicalDecomposeWithSingleQuotation(string)`: the
/// canonical decomposition of the rules, each decomposition whose first
/// code unit is a syntax character quoted.
fn canonical_decompose_with_single_quotation(string: &[u16]) -> Result<Vec<u16>> {
    let src = string;
    let mut src_index = 0usize;
    let src_limit = src.len();
    let mut dest: Vec<u16> = vec![0; src.len() * 3];
    let mut dest_index = 0usize;
    let mut dest_limit = dest.len();
    let min_no_maybe: u16 = 0x00c0;
    let mut reorder_start_index = 0usize;
    let mut prev_cc = 0i32;
    let mut norm: Option<&'static [u16]> = None;
    let mut cp = 0i32;
    let mut c1: u16 = 0;
    loop {
        let prev_src = src_index;
        // As in the JDK, a code unit below U+00C0 advances by the length of
        // the last code point read, so a decomposed supplementary character
        // makes the walk skip a unit and, at the end, read past the array.
        while src_index != src_limit && {
            c1 = array_get(src, src_index)?;
            c1 < min_no_maybe
                || {
                    cp = code_point_at(string, src_index);
                    norm = decomposition(cp);
                    norm.is_none()
                }
                || (0xac00..=0xd7a3).contains(&c1)
        } {
            prev_cc = 0;
            src_index += if cp < 0x10000 { 1 } else { 2 };
        }
        if src_index != prev_src {
            let length = src_index - prev_src;
            if dest_index + length <= dest_limit {
                dest[dest_index..dest_index + length].copy_from_slice(&src[prev_src..src_index]);
            }
            dest_index += length;
            reorder_start_index = dest_index;
        }
        if src_index == src_limit {
            break;
        }
        src_index += if cp < 0x10000 { 1 } else { 2 };
        let mut c2: u16;
        let mut length: usize;
        if cp < 0x10000 {
            c2 = 0;
            length = 1;
            if is_high_surrogate(c1) || is_low_surrogate(c1) {
                norm = None;
            }
        } else {
            length = 2;
            c2 = src[src_index - 1];
        }
        let cc: i32;
        let mut trail_cc: i32;
        let mut p: Option<Vec<u16>>;
        match norm {
            None => {
                cc = combining_class(cp);
                trail_cc = cc;
                p = None;
            }
            Some(n) => {
                let pv = n.to_vec();
                length = pv.len();
                let cps = code_points(n);
                cc = combining_class(cps[0]);
                trail_cc = combining_class(cps[cps.len() - 1]);
                if length == 1 {
                    c1 = pv[0];
                    c2 = 0;
                    p = None;
                } else {
                    p = Some(pv);
                }
            }
        }
        if dest_index + length * 3 >= dest_limit {
            let mut tmp = vec![0u16; dest_limit * 2];
            tmp[..dest_index].copy_from_slice(&dest[..dest_index]);
            dest = tmp;
            dest_limit = dest.len();
        }
        let reorder_split = dest_index;
        match p.as_mut() {
            None => {
                if need_single_quotation(c1) {
                    dest[dest_index] = u16::from(b'\'');
                    dest[dest_index + 1] = c1;
                    dest[dest_index + 2] = u16::from(b'\'');
                    dest_index += 3;
                    trail_cc = 0;
                } else if cc != 0 && cc < prev_cc {
                    dest_index += length;
                    trail_cc = insert_ordered(
                        &mut dest,
                        reorder_start_index,
                        reorder_split,
                        dest_index,
                        c1,
                        c2,
                        cc,
                    );
                } else {
                    dest[dest_index] = c1;
                    dest_index += 1;
                    if c2 != 0 {
                        dest[dest_index] = c2;
                        dest_index += 1;
                    }
                }
            }
            Some(pv) => {
                let mut p_start = 0usize;
                if need_single_quotation(pv[p_start]) {
                    dest[dest_index] = u16::from(b'\'');
                    dest[dest_index + 1] = pv[p_start];
                    dest[dest_index + 2] = u16::from(b'\'');
                    dest_index += 3;
                    p_start += 1;
                    length -= 1;
                    loop {
                        dest[dest_index] = pv[p_start];
                        dest_index += 1;
                        p_start += 1;
                        length -= 1;
                        if length == 0 {
                            break;
                        }
                    }
                } else if cc != 0 && cc < prev_cc {
                    dest_index += length;
                    trail_cc = merge_ordered(
                        &mut dest,
                        reorder_start_index,
                        reorder_split,
                        pv,
                        p_start,
                        p_start + length,
                    )?;
                } else {
                    loop {
                        dest[dest_index] = pv[p_start];
                        dest_index += 1;
                        p_start += 1;
                        length -= 1;
                        if length == 0 {
                            break;
                        }
                    }
                }
            }
        }
        prev_cc = trail_cc;
        if prev_cc == 0 {
            reorder_start_index = dest_index;
        }
    }
    dest.truncate(dest_index);
    Ok(dest)
}

// -----------------------------------------------------------------------------
// PatternEntry, MergeCollation.
// -----------------------------------------------------------------------------

/// `java.text.PatternEntry`: one rule (strength, characters, extension).
#[derive(Debug, Clone)]
struct PatternEntry {
    strength: i32,
    chars: Vec<u16>,
    extension: Vec<u16>,
}

impl PartialEq for PatternEntry {
    /// `equals`: the characters only.
    fn eq(&self, other: &PatternEntry) -> bool {
        self.chars == other.chars
    }
}

/// `PatternEntry.isSpecialChar(ch)`.
fn is_special_char(ch: u16) -> bool {
    ch == 0x0020
        || (0x0022..=0x002F).contains(&ch)
        || (0x003A..=0x003F).contains(&ch)
        || (0x005B..=0x0060).contains(&ch)
        || (0x007B..=0x007E).contains(&ch)
}

/// `PatternEntry.appendQuoted(chars, toAddTo)`.
fn append_quoted(chars: &[u16], to: &mut Vec<u16>) {
    let mut in_quote = false;
    let ch = chars[0];
    if is_space_char(ch)
        || is_special_char(ch)
        || matches!(
            ch,
            0x0010 | 0x000C | 0x000D | 0x0009 | 0x000A | 0x0040 | 0x0027
        )
    {
        in_quote = true;
        to.push(u16::from(b'\''));
    }
    to.extend_from_slice(chars);
    if in_quote {
        to.push(u16::from(b'\''));
    }
}

impl PatternEntry {
    /// `addToBuilder(toAddTo, showExtension, showWhiteSpace, lastEntry)`.
    fn add_to_builder(
        &self,
        to: &mut Vec<u16>,
        show_extension: bool,
        show_white_space: bool,
        last_entry: Option<&PatternEntry>,
    ) {
        if show_white_space && !to.is_empty() {
            if self.strength == PRIMARY || last_entry.is_some() {
                to.push(u16::from(b'\n'));
            } else {
                to.push(u16::from(b' '));
            }
        }
        if let Some(last) = last_entry {
            to.push(u16::from(b'&'));
            if show_white_space {
                to.push(u16::from(b' '));
            }
            append_quoted(&last.chars, to);
            append_quoted(&self.extension, to);
            if show_white_space {
                to.push(u16::from(b' '));
            }
        }
        let c = match self.strength {
            IDENTICAL => b'=',
            TERTIARY => b',',
            SECONDARY => b';',
            PRIMARY => b'<',
            RESET => b'&',
            UNSET => b'?',
            // Java's `default -> throw new IllegalStateException("Unexpected
            // value: " + strength)`: the parser gives an entry no other strength.
            s => unreachable!("INVARIANT: an entry has one of the six strengths, not {s}"),
        };
        to.push(u16::from(c));
        if show_white_space {
            to.push(u16::from(b' '));
        }
        append_quoted(&self.chars, to);
        if show_extension && !self.extension.is_empty() {
            to.push(u16::from(b'/'));
            append_quoted(&self.extension, to);
        }
    }

    /// `toString()`.
    fn to_java_string(&self) -> String {
        let mut b = Vec::new();
        self.add_to_builder(&mut b, true, false, None);
        to_java_string(&b)
    }
}

/// `PatternEntry.Parser`.
struct Parser<'a> {
    pattern: &'a [u16],
    i: usize,
}

impl Parser<'_> {
    fn substring_from(&self, i: usize) -> String {
        let end = (i + 10).min(self.pattern.len());
        to_java_string(&self.pattern[i.min(end)..end])
    }

    /// `pattern.charAt(i)`.
    fn char_at(&self, i: usize) -> Result<u16> {
        match self.pattern.get(i) {
            Some(&c) => Ok(c),
            None => Err(LuceneError::unchecked(
                "java.lang.StringIndexOutOfBoundsException",
                format!("Index {i} out of bounds for length {}", self.pattern.len()),
            )),
        }
    }

    /// `next()`.
    fn next(&mut self) -> Result<Option<PatternEntry>> {
        let mut new_strength = UNSET;
        let mut new_chars: Vec<u16> = Vec::new();
        let mut new_extension: Vec<u16> = Vec::new();
        let mut in_chars = true;
        let mut in_quote = false;
        while self.i < self.pattern.len() {
            let mut ch = self.pattern[self.i];
            if in_quote {
                if ch == u16::from(b'\'') {
                    in_quote = false;
                } else if new_chars.is_empty() || in_chars {
                    new_chars.push(ch);
                } else {
                    new_extension.push(ch);
                }
            } else {
                let strength = match ch {
                    0x3D => Some(IDENTICAL), // '='
                    0x2C => Some(TERTIARY),  // ','
                    0x3B => Some(SECONDARY), // ';'
                    0x3C => Some(PRIMARY),   // '<'
                    0x26 => Some(RESET),     // '&'
                    _ => None,
                };
                if let Some(s) = strength {
                    if new_strength != UNSET {
                        break;
                    }
                    new_strength = s;
                } else {
                    match ch {
                        0x09 | 0x0A | 0x0C | 0x0D | 0x20 => {}
                        0x2F => in_chars = false, // '/'
                        0x27 => {
                            // '\''
                            in_quote = true;
                            self.i += 1;
                            ch = self.char_at(self.i)?;
                            if new_chars.is_empty() || in_chars {
                                new_chars.push(ch);
                            } else {
                                new_extension.push(ch);
                            }
                        }
                        _ => {
                            if new_strength == UNSET {
                                return Err(ParseException::error(
                                    format!(
                                        "missing char (=,;<&) : {}",
                                        self.substring_from(self.i)
                                    ),
                                    self.i as i32,
                                ));
                            }
                            if is_special_char(ch) && !in_quote {
                                return Err(ParseException::error(
                                    format!(
                                        "Unquoted punctuation character : {}",
                                        format_radix_16(i32::from(ch))
                                    ),
                                    self.i as i32,
                                ));
                            }
                            if in_chars {
                                new_chars.push(ch);
                            } else {
                                new_extension.push(ch);
                            }
                        }
                    }
                }
            }
            self.i += 1;
        }
        if new_strength == UNSET {
            return Ok(None);
        }
        if new_chars.is_empty() {
            return Err(ParseException::error(
                format!("missing chars (=,;<&): {}", self.substring_from(self.i)),
                self.i as i32,
            ));
        }
        Ok(Some(PatternEntry {
            strength: new_strength,
            chars: new_chars,
            extension: new_extension,
        }))
    }
}

/// `Integer.toString(v, 16)`.
fn format_radix_16(v: i32) -> String {
    format!("{v:x}")
}

/// `java.text.MergeCollation`: the rules parsed and merged into one ordered
/// list of entries.
struct MergeCollation {
    patterns: Vec<PatternEntry>,
    /// The index of `saveEntry` / `lastEntry` in `patterns`, or the entry
    /// itself when it is a reset (never in `patterns`).
    save_entry: Option<PatternEntry>,
    last_entry: Option<PatternEntry>,
    status_array: Vec<u8>,
}

impl MergeCollation {
    /// `new MergeCollation(pattern)`.
    fn new(pattern: &[u16]) -> Result<MergeCollation> {
        let mut m = MergeCollation {
            patterns: Vec::new(),
            save_entry: None,
            last_entry: None,
            status_array: vec![0; 8192],
        };
        let mut parser = Parser { pattern, i: 0 };
        while let Some(entry) = parser.next()? {
            m.fix_entry(entry)?;
        }
        Ok(m)
    }

    fn last_index_of(&self, e: &PatternEntry) -> Option<usize> {
        self.patterns.iter().rposition(|x| x == e)
    }

    /// `fixEntry(newEntry)`.
    fn fix_entry(&mut self, mut new_entry: PatternEntry) -> Result<()> {
        if let Some(last) = &self.last_entry {
            if new_entry.chars == last.chars && new_entry.extension == last.extension {
                if new_entry.strength != IDENTICAL && new_entry.strength != RESET {
                    return Err(ParseException::error(
                        format!(
                            "The entries {} and {} are adjacent in the rules, but have \
                             conflicting strengths: A character can't be unequal to itself.",
                            last.to_java_string(),
                            new_entry.to_java_string()
                        ),
                        -1,
                    ));
                } else {
                    return Ok(());
                }
            }
        }
        let mut change_last_entry = true;
        if new_entry.strength != RESET {
            let mut old_index: Option<usize> = None;
            if new_entry.chars.len() == 1 {
                let c = new_entry.chars[0];
                let status_index = usize::from(c >> 3);
                let bit_clump = self.status_array[status_index];
                let set_bit = 1u8 << (c & 7);
                if bit_clump != 0 && (bit_clump & set_bit) != 0 {
                    old_index = self.last_index_of(&new_entry);
                } else {
                    self.status_array[status_index] = bit_clump | set_bit;
                }
            } else {
                old_index = self.last_index_of(&new_entry);
            }
            if let Some(old) = old_index {
                self.patterns.remove(old);
            }
            let mut excess: Vec<u16> = Vec::new();
            let last_entry = self.last_entry.clone();
            let last_index = self.find_last_entry(last_entry.as_ref(), &mut excess)?;
            if !excess.is_empty() {
                excess.extend_from_slice(&new_entry.extension);
                new_entry.extension = excess;
                if last_index != self.patterns.len() {
                    self.last_entry = self.save_entry.clone();
                    change_last_entry = false;
                }
            }
            if last_index == self.patterns.len() {
                self.patterns.push(new_entry.clone());
                self.save_entry = Some(new_entry.clone());
            } else {
                self.patterns.insert(last_index, new_entry.clone());
            }
        }
        if change_last_entry {
            self.last_entry = Some(new_entry);
        }
        Ok(())
    }

    /// `findLastEntry(entry, excessChars)`.
    fn find_last_entry(
        &self,
        entry: Option<&PatternEntry>,
        excess_chars: &mut Vec<u16>,
    ) -> Result<usize> {
        let Some(entry) = entry else {
            return Ok(0);
        };
        if entry.strength != RESET {
            let mut old_index: Option<usize> = None;
            if entry.chars.len() == 1 {
                let c = entry.chars[0];
                if self.status_array[usize::from(c >> 3)] & (1u8 << (c & 7)) != 0 {
                    old_index = self.last_index_of(entry);
                }
            } else {
                old_index = self.last_index_of(entry);
            }
            match old_index {
                Some(i) => Ok(i + 1),
                None => Err(ParseException::error(
                    format!("couldn't find last entry: {}", entry.to_java_string()),
                    -1,
                )),
            }
        } else {
            for i in (0..self.patterns.len()).rev() {
                let e = &self.patterns[i];
                if entry.chars.starts_with(&e.chars) {
                    excess_chars.extend_from_slice(&entry.chars[e.chars.len()..]);
                    return Ok(i + 1);
                }
            }
            Err(ParseException::error(
                format!("couldn't find: {}", entry.to_java_string()),
                -1,
            ))
        }
    }
}

// -----------------------------------------------------------------------------
// RBCollationTables, RBTableBuilder.
// -----------------------------------------------------------------------------

/// `java.text.EntryPair`.
#[derive(Debug, Clone)]
struct EntryPair {
    entry_name: Vec<u16>,
    value: i32,
    fwd: bool,
}

/// `java.text.RBCollationTables`: the tables a set of rules builds.
#[derive(Debug)]
struct RbCollationTables {
    rules: String,
    french_sec: bool,
    se_asian_swapping: bool,
    /// `mapping` (`UCompactIntArray` with default `UNMAPPED`), keyed by
    /// [`compact_index`].
    mapping: HashMap<i32, i32>,
    contract_table: Vec<Vec<EntryPair>>,
    expand_table: Vec<Vec<i32>>,
    /// `contractFlags`: the code points used in a contracting sequence.
    contract_flags: HashSet<i32>,
    max_sec_order: i16,
    max_ter_order: i16,
}

impl RbCollationTables {
    /// `new RBCollationTables(rules, decmp)`: `RBTableBuilder.build`.
    fn new(rules: &str) -> Result<RbCollationTables> {
        let mut b = RbTableBuilder {
            tables: RbCollationTables {
                rules: rules.to_string(),
                french_sec: false,
                se_asian_swapping: false,
                mapping: HashMap::new(),
                contract_table: Vec::new(),
                expand_table: Vec::new(),
                contract_flags: HashSet::new(),
                max_sec_order: 0,
                max_ter_order: 0,
            },
            is_over_ignore: false,
        };
        b.build(rules)?;
        Ok(b.tables)
    }

    /// `getUnicodeOrder(ch)`: `mapping.elementAt(ch)`.
    fn get_unicode_order(&self, ch: i32) -> i32 {
        self.mapping
            .get(&compact_index(ch))
            .copied()
            .unwrap_or(UNMAPPED)
    }

    /// `mapping.setElementAt(ch, value)`.
    fn set_unicode_order(&mut self, ch: i32, value: i32) {
        self.mapping.insert(compact_index(ch), value);
    }

    /// `getContractValuesImpl(index)`.
    fn contract_values_impl(&self, index: i32) -> Option<&Vec<EntryPair>> {
        usize::try_from(index).ok().map(|i| &self.contract_table[i])
    }

    /// `getContractValues(ch)`.
    fn get_contract_values(&self, ch: i32) -> Option<&Vec<EntryPair>> {
        self.contract_values_impl(self.get_unicode_order(ch).wrapping_sub(CONTRACTCHARINDEX))
    }

    /// `getExpandValueList(idx)`.
    fn get_expand_value_list(&self, idx: i32) -> &[i32] {
        &self.expand_table[(idx - EXPANDCHARINDEX) as usize]
    }
}

/// The slot `sun.text.UCompactIntArray` stores code point `ch` in: its
/// `PLANEMASK` (`0x30000`) keeps two plane bits, so a code point and every
/// code point `0x40000` away from it share one slot (U+40041 reads the
/// order of `A`), as on JDK 25.
fn compact_index(ch: i32) -> i32 {
    ch & 0x3FFFF
}

/// `RBCollationTables.getEntry(list, name, fwd)`.
fn get_entry(list: &[EntryPair], name: &[u16], fwd: bool) -> Option<usize> {
    list.iter()
        .position(|p| p.fwd == fwd && p.entry_name == name)
}

/// `RBCollationTables.reverse(result, from, to)`.
fn reverse_range(v: &mut [u16], from: usize, to: usize) {
    v[from..to].reverse();
}

/// `java.text.RBTableBuilder`.
struct RbTableBuilder {
    tables: RbCollationTables,
    is_over_ignore: bool,
}

impl RbTableBuilder {
    /// `build(pattern, decmp)`.
    fn build(&mut self, pattern: &str) -> Result<()> {
        if pattern.is_empty() {
            return Err(ParseException::error("Build rules empty.", 0));
        }
        let units: Vec<u16> = pattern.encode_utf16().collect();
        let units = canonical_decompose_with_single_quotation(&units)?;
        let m_pattern = MergeCollation::new(&units)?;
        let mut order = 0i32;
        for entry in &m_pattern.patterns {
            let mut group_chars: &[u16] = &entry.chars;
            if group_chars.len() > 1 {
                match group_chars[group_chars.len() - 1] {
                    0x40 => {
                        // '@'
                        self.tables.french_sec = true;
                        group_chars = &group_chars[..group_chars.len() - 1];
                    }
                    0x21 => {
                        // '!'
                        self.tables.se_asian_swapping = true;
                        group_chars = &group_chars[..group_chars.len() - 1];
                    }
                    _ => {}
                }
            }
            order = self.increment(entry.strength, order);
            let exp_chars = &entry.extension;
            if !exp_chars.is_empty() {
                self.add_expand_order(group_chars, exp_chars, order);
            } else if group_chars.len() > 1 {
                let ch = group_chars[0];
                if is_high_surrogate(ch) && group_chars.len() == 2 {
                    self.add_order(to_code_point(ch, group_chars[1]), order);
                } else {
                    self.add_contract_order(group_chars, order, true);
                }
            } else {
                self.add_order(i32::from(group_chars[0]), order);
            }
        }
        self.add_composed_chars();
        self.commit();
        Ok(())
    }

    /// `addComposedChars()`.
    fn add_composed_chars(&mut self) {
        for (c, s) in composed_chars() {
            if self.get_char_order(c) == UNMAPPED {
                if s.len() == 1 {
                    let order = self.get_char_order(i32::from(s[0]));
                    if order != UNMAPPED {
                        self.add_order(c, order);
                    }
                    continue;
                } else if s.len() == 2 && is_high_surrogate(s[0]) {
                    let order = self.get_char_order(code_point_at(s, 0));
                    if order != UNMAPPED {
                        self.add_order(c, order);
                    }
                    continue;
                }
                let contract_order = self.get_contract_order(s);
                if contract_order != UNMAPPED {
                    self.add_order(c, contract_order);
                } else {
                    let all_there = s
                        .iter()
                        .all(|&u| self.get_char_order(i32::from(u)) != UNMAPPED);
                    if all_there {
                        let table_index = self.add_expansion(UNMAPPED, s);
                        self.add_order(c, table_index);
                    }
                }
            }
        }
    }

    /// `commit()`: resolves the expansion entries that named a character
    /// not yet mapped when they were added.
    fn commit(&mut self) {
        for i in 0..self.tables.expand_table.len() {
            for j in 0..self.tables.expand_table[i].len() {
                let order = self.tables.expand_table[i][j];
                if order < EXPANDCHARINDEX && order > CHARINDEX {
                    let ch = order - CHARINDEX;
                    let real_value = self.get_char_order(ch);
                    self.tables.expand_table[i][j] = if real_value == UNMAPPED {
                        IGNORABLEMASK & ch
                    } else {
                        real_value
                    };
                }
            }
        }
    }

    /// `increment(aStrength, lastValue)`.
    fn increment(&mut self, strength: i32, mut last_value: i32) -> i32 {
        match strength {
            PRIMARY => {
                last_value = last_value.wrapping_add(PRIMARYORDERINCREMENT);
                last_value &= PRIMARYORDERMASK;
                self.is_over_ignore = true;
            }
            SECONDARY => {
                last_value = last_value.wrapping_add(SECONDARYORDERINCREMENT);
                last_value &= SECONDARYDIFFERENCEONLY;
                if !self.is_over_ignore {
                    self.tables.max_sec_order = self.tables.max_sec_order.wrapping_add(1);
                }
            }
            TERTIARY => {
                last_value = last_value.wrapping_add(TERTIARYORDERINCREMENT);
                if !self.is_over_ignore {
                    self.tables.max_ter_order = self.tables.max_ter_order.wrapping_add(1);
                }
            }
            _ => {}
        }
        last_value
    }

    /// `addOrder(ch, anOrder)`.
    fn add_order(&mut self, ch: i32, an_order: i32) {
        let order = self.tables.get_unicode_order(ch);
        if order >= CONTRACTCHARINDEX {
            let key = to_chars(ch);
            self.add_contract_order(&key, an_order, true);
        } else {
            self.tables.set_unicode_order(ch, an_order);
        }
    }

    /// `addContractOrder(groupChars, anOrder, fwd)`.
    fn add_contract_order(&mut self, group_chars: &[u16], an_order: i32, fwd: bool) {
        let ch = code_point_at(group_chars, 0);
        let entry = self.tables.get_unicode_order(ch);
        let table_index = match usize::try_from(entry.wrapping_sub(CONTRACTCHARINDEX)) {
            Ok(i) => i,
            Err(_) => {
                let index = self.tables.contract_table.len();
                let head = group_chars[..char_count(ch)].to_vec();
                self.tables.contract_table.push(vec![EntryPair {
                    entry_name: head,
                    value: entry,
                    fwd: true,
                }]);
                self.tables
                    .set_unicode_order(ch, CONTRACTCHARINDEX + index as i32);
                index
            }
        };
        let entry_table = &mut self.tables.contract_table[table_index];
        match get_entry(entry_table, group_chars, fwd) {
            Some(i) => entry_table[i].value = an_order,
            None => {
                let last_len = entry_table.last().map_or(0, |p| p.entry_name.len());
                let pair = EntryPair {
                    entry_name: group_chars.to_vec(),
                    value: an_order,
                    fwd,
                };
                if group_chars.len() > last_len {
                    entry_table.push(pair);
                } else {
                    let at = entry_table.len() - 1;
                    entry_table.insert(at, pair);
                }
            }
        }
        if fwd && group_chars.len() > 1 {
            self.add_contract_flags(group_chars);
            let reversed = reverse_string(group_chars);
            self.add_contract_order(&reversed, an_order, false);
        }
    }

    /// `getContractOrder(groupChars)`.
    fn get_contract_order(&self, group_chars: &[u16]) -> i32 {
        let ch = code_point_at(group_chars, 0);
        match self.tables.get_contract_values(ch) {
            Some(list) => match get_entry(list, group_chars, true) {
                Some(i) => list[i].value,
                None => UNMAPPED,
            },
            None => UNMAPPED,
        }
    }

    /// `getCharOrder(ch)`.
    fn get_char_order(&self, ch: i32) -> i32 {
        let order = self.tables.get_unicode_order(ch);
        if order >= CONTRACTCHARINDEX {
            match self.tables.contract_values_impl(order - CONTRACTCHARINDEX) {
                Some(list) => list[0].value,
                None => order,
            }
        } else {
            order
        }
    }

    /// `addExpandOrder(contractChars, expandChars, anOrder)`.
    fn add_expand_order(&mut self, contract_chars: &[u16], expand_chars: &[u16], an_order: i32) {
        let table_index = self.add_expansion(an_order, expand_chars);
        if contract_chars.len() > 1 {
            let ch = contract_chars[0];
            if is_high_surrogate(ch) && contract_chars.len() == 2 {
                let ch2 = contract_chars[1];
                if is_low_surrogate(ch2) {
                    self.add_order(to_code_point(ch, ch2), table_index);
                }
            } else {
                self.add_contract_order(contract_chars, table_index, true);
            }
        } else {
            self.add_order(i32::from(contract_chars[0]), table_index);
        }
    }

    /// `addExpansion(anOrder, expandChars)`.
    fn add_expansion(&mut self, an_order: i32, expand_chars: &[u16]) -> i32 {
        let offset = usize::from(an_order != UNMAPPED);
        let mut value_list = vec![0i32; expand_chars.len() + offset];
        if offset == 1 {
            value_list[0] = an_order;
        }
        let mut j = offset;
        let mut i = 0;
        while i < expand_chars.len() {
            let ch0 = expand_chars[i];
            let ch = if is_high_surrogate(ch0) {
                i += 1;
                if i == expand_chars.len() || !is_low_surrogate(expand_chars[i]) {
                    break;
                }
                to_code_point(ch0, expand_chars[i])
            } else {
                i32::from(ch0)
            };
            let map_value = self.get_char_order(ch);
            value_list[j] = if map_value != UNMAPPED {
                map_value
            } else {
                CHARINDEX + ch
            };
            j += 1;
            i += 1;
        }
        value_list.truncate(j);
        let table_index = EXPANDCHARINDEX + self.tables.expand_table.len() as i32;
        self.tables.expand_table.push(value_list);
        table_index
    }

    /// `addContractFlags(chars)`.
    fn add_contract_flags(&mut self, chars: &[u16]) {
        let mut i = 0;
        while i < chars.len() {
            let c0 = chars[i];
            let c = if is_high_surrogate(c0) {
                i += 1;
                to_code_point(c0, chars[i])
            } else {
                i32::from(c0)
            };
            self.tables.contract_flags.insert(c);
            i += 1;
        }
    }
}

// -----------------------------------------------------------------------------
// CollationElementIterator.
// -----------------------------------------------------------------------------

/// `CollationElementIterator.primaryOrder(order)`.
pub fn primary_order(order: i32) -> i32 {
    ((order & PRIMARYORDERMASK) as u32 >> 16) as i32
}

/// `CollationElementIterator.secondaryOrder(order)`.
pub fn secondary_order(order: i32) -> i16 {
    ((order & SECONDARYORDERMASK) >> 8) as i16
}

/// `CollationElementIterator.tertiaryOrder(order)`.
pub fn tertiary_order(order: i32) -> i16 {
    (order & TERTIARYORDERMASK) as i16
}

/// `CollationElementIterator.isIgnorable(order)`.
fn is_ignorable(order: i32) -> bool {
    primary_order(order) == 0
}

fn is_thai_pre_vowel(ch: i32) -> bool {
    (0x0e40..=0x0e44).contains(&ch)
}

fn is_thai_base_consonant(ch: i32) -> bool {
    (0x0e01..=0x0e2e).contains(&ch)
}

fn is_lao_pre_vowel(ch: i32) -> bool {
    (0x0ec0..=0x0ec4).contains(&ch)
}

fn is_lao_base_consonant(ch: i32) -> bool {
    (0x0e81..=0x0eae).contains(&ch)
}

/// The normalized text a `NormalizerBase` iterates: its code points and the
/// position.
#[derive(Clone)]
struct NormText {
    cps: Vec<i32>,
    pos: usize,
}

impl NormText {
    /// `next()`.
    fn next(&mut self) -> i32 {
        match self.cps.get(self.pos) {
            Some(&c) => {
                self.pos += 1;
                c
            }
            None => DONE,
        }
    }

    /// `previous()`.
    fn previous(&mut self) -> i32 {
        if self.pos > 0 {
            self.pos -= 1;
            self.cps[self.pos]
        } else {
            DONE
        }
    }
}

/// `java.text.CollationElementIterator`, forward iteration.
struct CollationElementIterator<'a> {
    ordering: &'a RbCollationTables,
    strength: i32,
    /// `text`: `None` for an empty source.
    text: Option<NormText>,
    buffer: Option<Vec<i32>>,
    exp_index: usize,
    swap_order: i32,
}

impl<'a> CollationElementIterator<'a> {
    /// `new CollationElementIterator(sourceText, owner)`.
    fn new(source: &[u16], owner: &'a RuleBasedCollator) -> CollationElementIterator<'a> {
        let text = if source.is_empty() {
            None
        } else {
            Some(NormText {
                cps: code_points(&normalized_for_mode(source, owner.decomposition)),
                pos: 0,
            })
        };
        CollationElementIterator {
            ordering: &owner.tables,
            strength: owner.strength,
            text,
            buffer: None,
            exp_index: 0,
            swap_order: 0,
        }
    }

    /// `next()`.
    fn next(&mut self) -> i32 {
        let ordering = self.ordering;
        if self.text.is_none() {
            return NULLORDER;
        }
        if let Some(buffer) = &self.buffer {
            if self.exp_index < buffer.len() {
                let v = buffer[self.exp_index];
                self.exp_index += 1;
                return self.strength_order(v);
            } else {
                self.buffer = None;
                self.exp_index = 0;
            }
        } else if self.swap_order != 0 {
            if is_supplementary(self.swap_order) {
                let chars = to_chars(self.swap_order);
                self.swap_order = i32::from(chars[1]);
                return i32::from(chars[0]) << 16;
            }
            let order = self.swap_order << 16;
            self.swap_order = 0;
            return order;
        }
        let ch = self.text_mut().next();
        if ch == DONE {
            return NULLORDER;
        }
        let mut value = ordering.get_unicode_order(ch);
        if value == UNMAPPED {
            self.swap_order = ch;
            return UNMAPPEDCHARVALUE;
        } else if value >= CONTRACTCHARINDEX {
            value = self.next_contract_char(ch);
        }
        if value >= EXPANDCHARINDEX {
            let buffer = ordering.get_expand_value_list(value).to_vec();
            value = buffer[0];
            self.buffer = Some(buffer);
            self.exp_index = 1;
        }
        if ordering.se_asian_swapping {
            if is_thai_pre_vowel(ch) {
                let consonant = self.text_mut().next();
                if is_thai_base_consonant(consonant) {
                    let buffer = self.make_reordered_buffer(consonant, value);
                    value = buffer[0];
                    self.buffer = Some(buffer);
                    self.exp_index = 1;
                } else if consonant != DONE {
                    self.text_mut().previous();
                }
            }
            if is_lao_pre_vowel(ch) {
                let consonant = self.text_mut().next();
                if is_lao_base_consonant(consonant) {
                    let buffer = self.make_reordered_buffer(consonant, value);
                    value = buffer[0];
                    self.buffer = Some(buffer);
                    self.exp_index = 1;
                } else if consonant != DONE {
                    self.text_mut().previous();
                }
            }
        }
        self.strength_order(value)
    }

    fn text_mut(&mut self) -> &mut NormText {
        self.text
            .as_mut()
            .expect("INVARIANT: next() reads the text only when the source was not empty")
    }

    /// `strengthOrder(order)`.
    fn strength_order(&self, mut order: i32) -> i32 {
        if self.strength == PRIMARY {
            order &= PRIMARYDIFFERENCEONLY;
        } else if self.strength == SECONDARY {
            order &= SECONDARYDIFFERENCEONLY;
        }
        order
    }

    /// `makeReorderedBuffer(colFirst, lastValue, lastExpansion, true)`: the
    /// last expansion is the iterator's current buffer.
    fn make_reordered_buffer(&mut self, col_first: i32, last_value: i32) -> Vec<i32> {
        let ordering = self.ordering;
        let last_expansion = self.buffer.take();
        let mut first_value = ordering.get_unicode_order(col_first);
        if first_value >= CONTRACTCHARINDEX {
            first_value = self.next_contract_char(col_first);
        }
        let first_expansion: Option<&[i32]> = if first_value >= EXPANDCHARINDEX {
            Some(ordering.get_expand_value_list(first_value))
        } else {
            None
        };
        if first_expansion.is_none() && last_expansion.is_none() {
            vec![first_value, last_value]
        } else {
            let mut result = Vec::new();
            match first_expansion {
                Some(e) => result.extend_from_slice(e),
                None => result.push(first_value),
            }
            match last_expansion {
                Some(e) => result.extend_from_slice(&e),
                None => result.push(last_value),
            }
            result
        }
    }

    /// `nextContractChar(ch)`.
    fn next_contract_char(&mut self, ch: i32) -> i32 {
        let ordering = self.ordering;
        let list = ordering
            .get_contract_values(ch)
            .expect("INVARIANT: a character mapped to a contract index has a contract table");
        let mut order = list[0].value;
        let mut max_length = list[list.len() - 1].entry_name.len() as i32;
        let mut temp_text = self.text_mut().clone();
        temp_text.previous();
        let mut key: Vec<u16> = Vec::new();
        let mut c = temp_text.next();
        while max_length > 0 && c != DONE {
            if is_supplementary(c) {
                key.extend(to_chars(c));
                max_length -= 2;
            } else {
                key.push(c as u16);
                max_length -= 1;
            }
            c = temp_text.next();
        }
        let mut max_length = 1usize;
        for pair in list[1..].iter().rev() {
            if !pair.fwd {
                continue;
            }
            if key.starts_with(&pair.entry_name) && pair.entry_name.len() > max_length {
                max_length = pair.entry_name.len();
                order = pair.value;
            }
        }
        let mut remaining = max_length as i32;
        while remaining > 1 {
            let c = self.text_mut().next();
            remaining -= char_count(c) as i32;
        }
        order
    }
}

// -----------------------------------------------------------------------------
// CollationKey, RuleBasedCollator.
// -----------------------------------------------------------------------------

/// `java.text.RuleBasedCollationKey` (a `CollationKey`): the source string
/// and the key string `getCollationKey` built.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CollationKey {
    source: String,
    key: Vec<u16>,
}

impl CollationKey {
    /// `compareTo(target)`: the sign of the key strings' `compareTo`.
    pub fn compare_to(&self, target: &CollationKey) -> i32 {
        java_compare_to(&self.key, &target.key).signum()
    }

    /// `getSourceString()`.
    pub fn get_source_string(&self) -> &str {
        &self.source
    }

    /// `toByteArray()`: each key code unit, big-endian.
    pub fn to_byte_array(&self) -> Vec<u8> {
        self.key.iter().flat_map(|c| c.to_be_bytes()).collect()
    }

    /// `hashCode()`: the key string's.
    pub fn hash_code(&self) -> i32 {
        java_hash_code(&self.key)
    }
}

/// The root tables, built once (`Collator.getInstance` caches its
/// collators).
static ROOT_TABLES: LazyLock<Arc<RbCollationTables>> = LazyLock::new(|| {
    Arc::new(
        RbCollationTables::new(ROOT_RULES)
            .expect("INVARIANT: the JDK's root rules parse (CollatorProviderImpl)"),
    )
});

/// The tables `Collator.getInstance` built for each tailoring of
/// [`TAILORING_RULES`], by its index (Java caches the collator of each
/// locale; equal tailorings build equal tables).
static TAILORED_TABLES: LazyLock<Mutex<HashMap<usize, Arc<RbCollationTables>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// One candidate locale of `ResourceBundle.Control.getCandidateLocales`:
/// language, script, region and variant.
type Candidate = (String, String, String, String);

/// `ResourceBundle.Control.getCandidateLocales(baseName, locale)` (its
/// private `createCandidateList`): the candidate locales of the base locale
/// `language`, `script`, `region`, `variant`, most specific first, the root
/// locale last; with the special lists of Norwegian (`nb`/`no` and
/// `nn`/`no_NO_NY`) and the script Chinese regions imply.
fn candidate_locales(language: &str, script: &str, region: &str, variant: &str) -> Vec<Candidate> {
    let mut variant = variant.to_string();
    let mut script = script.to_string();
    // Special handling for Norwegian
    let mut is_norwegian_bokmal = false;
    let mut is_norwegian_nynorsk = false;
    if language == "no" {
        if region == "NO" && variant == "NY" {
            variant = String::new();
            is_norwegian_nynorsk = true;
        } else {
            is_norwegian_bokmal = true;
        }
    }
    if language == "nb" || is_norwegian_bokmal {
        let tmp_list = default_candidate_list("nb", &script, region, &variant);
        // Insert a locale replacing "nb" with "no" for every list entry with precedence
        let mut bokmal_list = Vec::new();
        for l_nb in tmp_list {
            let is_root = l_nb.0.is_empty();
            let l_no = (
                if is_root {
                    String::new()
                } else {
                    "no".to_string()
                },
                l_nb.1.clone(),
                l_nb.2.clone(),
                l_nb.3.clone(),
            );
            if is_norwegian_bokmal {
                bokmal_list.push(l_no.clone());
            } else {
                bokmal_list.push(l_nb.clone());
            }
            if is_root {
                break;
            }
            bokmal_list.push(if is_norwegian_bokmal { l_nb } else { l_no });
        }
        return bokmal_list;
    } else if language == "nn" || is_norwegian_nynorsk {
        // Insert no_NO_NY, no_NO, no after nn
        let mut nynorsk_list = default_candidate_list("nn", &script, region, &variant);
        let idx = nynorsk_list.len() - 1;
        for (offset, (l, r, v)) in [("no", "NO", "NY"), ("no", "NO", ""), ("no", "", "")]
            .into_iter()
            .enumerate()
        {
            nynorsk_list.insert(
                idx + offset,
                (l.to_string(), String::new(), r.to_string(), v.to_string()),
            );
        }
        return nynorsk_list;
    }
    // Special handling for Chinese
    else if language == "zh" && script.is_empty() && !region.is_empty() {
        // Supply script for users who want to use zh_Hans/zh_Hant
        // as bundle names (recommended for Java7+)
        match region {
            "TW" | "HK" | "MO" => script = "Hant".to_string(),
            "CN" | "SG" => script = "Hans".to_string(),
            _ => {}
        }
    }

    default_candidate_list(language, &script, region, &variant)
}

/// The private `ResourceBundle.Control.getDefaultList(language, script,
/// region, variant)`.
fn default_candidate_list(
    language: &str,
    script: &str,
    region: &str,
    variant: &str,
) -> Vec<Candidate> {
    let candidate = |l: &str, s: &str, r: &str, v: &str| {
        (l.to_string(), s.to_string(), r.to_string(), v.to_string())
    };
    let mut variants: Option<Vec<String>> = None;

    if !variant.is_empty() {
        // `variant.substring(0, idx)`, then the prefix before each '_' from
        // the right.
        let mut list = vec![variant.to_string()];
        let mut end = variant.len();
        while let Some(idx) = variant[..end].rfind('_') {
            list.push(variant[..idx].to_string());
            end = idx;
        }
        variants = Some(list);
    }

    let mut list = Vec::new();

    if let Some(variants) = &variants {
        for v in variants {
            list.push(candidate(language, script, region, v));
        }
    }
    if !region.is_empty() {
        list.push(candidate(language, script, region, ""));
    }
    let mut region = region.to_string();
    if !script.is_empty() {
        list.push(candidate(language, script, "", ""));
        // Special handling for Chinese
        if language == "zh" && region.is_empty() {
            // Supply region(country) for users who still package Chinese
            // bundles using old convention.
            match script {
                "Hans" => region = "CN".to_string(),
                "Hant" => region = "TW".to_string(),
                _ => {}
            }
        }

        // With script, after truncating variant, region and script,
        // start over without script.
        if let Some(variants) = &variants {
            for v in variants {
                list.push(candidate(language, "", &region, v));
            }
        }
        if !region.is_empty() {
            list.push(candidate(language, "", &region, ""));
        }
    }
    if !language.is_empty() {
        list.push(candidate(language, "", "", ""));
    }
    // Add root locale at the end
    list.push(candidate("", "", "", ""));

    list
}

/// `LocaleResources.getCollationData()` of the JRE locale provider for
/// `locale`: the tailoring `CollatorProviderImpl` appends to the default
/// rules, as the `CollationData` resource bundles resolve it.
///
/// The key is `Rule`, or `Rule.<type>` for a `co` Unicode extension type
/// other than empty and `standard` (ignoring case). The bundle is that of the
/// first candidate locale (`ResourceBundle.Control.getCandidateLocales`)
/// whose language tag has `CollationData` (the set `LocaleData` weeds the
/// candidates with), and the string is its value for the key, through its
/// parent chain, or `""` when it has none. Returns the index of the string in
/// [`TAILORING_RULES`], `None` for `""` read from no table entry.
fn collation_data(locale: &Locale) -> Option<usize> {
    let mut key = String::from("Rule");
    if let Some(type_) = locale.get_unicode_locale_type("co") {
        if !type_.is_empty() && !type_.eq_ignore_ascii_case("standard") {
            key.push('.');
            key.push_str(&type_);
        }
    }
    let candidates = candidate_locales(
        locale.get_language(),
        locale.get_script(),
        locale.get_country(),
        locale.get_variant(),
    );
    for (language, script, region, variant) in candidates {
        // The tags with `CollationData` have no variant; a candidate with one
        // matches none of them.
        if !variant.is_empty() {
            continue;
        }
        let mut tag = language;
        for subtag in [script, region] {
            if !subtag.is_empty() {
                tag.push('-');
                tag.push_str(&subtag);
            }
        }
        if TAILORINGS.iter().any(|&(t, _, _)| t == tag) {
            return TAILORINGS
                .iter()
                .find(|&&(t, k, _)| t == tag && k == key)
                .map(|&(_, _, index)| usize::from(index));
        }
    }
    unreachable!("INVARIANT: the root locale, the last candidate, has CollationData")
}

/// `java.text.RuleBasedCollator` (with `java.text.Collator`'s members).
#[derive(Debug, Clone)]
pub struct RuleBasedCollator {
    tables: Arc<RbCollationTables>,
    strength: i32,
    decomposition: i32,
}

impl PartialEq for RuleBasedCollator {
    /// `equals(obj)`: the same strength, decomposition and rules.
    fn eq(&self, other: &RuleBasedCollator) -> bool {
        self.strength == other.strength
            && self.decomposition == other.decomposition
            && self.tables.rules == other.tables.rules
    }
}

impl RuleBasedCollator {
    /// `new RuleBasedCollator(rules)`: `TERTIARY`, `CANONICAL_DECOMPOSITION`.
    ///
    /// # Errors
    ///
    /// The `ParseException` the rules raise (`RBTableBuilder.build`,
    /// `MergeCollation`, `PatternEntry.Parser`).
    pub fn new(rules: &str) -> Result<RuleBasedCollator> {
        Ok(RuleBasedCollator {
            tables: Arc::new(RbCollationTables::new(rules)?),
            strength: TERTIARY,
            decomposition: CANONICAL_DECOMPOSITION,
        })
    }

    /// `Collator.getInstance(Locale.ROOT)`: the JDK's default rules,
    /// `TERTIARY`, and `NO_DECOMPOSITION` (`CollatorProviderImpl`).
    pub fn root() -> RuleBasedCollator {
        RuleBasedCollator {
            tables: Arc::clone(&ROOT_TABLES),
            strength: TERTIARY,
            decomposition: NO_DECOMPOSITION,
        }
    }

    /// `Collator.getInstance(desiredLocale)`: the collator of the locale, as
    /// JDK 25's locale providers build it (`CollatorProviderImpl`): a
    /// `RuleBasedCollator` of `CollationRules.DEFAULTRULES` and the locale's
    /// tailoring (the default rules alone when they do not parse), at
    /// `TERTIARY` strength and `NO_DECOMPOSITION`; then the `ks` Unicode
    /// extension (`level1`, `level2`, `level3`, `identic`, ignoring case)
    /// overrides the strength, and `kk` (`true`, `false`) the decomposition.
    /// Java returns a clone of a cached collator; this is a new collator
    /// sharing cached tables.
    pub fn get_instance(desired_locale: &Locale) -> RuleBasedCollator {
        let tables = match collation_data(desired_locale) {
            Some(index) if !TAILORING_RULES[index].is_empty() => {
                let mut cache = TAILORED_TABLES
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                Arc::clone(cache.entry(index).or_insert_with(|| {
                    let rules = format!("{ROOT_RULES}{}", TAILORING_RULES[index]);
                    match RbCollationTables::new(&rules) {
                        Ok(tables) => Arc::new(tables),
                        // predefined tables should contain correct grammar
                        // (Java catches the `ParseException` only)
                        Err(e) if is_parse_exception(&e) => Arc::clone(&ROOT_TABLES),
                        Err(e) => panic!(
                            "INVARIANT: the JDK's tailorings build (tools/gen/CollatorData.java \
                             checks every one): {e}"
                        ),
                    }
                }))
            }
            // `DEFAULTRULES + ""`: the root tables.
            _ => Arc::clone(&ROOT_TABLES),
        };
        let mut result = RuleBasedCollator {
            tables,
            strength: TERTIARY,
            // Now that RuleBasedCollator adds expansions for pre-composed characters
            // into their decomposed equivalents, the default collators don't need
            // to have decomposition turned on.  Laura, 5/5/98, bug 4114077
            decomposition: NO_DECOMPOSITION,
        };

        // Override strength and decomposition with `desiredLocale`, if any
        if let Some(strength) = desired_locale.get_unicode_locale_type("ks") {
            match strength.to_ascii_lowercase().as_str() {
                "level1" => result.strength = PRIMARY,
                "level2" => result.strength = SECONDARY,
                "level3" => result.strength = TERTIARY,
                "identic" => result.strength = IDENTICAL,
                _ => {}
            }
        }
        if let Some(norm) = desired_locale.get_unicode_locale_type("kk") {
            match norm.to_ascii_lowercase().as_str() {
                "true" => result.decomposition = CANONICAL_DECOMPOSITION,
                "false" => result.decomposition = NO_DECOMPOSITION,
                _ => {}
            }
        }
        result
    }

    /// `getRules()`.
    pub fn get_rules(&self) -> &str {
        &self.tables.rules
    }

    /// `getStrength()`.
    pub fn get_strength(&self) -> i32 {
        self.strength
    }

    /// `setStrength(newStrength)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` (`Incorrect comparison level.`) for a
    /// value other than the four strengths.
    pub fn set_strength(&mut self, new_strength: i32) -> Result<()> {
        if !(PRIMARY..=IDENTICAL).contains(&new_strength) {
            return Err(LuceneError::illegal_argument("Incorrect comparison level."));
        }
        self.strength = new_strength;
        Ok(())
    }

    /// `getDecomposition()`.
    pub fn get_decomposition(&self) -> i32 {
        self.decomposition
    }

    /// `setDecomposition(decompositionMode)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` (`Wrong decomposition mode.`) for a value
    /// other than the three modes.
    pub fn set_decomposition(&mut self, decomposition_mode: i32) -> Result<()> {
        if !(NO_DECOMPOSITION..=FULL_DECOMPOSITION).contains(&decomposition_mode) {
            return Err(LuceneError::illegal_argument("Wrong decomposition mode."));
        }
        self.decomposition = decomposition_mode;
        Ok(())
    }

    /// `equals(source, target)`: `compare(source, target) == EQUAL`.
    pub fn equals(&self, source: &str, target: &str) -> bool {
        self.compare(source, target) == EQUAL
    }

    /// `hashCode()`: the rules'.
    pub fn hash_code(&self) -> i32 {
        java_hash_code(&self.tables.rules.encode_utf16().collect::<Vec<_>>())
    }

    /// `compare(source, target)`: negative, zero or positive as `source`
    /// sorts before, with or after `target`.
    pub fn compare(&self, source: &str, target: &str) -> i32 {
        if source == target {
            return EQUAL;
        }
        let source: Vec<u16> = source.encode_utf16().collect();
        let target: Vec<u16> = target.encode_utf16().collect();
        let tables = &*self.tables;
        let mut result = EQUAL;
        let mut source_cursor = CollationElementIterator::new(&source, self);
        let mut target_cursor = CollationElementIterator::new(&target, self);
        let mut s_order = 0;
        let mut t_order = 0;
        let initial_check_sec_ter = self.strength >= SECONDARY;
        let mut check_sec_ter = initial_check_sec_ter;
        let mut check_tertiary = self.strength >= TERTIARY;
        let mut gets = true;
        let mut gett = true;
        loop {
            if gets {
                s_order = source_cursor.next();
            } else {
                gets = true;
            }
            if gett {
                t_order = target_cursor.next();
            } else {
                gett = true;
            }
            if s_order == NULLORDER || t_order == NULLORDER {
                break;
            }
            let p_s_order = primary_order(s_order);
            let p_t_order = primary_order(t_order);
            if s_order == t_order {
                if tables.french_sec && p_s_order != 0 && !check_sec_ter {
                    check_sec_ter = initial_check_sec_ter;
                    check_tertiary = false;
                }
                continue;
            }
            if p_s_order != p_t_order {
                if s_order == 0 {
                    gett = false;
                    continue;
                }
                if t_order == 0 {
                    gets = false;
                    continue;
                }
                if p_s_order == 0 {
                    if check_sec_ter {
                        result = GREATER;
                        check_sec_ter = false;
                    }
                    gett = false;
                } else if p_t_order == 0 {
                    if check_sec_ter {
                        result = LESS;
                        check_sec_ter = false;
                    }
                    gets = false;
                } else if p_s_order < p_t_order {
                    return LESS;
                } else {
                    return GREATER;
                }
            } else if check_sec_ter {
                let sec_s = secondary_order(s_order);
                let sec_t = secondary_order(t_order);
                if sec_s != sec_t {
                    result = if sec_s < sec_t { LESS } else { GREATER };
                    check_sec_ter = false;
                } else if check_tertiary {
                    let ter_s = tertiary_order(s_order);
                    let ter_t = tertiary_order(t_order);
                    if ter_s != ter_t {
                        result = if ter_s < ter_t { LESS } else { GREATER };
                        check_tertiary = false;
                    }
                }
            }
        }
        if s_order != NULLORDER {
            loop {
                if primary_order(s_order) != 0 {
                    return GREATER;
                } else if secondary_order(s_order) != 0 && check_sec_ter {
                    result = GREATER;
                    check_sec_ter = false;
                }
                s_order = source_cursor.next();
                if s_order == NULLORDER {
                    break;
                }
            }
        } else if t_order != NULLORDER {
            loop {
                if primary_order(t_order) != 0 {
                    return LESS;
                } else if secondary_order(t_order) != 0 && check_sec_ter {
                    result = LESS;
                    check_sec_ter = false;
                }
                t_order = target_cursor.next();
                if t_order == NULLORDER {
                    break;
                }
            }
        }
        if result == 0 && self.strength == IDENTICAL {
            return match self.decomposition {
                CANONICAL_DECOMPOSITION => {
                    java_compare_to(&normalize(&source, false), &normalize(&target, false))
                }
                FULL_DECOMPOSITION => {
                    java_compare_to(&normalize(&source, true), &normalize(&target, true))
                }
                _ => java_compare_to(&source, &target),
            };
        }
        result
    }

    /// `getCollationKey(source)`.
    pub fn get_collation_key(&self, source: &str) -> CollationKey {
        let units: Vec<u16> = source.encode_utf16().collect();
        let tables = &*self.tables;
        let mut prim: Vec<u16> = Vec::new();
        let mut sec: Vec<u16> = Vec::new();
        let mut ter: Vec<u16> = Vec::new();
        let compare_sec = self.strength >= SECONDARY;
        let compare_ter = self.strength >= TERTIARY;
        let mut pre_sec_ignore = 0usize;
        let mut cursor = CollationElementIterator::new(&units, self);
        loop {
            let order = cursor.next();
            if order == NULLORDER {
                break;
            }
            let sec_order = i32::from(secondary_order(order));
            let ter_order = i32::from(tertiary_order(order));
            if !is_ignorable(order) {
                prim.push((primary_order(order) + COLLATIONKEYOFFSET) as u16);
                if compare_sec {
                    if tables.french_sec && pre_sec_ignore < sec.len() {
                        let len = sec.len();
                        reverse_range(&mut sec, pre_sec_ignore, len);
                    }
                    sec.push((sec_order + COLLATIONKEYOFFSET) as u16);
                    pre_sec_ignore = sec.len();
                }
                if compare_ter {
                    ter.push((ter_order + COLLATIONKEYOFFSET) as u16);
                }
            } else {
                if compare_sec && sec_order != 0 {
                    sec.push(
                        (sec_order + i32::from(tables.max_sec_order) + COLLATIONKEYOFFSET) as u16,
                    );
                }
                if compare_ter && ter_order != 0 {
                    ter.push(
                        (ter_order + i32::from(tables.max_ter_order) + COLLATIONKEYOFFSET) as u16,
                    );
                }
            }
        }
        if tables.french_sec {
            if pre_sec_ignore < sec.len() {
                let len = sec.len();
                reverse_range(&mut sec, pre_sec_ignore, len);
            }
            let len = sec.len();
            reverse_range(&mut sec, 0, len);
        }
        prim.push(0);
        sec.push(0);
        sec.extend_from_slice(&ter);
        prim.extend_from_slice(&sec);
        if self.strength == IDENTICAL {
            prim.push(0);
            match self.decomposition {
                CANONICAL_DECOMPOSITION => prim.extend(normalize(&units, false)),
                FULL_DECOMPOSITION => prim.extend(normalize(&units, true)),
                _ => prim.extend_from_slice(&units),
            }
        }
        CollationKey {
            source: source.to_string(),
            key: prim,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn reverse_keeps_surrogate_pairs() {
        assert_eq!(reverse_string(&u("a\u{1F600}b")), u("b\u{1F600}a"));
    }

    #[test]
    fn java_compare_to_is_unit_difference_then_length() {
        assert_eq!(java_compare_to(&u("ab"), &u("ac")), -1);
        assert_eq!(java_compare_to(&u("ab"), &u("abcd")), -2);
        assert_eq!(java_compare_to(&u(""), &u("")), 0);
    }
}
