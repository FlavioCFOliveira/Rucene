//! `CheckIndex` and `IndexUpgrader` ported from `org.apache.lucene.index`.

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::codec_reader::CodecReader;
use crate::index::index_deletion_policy::KeepOnlyLastCommitDeletionPolicy;
use crate::index::index_writer::IndexWriter;
use crate::index::index_writer_config::IndexWriterConfig;
use crate::index::leaf_reader::LeafReader;
use crate::index::merge_policy::UpgradeIndexMergePolicy;
use crate::index::segment_infos::SegmentInfos;
use crate::index::segment_reader::SegmentReader;
use crate::store::{DefaultIOContext, Directory, IOContext};
use crate::util::InfoStream;

/// What `CheckIndex` found about one segment.
///
/// Equivalent to `CheckIndex.Status.SegmentInfoStatus`.
#[derive(Debug, Default, Clone)]
pub struct SegmentInfoStatus {
    /// The segment's name.
    pub name: String,
    /// How many documents the segment holds, before deletions.
    pub max_doc: i32,
    /// How many documents are deleted.
    pub del_count: i32,
    /// Number of files the segment owns.
    pub num_files: usize,
    /// Total size of those files, in megabytes.
    pub size_mb: f64,
    /// Whether the segment passed every check.
    pub clean: bool,
    /// Why the segment failed, when it did.
    pub error: Option<String>,
}

/// What `CheckIndex` found about the whole index.
///
/// Equivalent to `CheckIndex.Status`.
#[derive(Debug, Default, Clone)]
pub struct Status {
    /// Whether every segment passed.
    pub clean: bool,
    /// Whether no `segments_N` file was found.
    pub missing_segments: bool,
    /// The name of the `segments_N` that was checked.
    pub segments_file_name: Option<String>,
    /// How many segments the index holds.
    pub num_segments: usize,
    /// How many segments failed a check.
    pub num_bad_segments: usize,
    /// How many documents would be lost by dropping the bad segments.
    pub tot_lose_doc_count: i32,
    /// The Lucene version that wrote the commit.
    pub user_data: std::collections::HashMap<String, String>,
    /// One entry per segment.
    pub segment_infos: Vec<SegmentInfoStatus>,
}

/// Verifies the structural integrity of an index.
///
/// Equivalent to `org.apache.lucene.index.CheckIndex`.
///
/// **Divergence from Lucene 10.5.0.** Java's `CheckIndex` re-reads every posting,
/// stored field, term vector, doc value, point and vector and cross-checks the
/// decoded values against the field metadata, filling in a dozen per-kind status
/// records. This port runs the codec-level verification each producer already
/// exposes through `check_integrity`, which is what validates the checksums of
/// every file the segment owns, plus the segment-level checks Java performs
/// before it opens a reader. The per-kind value-level cross-checks are not
/// reproduced, so a `clean` result here is a weaker statement than Java's.
pub struct CheckIndex {
    directory: Arc<dyn Directory>,
    info_stream: Option<Arc<dyn InfoStream>>,
}

impl CheckIndex {
    /// Creates a checker over `directory`.
    pub fn new(directory: Arc<dyn Directory>) -> Self {
        Self {
            directory,
            info_stream: None,
        }
    }

    /// Sets where progress messages go.
    ///
    /// Equivalent to `CheckIndex.setInfoStream`.
    pub fn set_info_stream(&mut self, info_stream: Arc<dyn InfoStream>) -> &mut Self {
        self.info_stream = Some(info_stream);
        self
    }

    /// Checks every segment of the latest commit.
    ///
    /// Equivalent to `CheckIndex.checkIndex()`.
    pub fn check_index(&self) -> Result<Status> {
        let mut status = Status::default();

        let segments_file_name =
            SegmentInfos::get_last_commit_segments_file_name_dir(self.directory.as_ref())?;
        let Some(segments_file_name) = segments_file_name else {
            status.missing_segments = true;
            return Ok(status);
        };
        status.segments_file_name = Some(segments_file_name.clone());

        let segment_infos =
            SegmentInfos::read_commit(self.directory.as_ref(), &segments_file_name)?;
        status.num_segments = segment_infos.size();
        status.user_data = segment_infos.user_data().clone();

        let created_version_major = segment_infos.index_created_version_major();
        let context: Arc<dyn IOContext> = Arc::new(DefaultIOContext::new(Vec::new())?);

        for info in segment_infos.iter() {
            let mut segment_status = SegmentInfoStatus {
                name: info.info.name.clone(),
                del_count: info.get_del_count(),
                ..Default::default()
            };

            let result = (|| -> Result<()> {
                segment_status.max_doc = info.info.max_doc()?;
                let files = info.files()?;
                segment_status.num_files = files.len();
                let mut total = 0i64;
                for file in &files {
                    total += self.directory.file_length(file)?;
                }
                segment_status.size_mb = total as f64 / 1024.0 / 1024.0;

                if segment_status.del_count < 0 || segment_status.del_count > segment_status.max_doc
                {
                    return Err(LuceneError::corrupt_index(
                        format!(
                            "delCount {} is out of range for maxDoc {}",
                            segment_status.del_count, segment_status.max_doc
                        ),
                        &info.info.name,
                    ));
                }

                // Opening the reader validates the headers and footers of every
                // file the segment owns; `check_integrity` then verifies the
                // checksums the codecs recorded.
                let reader =
                    SegmentReader::new(info.clone(), created_version_major, context.as_ref())?;
                LeafReader::check_integrity(&reader)?;
                if let Some(producer) = CodecReader::get_postings_reader(&reader)? {
                    producer.check_integrity()?;
                }
                Ok(())
            })();

            match result {
                Ok(()) => segment_status.clean = true,
                Err(err) => {
                    segment_status.clean = false;
                    segment_status.error = Some(err.to_string());
                    status.num_bad_segments += 1;
                    status.tot_lose_doc_count += segment_status.max_doc;
                    if let Some(stream) = &self.info_stream {
                        stream.message(
                            "CheckIndex",
                            &format!("segment {} FAILED: {err}", segment_status.name),
                        );
                    }
                }
            }

            status.segment_infos.push(segment_status);
        }

        status.clean = status.num_bad_segments == 0;
        Ok(status)
    }
}

/// Rewrites every segment written by an older Lucene release.
///
/// Equivalent to `org.apache.lucene.index.IndexUpgrader`.
pub struct IndexUpgrader {
    directory: Arc<dyn Directory>,
    config: IndexWriterConfig,
    delete_prior_commits: bool,
}

impl IndexUpgrader {
    /// Creates an upgrader over `directory`.
    pub fn new(
        directory: Arc<dyn Directory>,
        config: IndexWriterConfig,
        delete_prior_commits: bool,
    ) -> Self {
        Self {
            directory,
            config,
            delete_prior_commits,
        }
    }

    /// Rewrites the index so every segment carries the current format.
    ///
    /// Equivalent to `IndexUpgrader.upgrade()`. It installs
    /// [`UpgradeIndexMergePolicy`] over the configured policy and forces the
    /// index down to one segment, which is what rewrites the old ones.
    pub fn upgrade(mut self) -> Result<()> {
        if SegmentInfos::get_last_commit_segments_file_name_dir(self.directory.as_ref())?.is_none()
        {
            return Err(LuceneError::index_not_found(
                "no segments file found in the directory",
            ));
        }

        if !self.delete_prior_commits {
            // Java refuses to run when several commits exist and it was told not
            // to delete them, because a forced merge would drop the old ones.
            let generation = SegmentInfos::get_last_commit_generation_dir(self.directory.as_ref())?;
            if generation > 1 {
                return Err(LuceneError::IllegalArgument(
                    "this tool was invoked to not delete prior commit points, but more than one commit was found"
                        .to_string(),
                ));
            }
        }

        let inner_policy = self.config.merge_policy();
        self.config
            .set_merge_policy(Arc::new(UpgradeIndexMergePolicy::new(inner_policy)))?;
        self.config
            .set_index_deletion_policy(Arc::new(KeepOnlyLastCommitDeletionPolicy))?;

        let writer = IndexWriter::new(Arc::clone(&self.directory), self.config)?;
        writer.force_merge(1)?;
        writer.close()
    }
}
