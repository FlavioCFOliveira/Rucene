//! Match positions of a single term, ported from
//! `org.apache.lucene.search.TermMatchesIterator`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::Result;
use crate::index::PostingsEnum;
use crate::search::matches::MatchesIterator;
use crate::search::query::Query;

/// A [`MatchesIterator`] over a single term's postings list.
///
/// Equivalent to `org.apache.lucene.search.TermMatchesIterator`, which is
/// package-private in Java; it is public here because Rust has no package
/// visibility and the weights that build it live in sibling modules.
pub struct TermMatchesIterator {
    upto: i32,
    pos: i32,
    pe: Box<dyn PostingsEnum>,
    query: Arc<dyn Query>,
}

impl std::fmt::Debug for TermMatchesIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TermMatchesIterator")
            .field("upto", &self.upto)
            .field("pos", &self.pos)
            .finish_non_exhaustive()
    }
}

impl TermMatchesIterator {
    /// Creates a new iterator for the given term and postings list.
    ///
    /// Equivalent to `new TermMatchesIterator(Query, PostingsEnum)`, which
    /// reads the term frequency of the document the postings enum is positioned
    /// on.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the frequency.
    pub fn new(query: Arc<dyn Query>, pe: Box<dyn PostingsEnum>) -> Result<Self> {
        let upto = pe.freq()?;
        Ok(Self {
            upto,
            pos: 0,
            pe,
            query,
        })
    }

    /// Unwraps this iterator, returning the postings enum it was built from.
    ///
    /// **Divergence from Lucene 10.5.0.** Java hands the same `PostingsEnum`
    /// instance to `TermsEnum.postings(reuse, flags)` for reuse after the
    /// iterator is discarded, because the reference is still in scope. Rust
    /// moves the enum into the iterator, so recovering it needs a method.
    pub fn into_postings(self) -> Box<dyn PostingsEnum> {
        self.pe
    }
}

impl MatchesIterator for TermMatchesIterator {
    fn next(&mut self) -> Result<bool> {
        let upto = self.upto;
        self.upto -= 1;
        if upto > 0 {
            self.pos = self.pe.next_position()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn start_position(&self) -> i32 {
        self.pos
    }

    fn end_position(&self) -> i32 {
        self.pos
    }

    fn start_offset(&self) -> Result<i32> {
        Ok(self.pe.start_offset())
    }

    fn end_offset(&self) -> Result<i32> {
        Ok(self.pe.end_offset())
    }

    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        Ok(None)
    }

    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query)
    }
}
