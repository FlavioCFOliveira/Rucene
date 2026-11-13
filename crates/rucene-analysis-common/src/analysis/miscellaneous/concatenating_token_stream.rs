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
//! Concatenates several token streams into one, ported from
//! `org.apache.lucene.analysis.miscellaneous.ConcatenatingTokenStream`.

use std::sync::Arc;

use rucene::analysis::{OffsetAttribute, PositionIncrementAttribute, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::AttributeSource;

/// A TokenStream that takes an array of input TokenStreams as sources, and
/// concatenates them together.
///
/// Offsets from the second and subsequent sources are incremented to behave
/// as if all the inputs were from a single source.
///
/// All of the input TokenStreams must have the same attribute
/// implementations.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.ConcatenatingTokenStream`.
#[derive(Debug)]
pub struct ConcatenatingTokenStream {
    source: AttributeSource,
    sources: Vec<Box<dyn TokenStream>>,
    current_source: usize,
    offset_increment: i32,
    initial_position_increment: i32,
}

fn offset_att_of(source: &dyn TokenStream) -> (i32, i32) {
    let att = source
        .attribute_source()
        .get_attribute_interface::<dyn OffsetAttribute>()
        .expect("INVARIANT: the constructor added the OffsetAttribute");
    (att.start_offset(), att.end_offset())
}

fn pos_inc_of(source: &dyn TokenStream) -> i32 {
    source
        .attribute_source()
        .get_attribute_interface::<dyn PositionIncrementAttribute>()
        .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
        .get_position_increment()
}

impl ConcatenatingTokenStream {
    /// Create a new ConcatenatingTokenStream from a set of inputs.
    ///
    /// Equivalent to `ConcatenatingTokenStream(TokenStream...)`:
    /// `super(combineSources(sources))`, then the `OffsetAttribute` and
    /// `PositionIncrementAttribute` are added to this stream and to every
    /// source.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` "Attempted to concatenate TokenStreams with
    /// different attribute types" when the sources' attributes cannot be
    /// combined, and what the attribute factories raise.
    ///
    /// # Panics
    ///
    /// With no source, where Java's `sources[0]` throws
    /// `ArrayIndexOutOfBoundsException`.
    pub fn new(sources: Vec<Box<dyn TokenStream>>) -> Result<Self> {
        let mut sources = sources;
        let mut source = Self::combine_sources(&sources)?;
        source.add_attribute_interface::<dyn OffsetAttribute>()?;
        source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
        for input in sources.iter_mut() {
            let input = input.attribute_source_mut();
            input.add_attribute_interface::<dyn OffsetAttribute>()?;
            input.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
        }
        Ok(Self {
            source,
            sources,
            current_source: 0,
            offset_increment: 0,
            initial_position_increment: 1,
        })
    }

    /// Equivalent to the private static `combineSources(TokenStream...)`.
    fn combine_sources(sources: &[Box<dyn TokenStream>]) -> Result<AttributeSource> {
        let mut base = sources[0].attribute_source().clone_attributes();
        let combine = |base: &mut AttributeSource| -> Result<()> {
            for input in &sources[1..] {
                let classes: Vec<_> = input.attribute_source().attribute_classes().collect();
                for class in classes {
                    base.add_attribute_by_id(class)?;
                }
                // check attributes can be captured
                input.attribute_source().copy_to(base)?;
            }
            Ok(())
        };
        match combine(&mut base) {
            Ok(()) => Ok(base),
            Err(e) if e.is_illegal_argument_exception() => {
                Err(LuceneError::illegal_argument_with_cause(
                    "Attempted to concatenate TokenStreams with different attribute types",
                    Arc::new(e),
                ))
            }
            Err(e) => Err(e),
        }
    }
}

impl TokenStream for ConcatenatingTokenStream {
    /// Equivalent to `ConcatenatingTokenStream.incrementToken()`.
    fn increment_token(&mut self) -> Result<bool> {
        let mut new_source = false;
        while !self.sources[self.current_source].increment_token()? {
            if self.current_source >= self.sources.len() - 1 {
                return Ok(false);
            }
            self.sources[self.current_source].end()?;
            self.initial_position_increment =
                pos_inc_of(self.sources[self.current_source].as_ref());
            let (_, end_offset) = offset_att_of(self.sources[self.current_source].as_ref());
            self.offset_increment += end_offset;
            self.current_source += 1;
            new_source = true;
        }

        self.source.clear_attributes();
        self.sources[self.current_source]
            .attribute_source()
            .copy_to(&self.source)?;
        {
            let mut offset_att = self
                .source
                .get_attribute_interface_mut::<dyn OffsetAttribute>()
                .expect("INVARIANT: the constructor added the OffsetAttribute");
            let start_offset = offset_att.start_offset() + self.offset_increment;
            let end_offset = offset_att.end_offset() + self.offset_increment;
            offset_att.set_offset(start_offset, end_offset);
        }
        if new_source {
            let mut pos_inc_att = self
                .source
                .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
                .expect("INVARIANT: the constructor added the PositionIncrementAttribute");
            let pos_inc = pos_inc_att.get_position_increment();
            pos_inc_att.set_position_increment(pos_inc + self.initial_position_increment);
        }

        Ok(true)
    }

    /// Equivalent to `ConcatenatingTokenStream.end()`.
    fn end(&mut self) -> Result<()> {
        self.sources[self.current_source].end()?;
        let (_, end_offset) = offset_att_of(self.sources[self.current_source].as_ref());
        let final_offset = end_offset + self.offset_increment;
        let final_pos_inc = pos_inc_of(self.sources[self.current_source].as_ref());
        self.source.end_attributes();
        self.source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("INVARIANT: the constructor added the OffsetAttribute")
            .set_offset(final_offset, final_offset);
        self.source
            .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
            .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
            .set_position_increment(final_pos_inc);
        Ok(())
    }

    /// Equivalent to `ConcatenatingTokenStream.reset()`.
    fn reset(&mut self) -> Result<()> {
        for source in self.sources.iter_mut() {
            source.reset()?;
        }
        self.current_source = 0;
        self.offset_increment = 0;
        Ok(())
    }

    /// Equivalent to `ConcatenatingTokenStream.close()`: `IOUtils.close(sources)`,
    /// which closes every source and throws the first exception with the
    /// others suppressed, then `super.close()`.
    fn close(&mut self) -> Result<()> {
        let mut first: Option<LuceneError> = None;
        for source in self.sources.iter_mut() {
            if let Err(e) = source.close() {
                match first.as_mut() {
                    None => first = Some(e),
                    Some(first) => first.add_suppressed(e),
                }
            }
        }
        match first {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}
