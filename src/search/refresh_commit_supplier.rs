//! Refresh commit selection, ported from
//! `org.apache.lucene.search.RefreshCommitSupplier`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::Result;
use crate::index::{DirectoryReader, IndexCommit};

/// Expert: supplies the commit a searcher should refresh on.
///
/// Equivalent to the interface
/// `org.apache.lucene.search.RefreshCommitSupplier`.
pub trait RefreshCommitSupplier: Send + Sync + std::fmt::Debug {
    /// Expert: returns the index commit the searcher should refresh on. `None`
    /// — the default — means the reader should refresh on the latest commit.
    ///
    /// Equivalent to
    /// `RefreshCommitSupplier.getSearcherRefreshCommit(DirectoryReader)`,
    /// whose default returns `null`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while locating the commit.
    fn get_searcher_refresh_commit(
        &self,
        _reader: &Arc<dyn DirectoryReader>,
    ) -> Result<Option<Arc<dyn IndexCommit>>> {
        Ok(None)
    }
}

/// The supplier `new RefreshCommitSupplier() {}` produces: it always refreshes
/// on the latest commit.
#[derive(Debug, Default, Clone, Copy)]
pub struct LatestCommitSupplier;

impl RefreshCommitSupplier for LatestCommitSupplier {}
