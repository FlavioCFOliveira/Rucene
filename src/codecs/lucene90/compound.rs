//! Lucene 9.0 compound-file format.
//!
//! Ports `Lucene90CompoundFormat` and `Lucene90CompoundReader` from Apache
//! Lucene Core 10.5.0.

#![deny(unsafe_code)]

use crate::util::{PriorityQueue, PriorityQueueComparator};
use std::collections::HashSet;

use crate::codecs::codec_util;
use crate::codecs::compound::CompoundFormat;
use crate::codecs::compound::{CompoundDirectory, CompoundFileEntry};
use crate::codecs::stub::SegmentInfo;
use crate::error::{LuceneError, Result};
use crate::index::index_file_names::{
    segment_file_name, strip_segment_name, COMPOUND_FILE_ENTRIES_EXTENSION, COMPOUND_FILE_EXTENSION,
};
use crate::store::{Directory, IOContext, IndexInput, IndexOutput, Lock};

const DATA_CODEC: &str = "Lucene90CompoundData";
const ENTRY_CODEC: &str = "Lucene90CompoundEntries";
const VERSION_START: i32 = 0;
const VERSION_CURRENT: i32 = VERSION_START;

/// Alignment applied to each embedded file's start offset.
///
/// Matches `Lucene90CompoundFormat.ALIGNMENT_BYTES`.
const ALIGNMENT_BYTES: i32 = 64;

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

/// A file to pack, with its length.
///
/// Equivalent to `Lucene90CompoundFormat.SizedFile`.
struct SizedFile {
    name: String,
    length: i64,
}

/// Orders files by ascending length, so that small files are more likely to
/// fit into one page.
///
/// Equivalent to `Lucene90CompoundFormat.SizedFileQueue`, a Lucene
/// `PriorityQueue` whose `lessThan` compares the lengths
/// (`Lucene90CompoundFormat.java:111-120`).
struct SizedFileQueue;

impl PriorityQueueComparator<SizedFile> for SizedFileQueue {
    fn less_than(&self, sf1: &SizedFile, sf2: &SizedFile) -> bool {
        sf1.length < sf2.length
    }
}

/// Equivalent to `Lucene90CompoundFormat.writeCompoundFile`
/// (`Lucene90CompoundFormat.java:122-171`): the files are queued in the
/// iteration order of the segment's file set and popped by ascending length,
/// each copied at a 64-byte aligned offset.
fn write_compound_file(
    entries: &mut dyn IndexOutput,
    data: &mut dyn IndexOutput,
    dir: &dyn Directory,
    segment_info: &SegmentInfo,
) -> Result<()> {
    let files = segment_info.files_ordered()?;
    let num_files = files.len();
    entries.write_v_int(num_files as i32)?;
    // first put files in ascending size order so small files fit more likely into one page
    let mut pq = PriorityQueue::new(num_files, SizedFileQueue)?;
    for filename in files.iter() {
        let length = dir.file_length(filename)?;
        pq.add(SizedFile {
            name: filename.clone(),
            length,
        });
    }
    while let Some(sized_file) = pq.pop() {
        let file = sized_file.name;
        // align file start offset
        let start_offset = data.align_file_pointer(ALIGNMENT_BYTES)?;
        // write bytes for file
        let mut input = dir.open_checksum_input(&file)?;

        // just copies the index header, verifying that its id matches what we expect
        codec_util::verify_and_copy_index_header(input.as_mut(), data, &segment_info.id())?;

        // copy all bytes except the footer
        let num_bytes_to_copy =
            input.length() - codec_util::footer_length() as i64 - input.file_pointer();
        data.copy_bytes(input.as_mut(), num_bytes_to_copy)?;

        // verify footer (checksum) matches for the incoming file we are copying
        let checksum = codec_util::check_footer(input.as_mut())?;

        // this is poached from CodecUtil.writeFooter, but we need to use our own
        // checksum, not data.getChecksum()
        codec_util::write_be_int(data, codec_util::FOOTER_MAGIC)?;
        codec_util::write_be_int(data, 0)?;
        codec_util::write_be_long(data, checksum)?;
        input.close()?;

        let end_offset = data.file_pointer();
        let length = end_offset - start_offset;

        // write entry for file
        entries.write_string(strip_segment_name(&file))?;
        entries.write_long(start_offset)?;
        entries.write_long(length)?;
    }
    Ok(())
}

/// Read-only directory view over a Lucene 9.0 compound file.
///
/// Equivalent to `org.apache.lucene.codecs.lucene90.Lucene90CompoundReader`:
/// the `.cfs` is opened once as `handle` and every sub-file is a slice of it.
pub struct Lucene90CompoundReader {
    segment_name: String,
    entries: CompoundEntries,
    handle: Box<dyn IndexInput>,
    /// How the directory this reader was opened from renders itself.
    ///
    /// Java keeps the `Directory` in a field and concatenates it in
    /// `toString()`, the only place it reads it. The reader cannot borrow the
    /// directory it was opened from beyond the constructor, so the rendering
    /// is captured instead — which is all `toString()` ever asked of it.
    directory_description: String,
}

impl std::fmt::Debug for Lucene90CompoundReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lucene90CompoundReader")
            .field("segment_name", &self.segment_name)
            .field("handle", &self.handle.resource_description())
            .finish_non_exhaustive()
    }
}

impl Lucene90CompoundReader {
    /// Opens the compound reader for the given segment.
    ///
    /// Equivalent to `Lucene90CompoundReader(Directory, SegmentInfo)`
    /// (`Lucene90CompoundReader.java:58-101`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::CorruptIndex`] when the entries file or the data
    /// file's header or footer structure is invalid, or when the data file's
    /// length is not the one the entries imply; propagates any I/O error.
    pub fn new(dir: &dyn Directory, segment_info: &SegmentInfo) -> Result<Self> {
        let segment_name = segment_info.name.clone();
        let directory_description = dir.directory_to_string();
        let data_file = segment_file_name(&segment_name, "", COMPOUND_FILE_EXTENSION);
        let entries_file = segment_file_name(&segment_name, "", COMPOUND_FILE_ENTRIES_EXTENSION);

        let (version, entries) = read_entries(&segment_info.id(), dir, &entries_file)?;

        // find the last FileEntry in the map (largest offset+length) and add length of codec footer:
        let expected_length = entries
            .mapping
            .values()
            .map(|e| e.offset + e.length)
            .max()
            .unwrap_or_else(|| codec_util::index_header_length(DATA_CODEC, "") as i64)
            + codec_util::footer_length() as i64;

        let mut handle = dir.open_input(&data_file, &*crate::store::DEFAULT_IO_CONTEXT)?;
        let checked = (|| -> Result<()> {
            codec_util::check_index_header(
                handle.as_mut(),
                DATA_CODEC,
                version,
                version,
                &segment_info.id(),
                "",
            )?;

            // NOTE: data file is too costly to verify checksum against all the bytes on open,
            // but for now we at least verify proper structure of the checksum footer: which looks
            // for FOOTER_MAGIC + algorithmID. This is cheap and can detect some forms of corruption
            // such as file truncation.
            codec_util::retrieve_checksum(handle.as_mut())?;

            // We also validate length, because e.g. if you strip 16 bytes off the .cfs we otherwise
            // would not detect it:
            if handle.length() != expected_length {
                return Err(LuceneError::CorruptIndex(format!(
                    "length should be {expected_length} bytes, but is {} instead (resource={})",
                    handle.length(),
                    handle.resource_description()
                )));
            }
            Ok(())
        })();
        if let Err(e) = checked {
            let _ = handle.close();
            return Err(e);
        }

        Ok(Self {
            segment_name,
            entries,
            handle,
            directory_description,
        })
    }
}

/// Reads the entries file, returning its format version and the mapping.
///
/// Equivalent to `Lucene90CompoundReader.readEntries`
/// (`Lucene90CompoundReader.java:103-129`), whose `checkFooter(entriesStream,
/// priorE)` also runs when reading the mapping failed.
fn read_entries(
    segment_id: &[u8],
    dir: &dyn Directory,
    entries_file: &str,
) -> Result<(i32, CompoundEntries)> {
    let mut entries_stream = dir.open_checksum_input(entries_file)?;
    let read = (|| -> Result<(i32, CompoundEntries)> {
        let version = codec_util::check_index_header(
            entries_stream.as_mut(),
            ENTRY_CODEC,
            VERSION_START,
            VERSION_CURRENT,
            segment_id,
            "",
        )?;
        Ok((version, read_mapping(entries_stream.as_mut())?))
    })();
    match read {
        Ok(read) => {
            codec_util::check_footer(entries_stream.as_mut())?;
            Ok(read)
        }
        Err(prior) => Err(codec_util::check_footer_with_prior(
            entries_stream.as_mut(),
            prior,
        )),
    }
}

/// Equivalent to `Lucene90CompoundReader.readMapping`
/// (`Lucene90CompoundReader.java:131-145`), which rejects a duplicate id.
///
/// The map is not pre-sized from the count: Java's
/// `HashMap.newHashMap(numEntries)` would size a table from a number read from
/// the file before any entry is verified, which a corrupt count turns into an
/// arbitrarily large allocation. Only the lookup structure differs; the order
/// Java's map would iterate is recomputed where it is observable, in
/// [`Lucene90CompoundReader`]'s file-not-found message.
fn read_mapping(entries_stream: &mut dyn IndexInput) -> Result<CompoundEntries> {
    let num_entries = entries_stream.read_v_int()?;
    let mut mapping = std::collections::HashMap::new();
    let mut ids = Vec::new();
    for _ in 0..num_entries {
        let id = entries_stream.read_string()?;
        if mapping.contains_key(&id) {
            // `new CorruptIndexException("Duplicate cfs entry id=" + id + " in CFS ", entriesStream)`
            return Err(LuceneError::CorruptIndex(format!(
                "Duplicate cfs entry id={id} in CFS  (resource={})",
                entries_stream.resource_description()
            )));
        }
        let offset = entries_stream.read_long()?;
        let length = entries_stream.read_long()?;
        mapping.insert(id.clone(), CompoundFileEntry { offset, length });
        ids.push(id);
    }
    Ok(CompoundEntries { mapping, ids })
}

/// The entries of a compound file: the lookup map, and the ids in the order
/// the entries file lists them, which is what Java's `HashMap` was filled in.
struct CompoundEntries {
    mapping: std::collections::HashMap<String, CompoundFileEntry>,
    ids: Vec<String>,
}

impl CompoundEntries {
    /// Renders `entries.keySet()` as Java does: `[a, b]` in the iteration
    /// order of the `HashMap.newHashMap(numEntries)` that `readMapping` fills
    /// in file order (`Lucene90CompoundReader.java:131-145`).
    fn java_key_set(&self) -> String {
        let capacity = (self.ids.len() as f64 / 0.75).ceil() as i32;
        let mut java_map =
            crate::util::java_collections::JavaHashMap::<()>::with_initial_capacity(capacity);
        for id in &self.ids {
            java_map.insert(id.clone(), ());
        }
        let keys: Vec<&str> = java_map.keys().map(String::as_str).collect();
        format!("[{}]", keys.join(", "))
    }
}

impl CompoundDirectory for Lucene90CompoundReader {
    /// Equivalent to `Lucene90CompoundReader.checkIntegrity()`:
    /// `CodecUtil.checksumEntireFile(handle)`, which reads a clone of the
    /// handle from the start.
    fn check_integrity(&self) -> Result<()> {
        let mut handle = self.handle.clone_input()?;
        codec_util::checksum_entire_file(handle.as_mut())?;
        Ok(())
    }
}

impl Directory for Lucene90CompoundReader {
    fn list_all(&self) -> Result<Vec<String>> {
        let mut names: Vec<String> = self
            .entries
            .mapping
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
        let id = strip_segment_name(name);
        // `if (e == null) throw new FileNotFoundException(name)`
        self.entries
            .mapping
            .get(id)
            .map(|e| e.length)
            .ok_or_else(|| {
                LuceneError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    name.to_string(),
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

    /// Equivalent to `Lucene90CompoundReader.openInput(String, IOContext)`
    /// (`Lucene90CompoundReader.java:152-171`): a slice of the handle.
    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        let id = strip_segment_name(name);
        let Some(entry) = self.entries.mapping.get(id) else {
            let dat_file_name = segment_file_name(&self.segment_name, "", COMPOUND_FILE_EXTENSION);
            return Err(LuceneError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "No sub-file with id {id} found in compound file \"{dat_file_name}\" (fileName={name} files: {})",
                    self.entries.java_key_set()
                ),
            )));
        };
        self.handle
            .slice_with_context(name, entry.offset, entry.length, context)
    }

    fn obtain_lock(&self, _name: &str) -> Result<Box<dyn Lock>> {
        Err(LuceneError::UnsupportedOperation(
            "obtainLock is not supported by CompoundDirectory".to_string(),
        ))
    }

    /// Equivalent to `Lucene90CompoundReader.close()`: `IOUtils.close(handle)`.
    fn close(&mut self) -> Result<()> {
        self.handle.close()
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

    fn write_dummy_file(
        dir: &dyn Directory,
        name: &str,
        codec: &str,
        payload: &[u8],
    ) -> Result<()> {
        let mut out = dir.create_output(name, &*crate::store::DEFAULT_IO_CONTEXT)?;
        codec_util::write_index_header(out.as_mut(), codec, 0, &[0u8; 16], "")?;
        out.write_bytes(payload, 0, payload.len())?;
        codec_util::write_footer(out.as_mut())?;
        out.close()?;
        Ok(())
    }

    /// Writes a three-file compound segment `_0` into `dir`.
    fn write_compound(dir: &dyn Directory) -> SegmentInfo {
        let info = test_segment_info("_0", 10);
        info.set_files(HashSet::from([
            "_0.fdt".to_string(),
            "_0.fdx".to_string(),
            "_0.fnm".to_string(),
        ]))
        .unwrap();
        write_dummy_file(dir, "_0.fdt", "DummyStoredFields", b"hello stored fields").unwrap();
        write_dummy_file(dir, "_0.fdx", "DummyStoredFieldsIndex", b"index data").unwrap();
        write_dummy_file(dir, "_0.fnm", "DummyFieldInfos", b"field info bytes").unwrap();
        Lucene90CompoundFormat::new()
            .write(dir, &info, &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        info
    }

    /// Replaces `name` in `dir` with `bytes`.
    fn overwrite(dir: &RamDirectory, name: &str, bytes: &[u8]) {
        dir.delete_file(name).unwrap();
        let mut out = dir
            .create_output(name, &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        out.write_bytes(bytes, 0, bytes.len()).unwrap();
        out.close().unwrap();
    }

    fn read_all(dir: &RamDirectory, name: &str) -> Vec<u8> {
        let mut input = dir
            .open_input(name, &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        let mut bytes = vec![0u8; input.length() as usize];
        let len = bytes.len();
        input.read_bytes(&mut bytes, 0, len).unwrap();
        bytes
    }

    /// The data file must be exactly as long as the entries imply: stripping
    /// bytes off the `.cfs` is a `CorruptIndexException`
    /// (`Lucene90CompoundReader.java:86-94`).
    #[test]
    fn a_truncated_data_file_is_rejected_on_open() {
        let dir = RamDirectory::default();
        let info = write_compound(&dir);
        let mut cfs = read_all(&dir, "_0.cfs");
        // Drop sixteen bytes before the footer, keeping a well-formed footer.
        let footer = cfs.split_off(cfs.len() - codec_util::footer_length() as usize);
        cfs.truncate(cfs.len() - 16);
        cfs.extend_from_slice(&footer);
        overwrite(&dir, "_0.cfs", &cfs);
        match Lucene90CompoundReader::new(&dir, &info) {
            Err(LuceneError::CorruptIndex(message)) => {
                assert!(message.starts_with("length should be "), "{message}")
            }
            other => panic!("expected CorruptIndex, got {other:?}"),
        }
    }

    /// A sub-file and its clones read the one `.cfs` handle, whose NIOFS
    /// channel every clone and slice shares (`NIOFSDirectory.java:95-123,
    /// 182`): once the handle is open, removing the file's name does not stop
    /// a clone from reading it.
    #[cfg(unix)]
    #[test]
    fn niofs_sub_file_clones_share_the_open_handle() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = crate::store::NIOFSDirectory::open(tmp.path()).unwrap();
        let info = write_compound(&dir);
        let reader = Lucene90CompoundReader::new(&dir, &info).unwrap();
        let input = reader
            .open_input("_0.fdt", &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        std::fs::remove_file(tmp.path().join("_0.cfs")).unwrap();
        let mut clone = input.clone_input().unwrap();
        codec_util::checksum_entire_file(clone.as_mut()).unwrap();
        let mut slice = input.slice("tail", 0, input.length()).unwrap();
        codec_util::checksum_entire_file(slice.as_mut()).unwrap();
    }

    /// A second entry with an id already read is a `CorruptIndexException`
    /// (`Lucene90CompoundReader.java:131-145`).
    #[test]
    fn a_duplicate_entry_id_is_rejected() {
        let dir = RamDirectory::default();
        let info = write_compound(&dir);
        dir.delete_file("_0.cfe").unwrap();
        let mut out = dir
            .create_output("_0.cfe", &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        codec_util::write_index_header(out.as_mut(), ENTRY_CODEC, VERSION_CURRENT, &info.id(), "")
            .unwrap();
        out.write_v_int(2).unwrap();
        for _ in 0..2 {
            out.write_string(".fnm").unwrap();
            out.write_long(64).unwrap();
            out.write_long(16).unwrap();
        }
        codec_util::write_footer(out.as_mut()).unwrap();
        out.close().unwrap();
        match Lucene90CompoundReader::new(&dir, &info) {
            Err(LuceneError::CorruptIndex(message)) => {
                // `"Duplicate cfs entry id=" + id + " in CFS "`, then
                // `CorruptIndexException`'s `" (resource=" + ... + ")"`.
                assert!(
                    message.starts_with("Duplicate cfs entry id=.fnm in CFS  (resource="),
                    "{message}"
                )
            }
            other => panic!("expected CorruptIndex, got {other:?}"),
        }
    }

    /// A missing sub-file is a `FileNotFoundException` that lists the ids in
    /// the iteration order of the `HashMap.newHashMap(numEntries)` filled in
    /// file order (`Lucene90CompoundReader.java:131-145, 155-169`). The files
    /// are sized so that they are packed `.fdx`, `.fnm`, `.fdt`, and the
    /// expected listing is the one OpenJDK 25 prints for
    /// `HashMap.newHashMap(3)` filled with those keys in that order — a
    /// different order from the file's.
    #[test]
    fn a_missing_sub_file_lists_the_ids_in_java_order() {
        let dir = RamDirectory::default();
        let info = test_segment_info("_0", 10);
        info.set_files(HashSet::from([
            "_0.fdt".to_string(),
            "_0.fdx".to_string(),
            "_0.fnm".to_string(),
        ]))
        .unwrap();
        write_dummy_file(&dir, "_0.fdx", "DummyStoredFieldsIndex", b"").unwrap();
        write_dummy_file(&dir, "_0.fnm", "DummyFieldInfos", &[7u8; 20]).unwrap();
        write_dummy_file(&dir, "_0.fdt", "DummyStoredFields", &[9u8; 40]).unwrap();
        Lucene90CompoundFormat::new()
            .write(&dir, &info, &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        let reader = Lucene90CompoundReader::new(&dir, &info).unwrap();
        let Err(err) = reader.open_input("_0.tim", &*crate::store::DEFAULT_IO_CONTEXT) else {
            panic!("a missing sub-file must not open")
        };
        let LuceneError::Io(io) = err else {
            panic!("expected a FileNotFoundException, got {err:?}")
        };
        assert_eq!(io.kind(), std::io::ErrorKind::NotFound);
        assert_eq!(
            io.to_string(),
            "No sub-file with id .tim found in compound file \"_0.cfs\" (fileName=_0.tim files: [.fnm, .fdx, .fdt])"
        );
        match reader.file_length("_0.tim") {
            Err(LuceneError::Io(io)) => assert_eq!(io.to_string(), "_0.tim"),
            other => panic!("expected a FileNotFoundException, got {other:?}"),
        }
    }

    /// Each sub-file is `handle.slice(name, offset, length, context)`
    /// (`Lucene90CompoundReader.java:152-171`): the slice is described by the
    /// sub-file's name within the data file, and `checkIntegrity` checksums
    /// the whole data file (`Lucene90CompoundReader.java:248-251`).
    #[test]
    fn sub_files_are_slices_of_the_data_file() {
        let dir = RamDirectory::default();
        let info = write_compound(&dir);
        let reader = Lucene90CompoundReader::new(&dir, &info).unwrap();
        reader.check_integrity().unwrap();
        let input = reader
            .open_input("_0.fnm", &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        assert!(
            input.resource_description().contains("[slice=_0.fnm]"),
            "{}",
            input.resource_description()
        );

        // A flipped payload byte inside the data file fails the whole-file
        // checksum.
        let mut cfs = read_all(&dir, "_0.cfs");
        let at = cfs.len() / 2;
        cfs[at] ^= 0xFF;
        overwrite(&dir, "_0.cfs", &cfs);
        let reader = Lucene90CompoundReader::new(&dir, &info).unwrap();
        assert!(reader.check_integrity().is_err());
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
        ]))
        .unwrap();

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

impl std::fmt::Display for Lucene90CompoundReader {
    /// Port of `Lucene90CompoundReader.toString()`, which names itself
    /// `CompoundFileDirectory` rather than after its own class and puts the
    /// segment name in double quotes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CompoundFileDirectory(segment=\"{}\" in dir={})",
            self.segment_name, self.directory_description
        )
    }
}
