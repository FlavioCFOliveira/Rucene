//! Live-docs format base trait.
//!
//! Equivalent to `org.apache.lucene.codecs.LiveDocsFormat`.

use std::fmt;

use crate::error::Result;
use crate::store::{Directory, IOContext};
use crate::util::Bits;

use super::stub::SegmentCommitInfo;

/// Format for live/deleted documents.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.LiveDocsFormat`.
pub trait LiveDocsFormat: Send + Sync + fmt::Debug {
    /// Returns this format's SPI name.
    fn name(&self) -> &str;

    /// Read live docs bits.
    fn read_live_docs(
        &self,
        dir: &dyn Directory,
        info: &SegmentCommitInfo,
        context: &dyn IOContext,
    ) -> Result<Box<dyn Bits>>;

    /// Persist live docs bits.
    fn write_live_docs(
        &self,
        bits: &dyn Bits,
        dir: &dyn Directory,
        info: &SegmentCommitInfo,
        new_del_count: i32,
        context: &dyn IOContext,
    ) -> Result<()>;

    /// Records all files in use by this [`SegmentCommitInfo`] into the files argument.
    fn files(&self, info: &SegmentCommitInfo, files: &mut Vec<String>) -> Result<()>;
}
