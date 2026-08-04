//! Segment-info format base trait.
//!
//! Equivalent to `org.apache.lucene.codecs.SegmentInfoFormat`.

use std::fmt;

use crate::error::Result;
use crate::index::SegmentInfo;
use crate::store::{Directory, IOContext};

/// Encodes and decodes the segment metadata file.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.SegmentInfoFormat`.
pub trait SegmentInfoFormat: Send + Sync + fmt::Debug {
    /// Returns this format's SPI name.
    fn name(&self) -> &str;

    /// Read [`SegmentInfo`] data from a directory.
    fn read(
        &self,
        directory: &dyn Directory,
        segment_name: &str,
        segment_id: &[u8],
        context: &dyn IOContext,
    ) -> Result<SegmentInfo>;

    /// Write [`SegmentInfo`] data.
    fn write(
        &self,
        directory: &dyn Directory,
        info: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<()>;
}

/// A minimal no-op segment-info format.
#[derive(Debug, Copy, Clone, Default)]
pub struct EmptySegmentInfoFormat;

impl SegmentInfoFormat for EmptySegmentInfoFormat {
    fn name(&self) -> &str {
        "EmptySegmentInfo"
    }

    fn read(
        &self,
        _directory: &dyn Directory,
        _segment_name: &str,
        _segment_id: &[u8],
        _context: &dyn IOContext,
    ) -> Result<SegmentInfo> {
        Err(crate::error::LuceneError::UnsupportedOperation(
            "EmptySegmentInfoFormat cannot read".to_string(),
        ))
    }

    fn write(
        &self,
        _directory: &dyn Directory,
        _info: &SegmentInfo,
        _context: &dyn IOContext,
    ) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_segment_info_format_name() {
        let format = EmptySegmentInfoFormat;
        assert_eq!(format.name(), "EmptySegmentInfo");
    }
}
