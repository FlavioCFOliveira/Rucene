//! Compound-file format and directory traits.
//!
//! Equivalent to `org.apache.lucene.codecs.CompoundFormat` and
//! `CompoundDirectory`.

use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::error::{LuceneError, Result};
use crate::store::{
    BufferedChecksumIndexInput, DataInput, DataOutput, Directory, IOContext, IndexInput,
    IndexOutput, Lock,
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

// -----------------------------------------------------------------------------
// Concrete compound directory
// -----------------------------------------------------------------------------

/// Offset and length describing a file slice inside a compound file.
///
/// Equivalent to `Lucene90CompoundReader.FileEntry`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CompoundFileEntry {
    /// Byte offset of the file's data within the compound data stream.
    pub offset: i64,
    /// Byte length of the file's data.
    pub length: i64,
}

/// A concrete, read-only compound directory backed by a shared byte buffer
/// and a file-entry table.
///
/// Equivalent in spirit to `Lucene90CompoundReader`: it keeps one shared data
/// buffer and returns slices of that buffer for each virtual file.
#[derive(Debug, Clone)]
pub struct CompoundFileDirectory {
    segment_name: String,
    data: std::sync::Arc<[u8]>,
    entries: HashMap<String, CompoundFileEntry>,
}

impl CompoundFileDirectory {
    /// Creates a compound directory from a data buffer and an entry map.
    ///
    /// `segment_name` is prepended to file names returned by [`Directory::list_all`].
    pub fn new(
        segment_name: impl Into<String>,
        data: std::sync::Arc<[u8]>,
        entries: HashMap<String, CompoundFileEntry>,
    ) -> Self {
        Self {
            segment_name: segment_name.into(),
            data,
            entries,
        }
    }

    /// Creates a compound directory by reading all bytes from `input`.
    ///
    /// The input is positioned at the start, read to the end, and closed.
    pub fn from_input(
        segment_name: impl Into<String>,
        input: &mut dyn IndexInput,
        entries: HashMap<String, CompoundFileEntry>,
    ) -> Result<Self> {
        input.seek(0)?;
        let len = input.length() as usize;
        let mut bytes = vec![0u8; len];
        input.read_bytes(&mut bytes, 0, len)?;
        input.close()?;
        Ok(Self::new(
            segment_name,
            std::sync::Arc::from(bytes),
            entries,
        ))
    }

    /// Reads the simple compound entry table format used by Lucene's compound
    /// files:
    ///
    /// - VInt file count
    /// - for each file: VInt/String name, Int64 offset, Int64 length
    pub fn read_entry_table(
        input: &mut dyn DataInput,
    ) -> Result<HashMap<String, CompoundFileEntry>> {
        let count = input.read_v_int()?;
        let mut entries = HashMap::with_capacity(count as usize);
        for _ in 0..count {
            let name = input.read_string()?;
            let offset = input.read_long()?;
            let length = input.read_long()?;
            entries.insert(name, CompoundFileEntry { offset, length });
        }
        Ok(entries)
    }

    /// Writes the simple compound entry table format.
    pub fn write_entry_table(
        output: &mut dyn DataOutput,
        entries: &HashMap<String, CompoundFileEntry>,
    ) -> Result<()> {
        output.write_v_int(entries.len() as i32)?;
        for (name, entry) in entries {
            output.write_string(name)?;
            output.write_long(entry.offset)?;
            output.write_long(entry.length)?;
        }
        Ok(())
    }

    fn strip_segment_name(&self, name: &str) -> String {
        if let Some(rest) = name.strip_prefix(&self.segment_name) {
            rest.to_string()
        } else {
            name.to_string()
        }
    }

    fn resource_description(&self) -> String {
        format!("CompoundFileDirectory(segment={})", self.segment_name)
    }
}

/// Index input over an immutable slice of a shared compound-file buffer.
#[derive(Debug, Clone)]
struct CompoundSliceInput {
    resource_description: String,
    data: std::sync::Arc<[u8]>,
    offset: usize,
    length: usize,
    position: usize,
}

impl CompoundSliceInput {
    fn new(
        resource_description: impl Into<String>,
        data: std::sync::Arc<[u8]>,
        offset: usize,
        length: usize,
    ) -> Self {
        Self {
            resource_description: resource_description.into(),
            data,
            offset,
            length,
            position: 0,
        }
    }
}

impl DataInput for CompoundSliceInput {
    fn read_byte(&mut self) -> Result<u8> {
        if self.position >= self.length {
            return Err(LuceneError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "read past EOF",
            )));
        }
        let b = self.data[self.offset + self.position];
        self.position += 1;
        Ok(b)
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        let end = offset
            .checked_add(len)
            .ok_or_else(|| LuceneError::IllegalArgument("offset + len overflowed".to_string()))?;
        if end > b.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "destination buffer too small: offset={offset}, len={len}, buf.len={}",
                b.len()
            )));
        }
        if self.position + len > self.length {
            return Err(LuceneError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "read past EOF",
            )));
        }
        b[offset..end].copy_from_slice(
            &self.data[self.offset + self.position..self.offset + self.position + len],
        );
        self.position += len;
        Ok(())
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numBytes must be non-negative (got: {num_bytes})"
            )));
        }
        let target = self.position + num_bytes as usize;
        if target > self.length {
            return Err(LuceneError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "skip past EOF",
            )));
        }
        self.position = target;
        Ok(())
    }
}

impl IndexInput for CompoundSliceInput {
    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn file_pointer(&self) -> i64 {
        self.position as i64
    }

    fn length(&self) -> i64 {
        self.length as i64
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        if pos < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "position must be non-negative (got: {pos})"
            )));
        }
        let pos = pos as usize;
        if pos > self.length {
            return Err(LuceneError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "seek past EOF",
            )));
        }
        self.position = pos;
        Ok(())
    }

    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        if offset < 0 || length < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "slice offset ({offset}) and length ({length}) must be non-negative"
            )));
        }
        let offset = offset as usize;
        let length = length as usize;
        let end = offset.checked_add(length).ok_or_else(|| {
            LuceneError::IllegalArgument("slice offset + length overflowed".to_string())
        })?;
        if end > self.length {
            return Err(LuceneError::IllegalArgument(format!(
                "slice(offset={offset}, length={length}) out of bounds, input length={}",
                self.length
            )));
        }
        let desc = if slice_description.is_empty() {
            self.resource_description.clone()
        } else {
            format!("{} [slice={slice_description}]", self.resource_description)
        };
        Ok(Box::new(CompoundSliceInput::new(
            desc,
            self.data.clone(),
            self.offset + offset,
            length,
        )))
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        Ok(Box::new(self.clone()))
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }
}

impl CompoundDirectory for CompoundFileDirectory {
    fn check_integrity(&self) -> Result<()> {
        let handle_len = self.data.len() as i64;
        for (name, entry) in &self.entries {
            if entry.offset < 0 || entry.length < 0 {
                return Err(LuceneError::IllegalArgument(format!(
                    "compound entry {name} has negative offset/length"
                )));
            }
            let end = entry.offset.checked_add(entry.length).ok_or_else(|| {
                LuceneError::IllegalArgument(format!(
                    "compound entry {name} offset+length overflowed"
                ))
            })?;
            if end > handle_len {
                return Err(LuceneError::IllegalArgument(format!(
                    "compound entry {name} slice [{}, {}) exceeds handle length {handle_len}",
                    entry.offset, end
                )));
            }
        }
        Ok(())
    }
}

impl Directory for CompoundFileDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        let mut names: Vec<String> = self
            .entries
            .keys()
            .map(|name| format!("{}{}", self.segment_name, name))
            .collect();
        names.sort();
        Ok(names)
    }

    fn delete_file(&self, _name: &str) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "deleteFile is not supported by CompoundDirectory".to_string(),
        ))
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        let id = self.strip_segment_name(name);
        self.entries.get(&id).map(|e| e.length).ok_or_else(|| {
            LuceneError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("file not found in compound directory: {name}"),
            ))
        })
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
        let id = self.strip_segment_name(name);
        let entry = self.entries.get(&id).ok_or_else(|| {
            LuceneError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "No sub-file with id {id} found in compound file \"{}\" (fileName={name})",
                    self.resource_description()
                ),
            ))
        })?;
        let offset = entry.offset as usize;
        let length = entry.length as usize;
        if offset + length > self.data.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "compound entry {id} slice [{offset}, {}) exceeds data length {}",
                offset + length,
                self.data.len()
            )));
        }
        Ok(Box::new(CompoundSliceInput::new(
            name.to_string(),
            self.data.clone(),
            offset,
            length,
        )))
    }

    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        let input = self.open_input(name, &*crate::store::READONCE_IO_CONTEXT)?;
        Ok(Box::new(BufferedChecksumIndexInput::new(input)))
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

    // CompoundFileDirectory tests ------------------------------------------------

    use crate::store::{ByteArrayDataInput, MockIndexOutput};

    fn arc_bytes(bytes: &[u8]) -> std::sync::Arc<[u8]> {
        std::sync::Arc::from(bytes.to_vec())
    }

    #[test]
    fn compound_directory_slices_files() {
        // Data file: "hello" at offset 0, "world" at offset 5.
        let data = arc_bytes(b"helloworld");

        let mut entries = HashMap::new();
        entries.insert(
            "a.txt".to_string(),
            CompoundFileEntry {
                offset: 0,
                length: 5,
            },
        );
        entries.insert(
            "b.txt".to_string(),
            CompoundFileEntry {
                offset: 5,
                length: 5,
            },
        );

        let dir = CompoundFileDirectory::new("_0", data, entries);

        let mut a = dir
            .open_input("_0a.txt", &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        let mut buf = [0u8; 5];
        a.read_bytes(&mut buf, 0, 5).unwrap();
        assert_eq!(&buf, b"hello");

        let mut b = dir
            .open_input("_0b.txt", &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        b.read_bytes(&mut buf, 0, 5).unwrap();
        assert_eq!(&buf, b"world");
    }

    #[test]
    fn compound_directory_list_all_prefixes_segment_name() {
        let data = arc_bytes(&[0u8; 8]);
        let mut entries = HashMap::new();
        entries.insert(
            "x.fnm".to_string(),
            CompoundFileEntry {
                offset: 0,
                length: 4,
            },
        );
        entries.insert(
            "y.fdt".to_string(),
            CompoundFileEntry {
                offset: 4,
                length: 4,
            },
        );
        let dir = CompoundFileDirectory::new("_1", data, entries);

        let names = dir.list_all().unwrap();
        assert_eq!(names, vec!["_1x.fnm".to_string(), "_1y.fdt".to_string()]);
    }

    #[test]
    fn compound_directory_file_length() {
        let data = arc_bytes(&[0u8; 10]);
        let mut entries = HashMap::new();
        entries.insert(
            "seg.fdt".to_string(),
            CompoundFileEntry {
                offset: 0,
                length: 10,
            },
        );
        let dir = CompoundFileDirectory::new("_2", data, entries);

        assert_eq!(dir.file_length("_2seg.fdt").unwrap(), 10);
        assert!(matches!(
            dir.file_length("missing"),
            Err(LuceneError::Io(_))
        ));
    }

    #[test]
    fn compound_directory_check_integrity_detects_overflow() {
        let data = arc_bytes(&[0u8; 4]);
        let mut entries = HashMap::new();
        entries.insert(
            "big.fdt".to_string(),
            CompoundFileEntry {
                offset: 2,
                length: 5,
            },
        );
        let dir = CompoundFileDirectory::new("_3", data, entries);

        let err = dir.check_integrity().unwrap_err();
        assert!(matches!(err, LuceneError::IllegalArgument(_)));
    }

    #[test]
    fn compound_directory_entry_table_round_trip() {
        let mut entries = HashMap::new();
        entries.insert(
            "a.fdt".to_string(),
            CompoundFileEntry {
                offset: 0,
                length: 10,
            },
        );
        entries.insert(
            "b.fdx".to_string(),
            CompoundFileEntry {
                offset: 10,
                length: 20,
            },
        );

        let mut output = MockIndexOutput::new("entries", "entries");
        CompoundFileDirectory::write_entry_table(&mut output, &entries).unwrap();
        let bytes = output.into_inner();

        let mut input = ByteArrayDataInput::new(bytes);
        let read = CompoundFileDirectory::read_entry_table(&mut input).unwrap();
        assert_eq!(read.len(), 2);
        assert_eq!(
            read.get("a.fdt"),
            Some(&CompoundFileEntry {
                offset: 0,
                length: 10
            })
        );
        assert_eq!(
            read.get("b.fdx"),
            Some(&CompoundFileEntry {
                offset: 10,
                length: 20
            })
        );
    }

    #[test]
    fn compound_directory_is_read_only() {
        let data = arc_bytes(&[0u8; 1]);
        let dir = CompoundFileDirectory::new("_4", data, HashMap::new());

        assert!(matches!(
            dir.create_output("x", &*crate::store::DEFAULT_IO_CONTEXT),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        assert!(matches!(
            dir.create_temp_output("p", "s", &*crate::store::DEFAULT_IO_CONTEXT),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        assert!(matches!(
            dir.delete_file("x"),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        assert!(matches!(
            dir.rename("a", "b"),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        assert!(matches!(
            dir.sync(&[]),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        assert!(matches!(
            dir.obtain_lock("x"),
            Err(LuceneError::UnsupportedOperation(_))
        ));
    }

    #[test]
    fn compound_directory_from_input_reads_all_bytes() {
        let data: Vec<u8> = (0..16).collect();
        let mut input = crate::store::MockIndexInput::new(data, "test.cfs");

        let mut entries = HashMap::new();
        entries.insert(
            "dummy".to_string(),
            CompoundFileEntry {
                offset: 0,
                length: 16,
            },
        );
        let dir = CompoundFileDirectory::from_input("_5", &mut input, entries).unwrap();

        // The data was loaded into memory; seeking/reading should work.
        let mut slice = dir
            .open_input("dummy", &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        slice.seek(4).unwrap();
        assert_eq!(slice.read_byte().unwrap(), 4);
    }
}
