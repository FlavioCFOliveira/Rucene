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

//! Port of `CachingMatchesIterator.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::search::matches::MatchesIterator;
use rucene::search::query::Query;
use rucene::util::{ArrayUtil, RamUsageEstimator};

use crate::intervals::interval_matches_iterator::IntervalMatchesIterator;

/// The positions, offsets and queries a [`CachingMatchesIterator`] caches;
/// shared with the sub-matches iterators it hands out, which read them as
/// Java's anonymous inner class reads its outer instance's fields.
struct CachedMatches {
    /// The `posAndOffsets` field: four entries per cached match.
    pos_and_offsets: Vec<i32>,
    /// The `matchingQueries` field; `None` stands for `null`.
    matching_queries: Vec<Option<Arc<dyn Query>>>,
    /// The `count` field.
    count: i32,
}

impl CachedMatches {
    /// `posAndOffsets[index]`, with the `ArrayIndexOutOfBoundsException` Java
    /// raises for an index outside the array.
    fn pos_and_offset(&self, index: i64) -> Result<i32> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.pos_and_offsets.get(i).copied())
            .ok_or_else(|| {
                LuceneError::unchecked(
                    "java.lang.ArrayIndexOutOfBoundsException",
                    format!(
                        "Index {index} out of bounds for length {}",
                        self.pos_and_offsets.len()
                    ),
                )
            })
    }

    /// `matchingQueries[index]`, which must be a query.
    fn query(&self, index: i64) -> Arc<dyn Query> {
        let query = usize::try_from(index)
            .ok()
            .and_then(|i| self.matching_queries.get(i).cloned());
        match query {
            Some(Some(query)) => query,
            Some(None) => panic!(
                "CachingMatchesIterator: the cached query at {index} is null, which Java returns as \
                 is and the core MatchesIterator::get_query contract cannot"
            ),
            None => panic!(
                "java.lang.ArrayIndexOutOfBoundsException: Index {index} out of bounds for length {}",
                self.matching_queries.len()
            ),
        }
    }

    /// `if (count * 4 >= posAndOffsets.length) { posAndOffsets =
    /// ArrayUtil.grow(posAndOffsets, (count + 1) * 4); matchingQueries =
    /// ArrayUtil.grow(matchingQueries, (count + 1)); }`: the arrays keep the
    /// lengths `ArrayUtil.grow` gives them in Java, `oversize` of the minimum
    /// size for 4-byte `int`s and object references, which is not always a
    /// multiple of four.
    fn grow(&mut self) {
        let count = self.count as usize;
        if count * 4 >= self.pos_and_offsets.len() {
            let min_size = (count + 1) * 4;
            if self.pos_and_offsets.len() < min_size {
                let new_len = ArrayUtil::oversize(min_size, 4);
                self.pos_and_offsets.resize(new_len, 0);
            }
            let min_size = count + 1;
            if self.matching_queries.len() < min_size {
                let new_len =
                    ArrayUtil::oversize(min_size, RamUsageEstimator::NUM_BYTES_OBJECT_REF as usize);
                self.matching_queries.resize(new_len, None);
            }
        }
    }

    /// `posAndOffsets[index] = value`, with the
    /// `ArrayIndexOutOfBoundsException` Java raises for an index outside the
    /// array.
    fn set_pos_and_offset(&mut self, index: usize, value: i32) -> Result<()> {
        let len = self.pos_and_offsets.len();
        match self.pos_and_offsets.get_mut(index) {
            Some(slot) => {
                *slot = value;
                Ok(())
            }
            None => Err(LuceneError::unchecked(
                "java.lang.ArrayIndexOutOfBoundsException",
                format!("Index {index} out of bounds for length {len}"),
            )),
        }
    }
}

/// A matches iterator that caches the positions and offsets of the current
/// match, and of its sub-matches, when told to.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.intervals.CachingMatchesIterator`, a
/// `FilterMatchesIterator` over an [`IntervalMatchesIterator`].
///
/// **Divergence.** Java's [`get_query`](MatchesIterator::get_query) answers
/// `matchingQueries[0]`, which can be `null` (an empty cache, or a sub-match
/// whose `getQuery()` returned `null`). The core
/// [`MatchesIterator::get_query`] returns a query, so a `null` entry panics
/// where Java would hand `null` to its caller.
pub(crate) struct CachingMatchesIterator {
    /// The `in` field.
    in_: Box<dyn IntervalMatchesIterator>,
    cache: Rc<RefCell<CachedMatches>>,
}

impl CachingMatchesIterator {
    /// Equivalent to `CachingMatchesIterator(IntervalMatchesIterator)`.
    pub(crate) fn new(in_: Box<dyn IntervalMatchesIterator>) -> Self {
        Self {
            in_,
            cache: Rc::new(RefCell::new(CachedMatches {
                pos_and_offsets: vec![0; 4 * 4],
                matching_queries: vec![None; 4],
                count: 0,
            })),
        }
    }

    /// Caches the current match: the match itself when it has no
    /// sub-matches, every sub-match otherwise.
    ///
    /// Equivalent to the package-private `CachingMatchesIterator.cache()`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading the matches.
    pub(crate) fn cache(&mut self) -> Result<()> {
        let mut cache = self.cache.borrow_mut();
        cache.count = 0;
        let mi = self.in_.get_sub_matches()?;
        match mi {
            None => {
                cache.count = 1;
                cache.pos_and_offsets[0] = self.in_.start_position();
                cache.pos_and_offsets[1] = self.in_.end_position();
                cache.pos_and_offsets[2] = self.in_.start_offset()?;
                cache.pos_and_offsets[3] = self.in_.end_offset()?;
                cache.matching_queries[0] = Some(self.in_.get_query());
            }
            Some(mut mi) => {
                while mi.next()? {
                    cache.grow();
                    let base = (cache.count * 4) as usize;
                    cache.set_pos_and_offset(base, mi.start_position())?;
                    cache.set_pos_and_offset(base + 1, mi.end_position())?;
                    cache.set_pos_and_offset(base + 2, mi.start_offset()?)?;
                    cache.set_pos_and_offset(base + 3, mi.end_offset()?)?;
                    let count = cache.count as usize;
                    let query = mi.get_query();
                    match cache.matching_queries.get_mut(count) {
                        Some(slot) => *slot = Some(query),
                        None => {
                            return Err(LuceneError::unchecked(
                                "java.lang.ArrayIndexOutOfBoundsException",
                                format!(
                                    "Index {count} out of bounds for length {}",
                                    cache.matching_queries.len()
                                ),
                            ))
                        }
                    }
                    cache.count += 1;
                }
            }
        }
        Ok(())
    }
}

impl MatchesIterator for CachingMatchesIterator {
    fn next(&mut self) -> Result<bool> {
        self.in_.next()
    }

    fn start_position(&self) -> i32 {
        self.in_.start_position()
    }

    fn end_position(&self) -> i32 {
        self.in_.end_position()
    }

    fn start_offset(&self) -> Result<i32> {
        self.cache.borrow().pos_and_offset(2)
    }

    fn end_offset(&self) -> Result<i32> {
        let cache = self.cache.borrow();
        cache.pos_and_offset(i64::from(cache.count - 1) * 4 + 3)
    }

    /// We always return a sub-matches iterator, even if there is only a single
    /// cached sub-match, because this way we can return the correct positions:
    /// the positions of the top-level match may have moved on due to
    /// minimisation.
    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        Ok(Some(Box::new(CachedSubMatches {
            cache: Rc::clone(&self.cache),
            upto: -1,
        })))
    }

    fn get_query(&self) -> Arc<dyn Query> {
        self.cache.borrow().query(0)
    }
}

impl IntervalMatchesIterator for CachingMatchesIterator {
    fn gaps(&self) -> i32 {
        self.in_.gaps()
    }

    fn width(&self) -> i32 {
        self.in_.width()
    }
}

/// The anonymous `MatchesIterator` of `CachingMatchesIterator.getSubMatches`,
/// which reads the cache of its outer instance.
struct CachedSubMatches {
    cache: Rc<RefCell<CachedMatches>>,
    upto: i32,
}

impl CachedSubMatches {
    fn entry(&self, offset: i64) -> i32 {
        let index = i64::from(self.upto) * 4 + offset;
        let cache = self.cache.borrow();
        match cache.pos_and_offset(index) {
            Ok(value) => value,
            Err(e) => panic!("{e}"),
        }
    }
}

impl MatchesIterator for CachedSubMatches {
    fn next(&mut self) -> Result<bool> {
        self.upto += 1;
        Ok(self.upto < self.cache.borrow().count)
    }

    fn start_position(&self) -> i32 {
        self.entry(0)
    }

    fn end_position(&self) -> i32 {
        self.entry(1)
    }

    fn start_offset(&self) -> Result<i32> {
        self.cache
            .borrow()
            .pos_and_offset(i64::from(self.upto) * 4 + 2)
    }

    fn end_offset(&self) -> Result<i32> {
        self.cache
            .borrow()
            .pos_and_offset(i64::from(self.upto) * 4 + 3)
    }

    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        Ok(None)
    }

    fn get_query(&self) -> Arc<dyn Query> {
        self.cache.borrow().query(i64::from(self.upto))
    }
}
