//! Segment-info format base trait.
//!
//! Equivalent to `org.apache.lucene.codecs.SegmentInfoFormat`.

use std::fmt;
use std::sync::Arc;

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
        directory: Arc<dyn Directory>,
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
