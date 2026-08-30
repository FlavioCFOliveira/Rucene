//! Port of `org.apache.lucene.util.automaton.LimitedFiniteStringsIterator`.

use crate::error::{LuceneError, Result};
use crate::util::IntsRef;

use super::automaton::Automaton;
use super::finite_strings_iterator::FiniteStringsIterator;

/// A [`FiniteStringsIterator`] which limits the number of iterated accepted strings.
///
/// Equivalent to `org.apache.lucene.util.automaton.LimitedFiniteStringsIterator`. If
/// more than `limit` strings are accepted, the first `limit` strings found are
/// returned.
pub struct LimitedFiniteStringsIterator<'a> {
    inner: FiniteStringsIterator<'a>,
    /// Maximum number of finite strings to create.
    limit: i32,
    /// Number of generated finite strings.
    count: i32,
}

impl<'a> LimitedFiniteStringsIterator<'a> {
    /// Creates an iterator over at most `limit` finite strings of `a`, or over all of
    /// them when `limit` is `-1`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `limit` is neither `-1` nor
    /// positive.
    pub fn new(a: &'a Automaton, limit: i32) -> Result<Self> {
        if limit != -1 && limit <= 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "limit must be -1 (which means no limit), or > 0; got: {}",
                limit
            )));
        }

        Ok(Self {
            inner: FiniteStringsIterator::new(a),
            limit: if limit > 0 { limit } else { i32::MAX },
            count: 0,
        })
    }

    /// Generates the next finite string, or `None` once the limit is reached or no
    /// more finite strings are available.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if a cycle is detected.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<IntsRef>> {
        if self.count >= self.limit {
            // Abort on limit.
            return Ok(None);
        }

        let result = self.inner.next()?;
        if result.is_some() {
            self.count += 1;
        }

        Ok(result)
    }

    /// Number of iterated finite strings.
    pub fn size(&self) -> i32 {
        self.count
    }
}
