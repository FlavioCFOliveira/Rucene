//! Segment metadata ported from `org.apache.lucene.index`.
//!
//! This module provides the data structures that identify a segment and carry
//! the context passed to every codec format during read and write operations.

#![deny(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::fmt::{self, Debug, Formatter};
use std::sync::{Arc, RwLock};

use crate::codecs::Codec;
use crate::error::{LuceneError, Result};
use crate::index::IndexReader;
use crate::search::Sort;
use crate::store::Directory;
use crate::util::string_helper::{StringHelper, ID_LENGTH};
use crate::util::Version;

pub use crate::codecs::state::{SegmentReadState, SegmentWriteState};

// -----------------------------------------------------------------------------
// SegmentInfo
// -----------------------------------------------------------------------------

/// Metadata identifying a single segment.
///
/// Equivalent to `org.apache.lucene.index.SegmentInfo`.
pub struct SegmentInfo {
    /// Unique segment name in the directory.
    pub name: String,

    /// Directory where the segment resides.
    pub directory: Arc<dyn Directory>,

    /// Number of documents in the segment (deletions not taken into account).
    max_doc: i32,

    /// Lucene version that wrote the segment.
    version: Version,

    /// Minimum Lucene version that contributed documents to the segment, if known.
    min_version: Option<Version>,

    /// Whether the segment is stored as a compound file.
    is_compound_file: bool,

    /// Whether the segment contains block-joined documents.
    has_blocks: bool,

    /// Codec that wrote the segment.
    codec: Option<Arc<dyn Codec>>,

    /// Diagnostics saved when the segment was written.
    diagnostics: HashMap<String, String>,

    /// Codec-private attributes.
    attributes: RwLock<HashMap<String, String>>,

    /// 16-byte id that uniquely identifies this segment.
    id: [u8; ID_LENGTH],

    /// Sort order of the segment, if any.
    index_sort: Sort,

    /// Files referenced by this segment.
    files: RwLock<Option<HashSet<String>>>,
}

impl SegmentInfo {
    /// Sentinel value meaning a feature is absent.
    pub const NO: i32 = -1;

    /// Sentinel value meaning a feature is present.
    pub const YES: i32 = 1;

    /// Creates a new complete `SegmentInfo`.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `max_doc` is negative.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        directory: Arc<dyn Directory>,
        version: Version,
        min_version: Option<Version>,
        name: String,
        max_doc: i32,
        is_compound_file: bool,
        has_blocks: bool,
        codec: Arc<dyn Codec>,
        diagnostics: HashMap<String, String>,
        id: [u8; ID_LENGTH],
        attributes: HashMap<String, String>,
        index_sort: Sort,
    ) -> Result<Self> {
        if max_doc < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "maxDoc must be non-negative: {max_doc}"
            )));
        }
        Ok(Self {
            directory,
            version,
            min_version,
            name,
            max_doc,
            is_compound_file,
            has_blocks,
            codec: Some(codec),
            diagnostics,
            id,
            attributes: RwLock::new(attributes),
            index_sort,
            files: RwLock::new(None),
        })
    }

    /// Creates a new `SegmentInfo` without an associated codec.
    ///
    /// This is used by `SegmentInfoFormat::read` before the codec name has been
    /// resolved from the enclosing `SegmentInfos`. The codec must be set later
    /// with [`set_codec`](Self::set_codec).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `max_doc` is negative.
    #[allow(clippy::too_many_arguments)]
    pub fn new_without_codec(
        directory: Arc<dyn Directory>,
        version: Version,
        min_version: Option<Version>,
        name: String,
        max_doc: i32,
        is_compound_file: bool,
        has_blocks: bool,
        diagnostics: HashMap<String, String>,
        id: [u8; ID_LENGTH],
        attributes: HashMap<String, String>,
        index_sort: Sort,
    ) -> Result<Self> {
        if max_doc < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "maxDoc must be non-negative: {max_doc}"
            )));
        }
        Ok(Self {
            directory,
            version,
            min_version,
            name,
            max_doc,
            is_compound_file,
            has_blocks,
            codec: None,
            diagnostics,
            id,
            attributes: RwLock::new(attributes),
            index_sort,
            files: RwLock::new(None),
        })
    }

    /// Returns the number of documents in the segment.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalState` if `max_doc` has not been set.
    pub fn max_doc(&self) -> Result<i32> {
        if self.max_doc < 0 {
            return Err(LuceneError::IllegalState(
                "maxDoc isn't set yet".to_string(),
            ));
        }
        Ok(self.max_doc)
    }

    /// Sets the number of documents in the segment.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalState` if `max_doc` was already set.
    pub fn set_max_doc(&mut self, max_doc: i32) -> Result<()> {
        if self.max_doc >= 0 {
            return Err(LuceneError::IllegalState(format!(
                "maxDoc was already set: this.maxDoc={} vs maxDoc={max_doc}",
                self.max_doc
            )));
        }
        if max_doc < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "maxDoc must be non-negative: {max_doc}"
            )));
        }
        self.max_doc = max_doc;
        Ok(())
    }

    /// Returns the version of the code that wrote the segment.
    pub fn version(&self) -> Version {
        self.version
    }

    /// Returns the minimum version that contributed documents, if known.
    pub fn min_version(&self) -> Option<Version> {
        self.min_version
    }

    /// Returns the 16-byte segment id.
    pub fn id(&self) -> [u8; ID_LENGTH] {
        self.id
    }

    /// Returns whether this segment is stored as a compound file.
    pub fn get_use_compound_file(&self) -> bool {
        self.is_compound_file
    }

    /// Sets whether this segment is stored as a compound file.
    pub fn set_use_compound_file(&mut self, is_compound_file: bool) {
        self.is_compound_file = is_compound_file;
    }

    /// Returns whether this segment contains block-joined documents.
    pub fn get_has_blocks(&self) -> bool {
        self.has_blocks
    }

    /// Marks this segment as containing block-joined documents.
    pub fn set_has_blocks(&mut self) {
        self.has_blocks = true;
    }

    /// Returns the codec that wrote this segment.
    pub fn codec(&self) -> Option<Arc<dyn Codec>> {
        self.codec.clone()
    }

    /// Sets the codec for this segment.
    pub fn set_codec(&mut self, codec: Arc<dyn Codec>) {
        self.codec = Some(codec);
    }

    /// Returns the diagnostics map.
    pub fn get_diagnostics(&self) -> &HashMap<String, String> {
        &self.diagnostics
    }

    /// Replaces the diagnostics map with a copy of the provided one.
    pub fn set_diagnostics(&mut self, diagnostics: HashMap<String, String>) {
        self.diagnostics = diagnostics;
    }

    /// Adds or overwrites entries from `diagnostics`.
    pub fn add_diagnostics(&mut self, diagnostics: HashMap<String, String>) {
        for (k, v) in diagnostics {
            self.diagnostics.insert(k, v);
        }
    }

    /// Returns a codec attribute, if present.
    pub fn get_attribute(&self, key: &str) -> Option<String> {
        self.attributes.read().unwrap().get(key).cloned()
    }

    /// Stores a codec attribute, returning the previous value if any.
    pub fn put_attribute(&self, key: String, value: String) -> Option<String> {
        let mut attrs = self.attributes.write().unwrap();
        attrs.insert(key, value)
    }

    /// Returns a copy of the codec attributes map.
    pub fn get_attributes(&self) -> HashMap<String, String> {
        self.attributes.read().unwrap().clone()
    }

    /// Returns the index sort for this segment.
    pub fn index_sort(&self) -> &Sort {
        &self.index_sort
    }

    /// Replaces the index sort for this segment.
    pub fn set_index_sort(&mut self, sort: Sort) {
        self.index_sort = sort;
    }

    /// Returns the files referenced by this segment.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalState` if the file set has not been computed.
    pub fn files(&self) -> Result<HashSet<String>> {
        let files = self.files.read().unwrap();
        match files.as_ref() {
            Some(f) => Ok(f.clone()),
            None => Err(LuceneError::IllegalState(format!(
                "files were not computed yet; segment={} maxDoc={}",
                self.name, self.max_doc
            ))),
        }
    }

    /// Replaces the set of files referenced by this segment.
    pub fn set_files(&self, files: HashSet<String>) {
        *self.files.write().unwrap() = Some(files);
    }

    /// Adds files to the set referenced by this segment.
    pub fn add_files(&self, files: &[String]) {
        let mut guard = self.files.write().unwrap();
        match guard.as_mut() {
            Some(set) => {
                for f in files {
                    set.insert(self.named_for_this_segment(f));
                }
            }
            None => {
                let mut set = HashSet::new();
                for f in files {
                    set.insert(self.named_for_this_segment(f));
                }
                *guard = Some(set);
            }
        }
    }

    /// Adds a single file to the set referenced by this segment.
    pub fn add_file(&self, file: String) {
        self.add_files(&[file]);
    }

    /// Strips any segment name from `file` and renames it with this segment's name.
    pub fn named_for_this_segment(&self, file: &str) -> String {
        format!("{}{}", self.name, strip_segment_name(file))
    }

    /// Returns a short debug representation, optionally including a deletion count.
    pub fn to_string_with_del_count(&self, del_count: i32) -> String {
        let mut s = String::new();
        s.push_str(&self.name);
        s.push('(');
        s.push_str(&self.version.to_string());
        s.push(')');
        s.push(':');
        s.push(if self.is_compound_file { 'c' } else { 'C' });
        s.push_str(&self.max_doc.to_string());
        if del_count != 0 {
            s.push('/');
            s.push_str(&del_count.to_string());
        }
        s
    }
}

impl Debug for SegmentInfo {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SegmentInfo")
            .field("name", &self.name)
            .field("max_doc", &self.max_doc)
            .field("version", &self.version)
            .field("is_compound_file", &self.is_compound_file)
            .field("has_blocks", &self.has_blocks)
            .finish()
    }
}

impl Clone for SegmentInfo {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            directory: Arc::clone(&self.directory),
            max_doc: self.max_doc,
            version: self.version,
            min_version: self.min_version,
            is_compound_file: self.is_compound_file,
            has_blocks: self.has_blocks,
            codec: self.codec.clone(),
            diagnostics: self.diagnostics.clone(),
            id: self.id,
            attributes: RwLock::new(self.get_attributes()),
            index_sort: self.index_sort.clone(),
            files: RwLock::new(Some(self.files().unwrap_or_default())),
        }
    }
}

impl PartialEq for SegmentInfo {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.directory, &other.directory) && self.name == other.name
    }
}

impl Eq for SegmentInfo {}

/// Strips the leading segment name from a file name, leaving the extension suffix.
///
/// Equivalent to `org.apache.lucene.index.IndexFileNames.stripSegmentName`.
fn strip_segment_name(name: &str) -> &str {
    // Match Lucene's IndexFileNames.stripSegmentName: the boundary is the first
    // '_' after the leading segment underscore, or the first '.' if no such '_'.
    match name.find('_').filter(|pos| *pos > 0) {
        Some(pos) => &name[pos..],
        None => match name.find('.') {
            Some(pos) => &name[pos..],
            None => name,
        },
    }
}

// -----------------------------------------------------------------------------
// SegmentCommitInfo
// -----------------------------------------------------------------------------

/// Embeds a read-only `SegmentInfo` and adds per-commit fields.
///
/// Equivalent to `org.apache.lucene.index.SegmentCommitInfo`.
pub struct SegmentCommitInfo {
    /// The `SegmentInfo` wrapped by this commit info.
    pub info: SegmentInfo,

    /// 16-byte id identifying this segment commit.
    id: [u8; ID_LENGTH],

    /// Number of hard-deleted documents.
    del_count: i32,

    /// Number of soft-deleted documents that are not also hard-deleted.
    soft_del_count: i32,

    /// Generation number of the live docs file, or -1 if no deletes.
    del_gen: i64,

    /// Normally `1 + del_gen`, unless a write failed.
    next_write_del_gen: i64,

    /// Generation number of the field infos update file, or -1 if no updates.
    field_infos_gen: i64,

    /// Normally `1 + field_infos_gen`, unless a write failed.
    next_write_field_infos_gen: i64,

    /// Generation number of the doc values update file, or -1 if no updates.
    doc_values_gen: i64,

    /// Normally `1 + doc_values_gen`, unless a write failed.
    next_write_doc_values_gen: i64,

    /// Per-field doc values update files.
    dv_updates_files: HashMap<i32, HashSet<String>>,

    /// Field infos update files.
    field_infos_files: HashSet<String>,

    /// Cached total size in bytes of all files for this segment; -1 if not computed.
    size_in_bytes: i64,

    /// Buffered deletes generation, only used in-RAM by IndexWriter.
    buffered_deletes_gen: i64,
}

impl SegmentCommitInfo {
    /// Creates a new `SegmentCommitInfo`.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `del_count` or `soft_del_count`
    /// are out of range for the wrapped `SegmentInfo::max_doc`.
    pub fn new(
        info: SegmentInfo,
        del_count: i32,
        soft_del_count: i32,
        del_gen: i64,
        field_infos_gen: i64,
        doc_values_gen: i64,
        id: [u8; ID_LENGTH],
    ) -> Result<Self> {
        let max_doc = info.max_doc()?;
        if del_count < 0 || del_count > max_doc {
            return Err(LuceneError::IllegalArgument(format!(
                "invalid delCount={del_count} (maxDoc={max_doc})"
            )));
        }
        if soft_del_count < 0 || soft_del_count > max_doc {
            return Err(LuceneError::IllegalArgument(format!(
                "invalid softDelCount={soft_del_count} (maxDoc={max_doc})"
            )));
        }
        if del_count + soft_del_count > max_doc {
            return Err(LuceneError::IllegalArgument(format!(
                "delCount + softDelCount > maxDoc (maxDoc={max_doc}, delCount={del_count}, softDelCount={soft_del_count})"
            )));
        }
        Ok(Self {
            info,
            id,
            del_count,
            soft_del_count,
            del_gen,
            next_write_del_gen: if del_gen == -1 { 1 } else { del_gen + 1 },
            field_infos_gen,
            next_write_field_infos_gen: if field_infos_gen == -1 {
                1
            } else {
                field_infos_gen + 1
            },
            doc_values_gen,
            next_write_doc_values_gen: if doc_values_gen == -1 {
                1
            } else {
                doc_values_gen + 1
            },
            dv_updates_files: HashMap::new(),
            field_infos_files: HashSet::new(),
            size_in_bytes: -1,
            buffered_deletes_gen: -1,
        })
    }

    /// Returns the id identifying this segment commit.
    pub fn id(&self) -> [u8; ID_LENGTH] {
        self.id
    }

    /// Returns the number of hard-deleted documents.
    pub fn get_del_count(&self) -> i32 {
        self.del_count
    }

    /// Sets the number of hard-deleted documents.
    pub fn set_del_count(&mut self, del_count: i32) -> Result<()> {
        let max_doc = self.info.max_doc()?;
        if del_count < 0 || del_count > max_doc {
            return Err(LuceneError::IllegalArgument(format!(
                "invalid delCount={del_count} (maxDoc={max_doc})"
            )));
        }
        if del_count + self.soft_del_count > max_doc {
            return Err(LuceneError::IllegalArgument(format!(
                "delCount + softDelCount > maxDoc (maxDoc={max_doc}, delCount={del_count}, softDelCount={})",
                self.soft_del_count
            )));
        }
        self.del_count = del_count;
        Ok(())
    }

    /// Returns the number of soft-deleted documents not also hard-deleted.
    pub fn get_soft_del_count(&self) -> i32 {
        self.soft_del_count
    }

    /// Sets the number of soft-deleted documents.
    pub fn set_soft_del_count(&mut self, soft_del_count: i32) -> Result<()> {
        let max_doc = self.info.max_doc()?;
        if soft_del_count < 0 || soft_del_count > max_doc {
            return Err(LuceneError::IllegalArgument(format!(
                "invalid softDelCount={soft_del_count} (maxDoc={max_doc})"
            )));
        }
        if self.del_count + soft_del_count > max_doc {
            return Err(LuceneError::IllegalArgument(format!(
                "delCount + softDelCount > maxDoc (maxDoc={max_doc}, delCount={}, softDelCount={soft_del_count})",
                self.del_count
            )));
        }
        self.soft_del_count = soft_del_count;
        Ok(())
    }

    /// Returns true if there are any deletions for this segment commit.
    pub fn has_deletions(&self) -> bool {
        self.del_gen != -1
    }

    /// Returns true if there are any field updates for this segment commit.
    pub fn has_field_updates(&self) -> bool {
        self.field_infos_gen != -1
    }

    /// Returns the live docs file generation.
    pub fn get_del_gen(&self) -> i64 {
        self.del_gen
    }

    /// Returns the next available live docs file generation.
    pub fn get_next_del_gen(&self) -> i64 {
        self.next_write_del_gen
    }

    /// Advances the live docs generation after a successful write.
    pub fn advance_del_gen(&mut self) {
        self.del_gen = self.next_write_del_gen;
        self.next_write_del_gen = self.del_gen + 1;
        self.generation_advanced();
    }

    /// Advances the next-write live docs generation after a failed write.
    pub fn advance_next_write_del_gen(&mut self) {
        self.next_write_del_gen += 1;
    }

    /// Returns the field infos update file generation.
    pub fn get_field_infos_gen(&self) -> i64 {
        self.field_infos_gen
    }

    /// Returns the next available field infos update generation.
    pub fn get_next_field_infos_gen(&self) -> i64 {
        self.next_write_field_infos_gen
    }

    /// Advances the field infos generation after a successful write.
    pub fn advance_field_infos_gen(&mut self) {
        self.field_infos_gen = self.next_write_field_infos_gen;
        self.next_write_field_infos_gen = self.field_infos_gen + 1;
        self.generation_advanced();
    }

    /// Advances the next-write field infos generation after a failed write.
    pub fn advance_next_write_field_infos_gen(&mut self) {
        self.next_write_field_infos_gen += 1;
    }

    /// Returns the doc values update file generation.
    pub fn get_doc_values_gen(&self) -> i64 {
        self.doc_values_gen
    }

    /// Returns the next available doc values update generation.
    pub fn get_next_doc_values_gen(&self) -> i64 {
        self.next_write_doc_values_gen
    }

    /// Advances the doc values generation after a successful write.
    pub fn advance_doc_values_gen(&mut self) {
        self.doc_values_gen = self.next_write_doc_values_gen;
        self.next_write_doc_values_gen = self.doc_values_gen + 1;
        self.generation_advanced();
    }

    /// Advances the next-write doc values generation after a failed write.
    pub fn advance_next_write_doc_values_gen(&mut self) {
        self.next_write_doc_values_gen += 1;
    }

    /// Returns the per-field doc values update files.
    pub fn get_doc_values_updates_files(&self) -> &HashMap<i32, HashSet<String>> {
        &self.dv_updates_files
    }

    /// Sets the per-field doc values update files.
    pub fn set_doc_values_updates_files(&mut self, files: HashMap<i32, HashSet<String>>) {
        self.dv_updates_files.clear();
        for (field_number, set) in files {
            let renamed: HashSet<String> = set
                .into_iter()
                .map(|f| self.info.named_for_this_segment(&f))
                .collect();
            self.dv_updates_files.insert(field_number, renamed);
        }
    }

    /// Returns the field infos update files.
    pub fn get_field_infos_files(&self) -> &HashSet<String> {
        &self.field_infos_files
    }

    /// Sets the field infos update files.
    pub fn set_field_infos_files(&mut self, files: HashSet<String>) {
        self.field_infos_files.clear();
        for f in files {
            self.field_infos_files
                .insert(self.info.named_for_this_segment(&f));
        }
    }

    /// Returns the buffered deletes generation.
    pub fn get_buffered_deletes_gen(&self) -> i64 {
        self.buffered_deletes_gen
    }

    /// Sets the buffered deletes generation.
    pub fn set_buffered_deletes_gen(&mut self, gen: i64) -> Result<()> {
        if self.buffered_deletes_gen != -1 {
            return Err(LuceneError::IllegalState(
                "buffered deletes gen should only be set once".to_string(),
            ));
        }
        self.buffered_deletes_gen = gen;
        self.generation_advanced();
        Ok(())
    }

    /// Returns the total number of deleted documents, optionally including soft deletes.
    pub fn get_del_count_with_soft(&self, include_soft: bool) -> i32 {
        if include_soft {
            self.del_count + self.soft_del_count
        } else {
            self.del_count
        }
    }

    fn generation_advanced(&mut self) {
        self.size_in_bytes = -1;
        self.id = StringHelper::random_id();
    }

    /// Returns all files in use by this segment commit.
    ///
    /// Equivalent to `SegmentCommitInfo.files()`.
    pub fn files(&self) -> Result<HashSet<String>> {
        let mut files = self.info.files()?;

        if self.has_deletions() {
            if let Some(codec) = self.info.codec() {
                let mut live_docs_files = Vec::new();
                codec.live_docs_format().files(self, &mut live_docs_files)?;
                for file in live_docs_files {
                    files.insert(file);
                }
            }
        }

        for set in self.dv_updates_files.values() {
            for file in set {
                files.insert(file.clone());
            }
        }

        for file in &self.field_infos_files {
            files.insert(file.clone());
        }

        Ok(files)
    }

    /// Returns the total size in bytes of all files for this segment.
    ///
    /// Equivalent to `SegmentCommitInfo.sizeInBytes()`.
    pub fn size_in_bytes(&mut self) -> Result<i64> {
        if self.size_in_bytes == -1 {
            let mut sum = 0i64;
            for file in self.files()? {
                sum += self.info.directory.file_length(&file)?;
            }
            self.size_in_bytes = sum;
        }
        Ok(self.size_in_bytes)
    }
}

impl Debug for SegmentCommitInfo {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SegmentCommitInfo")
            .field("info", &self.info)
            .field("del_count", &self.del_count)
            .field("soft_del_count", &self.soft_del_count)
            .field("del_gen", &self.del_gen)
            .field("field_infos_gen", &self.field_infos_gen)
            .field("doc_values_gen", &self.doc_values_gen)
            .finish()
    }
}

impl Clone for SegmentCommitInfo {
    fn clone(&self) -> Self {
        let mut other = Self {
            info: self.info.clone(),
            id: self.id,
            del_count: self.del_count,
            soft_del_count: self.soft_del_count,
            del_gen: self.del_gen,
            next_write_del_gen: self.next_write_del_gen,
            field_infos_gen: self.field_infos_gen,
            next_write_field_infos_gen: self.next_write_field_infos_gen,
            doc_values_gen: self.doc_values_gen,
            next_write_doc_values_gen: self.next_write_doc_values_gen,
            dv_updates_files: self.dv_updates_files.clone(),
            field_infos_files: self.field_infos_files.clone(),
            size_in_bytes: self.size_in_bytes,
            buffered_deletes_gen: self.buffered_deletes_gen,
        };
        other.id = StringHelper::random_id();
        other
    }
}

impl PartialEq for SegmentCommitInfo {
    fn eq(&self, other: &Self) -> bool {
        self.info == other.info
            && self.del_count == other.del_count
            && self.soft_del_count == other.soft_del_count
            && self.del_gen == other.del_gen
            && self.field_infos_gen == other.field_infos_gen
            && self.doc_values_gen == other.doc_values_gen
    }
}

impl Eq for SegmentCommitInfo {}

// -----------------------------------------------------------------------------
// SegmentOrder
// -----------------------------------------------------------------------------

/// Utility class to re-order segments within an `IndexReader` to assist in early
/// termination.
///
/// Equivalent to `org.apache.lucene.index.SegmentOrder`.
///
/// The full numeric sorter implementation depends on `LeafReader` point and
/// doc-values skipper APIs that are still being ported. This struct provides
/// the public API surface and an identity fallback.
#[derive(Debug)]
pub struct SegmentOrder {
    inner: Box<dyn SegmentOrderImpl>,
}

impl SegmentOrder {
    /// Builds a sorter from the primary numeric field of `sort`.
    ///
    /// If the primary sort field is not numeric, this currently returns an
    /// identity orderer (no reordering), matching the Java no-op behaviour for
    /// non-numeric sorts.
    pub fn from_sort(_sort: &Sort) -> Self {
        Self {
            inner: Box::new(IdentitySegmentOrder),
        }
    }

    /// Produces a new view over `reader` by re-ordering the reader's segments.
    pub fn reorder(&self, reader: Arc<dyn IndexReader>) -> Result<Arc<dyn IndexReader>> {
        self.inner.reorder(reader)
    }
}

trait SegmentOrderImpl: Send + Sync + Debug {
    fn reorder(&self, reader: Arc<dyn IndexReader>) -> Result<Arc<dyn IndexReader>>;
}

#[derive(Debug)]
struct IdentitySegmentOrder;

impl SegmentOrderImpl for IdentitySegmentOrder {
    fn reorder(&self, reader: Arc<dyn IndexReader>) -> Result<Arc<dyn IndexReader>> {
        Ok(reader)
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::state::{SegmentReadState, SegmentWriteState};
    use crate::codecs::tests::DummyCodec;
    use crate::codecs::FilterCodec;
    use crate::index::FieldInfos;
    use crate::store::{DefaultIOContext, Directory, RamDirectory};
    use crate::util::Version;
    use std::sync::Arc;

    fn test_codec() -> Arc<dyn Codec> {
        Arc::new(FilterCodec::new(
            "TestCodec",
            Arc::new(DummyCodec::new("Dummy")),
        ))
    }

    fn test_segment_info(name: &str, max_doc: i32) -> SegmentInfo {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        SegmentInfo::new(
            dir,
            Version::LUCENE_10_5_0,
            Some(Version::LUCENE_10_5_0),
            name.to_string(),
            max_doc,
            false,
            false,
            test_codec(),
            HashMap::from([("source".to_string(), "flush".to_string())]),
            StringHelper::random_id(),
            HashMap::from([("test".to_string(), "value".to_string())]),
            Sort::default(),
        )
        .unwrap()
    }

    #[test]
    fn segment_info_basic() {
        let info = test_segment_info("_0", 42);
        assert_eq!(info.name, "_0");
        assert_eq!(info.max_doc().unwrap(), 42);
        assert_eq!(info.version(), Version::LUCENE_10_5_0);
        assert!(!info.get_use_compound_file());
        assert!(!info.get_has_blocks());
        assert_eq!(info.get_attribute("test").unwrap(), "value");
        assert_eq!(info.get_diagnostics().get("source").unwrap(), "flush");
    }

    #[test]
    fn segment_info_files_round_trip() {
        let info = test_segment_info("_0", 10);
        let mut files = HashSet::new();
        files.insert("_0.fnm".to_string());
        files.insert("_0.si".to_string());
        info.set_files(files);
        assert_eq!(info.files().unwrap().len(), 2);

        info.add_file("_0.doc".to_string());
        let set = info.files().unwrap();
        assert!(set.contains("_0.doc"));
        assert!(set.contains("_0.fnm"));
        assert!(set.contains("_0.si"));
    }

    #[test]
    fn segment_info_named_for_this_segment() {
        let info = test_segment_info("_a", 5);
        assert_eq!(info.named_for_this_segment("_0.fnm"), "_a.fnm");
        assert_eq!(info.named_for_this_segment("fnm"), "_afnm");
    }

    #[test]
    fn segment_info_max_doc_validation() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let result = SegmentInfo::new(
            dir,
            Version::LUCENE_10_5_0,
            Some(Version::LUCENE_10_5_0),
            "_0".to_string(),
            -1,
            false,
            false,
            test_codec(),
            HashMap::new(),
            StringHelper::random_id(),
            HashMap::new(),
            Sort::default(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn segment_commit_info_basic() {
        let info = test_segment_info("_0", 100);
        let mut sci =
            SegmentCommitInfo::new(info, 5, 3, 0, -1, -1, StringHelper::random_id()).unwrap();
        assert_eq!(sci.get_del_count(), 5);
        assert_eq!(sci.get_soft_del_count(), 3);
        assert!(sci.has_deletions());
        assert!(!sci.has_field_updates());
        assert_eq!(sci.get_del_gen(), 0);
        assert_eq!(sci.get_next_del_gen(), 1);

        sci.advance_del_gen();
        assert_eq!(sci.get_del_gen(), 1);
        assert_eq!(sci.get_next_del_gen(), 2);
    }

    #[test]
    fn segment_commit_info_clone_changes_id() {
        let info = test_segment_info("_0", 10);
        let sci =
            SegmentCommitInfo::new(info, 0, 0, -1, -1, -1, StringHelper::random_id()).unwrap();
        let cloned = sci.clone();
        assert_eq!(sci.get_del_count(), cloned.get_del_count());
        assert_ne!(sci.id(), cloned.id());
    }

    #[test]
    fn segment_commit_info_del_count_validation() {
        let info = test_segment_info("_0", 11);
        let mut sci =
            SegmentCommitInfo::new(info, 0, 0, -1, -1, -1, StringHelper::random_id()).unwrap();
        assert!(sci.set_del_count(12).is_err());
        assert!(sci.set_soft_del_count(12).is_err());
        assert!(sci.set_del_count(6).is_ok());
        assert!(sci.set_soft_del_count(5).is_ok());
        assert!(sci.set_soft_del_count(6).is_err());
    }

    #[test]
    fn segment_read_state_new() {
        let dir: &dyn Directory = &RamDirectory::default();
        let info = test_segment_info("_0", 10);
        let field_infos = FieldInfos::default();
        let ctx: &dyn crate::store::IOContext = &DefaultIOContext::default();
        let state = SegmentReadState::new(dir, &info, &field_infos, ctx);
        assert!(state.segment_suffix.is_empty());
        assert_eq!(state.segment_info.name, "_0");

        let state2 = state.with_new_suffix("_1".to_string());
        assert_eq!(state2.segment_suffix, "_1");
        assert_eq!(state2.segment_info.name, "_0");
    }

    #[test]
    fn segment_write_state_new() {
        let info_stream: &dyn crate::util::InfoStream = crate::util::default_info_stream();
        let dir: &dyn Directory = &RamDirectory::default();
        let info = test_segment_info("_0", 10);
        let field_infos = FieldInfos::default();
        let seg_updates = crate::codecs::stub::BufferedUpdates;
        let ctx: &dyn crate::store::IOContext = &DefaultIOContext::default();
        let mut state =
            SegmentWriteState::new(info_stream, dir, &info, &field_infos, &seg_updates, ctx);
        assert!(state.segment_suffix.is_empty());
        assert_eq!(state.del_count_on_flush, 0);

        state.live_docs = Some(crate::util::FixedBitSet::new(10));
        let state2 = state.with_new_suffix("_2".to_string());
        assert_eq!(state2.segment_suffix, "_2");
        assert!(state2.live_docs.is_some());
    }

    #[test]
    fn segment_info_to_string_with_del_count() {
        let info = test_segment_info("_a", 45);
        let s = info.to_string_with_del_count(4);
        assert!(s.starts_with("_a("));
        assert!(s.contains("):C45/4"));
    }
}
