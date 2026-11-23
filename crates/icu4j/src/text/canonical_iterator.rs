// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.CanonicalIterator`.

use std::sync::{Arc, LazyLock};

use rucene::error::{LuceneError, Result};
use rucene::util::java_collections::JavaHashMap;

use crate::jdk::{char_count, code_point_at, to_rust_string, units};
use crate::lang::u_character::UCharacter;
use crate::r#impl::norm2_all_modes::{Norm2AllModes, Normalizer2WithImpl};
use crate::r#impl::normalizer2_impl::Normalizer2Impl;
use crate::text::normalizer::Normalizer;
use crate::text::normalizer2::Normalizer2;
use crate::text::unicode_set::UnicodeSet;
use crate::text::unicode_set_iterator::UnicodeSetIterator;
use crate::text::utf16::UTF16;

/// A `HashSet<String>` with Java's iteration order: the keys of a
/// [`JavaHashMap`] (Java's `HashSet` is a `HashMap` with dummy values).
/// The strings are canonical equivalents of normalized text, which never
/// holds unpaired surrogates, so they are Rust `String`s without loss.
type JavaHashSet = JavaHashMap<()>;

/// To avoid infinity loop caused by permute, we limit the depth of
/// recursive call to permute and throw exception. We know in some unit test
/// we need at least 4. Set to 8 just in case some unforseen use cases.
const PERMUTE_DEPTH_LIMIT: i32 = 8;

const SKIP_ZEROS: bool = true;

const RESULT_LIMIT: usize = 4096;

/// `SET_WITH_NULL_STRING`: constant, don't change.
static SET_WITH_NULL_STRING: LazyLock<JavaHashSet> = LazyLock::new(|| {
    let mut s = JavaHashSet::new();
    s.insert(String::new(), ());
    s
});

/// This class allows one to iterate through all the strings that are
/// canonically equivalent to a given string.
///
/// Equivalent to `com.ibm.icu.text.CanonicalIterator`. Java's
/// `HashSet<String>` collections are [`JavaHashMap`] key sets, so the
/// equivalents come out in Java's order.
#[derive(Debug)]
pub struct CanonicalIterator {
    nfd: Arc<Normalizer2WithImpl>,
    nfc_impl: Arc<Normalizer2Impl>,
    source: Vec<u16>,
    done: bool,
    pieces: Vec<Vec<Vec<u16>>>,
    current: Vec<i32>,
    buffer: Vec<u16>,
}

fn h(s: &[u16]) -> String {
    to_rust_string(s)
}

impl CanonicalIterator {
    /// Construct a CanonicalIterator object.
    ///
    /// Equivalent to `CanonicalIterator(String source)`.
    ///
    /// # Errors
    ///
    /// The exceptions of loading the NFC data and of `setSource`.
    pub fn new(source: &[u16]) -> Result<Self> {
        let all_modes = Norm2AllModes::get_nfc_instance()?;
        let nfd = all_modes.decomp.clone();
        let nfc_impl = all_modes.impl_.clone();
        nfc_impl.ensure_canon_iter_data()?;
        let mut it = Self {
            nfd,
            nfc_impl,
            source: Vec::new(),
            done: false,
            pieces: Vec::new(),
            current: Vec::new(),
            buffer: Vec::new(),
        };
        it.set_source(source)?;
        Ok(it)
    }

    /// Gets the NFD form of the current source we are iterating over.
    ///
    /// Equivalent to `getSource()`.
    pub fn get_source(&self) -> &[u16] {
        &self.source
    }

    /// Resets the iterator so that one can start again from the beginning.
    ///
    /// Equivalent to `reset()`.
    pub fn reset(&mut self) {
        self.done = false;
        for c in self.current.iter_mut() {
            *c = 0;
        }
    }

    /// Get the next canonically equivalent string. Warning: The strings
    /// are not guaranteed to be in any particular order.
    ///
    /// Equivalent to `next()`; `None` for Java's `null` when done.
    ///
    /// # Errors
    ///
    /// None; the signature leaves room for the iterator's callers.
    pub fn next(&mut self) -> Result<Option<Vec<u16>>> {
        if self.done {
            return Ok(None);
        }

        // construct return value
        self.buffer.clear(); // delete old contents
        for (i, piece) in self.pieces.iter().enumerate() {
            self.buffer.extend_from_slice(&piece[self.current[i] as usize]);
        }
        let result = self.buffer.clone();

        // find next value for next time
        let mut i = self.current.len() as i32 - 1;
        loop {
            if i < 0 {
                self.done = true;
                break;
            }
            self.current[i as usize] += 1;
            if (self.current[i as usize] as usize) < self.pieces[i as usize].len() {
                break; // got sequence
            }
            self.current[i as usize] = 0;
            i -= 1;
        }
        Ok(Some(result))
    }

    /// Set a new source for this iterator. Allows object reuse.
    ///
    /// Equivalent to `setSource(String)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` for too many permutations or a too
    /// deep permutation.
    pub fn set_source(&mut self, new_source: &[u16]) -> Result<()> {
        self.source = self.nfd.normalize_str(new_source)?;
        self.done = false;

        // catch degenerate case
        if new_source.is_empty() {
            self.pieces = vec![vec![Vec::new()]];
            self.current = vec![0];
            return Ok(());
        }

        // find the segments
        let mut segment_list: Vec<Vec<u16>> = Vec::new();
        let mut start = 0;

        // i should be the end of the first code point
        // break up the string into segments
        let source = self.source.clone();
        let mut i = UTF16::find_offset_from_code_point(&source, 1);

        while i < source.len() as i32 {
            let cp = code_point_at(&source, i);
            if self.nfc_impl.is_canon_segment_starter(cp) {
                segment_list.push(source[start as usize..i as usize].to_vec()); // add up to i
                start = i;
            }
            i += char_count(cp);
        }
        segment_list.push(source[start as usize..i as usize].to_vec()); // add last one

        // allocate the arrays, and find the strings that are CE to each segment
        self.pieces = Vec::with_capacity(segment_list.len());
        self.current = vec![0; segment_list.len()];
        for segment in &segment_list {
            let p = self.get_equivalents(segment)?;
            self.pieces.push(p);
        }
        Ok(())
    }

    /// Simple implementation of permutation.
    ///
    /// Equivalent to `permute(String, boolean, Set<String>)` (deprecated,
    /// internal).
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` when the recursion is too deep.
    pub fn permute(source: &[u16], skip_zeros: bool, output: &mut JavaHashMap<()>) -> Result<()> {
        Self::permute_depth(source, skip_zeros, output, 0)
    }

    fn permute_depth(source: &[u16], skip_zeros: bool, output: &mut JavaHashSet, depth: i32) -> Result<()> {
        // TODO: optimize
        if depth > PERMUTE_DEPTH_LIMIT {
            return Err(LuceneError::unsupported_operation(format!("Stack too deep:{depth}")));
        }
        // optimization:
        // if zero or one character, just return a set with it
        // we check for length < 2 to keep from counting code points all the time
        if source.len() <= 2 && UTF16::count_code_point(Some(source)) <= 1 {
            output.insert(h(source), ());
            return Ok(());
        }

        // otherwise iterate through the string, and recursively permute all the other characters
        let mut subpermute = JavaHashSet::new();
        let mut i = 0;
        while i < source.len() as i32 {
            let cp = UTF16::char_at(source, i);
            let cp_len = UTF16::get_char_count(cp);

            // optimization:
            // if the character is canonical combining class zero,
            // don't permute it
            if skip_zeros && i != 0 && UCharacter::get_combining_class(cp)? == 0 {
                i += cp_len;
                continue;
            }

            // see what the permutations of the characters before and after this one are
            subpermute.clear();
            let mut rest = source[..i as usize].to_vec();
            rest.extend_from_slice(&source[(i + cp_len) as usize..]);
            Self::permute_depth(&rest, skip_zeros, &mut subpermute, depth + 1)?;

            // prefix this character to all of them
            let ch_str = h(&UTF16::value_of_at(source, i));
            for s in subpermute.keys() {
                let piece = format!("{ch_str}{s}");
                output.insert(piece, ());
            }
            i += cp_len;
        }
        Ok(())
    }

    // we have a segment, in NFD. Find all the strings that are canonically equivalent to it.
    fn get_equivalents(&self, segment: &[u16]) -> Result<Vec<Vec<u16>>> {
        let mut result = JavaHashSet::new();
        let basic = self.get_equivalents2(segment)?;
        let mut permutations = JavaHashSet::new();

        // now get all the permutations
        // add only the ones that are canonically equivalent
        // TODO: optimize by not permuting any class zero.
        for item in basic.keys() {
            permutations.clear();
            Self::permute_depth(&units(item), SKIP_ZEROS, &mut permutations, 0)?;
            for possible in permutations.keys() {
                if Normalizer::compare_str(&units(possible), segment, 0)? == 0 {
                    result.insert(possible.clone(), ());
                }
            }
        }

        // convert into a String[] to clean up storage
        Ok(result.keys().map(|s| units(s)).collect())
    }

    fn get_equivalents2(&self, segment: &[u16]) -> Result<JavaHashSet> {
        let mut result = JavaHashSet::new();

        result.insert(h(segment), ());

        let mut working_buffer: Vec<u16> = Vec::new();
        let mut starts = UnicodeSet::new();

        // cycle through all the characters
        let mut i = 0;
        while i < segment.len() as i32 {
            // see if any character is at the start of some decomposition
            let cp = code_point_at(segment, i);
            if !self.nfc_impl.get_canon_start_set(cp, &mut starts)? {
                i += char_count(cp);
                continue;
            }
            // if so, see which decompositions match
            let mut iter = UnicodeSetIterator::new(&starts);
            while iter.next() {
                let cp2 = iter.codepoint;
                let Some(remainder) = self.extract(cp2, segment, i, &mut working_buffer)? else {
                    continue;
                };

                // there were some matches, so add all the possibilities to the set.
                let mut prefix = segment[..i as usize].to_vec();
                prefix.extend_from_slice(&UTF16::value_of(cp2)?);
                let prefix = h(&prefix);
                for item in remainder.keys() {
                    result.insert(format!("{prefix}{item}"), ());
                }
            }
            // ICU-22642 Guards against strings that have so many permutations
            // that they would otherwise hang the function.
            if result.len() > RESULT_LIMIT {
                return Err(LuceneError::unsupported_operation("Too many permutations"));
            }
            i += char_count(cp);
        }
        Ok(result)
    }

    fn extract(&self, comp: i32, segment: &[u16], segment_pos: i32, buf: &mut Vec<u16>) -> Result<Option<JavaHashSet>> {
        let decomp = match self.nfc_impl.get_decomposition(comp) {
            Some(d) => d,
            None => UTF16::value_of(comp)?,
        };

        // See if it matches the start of segment (at segmentPos)
        let mut ok = false;
        let mut decomp_pos = 0;
        let mut decomp_cp = UTF16::char_at(&decomp, 0);
        decomp_pos += UTF16::get_char_count(decomp_cp); // adjust position to skip first char
        buf.clear(); // initialize working buffer, shared among callees

        let mut i = segment_pos;
        while i < segment.len() as i32 {
            let cp = UTF16::char_at(segment, i);
            if cp == decomp_cp {
                // if equal, eat another cp from decomp
                if decomp_pos == decomp.len() as i32 {
                    // done, have all decomp characters!
                    buf.extend_from_slice(&segment[(i + UTF16::get_char_count(cp)) as usize..]); // add remaining segment chars
                    ok = true;
                    break;
                }
                decomp_cp = UTF16::char_at(&decomp, decomp_pos);
                decomp_pos += UTF16::get_char_count(decomp_cp);
            } else {
                // brute force approach
                UTF16::append(buf, cp)?;
            }
            i += UTF16::get_char_count(cp);
        }
        if !ok {
            return Ok(None); // we failed, characters left over
        }
        if buf.is_empty() {
            return Ok(Some(SET_WITH_NULL_STRING.clone())); // succeed, but no remainder
        }
        let remainder = buf.clone();

        // brute force approach
        // to check to make sure result is canonically equivalent
        let mut composed = UTF16::value_of(comp)?;
        composed.extend_from_slice(&remainder);
        if 0 != Normalizer::compare_str(&composed, &segment[segment_pos as usize..], 0)? {
            return Ok(None);
        }

        // get the remaining combinations
        Ok(Some(self.get_equivalents2(&remainder)?))
    }
}
