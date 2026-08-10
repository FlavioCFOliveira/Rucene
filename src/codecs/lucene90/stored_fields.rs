//! Stub implementation of Lucene 9.0 stored-fields format.
//!
//! This placeholder satisfies the codec exports while the real
//! `Lucene90CompressingStoredFieldsFormat` is ported. It compiles and
//! delegates to the no-op stored-fields traits so the rest of the
//! crate can be built and tested independently.

use crate::codecs::stored_fields::{
    EmptyStoredFieldsReader, EmptyStoredFieldsWriter, StoredFieldsFormat, StoredFieldsReader,
    StoredFieldsWriter,
};
use crate::codecs::stub::{FieldInfos, SegmentInfo};
use crate::error::Result;
use crate::store::{Directory, IOContext};

/// Compression mode selector for `Lucene90StoredFieldsFormat`.
///
/// Mirrors `org.apache.lucene.codecs.lucene90.Lucene90StoredFieldsFormat.Mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// LZ4 compression optimized for speed.
    #[default]
    BestSpeed,
    /// Deflate compression optimized for space.
    BestCompression,
}

/// Default Lucene 9.0 stored-fields format factory.
///
/// Placeholder: returns no-op readers/writers.
#[derive(Debug, Clone, Copy, Default)]
pub struct Lucene90StoredFieldsFormat;

impl Lucene90StoredFieldsFormat {
    /// Creates a new format instance using the default mode.
    pub fn new() -> Self {
        Self
    }
}

impl StoredFieldsFormat for Lucene90StoredFieldsFormat {
    fn name(&self) -> &str {
        "Lucene90StoredFieldsFormat"
    }

    fn fields_reader(
        &self,
        _directory: &dyn Directory,
        _segment_info: &SegmentInfo,
        _field_infos: &FieldInfos,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsReader>> {
        Ok(Box::new(EmptyStoredFieldsReader))
    }

    fn fields_writer(
        &self,
        _directory: &dyn Directory,
        _segment_info: &SegmentInfo,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsWriter>> {
        Ok(Box::new(EmptyStoredFieldsWriter))
    }
}

/// Compressing stored-fields format used by `Lucene90StoredFieldsFormat`.
///
/// Placeholder: returns no-op readers/writers.
#[derive(Debug, Clone, Copy, Default)]
pub struct Lucene90CompressingStoredFieldsFormat;

impl Lucene90CompressingStoredFieldsFormat {
    /// Creates a new compressing stored-fields format.
    pub fn new(_mode: Mode) -> Self {
        Self
    }
}

impl StoredFieldsFormat for Lucene90CompressingStoredFieldsFormat {
    fn name(&self) -> &str {
        "Lucene90CompressingStoredFieldsFormat"
    }

    fn fields_reader(
        &self,
        _directory: &dyn Directory,
        _segment_info: &SegmentInfo,
        _field_infos: &FieldInfos,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsReader>> {
        Ok(Box::new(EmptyStoredFieldsReader))
    }

    fn fields_writer(
        &self,
        _directory: &dyn Directory,
        _segment_info: &SegmentInfo,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn StoredFieldsWriter>> {
        Ok(Box::new(EmptyStoredFieldsWriter))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_names() {
        assert_eq!(
            Lucene90StoredFieldsFormat::new().name(),
            "Lucene90StoredFieldsFormat"
        );
        assert_eq!(
            Lucene90CompressingStoredFieldsFormat::new(Mode::BestSpeed).name(),
            "Lucene90CompressingStoredFieldsFormat"
        );
    }

    #[test]
    fn mode_default_is_best_speed() {
        assert_eq!(Mode::default(), Mode::BestSpeed);
    }
}
