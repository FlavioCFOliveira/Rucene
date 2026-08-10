//! Lucene 9.0 compound-file format.
//!
//! Ports `Lucene90CompoundFormat` and `Lucene90CompoundReader` from Apache
//! Lucene Core 10.5.0.

#![deny(unsafe_code)]

use std::collections::HashSet;
use std::sync::Arc;

use crate::codecs::codec_util;
use crate::codecs::compound::CompoundFormat;
use crate::codecs::compound::{CompoundDirectory, CompoundFileEntry};
use crate::codecs::stub::SegmentInfo;
use crate::error::{LuceneError, Result};
use crate::index::index_file_names::{
    segment_file_name, strip_segment_name, COMPOUND_FILE_ENTRIES_EXTENSION, COMPOUND_FILE_EXTENSION,
};
use crate::store::{DataInput, DataOutput, Directory, IOContext, IndexInput, IndexOutput, Lock};

const DATA_CODEC: &str = "Lucene90CompoundData";
const ENTRY_CODEC: &str = "Lucene90CompoundEntries";
const VERSION_START: i32 = 0;
const VERSION_CURRENT: i32 = VERSION_START;

/// Alignment applied to each embedded file's start offset.
///
/// Matches `Lucene90CompoundFormat.ALIGNMENT_BYTES`.
const ALIGNMENT_BYTES: i64 = 64;

/// Lucene 9.0 compound-file format.
///
/// Equivalent to `org.apache.lucene.codecs.lucene90.Lucene90CompoundFormat`.
#[derive(Debug, Default, Clone, Copy)]
pub struct Lucene90CompoundFormat;

impl Lucene90CompoundFormat {
    /// Creates a new compound-file format instance.
    pub fn new() -> Self {
        Self
    }
}

impl CompoundFormat for Lucene90CompoundFormat {
    fn name(&self) -> &str {
        "Lucene90CompoundFormat"
    }

    fn get_compound_reader(
        &self,
        dir: &dyn Directory,
        segment_info: &SegmentInfo,
    ) -> Result<Box<dyn CompoundDirectory>> {
        let reader = Lucene90CompoundReader::new(dir, segment_info)?;
        Ok(Box::new(reader))
    }

    fn write(
        &self,
        dir: &dyn Directory,
        segment_info: &SegmentInfo,
        context: &dyn IOContext,
    ) -> Result<()> {
        let data_file = segment_file_name(&segment_info.name, "", COMPOUND_FILE_EXTENSION);
        let entries_file =
            segment_file_name(&segment_info.name, "", COMPOUND_FILE_ENTRIES_EXTENSION);

        let mut data = dir.create_output(&data_file, context)?;
        let mut entries = dir.create_output(&entries_file, context)?;

        codec_util::write_index_header(
            data.as_mut(),
            DATA_CODEC,
            VERSION_CURRENT,
            &segment_info.id(),
            "",
        )?;
        codec_util::write_index_header(
            entries.as_mut(),
            ENTRY_CODEC,
            VERSION_CURRENT,
            &segment_info.id(),
            "",
        )?;

        write_compound_file(entries.as_mut(), data.as_mut(), dir, segment_info)?;

        codec_util::write_footer(data.as_mut())?;
        codec_util::write_footer(entries.as_mut())?;
        data.close()?;
        entries.close()?;
        Ok(())
    }
}

fn write_compound_file(
    entries: &mut dyn IndexOutput,
    data: &mut dyn IndexOutput,
    dir: &dyn Directory,
    segment_info: &SegmentInfo,
) -> Result<()> {
    let files = segment_info.files()?;
    entries.write_v_int(files.len() as i32)?;

    // Sort files by ascending length so small files pack together, matching Java.
    let mut sized: Vec<(&String, i64)> = files
        .iter()
        .map(|name| (name, dir.file_length(name).unwrap_or(-1)))
        .collect();
    sized.sort_by_key(|&(_, len)| len);

    for (file, _length) in sized {
        align_file_pointer(data, ALIGNMENT_BYTES)?;
        let start_offset = data.file_pointer();

        let mut input = dir.open_checksum_input(file)?;
        // Verify and copy the index header, ensuring the segment id matches.
        verify_and_copy_index_header(input.as_mut(), data, &segment_info.id(), "")?;

        let num_bytes_to_copy =
            input.length() - codec_util::footer_length() as i64 - input.file_pointer();
        data.copy_bytes(input.as_mut(), num_bytes_to_copy)?;
        let checksum = codec_util::check_footer(input.as_mut())?;

        // Reproduce the footer using the original file's checksum rather than
        // the data output's checksum, so each embedded file remains valid.
        codec_util::write_be_int(data, codec_util::FOOTER_MAGIC)?;
        codec_util::write_be_int(data, 0)?;
        codec_util::write_be_long(data, checksum)?;

        let end_offset = data.file_pointer();
        let length = end_offset - start_offset;

        entries.write_string(strip_segment_name(file))?;
        entries.write_long(start_offset)?;
        entries.write_long(length)?;
    }

    Ok(())
}

/// Reads an index header from `input`, verifies the object id and suffix, and
/// copies the header fields to `output` verbatim.
///
/// This mirrors Java's `CodecUtil.verifyAndCopyIndexHeader` used by the
/// compound writer: the actual codec name is not validated because the compound
/// format embeds arbitrary sub-files.
fn verify_and_copy_index_header(
    input: &mut dyn DataInput,
    output: &mut dyn DataOutput,
    expected_id: &[u8],
    expected_suffix: &str,
) -> Result<()> {
    let magic = codec_util::read_be_int(input)?;
    if magic != codec_util::CODEC_MAGIC {
        return Err(LuceneError::CorruptIndex(format!(
            "codec header mismatch: actual header={magic} vs expected header={}",
            codec_util::CODEC_MAGIC
        )));
    }
    codec_util::write_be_int(output, magic)?;
    let codec_name = input.read_string()?;
    output.write_string(&codec_name)?;
    let version = codec_util::read_be_int(input)?;
    codec_util::write_be_int(output, version)?;
    codec_util::check_index_header_id(input, expected_id)?;
    output.write_bytes(expected_id, 0, expected_id.len())?;
    codec_util::check_index_header_suffix(input, expected_suffix)?;
    output.write_byte(expected_suffix.len() as u8)?;
    output.write_bytes(expected_suffix.as_bytes(), 0, expected_suffix.len())?;
    Ok(())
}

fn align_file_pointer(out: &mut dyn IndexOutput, alignment: i64) -> Result<()> {
    let pos = out.file_pointer();
    let rem = pos % alignment;
    if rem != 0 {
        let padding = alignment - rem;
        for _ in 0..padding {
            out.write_byte(0)?;
        }
    }
    Ok(())
}

/// Read-only directory view over a Lucene 9.0 compound file.
///
/// Equivalent to `org.apache.lucene.codecs.lucene90.Lucene90CompoundReader`.
#[derive(Debug, Clone)]
pub struct Lucene90CompoundReader {
    segment_name: String,
    entries: std::collections::HashMap<String, CompoundFileEntry>,
    data: Arc<[u8]>,
}

impl Lucene90CompoundReader {
    /// Opens the compound reader for the given segment.
    pub fn new(dir: &dyn Directory, segment_info: &SegmentInfo) -> Result<Self> {
        let segment_name = segment_info.name.clone();
        let data_file = segment_file_name(&segment_name, "", COMPOUND_FILE_EXTENSION);
        let entries_file = segment_file_name(&segment_name, "", COMPOUND_FILE_ENTRIES_EXTENSION);

        let entries = read_entries(dir, &entries_file, &segment_info.id())?;

        let expected_length = entries
            .values()
            .map(|e| e.offset + e.length)
            .max()
            .unwrap_or_else(|| codec_util::index_header_length(DATA_CODEC, "") as i64)
            + codec_util::footer_length() as i64;

        let mut handle = dir.open_input(&data_file, &*crate::store::READONCE_IO_CONTEXT)?;
        codec_util::check_index_header(
            handle.as_mut(),
            DATA_CODEC,
            VERSION_START,
            VERSION_CURRENT,
            &segment_info.id(),
            "",
        )?;
        codec_util::retrieve_checksum(handle.as_mut())?;
        if handle.length() != expected_length {
            return Err(LuceneError::CorruptIndex(format!(
                "length should be {expected_length} bytes, but is {} instead",
                handle.length()
            )));
        }

        handle.seek(0)?;
        let len = handle.length() as usize;
        let mut bytes = vec![0u8; len];
        handle.read_bytes(&mut bytes, 0, len)?;
        handle.close()?;

        Ok(Self {
            segment_name,
            entries,
            data: Arc::from(bytes),
        })
    }

    fn strip_segment_name(&self, name: &str) -> String {
        if let Some(rest) = name.strip_prefix(&self.segment_name) {
            rest.to_string()
        } else {
            name.to_string()
        }
    }
}

fn read_entries(
    dir: &dyn Directory,
    entries_file: &str,
    segment_id: &[u8],
) -> Result<std::collections::HashMap<String, CompoundFileEntry>> {
    let mut input = dir.open_checksum_input(entries_file)?;
    let version = codec_util::check_index_header(
        input.as_mut(),
        ENTRY_CODEC,
        VERSION_START,
        VERSION_CURRENT,
        segment_id,
        "",
    )?;
    let mapping = read_mapping(input.as_mut(), version)?;
    codec_util::check_footer(input.as_mut())?;
    Ok(mapping)
}

fn read_mapping(
    input: &mut dyn DataInput,
    _version: i32,
) -> Result<std::collections::HashMap<String, CompoundFileEntry>> {
    let count = input.read_v_int()?;
    let mut mapping = std::collections::HashMap::with_capacity(count as usize);
    for _ in 0..count {
        let id = input.read_string()?;
        let offset = input.read_long()?;
        let length = input.read_long()?;
        mapping.insert(id, CompoundFileEntry { offset, length });
    }
    Ok(mapping)
}

impl CompoundDirectory for Lucene90CompoundReader {
    fn check_integrity(&self) -> Result<()> {
        let mut input = crate::store::MockIndexInput::new(
            self.data.to_vec(),
            format!("{} compound data", self.segment_name),
        );
        let _checksum = codec_util::checksum_entire_file(&mut input)?;
        Ok(())
    }
}

impl Directory for Lucene90CompoundReader {
    fn list_all(&self) -> Result<Vec<String>> {
        let mut names: Vec<String> = self
            .entries
            .keys()
            .map(|id| format!("{}{}", self.segment_name, id))
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
                    "No sub-file with id {id} found in compound file \"{}{}\"",
                    self.segment_name, COMPOUND_FILE_EXTENSION
                ),
            ))
        })?;
        let offset = entry.offset as usize;
        let length = entry.length as usize;
        let slice = self.data[offset..offset + length].to_vec();
        Ok(Box::new(crate::store::MockIndexInput::from_slice(
            &slice, name,
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
    use crate::codecs::codec_util;
    use crate::codecs::tests::test_segment_info;
    use crate::store::RamDirectory;

    fn write_dummy_file(dir: &RamDirectory, name: &str, codec: &str, payload: &[u8]) -> Result<()> {
        let mut out = dir.create_output(name, &*crate::store::DEFAULT_IO_CONTEXT)?;
        codec_util::write_index_header(out.as_mut(), codec, 0, &[0u8; 16], "")?;
        out.write_bytes(payload, 0, payload.len())?;
        codec_util::write_footer(out.as_mut())?;
        out.close()?;
        Ok(())
    }

    #[test]
    fn round_trip_small_files() {
        let dir = RamDirectory::default();
        let format = Lucene90CompoundFormat::new();
        let info = test_segment_info("_0", 10);
        info.set_files(HashSet::from([
            "_0.fdt".to_string(),
            "_0.fdx".to_string(),
            "_0.fnm".to_string(),
        ]));

        write_dummy_file(&dir, "_0.fdt", "DummyStoredFields", b"hello stored fields").unwrap();
        write_dummy_file(&dir, "_0.fdx", "DummyStoredFieldsIndex", b"index data").unwrap();
        write_dummy_file(&dir, "_0.fnm", "DummyFieldInfos", b"field info bytes").unwrap();

        format
            .write(&dir, &info, &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();

        let reader = format.get_compound_reader(&dir, &info).unwrap();

        let mut names = reader.list_all().unwrap();
        names.sort();
        assert_eq!(names, vec!["_0.fdt", "_0.fdx", "_0.fnm"]);

        assert_eq!(
            reader.file_length("_0.fdt").unwrap(),
            dir.file_length("_0.fdt").unwrap()
        );

        let mut input = reader
            .open_input("_0.fdt", &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        let len = input.length() as usize;
        let mut bytes = vec![0u8; len];
        input.read_bytes(&mut bytes, 0, len).unwrap();

        let mut original = dir
            .open_input("_0.fdt", &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        let orig_len = original.length() as usize;
        let mut orig_bytes = vec![0u8; orig_len];
        original.read_bytes(&mut orig_bytes, 0, orig_len).unwrap();

        assert_eq!(bytes, orig_bytes);
    }
}
