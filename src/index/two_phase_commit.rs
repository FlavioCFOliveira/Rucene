//! `TwoPhaseCommit` and `TwoPhaseCommitTool` ported from
//! `org.apache.lucene.index`.

use crate::error::{LuceneError, Result};

/// An object that can take part in a two-phase commit.
///
/// Equivalent to `org.apache.lucene.index.TwoPhaseCommit`.
pub trait TwoPhaseCommit {
    /// Does as much work as possible without making the changes visible.
    ///
    /// Equivalent to `TwoPhaseCommit.prepareCommit()`.
    fn prepare_commit(&self) -> Result<i64>;

    /// Makes the prepared changes durable. Should do very little work.
    ///
    /// Equivalent to `TwoPhaseCommit.commit()`.
    fn commit(&self) -> Result<i64>;

    /// Discards everything since the last successful commit.
    ///
    /// Equivalent to `TwoPhaseCommit.rollback()`.
    fn rollback(&self) -> Result<()>;
}

/// Runs a two-phase commit across several participants.
///
/// Equivalent to `org.apache.lucene.index.TwoPhaseCommitTool`.
pub struct TwoPhaseCommitTool;

impl TwoPhaseCommitTool {
    /// Prepares then commits every participant, rolling all of them back if any
    /// step fails.
    ///
    /// Equivalent to `TwoPhaseCommitTool.execute(TwoPhaseCommit...)`. The error
    /// carries which phase failed, as Java's `PrepareCommitFailException` and
    /// `CommitFailException` do.
    pub fn execute(objects: &[&dyn TwoPhaseCommit]) -> Result<()> {
        for (index, object) in objects.iter().enumerate() {
            if let Err(err) = object.prepare_commit() {
                Self::rollback(objects);
                return Err(LuceneError::Other(format!(
                    "prepareCommit() failed on participant {index}: {err}"
                )));
            }
        }
        for (index, object) in objects.iter().enumerate() {
            if let Err(err) = object.commit() {
                Self::rollback(objects);
                return Err(LuceneError::Other(format!(
                    "commit() failed on participant {index}: {err}"
                )));
            }
        }
        Ok(())
    }

    /// Rolls every participant back, ignoring the errors each raises so that
    /// none is left half-committed.
    ///
    /// Equivalent to the private `TwoPhaseCommitTool.rollback`.
    pub fn rollback(objects: &[&dyn TwoPhaseCommit]) {
        for object in objects {
            let _ = object.rollback();
        }
    }
}
