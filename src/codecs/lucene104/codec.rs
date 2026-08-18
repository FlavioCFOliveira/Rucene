//! Lucene 10.4 codec assembly.
//!
//! This module wires together all the sub-formats that make up the default
//! `Lucene104Codec`, matching `org.apache.lucene.codecs.lucene104.Lucene104Codec`.
//!
//! Lucene Core equivalent: `org.apache.lucene.codecs.lucene104.Lucene104Codec`.

#![deny(unsafe_code)]

use crate::codecs::lucene90::stored_fields::Mode as StoredFieldsMode;
use crate::codecs::lucene90::{
    Lucene90CompoundFormat, Lucene90LiveDocsFormat, Lucene90NormsFormat, Lucene90PointsFormat,
    Lucene90TermVectorsFormat,
};
use std::sync::Arc;

use crate::codecs::{
    Codec, CompoundFormat, DocValuesFormat, FieldInfosFormat, KnnVectorsFormat, LiveDocsFormat,
    Lucene104PostingsFormat, Lucene90DocValuesFormat, Lucene90StoredFieldsFormat,
    Lucene94FieldInfosFormat, Lucene99HnswVectorsFormat, Lucene99SegmentInfoFormat, NormsFormat,
    PerFieldDocValuesFormat, PerFieldKnnVectorsFormat, PerFieldPostingsFormat, PointsFormat,
    PostingsFormat, SegmentInfoFormat, StoredFieldsFormat, TermVectorsFormat,
};

/// Stored-fields compression mode for the codec.
///
/// Mirrors `Lucene104Codec.Mode` in Java.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Trade compression ratio for retrieval speed.
    #[default]
    BestSpeed,
    /// Trade retrieval speed for compression ratio.
    BestCompression,
}

impl Mode {
    fn stored_mode(self) -> StoredFieldsMode {
        match self {
            Mode::BestSpeed => StoredFieldsMode::BestSpeed,
            Mode::BestCompression => StoredFieldsMode::BestCompression,
        }
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mode::BestSpeed => write!(f, "BEST_SPEED"),
            Mode::BestCompression => write!(f, "BEST_COMPRESSION"),
        }
    }
}

/// Lucene 10.4 codec.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.lucene104.Lucene104Codec`.
#[derive(Debug)]
pub struct Lucene104Codec {
    mode: Mode,
    stored_fields_format: Lucene90StoredFieldsFormat,
    default_postings_format: Arc<dyn PostingsFormat>,
    per_field_postings_format: PerFieldPostingsFormat,
    default_doc_values_format: Arc<dyn DocValuesFormat>,
    per_field_doc_values_format: PerFieldDocValuesFormat,
    default_knn_vectors_format: Arc<dyn KnnVectorsFormat>,
    per_field_knn_vectors_format: PerFieldKnnVectorsFormat,
}

impl Lucene104Codec {
    /// Creates the codec with the default stored-fields mode (`BestSpeed`).
    pub fn new() -> Self {
        Self::with_mode(Mode::default())
    }

    /// Creates the codec with the given stored-fields mode.
    pub fn with_mode(mode: Mode) -> Self {
        let stored_fields_format = Lucene90StoredFieldsFormat::with_mode(mode.stored_mode());
        let default_postings_format: Arc<dyn PostingsFormat> =
            Arc::new(Lucene104PostingsFormat::new());
        let default_doc_values_format: Arc<dyn DocValuesFormat> =
            Arc::new(Lucene90DocValuesFormat::new());
        let default_knn_vectors_format: Arc<dyn KnnVectorsFormat> =
            Arc::new(Lucene99HnswVectorsFormat::new());
        Self {
            mode,
            stored_fields_format,
            per_field_postings_format: PerFieldPostingsFormat::from_map(
                std::collections::HashMap::new(),
                Arc::clone(&default_postings_format),
            ),
            default_postings_format,
            per_field_doc_values_format: PerFieldDocValuesFormat::from_map(
                std::collections::HashMap::new(),
                Arc::clone(&default_doc_values_format),
            ),
            default_doc_values_format,
            per_field_knn_vectors_format: PerFieldKnnVectorsFormat::from_map(
                std::collections::HashMap::new(),
                Arc::clone(&default_knn_vectors_format),
            ),
            default_knn_vectors_format,
        }
    }

    /// Returns the stored-fields compression mode configured for this codec.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Returns the postings format used for fields that do not select a custom
    /// format.
    ///
    /// Mirrors `Lucene104Codec.getPostingsFormatForField`.
    pub fn get_postings_format_for_field(&self, _field: &str) -> &dyn PostingsFormat {
        &*self.default_postings_format
    }

    /// Returns the doc-values format used for fields that do not select a custom
    /// format.
    ///
    /// Mirrors `Lucene104Codec.getDocValuesFormatForField`.
    pub fn get_doc_values_format_for_field(&self, _field: &str) -> &dyn DocValuesFormat {
        &*self.default_doc_values_format
    }

    /// Returns the KNN-vectors format used for fields that do not select a
    /// custom format.
    ///
    /// Mirrors `Lucene104Codec.getKnnVectorsFormatForField`.
    pub fn get_knn_vectors_format_for_field(&self, _field: &str) -> &dyn KnnVectorsFormat {
        &*self.default_knn_vectors_format
    }
}

impl Codec for Lucene104Codec {
    fn name(&self) -> &str {
        "Lucene104"
    }

    fn postings_format(&self) -> &dyn PostingsFormat {
        &self.per_field_postings_format
    }

    fn doc_values_format(&self) -> &dyn DocValuesFormat {
        &self.per_field_doc_values_format
    }

    fn stored_fields_format(&self) -> &dyn StoredFieldsFormat {
        &self.stored_fields_format
    }

    fn term_vectors_format(&self) -> &dyn TermVectorsFormat {
        // The term-vectors format is stateless; return a fresh instance.
        static FORMAT: std::sync::OnceLock<Lucene90TermVectorsFormat> = std::sync::OnceLock::new();
        FORMAT.get_or_init(Lucene90TermVectorsFormat::new)
    }

    fn field_infos_format(&self) -> &dyn FieldInfosFormat {
        static FORMAT: std::sync::OnceLock<Lucene94FieldInfosFormat> = std::sync::OnceLock::new();
        FORMAT.get_or_init(Lucene94FieldInfosFormat::new)
    }

    fn segment_info_format(&self) -> &dyn SegmentInfoFormat {
        static FORMAT: std::sync::OnceLock<Lucene99SegmentInfoFormat> = std::sync::OnceLock::new();
        FORMAT.get_or_init(Lucene99SegmentInfoFormat::new)
    }

    fn norms_format(&self) -> &dyn NormsFormat {
        static FORMAT: std::sync::OnceLock<Lucene90NormsFormat> = std::sync::OnceLock::new();
        FORMAT.get_or_init(Lucene90NormsFormat::new)
    }

    fn live_docs_format(&self) -> &dyn LiveDocsFormat {
        static FORMAT: std::sync::OnceLock<Lucene90LiveDocsFormat> = std::sync::OnceLock::new();
        FORMAT.get_or_init(Lucene90LiveDocsFormat::new)
    }

    fn compound_format(&self) -> &dyn CompoundFormat {
        static FORMAT: std::sync::OnceLock<Lucene90CompoundFormat> = std::sync::OnceLock::new();
        FORMAT.get_or_init(Lucene90CompoundFormat::new)
    }

    fn points_format(&self) -> &dyn PointsFormat {
        static FORMAT: std::sync::OnceLock<Lucene90PointsFormat> = std::sync::OnceLock::new();
        FORMAT.get_or_init(Lucene90PointsFormat::new)
    }

    fn knn_vectors_format(&self) -> &dyn KnnVectorsFormat {
        &self.per_field_knn_vectors_format
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::Codec as _;

    #[test]
    fn codec_reports_correct_name_and_subformats() {
        let codec = Lucene104Codec::new();
        assert_eq!(codec.name(), "Lucene104");
        // The codec exposes per-field wrappers for postings, doc-values and
        // knn-vectors, matching Lucene104Codec's use of PerFieldPostingsFormat,
        // PerFieldDocValuesFormat and PerFieldKnnVectorsFormat.
        assert_eq!(codec.postings_format().name(), "PerField40");
        assert_eq!(codec.doc_values_format().name(), "PerFieldDV40");
        assert_eq!(codec.knn_vectors_format().name(), "PerFieldVectors90");
        // Per-field hooks still resolve to the concrete default formats.
        assert_eq!(codec.get_postings_format_for_field("").name(), "Lucene104");
        assert_eq!(codec.get_doc_values_format_for_field("").name(), "Lucene90");
        assert_eq!(
            codec.get_knn_vectors_format_for_field("").name(),
            "Lucene99HnswVectorsFormat"
        );
        assert_eq!(
            codec.stored_fields_format().name(),
            "Lucene90StoredFieldsFormat"
        );
        assert_eq!(codec.term_vectors_format().name(), "Lucene90");
        assert_eq!(codec.field_infos_format().name(), "Lucene94FieldInfos");
        assert_eq!(codec.segment_info_format().name(), "Lucene99SegmentInfo");
        assert_eq!(codec.norms_format().name(), "Lucene90Norms");
        assert_eq!(codec.live_docs_format().name(), "Lucene90LiveDocs");
        assert_eq!(codec.compound_format().name(), "Lucene90CompoundFormat");
        assert_eq!(codec.points_format().name(), "Lucene90");
    }

    #[test]
    fn codec_supports_both_modes() {
        assert_eq!(
            Lucene104Codec::with_mode(Mode::BestSpeed).mode(),
            Mode::BestSpeed
        );
        assert_eq!(
            Lucene104Codec::with_mode(Mode::BestCompression).mode(),
            Mode::BestCompression
        );
    }
}
