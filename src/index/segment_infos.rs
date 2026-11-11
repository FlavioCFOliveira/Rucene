//! Segment collection and `segments_N` file management.
//!
//! Equivalent to `org.apache.lucene.index.SegmentInfos`.
//!
//! This module tracks the active set of segments in an index, assigns
//! generation numbers to the `segments_N` commit file, and serializes the
//! commit metadata in the exact byte format used by Apache Lucene Core 10.5.0.

#![deny(unsafe_code)]

use std::collections::HashSet;
use std::fmt::{self, Debug, Formatter};
use std::io;
use std::sync::{Arc, RwLock};

use crate::codecs::codec_util;
use crate::error::{LuceneError, LuceneErrorKind, Result};
use crate::index::index_file_names::{file_name_from_generation, PENDING_SEGMENTS, SEGMENTS};
use crate::index::SegmentCommitInfo;
use crate::store::{Directory, IndexOutput};
use crate::util::java_collections::{JavaHashSet, JavaIntHashMap, JavaStringMap};
use crate::util::string_helper::{StringHelper, ID_LENGTH};
use crate::util::{InfoStream, Version};

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

/// The info-stream component name Lucene logs `segments_N` retries under.
///
/// Java has no constant: `SegmentInfos.message` writes the literal `"SIS "`
/// prefix itself.
const SIS: &str = "SIS";

/// The process-wide sink `SegmentInfos` reports commit-file retries to.
///
/// Equivalent to the private static `SegmentInfos.infoStream` field, which
/// [`SegmentInfos::set_info_stream`] installs and
/// [`SegmentInfos::get_info_stream`] reads back.
///
/// **Divergence from Lucene 10.5.0.** Java's field is a `PrintStream` and
/// `SegmentInfos.message` writes `"SIS [" + threadName + "]: " + message` to it
/// directly. This port holds an [`InfoStream`] instead, the sink every other
/// class in the crate already logs through, and tags the messages with the
/// `"SIS"` component; a [`PrintStreamInfoStream`](crate::util::info_stream::PrintStreamInfoStream)
/// renders the component and the thread name, so the information reaching the
/// sink is the same. Java's field is not synchronised; this one is, because
/// Rust has no unsynchronised process-wide mutable state.
static INFO_STREAM: RwLock<Option<Arc<dyn InfoStream>>> = RwLock::new(None);

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
    /// Carries an iteration order because `segments_N` does: it is serialised
    /// with `DataOutput.writeMapOfStrings`, which walks the map. Java stores
    /// whatever `Map` `setUserData` was handed — the `HashMap` `IndexWriter`
    /// builds from the live commit data (`IndexWriter.java:3816-3820`) or the
    /// map `DataInput.readMapOfStrings` returned on the way in. See
    /// [`JavaStringMap`].
    pub user_data: JavaStringMap,

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
    /// Returns `LuceneErrorKind::IllegalArgument` if the creation version is out of
    /// the supported range.
    ///
    /// Equivalent to `SegmentInfos(int)`.
    pub fn new(index_created_version_major: i32) -> Result<Self> {
        if index_created_version_major > Version::LATEST.major as i32 {
            return Err(LuceneError::illegal_argument(format!(
                "indexCreatedVersionMajor is in the future: {index_created_version_major}"
            )));
        }
        if index_created_version_major < 6 {
            return Err(LuceneError::illegal_argument(format!(
                "indexCreatedVersionMajor must be >= 6, got: {index_created_version_major}"
            )));
        }
        // `generation` and `lastGeneration` are left at Java's field default,
        // 0 (`SegmentInfos.java:132-133, 169-179`): a new instance's segments
        // file name is `segments`, so an `IndexFileDeleter` built on it scans
        // the directory and removes the files no commit references
        // (`IndexFileDeleter.java:139-225`).
        Ok(Self {
            generation: 0,
            last_generation: 0,
            counter: 0,
            version: 0,
            user_data: JavaStringMap::default(),
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

    /// Appends the provided `SegmentCommitInfo`.
    ///
    /// # Errors
    ///
    /// Returns `LuceneErrorKind::IllegalArgument` if the wrapped `SegmentInfo` does
    /// not record `min_version` for an index created on or after Lucene 7.
    pub fn add(&mut self, si: SegmentCommitInfo) -> Result<()> {
        if self.index_created_version_major >= 7 && si.info.min_version().is_none() {
            return Err(LuceneError::illegal_argument(format!(
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
    /// Equivalent to `SegmentInfos.remove(SegmentCommitInfo)`, which finds the
    /// segment by `List.remove(Object)`. `SegmentCommitInfo` does not override
    /// `equals`, so Java matches the very instance the writer holds, whatever
    /// its delete counts and generations are at the time. This port hands out
    /// clones (the reader pool keeps its own copy of each segment), so the
    /// instance is recognised by its segment name, which is unique within one
    /// `SegmentInfos`. Comparing by value instead would miss a segment whose
    /// pooled copy has a different delete count than the writer's, and leave it
    /// in the index after its reader was dropped.
    ///
    /// # Warning
    ///
    /// O(N) cost.
    pub fn remove(&mut self, si: &SegmentCommitInfo) -> bool {
        if let Some(pos) = self.index_of(si) {
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
    /// Equivalent to `SegmentInfos.contains(SegmentCommitInfo)`; the segment is
    /// recognised by its name, as [`remove`](Self::remove) explains.
    ///
    /// # Warning
    ///
    /// O(N) cost.
    pub fn contains(&self, si: &SegmentCommitInfo) -> bool {
        self.index_of(si).is_some()
    }

    /// Returns the index of the provided `SegmentCommitInfo`, or `None`.
    ///
    /// Equivalent to `SegmentInfos.indexOf(SegmentCommitInfo)`; the segment is
    /// recognised by its name, as [`remove`](Self::remove) explains.
    ///
    /// # Warning
    ///
    /// O(N) cost.
    pub fn index_of(&self, si: &SegmentCommitInfo) -> Option<usize> {
        self.segments
            .iter()
            .position(|s| s.info.name == si.info.name)
    }

    /// Returns an iterator over the contained segments.
    pub fn iter(&self) -> impl Iterator<Item = &SegmentCommitInfo> {
        self.segments.iter()
    }

    /// Returns a mutable iterator over the segments in this commit.
    ///
    /// Java's `SegmentInfos` implements `Iterable<SegmentCommitInfo>` and hands
    /// out the live objects, so iteration there is already mutating; Rust needs
    /// an explicit mutable iterator to express the same access. Used by
    /// `IndexFileDeleter::inflate_gens`.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut SegmentCommitInfo> {
        self.segments.iter_mut()
    }

    /// Returns a slice view of the contained segments.
    /// Reads a codec name and resolves it to the registered codec.
    ///
    /// Equivalent to the private `SegmentInfos.readCodec(DataInput)`
    /// (`SegmentInfos.java:524-541`). A name that starts with `Lucene` and
    /// cannot be resolved is reported as an index written by a version whose
    /// codec is no longer on the path: `new IndexFormatTooOldException(input,
    /// "Could not load codec '" + name + "'. " + e.getMessage() + ". Did you
    /// forget to add lucene-backward-codecs.jar?")`, `e` the
    /// `IllegalArgumentException` of `Codec.forName`. As with
    /// [`parse_segment_infos`](Self::parse_segment_infos), Java's `DataInput`
    /// is always the commit's `ChecksumIndexInput`, whose `toString()` is the
    /// exception's resource; this takes that input.
    ///
    /// **Divergence from Lucene 10.5.0.** For a name that does not start with
    /// `Lucene`, Java rethrows `Codec.forName`'s `IllegalArgumentException`
    /// (`SegmentInfos.java:539`); this port still reports
    /// `IndexFormatNotSupported("Could not load codec '<name>'.")`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IndexFormatTooOld`] when a `Lucene` codec cannot
    /// be resolved, [`LuceneErrorKind::IndexFormatNotSupported`] when another
    /// codec cannot be, and propagates any I/O error.
    fn read_codec(
        input: &mut dyn crate::store::ChecksumIndexInput,
    ) -> Result<std::sync::Arc<dyn crate::codecs::Codec>> {
        let name = input.read_string()?;
        match crate::codecs::lookup(&name) {
            Ok(codec) => Ok(codec),
            // Maybe it is an old default codec that has moved.
            Err(LuceneError {
                kind: LuceneErrorKind::IllegalArgument { message, .. },
                ..
            }) if name.starts_with("Lucene") => Err(LuceneError::index_format_too_old_reason(
                input.resource_description(),
                format!(
                    "Could not load codec '{name}'. {message}. Did you forget to add \
                         lucene-backward-codecs.jar?"
                ),
            )),
            Err(LuceneError {
                kind: LuceneErrorKind::IllegalArgument { .. },
                ..
            }) => Err(LuceneError::from(LuceneErrorKind::IndexFormatNotSupported(
                format!("Could not load codec '{name}'."),
            ))),
            Err(error) => Err(error),
        }
    }

    /// Returns the segments as a slice.
    ///
    /// Equivalent to `SegmentInfos.asList()`, which Java wraps in
    /// `Collections.unmodifiableList`; a shared slice is already read-only.
    pub fn as_list(&self) -> &[SegmentCommitInfo] {
        self.as_slice()
    }

    /// Returns the segments as a slice; the same view as
    /// [`as_list`](Self::as_list).
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
    pub fn user_data(&self) -> &JavaStringMap {
        &self.user_data
    }

    /// Sets the commit user data.
    ///
    /// If `do_increment_version` is `true`, `changed()` is called.
    pub fn set_user_data(&mut self, data: JavaStringMap, do_increment_version: bool) {
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
            return Err(LuceneError::illegal_argument(format!(
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
            return Err(LuceneError::illegal_state(format!(
                "cannot decrease generation to {generation} from current generation {}",
                self.generation
            )));
        }
        self.generation = generation;
        Ok(())
    }

    /// Replaces the segments a merge consumed with the merged one, in place.
    ///
    /// Equivalent to `SegmentInfos.applyMergeChanges(OneMerge, boolean)`. The
    /// merged segment takes the position of the first segment it replaced, so
    /// the segment order — which an index sort relies on — is preserved. When
    /// `drop_segment` is set the merged segment is not inserted at all, which
    /// is what happens when every document it would hold has been deleted.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when the merged segment records
    /// no minimum version and the index was created by Lucene 7 or later, which
    /// requires one.
    pub fn apply_merge_changes(
        &mut self,
        merged: &SegmentCommitInfo,
        merged_away: &[SegmentCommitInfo],
        drop_segment: bool,
    ) -> Result<()> {
        if self.index_created_version_major >= 7 && merged.info.min_version().is_none() {
            return Err(LuceneError::illegal_argument(
                "All segments must record the minVersion for indices created on or after Lucene 7"
                    .to_string(),
            ));
        }
        let merged_away: HashSet<&str> = merged_away
            .iter()
            .map(|info| info.info.name.as_str())
            .collect();

        let mut inserted = false;
        let mut new_seg_idx = 0usize;
        for seg_idx in 0..self.segments.len() {
            if merged_away.contains(self.segments[seg_idx].info.name.as_str()) {
                if !inserted && !drop_segment {
                    self.segments[seg_idx] = merged.clone();
                    inserted = true;
                    new_seg_idx += 1;
                }
            } else {
                debug_assert!(seg_idx >= new_seg_idx);
                self.segments.swap(new_seg_idx, seg_idx);
                new_seg_idx += 1;
            }
        }
        // Whatever is left past `new_seg_idx` duplicates something already kept.
        self.segments.truncate(new_seg_idx);

        // Either a place was found for the merged segment, or every segment it
        // replaced was deleted while the merge ran — in which case the merged
        // segment is itself fully deleted, and is inserted at the front only if
        // it is not being dropped.
        if !inserted && !drop_segment {
            self.segments.insert(0, merged.clone());
        }
        Ok(())
    }

    /// Returns the sum of all segment `max_doc` values.
    ///
    /// Deletions are not included.
    ///
    /// Equivalent to `SegmentInfos.totalMaxDoc()` (`SegmentInfos.java:1054-1062`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::Arithmetic`] with the message `integer overflow`
    /// when the sum does not fit in an `i32`: Java's `Math.toIntExact(count)`
    /// throwing `ArithmeticException("integer overflow")`.
    ///
    /// # Panics
    ///
    /// In a debug build, when the sum exceeds
    /// [`get_actual_max_docs`](crate::index::index_writer::get_actual_max_docs):
    /// Java's `assert count <= IndexWriter.getActualMaxDocs()`
    /// (`SegmentInfos.java:1060`), which fires before the conversion under
    /// `-ea`.
    pub fn total_max_doc(&self) -> Result<i32> {
        let count: i64 = self
            .segments
            .iter()
            .map(|s| s.info.max_doc().unwrap_or(0) as i64)
            .sum();
        // We should never hit this, checks should happen elsewhere.
        debug_assert!(count <= i64::from(crate::index::index_writer::get_actual_max_docs()));
        // `Math.toIntExact(count)` (`SegmentInfos.java:1061`).
        crate::error::to_int_exact(count)
    }

    /// Returns all file names referenced by this commit.
    ///
    /// # Arguments
    ///
    /// * `include_segments_file` - if `true`, the committed `segments_N` file is
    ///   included.
    pub fn files(&self, include_segments_file: bool) -> Result<HashSet<String>> {
        Ok(self.files_ordered(include_segments_file)?.to_std())
    }

    /// Returns all file names referenced by this commit, iteration order
    /// included.
    ///
    /// Equivalent to `SegmentInfos.files(boolean)` (`SegmentInfos.java:932-947`):
    /// a `java.util.HashSet` that receives the `segments_N` name first, then
    /// every segment's [`SegmentCommitInfo::files_ordered`] in segment order.
    pub fn files_ordered(&self, include_segments_file: bool) -> Result<JavaHashSet> {
        let mut files = JavaHashSet::new();
        if include_segments_file {
            if let Some(segment_file_name) = self.segments_file_name() {
                files.insert(segment_file_name);
            }
        }
        for info in &self.segments {
            files.extend(info.files_ordered()?);
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

    /// Replaces every segment with `infos`, leaving the generation, version,
    /// counter and user data as they are.
    ///
    /// Equivalent to the package-private
    /// `SegmentInfos.rollbackSegmentInfos(List<SegmentCommitInfo>)`
    /// (`SegmentInfos.java:1132-1135`).
    pub(crate) fn rollback_segment_infos(&mut self, infos: &[SegmentCommitInfo]) {
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
            return Err(LuceneError::illegal_argument(format!(
                "\"{OLD_SEGMENTS_GEN}\" is not a valid segment file name since 4.0"
            )));
        }
        if file_name == SEGMENTS {
            return Ok(0);
        }
        if file_name.starts_with(SEGMENTS) {
            let gen_str = &file_name[1 + SEGMENTS.len()..];
            i64::from_str_radix(gen_str, 36).map_err(|e| {
                LuceneError::illegal_argument(format!(
                    "fileName \"{file_name}\" does not contain a valid generation: {e}"
                ))
            })
        } else {
            Err(LuceneError::illegal_argument(format!(
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
    pub fn read_commit(directory: Arc<dyn Directory>, segment_file_name: &str) -> Result<Self> {
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
        directory: Arc<dyn Directory>,
        segment_file_name: &str,
        min_supported_major_version: i32,
    ) -> Result<Self> {
        let generation = Self::generation_from_segments_file_name(segment_file_name)?;
        // `try (ChecksumIndexInput input = directory.openChecksumInput(...))`
        // (`SegmentInfos.java:296-304`): the input is closed on every exit.
        let mut input = directory.open_checksum_input(segment_file_name)?;
        let read = Self::read_commit_input(
            directory,
            input.as_mut(),
            generation,
            min_supported_major_version,
        );
        crate::store::close_after(input.as_mut(), read)
    }

    /// Reads a commit from an already opened checksum input.
    ///
    /// Equivalent to `SegmentInfos.readCommit(Directory, ChecksumIndexInput, long, int)`.
    pub fn read_commit_input(
        directory: Arc<dyn Directory>,
        input: &mut dyn crate::store::ChecksumIndexInput,
        generation: i64,
        min_supported_major_version: i32,
    ) -> Result<Self> {
        // `int format = -1`: set once `checkHeaderNoMagic` accepted the header.
        let mut format = -1;
        let result = (|| {
            let magic = codec_util::read_be_int(input)?;
            if magic != codec_util::CODEC_MAGIC {
                // `new IndexFormatTooOldException(input, magic, CODEC_MAGIC,
                // CODEC_MAGIC)` (`SegmentInfos.java:326-329`).
                return Err(LuceneError::index_format_too_old(
                    input.resource_description(),
                    magic,
                    codec_util::CODEC_MAGIC,
                    codec_util::CODEC_MAGIC,
                ));
            }
            format =
                codec_util::check_header_no_magic(input, "segments", VERSION_74, VERSION_CURRENT)?;

            let mut id = [0u8; ID_LENGTH];
            input.read_bytes(Some(&mut id), 0, ID_LENGTH)?;
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
                // `new CorruptIndexException(..., input)` (`SegmentInfos.java:338-346`).
                return Err(LuceneError::corrupt_index(
                    format!(
                        "Creation version [{index_created_version}.x] can't be greater than the version that wrote the segment infos: [{lucene_version}]"
                    ),
                    input.resource_description(),
                ));
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

        // Java's `finally` (`SegmentInfos.java:358-364`) checks the footer only
        // once the header was accepted (`format >= VERSION_74`); before that,
        // the failure is rethrown as is, so a wrong magic or an unsupported
        // header version is reported whatever the checksum says. The check is
        // `CodecUtil.checkFooter(input, priorE)`, which *rethrows* the parse
        // failure when there is one, reporting the footer only as extra
        // context. Validating the footer with `?` before the parse result
        // would replace the real cause with a footer complaint, because a
        // parse that stopped early always leaves unread bytes.
        if format < VERSION_74 {
            return result;
        }
        match result {
            Ok(infos) => {
                codec_util::check_footer(input)?;
                Ok(infos)
            }
            Err(prior) => Err(codec_util::check_footer_with_prior(input, prior)),
        }
    }

    /// Reads the segments of a commit.
    ///
    /// Equivalent to the private `SegmentInfos.parseSegmentInfos(Directory,
    /// DataInput, SegmentInfos, int, int)` (`SegmentInfos.java:368-522`). Java
    /// declares a `DataInput` but is only ever handed the commit's
    /// `ChecksumIndexInput`, whose `toString()` is the resource every
    /// `CorruptIndexException(message, input)` it throws names; this takes that
    /// input, so the errors name the same resource.
    fn parse_segment_infos(
        directory: Arc<dyn Directory>,
        input: &mut dyn crate::store::ChecksumIndexInput,
        infos: &mut SegmentInfos,
        format: i32,
        min_supported_major_version: i32,
    ) -> Result<()> {
        infos.version = codec_util::read_be_long(input)?;
        infos.counter = input.read_v_long()?;
        let num_segments = codec_util::read_be_int(input)?;
        if num_segments < 0 {
            return Err(LuceneError::corrupt_index(
                format!("invalid segment count: {num_segments}"),
                input.resource_description(),
            ));
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
            input.read_bytes(Some(&mut segment_id), 0, ID_LENGTH)?;
            let codec = Self::read_codec(input)?;

            let mut info = codec.segment_info_format().read(
                Arc::clone(&directory),
                &seg_name,
                &segment_id,
                &*crate::store::READONCE_IO_CONTEXT,
            )?;
            info.set_codec(Some(codec))?;

            let max_doc = info.max_doc()?;
            total_docs += max_doc as i64;

            let del_gen = codec_util::read_be_long(input)?;
            let del_count = codec_util::read_be_int(input)?;
            if del_count < 0 || del_count > max_doc {
                return Err(LuceneError::corrupt_index(
                    format!("invalid deletion count: {del_count} vs maxDoc={max_doc}"),
                    input.resource_description(),
                ));
            }

            let field_infos_gen = codec_util::read_be_long(input)?;
            let doc_values_gen = codec_util::read_be_long(input)?;
            let soft_del_count = codec_util::read_be_int(input)?;
            // Java reports an invalid soft deletion count with the same
            // "invalid deletion count" text (`SegmentInfos.java:410-413`).
            if soft_del_count < 0 || soft_del_count > max_doc {
                return Err(LuceneError::corrupt_index(
                    format!("invalid deletion count: {soft_del_count} vs maxDoc={max_doc}"),
                    input.resource_description(),
                ));
            }
            if (soft_del_count as i64 + del_count as i64) > max_doc as i64 {
                return Err(LuceneError::corrupt_index(
                    format!(
                        "invalid deletion count: {} vs maxDoc={max_doc}",
                        soft_del_count + del_count
                    ),
                    input.resource_description(),
                ));
            }

            let sci_id: [u8; ID_LENGTH] = if format > VERSION_74 {
                let marker = input.read_byte()?;
                match marker {
                    1 => {
                        let mut buf = [0u8; ID_LENGTH];
                        input.read_bytes(Some(&mut buf), 0, ID_LENGTH)?;
                        buf
                    }
                    0 => StringHelper::random_id(),
                    _ => {
                        return Err(LuceneError::corrupt_index(
                            format!("invalid SegmentCommitInfo ID marker: {marker}"),
                            input.resource_description(),
                        ))
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
            // `HashMap.newHashMap(numDVFields)`, iterated in its order by
            // `setDocValuesUpdatesFiles` (`SegmentInfos.java:440-451`); an
            // empty count is `Collections.emptyMap()`.
            let mut dv_updates_files: JavaIntHashMap<Vec<String>> =
                JavaIntHashMap::new_hash_map(num_dv_fields.max(0) as usize);
            for _ in 0..num_dv_fields {
                let field_number = codec_util::read_be_int(input)?;
                let files = input.read_set_of_strings()?;
                dv_updates_files.insert(field_number, files.into_iter().collect());
            }
            si_per_commit.set_doc_values_updates_files(
                dv_updates_files
                    .iter()
                    .map(|(field_number, files)| (field_number, files.clone()))
                    .collect::<Vec<_>>(),
            );

            infos.add(si_per_commit)?;

            let segment_version = infos.segments.last().unwrap().info.version();
            // Java names the segment by `info`, its `SegmentInfo.toString()`
            // (`SegmentInfos.java:456-477`).
            if let Some(min_version) = infos.min_segment_lucene_version {
                if !segment_version.on_or_after(&min_version) {
                    return Err(LuceneError::corrupt_index(
                        format!(
                            "segments file recorded minSegmentLuceneVersion={min_version} but segment={} has older version={segment_version}",
                            infos.segments.last().unwrap().info.to_string_with_del_count(0)
                        ),
                        input.resource_description(),
                    ));
                }
            }

            if infos.index_created_version_major >= 7
                && (segment_version.major as i32) < infos.index_created_version_major
            {
                return Err(LuceneError::corrupt_index(
                    format!(
                        "segments file recorded indexCreatedVersionMajor={} but segment={} has older version={segment_version}",
                        infos.index_created_version_major,
                        infos.segments.last().unwrap().info.to_string_with_del_count(0)
                    ),
                    input.resource_description(),
                ));
            }

            if infos.index_created_version_major >= 7
                && infos.segments.last().unwrap().info.min_version().is_none()
            {
                return Err(LuceneError::corrupt_index(
                    format!(
                        "segments infos must record minVersion with indexCreatedVersionMajor={}",
                        infos.index_created_version_major
                    ),
                    input.resource_description(),
                ));
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
                // `new IndexFormatTooOldException(input, "Index has segments
                // derived from ...")` (`SegmentInfos.java:492-508`).
                return Err(LuceneError::index_format_too_old_reason(
                    input.resource_description(),
                    format!(
                        "Index has segments derived from Lucene version \
                         {created_or_segment_min_version}.x and is not supported by Lucene {}. \
                         This Lucene version only supports indexes with major version \
                         {min_supported_major_version} or later (found: \
                         {created_or_segment_min_version}, minimum supported: \
                         {min_supported_major_version}). To resolve this issue re-index your \
                         data using Lucene {min_supported_major_version}.x or later.",
                        Version::LATEST
                    ),
                ));
            }
        }

        infos.user_data = input.read_map_of_strings()?;

        // LUCENE-6299: check we are in bounds (`SegmentInfos.java:513-520`).
        // The limit is `IndexWriter.getActualMaxDocs()`, the one
        // `IndexWriter.setMaxDocs` may have lowered, read each time.
        let actual_max_docs = crate::index::index_writer::get_actual_max_docs();
        if total_docs > i64::from(actual_max_docs) {
            return Err(LuceneError::corrupt_index(
                format!(
                    "Too many documents: an index cannot exceed {actual_max_docs} but readers have total maxDoc={total_docs}"
                ),
                input.resource_description(),
            ));
        }

        Ok(())
    }

    /// Finds the latest commit and loads all `SegmentCommitInfo`s.
    ///
    /// Equivalent to `SegmentInfos.readLatestCommit(Directory)`.
    pub fn read_latest_commit(directory: Arc<dyn Directory>) -> Result<Self> {
        Self::read_latest_commit_with_min_version(directory, Version::MIN_SUPPORTED_MAJOR)
    }

    /// Installs the sink that commit-file retries are reported to.
    ///
    /// Equivalent to `org.apache.lucene.index.SegmentInfos#setInfoStream`, whose
    /// argument is a `PrintStream`; see the note on [`INFO_STREAM`] for why this
    /// port takes an [`InfoStream`] instead. Pass `None` for Java's `null`.
    pub fn set_info_stream(info_stream: Option<Arc<dyn InfoStream>>) {
        if let Ok(mut guard) = INFO_STREAM.write() {
            *guard = info_stream;
        }
    }

    /// Returns the sink commit-file retries are reported to, if one is set.
    ///
    /// Equivalent to `org.apache.lucene.index.SegmentInfos#getInfoStream`.
    pub fn get_info_stream() -> Option<Arc<dyn InfoStream>> {
        INFO_STREAM.read().ok().and_then(|guard| guard.clone())
    }

    /// Prints `message` to the installed info stream.
    ///
    /// Equivalent to the private static `org.apache.lucene.index.SegmentInfos#message`.
    /// Java does not check for a null stream — every caller guards the call
    /// with `if (infoStream != null)` so that the message is not built when
    /// nothing will read it — and this port keeps the same guard at the call
    /// sites, so reaching here with no stream installed simply does nothing.
    fn message(message: &str) {
        if let Some(info_stream) = Self::get_info_stream() {
            info_stream.message(SIS, message);
        }
    }

    /// Finds the latest commit, enforcing a minimum supported major version.
    ///
    /// Equivalent to `SegmentInfos.readLatestCommit(Directory, int)`.
    pub fn read_latest_commit_with_min_version(
        directory: Arc<dyn Directory>,
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

            if Self::get_info_stream().is_some() {
                Self::message(&format!("directory listing gen={gen}"));
            }

            if gen == -1 {
                // `throw new IndexNotFoundException("no segments* file found in
                // " + directory + ": files: " + Arrays.toString(files))`
                // (`SegmentInfos.java:825-827`): the directory's `toString()`
                // and the sorted listing as `Arrays.toString` renders it.
                return Err(LuceneError::index_not_found(format!(
                    "no segments* file found in {}: files: [{}]",
                    directory.directory_to_string(),
                    files.join(", ")
                )));
            }

            if gen > last_gen {
                last_gen = gen;
                let segment_file_name =
                    file_name_from_generation(SEGMENTS, "", gen).ok_or_else(|| {
                        LuceneError::illegal_state(format!(
                            "could not build segments file name for generation {gen}"
                        ))
                    })?;
                match Self::read_commit_with_min_version(
                    Arc::clone(&directory),
                    &segment_file_name,
                    min_supported_major_version,
                ) {
                    Ok(infos) => {
                        if Self::get_info_stream().is_some() {
                            Self::message(&format!("success on {segment_file_name}"));
                        }
                        return Ok(infos);
                    }
                    Err(err) => {
                        if Self::get_info_stream().is_some() {
                            Self::message(&format!(
                                "primary Exception on '{segment_file_name}': {err}'; will retry: gen = {gen}"
                            ));
                        }
                        if exc.is_none() {
                            exc = Some(err);
                        }
                    }
                }
            } else {
                return Err(exc.unwrap_or_else(|| {
                    LuceneError::from(LuceneErrorKind::Io(io::Error::new(
                        io::ErrorKind::NotFound,
                        "no segments* file found".to_string(),
                    )))
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
            return Err(LuceneError::illegal_state(
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
                LuceneError::illegal_state(format!(
                    "could not build pending segments file name for generation {next_generation}"
                ))
            })?;

        // Always advance the generation on write.
        self.generation = next_generation;

        // `SegmentInfos.write(Directory)` (`SegmentInfos.java:578-603`): every
        // step from creating the output to syncing it is inside the `try`, so
        // any of them failing closes the output — again, if it was closed —
        // and deletes the truncated file.
        let mut seg_output: Option<Box<dyn IndexOutput>> = None;
        let outcome = (|| -> Result<()> {
            let output = seg_output.insert(
                directory.create_output(&segment_file_name, &*crate::store::DEFAULT_IO_CONTEXT)?,
            );
            self.write_to_output(output.as_mut())?;
            output.close()?;
            directory.sync(std::slice::from_ref(&segment_file_name))?;
            Ok(())
        })();
        match outcome {
            Ok(()) => {
                self.pending_commit = true;
                Ok(())
            }
            Err(err) => {
                // We hit an exception above; try to close the file but
                // suppress any exception.
                crate::util::IOUtils::close_while_handling_exception([seg_output
                    .as_mut()
                    .map(|output| move || output.close())])?;
                // Try not to leave a truncated segments_N file in the index:
                // `IOUtils.deleteFilesIgnoringExceptions(directory,
                // segmentFileName)`.
                let _ = directory.delete_file(&segment_file_name);
                Err(err)
            }
        }
    }

    /// Writes this commit to the provided output.
    ///
    /// Equivalent to `SegmentInfos.write(IndexOutput)`.
    pub fn write_to_output(&self, out: &mut dyn IndexOutput) -> Result<()> {
        // Java stamps every written commit with a *fresh* id
        // (`StringHelper.randomId()` in `SegmentInfos.write(IndexOutput)`),
        // not with the `id` field, which only records the id that was read
        // back from disk. Two commits of the same `SegmentInfos` therefore
        // carry different ids.
        let commit_id = StringHelper::random_id();
        codec_util::write_index_header(
            out,
            "segments",
            VERSION_CURRENT,
            &commit_id,
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
                return Err(LuceneError::illegal_state(format!(
                    "Segments must record minVersion if they have been created on or after Lucene 7: {si:?}"
                )));
            }

            out.write_string(&si.name)?;
            let segment_id = si.id();
            if segment_id.len() != ID_LENGTH {
                return Err(LuceneError::illegal_state(format!(
                    "cannot write segment: invalid id segment={} id={:?}",
                    si.name, segment_id
                )));
            }
            out.write_bytes(&segment_id, 0, segment_id.len())?;
            let codec_name = si
                .codec()
                .ok_or_else(|| {
                    LuceneError::illegal_state(format!(
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
                return Err(LuceneError::illegal_state(format!(
                    "cannot write segment: invalid maxDoc segment={} maxDoc={max_doc} delCount={del_count}",
                    si.name
                )));
            }
            codec_util::write_be_int(out, del_count)?;
            codec_util::write_be_long(out, si_per_commit.get_field_infos_gen())?;
            codec_util::write_be_long(out, si_per_commit.get_doc_values_gen())?;
            let soft_del_count = si_per_commit.get_soft_del_count();
            if soft_del_count < 0 || soft_del_count > max_doc {
                return Err(LuceneError::illegal_state(format!(
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
            for (field_number, files) in dv_updates_files.iter() {
                codec_util::write_be_int(out, field_number)?;
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
            return Err(LuceneError::illegal_state(
                "prepareCommit was not called".to_string(),
            ));
        }

        let src =
            file_name_from_generation(PENDING_SEGMENTS, "", self.generation).ok_or_else(|| {
                LuceneError::illegal_state(format!(
                    "could not build pending segments file name for generation {}",
                    self.generation
                ))
            })?;
        let dest = file_name_from_generation(SEGMENTS, "", self.generation).ok_or_else(|| {
            LuceneError::illegal_state(format!(
                "could not build segments file name for generation {}",
                self.generation
            ))
        })?;

        let mut success_rename_and_sync = false;
        let result: Result<()> = (|| {
            directory.rename(&src, &dest)?;
            // Only now does `dest` name a file this commit created. Java's
            // inner `finally` — the one that deletes `dest` — is entered only
            // after `rename` returns, so a failing `rename` must leave `dest`
            // alone: it is still the previous, valid commit point.
            match directory.sync_metadata() {
                Ok(()) => {
                    success_rename_and_sync = true;
                    Ok(())
                }
                Err(err) => {
                    // The file was created but the directory was never synced,
                    // so remove the renamed file as Java does.
                    let _ = directory.delete_file(&dest);
                    Err(err)
                }
            }
        })();

        if !success_rename_and_sync {
            // Deletes pending_segments_N.
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
        // `SegmentInfos.java:715`, per cloned segment.
        debug_assert!(self.segments.iter().all(|info| info.info.codec().is_some()));
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
    use crate::codecs::tests::{
        test_segment_commit_info, test_segment_info_with_codec, DummyCodec,
    };
    use crate::codecs::{Codec, FilterCodec, SegmentInfoFormat};
    use crate::index::index_file_names::{file_name_from_generation, SEGMENTS};
    use crate::index::SegmentCommitInfo;
    use crate::search::{Sort, SortField, SortFieldType};
    use crate::store::{ByteBuffersDirectory, Directory};
    use crate::util::string_helper::StringHelper;
    use std::collections::HashSet;
    use std::sync::Arc;

    /// A directory with no commit fails `readLatestCommit` with
    /// `IndexNotFoundException("no segments* file found in " + directory +
    /// ": files: " + Arrays.toString(files))` (`SegmentInfos.java:825-827`).
    /// Java oracle (`fp/SisOracle.java`): "empty: IndexNotFoundException: no
    /// segments* file found in ByteBuffersDirectory@3bd40a57
    /// lockFactory=org.apache.lucene.store.SingleInstanceLockFactory@a4102b8:
    /// files: []", "files: ...: files: [a, b]".
    #[test]
    fn read_latest_commit_without_a_commit_raises_index_not_found() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let in_dir = "no segments* file found in ByteBuffersDirectory@{hash} \
                      lockFactory=org.apache.lucene.store.SingleInstanceLockFactory@{hash}";
        let message =
            |dir: &Arc<dyn Directory>| match SegmentInfos::read_latest_commit(Arc::clone(dir)) {
                Err(LuceneError {
                    kind: LuceneErrorKind::IndexNotFound(message),
                    ..
                }) => message,
                other => panic!("expected IndexNotFoundException, got {:?}", other.err()),
            };
        let empty = message(&dir);
        assert!(
            crate::store::matches_java_text(&empty, &format!("{in_dir}: files: []")),
            "{empty}"
        );
        for name in ["b", "a"] {
            dir.create_output(name, &*crate::store::DEFAULT_IO_CONTEXT)
                .unwrap()
                .close()
                .unwrap();
        }
        let files = message(&dir);
        assert!(
            crate::store::matches_java_text(&files, &format!("{in_dir}: files: [a, b]")),
            "{files}"
        );
    }

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
        assert_eq!(sis.total_max_doc().unwrap(), 0);
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
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let mut sis = SegmentInfos::new(10).unwrap();
        sis.user_data = [("foo".to_string(), "bar".to_string())]
            .into_iter()
            .collect();

        let written = sis.commit(dir.as_ref()).unwrap();
        assert_eq!(written, "segments_1");

        let read = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
        assert_eq!(read.size(), 0);
        assert_eq!(read.version(), sis.version());
        assert_eq!(read.counter, sis.counter);
        assert_eq!(read.user_data().get("foo"), Some(&"bar".to_string()));
        assert_eq!(read.last_generation(), 1);
    }

    #[test]
    fn round_trip_single_segment() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let mut info = test_segment_info_with_codec("_0", 42, test_codec());
        info.set_index_sort(
            Sort::new_fields(vec![SortField::new(
                Some("id".to_string()),
                SortFieldType::String,
            )
            .unwrap()])
            .unwrap(),
        );
        info.set_diagnostics(std::collections::HashMap::from([(
            "source".to_string(),
            "test".to_string(),
        )]));
        info.set_files(HashSet::from(["_0.fnm".to_string(), "_0.fdt".to_string()]))
            .unwrap();

        write_segment_info_file(dir.as_ref(), &info).unwrap();

        let sci = SegmentCommitInfo::new(info, 2, 1, 0, 1, 2, StringHelper::random_id()).unwrap();
        let mut sis = SegmentInfos::new(10).unwrap();
        sis.counter = 1;
        sis.changed();
        sis.user_data = [("user".to_string(), "data".to_string())]
            .into_iter()
            .collect();
        sis.add(sci).unwrap();

        let written = sis.commit(dir.as_ref()).unwrap();
        assert_eq!(written, "segments_1");

        let read = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
        assert_eq!(read.size(), 1);
        assert_eq!(read.counter, 1);
        assert_eq!(read.version(), sis.version());
        assert_eq!(read.total_max_doc().unwrap(), 42);
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
        let dir = ByteBuffersDirectory::default();
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

    /// `totalMaxDoc()` sums in a `long` and returns `Math.toIntExact(count)`
    /// (`SegmentInfos.java:1054-1062`), which throws
    /// `ArithmeticException("integer overflow")` for a sum past an `int`;
    /// under `-ea` its `assert count <= IndexWriter.getActualMaxDocs()` fires
    /// first. This port raises `Arithmetic("integer overflow")` in a release
    /// build and panics on the `debug_assert!` in a debug build.
    #[test]
    fn total_max_doc_past_an_int_is_lucenes_integer_overflow() {
        let mut sis = SegmentInfos::new(10).unwrap();
        for name in ["_0", "_1", "_2"] {
            sis.add(test_segment_commit_info(name, 1_000_000_000))
                .unwrap();
        }
        let total = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sis.total_max_doc()));
        if cfg!(debug_assertions) {
            assert!(total.is_err(), "Java's assertion fires first: {total:?}");
        } else {
            let error = total.unwrap().unwrap_err();
            assert!(
                matches!(&error, LuceneError { kind: LuceneErrorKind::Arithmetic(m), .. } if m == "integer overflow"),
                "{error:?}"
            );
        }
    }

    /// Commits `_0`, a segment of 42 documents, and returns the directory and
    /// the bytes of its `segments_1`.
    fn committed_single_segment() -> (Arc<dyn Directory>, Vec<u8>) {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let info = test_segment_info_with_codec("_0", 42, test_codec());
        info.set_files(HashSet::from(["_0.fnm".to_string()]))
            .unwrap();
        write_segment_info_file(dir.as_ref(), &info).unwrap();
        let sci =
            SegmentCommitInfo::new(info, 0, 0, -1, -1, -1, StringHelper::random_id()).unwrap();
        let mut sis = SegmentInfos::new(10).unwrap();
        sis.counter = 1;
        sis.changed();
        sis.add(sci).unwrap();
        assert_eq!(sis.commit(dir.as_ref()).unwrap(), "segments_1");
        let mut input = dir
            .open_input("segments_1", &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        let mut bytes = vec![0u8; input.length() as usize];
        let length = bytes.len();
        input.read_bytes(Some(&mut bytes), 0, length).unwrap();
        input.close().unwrap();
        (dir, bytes)
    }

    /// Replaces `segments_1` with `bytes`, its footer checksum recomputed so
    /// that only the patched field is wrong, and returns the error reading it
    /// raises together with the resource description of its checksum input.
    fn read_patched(dir: &Arc<dyn Directory>, mut bytes: Vec<u8>) -> (LuceneError, String) {
        let length = bytes.len();
        let checksum = u64::from(crc32fast::hash(&bytes[..length - 8]));
        bytes[length - 8..].copy_from_slice(&checksum.to_be_bytes());
        read_as_written(dir, bytes)
    }

    /// Replaces `segments_1` with `bytes`, footer as given, and returns the
    /// error reading it raises together with the resource description of its
    /// checksum input.
    fn read_as_written(dir: &Arc<dyn Directory>, bytes: Vec<u8>) -> (LuceneError, String) {
        use crate::store::IndexInput;
        let length = bytes.len();
        dir.delete_file("segments_1").unwrap();
        let mut output = dir
            .create_output("segments_1", &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
        output.write_bytes(&bytes, 0, length).unwrap();
        output.close().unwrap();
        let mut input = dir.open_checksum_input("segments_1").unwrap();
        let resource = input.resource_description().to_string();
        input.close().unwrap();
        let error = SegmentInfos::read_commit(Arc::clone(dir), "segments_1").unwrap_err();
        (error, resource)
    }

    /// Every `CorruptIndexException(message, input)` that `readCommit` and
    /// `parseSegmentInfos` throw names the commit's input as its resource
    /// (`SegmentInfos.java:338-346, 368-522`), with Java's text: an invalid
    /// soft deletion count is reported as an "invalid deletion count"
    /// (`SegmentInfos.java:410-413`), and an old segment is named by its
    /// `SegmentInfo.toString()` (`SegmentInfos.java:456-477`).
    #[test]
    fn a_corrupt_commit_names_its_input_as_lucene_does() {
        let (dir, original) = committed_single_segment();
        let header = crate::codecs::codec_util::index_header_length("segments", "1") as usize;
        // The segment's name, after the segment count and the three VInts of
        // the minimum segment version 10.5.0.
        let name_at = original
            .windows(3)
            .position(|window| window == [2, b'_', b'0'])
            .unwrap();
        assert_eq!(&original[name_at - 7..name_at], &[0, 0, 0, 1, 10, 5, 0]);
        // The soft deletion count follows the codec name, the deletion
        // generation, the deletion count and the two field generations.
        let codec_at = original
            .windows(10)
            .position(|window| window == b"\x09TestCodec")
            .unwrap();
        let soft_del_count_at = codec_at + 10 + 8 + 4 + 8 + 8;

        /// Patches one field of the commit's bytes.
        type Patch = Box<dyn Fn(&mut Vec<u8>)>;
        let cases: [(&str, Patch); 4] = [
            (
                "Creation version [10.x] can't be greater than the version that wrote the \
                 segment infos: [9.5.0]",
                Box::new(move |bytes| bytes[header] = 9),
            ),
            (
                "invalid segment count: -1",
                Box::new(move |bytes| bytes[name_at - 7..name_at - 3].copy_from_slice(&[0xff; 4])),
            ),
            (
                "invalid deletion count: 43 vs maxDoc=42",
                Box::new(move |bytes| {
                    bytes[soft_del_count_at..soft_del_count_at + 4]
                        .copy_from_slice(&43i32.to_be_bytes())
                }),
            ),
            (
                "segments file recorded minSegmentLuceneVersion=10.5.1 but segment=_0(10.5.0):C42 \
                 has older version=10.5.0",
                Box::new(move |bytes| bytes[name_at - 1] = 1),
            ),
        ];
        let mut mismatches = Vec::new();
        for (message, patch) in cases {
            let mut bytes = original.clone();
            patch(&mut bytes);
            let (error, resource) = read_patched(&dir, bytes);
            let expected = LuceneError::corrupt_index(message, &resource).to_string();
            if error.to_string() != expected {
                mismatches.push(format!("got {error}, expected {expected}"));
            }
        }
        assert!(mismatches.is_empty(), "{mismatches:#?}");
    }

    /// `readCommit`'s two `IndexFormatTooOldException`s carry the text that
    /// exception's constructors build around the commit's input
    /// (`IndexFormatTooOldException.java:40-48, 63-65, 76-89, 106-108`): a
    /// wrong magic is reported with the version triple
    /// (`SegmentInfos.java:326-329`), a segment older than the minimum
    /// supported major version with its reason (`SegmentInfos.java:492-508`).
    /// Measured against
    /// `lucene-core-10.5.0.jar` (`oracle6/Probe13.java`, cases A1 and A2),
    /// where only the resource differs.
    #[test]
    fn a_too_old_commit_is_reported_as_lucene_reports_it() {
        use crate::store::IndexInput;
        let (dir, original) = committed_single_segment();
        let resource = {
            let mut input = dir.open_checksum_input("segments_1").unwrap();
            let resource = input.resource_description().to_string();
            input.close().unwrap();
            resource
        };
        let error = SegmentInfos::read_commit_with_min_version(Arc::clone(&dir), "segments_1", 11)
            .unwrap_err();
        assert!(
            matches!(
                error,
                LuceneError {
                    kind: LuceneErrorKind::IndexFormatTooOld { .. },
                    ..
                }
            ),
            "{error:?}"
        );
        assert_eq!(
            error.to_string(),
            format!(
                "Format version is not supported (resource {resource}): Index has segments \
                 derived from Lucene version 10.x and is not supported by Lucene 10.5.0. This \
                 Lucene version only supports indexes with major version 11 or later (found: \
                 10, minimum supported: 11). To resolve this issue re-index your data using \
                 Lucene 11.x or later.. This version of Lucene only supports indexes created \
                 with release 9.0 and later by default."
            )
        );

        let mut bytes = original;
        bytes[3] ^= 1;
        let (error, resource) = read_patched(&dir, bytes);
        assert!(
            matches!(
                error,
                LuceneError {
                    kind: LuceneErrorKind::IndexFormatTooOld { .. },
                    ..
                }
            ),
            "{error:?}"
        );
        assert_eq!(
            error.to_string(),
            format!(
                "Format version is not supported (resource {resource}): 1071082518 (needs to \
                 be between 1071082519 and 1071082519). This version of Lucene only supports \
                 indexes created with release 9.0 and later."
            )
        );
    }

    /// `readCommit` checks the footer only once `checkHeaderNoMagic` accepted
    /// the header (`format >= VERSION_74`, `SegmentInfos.java:358-364`): a
    /// wrong magic or a header version below `VERSION_74` is reported as such
    /// even when the checksum is wrong as well. Measured against
    /// `lucene-core-10.5.0.jar` (`oracle7/Probe16.java`, cases C1 and C2, the
    /// checksum left stale): `IndexFormatTooOldException` with `1071082518
    /// (needs to be between 1071082519 and 1071082519)` and with `8 (needs to
    /// be between 9 and 10)`, both naming the checksum input as the resource.
    /// The second is raised by `codec_util::check_header_no_magic`
    /// (`CodecUtil.java:209-212`).
    #[test]
    fn a_header_refused_before_its_format_is_known_skips_the_footer_check() {
        let (dir, original) = committed_single_segment();

        let mut bytes = original.clone();
        bytes[3] ^= 1;
        let (error, resource) = read_as_written(&dir, bytes);
        assert!(
            matches!(
                error,
                LuceneError {
                    kind: LuceneErrorKind::IndexFormatTooOld { .. },
                    ..
                }
            ),
            "{error:?}"
        );
        assert_eq!(
            error.to_string(),
            format!(
                "Format version is not supported (resource {resource}): 1071082518 (needs to be \
                 between 1071082519 and 1071082519). This version of Lucene only supports \
                 indexes created with release 9.0 and later."
            )
        );

        // The header's version follows the magic and the codec name
        // `segments`, a length byte and eight bytes.
        let mut bytes = original;
        assert_eq!(bytes[13..17], [0, 0, 0, 10]);
        bytes[16] = 8;
        let (error, resource) = read_as_written(&dir, bytes);
        assert!(
            matches!(
                error,
                LuceneError {
                    kind: LuceneErrorKind::IndexFormatTooOld { .. },
                    ..
                }
            ),
            "the header's refusal, not the footer's checksum failure: {error:?}"
        );
        assert_eq!(
            error.to_string(),
            format!(
                "Format version is not supported (resource {resource}): 8 (needs to be between 9 \
                 and 10). This version of Lucene only supports indexes created with release 9.0 \
                 and later."
            )
        );
    }

    /// `readCodec` reports a `Lucene` codec it cannot load as
    /// `IndexFormatTooOldException(input, "Could not load codec '" + name +
    /// "'. " + e.getMessage() + ". Did you forget to add
    /// lucene-backward-codecs.jar?")`, `e` the `IllegalArgumentException` of
    /// `Codec.forName` (`SegmentInfos.java:524-541`,
    /// `NamedSPILoader.java:102-114`). Measured against
    /// `lucene-core-10.5.0.jar` (`oracle7/Probe16.java`, case D1, the
    /// checksum recomputed), resource aside; Java lists `[Lucene104]`, this
    /// registry the codecs the crate's tests register, so only the list's
    /// form and one name are asserted.
    #[test]
    fn an_unknown_lucene_codec_is_reported_as_lucene_reports_it() {
        let (dir, mut bytes) = committed_single_segment();
        let at: Vec<usize> = bytes
            .windows(9)
            .enumerate()
            .filter(|(_, window)| window == b"TestCodec")
            .map(|(at, _)| at)
            .collect();
        assert_eq!(at.len(), 1, "the segment's codec name is written once");
        bytes[at[0]..at[0] + 9].copy_from_slice(b"Lucene000");
        let (error, resource) = read_patched(&dir, bytes);
        assert!(
            matches!(
                error,
                LuceneError {
                    kind: LuceneErrorKind::IndexFormatTooOld { .. },
                    ..
                }
            ),
            "{error:?}"
        );
        let message = error.to_string();
        let prefix = format!(
            "Format version is not supported (resource {resource}): Could not load codec \
             'Lucene000'. An SPI class of type org.apache.lucene.codecs.Codec with name \
             'Lucene000' does not exist.  You need to add the corresponding JAR file supporting \
             this SPI to your classpath.  The current classpath supports the following names: ["
        );
        let suffix = "]. Did you forget to add lucene-backward-codecs.jar?. This version of \
                      Lucene only supports indexes created with release 9.0 and later by \
                      default.";
        assert!(
            message.starts_with(&prefix) && message.ends_with(suffix),
            "{message}"
        );
        let names = &message[prefix.len()..message.len() - suffix.len()];
        assert!(names.split(", ").any(|name| name == "TestCodec"), "{names}");
    }
}

impl std::fmt::Display for SegmentInfos {
    /// Port of `SegmentInfos.toString()`, which is `toString(false)`: the
    /// segments file name, `": "`, then each segment separated by a space.
    ///
    /// Each segment is rendered by `SegmentCommitInfo.toString(0)` — the
    /// *overload*, which folds in that commit's own deletion count — not by its
    /// no-argument `toString()`, which does not.
    ///
    /// `getSegmentsFileName()` returns `null` before the first commit; Java's
    /// `StringBuilder.append` then writes the four characters `null`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.write_to_string(f, false)
    }
}

impl SegmentInfos {
    /// Renders the commit, verbosely or not.
    ///
    /// Equivalent to the package-private `SegmentInfos.toString(boolean)`, the
    /// body both `toString()` and
    /// [`to_string_verbose`](Self::to_string_verbose) share. It writes into the
    /// formatter rather than building a `StringBuilder`, so that
    /// [`Display`](std::fmt::Display) does not have to allocate one.
    fn write_to_string(&self, f: &mut std::fmt::Formatter<'_>, verbose: bool) -> std::fmt::Result {
        match self.segments_file_name() {
            Some(name) => f.write_str(&name)?,
            None => f.write_str("null")?,
        }
        f.write_str(": ")?;
        for (i, info) in self.segments.iter().enumerate() {
            if i > 0 {
                f.write_str(" ")?;
            }
            if verbose {
                f.write_str(&info.to_string_verbose())?;
            } else {
                f.write_str(&info.to_string_with_pending_del_count(0))?;
            }
        }
        Ok(())
    }

    /// Returns the long-form description the info stream logs for this commit.
    ///
    /// Equivalent to `SegmentInfos.toStringVerbose()`, which is
    /// `toString(true)`: the same shape as
    /// [`Display`](std::fmt::Display), with every segment rendered by
    /// [`SegmentCommitInfo::to_string_verbose`] instead.
    pub fn to_string_verbose(&self) -> String {
        struct Verbose<'a>(&'a SegmentInfos);
        impl std::fmt::Display for Verbose<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.write_to_string(f, true)
            }
        }
        Verbose(self).to_string()
    }

    /// Returns a deep copy of the segment list, to roll back to.
    ///
    /// Equivalent to `SegmentInfos.createBackupSegmentInfos()`, which clones
    /// every [`SegmentCommitInfo`] — so that a later mutation of the live list
    /// does not reach the backup — and asserts that each carries a codec.
    ///
    /// The codec assertion has no counterpart here: [`SegmentInfo`]'s codec is
    /// a non-optional `Arc<dyn Codec>`, so a `SegmentCommitInfo` without one
    /// cannot be built.
    pub fn create_backup_segment_infos(&self) -> Vec<SegmentCommitInfo> {
        // `SegmentInfos.java:1126`, per backed-up segment.
        debug_assert!(self.segments.iter().all(|info| info.info.codec().is_some()));
        self.segments.to_vec()
    }
}

#[cfg(test)]
mod write_failure_tests {
    //! `SegmentInfos.write(Directory)` (`SegmentInfos.java:578-603`): every
    //! step from `createOutput` to `sync` is inside the `try`, so a failure of
    //! any of them closes the output with
    //! `IOUtils.closeWhileHandlingException(segnOutput)` and deletes the
    //! truncated `pending_segments_N`.
    use super::*;
    use crate::codecs::tests::CloseTrackingDirectory;

    #[test]
    fn a_failed_sync_deletes_the_pending_segments_file() {
        let dir = CloseTrackingDirectory::default();
        dir.log.fail_sync();
        let mut infos = SegmentInfos::new(crate::util::Version::LATEST.major as i32).unwrap();
        let error = infos.prepare_commit(&dir).unwrap_err();
        assert!(error.to_string().contains("sync failed"), "{error}");
        let files = dir.list_all().unwrap();
        assert!(
            files
                .iter()
                .all(|name| !name.starts_with("pending_segments")),
            "the truncated file is deleted: {files:?}"
        );
        assert!(
            dir.log.close_count("pending_segments_1") >= 1,
            "the output was closed"
        );
    }
}
