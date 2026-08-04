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

/// A minimal no-op live-docs format.
#[derive(Debug, Copy, Clone, Default)]
pub struct EmptyLiveDocsFormat;

impl LiveDocsFormat for EmptyLiveDocsFormat {
    fn name(&self) -> &str {
        "EmptyLiveDocs"
    }

    fn read_live_docs(
        &self,
        _dir: &dyn Directory,
        _info: &SegmentCommitInfo,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn Bits>> {
        Ok(Box::new(crate::util::MatchAllBits::new(0)))
    }

    fn write_live_docs(
        &self,
        _bits: &dyn Bits,
        _dir: &dyn Directory,
        _info: &SegmentCommitInfo,
        _new_del_count: i32,
        _context: &dyn IOContext,
    ) -> Result<()> {
        Ok(())
    }

    fn files(&self, _info: &SegmentCommitInfo, _files: &mut Vec<String>) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_live_docs_format_methods() {
        let format = EmptyLiveDocsFormat;
        assert_eq!(format.name(), "EmptyLiveDocs");

        let dir: &dyn Directory = &crate::store::RamDirectory::default();
        let info = SegmentCommitInfo;
        let bits = format
            .read_live_docs(dir, &info, &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        assert_eq!(bits.length(), 0);
        format
            .write_live_docs(
                bits.as_ref(),
                dir,
                &info,
                0,
                &*crate::store::DEFAULT_IO_CONTEXT,
            )
            .unwrap();
        let mut files = Vec::new();
        format.files(&info, &mut files).unwrap();
        assert!(files.is_empty());
    }
}
