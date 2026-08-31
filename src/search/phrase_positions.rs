//! The position of a phrase term in a document, ported from
//! `org.apache.lucene.search.PhrasePositions`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::index::Term;
use crate::search::phrase_matcher::SharedPostings;

/// The position of a term in a document, taking the term's offset within the
/// phrase into account.
///
/// Equivalent to the `final org.apache.lucene.search.PhrasePositions`, which is
/// package-private in Java; it is public here because Rust has no package
/// visibility and [`SloppyPhraseMatcher`](crate::search::SloppyPhraseMatcher)
/// lives in a sibling module.
#[derive(Debug)]
pub struct PhrasePositions {
    /// The position in the document, minus [`offset`](Self::offset).
    pub position: i32,
    /// The number of positions remaining in this document.
    pub count: i32,
    /// The position in the phrase.
    pub offset: i32,
    /// Unique across all `PhrasePositions` instances of one matcher.
    pub ord: i32,
    /// The stream of docs and positions.
    pub postings: SharedPostings,
    /// Used to make lists.
    ///
    /// Java declares it and nothing in the search package reads it; it is kept
    /// as an index into the matcher's slice rather than a pointer.
    pub next: Option<usize>,
    /// `>= 0` indicates that this is a repeating position.
    pub rpt_group: i32,
    /// The index within [`rpt_group`](Self::rpt_group).
    pub rpt_ind: i32,
    /// The terms, for repetition initialisation.
    pub terms: Vec<Term>,
    /// The cached frequency for the current document.
    pub freq: i32,
}

impl PhrasePositions {
    /// Creates a phrase position over a postings list.
    ///
    /// Equivalent to
    /// `PhrasePositions(PostingsEnum, int, int, Term[])`.
    pub fn new(postings: SharedPostings, o: i32, ord: i32, terms: Vec<Term>) -> Self {
        Self {
            position: 0,
            count: 0,
            offset: o,
            ord,
            postings,
            next: None,
            rpt_group: -1,
            rpt_ind: 0,
            terms,
            freq: 0,
        }
    }

    /// Moves to the first position of the current document.
    ///
    /// Equivalent to the `final PhrasePositions.firstPosition()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the position.
    pub fn first_position(&mut self) -> Result<()> {
        // Use the cached frequency.
        self.count = self.freq;
        self.next_position()?;
        Ok(())
    }

    /// Goes to the next location of this term in the current document, setting
    /// [`position`](Self::position) to `location - offset`, so that a matching
    /// exact phrase is easily identified when all phrase positions have exactly
    /// the same position.
    ///
    /// Equivalent to the `final PhrasePositions.nextPosition()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the position.
    pub fn next_position(&mut self) -> Result<bool> {
        let count = self.count;
        self.count -= 1;
        if count > 0 {
            // Read the subsequent positions.
            self.position = self.postings.next_position()? - self.offset;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

impl std::fmt::Display for PhrasePositions {
    /// Equivalent to `PhrasePositions.toString()`, which exists for debugging.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "o:{} p:{} c:{}", self.offset, self.position, self.count)?;
        if self.rpt_group >= 0 {
            write!(f, " rpt:{},i{}", self.rpt_group, self.rpt_ind)?;
        }
        Ok(())
    }
}
