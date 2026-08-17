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
use crate::codecs::{
    Codec, CompoundFormat, DocValuesFormat, EmptyKnnVectorsFormat, FieldInfosFormat,
    KnnVectorsFormat, LiveDocsFormat, Lucene104PostingsFormat, Lucene90DocValuesFormat,
    Lucene90StoredFieldsFormat, Lucene94FieldInfosFormat, Lucene99SegmentInfoFormat, NormsFormat,
    PointsFormat, PostingsFormat, SegmentInfoFormat, StoredFieldsFormat, TermVectorsFormat,
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
#[derive(Debug, Clone, Default)]
pub struct Lucene104Codec {
    mode: Mode,
    stored_fields_format: Lucene90StoredFieldsFormat,
    default_postings_format: Lucene104PostingsFormat,
    default_doc_values_format: Lucene90DocValuesFormat,
    default_knn_vectors_format: EmptyKnnVectorsFormat,
}

impl Lucene104Codec {
    /// Creates the codec with the default stored-fields mode (`BestSpeed`).
    pub fn new() -> Self {
        Self::with_mode(Mode::default())
    }

    /// Creates the codec with the given stored-fields mode.
    pub fn with_mode(mode: Mode) -> Self {
        let stored_fields_format = Lucene90StoredFieldsFormat::with_mode(mode.stored_mode());
        Self {
            mode,
            stored_fields_format,
            default_postings_format: Lucene104PostingsFormat::new(),
            default_doc_values_format: Lucene90DocValuesFormat::new(),
            default_knn_vectors_format: EmptyKnnVectorsFormat::new("Lucene99HnswVectorsFormat"),
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
        &self.default_postings_format
    }

    /// Returns the doc-values format used for fields that do not select a custom
    /// format.
    ///
    /// Mirrors `Lucene104Codec.getDocValuesFormatForField`.
    pub fn get_doc_values_format_for_field(&self, _field: &str) -> &dyn DocValuesFormat {
        &self.default_doc_values_format
    }

    /// Returns the KNN-vectors format used for fields that do not select a
    /// custom format.
    ///
    /// Mirrors `Lucene104Codec.getKnnVectorsFormatForField`.
    pub fn get_knn_vectors_format_for_field(&self, _field: &str) -> &dyn KnnVectorsFormat {
        &self.default_knn_vectors_format
    }
}

impl Codec for Lucene104Codec {
    fn name(&self) -> &str {
        "Lucene104"
    }

    fn postings_format(&self) -> &dyn PostingsFormat {
        self.get_postings_format_for_field("")
    }

    fn doc_values_format(&self) -> &dyn DocValuesFormat {
        self.get_doc_values_format_for_field("")
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
        self.get_knn_vectors_format_for_field("")
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
        assert_eq!(codec.postings_format().name(), "Lucene104");
        assert_eq!(codec.doc_values_format().name(), "Lucene90");
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
