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
