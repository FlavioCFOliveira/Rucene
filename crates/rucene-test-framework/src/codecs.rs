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

//! Port of `org.apache.lucene.tests.codecs`: the test codecs Lucene's tests
//! randomize over.
//!
//! * [`asserting`]: acts like the default codec but with additional asserts.
//! * [`blockterms`]: postings formats over `lucene-codecs`' block terms
//!   dictionary and its terms indexes.
//! * [`bloom`]: a Bloom-filtered postings format with tiny filters.
//! * [`cheapbastard`]: the default codec with a low-memory postings format.
//! * [`compressing`]: the default codec with configurable compressing stored
//!   fields and term vectors.
//! * [`cranky`]: wraps a codec and throws random `IOException`s.
//! * [`mockrandom`]: a postings format that picks its terms dictionary at
//!   random, per segment.
//! * [`ramonly`]: a postings format that keeps the postings in RAM.
//! * [`uniformsplit`]: the `UniformSplit` postings formats with ROT13 block
//!   encoding.
//! * [`vector`]: the default codec with a configurable HNSW `M`.
//!
//! The `asserting` and `cranky` families share one skeleton: a
//! `FilterCodec` whose every format wraps another codec's format of the same
//! kind. [`CodecFormat`] is that delegation: a handle on the format a codec
//! hands out, which a wrapper format holds as its `in` (Asserting) or
//! `delegate` (Cranky) field. Java keeps the format object itself; Rust's
//! [`Codec`] lends its formats (`&dyn PostingsFormat`), so the handle keeps the
//! codec alive and re-borrows the format on use, which reaches the same object
//! every time.

pub mod asserting;
pub mod blockterms;
pub mod bloom;
pub mod cheapbastard;
pub mod compressing;
pub mod cranky;
pub mod mockrandom;
pub mod ramonly;
pub mod uniformsplit;
pub mod vector;

use std::fmt;
use std::sync::Arc;

use rucene::codecs::{
    Codec, CompoundFormat, DocValuesFormat, FieldInfosFormat, KnnVectorsFormat, LiveDocsFormat,
    NormsFormat, PointsFormat, PostingsFormat, SegmentInfoFormat, StoredFieldsFormat,
    TermVectorsFormat,
};

/// One kind of format a [`Codec`] hands out, and the accessor that lends it
/// (`codec.postingsFormat()`, ...).
pub trait FormatKind: 'static {
    /// The format's trait object, borrowed for `'a`.
    type Format<'a>: ?Sized + 'a;
    /// The accessor.
    fn select<'a>(codec: &'a dyn Codec) -> &'a Self::Format<'a>;
}

/// A handle on one format of a codec: the codec, kept alive, and the kind of
/// format, whose accessor lends the format on use.
///
/// The shared delegation of the asserting and cranky formats (see the
/// [module documentation](self)).
pub struct CodecFormat<K: FormatKind> {
    codec: Arc<dyn Codec>,
    kind: std::marker::PhantomData<K>,
}

impl<K: FormatKind> CodecFormat<K> {
    /// The format of kind `K` of `codec`.
    pub fn new(codec: Arc<dyn Codec>) -> Self {
        CodecFormat {
            codec,
            kind: std::marker::PhantomData,
        }
    }

    /// The format.
    pub fn get(&self) -> &K::Format<'_> {
        K::select(&*self.codec)
    }

    /// The codec the format comes from.
    pub fn codec(&self) -> &Arc<dyn Codec> {
        &self.codec
    }
}

impl<K: FormatKind> Clone for CodecFormat<K> {
    fn clone(&self) -> Self {
        CodecFormat::new(Arc::clone(&self.codec))
    }
}

impl<K: FormatKind> fmt::Debug for CodecFormat<K>
where
    for<'a> K::Format<'a>: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.get(), f)
    }
}

macro_rules! format_kinds {
    ($($(#[$doc:meta])* $kind:ident => $format:ident, $method:ident;)*) => {
        $(
            $(#[$doc])*
            #[derive(Debug, Clone, Copy)]
            pub struct $kind;

            impl FormatKind for $kind {
                type Format<'a> = dyn $format + 'a;
                fn select<'a>(codec: &'a dyn Codec) -> &'a Self::Format<'a> {
                    codec.$method()
                }
            }
        )*
    };
}

format_kinds! {
    /// `codec.postingsFormat()`.
    PostingsKind => PostingsFormat, postings_format;
    /// `codec.docValuesFormat()`.
    DocValuesKind => DocValuesFormat, doc_values_format;
    /// `codec.storedFieldsFormat()`.
    StoredFieldsKind => StoredFieldsFormat, stored_fields_format;
    /// `codec.termVectorsFormat()`.
    TermVectorsKind => TermVectorsFormat, term_vectors_format;
    /// `codec.fieldInfosFormat()`.
    FieldInfosKind => FieldInfosFormat, field_infos_format;
    /// `codec.segmentInfoFormat()`.
    SegmentInfoKind => SegmentInfoFormat, segment_info_format;
    /// `codec.normsFormat()`.
    NormsKind => NormsFormat, norms_format;
    /// `codec.liveDocsFormat()`.
    LiveDocsKind => LiveDocsFormat, live_docs_format;
    /// `codec.compoundFormat()`.
    CompoundKind => CompoundFormat, compound_format;
    /// `codec.pointsFormat()`.
    PointsKind => PointsFormat, points_format;
    /// `codec.knnVectorsFormat()`.
    KnnVectorsKind => KnnVectorsFormat, knn_vectors_format;
}

/// The `Codec` methods of a codec that forwards them to the
/// [`FilterCodec`](rucene::codecs::FilterCodec) in its `$field`.
macro_rules! forward_codec {
    ($field:ident) => {
        fn name(&self) -> &str {
            self.$field.name()
        }
        fn postings_format(&self) -> &dyn rucene::codecs::PostingsFormat {
            self.$field.postings_format()
        }
        fn doc_values_format(&self) -> &dyn rucene::codecs::DocValuesFormat {
            self.$field.doc_values_format()
        }
        fn stored_fields_format(&self) -> &dyn rucene::codecs::StoredFieldsFormat {
            self.$field.stored_fields_format()
        }
        fn term_vectors_format(&self) -> &dyn rucene::codecs::TermVectorsFormat {
            self.$field.term_vectors_format()
        }
        fn field_infos_format(&self) -> &dyn rucene::codecs::FieldInfosFormat {
            self.$field.field_infos_format()
        }
        fn segment_info_format(&self) -> &dyn rucene::codecs::SegmentInfoFormat {
            self.$field.segment_info_format()
        }
        fn norms_format(&self) -> &dyn rucene::codecs::NormsFormat {
            self.$field.norms_format()
        }
        fn live_docs_format(&self) -> &dyn rucene::codecs::LiveDocsFormat {
            self.$field.live_docs_format()
        }
        fn compound_format(&self) -> &dyn rucene::codecs::CompoundFormat {
            self.$field.compound_format()
        }
        fn points_format(&self) -> &dyn rucene::codecs::PointsFormat {
            self.$field.points_format()
        }
        fn knn_vectors_format(&self) -> &dyn rucene::codecs::KnnVectorsFormat {
            self.$field.knn_vectors_format()
        }
    };
}
pub(crate) use forward_codec;
