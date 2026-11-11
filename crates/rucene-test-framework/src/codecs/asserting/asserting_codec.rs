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

//! Port of `org.apache.lucene.tests.codecs.asserting.AssertingCodec`.

use std::fmt;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::Arc;

use randomizedtesting::jdk::exception::{panic_message, QuietPanics, ILLEGAL_STATE_EXCEPTION};
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::RandomizedContext;
use rucene::codecs::{
    Codec, CompoundFormat, DocValuesFormat, FieldInfosFormat, KnnVectorsFormat, LiveDocsFormat,
    NormsFormat, PerFieldDocValuesFormat, PerFieldKnnVectorsFormat, PerFieldPostingsFormat,
    PointsFormat, PostingsFormat, SegmentInfoFormat, StoredFieldsFormat, TermVectorsFormat,
};
use rucene::error::{LuceneError, LuceneErrorKind, Result};

use super::{
    AssertingDocValuesFormat, AssertingKnnVectorsFormat, AssertingLiveDocsFormat,
    AssertingNormsFormat, AssertingPointsFormat, AssertingPostingsFormat,
    AssertingStoredFieldsFormat, AssertingTermVectorsFormat,
};
use crate::util::lucene_test_case::{SuppressAssertingFormats, SUPPRESS_ASSERTING_FORMATS};
use crate::util::TestUtil;

/// Port of the enum `AssertingCodec.Format`: the asserting format types that
/// can be suppressed (`@SuppressAssertingFormats`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Format {
    /// `STORED_FIELDS`.
    StoredFields,
    /// `TERM_VECTORS`.
    TermVectors,
    /// `NORMS`.
    Norms,
    /// `LIVE_DOCS`.
    LiveDocs,
    /// `POINTS`.
    Points,
    /// `KNN_VECTORS`.
    KnnVectors,
}

impl fmt::Display for Format {
    /// `Enum.toString()`: the constant's name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Format::StoredFields => "STORED_FIELDS",
            Format::TermVectors => "TERM_VECTORS",
            Format::Norms => "NORMS",
            Format::LiveDocs => "LIVE_DOCS",
            Format::Points => "POINTS",
            Format::KnnVectors => "KNN_VECTORS",
        })
    }
}

/// The per-field choice of a postings format
/// (`getPostingsFormatForField(field)`).
pub type PostingsFormatForField = Arc<dyn Fn(&str) -> Arc<dyn PostingsFormat> + Send + Sync>;
/// The per-field choice of a doc-values format
/// (`getDocValuesFormatForField(field)`).
pub type DocValuesFormatForField = Arc<dyn Fn(&str) -> Arc<dyn DocValuesFormat> + Send + Sync>;
/// The per-field choice of a KNN-vectors format
/// (`getKnnVectorsFormatForField(field)`).
pub type KnnVectorsFormatForField = Arc<dyn Fn(&str) -> Arc<dyn KnnVectorsFormat> + Send + Sync>;

/// Acts like the default codec but with additional asserts.
///
/// Port of `org.apache.lucene.tests.codecs.asserting.AssertingCodec` (`extends
/// FilterCodec`, named `Asserting`, over `TestUtil.getDefaultCodec()`).
///
/// Java subclasses override `getPostingsFormatForField`,
/// `getDocValuesFormatForField` and `getKnnVectorsFormatForField`; the port
/// takes those choices as functions ([`with_formats_for_field`]), which the
/// codec's per-field formats call exactly where Java's anonymous
/// `PerField…Format`s call the overridable methods.
///
/// [`with_formats_for_field`]: AssertingCodec::with_formats_for_field
pub struct AssertingCodec {
    delegate: Arc<dyn Codec>,
    suppressed_formats: Vec<Format>,
    postings: PerFieldPostingsFormat,
    doc_values: PerFieldDocValuesFormat,
    knn_vectors_format: PerFieldKnnVectorsFormat,
    vectors: AssertingTermVectorsFormat,
    stored_fields: AssertingStoredFieldsFormat,
    norms: AssertingNormsFormat,
    live_docs: AssertingLiveDocsFormat,
    points_format: AssertingPointsFormat,
    postings_for_field: PostingsFormatForField,
    doc_values_for_field: DocValuesFormatForField,
    knn_vectors_for_field: KnnVectorsFormatForField,
}

impl fmt::Debug for AssertingCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl AssertingCodec {
    /// `AssertingCodec.assertThread(object, creationThread)`: Java throws an
    /// `AssertionError` (always, not under `-ea` only) when the calling thread
    /// is not the one that created the object.
    pub(crate) fn assert_thread(object: &str, creation_thread: &Thread) -> Result<()> {
        let current = Thread::current();
        if *creation_thread != current {
            return Err(LuceneError::from(LuceneErrorKind::AssertionError(format!(
                "{object} are only supposed to be consumed in the thread in which they have \
                 been acquired. But was acquired in {creation_thread} and consumed in \
                 {current}."
            ))));
        }
        Ok(())
    }

    /// `new AssertingCodec()`: every field gets the `Asserting` postings,
    /// doc-values and KNN-vectors formats.
    pub fn new() -> Self {
        Self::with_formats_for_field(None, None, None)
    }

    /// `new AssertingCodec()` of a subclass that overrides the per-field
    /// choices given here (`None` keeps Java's default, the codec's own
    /// `Asserting` format for every field).
    pub fn with_formats_for_field(
        postings_for_field: Option<PostingsFormatForField>,
        doc_values_for_field: Option<DocValuesFormatForField>,
        knn_vectors_for_field: Option<KnnVectorsFormatForField>,
    ) -> Self {
        let default_format: Arc<dyn PostingsFormat> = Arc::new(AssertingPostingsFormat::new());
        let default_dv_format: Arc<dyn DocValuesFormat> = Arc::new(AssertingDocValuesFormat::new());
        let default_knn_vectors_format: Arc<dyn KnnVectorsFormat> =
            Arc::new(AssertingKnnVectorsFormat::new());
        let postings_for_field = postings_for_field
            .unwrap_or_else(|| Arc::new(move |_field: &str| Arc::clone(&default_format)));
        let doc_values_for_field = doc_values_for_field
            .unwrap_or_else(|| Arc::new(move |_field: &str| Arc::clone(&default_dv_format)));
        let knn_vectors_for_field = knn_vectors_for_field.unwrap_or_else(|| {
            Arc::new(move |_field: &str| Arc::clone(&default_knn_vectors_format))
        });
        let postings = {
            let f = Arc::clone(&postings_for_field);
            PerFieldPostingsFormat::new(move |field| f(field))
        };
        let doc_values = {
            let f = Arc::clone(&doc_values_for_field);
            PerFieldDocValuesFormat::new(move |field| f(field))
        };
        let knn_vectors_format = {
            let f = Arc::clone(&knn_vectors_for_field);
            PerFieldKnnVectorsFormat::new(move |field| f(field))
        };
        AssertingCodec {
            delegate: TestUtil::get_default_codec(),
            suppressed_formats: suppressed_formats_of_target_class(),
            postings,
            doc_values,
            knn_vectors_format,
            vectors: AssertingTermVectorsFormat::new(),
            stored_fields: AssertingStoredFieldsFormat::new(),
            norms: AssertingNormsFormat::new(),
            live_docs: AssertingLiveDocsFormat::new(),
            points_format: AssertingPointsFormat::new(),
            postings_for_field,
            doc_values_for_field,
            knn_vectors_for_field,
        }
    }

    fn is_suppressed(&self, format: Format) -> bool {
        self.suppressed_formats.contains(&format)
    }

    /// The suppressed formats, in `EnumSet` (declaration) order.
    pub fn suppressed_formats(&self) -> &[Format] {
        &self.suppressed_formats
    }

    /// `getPostingsFormatForField(field)`: the postings format that should be
    /// used for writing new segments of `field`. The default implementation
    /// always returns "Asserting".
    pub fn get_postings_format_for_field(&self, field: &str) -> Arc<dyn PostingsFormat> {
        (self.postings_for_field)(field)
    }

    /// `getDocValuesFormatForField(field)`: the doc-values format that should
    /// be used for writing new segments of `field`. The default
    /// implementation always returns "Asserting".
    pub fn get_doc_values_format_for_field(&self, field: &str) -> Arc<dyn DocValuesFormat> {
        (self.doc_values_for_field)(field)
    }

    /// `getKnnVectorsFormatForField(field)`: the vectors format that should
    /// be used for writing new segments of `field`. The default
    /// implementation always returns "Asserting".
    pub fn get_knn_vectors_format_for_field(&self, field: &str) -> Arc<dyn KnnVectorsFormat> {
        (self.knn_vectors_for_field)(field)
    }
}

impl Default for AssertingCodec {
    fn default() -> Self {
        Self::new()
    }
}

/// The constructor's reading of `@SuppressAssertingFormats` on
/// `RandomizedContext.current().getTargetClass()`; no formats outside any
/// randomized context (Java catches the `IllegalStateException`).
fn suppressed_formats_of_target_class() -> Vec<Format> {
    let target_class = {
        let _quiet = QuietPanics::new();
        match catch_unwind(AssertUnwindSafe(|| {
            RandomizedContext::current().get_target_class_object()
        })) {
            Ok(class) => class,
            Err(payload) => {
                if panic_message(&*payload).starts_with(ILLEGAL_STATE_EXCEPTION) {
                    // Not under any randomized context.
                    None
                } else {
                    resume_unwind(payload)
                }
            }
        }
    };
    let mut suppressed = Vec::new();
    if let Some(class) = target_class {
        if class.is_annotation_present(&SUPPRESS_ASSERTING_FORMATS) {
            if let Some(annotation) = class.get_annotation::<SuppressAssertingFormats>() {
                suppressed.extend(annotation.value);
            }
        }
    }
    suppressed.sort();
    suppressed.dedup();
    suppressed
}

impl fmt::Display for AssertingCodec {
    /// `AssertingCodec.toString()` (`AssertingCodec.java:167-176`): the
    /// delegate through its own `toString()`, then the suppressed formats in
    /// `EnumSet` order.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Asserting({}", &*self.delegate as &dyn Codec)?;
        if !self.suppressed_formats.is_empty() {
            let names: Vec<String> = self
                .suppressed_formats
                .iter()
                .map(|s| s.to_string())
                .collect();
            write!(f, ", suppressed: {}", names.join(", "))?;
        }
        f.write_str(")")
    }
}

impl Codec for AssertingCodec {
    fn name(&self) -> &str {
        "Asserting"
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
        if self.is_suppressed(Format::StoredFields) {
            self.delegate.stored_fields_format()
        } else {
            &self.stored_fields
        }
    }

    fn term_vectors_format(&self) -> &dyn TermVectorsFormat {
        if self.is_suppressed(Format::TermVectors) {
            self.delegate.term_vectors_format()
        } else {
            &self.vectors
        }
    }

    fn field_infos_format(&self) -> &dyn FieldInfosFormat {
        self.delegate.field_infos_format()
    }

    fn segment_info_format(&self) -> &dyn SegmentInfoFormat {
        self.delegate.segment_info_format()
    }

    fn norms_format(&self) -> &dyn NormsFormat {
        if self.is_suppressed(Format::Norms) {
            self.delegate.norms_format()
        } else {
            &self.norms
        }
    }

    fn live_docs_format(&self) -> &dyn LiveDocsFormat {
        if self.is_suppressed(Format::LiveDocs) {
            self.delegate.live_docs_format()
        } else {
            &self.live_docs
        }
    }

    fn compound_format(&self) -> &dyn CompoundFormat {
        self.delegate.compound_format()
    }

    fn points_format(&self) -> &dyn PointsFormat {
        if self.is_suppressed(Format::Points) {
            self.delegate.points_format()
        } else {
            &self.points_format
        }
    }

    fn knn_vectors_format(&self) -> &dyn KnnVectorsFormat {
        if self.is_suppressed(Format::KnnVectors) {
            self.delegate.knn_vectors_format()
        } else {
            &self.knn_vectors_format
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `AssertingCodec.toString()` (`AssertingCodec.java:167-176`) with
    /// suppressed formats: `EnumSet` order, `Enum.toString()` names joined by
    /// `", "` after `", suppressed: "`.
    #[test]
    fn to_string_lists_suppressed_formats() {
        let mut codec = AssertingCodec::new();
        codec.suppressed_formats = vec![Format::StoredFields, Format::Norms, Format::KnnVectors];
        assert_eq!(
            (&codec as &dyn Codec).to_string(),
            "Asserting(Lucene104, suppressed: STORED_FIELDS, NORMS, KNN_VECTORS)"
        );
    }
}
