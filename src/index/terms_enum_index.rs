//! `TermsEnumIndex` ported from `org.apache.lucene.index`.
//!
//! A [`TermsEnumIndex`] pairs a [`TermsEnum`] with the ordinal of the
//! sub-reader (or sub-doc-values instance) it belongs to, and caches that
//! enum's current term. Every operation that moves the enum's position must go
//! through the wrapper so the cached term — and the 8-byte comparison prefix
//! derived from it — stay in sync with the underlying enum.
//!
//! # Why this type exists
//!
//! Merging N sorted term streams compares the heads of those streams once per
//! heap sift, which makes term comparison the inner loop of every multi-segment
//! merge (`MultiTermsEnum`, `OrdinalMap`, postings merging). Lucene makes that
//! loop cheap in two ways, both reproduced here:
//!
//! 1. the current term is cached, so comparing two heads never re-enters the
//!    codec;
//! 2. the first eight bytes of the term are folded into a single `u64` whose
//!    unsigned order agrees with the term's byte order, so the common case is
//!    settled by one integer comparison instead of a byte-slice comparison.
//!
//! # Decision: one shared type instead of per-caller copies
//!
//! Java has a single package-private `TermsEnumIndex` shared by `OrdinalMap`
//! and `MultiTermsEnum` (the latter through
//! `MultiTermsEnum.TermsEnumWithSlice extends TermsEnumIndex`). This port keeps
//! that single home rather than duplicating the wrapper per caller: the
//! prefix-8 comparison is subtle enough (see
//! [`prefix8_to_comparable_unsigned_long`]) that having exactly one
//! implementation, tested once, is worth more than the coupling it introduces.

#![deny(unsafe_code)]

use std::{
    cmp::Ordering,
    fmt::{Debug, Formatter},
};

use crate::error::Result;
use crate::index::terms::{SeekStatus, TermsEnum};
use crate::util::BytesRef;

/// Number of leading term bytes folded into the comparison prefix.
const PREFIX_BYTES: usize = 8;

/// Folds the first eight bytes of `term` into a `u64` whose **unsigned** order
/// agrees with the term's unsigned byte order.
///
/// Equivalent to `TermsEnumIndex.prefix8ToComparableUnsignedLong(BytesRef)`.
/// Terms shorter than eight bytes are zero-padded on the right, so two
/// different terms can fold to the same value (`[1, 0]` and `[1]` both give
/// `0x01000000_00000000`). The converse never happens: if two folded values
/// differ, their order is the terms' order, which is what makes the fast path
/// in [`TermsEnumIndex::compare_term_to`] sound.
///
/// Zero padding is exactly the "a proper prefix sorts first" rule, which is why
/// the equivalence holds: at the first byte position where two padded prefixes
/// differ, the one holding padding is the shorter term and the other holds a
/// non-zero byte, so both orders agree.
///
/// # Examples
///
/// ```
/// use rucene::index::terms_enum_index::prefix8_to_comparable_unsigned_long;
/// use rucene::util::BytesRef;
///
/// let short = BytesRef::new(vec![0x01]);
/// let long = BytesRef::new(vec![0x01, 0x00]);
/// // Zero padding makes these two terms fold to the same prefix.
/// assert_eq!(
///     prefix8_to_comparable_unsigned_long(&short),
///     prefix8_to_comparable_unsigned_long(&long)
/// );
/// ```
pub fn prefix8_to_comparable_unsigned_long(term: &BytesRef) -> u64 {
    let bytes = term.slice();
    match bytes.len() {
        // Nothing to fold: an empty term sorts before everything else.
        //
        // This case must be singled out. Java reaches it through a shift of 64
        // on a `long`, which the JLS defines as a shift of 0 (the distance is
        // masked to 6 bits); Rust treats a shift of 64 on a `u64` as an
        // overflow instead, so the same expression would panic in a debug
        // build.
        0 => 0,
        // The term is long enough to fill the prefix outright.
        len if len >= PREFIX_BYTES => {
            let head: [u8; PREFIX_BYTES] = bytes[..PREFIX_BYTES]
                .try_into()
                .expect("INVARIANT: the slice was just bounded to PREFIX_BYTES");
            u64::from_be_bytes(head)
        }
        // Shorter terms are accumulated big-endian, then pushed to the top of
        // the word so that the unused low bytes read as zero padding.
        len => {
            let folded = bytes
                .iter()
                .fold(0u64, |acc, &byte| (acc << 8) | u64::from(byte));
            folded << ((PREFIX_BYTES - len) * 8)
        }
    }
}

/// A [`TermsEnum`] tagged with the ordinal of the sub-reader that produced it,
/// caching the enum's current term.
///
/// Equivalent to `org.apache.lucene.index.TermsEnumIndex`.
///
/// All positioning goes through this wrapper ([`next`](Self::next),
/// [`seek_ceil`](Self::seek_ceil), [`seek_exact`](Self::seek_exact),
/// [`seek_ord`](Self::seek_ord)); moving the wrapped enum directly through
/// [`terms_enum_mut`](Self::terms_enum_mut) would leave the cached term stale.
pub struct TermsEnumIndex {
    terms_enum: Box<dyn TermsEnum>,
    sub_index: usize,
    current_term: Option<BytesRef>,
    current_term_prefix8: u64,
}

impl TermsEnumIndex {
    /// Wraps `terms_enum`, tagging it with `sub_index`.
    ///
    /// The wrapper starts unpositioned: [`term`](Self::term) returns `None`
    /// until the enum is moved.
    ///
    /// Equivalent to `TermsEnumIndex(TermsEnum, int)`.
    pub fn new(terms_enum: Box<dyn TermsEnum>, sub_index: usize) -> Self {
        Self {
            terms_enum,
            sub_index,
            current_term: None,
            current_term_prefix8: 0,
        }
    }

    /// Returns the ordinal identifying the sub-reader this enum belongs to.
    ///
    /// Equivalent to reading the `TermsEnumIndex.subIndex` field.
    pub fn sub_index(&self) -> usize {
        self.sub_index
    }

    /// Returns the wrapped enum for read-only queries such as
    /// [`TermsEnum::ord`], [`TermsEnum::doc_freq`] or
    /// [`TermsEnum::total_term_freq`].
    pub fn terms_enum(&self) -> &dyn TermsEnum {
        self.terms_enum.as_ref()
    }

    /// Returns the wrapped enum mutably, for operations that read the current
    /// position without moving it (for example [`TermsEnum::postings`]).
    ///
    /// Callers must not reposition the enum through this reference: the cached
    /// term would no longer describe the enum's position. Use the positioning
    /// methods on this wrapper instead.
    pub fn terms_enum_mut(&mut self) -> &mut dyn TermsEnum {
        self.terms_enum.as_mut()
    }

    /// Returns the current term, or `None` when the enum is exhausted or has
    /// not been positioned yet.
    ///
    /// Equivalent to `TermsEnumIndex.term()`.
    pub fn term(&self) -> Option<&BytesRef> {
        self.current_term.as_ref()
    }

    /// Returns the comparison prefix of the current term, as produced by
    /// [`prefix8_to_comparable_unsigned_long`]; zero when there is no current
    /// term.
    pub fn term_prefix8(&self) -> u64 {
        self.current_term_prefix8
    }

    /// Records `term` as the current term and refreshes the cached prefix.
    fn set_term(&mut self, term: Option<BytesRef>) {
        self.current_term_prefix8 = match &term {
            Some(term) => prefix8_to_comparable_unsigned_long(term),
            None => 0,
        };
        self.current_term = term;
    }

    /// Advances to the next term, returning it, or `None` once the enum is
    /// exhausted.
    ///
    /// Equivalent to `TermsEnumIndex.next()`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the wrapped enum.
    // Named after Lucene's `TermsEnumIndex.next()`, which this port keeps for
    // parity. It is not `Iterator::next`: it is fallible and it lends the term
    // out of the wrapper rather than yielding an owned item, so neither the
    // signature nor the borrow shape fits that trait.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<&BytesRef>> {
        let term = self.terms_enum.next()?;
        self.set_term(term);
        Ok(self.current_term.as_ref())
    }

    /// Seeks to the smallest term greater than or equal to `term`.
    ///
    /// Equivalent to `TermsEnumIndex.seekCeil(BytesRef)`. On
    /// [`SeekStatus::END`] the cached term is cleared, matching Lucene.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the wrapped enum.
    pub fn seek_ceil(&mut self, term: &BytesRef) -> Result<SeekStatus> {
        let status = self.terms_enum.seek_ceil(term)?;
        let positioned = if status == SeekStatus::END {
            None
        } else {
            Some(self.terms_enum.term()?)
        };
        self.set_term(positioned);
        Ok(status)
    }

    /// Seeks to `term` exactly, reporting whether it exists.
    ///
    /// Equivalent to `TermsEnumIndex.seekExact(BytesRef)`. When the term is
    /// absent the cached term is cleared, matching Lucene.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the wrapped enum.
    pub fn seek_exact(&mut self, term: &BytesRef) -> Result<bool> {
        let found = self.terms_enum.seek_exact(term)?;
        let positioned = if found {
            Some(self.terms_enum.term()?)
        } else {
            None
        };
        self.set_term(positioned);
        Ok(found)
    }

    /// Seeks to the term at ordinal `ord`.
    ///
    /// Equivalent to `TermsEnumIndex.seekExact(long)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the wrapped enum, including the
    /// unsupported-operation error raised by enums that do not expose
    /// ordinals.
    pub fn seek_ord(&mut self, ord: i64) -> Result<()> {
        self.terms_enum.seek_ord(ord)?;
        let positioned = self.terms_enum.term()?;
        self.set_term(Some(positioned));
        Ok(())
    }

    /// Adopts `other`'s enum and cached position, keeping this wrapper's own
    /// [`sub_index`](Self::sub_index).
    ///
    /// Equivalent to `TermsEnumIndex.reset(TermsEnumIndex)`, which likewise
    /// leaves `subIndex` untouched so that a pre-allocated slot keeps
    /// identifying its own sub-reader.
    ///
    /// **Deliberate divergence**: Java copies a reference and leaves both
    /// wrappers pointing at the same `TermsEnum`. Rust owns the boxed enum, so
    /// `other` is consumed instead. Aliased ownership is exactly what the Java
    /// version leaves the caller responsible for avoiding; taking `other` by
    /// value makes that responsibility unnecessary.
    pub fn reset(&mut self, other: TermsEnumIndex) {
        self.terms_enum = other.terms_enum;
        self.current_term = other.current_term;
        self.current_term_prefix8 = other.current_term_prefix8;
    }

    /// Compares this wrapper's current term against `other`'s.
    ///
    /// Equivalent to `TermsEnumIndex.compareTermTo(TermsEnumIndex)`. The cached
    /// 8-byte prefixes settle the comparison whenever they differ; only terms
    /// sharing a prefix fall through to a full byte comparison.
    ///
    /// **Deliberate divergence**: Java dereferences the cached terms and throws
    /// `NullPointerException` when either enum is exhausted, because its
    /// callers drop exhausted enums from the merge queue before comparing. This
    /// port defines the missing case instead — an exhausted enum sorts *after*
    /// every positioned one — so that the result is a total order usable as a
    /// priority-queue comparator without a separate emptiness check.
    pub fn compare_term_to(&self, other: &TermsEnumIndex) -> Ordering {
        match (&self.current_term, &other.current_term) {
            (Some(this), Some(that)) => {
                if self.current_term_prefix8 != other.current_term_prefix8 {
                    self.current_term_prefix8.cmp(&other.current_term_prefix8)
                } else {
                    this.cmp(that)
                }
            }
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
    }

    /// Returns `true` if the current term equals the term captured in `state`.
    ///
    /// Equivalent to `TermsEnumIndex.termEquals(TermsEnumIndex.TermState)`.
    /// The cached prefixes reject unequal terms without touching the bytes in
    /// the overwhelming majority of cases.
    ///
    /// **Deliberate divergence**: as with [`compare_term_to`](Self::compare_term_to),
    /// an absent term on either side reports "not equal" rather than throwing.
    pub fn term_equals(&self, state: &TermsEnumIndexState) -> bool {
        match (&self.current_term, &state.term) {
            (Some(this), Some(that)) => {
                self.current_term_prefix8 == state.term_prefix8 && this == that
            }
            _ => false,
        }
    }
}

impl Debug for TermsEnumIndex {
    /// **Deliberate divergence**: Java's `toString()` forwards to the wrapped
    /// `TermsEnum`. [`TermsEnum`] carries no `Debug` bound in this crate, so
    /// the wrapper reports its own identifying state instead.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TermsEnumIndex")
            .field("sub_index", &self.sub_index)
            .field("current_term", &self.current_term)
            .finish()
    }
}

/// A snapshot of a [`TermsEnumIndex`]'s current term, kept for repeated
/// equality checks against enums that keep moving.
///
/// Equivalent to the nested `org.apache.lucene.index.TermsEnumIndex.TermState`.
/// It is named `TermsEnumIndexState` here because
/// [`TermState`](crate::index::terms::TermState) is already the crate-root name
/// of the unrelated codec-position trait that Java calls
/// `org.apache.lucene.index.TermState`.
#[derive(Debug, Clone, Default)]
pub struct TermsEnumIndexState {
    term: Option<BytesRef>,
    term_prefix8: u64,
}

impl TermsEnumIndexState {
    /// Captures `tei`'s current term, if any.
    ///
    /// Equivalent to `TermsEnumIndex.TermState.copyFrom(TermsEnumIndex)`.
    ///
    /// **Deliberate divergence**: Java throws `NullPointerException` when the
    /// enum has no current term; this port records "no term", which
    /// [`TermsEnumIndex::term_equals`] then reports as unequal to everything.
    pub fn copy_from(tei: &TermsEnumIndex) -> Self {
        Self {
            term: tei.current_term.clone(),
            term_prefix8: tei.current_term_prefix8,
        }
    }

    /// Returns the captured term, or `None` if the source had no current term.
    pub fn term(&self) -> Option<&BytesRef> {
        self.term.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::LuceneError;
    use crate::index::postings_enum::{ImpactsEnum, PostingsEnum};
    use crate::util::attribute::AttributeSource;

    /// Minimal in-memory [`TermsEnum`] over a sorted term list, used to drive
    /// the wrapper without pulling in a codec.
    struct VecTermsEnum {
        terms: Vec<BytesRef>,
        /// Index of the current term; `None` before the first move and after
        /// exhaustion.
        pos: Option<usize>,
        atts: AttributeSource,
    }

    impl VecTermsEnum {
        fn boxed(terms: Vec<&[u8]>) -> Box<dyn TermsEnum> {
            Box::new(Self {
                terms: terms
                    .into_iter()
                    .map(|t| BytesRef::new(t.to_vec()))
                    .collect(),
                pos: None,
                atts: AttributeSource::new(),
            })
        }
    }

    impl TermsEnum for VecTermsEnum {
        fn attributes(&mut self) -> &mut AttributeSource {
            &mut self.atts
        }

        fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
            match self.terms.iter().position(|t| t >= text) {
                Some(idx) => {
                    self.pos = Some(idx);
                    Ok(if &self.terms[idx] == text {
                        SeekStatus::FOUND
                    } else {
                        SeekStatus::NOT_FOUND
                    })
                }
                None => {
                    self.pos = None;
                    Ok(SeekStatus::END)
                }
            }
        }

        fn seek_ord(&mut self, ord: i64) -> Result<()> {
            let idx = usize::try_from(ord)
                .ok()
                .filter(|&idx| idx < self.terms.len())
                .ok_or_else(|| LuceneError::IllegalArgument(format!("bad ord {ord}")))?;
            self.pos = Some(idx);
            Ok(())
        }

        fn term(&self) -> Result<BytesRef> {
            self.pos
                .map(|idx| self.terms[idx].clone())
                .ok_or_else(|| LuceneError::IllegalState("not positioned".to_string()))
        }

        fn ord(&self) -> Result<i64> {
            self.pos
                .map(|idx| idx as i64)
                .ok_or_else(|| LuceneError::IllegalState("not positioned".to_string()))
        }

        fn doc_freq(&self) -> Result<i32> {
            Ok(1)
        }

        fn total_term_freq(&self) -> Result<i64> {
            Ok(1)
        }

        fn postings(
            &mut self,
            _reuse: Option<Box<dyn PostingsEnum>>,
            _flags: i32,
        ) -> Result<Box<dyn PostingsEnum>> {
            Err(LuceneError::UnsupportedOperation("postings".to_string()))
        }

        fn impacts(&mut self, _flags: i32) -> Result<Box<dyn ImpactsEnum>> {
            Err(LuceneError::UnsupportedOperation("impacts".to_string()))
        }

        fn next(&mut self) -> Result<Option<BytesRef>> {
            let next = match self.pos {
                Some(idx) => idx + 1,
                None => 0,
            };
            if next < self.terms.len() {
                self.pos = Some(next);
                Ok(Some(self.terms[next].clone()))
            } else {
                self.pos = None;
                Ok(None)
            }
        }
    }

    fn term(bytes: &[u8]) -> BytesRef {
        BytesRef::new(bytes.to_vec())
    }

    fn index_over(terms: Vec<&[u8]>, sub_index: usize) -> TermsEnumIndex {
        TermsEnumIndex::new(VecTermsEnum::boxed(terms), sub_index)
    }

    /// Positions a wrapper on `term` by scanning forward, so tests can build a
    /// wrapper in a known state without depending on seek.
    fn positioned_on(terms: Vec<&[u8]>, sub_index: usize, target: &[u8]) -> TermsEnumIndex {
        let mut tei = index_over(terms, sub_index);
        while let Some(current) = tei.next().unwrap() {
            if current.slice() == target {
                return tei;
            }
        }
        panic!("term {target:?} not present in the fixture");
    }

    // -----------------------------------------------------------------------
    // prefix8_to_comparable_unsigned_long
    // -----------------------------------------------------------------------

    #[test]
    fn prefix8_of_empty_term_is_zero() {
        // Java reaches this through `l <<= 64`, which the JLS masks to a shift
        // of 0. Rust would overflow, so the case is handled separately; this
        // test pins that it still yields Java's answer.
        assert_eq!(prefix8_to_comparable_unsigned_long(&term(&[])), 0);
    }

    #[test]
    fn prefix8_pads_short_terms_on_the_right() {
        assert_eq!(
            prefix8_to_comparable_unsigned_long(&term(&[0x01])),
            0x0100_0000_0000_0000
        );
        assert_eq!(
            prefix8_to_comparable_unsigned_long(&term(&[0x01, 0x02, 0x03])),
            0x0102_0300_0000_0000
        );
        assert_eq!(
            prefix8_to_comparable_unsigned_long(&term(&[0x01, 0x02, 0x03, 0x04, 0x05])),
            0x0102_0304_0500_0000
        );
        assert_eq!(
            prefix8_to_comparable_unsigned_long(&term(&[1, 2, 3, 4, 5, 6, 7])),
            0x0102_0304_0506_0700
        );
    }

    #[test]
    fn prefix8_reads_all_lengths_big_endian() {
        // Every length from 1 to 8 must place byte i at bit offset 56 - 8*i.
        for len in 1..=PREFIX_BYTES {
            let bytes: Vec<u8> = (1..=len as u8).collect();
            let expected = bytes
                .iter()
                .enumerate()
                .fold(0u64, |acc, (i, &b)| acc | (u64::from(b) << (56 - 8 * i)));
            assert_eq!(
                prefix8_to_comparable_unsigned_long(&term(&bytes)),
                expected,
                "length {len}"
            );
        }
    }

    #[test]
    fn prefix8_ignores_bytes_past_the_eighth() {
        let head = term(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let longer = term(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        assert_eq!(
            prefix8_to_comparable_unsigned_long(&head),
            prefix8_to_comparable_unsigned_long(&longer)
        );
        assert_eq!(
            prefix8_to_comparable_unsigned_long(&head),
            0x0102_0304_0506_0708
        );
    }

    #[test]
    fn prefix8_uses_the_active_slice_not_the_whole_buffer() {
        // A BytesRef carrying an offset must fold the bytes it points at.
        let sliced = BytesRef {
            bytes: vec![0xFF, 0xFF, 0x01, 0x02],
            offset: 2,
            length: 2,
        };
        assert_eq!(
            prefix8_to_comparable_unsigned_long(&sliced),
            0x0102_0000_0000_0000
        );
    }

    #[test]
    fn prefix8_is_unsigned_for_high_bytes() {
        // Java folds through a sign-extending `int` read; the trailing shift
        // discards those bits. A byte >= 0x80 must not turn the prefix
        // negative-looking relative to a smaller one.
        let low = prefix8_to_comparable_unsigned_long(&term(&[0x7F]));
        let high = prefix8_to_comparable_unsigned_long(&term(&[0x80]));
        let higher = prefix8_to_comparable_unsigned_long(&term(&[0xFF, 0xFF, 0xFF, 0xFF]));
        assert!(low < high, "0x7F must fold below 0x80");
        assert!(high < higher, "0x80 must fold below 0xFF...");
        assert_eq!(higher, 0xFFFF_FFFF_0000_0000);
    }

    #[test]
    fn prefix8_ordering_agrees_with_byte_ordering() {
        // The whole point of the fold: whenever two prefixes differ, their
        // unsigned order is the terms' order. Exhaustively checked over every
        // term of length 0..=3 drawn from a byte alphabet chosen to include
        // the 0x00 padding value and a high-bit byte.
        let alphabet = [0x00u8, 0x01, 0x7F, 0x80, 0xFF];
        let mut terms: Vec<Vec<u8>> = vec![Vec::new()];
        let mut frontier: Vec<Vec<u8>> = vec![Vec::new()];
        for _ in 0..3 {
            frontier = frontier
                .iter()
                .flat_map(|prefix| {
                    alphabet.iter().map(move |&byte| {
                        let mut next = prefix.clone();
                        next.push(byte);
                        next
                    })
                })
                .collect();
            terms.extend(frontier.iter().cloned());
        }
        assert_eq!(terms.len(), 1 + 5 + 25 + 125, "every term of length 0..=3");

        for a in &terms {
            for b in &terms {
                let (ra, rb) = (term(a), term(b));
                let pa = prefix8_to_comparable_unsigned_long(&ra);
                let pb = prefix8_to_comparable_unsigned_long(&rb);
                if pa != pb {
                    assert_eq!(
                        pa.cmp(&pb),
                        ra.cmp(&rb),
                        "prefix order disagrees for {a:?} vs {b:?}"
                    );
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Positioning
    // -----------------------------------------------------------------------

    #[test]
    fn new_wrapper_is_unpositioned() {
        let tei = index_over(vec![b"a"], 3);
        assert_eq!(tei.sub_index(), 3);
        assert!(tei.term().is_none());
        assert_eq!(tei.term_prefix8(), 0);
    }

    #[test]
    fn next_walks_every_term_then_clears_the_cache() {
        let mut tei = index_over(vec![b"aa", b"ab", b"b"], 0);
        for expected in [b"aa".as_slice(), b"ab".as_slice(), b"b".as_slice()] {
            assert_eq!(tei.next().unwrap().unwrap().slice(), expected);
            assert_eq!(tei.term().unwrap().slice(), expected);
            assert_eq!(
                tei.term_prefix8(),
                prefix8_to_comparable_unsigned_long(&term(expected))
            );
        }
        assert!(tei.next().unwrap().is_none());
        assert!(tei.term().is_none());
        assert_eq!(tei.term_prefix8(), 0, "prefix must reset with the term");
    }

    #[test]
    fn seek_ceil_found_caches_the_exact_term() {
        let mut tei = index_over(vec![b"aa", b"ab", b"b"], 0);
        assert_eq!(tei.seek_ceil(&term(b"ab")).unwrap(), SeekStatus::FOUND);
        assert_eq!(tei.term().unwrap().slice(), b"ab");
        assert_eq!(
            tei.term_prefix8(),
            prefix8_to_comparable_unsigned_long(&term(b"ab"))
        );
    }

    #[test]
    fn seek_ceil_not_found_caches_the_ceiling_term() {
        let mut tei = index_over(vec![b"aa", b"ac"], 0);
        assert_eq!(tei.seek_ceil(&term(b"ab")).unwrap(), SeekStatus::NOT_FOUND);
        assert_eq!(tei.term().unwrap().slice(), b"ac");
    }

    #[test]
    fn seek_ceil_end_clears_the_cache() {
        let mut tei = index_over(vec![b"aa", b"ab"], 0);
        // Position first, so the test proves the cache is cleared and not
        // merely never set.
        assert!(tei.next().unwrap().is_some());
        assert_eq!(tei.seek_ceil(&term(b"z")).unwrap(), SeekStatus::END);
        assert!(tei.term().is_none());
        assert_eq!(tei.term_prefix8(), 0);
    }

    #[test]
    fn seek_exact_hit_and_miss() {
        let mut tei = index_over(vec![b"aa", b"ab"], 0);
        assert!(tei.seek_exact(&term(b"ab")).unwrap());
        assert_eq!(tei.term().unwrap().slice(), b"ab");

        assert!(!tei.seek_exact(&term(b"zz")).unwrap());
        assert!(tei.term().is_none(), "a miss must clear the cached term");
        assert_eq!(tei.term_prefix8(), 0);
    }

    #[test]
    fn seek_ord_caches_the_term_at_that_ordinal() {
        let mut tei = index_over(vec![b"aa", b"ab", b"b"], 0);
        tei.seek_ord(2).unwrap();
        assert_eq!(tei.term().unwrap().slice(), b"b");
        assert_eq!(tei.terms_enum().ord().unwrap(), 2);
    }

    #[test]
    fn seek_ord_out_of_range_is_an_error() {
        let mut tei = index_over(vec![b"aa"], 0);
        assert!(tei.seek_ord(7).is_err());
    }

    #[test]
    fn reset_adopts_the_position_but_keeps_the_sub_index() {
        let source = positioned_on(vec![b"x", b"y"], 9, b"y");
        let mut target = index_over(vec![b"aa"], 4);
        assert!(target.next().unwrap().is_some());

        target.reset(source);

        assert_eq!(target.sub_index(), 4, "reset must not overwrite sub_index");
        assert_eq!(target.term().unwrap().slice(), b"y");
        assert_eq!(
            target.term_prefix8(),
            prefix8_to_comparable_unsigned_long(&term(b"y"))
        );
        // The adopted enum must be the source's, so iteration continues from
        // its position rather than restarting the old one.
        assert!(target.next().unwrap().is_none());
    }

    #[test]
    fn reset_adopts_an_exhausted_position() {
        let mut source = index_over(vec![b"x"], 1);
        while source.next().unwrap().is_some() {}
        let mut target = positioned_on(vec![b"aa", b"ab"], 2, b"aa");

        target.reset(source);

        assert!(target.term().is_none());
        assert_eq!(target.term_prefix8(), 0);
    }

    // -----------------------------------------------------------------------
    // Comparison
    // -----------------------------------------------------------------------

    #[test]
    fn compare_term_to_orders_by_term_bytes() {
        let a = positioned_on(vec![b"aa", b"ab"], 0, b"aa");
        let b = positioned_on(vec![b"aa", b"ab"], 1, b"ab");
        assert_eq!(a.compare_term_to(&b), Ordering::Less);
        assert_eq!(b.compare_term_to(&a), Ordering::Greater);
        assert_eq!(a.compare_term_to(&a), Ordering::Equal);
    }

    #[test]
    fn compare_term_to_falls_through_when_prefixes_collide() {
        // Both terms fold to the same prefix (zero padding), so only the full
        // byte comparison can separate them.
        let short = positioned_on(vec![b"\x01"], 0, b"\x01");
        let long = positioned_on(vec![b"\x01\x00"], 1, b"\x01\x00");
        assert_eq!(short.term_prefix8(), long.term_prefix8());
        assert_eq!(short.compare_term_to(&long), Ordering::Less);
        assert_eq!(long.compare_term_to(&short), Ordering::Greater);
    }

    #[test]
    fn compare_term_to_separates_terms_sharing_eight_bytes() {
        let short = positioned_on(vec![b"abcdefgh"], 0, b"abcdefgh");
        let long = positioned_on(vec![b"abcdefghi"], 1, b"abcdefghi");
        assert_eq!(short.term_prefix8(), long.term_prefix8());
        assert_eq!(short.compare_term_to(&long), Ordering::Less);
    }

    #[test]
    fn compare_term_to_sorts_exhausted_enums_last() {
        // Deliberate divergence from Java, which would throw here.
        let positioned = positioned_on(vec![b"aa"], 0, b"aa");
        let mut exhausted = index_over(vec![b"aa"], 1);
        while exhausted.next().unwrap().is_some() {}

        assert_eq!(positioned.compare_term_to(&exhausted), Ordering::Less);
        assert_eq!(exhausted.compare_term_to(&positioned), Ordering::Greater);
        assert_eq!(exhausted.compare_term_to(&exhausted), Ordering::Equal);
    }

    #[test]
    fn compare_term_to_matches_full_byte_comparison() {
        // Cross-check the prefix fast path against the ground truth over terms
        // engineered to collide, differ inside the prefix, and differ only
        // after it.
        let corpus: Vec<&[u8]> = vec![
            b"",
            b"\x00",
            b"\x01",
            b"\x01\x00",
            b"\x01\x00\x00",
            b"abcdefgh",
            b"abcdefgh\x00",
            b"abcdefghi",
            b"abcdefghz",
            b"\xff\xff\xff\xff\xff\xff\xff\xff",
            b"\xff\xff\xff\xff\xff\xff\xff\xff\x01",
        ];
        for &left in &corpus {
            for &right in &corpus {
                let a = positioned_on(vec![left], 0, left);
                let b = positioned_on(vec![right], 1, right);
                assert_eq!(
                    a.compare_term_to(&b),
                    term(left).cmp(&term(right)),
                    "{left:?} vs {right:?}"
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // TermsEnumIndexState
    // -----------------------------------------------------------------------

    #[test]
    fn term_equals_matches_the_captured_term() {
        let mut tei = index_over(vec![b"aa", b"ab"], 0);
        assert!(tei.next().unwrap().is_some());
        let state = TermsEnumIndexState::copy_from(&tei);
        assert_eq!(state.term().unwrap().slice(), b"aa");
        assert!(tei.term_equals(&state));

        assert!(tei.next().unwrap().is_some());
        assert!(!tei.term_equals(&state), "moving must break the match");
    }

    #[test]
    fn term_equals_distinguishes_prefix_colliding_terms() {
        // The prefixes are equal, so a correct implementation must still
        // compare the bytes.
        let short = positioned_on(vec![b"\x01"], 0, b"\x01");
        let long = positioned_on(vec![b"\x01\x00"], 1, b"\x01\x00");
        let state = TermsEnumIndexState::copy_from(&short);
        assert_eq!(short.term_prefix8(), long.term_prefix8());
        assert!(short.term_equals(&state));
        assert!(!long.term_equals(&state));
    }

    #[test]
    fn term_equals_is_false_when_either_side_has_no_term() {
        let mut tei = index_over(vec![b"aa"], 0);
        let empty_state = TermsEnumIndexState::copy_from(&tei);
        assert!(empty_state.term().is_none());

        // Unpositioned wrapper against an empty capture.
        assert!(!tei.term_equals(&empty_state));

        // Positioned wrapper against an empty capture.
        assert!(tei.next().unwrap().is_some());
        assert!(!tei.term_equals(&empty_state));

        // Exhausted wrapper against a real capture.
        let real_state = TermsEnumIndexState::copy_from(&tei);
        assert!(tei.next().unwrap().is_none());
        assert!(!tei.term_equals(&real_state));
    }

    #[test]
    fn term_equals_handles_the_empty_term() {
        // The empty term is a legitimate Lucene term and folds to prefix 0,
        // the same value an absent term reports. Equality must still hold.
        let tei = positioned_on(vec![b""], 0, b"");
        let state = TermsEnumIndexState::copy_from(&tei);
        assert_eq!(tei.term_prefix8(), 0);
        assert!(tei.term_equals(&state));
    }

    #[test]
    fn debug_reports_sub_index_and_term() {
        let tei = positioned_on(vec![b"aa"], 7, b"aa");
        let rendered = format!("{tei:?}");
        assert!(rendered.contains("sub_index: 7"), "{rendered}");
        assert!(rendered.contains("TermsEnumIndex"), "{rendered}");
    }
}
