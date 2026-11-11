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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextCodec`.

use rucene::codecs::{
    Codec, CompoundFormat, DocValuesFormat, FieldInfosFormat, KnnVectorsFormat, LiveDocsFormat,
    NormsFormat, PointsFormat, PostingsFormat, SegmentInfoFormat, StoredFieldsFormat,
    TermVectorsFormat,
};

use super::{
    SimpleTextCompoundFormat, SimpleTextDocValuesFormat, SimpleTextFieldInfosFormat,
    SimpleTextKnnVectorsFormat, SimpleTextLiveDocsFormat, SimpleTextNormsFormat,
    SimpleTextPointsFormat, SimpleTextPostingsFormat, SimpleTextSegmentInfoFormat,
    SimpleTextStoredFieldsFormat, SimpleTextTermVectorsFormat,
};

/// Plain text index format.
///
/// **FOR RECREATIONAL USE ONLY**
///
/// Port of `org.apache.lucene.codecs.simpletext.SimpleTextCodec`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SimpleTextCodec {
    postings: SimpleTextPostingsFormat,
    stored_fields: SimpleTextStoredFieldsFormat,
    segment_infos: SimpleTextSegmentInfoFormat,
    field_infos_format: SimpleTextFieldInfosFormat,
    vectors_format: SimpleTextTermVectorsFormat,
    norms_format: SimpleTextNormsFormat,
    live_docs: SimpleTextLiveDocsFormat,
    dv_format: SimpleTextDocValuesFormat,
    compound_format: SimpleTextCompoundFormat,
    points_format: SimpleTextPointsFormat,
    knn_vectors_format: SimpleTextKnnVectorsFormat,
}

impl SimpleTextCodec {
    /// Creates the codec, named `SimpleText`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Codec for SimpleTextCodec {
    fn name(&self) -> &str {
        "SimpleText"
    }

    fn postings_format(&self) -> &dyn PostingsFormat {
        &self.postings
    }

    fn doc_values_format(&self) -> &dyn DocValuesFormat {
        &self.dv_format
    }

    fn stored_fields_format(&self) -> &dyn StoredFieldsFormat {
        &self.stored_fields
    }

    fn term_vectors_format(&self) -> &dyn TermVectorsFormat {
        &self.vectors_format
    }

    fn field_infos_format(&self) -> &dyn FieldInfosFormat {
        &self.field_infos_format
    }

    fn segment_info_format(&self) -> &dyn SegmentInfoFormat {
        &self.segment_infos
    }

    fn norms_format(&self) -> &dyn NormsFormat {
        &self.norms_format
    }

    fn live_docs_format(&self) -> &dyn LiveDocsFormat {
        &self.live_docs
    }

    fn compound_format(&self) -> &dyn CompoundFormat {
        &self.compound_format
    }

    fn points_format(&self) -> &dyn PointsFormat {
        &self.points_format
    }

    fn knn_vectors_format(&self) -> &dyn KnnVectorsFormat {
        &self.knn_vectors_format
    }
}
