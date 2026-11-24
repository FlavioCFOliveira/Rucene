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

//! Port of `FieldTermStack.java`, of the Java package
//! `org.apache.lucene.search.vectorhighlight`.

use std::collections::VecDeque;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{IndexReader, PostingsEnum, Term, POSTINGS_ENUM_POSITIONS};

use crate::vectorhighlight::field_query::FieldQuery;

/// `FieldTermStack` is a stack that keeps query terms in the specified
/// field of the document to be highlighted.
///
/// Equivalent to `org.apache.lucene.search.vectorhighlight.FieldTermStack`.
#[derive(Debug, Clone)]
pub struct FieldTermStack {
    field_name: String,
    pub(crate) term_list: VecDeque<TermInfo>,
}

impl FieldTermStack {
    /// Equivalent to `new FieldTermStack(IndexReader, int, String,
    /// FieldQuery)`: `doc_id` is the document id to be highlighted and
    /// `field_name` the field of the document to be highlighted.
    ///
    /// # Errors
    ///
    /// Propagates I/O errors.
    pub fn new(
        reader: &dyn IndexReader,
        doc_id: i32,
        field_name: &str,
        field_query: &FieldQuery,
    ) -> Result<Self> {
        let mut this = Self {
            field_name: field_name.to_string(),
            term_list: VecDeque::new(),
        };

        let Some(term_set) = field_query.get_term_set(field_name) else {
            return Ok(this);
        };

        let Some(vectors) = reader.term_vectors()?.get(doc_id)? else {
            // null snippet
            return Ok(this);
        };

        let vector = vectors.terms(field_name)?;
        let Some(vector) = vector.filter(|v| v.has_positions()) else {
            // null snippet
            return Ok(this);
        };

        let mut terms_enum = vector.iterator()?;
        let mut dp_enum: Option<Box<dyn PostingsEnum>> = None;
        let num_docs = reader.max_doc();

        let mut infos: Vec<TermInfoData> = Vec::new();
        loop {
            let text = match terms_enum.next()? {
                Some(text) => text.clone(),
                None => break,
            };
            let term = String::from_utf8_lossy(text.slice()).into_owned();

            // ... and on its way to the TermInfos the query term set filters
            if !term_set.contains(&term) {
                continue;
            }
            let mut dp = terms_enum
                .postings(dp_enum.take(), POSTINGS_ENUM_POSITIONS)?
                .expect("INVARIANT: a term vector term has postings");
            dp.next_doc()?;

            // For weight look here: http://lucene.apache.org/core/3_6_0/api/core/org/apache/lucene/search/DefaultSimilarity.html
            let weight = ((num_docs as f64
                / (reader.doc_freq(&Term::new(field_name.to_string(), text.clone()))? as f64
                    + 1.0))
                .ln()
                + 1.0) as f32;

            let freq = dp.freq()?;

            for _ in 0..freq {
                let pos = dp.next_position()?;
                if dp.start_offset() < 0 {
                    return Ok(this); // no offsets, null snippet
                }
                infos.push(TermInfoData {
                    text: term.clone(),
                    start_offset: dp.start_offset(),
                    end_offset: dp.end_offset(),
                    position: pos,
                    weight,
                    next: infos.len(),
                });
            }
            dp_enum = Some(dp);
        }

        // sort by position
        infos.sort_by_key(|info| info.position);

        // now look for dups at the same position, linking them together
        let mut current_pos = -1;
        let mut previous: Option<usize> = None;
        let mut first: Option<usize> = None;
        let mut kept: Vec<usize> = Vec::new();
        for current in 0..infos.len() {
            if infos[current].position == current_pos {
                let prev = previous.expect("INVARIANT: a previous term at this position");
                infos[prev].next = current;
                previous = Some(current);
                // removed from the term list
            } else {
                if let Some(prev) = previous {
                    infos[prev].next = first.expect("INVARIANT: set with previous");
                }
                previous = Some(current);
                first = Some(current);
                current_pos = infos[current].position;
                kept.push(current);
            }
        }
        if let Some(prev) = previous {
            infos[prev].next = first.expect("INVARIANT: set with previous");
        }
        let arena: Arc<[TermInfoData]> = Arc::from(infos);
        this.term_list = kept
            .into_iter()
            .map(|index| TermInfo {
                arena: Arc::clone(&arena),
                index,
            })
            .collect();
        Ok(this)
    }

    /// Returns field name.
    ///
    /// Equivalent to `getFieldName()`.
    pub fn get_field_name(&self) -> &str {
        &self.field_name
    }

    /// Returns the top TermInfo object of the stack.
    ///
    /// Equivalent to `pop()`.
    pub fn pop(&mut self) -> Option<TermInfo> {
        self.term_list.pop_front()
    }

    /// Equivalent to `push(TermInfo)`: the TermInfo object to be put on the
    /// top of the stack.
    pub fn push(&mut self, term_info: TermInfo) {
        self.term_list.push_front(term_info);
    }

    /// Checks if the stack is empty.
    ///
    /// Equivalent to `isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.term_list.is_empty()
    }

    /// The package-private field `termList`, read by the same package's
    /// tests (`TestFieldTermStack`); Rust's integration tests are another
    /// crate, so the port exposes it (hidden from the documentation).
    #[doc(hidden)]
    pub fn term_list(&self) -> &VecDeque<TermInfo> {
        &self.term_list
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TermInfoData {
    text: String,
    start_offset: i32,
    end_offset: i32,
    position: i32,
    // IDF-weight of this term
    weight: f32,
    // pointer to other TermInfo's at the same position.
    // this is a circular list, so with no syns, just points to itself
    next: usize,
}

/// Single term with its position/offsets in the document and IDF weight. It
/// is Comparable but considers only position.
///
/// Equivalent to `org.apache.lucene.search.vectorhighlight.FieldTermStack.TermInfo`.
/// A value is a handle onto one term info, as a Java reference is: the
/// circular list of the term infos at one position (`getNext()`) is shared
/// by every handle of that stack.
#[derive(Clone)]
pub struct TermInfo {
    arena: Arc<[TermInfoData]>,
    index: usize,
}

impl TermInfo {
    /// Equivalent to the package-private `new TermInfo(String, int, int,
    /// int, float)`: a term info whose next term info is itself.
    pub fn new(text: &str, start_offset: i32, end_offset: i32, position: i32, weight: f32) -> Self {
        Self {
            arena: Arc::from(vec![TermInfoData {
                text: text.to_string(),
                start_offset,
                end_offset,
                position,
                weight,
                next: 0,
            }]),
            index: 0,
        }
    }

    fn data(&self) -> &TermInfoData {
        &self.arena[self.index]
    }

    /// Equivalent to `getNext()`.
    pub fn get_next(&self) -> TermInfo {
        TermInfo {
            arena: Arc::clone(&self.arena),
            index: self.data().next,
        }
    }

    /// Whether `self` and `other` are the same term info (Java's `==`).
    pub fn same(&self, other: &TermInfo) -> bool {
        Arc::ptr_eq(&self.arena, &other.arena) && self.index == other.index
    }

    /// Equivalent to `getText()`.
    pub fn get_text(&self) -> &str {
        &self.data().text
    }

    /// Equivalent to `getStartOffset()`.
    pub fn get_start_offset(&self) -> i32 {
        self.data().start_offset
    }

    /// Equivalent to `getEndOffset()`.
    pub fn get_end_offset(&self) -> i32 {
        self.data().end_offset
    }

    /// Equivalent to `getPosition()`.
    pub fn get_position(&self) -> i32 {
        self.data().position
    }

    /// Equivalent to `getWeight()`.
    pub fn get_weight(&self) -> f32 {
        self.data().weight
    }

    /// Equivalent to `compareTo(TermInfo)`.
    pub fn compare_to(&self, o: &TermInfo) -> i32 {
        self.get_position().wrapping_sub(o.get_position())
    }

    /// Equivalent to `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        let prime = 31i32;
        let result = 1i32;
        prime.wrapping_mul(result).wrapping_add(self.get_position())
    }
}

impl fmt::Debug for TermInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for TermInfo {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}({},{},{})",
            self.get_text(),
            self.get_start_offset(),
            self.get_end_offset(),
            self.get_position()
        )
    }
}

impl PartialEq for TermInfo {
    /// Equivalent to `equals(Object)`: equal positions.
    fn eq(&self, other: &Self) -> bool {
        self.get_position() == other.get_position()
    }
}

impl Eq for TermInfo {}

impl Hash for TermInfo {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash_code().hash(state);
    }
}
