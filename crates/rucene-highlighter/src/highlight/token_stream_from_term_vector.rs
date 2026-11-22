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

//! Port of `TokenStreamFromTermVector.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use rucene::analysis::tokenattributes::{
    CharTermAttribute, OffsetAttribute, PayloadAttribute, PositionIncrementAttribute,
};
use rucene::analysis::{default_token_attribute_factory, AttributeHandle, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::index::{
    PostingsEnum, Terms, POSTINGS_ENUM_OFFSETS, POSTINGS_ENUM_PAYLOADS, POSTINGS_ENUM_POSITIONS,
};
use rucene::util::attribute::AttributeSource;
use rucene::util::bytes_ref_array::{BytesRefArray, SortableBytesRefArray};
use rucene::util::concurrent::Counter;
use rucene::util::BytesRef;

/// TokenStream created from a term vector field. The term vector requires
/// positions and/or offsets (either). If you want payloads add
/// PayloadAttributeImpl (as you would normally) but don't assume the
/// attribute is already added just because you know the term vector has
/// payloads, since the first call to incrementToken() will observe if you
/// asked for them and if not then won't get them. This TokenStream supports
/// an efficient `reset()`, so there's no need to wrap with a caching impl.
///
/// Equivalent to `org.apache.lucene.search.highlight.TokenStreamFromTermVector`.
///
/// The term text is held as the `char`s of the port's `CharTermAttribute`
/// (Java holds UTF-16 units); a term's length is limited to
/// `Short.MAX_VALUE` of those, as Java limits its UTF-16 length.
pub struct TokenStreamFromTermVector {
    source: AttributeSource,
    vector: Arc<dyn Terms>,
    term_attribute: AttributeHandle<dyn CharTermAttribute>,
    position_increment_attribute: AttributeHandle<dyn PositionIncrementAttribute>,
    max_start_offset: i32,
    offset_attribute: Option<AttributeHandle<dyn OffsetAttribute>>, // maybe null
    payload_attribute: Option<AttributeHandle<dyn PayloadAttribute>>, // maybe null
    term_chars_builder: Vec<char>, // term data here
    payloads_bytes_ref_array: Option<BytesRefArray>, // only used when payloadAttribute is non-null
    /// The nodes of the token linked lists; `TokenLL.next` is an index here.
    tokens: Vec<TokenLL>,
    first_token: Option<usize>, // the head of a linked-list
    increment_token: Option<usize>,
    initialized: bool, // lazy
}

impl fmt::Debug for TokenStreamFromTermVector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenStreamFromTermVector")
            .field("max_start_offset", &self.max_start_offset)
            .field("initialized", &self.initialized)
            .finish_non_exhaustive()
    }
}

impl TokenStreamFromTermVector {
    /// Constructor. The uninversion doesn't happen here; it's delayed till
    /// the first call to `incrementToken`.
    ///
    /// Equivalent to `new TokenStreamFromTermVector(Terms, int)`:
    /// `max_start_offset` filters tokens whose start offset is beyond it; a
    /// negative value means no limit.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the term vector has neither positions
    /// nor offsets.
    pub fn new(vector: Arc<dyn Terms>, max_start_offset: i32) -> Result<Self> {
        let max_start_offset = if max_start_offset < 0 {
            i32::MAX
        } else {
            max_start_offset
        };
        let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
        if !vector.has_positions() && !vector.has_offsets() {
            return Err(LuceneError::illegal_argument(
                "The term vector needs positions and/or offsets.",
            ));
        }
        debug_assert!(vector.has_freqs());
        let term_attribute = AttributeHandle::add(&mut source)?;
        let position_increment_attribute = AttributeHandle::add(&mut source)?;
        Ok(Self {
            source,
            vector,
            term_attribute,
            position_increment_attribute,
            max_start_offset,
            offset_attribute: None,
            payload_attribute: None,
            term_chars_builder: Vec::new(),
            payloads_bytes_ref_array: None,
            tokens: Vec::new(),
            first_token: None,
            increment_token: None,
            initialized: false,
        })
    }

    /// Equivalent to `getTermVectorTerms()`.
    pub fn get_term_vector_terms(&self) -> &Arc<dyn Terms> {
        &self.vector
    }

    // We delay initialization because we can see which attributes the
    // consumer wants, particularly payloads
    fn init(&mut self) -> Result<()> {
        debug_assert!(!self.initialized);
        let mut dp_enum_flags = POSTINGS_ENUM_POSITIONS;
        if self.vector.has_offsets() {
            dp_enum_flags |= POSTINGS_ENUM_OFFSETS;
            self.offset_attribute = Some(AttributeHandle::add(&mut self.source)?);
        }
        if self.vector.has_payloads()
            && AttributeHandle::<dyn PayloadAttribute>::get(&self.source).is_some()
        {
            dp_enum_flags |= POSTINGS_ENUM_OFFSETS | POSTINGS_ENUM_PAYLOADS; // must ask for offsets too
            self.payload_attribute = AttributeHandle::get(&self.source);
            self.payloads_bytes_ref_array =
                Some(BytesRefArray::new(Arc::new(Counter::new_counter())));
        }

        // Step 1: iterate termsEnum and create a token, placing into an array
        // of tokens by position

        self.term_chars_builder
            .reserve((self.vector.size()? as f64 * 7.0) as usize); // 7 is over-estimate of average term len

        let mut positioned_tokens = self.init_tokens_array()?;

        let mut last_position: i32 = -1;

        let mut terms_enum = self.vector.iterator()?;
        let mut dp_enum: Option<Box<dyn PostingsEnum>> = None;
        loop {
            let term_bytes_ref = match terms_enum.next()? {
                Some(term) => term.clone(),
                None => break,
            };
            // Grab the term (in same way as BytesRef.utf8ToString() but we
            // don't want a String obj)
            let term_chars: Vec<char> =
                String::from_utf8_lossy(term_bytes_ref.slice()).chars().collect();
            let term_chars_len = term_chars.len() as i32;
            let term_chars_off = self.term_chars_builder.len() as i32;
            self.term_chars_builder.extend_from_slice(&term_chars);
            // Get the positions and offsets of this term

            let mut dp = terms_enum
                .postings(dp_enum.take(), dp_enum_flags)?
                .expect("INVARIANT: a term vector term has postings"); // presumably checked by TokenSources.hasPositions earlier
            dp.next_doc()?;
            let freq = dp.freq()?;
            for _ in 0..freq {
                let mut pos = dp.next_position()?;
                let mut token = TokenLL {
                    term_chars_off,
                    term_chars_len: term_chars_len.min(i16::MAX as i32) as i16,
                    ..TokenLL::default()
                };
                if self.offset_attribute.is_some() {
                    token.start_offset = dp.start_offset();
                    if token.start_offset > self.max_start_offset {
                        continue; // filter this token out; exceeds threshold
                    }
                    token.end_offset_inc =
                        (dp.end_offset() - token.start_offset).min(i16::MAX as i32) as i16;
                    if pos == -1 {
                        pos = token.start_offset >> 3; // divide by 8
                    }
                }

                if self.payload_attribute.is_some() {
                    token.payload_index = match dp.get_payload()? {
                        None => -1,
                        Some(payload) => self
                            .payloads_bytes_ref_array
                            .as_mut()
                            .expect("INVARIANT: created with the payload attribute")
                            .append(&BytesRef::new(payload.to_vec()))?
                            as i32,
                    };
                }

                // Add token to an array indexed by position
                if positioned_tokens.len() as i32 <= pos {
                    // grow, but not 2x since we think our original length
                    // estimate is close
                    let new_len = ((pos + 1) as f32 * 1.5) as usize;
                    let mut new_positioned_tokens = vec![None; new_len];
                    let copy = (last_position + 1) as usize;
                    new_positioned_tokens[..copy].copy_from_slice(&positioned_tokens[..copy]);
                    positioned_tokens = new_positioned_tokens;
                }
                let index = self.tokens.len();
                self.tokens.push(token);
                positioned_tokens[pos as usize] =
                    Some(self.insert_into_sorted_linked_list(index, positioned_tokens[pos as usize]));

                last_position = last_position.max(pos);
            }
            dp_enum = Some(dp);
        }

        // Step 2: Link all Tokens into a linked-list and set
        // position increments as we go

        let mut prev_token_pos: i32 = -1;
        let mut prev_token: Option<usize> = None;
        for pos in 0..=last_position {
            let Some(mut token) = positioned_tokens[pos as usize] else {
                continue;
            };
            // link
            if let Some(prev) = prev_token {
                debug_assert!(self.tokens[prev].next.is_none());
                self.tokens[prev].next = Some(token); // concatenate linked-list
            } else {
                debug_assert!(self.first_token.is_none());
                self.first_token = Some(token);
            }
            // set increments
            if self.vector.has_positions() {
                self.tokens[token].position_increment = pos - prev_token_pos;
                while let Some(next) = self.tokens[token].next {
                    token = next;
                    self.tokens[token].position_increment = 0;
                }
            } else {
                self.tokens[token].position_increment = 1;
                while let Some(next) = self.tokens[token].next {
                    let prev = token; // Java: prevToken = token
                    token = next;
                    if self.tokens[prev].start_offset == self.tokens[token].start_offset {
                        self.tokens[token].position_increment = 0;
                    } else {
                        self.tokens[token].position_increment = 1;
                    }
                }
            }
            prev_token_pos = pos;
            prev_token = Some(token);
        }

        self.initialized = true;
        Ok(())
    }

    fn init_tokens_array(&self) -> Result<Vec<Option<usize>>> {
        // Estimate the number of position slots we need from term stats.
        // We use this to estimate the size of an array.
        let sum_total_term_freq = self.vector.sum_total_term_freq()? as i32;
        debug_assert!(sum_total_term_freq != -1);

        let original_position_estimate = (sum_total_term_freq as f64 * 1.5) as i32; // less than 1 in 10 docs exceed this

        // This estimate is based on maxStartOffset. Err on the side of this
        // being larger than needed.
        let offset_limit_position_estimate = (self.max_start_offset as f64 / 5.0) as i32;

        // Take the smaller of the two estimates, but no smaller than 64
        Ok(vec![
            None;
            64.max(original_position_estimate.min(offset_limit_position_estimate)) as usize
        ])
    }

    /// Equivalent to `TokenLL.insertIntoSortedLinkedList(TokenLL)`: inserts
    /// the token at `this` into the list headed by `head`, answering the
    /// new head.
    fn insert_into_sorted_linked_list(&mut self, this: usize, head: Option<usize>) -> usize {
        debug_assert!(self.tokens[this].next.is_none());
        let Some(head) = head else {
            return this;
        };
        if self.tokens[this].compare_offsets(&self.tokens[head]) <= 0 {
            self.tokens[this].next = Some(head);
            return this;
        }
        let mut prev = head;
        while let Some(next) = self.tokens[prev].next {
            if self.tokens[this].compare_offsets(&self.tokens[next]) > 0 {
                prev = next;
            } else {
                break;
            }
        }
        self.tokens[this].next = self.tokens[prev].next;
        self.tokens[prev].next = Some(this);
        head
    }
}

impl TokenStream for TokenStreamFromTermVector {
    fn reset(&mut self) -> Result<()> {
        self.increment_token = None;
        Ok(())
    }

    fn increment_token(&mut self) -> Result<bool> {
        match self.increment_token {
            None => {
                if !self.initialized {
                    self.init()?;
                    debug_assert!(self.initialized);
                }
                self.increment_token = self.first_token;
                if self.increment_token.is_none() {
                    return Ok(false);
                }
            }
            Some(current) => match self.tokens[current].next {
                Some(next) => self.increment_token = Some(next),
                None => return Ok(false),
            },
        }
        let token = self.tokens[self.increment_token.expect("INVARIANT: set above")];
        self.source.clear_attributes();
        {
            let off = token.term_chars_off as usize;
            let len = token.term_chars_len as usize;
            self.term_attribute
                .borrow_mut()
                .copy_buffer(&self.term_chars_builder, off, len);
        }
        self.position_increment_attribute
            .borrow_mut()
            .set_position_increment(token.position_increment);
        if let Some(offset_attribute) = &self.offset_attribute {
            offset_attribute.borrow_mut().set_offset(
                token.start_offset,
                token.start_offset + token.end_offset_inc as i32,
            );
        }
        if let Some(payload_attribute) = &self.payload_attribute {
            if token.payload_index == -1 {
                payload_attribute.borrow_mut().set_payload(None);
            } else {
                let payload = self
                    .payloads_bytes_ref_array
                    .as_ref()
                    .expect("INVARIANT: created with the payload attribute")
                    .get(token.payload_index as usize)?;
                payload_attribute.borrow_mut().set_payload(Some(payload));
            }
        }
        Ok(true)
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

/// Equivalent to the private nested class `TokenStreamFromTermVector.TokenLL`.
#[derive(Debug, Clone, Copy, Default)]
struct TokenLL {
    term_chars_off: i32, // see termCharsBuilder
    term_chars_len: i16,

    position_increment: i32,
    start_offset: i32,
    end_offset_inc: i16, // add to startOffset to get endOffset
    payload_index: i32,

    next: Option<usize>,
}

impl TokenLL {
    /// Equivalent to `compareOffsets(TokenLL)`.
    fn compare_offsets(&self, token_b: &TokenLL) -> i32 {
        let mut cmp = self.start_offset.cmp(&token_b.start_offset) as i32;
        if cmp == 0 {
            cmp = self.end_offset_inc.cmp(&token_b.end_offset_inc) as i32;
        }
        cmp
    }
}