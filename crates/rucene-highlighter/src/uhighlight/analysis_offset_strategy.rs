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

//! Port of `AnalysisOffsetStrategy.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use std::fmt;
use std::sync::Arc;

use rucene::analysis::tokenattributes::{OffsetAttribute, PositionIncrementAttribute};
use rucene::analysis::{Analyzer, AttributeHandle, SharedTokenStream, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeSource;

use crate::java::{substring, units};
use crate::uhighlight::uh_components::UHComponents;
use crate::uhighlight::unified_highlighter::{OffsetSource, MULTIVAL_SEP_CHAR};

/// Provides a base class for analysis based offset strategies to extend
/// from. Requires an Analyzer and provides an override-able method for
/// altering how the TokenStream is created.
///
/// Equivalent to the state and the concrete methods of the abstract class
/// `org.apache.lucene.search.uhighlight.AnalysisOffsetStrategy`; its
/// subclasses hold it and reach it through `Deref`.
pub struct AnalysisOffsetStrategy {
    pub(crate) components: UHComponents,
    /// Equivalent to the protected final field `analyzer`.
    pub analyzer: Arc<dyn Analyzer>,
}

impl fmt::Debug for AnalysisOffsetStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnalysisOffsetStrategy")
            .field("components", &self.components)
            .finish_non_exhaustive()
    }
}

impl AnalysisOffsetStrategy {
    /// Equivalent to `new AnalysisOffsetStrategy(UHComponents, Analyzer)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the analyzer's offset gap is not 1.
    pub fn new(components: UHComponents, analyzer: Arc<dyn Analyzer>) -> Result<Self> {
        if analyzer.get_offset_gap(Some(components.field())) != 1 {
            // note: 1 is the default. It is RARELY changed.
            return Err(LuceneError::illegal_argument(format!(
                "offset gap of the provided analyzer should be 1 (field {})",
                components.field()
            )));
        }
        Ok(Self {
            components,
            analyzer,
        })
    }

    /// Equivalent to the final `getOffsetSource()`.
    pub fn get_offset_source(&self) -> OffsetSource {
        OffsetSource::ANALYSIS
    }

    /// Equivalent to the protected `tokenStream(String)`.
    ///
    /// # Errors
    ///
    /// What the analyzer raises.
    pub fn token_stream(&self, content: &str) -> Result<Box<dyn TokenStream>> {
        let field = self.components.field();
        // If there is no splitChar in content then we needn't wrap:
        let content_units = units(content);
        let Some(split_char_idx) = content_units.iter().position(|&c| c == MULTIVAL_SEP_CHAR)
        else {
            return Ok(Box::new(SharedTokenStream::new(
                self.analyzer.token_stream_from_str(field, content)?,
            )));
        };

        let sub_token_stream = self
            .analyzer
            .token_stream_from_str(field, &substring(&content_units, 0, split_char_idx as i32)?)?;

        Ok(Box::new(MultiValueTokenStream::new(
            sub_token_stream,
            field,
            Arc::clone(&self.analyzer),
            content_units,
            MULTIVAL_SEP_CHAR,
            split_char_idx as i32,
        )?))
    }
}

/// Wraps an `Analyzer` and string text that represents multiple values
/// delimited by a specified character. This exposes a TokenStream that
/// matches what would get indexed considering the
/// `Analyzer.getPositionIncrementGap(String)`. Currently this assumes
/// `Analyzer.getOffsetGap(String)` is 1; an exception will be thrown if it
/// isn't.
///
/// It would be more orthogonal for this to be an Analyzer since we're
/// wrapping an Analyzer but doing so seems like more work. The underlying
/// components see a Reader not a String -- and the String is easy to split
/// up without redundant buffering.
///
/// Equivalent to the private nested class
/// `AnalysisOffsetStrategy.MultiValueTokenStream`, a `TokenFilter` whose
/// input is the analyzer's reused stream.
pub(crate) struct MultiValueTokenStream {
    source: AttributeSource,
    input: Arc<AtomicRefCell<dyn TokenStream>>,
    field_name: String,
    index_analyzer: Arc<dyn Analyzer>,
    content: Vec<u16>,
    split_char: u16,

    pos_inc_att: AttributeHandle<dyn PositionIncrementAttribute>,
    offset_att: AttributeHandle<dyn OffsetAttribute>,

    start_val_idx: i32,
    end_val_idx: i32,
    remaining_pos_inc: i32,
}

impl fmt::Debug for MultiValueTokenStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MultiValueTokenStream")
            .field("field_name", &self.field_name)
            .field("start_val_idx", &self.start_val_idx)
            .field("end_val_idx", &self.end_val_idx)
            .finish_non_exhaustive()
    }
}

impl MultiValueTokenStream {
    fn new(
        sub_token_stream: Arc<AtomicRefCell<dyn TokenStream>>,
        field_name: &str,
        index_analyzer: Arc<dyn Analyzer>,
        content: Vec<u16>,
        split_char: u16,
        split_char_idx: i32,
    ) -> Result<Self> {
        // subTokenStream is already initialized to operate on the first value
        let mut source = AttributeSource::new_from(sub_token_stream.borrow().attribute_source());
        let pos_inc_att = AttributeHandle::add(&mut source)?;
        let offset_att = AttributeHandle::add(&mut source)?;
        Ok(Self {
            source,
            input: sub_token_stream,
            field_name: field_name.to_string(),
            index_analyzer,
            content,
            split_char,
            pos_inc_att,
            offset_att,
            start_val_idx: 0,
            end_val_idx: split_char_idx,
            remaining_pos_inc: 0,
        })
    }

    fn adjust_offsets(&self) {
        let mut att = self.offset_att.borrow_mut();
        let (start, end) = (att.start_offset(), att.end_offset());
        att.set_offset(self.start_val_idx + start, self.start_val_idx + end);
    }
}

impl TokenStream for MultiValueTokenStream {
    fn reset(&mut self) -> Result<()> {
        if self.start_val_idx != 0 {
            return Err(LuceneError::illegal_state(
                "This TokenStream wasn't developed to be re-used.",
            ));
            // ... although we could if a need for it arises.
        }
        self.input.borrow_mut().reset()
    }

    fn increment_token(&mut self) -> Result<bool> {
        loop {
            if self.input.borrow_mut().increment_token()? {
                // Position tracking:
                if self.remaining_pos_inc > 0 {
                    // usually true first token of additional values
                    // (not first val)
                    let mut att = self.pos_inc_att.borrow_mut();
                    let inc = att.get_position_increment();
                    att.set_position_increment(self.remaining_pos_inc + inc);
                    self.remaining_pos_inc = 0; // reset
                }
                // Offset tracking:
                self.adjust_offsets();
                return Ok(true);
            }

            if self.end_val_idx == self.content.len() as i32 {
                // no more
                return Ok(false);
            }

            self.input.borrow_mut().end()?; // might adjust position increment
            self.remaining_pos_inc += self.pos_inc_att.borrow().get_position_increment();
            self.input.borrow_mut().close()?;
            self.remaining_pos_inc += self
                .index_analyzer
                .get_position_increment_gap(Some(&self.field_name));

            // Get new tokenStream based on next segment divided by the
            // splitChar
            self.start_val_idx = self.end_val_idx + 1;
            self.end_val_idx = match self.content[self.start_val_idx as usize..]
                .iter()
                .position(|&c| c == self.split_char)
            {
                Some(p) => self.start_val_idx + p as i32,
                None => self.content.len() as i32, // EOF
            };
            let value = substring(&self.content, self.start_val_idx, self.end_val_idx)?;
            let token_stream = self
                .index_analyzer
                .token_stream_from_str(&self.field_name, &value)?;
            if !Arc::ptr_eq(&token_stream, &self.input) {
                // (input is defined in TokenFilter set in the constructor)
                // This is a grave problem; it probably won't happen. If it
                // happens we can avoid it by subclassing analyzer and
                // installing a ReuseStrategy that "re-uses" components
                // regardless of the field name.
                return Err(LuceneError::illegal_state(format!(
                    "Require TokenStream re-use.  Unsupported re-use strategy?: {:?}",
                    self.index_analyzer.analyzer_state().reuse_strategy()
                )));
            }
            token_stream.borrow_mut().reset()?;
        } // while loop to increment token of this new value
    }

    fn end(&mut self) -> Result<()> {
        self.input.borrow_mut().end()?;
        // Offset tracking:
        self.adjust_offsets();
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        self.input.borrow_mut().close()
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}
