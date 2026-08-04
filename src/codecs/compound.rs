//! Compound-file format and directory traits.
//!
//! Equivalent to `org.apache.lucene.codecs.CompoundFormat` and
//! `CompoundDirectory`.

use std::collections::HashSet;
use std::fmt;

use crate::error::{LuceneError, Result};
use crate::store::{
    BufferedChecksumIndexInput, Directory, IOContext, IndexInput, IndexOutput, Lock,
};

use super::stub::SegmentInfo;

/// A read-only [`Directory`] that consists of a view over a compound file.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.CompoundDirectory`.
pub trait CompoundDirectory: Directory + Send + Sync + fmt::Debug {
    /// Checks consistency of this directory.
    fn check_integrity(&self) -> Result<()>;
}

/// Encodes and decodes compound files.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.CompoundFormat`.
pub trait CompoundFormat: Send + Sync + fmt::Debug {
    /// Returns this format's SPI name.
    fn name(&self) -> &str;

    /// Returns a [`CompoundDirectory`] view for the compound files in this segment.
    fn get_compound_reader(
        &self,
        dir: &dyn Directory,
        segment_info: &SegmentInfo,
    ) -> Result<Box<dyn CompoundDirectory>>;

    /// Packs the provided segment's files into a compound format.
    fn write(
        &self,
        dir: &dyn Directory,
        segment_info: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<()>;
}

/// A minimal no-op compound format.
#[derive(Debug, Copy, Clone, Default)]
pub struct EmptyCompoundFormat;

impl CompoundFormat for EmptyCompoundFormat {
    fn name(&self) -> &str {
        "EmptyCompound"
    }

    fn get_compound_reader(
        &self,
        _dir: &dyn Directory,
        _segment_info: &SegmentInfo,
    ) -> Result<Box<dyn CompoundDirectory>> {
        Ok(Box::new(EmptyCompoundDirectory))
    }

    fn write(
        &self,
        _dir: &dyn Directory,
        _segment_info: &SegmentInfo,
        _context: &dyn IOContext,
    ) -> Result<()> {
        Ok(())
    }
}

/// A minimal no-op compound directory.
///
/// The mutating operations required by [`Directory`] return
/// [`LuceneError::UnsupportedOperation`], matching Lucene's
/// `UnsupportedOperationException`.
#[derive(Debug, Copy, Clone, Default)]
pub struct EmptyCompoundDirectory;

impl CompoundDirectory for EmptyCompoundDirectory {
    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }
}

impl Directory for EmptyCompoundDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    fn delete_file(&self, _name: &str) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "deleteFile is not supported by CompoundDirectory".to_string(),
        ))
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        Err(LuceneError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("file not found in compound directory: {name}"),
        )))
    }

    fn create_output(&self, _name: &str, _context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        Err(LuceneError::UnsupportedOperation(
            "createOutput is not supported by CompoundDirectory".to_string(),
        ))
    }

    fn create_temp_output(
        &self,
        _prefix: &str,
        _suffix: &str,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        Err(LuceneError::UnsupportedOperation(
            "createTempOutput is not supported by CompoundDirectory".to_string(),
        ))
    }

    fn sync(&self, _names: &[String]) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "sync is not supported by CompoundDirectory".to_string(),
        ))
    }

    fn sync_metadata(&self) -> Result<()> {
        Ok(())
    }

    fn rename(&self, _source: &str, _dest: &str) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "rename is not supported by CompoundDirectory".to_string(),
        ))
    }

    fn open_input(&self, name: &str, _context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        Err(LuceneError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("file not found in compound directory: {name}"),
        )))
    }

    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        Err(LuceneError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("file not found in compound directory: {name}"),
        )))
    }

    fn obtain_lock(&self, _name: &str) -> Result<Box<dyn Lock>> {
        Err(LuceneError::UnsupportedOperation(
            "obtainLock is not supported by CompoundDirectory".to_string(),
        ))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        Ok(HashSet::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_compound_format_name() {
        let format = EmptyCompoundFormat;
        assert_eq!(format.name(), "EmptyCompound");
    }

    #[test]
    fn empty_compound_directory_is_read_only() {
        let dir = EmptyCompoundDirectory;

        assert!(dir.list_all().unwrap().is_empty());
        assert!(matches!(
            dir.delete_file("x"),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        assert!(matches!(
            dir.create_output("x", &*crate::store::DEFAULT_IO_CONTEXT),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        assert!(matches!(
            dir.create_temp_output("p", "s", &*crate::store::DEFAULT_IO_CONTEXT),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        assert!(matches!(
            dir.sync(&[]),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        assert!(matches!(
            dir.rename("a", "b"),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        assert!(matches!(
            dir.obtain_lock("x"),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        assert!(dir.sync_metadata().is_ok());
        assert!(dir.get_pending_deletions().unwrap().is_empty());
        assert!(dir.check_integrity().is_ok());
    }

    #[test]
    fn empty_compound_directory_open_input_not_found() {
        let dir = EmptyCompoundDirectory;
        assert!(matches!(
            dir.open_input("x", &*crate::store::DEFAULT_IO_CONTEXT),
            Err(LuceneError::Io(_))
        ));
    }

    #[test]
    fn empty_compound_directory_implements_directory() {
        fn assert_directory(_: &dyn Directory) {}
        let dir = EmptyCompoundDirectory;
        assert_directory(&dir);
    }
}
