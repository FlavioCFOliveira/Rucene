//! Segment collection and `segments_N` file management.
//!
//! Equivalent to `org.apache.lucene.index.SegmentInfos`.
//!
//! This module tracks the active set of segments in an index, assigns
//! generation numbers to the `segments_N` commit file, and serializes the
//! commit metadata in the exact byte format used by Apache Lucene Core 10.5.0.

#![deny(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::fmt::{self, Debug, Formatter};
use std::io;

use crate::codecs::codec_util;
use crate::error::{LuceneError, Result};
use crate::index::index_file_names::{file_name_from_generation, PENDING_SEGMENTS, SEGMENTS};
use crate::index::SegmentCommitInfo;
use crate::store::{Directory, IndexOutput};
use crate::util::string_helper::{StringHelper, ID_LENGTH};
use crate::util::Version;

/// Maximum number of documents Lucene allows in a single index.
///
/// Equivalent to `IndexWriter.MAX_DOCS`.
pub const MAX_DOCS: i32 = i32::MAX - 128;

/// Format version that introduced the current `segments_N` layout.
///
/// Equivalent to `SegmentInfos.VERSION_74`.
pub const VERSION_74: i32 = 9;

/// Format version that added per-commit `SegmentCommitInfo` IDs.
///
/// Equivalent to `SegmentInfos.VERSION_86`.
pub const VERSION_86: i32 = 10;

/// Format version written by this implementation.
///
/// Equivalent to `SegmentInfos.VERSION_CURRENT`.
pub const VERSION_CURRENT: i32 = VERSION_86;

/// Name of the legacy generation reference file used before Lucene 4.0.
///
/// Equivalent to `SegmentInfos.OLD_SEGMENTS_GEN`.
pub const OLD_SEGMENTS_GEN: &str = "segments.gen";

// -----------------------------------------------------------------------------
// SegmentInfos
// -----------------------------------------------------------------------------

/// Collection of `SegmentCommitInfo` objects with methods for operating on those
/// segments in relation to the file system.
///
/// Equivalent to `org.apache.lucene.index.SegmentInfos`.
pub struct SegmentInfos {
    /// Generation of the next `segments_N` file to be written.
    generation: i64,

    /// Generation of the last `segments_N` file successfully read or written.
    last_generation: i64,

    /// Used to name new segments.
    pub counter: i64,

    /// Counts how often the index has been changed.
    pub version: i64,

    /// Opaque user data attached to this commit.
    pub user_data: HashMap<String, String>,

    /// Segments in this commit, in order.
    segments: Vec<SegmentCommitInfo>,

    /// Unique id for this commit.
    id: [u8; ID_LENGTH],

    /// Lucene version that wrote this commit.
    lucene_version: Version,

    /// Version of the oldest segment in the index, or `None` if there are no
    /// segments.
    min_segment_lucene_version: Option<Version>,

    /// Lucene major version used to initially create the index.
    index_created_version_major: i32,

    /// `true` between `prepare_commit` and `finish_commit`/`rollback_commit`.
    pending_commit: bool,
}

impl SegmentInfos {
    /// Creates a new empty `SegmentInfos` for the given index creation major
    /// version.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if the creation version is out of
    /// the supported range.
    ///
    /// Equivalent to `SegmentInfos(int)`.
    pub fn new(index_created_version_major: i32) -> Result<Self> {
        if index_created_version_major > Version::LATEST.major as i32 {
            return Err(LuceneError::IllegalArgument(format!(
                "indexCreatedVersionMajor is in the future: {index_created_version_major}"
            )));
        }
        if index_created_version_major < 6 {
            return Err(LuceneError::IllegalArgument(format!(
                "indexCreatedVersionMajor must be >= 6, got: {index_created_version_major}"
            )));
        }
        Ok(Self {
            generation: -1,
            last_generation: -1,
            counter: 0,
            version: 0,
            user_data: HashMap::new(),
            segments: Vec::new(),
            id: StringHelper::random_id(),
            lucene_version: Version::LATEST,
            min_segment_lucene_version: None,
            index_created_version_major,
            pending_commit: false,
        })
    }

    /// Returns the `SegmentCommitInfo` at the provided index.
    pub fn info(&self, i: usize) -> &SegmentCommitInfo {
        &self.segments[i]
    }

    /// Returns the first `SegmentCommitInfo`, if any.
    pub fn first_info(&self) -> Option<&SegmentCommitInfo> {
        self.segments.first()
    }

    /// Returns the number of `SegmentCommitInfo`s.
    pub fn size(&self) -> usize {
        self.segments.len()
    }

    /// Returns `true` if there are no segments.
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Appends the provided `SegmentCommitInfo`.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if the wrapped `SegmentInfo` does
    /// not record `min_version` for an index created on or after Lucene 7.
    pub fn add(&mut self, si: SegmentCommitInfo) -> Result<()> {
        if self.index_created_version_major >= 7 && si.info.min_version().is_none() {
            return Err(LuceneError::IllegalArgument(format!(
                "All segments must record the minVersion for indices created on or after Lucene 7: {:?}",
                si.info
            )));
        }
        self.segments.push(si);
        Ok(())
    }

    /// Appends all provided `SegmentCommitInfo`s.
    pub fn add_all(&mut self, sis: impl IntoIterator<Item = SegmentCommitInfo>) -> Result<()> {
        for si in sis {
            self.add(si)?;
        }
        Ok(())
    }

    /// Clears all `SegmentCommitInfo`s.
    pub fn clear(&mut self) {
        self.segments.clear();
    }

    /// Removes the provided `SegmentCommitInfo`.
    ///
    /// # Warning
    ///
    /// O(N) cost.
    pub fn remove(&mut self, si: &SegmentCommitInfo) -> bool {
        if let Some(pos) = self.segments.iter().position(|s| s == si) {
            self.segments.remove(pos);
            true
        } else {
            false
        }
    }

    /// Removes the `SegmentCommitInfo` at the provided index.
    ///
    /// # Warning
    ///
    /// O(N) cost.
    pub fn remove_at(&mut self, index: usize) -> SegmentCommitInfo {
        self.segments.remove(index)
    }

    /// Returns `true` if the provided `SegmentCommitInfo` is contained.
    ///
    /// # Warning
    ///
    /// O(N) cost.
    pub fn contains(&self, si: &SegmentCommitInfo) -> bool {
        self.segments.iter().any(|s| s == si)
    }

    /// Returns the index of the provided `SegmentCommitInfo`, or `None`.
    ///
    /// # Warning
    ///
    /// O(N) cost.
    pub fn index_of(&self, si: &SegmentCommitInfo) -> Option<usize> {
        self.segments.iter().position(|s| s == si)
    }

    /// Returns an iterator over the contained segments.
    pub fn iter(&self) -> impl Iterator<Item = &SegmentCommitInfo> {
        self.segments.iter()
    }

    /// Returns a slice view of the contained segments.
    pub fn as_slice(&self) -> &[SegmentCommitInfo] {
        &self.segments
    }

    /// Returns the current generation.
    pub fn generation(&self) -> i64 {
        self.generation
    }

    /// Returns the last successfully read or written generation.
    pub fn last_generation(&self) -> i64 {
        self.last_generation
    }

    /// Returns the version number when this `SegmentInfos` was generated.
    pub fn version(&self) -> i64 {
        self.version
    }

    /// Returns the `segments_N` filename in use by this commit.
    pub fn segments_file_name(&self) -> Option<String> {
        file_name_from_generation(SEGMENTS, "", self.last_generation)
    }

    /// Returns the unique id for this commit.
    pub fn id(&self) -> [u8; ID_LENGTH] {
        self.id
    }

    /// Returns which Lucene version wrote this commit.
    pub fn commit_lucene_version(&self) -> Version {
        self.lucene_version
    }

    /// Returns the version of the oldest segment, or `None` if there are no
    /// segments.
    pub fn min_segment_lucene_version(&self) -> Option<Version> {
        self.min_segment_lucene_version
    }

    /// Returns the Lucene major version used to initially create the index.
    pub fn index_created_version_major(&self) -> i32 {
        self.index_created_version_major
    }

    /// Returns the user data attached to this commit.
    pub fn user_data(&self) -> &HashMap<String, String> {
        &self.user_data
    }

    /// Sets the commit user data.
    ///
    /// If `do_increment_version` is `true`, `changed()` is called.
    pub fn set_user_data(&mut self, data: HashMap<String, String>, do_increment_version: bool) {
        self.user_data = data;
        if do_increment_version {
            self.changed();
        }
    }

    /// Call this before committing if changes have been made to the segments.
    pub fn changed(&mut self) {
        self.version += 1;
    }

    /// Sets the version, failing if it would decrease.
    pub fn set_version(&mut self, new_version: i64) -> Result<()> {
        if new_version < self.version {
            return Err(LuceneError::IllegalArgument(format!(
                "newVersion (={new_version}) cannot be less than current version (={})",
                self.version
            )));
        }
        self.version = new_version;
        Ok(())
    }

    /// Carry over generation numbers from another `SegmentInfos`.
    pub fn update_generation(&mut self, other: &SegmentInfos) {
        self.last_generation = other.last_generation;
        self.generation = other.generation;
    }

    /// Carry over generation numbers, version and counter from another
    /// `SegmentInfos`.
    pub fn update_generation_version_and_counter(&mut self, other: &SegmentInfos) {
        self.update_generation(other);
        self.version = other.version;
        self.counter = other.counter;
    }

    /// Sets the generation to be used for the next commit.
    pub fn set_next_write_generation(&mut self, generation: i64) -> Result<()> {
        if generation < self.generation {
            return Err(LuceneError::IllegalState(format!(
                "cannot decrease generation to {generation} from current generation {}",
                self.generation
            )));
        }
        self.generation = generation;
        Ok(())
    }

    /// Returns the sum of all segment `max_doc` values.
    ///
    /// Deletions are not included.
    pub fn total_max_doc(&self) -> i32 {
        let count: i64 = self
            .segments
            .iter()
            .map(|s| s.info.max_doc().unwrap_or(0) as i64)
            .sum();
        count.min(i32::MAX as i64) as i32
    }

    /// Returns all file names referenced by this commit.
    ///
    /// # Arguments
    ///
    /// * `include_segments_file` - if `true`, the committed `segments_N` file is
    ///   included.
    pub fn files(&self, include_segments_file: bool) -> Result<HashSet<String>> {
        let mut files = HashSet::new();
        if include_segments_file {
            if let Some(segment_file_name) = self.segments_file_name() {
                files.insert(segment_file_name);
            }
        }
        for info in &self.segments {
            for file in info.files()? {
                files.insert(file);
            }
        }
        Ok(files)
    }

    /// Replaces all segments, but keeps generation, version and counter so that
    /// future commits remain write-once.
    pub fn replace(&mut self, other: &SegmentInfos) {
        self.rollback_segment_infos(other.as_slice());
        self.last_generation = other.last_generation;
        self.user_data.clone_from(&other.user_data);
    }

    fn rollback_segment_infos(&mut self, infos: &[SegmentCommitInfo]) {
        self.clear();
        self.segments.extend_from_slice(infos);
    }

    /// Returns the generation of the most recent commit in the provided file
    /// list.
    ///
    /// Equivalent to `SegmentInfos.getLastCommitGeneration(String[])`.
    pub fn get_last_commit_generation(files: &[String]) -> i64 {
        let mut max_gen = -1i64;
        for file in files {
            if file.starts_with(SEGMENTS) && !file.starts_with(OLD_SEGMENTS_GEN) {
                if let Ok(gen) = Self::generation_from_segments_file_name(file) {
                    if gen > max_gen {
                        max_gen = gen;
                    }
                }
            }
        }
        max_gen
    }

    /// Returns the generation of the most recent commit in the directory.
    ///
    /// Equivalent to `SegmentInfos.getLastCommitGeneration(Directory)`.
    pub fn get_last_commit_generation_dir(directory: &dyn Directory) -> Result<i64> {
        Ok(Self::get_last_commit_generation(&directory.list_all()?))
    }

    /// Returns the filename of the most recent commit in the provided file list.
    ///
    /// Equivalent to `SegmentInfos.getLastCommitSegmentsFileName(String[])`.
    pub fn get_last_commit_segments_file_name(files: &[String]) -> Option<String> {
        file_name_from_generation(SEGMENTS, "", Self::get_last_commit_generation(files))
    }

    /// Returns the filename of the most recent commit in the directory.
    ///
    /// Equivalent to `SegmentInfos.getLastCommitSegmentsFileName(Directory)`.
    pub fn get_last_commit_segments_file_name_dir(
        directory: &dyn Directory,
    ) -> Result<Option<String>> {
        Ok(Self::get_last_commit_segments_file_name(
            &directory.list_all()?,
        ))
    }

    /// Parses the generation from a `segments_N` filename.
    ///
    /// Equivalent to `SegmentInfos.generationFromSegmentsFileName(String)`.
    pub fn generation_from_segments_file_name(file_name: &str) -> Result<i64> {
        if file_name == OLD_SEGMENTS_GEN {
            return Err(LuceneError::IllegalArgument(format!(
                "\"{OLD_SEGMENTS_GEN}\" is not a valid segment file name since 4.0"
            )));
        }
        if file_name == SEGMENTS {
            return Ok(0);
        }
        if file_name.starts_with(SEGMENTS) {
            let gen_str = &file_name[1 + SEGMENTS.len()..];
            i64::from_str_radix(gen_str, 36).map_err(|e| {
                LuceneError::IllegalArgument(format!(
                    "fileName \"{file_name}\" does not contain a valid generation: {e}"
                ))
            })
        } else {
            Err(LuceneError::IllegalArgument(format!(
                "fileName \"{file_name}\" is not a segments file"
            )))
        }
    }

    fn next_pending_generation(&self) -> i64 {
        if self.generation == -1 {
            1
        } else {
            self.generation + 1
        }
    }

    /// Reads the named commit file from the directory.
    ///
    /// Equivalent to `SegmentInfos.readCommit(Directory, String)`.
    pub fn read_commit(directory: &dyn Directory, segment_file_name: &str) -> Result<Self> {
        Self::read_commit_with_min_version(
            directory,
            segment_file_name,
            Version::MIN_SUPPORTED_MAJOR,
        )
    }

    /// Reads the named commit file, enforcing a minimum supported major
    /// version.
    ///
    /// Equivalent to `SegmentInfos.readCommit(Directory, String, int)`.
    pub fn read_commit_with_min_version(
        directory: &dyn Directory,
        segment_file_name: &str,
        min_supported_major_version: i32,
    ) -> Result<Self> {
        let generation = Self::generation_from_segments_file_name(segment_file_name)?;
        let mut input = directory.open_checksum_input(segment_file_name)?;
        Self::read_commit_input(
            directory,
            input.as_mut(),
            generation,
            min_supported_major_version,
        )
    }

    /// Reads a commit from an already opened checksum input.
    ///
    /// Equivalent to `SegmentInfos.readCommit(Directory, ChecksumIndexInput, long, int)`.
    pub fn read_commit_input(
        directory: &dyn Directory,
        input: &mut dyn crate::store::ChecksumIndexInput,
        generation: i64,
        min_supported_major_version: i32,
    ) -> Result<Self> {
        let result = (|| {
            let magic = codec_util::read_be_int(input)?;
            if magic != codec_util::CODEC_MAGIC {
                return Err(LuceneError::IndexFormatNotSupported(format!(
                    "index format too old: magic={magic}, expected={}",
                    codec_util::CODEC_MAGIC
                )));
            }
            let format =
                codec_util::check_header_no_magic(input, "segments", VERSION_74, VERSION_CURRENT)?;

            let mut id = [0u8; ID_LENGTH];
            input.read_bytes(&mut id, 0, ID_LENGTH)?;
            codec_util::check_index_header_suffix(input, &radix_36(generation as u64))?;

            let lucene_version = Version::from_bits(
                input.read_v_int()? as u8,
                input.read_v_int()? as u8,
                input.read_v_int()? as u8,
            )?;
            let index_created_version = input.read_v_int()?;
            if (lucene_version.major as i32) > index_created_version {
                // continue
            } else if (lucene_version.major as i32) < index_created_version {
                return Err(LuceneError::CorruptIndex(format!(
                    "Creation version [{index_created_version}.x] can't be greater than the version that wrote the segment infos: [{lucene_version}]"
                )));
            }

            let mut infos = Self::new(index_created_version)?;
            infos.id = id;
            infos.generation = generation;
            infos.last_generation = generation;
            infos.lucene_version = lucene_version;
            Self::parse_segment_infos(
                directory,
                input,
                &mut infos,
                format,
                min_supported_major_version,
            )?;
            Ok(infos)
        })();

        codec_util::check_footer(input)?;
        result
    }

    fn parse_segment_infos(
        directory: &dyn Directory,
        input: &mut dyn crate::store::DataInput,
        infos: &mut SegmentInfos,
        format: i32,
        min_supported_major_version: i32,
    ) -> Result<()> {
        infos.version = codec_util::read_be_long(input)?;
        infos.counter = input.read_v_long()?;
        let num_segments = codec_util::read_be_int(input)?;
        if num_segments < 0 {
            return Err(LuceneError::CorruptIndex(format!(
                "invalid segment count: {num_segments}"
            )));
        }

        if num_segments > 0 {
            infos.min_segment_lucene_version = Some(Version::from_bits(
                input.read_v_int()? as u8,
                input.read_v_int()? as u8,
                input.read_v_int()? as u8,
            )?);
        }

        let mut total_docs: i64 = 0;

        for _ in 0..num_segments {
            let seg_name = input.read_string()?;
            let mut segment_id = [0u8; ID_LENGTH];
            input.read_bytes(&mut segment_id, 0, ID_LENGTH)?;
            let codec_name = input.read_string()?;
            let codec = crate::codecs::for_name(&codec_name).ok_or_else(|| {
                LuceneError::IndexFormatNotSupported(format!(
                    "Could not load codec '{codec_name}'. Did you forget to add backward-codecs?"
                ))
            })?;

            let mut info = codec.segment_info_format().read(
                directory,
                &seg_name,
                &segment_id,
                &*crate::store::READONCE_IO_CONTEXT,
            )?;
            info.set_codec(codec);

            let max_doc = info.max_doc()?;
            total_docs += max_doc as i64;

            let del_gen = codec_util::read_be_long(input)?;
            let del_count = codec_util::read_be_int(input)?;
            if del_count < 0 || del_count > max_doc {
                return Err(LuceneError::CorruptIndex(format!(
                    "invalid deletion count: {del_count} vs maxDoc={max_doc}"
                )));
            }

            let field_infos_gen = codec_util::read_be_long(input)?;
            let doc_values_gen = codec_util::read_be_long(input)?;
            let soft_del_count = codec_util::read_be_int(input)?;
            if soft_del_count < 0 || soft_del_count > max_doc {
                return Err(LuceneError::CorruptIndex(format!(
                    "invalid soft deletion count: {soft_del_count} vs maxDoc={max_doc}"
                )));
            }
            if (soft_del_count as i64 + del_count as i64) > max_doc as i64 {
                return Err(LuceneError::CorruptIndex(format!(
                    "invalid deletion count: {} vs maxDoc={max_doc}",
                    soft_del_count + del_count
                )));
            }

            let sci_id: [u8; ID_LENGTH] = if format > VERSION_74 {
                let marker = input.read_byte()?;
                match marker {
                    1 => {
                        let mut buf = [0u8; ID_LENGTH];
                        input.read_bytes(&mut buf, 0, ID_LENGTH)?;
                        buf
                    }
                    0 => StringHelper::random_id(),
                    _ => {
                        return Err(LuceneError::CorruptIndex(format!(
                            "invalid SegmentCommitInfo ID marker: {marker}"
                        )))
                    }
                }
            } else {
                StringHelper::random_id()
            };

            let mut si_per_commit = SegmentCommitInfo::new(
                info,
                del_count,
                soft_del_count,
                del_gen,
                field_infos_gen,
                doc_values_gen,
                sci_id,
            )?;

            let field_infos_files = input.read_set_of_strings()?;
            si_per_commit.set_field_infos_files(field_infos_files);

            let num_dv_fields = codec_util::read_be_int(input)?;
            let mut dv_updates_files: HashMap<i32, HashSet<String>> = HashMap::new();
            for _ in 0..num_dv_fields {
                let field_number = codec_util::read_be_int(input)?;
                let files = input.read_set_of_strings()?;
                dv_updates_files.insert(field_number, files);
            }
            si_per_commit.set_doc_values_updates_files(dv_updates_files);

            infos.add(si_per_commit)?;

            let segment_version = infos.segments.last().unwrap().info.version();
            if let Some(min_version) = infos.min_segment_lucene_version {
                if !segment_version.on_or_after(&min_version) {
                    return Err(LuceneError::CorruptIndex(format!(
                        "segments file recorded minSegmentLuceneVersion={min_version} but segment={seg_name} has older version={segment_version}"
                    )));
                }
            }

            if infos.index_created_version_major >= 7
                && (segment_version.major as i32) < infos.index_created_version_major
            {
                return Err(LuceneError::CorruptIndex(format!(
                    "segments file recorded indexCreatedVersionMajor={} but segment={seg_name} has older version={segment_version}",
                    infos.index_created_version_major
                )));
            }

            if infos.index_created_version_major >= 7
                && infos.segments.last().unwrap().info.min_version().is_none()
            {
                return Err(LuceneError::CorruptIndex(format!(
                    "segments infos must record minVersion with indexCreatedVersionMajor={}",
                    infos.index_created_version_major
                )));
            }

            let created_or_segment_min_version = infos
                .segments
                .last()
                .unwrap()
                .info
                .min_version()
                .map_or(infos.index_created_version_major, |v| v.major as i32);

            if infos.segments.last().unwrap().info.min_version().is_none()
                || (infos
                    .segments
                    .last()
                    .unwrap()
                    .info
                    .min_version()
                    .unwrap()
                    .major as i32)
                    < min_supported_major_version
            {
                return Err(LuceneError::IndexFormatNotSupported(format!(
                    "Index has segments derived from Lucene version {created_or_segment_min_version}.x and is not supported by Lucene {}. This Lucene version only supports indexes with major version {min_supported_major_version} or later (found: {created_or_segment_min_version}, minimum supported: {min_supported_major_version}).",
                    Version::LATEST
                )));
            }
        }

        infos.user_data = input.read_map_of_strings()?;

        if total_docs > MAX_DOCS as i64 {
            return Err(LuceneError::CorruptIndex(format!(
                "Too many documents: an index cannot exceed {MAX_DOCS} but readers have total maxDoc={total_docs}"
            )));
        }

        Ok(())
    }

    /// Finds the latest commit and loads all `SegmentCommitInfo`s.
    ///
    /// Equivalent to `SegmentInfos.readLatestCommit(Directory)`.
    pub fn read_latest_commit(directory: &dyn Directory) -> Result<Self> {
        Self::read_latest_commit_with_min_version(directory, Version::MIN_SUPPORTED_MAJOR)
    }

    /// Finds the latest commit, enforcing a minimum supported major version.
    ///
    /// Equivalent to `SegmentInfos.readLatestCommit(Directory, int)`.
    pub fn read_latest_commit_with_min_version(
        directory: &dyn Directory,
        min_supported_major_version: i32,
    ) -> Result<Self> {
        let mut last_gen = -1i64;
        let mut exc: Option<LuceneError> = None;

        loop {
            let files = directory.list_all()?;
            let files2 = directory.list_all()?;
            if files != files2 {
                // Directory listing changed between calls; retry.
                continue;
            }
            let gen = Self::get_last_commit_generation(&files);

            if gen == -1 {
                return Err(LuceneError::Io(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!(
                        "no segments* file found in {}: files: {:?}",
                        directory.directory_type_name(),
                        files
                    ),
                )));
            }

            if gen > last_gen {
                last_gen = gen;
                let segment_file_name =
                    file_name_from_generation(SEGMENTS, "", gen).ok_or_else(|| {
                        LuceneError::IllegalState(format!(
                            "could not build segments file name for generation {gen}"
                        ))
                    })?;
                match Self::read_commit_with_min_version(
                    directory,
                    &segment_file_name,
                    min_supported_major_version,
                ) {
                    Ok(infos) => return Ok(infos),
                    Err(err) => {
                        if exc.is_none() {
                            exc = Some(err);
                        }
                    }
                }
            } else {
                return Err(exc.unwrap_or_else(|| {
                    LuceneError::Io(io::Error::new(
                        io::ErrorKind::NotFound,
                        "no segments* file found".to_string(),
                    ))
                }));
            }
        }
    }

    /// Writes this commit to a new `pending_segments_N` file in the directory.
    ///
    /// This is the first half of a two-phase commit; call `finish_commit` to
    /// make it visible as `segments_N`.
    ///
    /// Equivalent to `SegmentInfos.commit(Directory)`.
    pub fn commit(&mut self, directory: &dyn Directory) -> Result<String> {
        self.prepare_commit(directory)?;
        self.finish_commit(directory)
    }

    /// Starts a commit by writing a pending segments file.
    ///
    /// Equivalent to `SegmentInfos.prepareCommit(Directory)`.
    pub fn prepare_commit(&mut self, directory: &dyn Directory) -> Result<()> {
        if self.pending_commit {
            return Err(LuceneError::IllegalState(
                "prepareCommit was already called".to_string(),
            ));
        }
        directory.sync_metadata()?;
        self.write(directory)
    }

    fn write(&mut self, directory: &dyn Directory) -> Result<()> {
        let next_generation = self.next_pending_generation();
        let segment_file_name = file_name_from_generation(PENDING_SEGMENTS, "", next_generation)
            .ok_or_else(|| {
                LuceneError::IllegalState(format!(
                    "could not build pending segments file name for generation {next_generation}"
                ))
            })?;

        // Always advance the generation on write.
        self.generation = next_generation;

        let mut output =
            directory.create_output(&segment_file_name, &*crate::store::DEFAULT_IO_CONTEXT)?;
        let result = self.write_to_output(output.as_mut());
        match result {
            Ok(_) => {
                output.close()?;
                directory.sync(std::slice::from_ref(&segment_file_name))?;
                self.pending_commit = true;
                Ok(())
            }
            Err(err) => {
                let _ = output.close();
                let _ = directory.delete_file(&segment_file_name);
                Err(err)
            }
        }
    }

    /// Writes this commit to the provided output.
    ///
    /// Equivalent to `SegmentInfos.write(IndexOutput)`.
    pub fn write_to_output(&self, out: &mut dyn IndexOutput) -> Result<()> {
        codec_util::write_index_header(
            out,
            "segments",
            VERSION_CURRENT,
            &self.id,
            &radix_36(self.generation as u64),
        )?;

        let latest = Version::LATEST;
        out.write_v_int(latest.major as i32)?;
        out.write_v_int(latest.minor as i32)?;
        out.write_v_int(latest.bugfix as i32)?;
        out.write_v_int(self.index_created_version_major)?;

        codec_util::write_be_long(out, self.version)?;
        out.write_v_long(self.counter)?;
        codec_util::write_be_int(out, self.segments.len() as i32)?;

        if !self.segments.is_empty() {
            let min_segment_version = self
                .segments
                .iter()
                .map(|s| s.info.version())
                .min()
                .unwrap();
            out.write_v_int(min_segment_version.major as i32)?;
            out.write_v_int(min_segment_version.minor as i32)?;
            out.write_v_int(min_segment_version.bugfix as i32)?;
        }

        for si_per_commit in &self.segments {
            let si = &si_per_commit.info;
            if self.index_created_version_major >= 7 && si.min_version().is_none() {
                return Err(LuceneError::IllegalState(format!(
                    "Segments must record minVersion if they have been created on or after Lucene 7: {si:?}"
                )));
            }

            out.write_string(&si.name)?;
            let segment_id = si.id();
            if segment_id.len() != ID_LENGTH {
                return Err(LuceneError::IllegalState(format!(
                    "cannot write segment: invalid id segment={} id={:?}",
                    si.name, segment_id
                )));
            }
            out.write_bytes(&segment_id, 0, segment_id.len())?;
            let codec_name = si
                .codec()
                .ok_or_else(|| {
                    LuceneError::IllegalState(format!(
                        "cannot write segment: codec not set for {si:?}"
                    ))
                })?
                .name()
                .to_string();
            out.write_string(&codec_name)?;

            codec_util::write_be_long(out, si_per_commit.get_del_gen())?;
            let del_count = si_per_commit.get_del_count();
            let max_doc = si.max_doc()?;
            if del_count < 0 || del_count > max_doc {
                return Err(LuceneError::IllegalState(format!(
                    "cannot write segment: invalid maxDoc segment={} maxDoc={max_doc} delCount={del_count}",
                    si.name
                )));
            }
            codec_util::write_be_int(out, del_count)?;
            codec_util::write_be_long(out, si_per_commit.get_field_infos_gen())?;
            codec_util::write_be_long(out, si_per_commit.get_doc_values_gen())?;
            let soft_del_count = si_per_commit.get_soft_del_count();
            if soft_del_count < 0 || soft_del_count > max_doc {
                return Err(LuceneError::IllegalState(format!(
                    "cannot write segment: invalid maxDoc segment={} maxDoc={max_doc} softDelCount={soft_del_count}",
                    si.name
                )));
            }
            codec_util::write_be_int(out, soft_del_count)?;

            // SegmentCommitInfo id is always written (matches Java's
            // VERSION_CURRENT behaviour where IndexWriter supplies a non-null
            // id).
            out.write_byte(1)?;
            let sci_id = si_per_commit.id();
            out.write_bytes(&sci_id, 0, sci_id.len())?;

            out.write_set_of_strings(si_per_commit.get_field_infos_files())?;
            let dv_updates_files = si_per_commit.get_doc_values_updates_files();
            codec_util::write_be_int(out, dv_updates_files.len() as i32)?;
            for (field_number, files) in dv_updates_files {
                codec_util::write_be_int(out, *field_number)?;
                out.write_set_of_strings(files)?;
            }
        }

        out.write_map_of_strings(&self.user_data)?;
        codec_util::write_footer(out)?;
        Ok(())
    }

    /// Completes a pending commit by renaming `pending_segments_N` to
    /// `segments_N`.
    ///
    /// Equivalent to `SegmentInfos.finishCommit(Directory)`.
    pub fn finish_commit(&mut self, directory: &dyn Directory) -> Result<String> {
        if !self.pending_commit {
            return Err(LuceneError::IllegalState(
                "prepareCommit was not called".to_string(),
            ));
        }

        let src =
            file_name_from_generation(PENDING_SEGMENTS, "", self.generation).ok_or_else(|| {
                LuceneError::IllegalState(format!(
                    "could not build pending segments file name for generation {}",
                    self.generation
                ))
            })?;
        let dest = file_name_from_generation(SEGMENTS, "", self.generation).ok_or_else(|| {
            LuceneError::IllegalState(format!(
                "could not build segments file name for generation {}",
                self.generation
            ))
        })?;

        let mut success_rename_and_sync = false;
        let result: Result<()> = (|| {
            directory.rename(&src, &dest)?;
            directory.sync_metadata()?;
            success_rename_and_sync = true;
            Ok(())
        })();

        if result.is_err() && !success_rename_and_sync {
            let _ = directory.delete_file(&dest);
        }

        if result.is_err() {
            self.rollback_commit(directory);
        }

        result?;
        self.pending_commit = false;
        self.last_generation = self.generation;
        Ok(dest)
    }

    /// Aborts a pending commit, deleting the pending segments file.
    ///
    /// Equivalent to `SegmentInfos.rollbackCommit(Directory)`.
    pub fn rollback_commit(&mut self, directory: &dyn Directory) {
        if self.pending_commit {
            self.pending_commit = false;
            let pending = file_name_from_generation(PENDING_SEGMENTS, "", self.generation);
            if let Some(pending) = pending {
                let _ = directory.delete_file(&pending);
            }
        }
    }
}

impl Clone for SegmentInfos {
    fn clone(&self) -> Self {
        Self {
            generation: self.generation,
            last_generation: self.last_generation,
            counter: self.counter,
            version: self.version,
            user_data: self.user_data.clone(),
            segments: self.segments.to_vec(),
            id: self.id,
            lucene_version: self.lucene_version,
            min_segment_lucene_version: self.min_segment_lucene_version,
            index_created_version_major: self.index_created_version_major,
            pending_commit: false,
        }
    }
}

impl Debug for SegmentInfos {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SegmentInfos")
            .field("generation", &self.generation)
            .field("last_generation", &self.last_generation)
            .field("counter", &self.counter)
            .field("version", &self.version)
            .field("size", &self.segments.len())
            .field(
                "index_created_version_major",
                &self.index_created_version_major,
            )
            .finish_non_exhaustive()
    }
}

fn radix_36(value: u64) -> String {
    if value == 0 {
        "0".to_string()
    } else {
        let mut chars: Vec<char> = Vec::new();
        let mut v = value;
        while v > 0 {
            let digit = (v % 36) as u32;
            chars.push(std::char::from_digit(digit, 36).expect("radix 36 digits are always valid"));
            v /= 36;
        }
        chars.reverse();
        chars.into_iter().collect()
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::lucene99::Lucene99SegmentInfoFormat;
    use crate::codecs::tests::{test_segment_commit_info, test_segment_info, DummyCodec};
    use crate::codecs::{Codec, FilterCodec, SegmentInfoFormat};
    use crate::index::index_file_names::{file_name_from_generation, SEGMENTS};
    use crate::index::SegmentCommitInfo;
    use crate::search::{Sort, SortField, SortFieldType};
    use crate::store::{Directory, RamDirectory};
    use crate::util::string_helper::StringHelper;
    use std::collections::HashSet;
    use std::sync::Arc;

    fn test_codec() -> Arc<dyn Codec> {
        static REGISTER: std::sync::Once = std::sync::Once::new();
        let inner: Arc<dyn Codec> = Arc::new(DummyCodec::new("Dummy"));
        let codec: Arc<dyn Codec> = Arc::new(
            FilterCodec::new("TestCodec", Arc::clone(&inner))
                .with_segment_info_format(Lucene99SegmentInfoFormat::new()),
        );
        REGISTER.call_once(|| {
            let registered = FilterCodec::new("TestCodec", Arc::new(DummyCodec::new("Dummy")))
                .with_segment_info_format(Lucene99SegmentInfoFormat::new());
            let _ = crate::codecs::register_codec("TestCodec", registered);
        });
        codec
    }

    fn write_segment_info_file(
        directory: &dyn Directory,
        info: &crate::index::SegmentInfo,
    ) -> Result<()> {
        let format = Lucene99SegmentInfoFormat::new();
        format.write(directory, info, &*crate::store::DEFAULT_IO_CONTEXT)
    }

    #[test]
    fn segment_infos_basic() {
        let mut sis = SegmentInfos::new(10).unwrap();
        assert_eq!(sis.size(), 0);
        assert_eq!(sis.total_max_doc(), 0);
        assert_eq!(sis.index_created_version_major(), 10);
        sis.changed();
        assert_eq!(sis.version(), 1);
    }

    #[test]
    fn segment_infos_version_validation() {
        assert!(SegmentInfos::new(5).is_err());
        assert!(SegmentInfos::new(11).is_err());
        assert!(SegmentInfos::new(10).is_ok());
    }

    #[test]
    fn segment_infos_add_and_remove() {
        let mut sis = SegmentInfos::new(10).unwrap();
        let sci = test_segment_commit_info("_0", 10);
        sis.add(sci.clone()).unwrap();
        assert_eq!(sis.size(), 1);
        assert!(sis.contains(&sci));
        assert!(sis.remove(&sci));
        assert_eq!(sis.size(), 0);
    }

    #[test]
    fn segment_infos_first_info() {
        let mut sis = SegmentInfos::new(10).unwrap();
        assert!(sis.first_info().is_none());
        let sci = test_segment_commit_info("_0", 10);
        sis.add(sci.clone()).unwrap();
        assert_eq!(sis.first_info().unwrap().info.name, "_0");
    }

    #[test]
    fn segment_infos_update_generation() {
        let mut a = SegmentInfos::new(10).unwrap();
        a.set_next_write_generation(5).unwrap();
        a.last_generation = 3;
        let mut b = SegmentInfos::new(10).unwrap();
        b.update_generation(&a);
        assert_eq!(b.generation(), 5);
        assert_eq!(b.last_generation(), 3);

        let mut c = SegmentInfos::new(10).unwrap();
        c.update_generation_version_and_counter(&a);
        assert_eq!(c.generation(), a.generation());
        assert_eq!(c.version(), a.version());
        assert_eq!(c.counter, a.counter);
    }

    #[test]
    fn generation_math_matches_index_file_names() {
        assert_eq!(
            SegmentInfos::generation_from_segments_file_name(SEGMENTS).unwrap(),
            0
        );
        assert_eq!(
            SegmentInfos::generation_from_segments_file_name("segments_1").unwrap(),
            1
        );
        assert_eq!(
            SegmentInfos::generation_from_segments_file_name("segments_z").unwrap(),
            35
        );
        assert_eq!(
            SegmentInfos::generation_from_segments_file_name("segments_10").unwrap(),
            36
        );

        assert_eq!(
            file_name_from_generation(SEGMENTS, "", 0).unwrap(),
            "segments"
        );
        assert_eq!(
            file_name_from_generation(SEGMENTS, "", 1).unwrap(),
            "segments_1"
        );
        assert_eq!(
            file_name_from_generation(SEGMENTS, "", 36).unwrap(),
            "segments_10"
        );

        let files = vec![
            "_0.fnm".to_string(),
            "segments".to_string(),
            "segments_5".to_string(),
            "pending_segments_6".to_string(),
        ];
        assert_eq!(SegmentInfos::get_last_commit_generation(&files), 5);
        assert_eq!(
            SegmentInfos::get_last_commit_segments_file_name(&files),
            Some("segments_5".to_string())
        );
    }

    #[test]
    fn round_trip_empty_segments_file() {
        let dir = RamDirectory::default();
        let mut sis = SegmentInfos::new(10).unwrap();
        sis.user_data = HashMap::from([("foo".to_string(), "bar".to_string())]);

        let written = sis.commit(&dir).unwrap();
        assert_eq!(written, "segments_1");

        let read = SegmentInfos::read_latest_commit(&dir).unwrap();
        assert_eq!(read.size(), 0);
        assert_eq!(read.version(), sis.version());
        assert_eq!(read.counter, sis.counter);
        assert_eq!(read.user_data().get("foo"), Some(&"bar".to_string()));
        assert_eq!(read.last_generation(), 1);
    }

    #[test]
    fn round_trip_single_segment() {
        let dir = RamDirectory::default();
        let mut info = test_segment_info("_0", 42);
        info.set_codec(test_codec());
        info.set_index_sort(
            Sort::new_fields(vec![SortField::new(
                Some("id".to_string()),
                SortFieldType::String,
            )
            .unwrap()])
            .unwrap(),
        );
        info.set_diagnostics(HashMap::from([("source".to_string(), "test".to_string())]));
        info.set_files(HashSet::from(["_0.fnm".to_string(), "_0.fdt".to_string()]));

        write_segment_info_file(&dir, &info).unwrap();

        let sci = SegmentCommitInfo::new(info, 2, 1, 0, 1, 2, StringHelper::random_id()).unwrap();
        let mut sis = SegmentInfos::new(10).unwrap();
        sis.counter = 1;
        sis.changed();
        sis.user_data = HashMap::from([("user".to_string(), "data".to_string())]);
        sis.add(sci).unwrap();

        let written = sis.commit(&dir).unwrap();
        assert_eq!(written, "segments_1");

        let read = SegmentInfos::read_latest_commit(&dir).unwrap();
        assert_eq!(read.size(), 1);
        assert_eq!(read.counter, 1);
        assert_eq!(read.version(), sis.version());
        assert_eq!(read.total_max_doc(), 42);
        assert_eq!(read.user_data().get("user"), Some(&"data".to_string()));

        let read_sci = read.info(0);
        assert_eq!(read_sci.info.name, "_0");
        assert_eq!(read_sci.get_del_count(), 2);
        assert_eq!(read_sci.get_soft_del_count(), 1);
        assert_eq!(read_sci.get_del_gen(), 0);
        assert_eq!(read_sci.get_field_infos_gen(), 1);
        assert_eq!(read_sci.get_doc_values_gen(), 2);
    }

    #[test]
    fn pending_commit_rollback_cleans_up() {
        let dir = RamDirectory::default();
        let mut sis = SegmentInfos::new(10).unwrap();
        sis.prepare_commit(&dir).unwrap();
        assert!(dir.file_length("pending_segments_1").is_ok());
        sis.rollback_commit(&dir);
        assert!(dir.file_length("pending_segments_1").is_err());
    }

    #[test]
    fn clone_is_independent() {
        let mut sis = SegmentInfos::new(10).unwrap();
        let sci = test_segment_commit_info("_0", 10);
        sis.add(sci.clone()).unwrap();
        let mut cloned = sis.clone();
        assert_eq!(cloned.size(), 1);
        assert_eq!(cloned.generation(), sis.generation());
        assert_eq!(cloned.id(), sis.id());
        cloned.remove(&sci);
        assert_eq!(cloned.size(), 0);
        assert_eq!(sis.size(), 1);
    }
}
