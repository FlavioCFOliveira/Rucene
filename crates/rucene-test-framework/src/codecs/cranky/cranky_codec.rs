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

//! Port of `org.apache.lucene.tests.codecs.cranky.CrankyCodec`.

use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use rucene::codecs::{
    Codec, CompoundFormat, DocValuesFormat, FieldInfosFormat, KnnVectorsFormat, LiveDocsFormat,
    NormsFormat, PointsFormat, PostingsFormat, SegmentInfoFormat, StoredFieldsFormat,
    TermVectorsFormat,
};

use super::cranky_compound_format::CrankyCompoundFormat;
use super::cranky_doc_values_format::CrankyDocValuesFormat;
use super::cranky_field_infos_format::CrankyFieldInfosFormat;
use super::cranky_live_docs_format::CrankyLiveDocsFormat;
use super::cranky_norms_format::CrankyNormsFormat;
use super::cranky_points_format::CrankyPointsFormat;
use super::cranky_postings_format::CrankyPostingsFormat;
use super::cranky_segment_info_format::CrankySegmentInfoFormat;
use super::cranky_stored_fields_format::CrankyStoredFieldsFormat;
use super::cranky_term_vectors_format::CrankyTermVectorsFormat;
use crate::codecs::CodecFormat;

/// Codec for testing that throws random `IOException`s.
///
/// Port of `CrankyCodec` (`extends FilterCodec`): it impersonates the
/// wrapped codec (same name, same file formats, so it needs no SPI entry) and
/// wraps every format but the KNN-vectors one in a cranky format drawing from
/// `random`.
///
/// Java builds a new cranky format on every `…Format()` call; the formats
/// hold nothing but the delegate and the shared `random`, so the port builds
/// them once and lends them, which draws the same numbers from the same
/// `random`.
pub struct CrankyCodec {
    delegate: Arc<dyn Codec>,
    random: Arc<dyn Random>,
    doc_values: CrankyDocValuesFormat,
    field_infos: CrankyFieldInfosFormat,
    live_docs: CrankyLiveDocsFormat,
    norms: CrankyNormsFormat,
    postings: CrankyPostingsFormat,
    segment_info: CrankySegmentInfoFormat,
    stored_fields: CrankyStoredFieldsFormat,
    term_vectors: CrankyTermVectorsFormat,
    compound: CrankyCompoundFormat,
    points: CrankyPointsFormat,
}

impl CrankyCodec {
    /// `new CrankyCodec(delegate, random)`: wraps the provided codec with
    /// crankiness. Try passing Asserting for the most fun.
    pub fn new(delegate: Arc<dyn Codec>, random: Arc<dyn Random>) -> Self {
        let r = || Arc::clone(&random);
        CrankyCodec {
            doc_values: CrankyDocValuesFormat::new(CodecFormat::new(Arc::clone(&delegate)), r()),
            field_infos: CrankyFieldInfosFormat::new(CodecFormat::new(Arc::clone(&delegate)), r()),
            live_docs: CrankyLiveDocsFormat::new(CodecFormat::new(Arc::clone(&delegate)), r()),
            norms: CrankyNormsFormat::new(CodecFormat::new(Arc::clone(&delegate)), r()),
            postings: CrankyPostingsFormat::new(CodecFormat::new(Arc::clone(&delegate)), r()),
            segment_info: CrankySegmentInfoFormat::new(
                CodecFormat::new(Arc::clone(&delegate)),
                r(),
            ),
            stored_fields: CrankyStoredFieldsFormat::new(
                CodecFormat::new(Arc::clone(&delegate)),
                r(),
            ),
            term_vectors: CrankyTermVectorsFormat::new(
                CodecFormat::new(Arc::clone(&delegate)),
                r(),
            ),
            compound: CrankyCompoundFormat::new(CodecFormat::new(Arc::clone(&delegate)), r()),
            points: CrankyPointsFormat::new(CodecFormat::new(Arc::clone(&delegate)), r()),
            delegate,
            random,
        }
    }

    /// The `random` the formats draw from.
    pub fn random(&self) -> &Arc<dyn Random> {
        &self.random
    }

    /// `FilterCodec.delegate`.
    pub fn delegate(&self) -> &Arc<dyn Codec> {
        &self.delegate
    }
}

impl fmt::Display for CrankyCodec {
    /// `CrankyCodec.toString()` (`CrankyCodec.java:95-98`): the delegate
    /// through its own `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Cranky({})", &*self.delegate as &dyn Codec)
    }
}

impl fmt::Debug for CrankyCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Codec for CrankyCodec {
    fn name(&self) -> &str {
        self.delegate.name()
    }

    fn to_java_string(&self) -> String {
        self.to_string()
    }

    fn postings_format(&self) -> &dyn PostingsFormat {
        &self.postings
    }

    fn doc_values_format(&self) -> &dyn DocValuesFormat {
        &self.doc_values
    }

    fn stored_fields_format(&self) -> &dyn StoredFieldsFormat {
        &self.stored_fields
    }

    fn term_vectors_format(&self) -> &dyn TermVectorsFormat {
        &self.term_vectors
    }

    fn field_infos_format(&self) -> &dyn FieldInfosFormat {
        &self.field_infos
    }

    fn segment_info_format(&self) -> &dyn SegmentInfoFormat {
        &self.segment_info
    }

    fn norms_format(&self) -> &dyn NormsFormat {
        &self.norms
    }

    fn live_docs_format(&self) -> &dyn LiveDocsFormat {
        &self.live_docs
    }

    fn compound_format(&self) -> &dyn CompoundFormat {
        &self.compound
    }

    fn points_format(&self) -> &dyn PointsFormat {
        &self.points
    }

    /// Not overridden in Java: the delegate's.
    fn knn_vectors_format(&self) -> &dyn KnnVectorsFormat {
        self.delegate.knn_vectors_format()
    }
}
