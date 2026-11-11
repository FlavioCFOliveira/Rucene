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

//! Port of `org.apache.lucene.tests.codecs.cheapbastard.CheapBastardCodec`.

use std::fmt;
use std::sync::Arc;

use rucene::codecs::{Codec, FilterCodec, PostingsFormat};

use crate::codecs::forward_codec;
use crate::util::TestUtil;

/// The default codec with the postings format
/// `TestUtil.getDefaultPostingsFormat(100, 200)` (term blocks of 100 to 200
/// entries) for every field.
///
/// Port of `CheapBastardCodec` (`extends FilterCodec`, named `CheapBastard`).
pub struct CheapBastardCodec {
    inner: FilterCodec,
}

/// A shared postings format, lent as the codec's own.
struct SharedPostings(Arc<dyn PostingsFormat>);

impl fmt::Debug for SharedPostings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.0, f)
    }
}

impl PostingsFormat for SharedPostings {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn fields_consumer<'a>(
        &self,
        state: &rucene::codecs::SegmentWriteState<'a>,
    ) -> rucene::error::Result<Box<dyn rucene::codecs::FieldsConsumer + 'a>> {
        self.0.fields_consumer(state)
    }

    fn fields_producer<'a>(
        &self,
        state: &rucene::codecs::SegmentReadState<'a>,
    ) -> rucene::error::Result<Box<dyn rucene::codecs::FieldsProducer>> {
        self.0.fields_producer(state)
    }
}

impl CheapBastardCodec {
    /// `new CheapBastardCodec()`.
    pub fn new() -> Self {
        CheapBastardCodec {
            inner: FilterCodec::new("CheapBastard", TestUtil::get_default_codec())
                .with_postings_format(SharedPostings(TestUtil::get_default_postings_format_with(
                    100, 200,
                ))),
        }
    }
}

impl Default for CheapBastardCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for CheapBastardCodec {
    /// `Codec.toString()`: the name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CheapBastard")
    }
}

impl Codec for CheapBastardCodec {
    forward_codec!(inner);
}
