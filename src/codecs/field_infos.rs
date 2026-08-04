//! Field-infos format base trait.
//!
//! Equivalent to `org.apache.lucene.codecs.FieldInfosFormat`.

use std::fmt;

use crate::error::Result;
use crate::store::{Directory, IOContext};

use super::stub::{FieldInfos, SegmentInfo};

/// Encodes and decodes the field infos file.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.FieldInfosFormat`.
pub trait FieldInfosFormat: Send + Sync + fmt::Debug {
    /// Returns this format's SPI name.
    fn name(&self) -> &str;

    /// Read the [`FieldInfos`] previously written with [`write`](Self::write).
    fn read(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        segment_suffix: &str,
        context: &dyn IOContext,
    ) -> Result<FieldInfos>;

    /// Writes the provided [`FieldInfos`] to the directory.
    fn write(
        &self,
        directory: &dyn Directory,
        segment_info: &SegmentInfo,
        segment_suffix: &str,
        infos: &FieldInfos,
        context: &dyn IOContext,
    ) -> Result<()>;
}

/// A minimal no-op field-infos format.
#[derive(Debug, Copy, Clone, Default)]
pub struct EmptyFieldInfosFormat;

impl FieldInfosFormat for EmptyFieldInfosFormat {
    fn name(&self) -> &str {
        "EmptyFieldInfos"
    }

    fn read(
        &self,
        _directory: &dyn Directory,
        _segment_info: &SegmentInfo,
        _segment_suffix: &str,
        _context: &dyn IOContext,
    ) -> Result<FieldInfos> {
        Ok(FieldInfos)
    }

    fn write(
        &self,
        _directory: &dyn Directory,
        _segment_info: &SegmentInfo,
        _segment_suffix: &str,
        _infos: &FieldInfos,
        _context: &dyn IOContext,
    ) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_field_infos_format_round_trip() {
        let format = EmptyFieldInfosFormat;
        assert_eq!(format.name(), "EmptyFieldInfos");

        let dir: &dyn Directory = &crate::store::RamDirectory::default();
        let segment_info = SegmentInfo;
        let infos = FieldInfos;
        format
            .write(
                dir,
                &segment_info,
                "",
                &infos,
                &*crate::store::DEFAULT_IO_CONTEXT,
            )
            .unwrap();
        let read = format
            .read(dir, &segment_info, "", &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        assert_eq!(read, FieldInfos);
    }
}
